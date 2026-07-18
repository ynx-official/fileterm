//! Terminal view: paints the [`TermModel`] grid via gpui's text system.
//!
//! Phase G-1.4 of `docs/plans/active/gpui-spike.md`.
//!
//! Deviations from the spike skeleton, all forced by the real
//! `gpui-unofficial` 1.8.2 API surface:
//!   * `cx.new_view()` does not exist on `&mut App`. Entity construction is
//!     `Context::new` (trait method on `AppContext`, impl'd by both `App` and
//!     `Context<T>`). The spike's `cx.new(|_| ...)` works inside
//!     `Context<TermView>` to create a child `Entity<TermSession>`.
//!   * There is no `Canvas::new`. The free function `gpui::canvas(prepaint,
//!     paint)` constructs the canvas element directly.
//!   * `Render::render` returns `impl IntoElement`, not `Element`. Both `div`
//!     and `canvas` satisfy `IntoElement`, so a `Div` with a `Canvas` child is
//!     fine.
//!   * `App::focus_handle()` is the constructor for `FocusHandle`; on
//!     `Context<T>` it's accessible via Deref<Target = App>. There's no
//!     separate `build_focus_handle` helper.
//!   * `Window::focus(&FocusHandle, &mut App)` moves focus. `Window::focused`
//!     queries the current focus. We auto-focus on first render via a
//!     `did_focus` flag stored on the view.
//!   * `cx.spawn(async move |cx| ...)` returns a `Task` that is cancelled when
//!     dropped. `Task::detach()` keeps it running for the entity's lifetime —
//!     the spike's PTY pump uses this to drive the foreground broadcast
//!     consumer.
//!   * `Entity<T>::read(&App)` returns `&T` borrowing the cx; `update(cx, ...)`
//!     mutates with `&mut Context<T>`. The canvas paint callback receives
//!     `&mut App`, so reads use `session.read(cx)` (immutable borrow, dropped
//!     before any mutation) and resizes use `cx.defer(...)` to schedule
//!     mutation after the paint phase.
//!   * `cx.listener(...)` returns `impl Fn(&E, &mut Window, &mut App)` and is
//!     the idiomatic way to get `&mut T` inside a div listener. We use it for
//!     `on_key_down` so the keystroke→bytes translation can write to the
//!     PTY via `view.pty.write_input(...)`.
//!
//! Color rendering: SGR `Default` falls back to theme defaults; `Indexed(u8)`
//! looks up a 16-color xterm palette (and a gray ramp for 17–255, since the
//! spike only spec'd the 16-color base palette); `Rgb(r,g,b)` converts to
//! `Hsla` via `gpui::rgb` + `Into<Hsla>`. SGR `REVERSE` swaps fg/bg at paint
//! time. `BOLD`/`ITALIC`/`UNDERLINE` map to per-run `FontWeight` /
//! `FontStyle` / `UnderlineStyle`. `DIM` and `HIDDEN` are honored (`HIDDEN`
//! makes text fully transparent); the 256-color cube is a gray ramp fallback
//! (full cube is G3).

use std::sync::Arc;

use gpui::{
    canvas, div, fill, font, point, px, rgb, size, transparent_black, App, AppContext, Bounds,
    Context, Entity, FocusHandle, Focusable, FontStyle, FontWeight, Hsla, InteractiveElement,
    IntoElement, KeyDownEvent, Keystroke, ParentElement, Pixels, Render, ShapedLine, SharedString,
    Styled, TextAlign, Window,
};

use crate::term::{CellFlags, Color, ColorKind, Cursor, CursorStyle, PtyHandle, TermSession};

/// Light-gray on near-black, matching the spike's `#0c0c0c` background.
/// Constructed as struct literals (not via `hsla(...)`) because `hsla` is
/// not a const fn and these are used in `const` contexts.
const DEFAULT_FG: Hsla = Hsla {
    h: 0.0,
    s: 0.0,
    l: 0.85,
    a: 1.0,
};
const DEFAULT_BG: Hsla = Hsla {
    h: 0.0,
    s: 0.0,
    l: 0.047,
    a: 1.0,
};
const CURSOR_COLOR: Hsla = Hsla {
    h: 0.0,
    s: 0.0,
    l: 0.85,
    a: 0.6,
};

