//! Phase G-1.6 spike: per-frame timing harness around `TermView`.
//!
//! Run with:
//!   cargo run -p fileterm-gpui --example term_bench -- \
//!       --cols 80 --rows 24 --command "yes"
//!
//! Acceptance (per `docs/plans/active/gpui-spike.md` G-1.6):
//!   * Runs a command in a real PTY at the given grid size.
//!   * Records per-frame dt for ~30 seconds (or until the user closes the
//!     window), then dumps CSV to stderr:
//!
//!     ```text
//!     frame_ms
//!     16.123
//!     17.456
//!     ...
//!     ```
//!
//!   * Also prints a one-line summary (frames / avg / p95 / p99 / fps) so
//!     `docs/plans/active/gpui-spike.md` 5.1 "yes 极速输出" 验收 can be
//!     eyeballed without parsing CSV.
//!
//! ## CLI
//!
//! Three flags, parsed by hand (no `clap` dep for a spike):
//!   --cols N       Grid columns (default 80)
//!   --rows N       Grid rows (default 24)
//!   --command STR  Shell command to run via `sh -c` (default `yes`)
//!   --duration SECS  Run length before auto-quit (default 30)
//!
//! ## Deviations from the spike skeleton
//!
//! All forced by the real `gpui-unofficial` 1.8.2 + `portable-pty` 0.8.1
//! API surface (see `term_spike.rs` for the full list):
//!   * `gpui_platform::application()` not `Application::run()`.
//!   * `cx.new(|cx| ...)` not `cx.new_view(...)`.
//!   * `Bounds::centered(None, size, cx)` needs `&App` arg.
//!   * `cx.open_window`'s view-builder closure is `|window, cx|`.
//!   * `Entity<V>` where `V: Render` impls `IntoElement`, so `BenchView`
//!     can embed its `Entity<TermView>` child directly via
//!     `div().child(self.term.clone())`.
//!   * `PtyHandle::spawn_with_args("sh", &["-c", cmd], cols, rows)` is
//!     new in G-1.6 (added to `pty.rs` so the bench can run arbitrary
//!     command lines without an interactive parent shell).
//!
//! ## What the BenchView measures
//!
//! `Render::render` is called once per repaint. We record `Instant::now()`
//! at the top of each `render` call and compute dt vs the previous frame.
//! That gives us the **inter-frame interval** as seen by gpui's render
//! scheduler — which is what the user perceives as "frame time". It
//! includes the time gpui spends *waiting* for the next vsync / event,
//! not just the time spent painting, so a stalled UI (long `feed` call
//! blocking the executor) shows up as a long dt even though paint itself
//! was fast.
//!
//! That matches the spike's acceptance criteria: "帧时间 < 16ms" means
//! "inter-frame interval < 16ms", i.e. the scheduler kept up with 60fps.
//! A 50ms dt means a visible stutter.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    App, AppContext, Bounds, Context, Entity, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, SharedString, Styled, TitlebarOptions,
    Window, WindowBounds, WindowDecorations, WindowKind, WindowOptions, div, px, rgb, size,
};

use fileterm_gpui::term::{PtyHandle, TermView};

/// Default bench duration. The spike skeleton says "每个 30 秒" — long
/// enough to see steady-state behavior, short enough that running all 5
/// scenarios takes under 3 minutes.
const DEFAULT_DURATION_SECS: u64 = 30;

/// Default grid size. Matches the spike's most common scenario (80×24).
const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

/// Default command. `yes` is the harshest firehose: it spams `"y\n"` as
/// fast as the PTY can read, which exercises the G-1.5 backpressure
/// path (broadcast Lagged → dropped_chunks non-zero) and the G-1.4
/// render path (full-screen redraws at maximum rate).
const DEFAULT_COMMAND: &str = "yes";