/// 16-color xterm palette (bright variants 8–15 follow standard ANSI).
/// Built as a function rather than a `const` because `Hsla` doesn't have a
/// const constructor and we want to keep `hsla(...)`'s clamping behavior.
fn ansi_palette() -> [Hsla; 16] {
    [
        Hsla {
            h: 0.0,
            s: 0.0,
            l: 0.0,
            a: 1.0,
        }, // 0 black
        Hsla {
            h: 0.0,
            s: 1.0,
            l: 0.5,
            a: 1.0,
        }, // 1 red
        Hsla {
            h: 0.33,
            s: 1.0,
            l: 0.4,
            a: 1.0,
        }, // 2 green
        Hsla {
            h: 0.17,
            s: 1.0,
            l: 0.5,
            a: 1.0,
        }, // 3 yellow
        Hsla {
            h: 0.66,
            s: 1.0,
            l: 0.5,
            a: 1.0,
        }, // 4 blue
        Hsla {
            h: 0.83,
            s: 1.0,
            l: 0.5,
            a: 1.0,
        }, // 5 magenta
        Hsla {
            h: 0.5,
            s: 1.0,
            l: 0.5,
            a: 1.0,
        }, // 6 cyan
        Hsla {
            h: 0.0,
            s: 0.0,
            l: 0.85,
            a: 1.0,
        }, // 7 white (light gray)
        Hsla {
            h: 0.0,
            s: 0.0,
            l: 0.5,
            a: 1.0,
        }, // 8 bright black (dark gray)
        Hsla {
            h: 0.0,
            s: 1.0,
            l: 0.65,
            a: 1.0,
        }, // 9 bright red
        Hsla {
            h: 0.33,
            s: 1.0,
            l: 0.6,
            a: 1.0,
        }, // 10 bright green
        Hsla {
            h: 0.17,
            s: 1.0,
            l: 0.65,
            a: 1.0,
        }, // 11 bright yellow
        Hsla {
            h: 0.66,
            s: 1.0,
            l: 0.7,
            a: 1.0,
        }, // 12 bright blue
        Hsla {
            h: 0.83,
            s: 1.0,
            l: 0.7,
            a: 1.0,
        }, // 13 bright magenta
        Hsla {
            h: 0.5,
            s: 1.0,
            l: 0.7,
            a: 1.0,
        }, // 14 bright cyan
        Hsla {
            h: 0.0,
            s: 0.0,
            l: 1.0,
            a: 1.0,
        }, // 15 bright white
    ]
}

fn color_to_hsla(c: Color, is_fg: bool) -> Hsla {
    match c.kind {
        ColorKind::Default => {
            if is_fg {
                DEFAULT_FG
            } else {
                DEFAULT_BG
            }
        }
        ColorKind::Indexed(idx) => {
            let palette = ansi_palette();
            if (idx as usize) < palette.len() {
                palette[idx as usize]
            } else {
                // 256-color cube fallback: simple gray ramp. Full cube is G3.
                let l = 0.25 + 0.55 * (idx as f32 - 16.0) / (255.0 - 16.0);
                Hsla {
                    h: 0.0,
                    s: 0.0,
                    l: l.min(1.0),
                    a: 1.0,
                }
            }
        }
        ColorKind::Rgb(r, g, b) => rgb(((r as u32) << 16) | ((g as u32) << 8) | (b as u32)).into(),
    }
}

/// The terminal view: owns a child `Entity<TermSession>`, an `Arc<PtyHandle>`
/// for input writes, and the font metrics used to map grid cells ↔ pixels.
pub struct TermView {
    pub session: Entity<TermSession>,
    pub pty: Arc<PtyHandle>,
    font: gpui::Font,
    font_size: Pixels,
    cell_w: Pixels,
    cell_h: Pixels,
    focus: FocusHandle,
    /// Last grid dimensions in cells. Captured into the canvas paint callback
    /// to detect size changes; updated when a deferred resize lands.
    cols: usize,
    rows: usize,
    /// Whether we've grabbed window focus at least once. Render is the only
    /// place we have a `&mut Window`, so we use this flag to grab focus on
    /// the first frame.
    did_focus: bool,
    /// Count of broadcast chunks dropped due to `Lagged` since startup.
    /// Surfaced in the window title for the G-1.5 acceptance test ("run
    /// `yes` and watch the count go non-zero"). `Arc<AtomicU64>` because
    /// the foreground feed pump (in `spawn.rs`) writes it from inside the
    /// `session.update` closure while the render path reads it here.
    pub dropped_chunks: Arc<std::sync::atomic::AtomicU64>,
    /// Frame counter used to throttle `set_window_title` calls — updating
    /// the platform title every frame is wasteful (X11/Wayland round-trip)
    /// and most frames the count hasn't changed anyway. We refresh every
    /// 30 frames (~0.5s at 60fps), which is plenty for a status indicator.
    frame_counter: u32,
    /// Cached dropped count from the last time we updated the title.
    /// Compared against the atomic on each frame so we skip the platform
    /// call entirely when nothing changed.
    last_title_dropped: u64,
}

impl TermView {
    /// Construct a terminal view around an already-spawned PTY. The view
    /// takes ownership of one broadcast subscriber (the PTY keeps the
    /// sender, so additional views can `pty.subscribe()` if needed).
    ///
    /// `cols` and `rows` are the initial grid dimensions; they will be
    /// re-derived from the actual canvas bounds on the first paint.
    pub fn new(cx: &mut Context<Self>, pty: Arc<PtyHandle>, cols: usize, rows: usize) -> Self {
        let session = cx.new(|_cx| TermSession::new(cols, rows));
        let focus = cx.focus_handle();
        let dropped_chunks = Arc::new(std::sync::atomic::AtomicU64::new(0));

        // Pick a monospace family that exists on the dev box (DejaVu Sans Mono
        // is shipped with fontconfig on essentially every Linux distro).
        // Cross-platform fallbacks land later — for now the spike runs on
        // Linux only.
        let font = font("DejaVu Sans Mono");
        let font_size = px(14.0);
        let text_system = cx.text_system().clone();
        let font_id = text_system.resolve_font(&font);
        // `Pixels.0` is `pub(crate)`, so use the public `Mul<f32> for Pixels`
        // impl (`font_size * 0.6` returns `Pixels`) instead of unwrapping.
        let cell_w = text_system
            .advance(font_id, font_size, 'M')
            .map(|s| s.width)
            .unwrap_or_else(|_| font_size * 0.6);
        let cell_h = text_system.ascent(font_id, font_size)
            + text_system.descent(font_id, font_size)
            + px(1.0);
        // font_id is consumed by the metrics calls above; we keep it around
        // for diagnostics but don't need to store it on the view (shape_line
        // picks its own font_id from each TextRun's Font).
        let _ = font_id;

        // Spawn the foreground coalescing pump (G-1.5). The pump drains
        // the PTY broadcast on a 16ms tick, concatenating all chunks
        // that arrived since the last tick into a single `session.feed`
        // call. Under a `yes` firehose this collapses ~100k mutations/sec
        // down to ~60/sec, matching the repaint budget and keeping the
        // UI responsive. `Lagged` gaps are counted into `dropped_chunks`
        // and trigger a `mark_all_dirty` so no torn rows survive.
        //
        // See `spawn.rs` for the full backpressure / coalescing story.
        let rx = pty.subscribe();
        let session_weak = session.downgrade();
        crate::term::spawn::spawn_term_feed(cx, session_weak, rx, dropped_chunks.clone()).detach();

        Self {
            session,
            pty,
            font,
            font_size,
            cell_w,
            cell_h,
            focus,
            cols,
            rows,
            did_focus: false,
            dropped_chunks,
            frame_counter: 0,
            last_title_dropped: 0,
        }
    }
}