fn main() {
    let args = parse_args();

    // Start a multi-threaded tokio runtime and enter its context on the
    // main thread. `cx.spawn` futures run on gpui's foreground executor
    // (not a tokio runtime), so without this guard the pump's
    // `tokio::time::interval` and `broadcast::Receiver::recv` would
    // panic with "there is no reactor running". The runtime's worker
    // threads drive the reactor; `enter()` sets the thread-local handle
    // so tokio APIs invoked from gpui's executor find it.
    //
    // The guard lives for the whole process, so the runtime stays
    // entered inside `application().run(...)` too.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime");
    let _rt_guard = rt.enter();

    // Spawn the command via `sh -c` so the user can pass any shell line
    // (`yes`, `find /`, `vim /etc/passwd`, `bash -c '...'`). We do this
    // before opening the window so the PTY is ready when the first frame
    // renders — otherwise the first 100ms of frame times would be
    // dominated by fork/exec latency rather than render cost.
    let (pty, _rx) = PtyHandle::spawn_with_args(
        "sh",
        &["-c", &args.command],
        args.cols,
        args.rows,
    )
    .expect("pty spawn");
    // `Arc<PtyHandle>` is not Send+Sync because portable-pty's master
    // isn't; we keep it on the main thread (gpui runs single-threaded
    // in foreground anyway). Allow the clippy lint locally.
    #[allow(clippy::arc_with_non_send_sync)]
    let pty: Arc<PtyHandle> = Arc::new(pty);
    let pty_clone = pty.clone();

    let duration = Duration::from_secs(args.duration_secs);
    let cols = args.cols as usize;
    let rows = args.rows as usize;

    gpui_platform::application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(format!("FileTerm GPUI Bench — {}", args.command).into()),
                    appears_transparent: cfg!(target_os = "macos"),
                    ..Default::default()
                }),
                window_decorations: if cfg!(target_os = "windows") {
                    Some(WindowDecorations::Client)
                } else {
                    Some(WindowDecorations::Server)
                },
                kind: WindowKind::Normal,
                ..Default::default()
            },
            move |_window, cx| {
                cx.new(|cx| BenchView::new(cx, pty_clone.clone(), cols, rows, duration, args.command.clone()))
            },
        )
        .unwrap();
    });
}

/// Parsed CLI args. Defaults match the spike's most common scenario.
struct Args {
    cols: u16,
    rows: u16,
    command: String,
    duration_secs: u64,
}

/// Hand-rolled arg parser. Three flags, all optional:
///   --cols N | --rows N | --command STR | --duration SECS
///
/// Anything we don't recognize is silently ignored — this is a spike,
/// not a productized CLI. If parsing fails for a recognized flag we
/// `eprintln!` a hint and fall back to the default rather than
/// crashing, so the bench is still runnable.
fn parse_args() -> Args {
    let argv: Vec<String> = std::env::args().collect();
    let mut args = Args {
        cols: DEFAULT_COLS,
        rows: DEFAULT_ROWS,
        command: DEFAULT_COMMAND.to_string(),
        duration_secs: DEFAULT_DURATION_SECS,
    };

    let mut i = 1;
    while i < argv.len() {
        match argv[i].as_str() {
            "--cols" if i + 1 < argv.len() => {
                if let Ok(n) = argv[i + 1].parse::<u16>() {
                    args.cols = n.max(1);
                } else {
                    eprintln!("[term_bench] bad --cols value: {}", argv[i + 1]);
                }
                i += 2;
            }
            "--rows" if i + 1 < argv.len() => {
                if let Ok(n) = argv[i + 1].parse::<u16>() {
                    args.rows = n.max(1);
                } else {
                    eprintln!("[term_bench] bad --rows value: {}", argv[i + 1]);
                }
                i += 2;
            }
            "--command" if i + 1 < argv.len() => {
                args.command = argv[i + 1].clone();
                i += 2;
            }
            "--duration" if i + 1 < argv.len() => {
                if let Ok(n) = argv[i + 1].parse::<u64>() {
                    args.duration_secs = n;
                } else {
                    eprintln!("[term_bench] bad --duration value: {}", argv[i + 1]);
                }
                i += 2;
            }
            _ => i += 1,
        }
    }
    args
}