impl Focusable for TermView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TermView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Auto-focus on the first frame so keystrokes reach the terminal
        // without the user having to click. Subsequent renders skip this.
        if !self.did_focus {
            self.did_focus = true;
            _window.focus(&self.focus, cx);
        }

        // Surface `dropped_chunks` in the window title for the G-1.5
        // acceptance test ("run `yes` and observe the count go non-zero").
        // Refresh every 30 frames OR when the count changes — whichever
        // comes first. The atomic load is `Relaxed` because we only need
        // eventual consistency for a status indicator; the title string
        // is built cheaply from a single integer.
        self.frame_counter = self.frame_counter.wrapping_add(1);
        let dropped = self
            .dropped_chunks
            .load(std::sync::atomic::Ordering::Relaxed);
        if dropped != self.last_title_dropped || self.frame_counter.is_multiple_of(30) {
            self.last_title_dropped = dropped;
            _window.set_window_title(&format!("FileTerm GPUI Spike — dropped: {}", dropped));
        }

        let cell_w = self.cell_w;
        let cell_h = self.cell_h;
        let font = self.font.clone();
        let font_size = self.font_size;
        let session = self.session.clone();
        let pty = self.pty.clone();
        let weak_self = cx.weak_entity();
        let cols = self.cols;
        let rows = self.rows;

        let on_key_down = cx.listener(move |view, event: &KeyDownEvent, _window, _cx| {
            let bytes = keystroke_to_bytes(&event.keystroke);
            if !bytes.is_empty() {
                if let Err(e) = view.pty.write_input(&bytes) {
                    eprintln!("[term-view] pty write error: {e:#}");
                }
            }
        });

        div()
            .size_full()
            .bg(DEFAULT_BG)
            .track_focus(&self.focus)
            .on_key_down(on_key_down)
            .child(canvas(
                move |bounds, _window, _cx| bounds,
                move |bounds, _state, window, cx| {
                    // ---- Phase 1: paint (handles its own session borrow) ----
                    paint_terminal_grid(
                        window, cx, &session, bounds, &font, font_size, cell_w, cell_h,
                    );
                    // ---- Phase 2: detect resize and defer the mutation ----
                    // `Pixels / Pixels -> f32` (Div impl), so no `.0` unwrap.
                    let new_cols = ((bounds.size.width / cell_w).floor() as usize).max(1);
                    let new_rows = ((bounds.size.height / cell_h).floor() as usize).max(1);
                    if new_cols != cols || new_rows != rows {
                        let session_clone = session.clone();
                        let pty_clone = pty.clone();
                        let weak_self_clone = weak_self.clone();
                        cx.defer(move |cx| {
                            let _ = weak_self_clone.update(cx, |view, cx| {
                                view.cols = new_cols;
                                view.rows = new_rows;
                                cx.notify();
                            });
                            session_clone.update(cx, |s, cx| {
                                s.resize(new_cols, new_rows);
                                cx.notify();
                            });
                            let _ = pty_clone.resize(new_cols as u16, new_rows as u16);
                        });
                    }
                },
            ))
    }
}

/// Paint the entire visible grid plus the cursor.
///
/// ## Borrow strategy
///
/// The canvas paint callback gives us `&mut Window` and `&mut App`, but
/// `Entity::read(cx)` returns `&T` borrowing `cx` immutably — which conflicts
/// with the `&mut App` that `ShapedLine::paint` needs. To work around this
/// without cloning the whole grid per frame, we split the work into two
/// phases:
///
/// 1. **Shape phase** (inside `Entity::read_with`): hold an immutable borrow
///    of the session, iterate the grid, and call `WindowTextSystem::shape_line`
///    for each row. `ShapedLine` owns its layout (`Arc<LineLayout>`), so the
///    vec of shaped lines can be returned from the closure, releasing the
///    session borrow.
/// 2. **Paint phase** (after `read_with` returns): iterate the shaped lines
///    and call `ShapedLine::paint_background` / `ShapedLine::paint`, which
///    need `&mut Window, &mut App` — now freely available because the session
///    borrow is gone.
///
/// The `WindowTextSystem` Arc is cloned from `window.text_system()` before
/// the `read_with` closure so we can call `shape_line` inside it without
/// borrowing `window` (the closure only has `&App`, not `&Window`).
#[allow(clippy::too_many_arguments)]
fn paint_terminal_grid(
    window: &mut Window,
    cx: &mut App,
    session: &Entity<TermSession>,
    bounds: Bounds<Pixels>,
    font: &gpui::Font,
    font_size: Pixels,
    cell_w: Pixels,
    cell_h: Pixels,
) {
    // Background fill for the whole terminal area. Per-cell background colors
    // paint on top of this; cells with `ColorKind::Default` bg show through
    // to this base.
    window.paint_quad(fill(bounds, DEFAULT_BG));

    // `shape_line` is defined on `WindowTextSystem`. `Window::text_system()`
    // returns `&Arc<WindowTextSystem>`; clone the Arc so (a) the immutable
    // borrow of `window` ends here, and (b) we can use it inside the
    // `read_with` closure below (which only has `&App`).
    let text_system = window.text_system().clone();
    let line_height = cell_h;

    // ---- Phase 1: shape all rows + snapshot cursor ----
    // `Entity::read_with` scopes the immutable session borrow to the closure.
    // After it returns, `cx` is free for `&mut App` calls.
    let (rows_to_paint, cursor_snap): (Vec<(ShapedLine, gpui::Point<Pixels>)>, Option<Cursor>) =
        session.read_with(cx, |s, _cx| {
            let mut out: Vec<(ShapedLine, gpui::Point<Pixels>)> = Vec::with_capacity(s.rows);
            for r in 0..s.rows {
                let row = &s.grid[r];
                if row.is_empty() {
                    continue;
                }

                // Group consecutive cells with identical (fg, bg, flags) into
                // a run so we issue a single `shape_line` per row and a single
                // `TextRun` per attribute-stable segment.
                let mut text = String::with_capacity(row.len());
                let mut runs: Vec<gpui::TextRun> = Vec::new();
                let mut run_start_byte: usize = 0;
                let mut cur_fg = row[0].fg;
                let mut cur_bg = row[0].bg;
                let mut cur_flags = row[0].flags;

                // Helper: push a TextRun for [run_start_byte, text.len()).
                let flush_run = |runs: &mut Vec<gpui::TextRun>,
                                 text: &String,
                                 start: &mut usize,
                                 fg: Color,
                                 bg: Color,
                                 flags: CellFlags| {
                    let len = text.len() - *start;
                    if len == 0 {
                        return;
                    }
                    // Reverse video: swap fg/bg at paint time.
                    let (eff_fg, eff_bg) = if flags.contains(CellFlags::REVERSE) {
                        (bg, fg)
                    } else {
                        (fg, bg)
                    };
                    let mut run_font = font.clone();
                    if flags.contains(CellFlags::BOLD) {
                        run_font.weight = FontWeight::BOLD;
                    }
                    if flags.contains(CellFlags::ITALIC) {
                        run_font.style = FontStyle::Italic;
                    }
                    let underline = if flags.contains(CellFlags::UNDERLINE) {
                        Some(gpui::UnderlineStyle {
                            thickness: px(1.0),
                            color: Some(color_to_hsla(eff_fg, true)),
                            wavy: false,
                        })
                    } else {
                        None
                    };
                    runs.push(gpui::TextRun {
                        len,
                        font: run_font,
                        color: if flags.contains(CellFlags::HIDDEN) {
                            transparent_black()
                        } else {
                            color_to_hsla(eff_fg, true)
                        },
                        background_color: Some(color_to_hsla(eff_bg, false)),
                        underline,
                        strikethrough: None,
                    });
                    *start = text.len();
                };

                for (i, cell) in row.iter().enumerate() {
                    if i > 0 && (cell.fg != cur_fg || cell.bg != cur_bg || cell.flags != cur_flags)
                    {
                        flush_run(
                            &mut runs,
                            &text,
                            &mut run_start_byte,
                            cur_fg,
                            cur_bg,
                            cur_flags,
                        );
                        cur_fg = cell.fg;
                        cur_bg = cell.bg;
                        cur_flags = cell.flags;
                    }
                    // Replace NUL / control chars with space so the grid stays
                    // visually aligned (vte's `print` filters most C0 already,
                    // but we defensively normalize anything that slipped
                    // through).
                    let ch = if cell.ch == '\0' || (cell.ch.is_control() && cell.ch != '\t') {
                        ' '
                    } else {
                        cell.ch
                    };
                    text.push(ch);
                }
                flush_run(
                    &mut runs,
                    &text,
                    &mut run_start_byte,
                    cur_fg,
                    cur_bg,
                    cur_flags,
                );

                if text.is_empty() {
                    continue;
                }

                let shaped = text_system.shape_line(
                    SharedString::from(text),
                    font_size,
                    &runs,
                    // Force glyphs onto the monospace cell grid so cursor
                    // positioning matches user expectations even if the
                    // chosen font has a slightly different advance for some
                    // glyphs.
                    Some(cell_w),
                );

                // `Pixels.0` is `pub(crate)`; `cell_h * (r as f32)` returns
                // `Pixels` via `Mul<f32> for Pixels`, no `px()` wrap needed.
                let origin = point(bounds.origin.x, bounds.origin.y + cell_h * (r as f32));
                out.push((shaped, origin));
            }

            // Snapshot the cursor (cloneable — `Cursor` derives `Clone`) so we
            // can paint it after releasing the session borrow.
            let cursor_snap = if s.cursor.visible {
                Some(s.cursor.clone())
            } else {
                None
            };
            (out, cursor_snap)
        });

    // ---- Phase 2: paint each shaped line ----
    // Now `cx` is mutably available for `ShapedLine::paint` (which internally
    // calls `window.paint_glyph` etc.).
    for (shaped, origin) in &rows_to_paint {
        let _ = shaped.paint_background(*origin, line_height, TextAlign::Left, None, window, cx);
        let _ = shaped.paint(*origin, line_height, TextAlign::Left, None, window, cx);
    }

    // ---- Cursor: render as a translucent quad at the cursor position ----
    // We don't yet blink (would need an `on_next_frame` timer; planned for G3).
    if let Some(cursor) = cursor_snap {
        let cur_bounds = Bounds::new(
            point(
                bounds.origin.x + cell_w * (cursor.col as f32),
                bounds.origin.y + cell_h * (cursor.row as f32),
            ),
            size(cell_w, cell_h),
        );
        let color = match cursor.style {
            CursorStyle::Block => CURSOR_COLOR,
            CursorStyle::Bar => CURSOR_COLOR,
            CursorStyle::Underline => CURSOR_COLOR,
        };
        window.paint_quad(fill(cur_bounds, color));
    }
}