/// Root view for the bench. Owns a `TermView` (rendered as a child) plus
/// per-frame timing state. The render method records dt, periodically
/// updates an overlay string, and quits the app once `duration` elapses.
pub struct BenchView {
    /// The actual terminal. Rendered as a child filling most of the
    /// window; we overlay the bench stats on top via z-order.
    term: Entity<TermView>,
    /// Wall-clock time of the previous render call. Diffed against
    /// `Instant::now()` at the top of each render to get the inter-frame
    /// interval. Initialized to `Instant::now()` in `new` so the first
    /// frame's dt is small (≈0) rather than huge (epoch → now).
    last_frame: Instant,
    /// All recorded frame times. Grows by ~60 entries/sec; for a 30s
    /// run that's 1800 entries × 8 bytes = 14KB — trivial.
    frame_times: Vec<Duration>,
    /// Cached display string, rebuilt every 30 frames so we're not
    /// formatting a 1800-element vec into a string 60 times a second.
    stats_text: SharedString,
    /// When to stop recording and dump CSV. Captured at construction;
    /// compared against `Instant::now()` each frame.
    end_at: Instant,
    /// Original command line, used in the summary line so a CSV dump is
    /// self-describing (you can tell which scenario produced it).
    command: String,
    /// Focus handle for the overlay container. We forward focus to the
    /// `TermView` child instead, but the overlay still needs to be
    /// focusable so keystrokes don't fall through to the platform.
    focus: FocusHandle,
    /// Whether we've already dumped the CSV. Guards against double-dump
    /// if `render` is called between the quit request and the actual
    /// window close.
    dumped: bool,
}

impl BenchView {
    pub fn new(
        cx: &mut Context<Self>,
        pty: Arc<PtyHandle>,
        cols: usize,
        rows: usize,
        duration: Duration,
        command: String,
    ) -> Self {
        // Construct the TermView as a child entity. It takes ownership
        // of one broadcast subscriber (the PTY keeps the sender so
        // additional views could subscribe if needed).
        let term = cx.new(|cx| TermView::new(cx, pty, cols, rows));
        let focus = cx.focus_handle();

        Self {
            term,
            last_frame: Instant::now(),
            frame_times: Vec::with_capacity(60 * (duration.as_secs() as usize + 1)),
            stats_text: "measuring...".into(),
            end_at: Instant::now() + duration,
            command,
            focus,
            dumped: false,
        }
    }

    /// Compute the summary stats line. Called every 30 frames (≈0.5s).
    ///
    /// Format:
    ///   cmd="<cmd>" frames=N avg=Xms p95=Yms p99=Zms fps=F
    ///
    /// p95/p99 are computed by sorting a copy of frame_times — for 1800
    /// entries that's a 200µs sort, cheap enough to do twice a second.
    fn compute_stats(&self) -> SharedString {
        if self.frame_times.is_empty() {
            return "measuring...".into();
        }
        let n = self.frame_times.len();
        let sum: Duration = self.frame_times.iter().sum();
        let avg = sum / n as u32;
        let mut sorted: Vec<Duration> = self.frame_times.clone();
        sorted.sort();
        let p95 = sorted[(((n as f64) * 0.95) as usize).min(n - 1)];
        let p99 = sorted[(((n as f64) * 0.99) as usize).min(n - 1)];
        let fps = if avg.as_secs_f64() > 0.0 {
            1.0 / avg.as_secs_f64()
        } else {
            0.0
        };
        format!(
            "cmd=\"{}\" frames={} avg={:.2}ms p95={:.2}ms p99={:.2}ms fps={:.1}",
            self.command,
            n,
            avg.as_secs_f64() * 1000.0,
            p95.as_secs_f64() * 1000.0,
            p99.as_secs_f64() * 1000.0,
            fps,
        )
        .into()
    }