/// Translate a gpui `Keystroke` into the raw bytes a Unix PTY expects.
///
/// Coverage:
///   * Printable ASCII (key_char or single-char `key`): emit as-is. Shift is
///     implicit in key_char ("A" for shift+a), so no special handling.
///   * Ctrl+A..Z → 0x01..0x1A. Ctrl+@, [, \, ], ^, _ → their C0 equivalents.
///   * Enter → `\r` (CR, what shells expect — not LF).
///   * Backspace → `\x7f` (DEL, xterm convention).
///   * Tab → `\t`. Escape → `\x1b`.
///   * Arrows / Home / End / PageUp / PageDown / Delete / Insert → xterm
///     CSI sequences.
///   * Alt+key prefixes `\x1b` to the key's bytes (xterm Meta convention).
///
/// Modifiers like Cmd/Super are dropped (terminal apps don't read them) to
/// avoid e.g. Cmd+C being swallowed as a copy without writing bytes.
fn keystroke_to_bytes(ks: &Keystroke) -> Vec<u8> {
    let mods = &ks.modifiers;

    // Control + letter / punct → C0 control character.
    if mods.control && !mods.platform {
        let lc_key = ks.key.to_ascii_lowercase();
        if let Some(ch) = lc_key.chars().next() {
            if ch.is_ascii_lowercase() {
                let mut out = Vec::with_capacity(2);
                if mods.alt {
                    out.push(0x1b);
                }
                out.push((ch as u8) - b'a' + 1);
                return out;
            }
            // Ctrl + punct C0 set.
            let c0 = match lc_key.as_str() {
                "@" => Some(0x00),
                "[" => Some(0x1b),
                "\\" => Some(0x1c),
                "]" => Some(0x1d),
                "^" => Some(0x1e),
                "_" => Some(0x1f),
                _ => None,
            };
            if let Some(b) = c0 {
                let mut out = Vec::with_capacity(2);
                if mods.alt {
                    out.push(0x1b);
                }
                out.push(b);
                return out;
            }
        }
    }

    // Special keys.
    let special: Option<Vec<u8>> = match ks.key.as_str() {
        "enter" | "return" => Some(vec![b'\r']),
        "backspace" => Some(vec![0x7f]),
        "tab" => Some(vec![b'\t']),
        "escape" => Some(vec![0x1b]),
        "up" => Some(vec![0x1b, b'[', b'A']),
        "down" => Some(vec![0x1b, b'[', b'B']),
        "right" => Some(vec![0x1b, b'[', b'C']),
        "left" => Some(vec![0x1b, b'[', b'D']),
        "home" => Some(vec![0x1b, b'O', b'H']),
        "end" => Some(vec![0x1b, b'O', b'F']),
        "pageup" => Some(vec![0x1b, b'[', b'5', b'~']),
        "pagedown" => Some(vec![0x1b, b'[', b'6', b'~']),
        "delete" => Some(vec![0x1b, b'[', b'3', b'~']),
        "insert" => Some(vec![0x1b, b'[', b'2', b'~']),
        _ => None,
    };
    if let Some(mut bytes) = special {
        if mods.alt {
            let mut prefixed = vec![0x1b];
            prefixed.append(&mut bytes);
            return prefixed;
        }
        return bytes;
    }

    // Prefer key_char (the actually-typed character, taking shift/AltGr into
    // account) over `key` (the physical key name). Fall back to `key` when
    // key_char is None (e.g., some layouts under plain letter presses).
    let source = ks.key_char.as_deref().unwrap_or(ks.key.as_str());
    if !mods.control && !mods.platform {
        let mut bytes = source.as_bytes().to_vec();
        if bytes.is_empty() {
            return Vec::new();
        }
        if mods.alt {
            let mut prefixed = vec![0x1b];
            prefixed.append(&mut bytes);
            return prefixed;
        }
        return bytes;
    }

    // Cmd/Super or unhandled combos: emit nothing so the platform's default
    // (clipboard, window manager, etc.) can handle them.
    Vec::new()
}