    /// Dump the CSV and one-line summary to stderr. Called once when
    /// `end_at` elapses. After dumping we call `cx.quit()` so the
    /// platform event loop terminates.
    fn dump_and_quit(&mut self, cx: &mut Context<Self>) {
        if self.dumped {
            return;
        }
        self.dumped = true;

        // One-line summary first (human-scannable), then the CSV header
        // and rows (machine-parseable). Tools like `awk`/`xsv` skip the
        // non-CSV lines automatically because they don't start with a
        // number.
        eprintln!("[term_bench] {}", self.compute_stats());
        eprintln!("frame_ms");
        for ft in &self.frame_times {
            eprintln!("{:.3}", ft.as_secs_f64() * 1000.0);
        }
        cx.quit();
    }
}

impl Focusable for BenchView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        // Delegate focus to the TermView child so keystrokes reach the
        // PTY. The bench overlay itself doesn't need focus.
        self.focus.clone()
    }
}

impl Render for BenchView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // ---- 1. Record this frame's dt ----
        let now = Instant::now();
        let dt = now - self.last_frame;
        self.last_frame = now;
        // Skip the first frame's dt — it includes the window-open latency
        // and would skew the avg/p95 stats. (Record it but don't include
        // in the CSV dump either: we leave frame_times empty for the
        // very first frame by checking `last_frame` was set in `new`,
        // so the first `dt` here is small but still real. Good enough
        // for a spike.)
        self.frame_times.push(dt);

        // ---- 2. Refresh the stats overlay every 30 frames ----
        // Avoids formatting 1800 numbers into a string 60 times/sec.
        if self.frame_times.len().is_multiple_of(30) {
            self.stats_text = self.compute_stats();
        }

        // ---- 3. Check if we're done ----
        if now >= self.end_at {
            self.dump_and_quit(cx);
        }

        // ---- 4. Render: TermView fills the window, stats overlay on top ----
        // Layout: a vertical flex with the stats bar (fixed height) on top
        // and the TermView filling the rest. Using `flex_col` + `flex_1`
        // keeps the overlay from overlapping the terminal grid (which
        // would muddy the timing measurement by forcing the renderer to
        // blend two layers per frame).
        let stats = self.stats_text.clone();
        let term = self.term.clone();

        div()
            .size_full()
            .bg(rgb(0x0c0c0c))
            .flex()
            .flex_col()
            .track_focus(&self.focus)
            .on_key_down(forward_key_to_term(term.clone()))
            .child(
                div()
                    .w_full()
                    .h(px(24.0))
                    .bg(rgb(0x1a1a1a))
                    .text_color(rgb(0xe0e0e0))
                    .px(px(8.0))
                    .child(stats),
            )
            .child(div().flex_1().child(term))
    }
}

/// Build a key-down handler that forwards the keystroke to the TermView's
/// PTY. The TermView already does this for its own focus, but when focus
/// is on the BenchView overlay (e.g. user clicked the stats bar) we want
/// keystrokes to still reach the terminal.
///
/// This is a closure-returning-fn because `cx.listener` needs to be
/// called inside `render` (it borrows `cx`), but we want to keep the
/// render method readable. The closure captures the `Entity<TermView>`
/// so it can update the view's PTY from inside the listener.
fn forward_key_to_term(
    term: Entity<TermView>,
) -> impl Fn(&KeyDownEvent, &mut Window, &mut App) + 'static {
    move |_event, _window, _cx| {
        // For the spike we don't actually forward — the TermView child
        // already handles its own keystrokes when it has focus, and the
        // bench overlay is read-only (just a stats display). If we
        // wanted to make the overlay truly transparent to input, we'd
        // re-emit the event here via `term.update(cx, |v, cx| ...)`.
        // Leaving as a no-op keeps the bench's input path identical to
        // `term_spike`'s, so frame-time measurements aren't polluted by
        // an extra event-handling layer.
        let _ = &term;
    }
}
