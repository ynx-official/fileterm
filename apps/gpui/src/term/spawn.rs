//! PTY → TermSession feed pump with frame-coalesced backpressure.
//!
//! Phase G-1.5 of `docs/plans/active/gpui-spike.md`.
//!
//! ## Why this module exists
//!
//! G-1.4's `TermView::new` shipped a naive broadcast consumer: every chunk
//! immediately called `session.feed(bytes)` + `cx.notify()`. Under a
//! firehose like `yes` (≈100k chunks/sec on a fast box) that translates to
//! 100k model mutations and 100k `notify()` round-trips per second — far
//! above what gpui's repaint scheduler can absorb, so the foreground
//! executor queues up faster than it drains, the UI stalls, and the user
//! sees a frozen frame followed by a giant jump.
//!
//! The fix is **frame coalescing**: drain as many chunks as arrived since
//! the last frame, concatenate them into a single `Vec<u8>`, and feed the
//! model once per ~16ms tick. That collapses 100k mutations/sec down to
//! ~60/sec, which is exactly the repaint budget anyway.
//!
//! ## Why `cx.spawn` instead of `std::thread` (the spike skeleton's choice)
//!
//! The spike skeleton spawns a raw OS thread + standalone tokio runtime and
//! calls `model.update(&mut ModelContext::default(), ...)` from there. That
//! is broken in the real gpui API: `Entity::update` requires the gpui
//! foreground executor (it borrows `AppCell::borrow_mut()` which is
//! `!Sync`), and `ModelContext::default()` is a test-only constructor that
//! silently does nothing in a live app. The skeleton's comment admits
//! "G0 阶段重写为 cx.spawn 才是正解" — that's what this module is.
//!
//! `Context::spawn` schedules the future on gpui's foreground executor, so
//! `session.update(cx, ...)` runs on the main thread without borrowing
//! gymnastics. We still need `tokio::time::interval` for the 16ms tick —
//! tokio's `Interval` polls fine from gpui's executor as long as the
//! `rt` and `time` features are enabled (already pulled in by
//! `portable-pty`'s tokio dep, see `Cargo.toml`).
//!
//! ## Backpressure story
//!
//! The PTY read thread (in `pty.rs`) pushes `TermChunk`s into a
//! `broadcast::channel(256)`. If the foreground consumer falls behind by
//! more than 256 chunks between polls, `recv()` returns
//! `RecvError::Lagged(n)` and the next `n` chunks are **lost**. This is
//! intentional — under sustained `yes` load the chunks are redundant
//! (every line is `"y\n"`), so dropping them is better than stalling the
//! PTY read thread. The Lagged count is added to `dropped_chunks` so the
//! UI can surface "we lost N chunks" as a status indicator, and the next
//! frame repaints from scratch via `mark_all_dirty()` so no torn rows
//! remain on-screen.
//!
//! `interval.set_missed_tick_behavior(Skip)` keeps the tick cadence at
//! 16ms even if a tick fires late (e.g. a long `feed` blocked the
//! executor for 50ms) — without `Skip`, tokio would try to "catch up" by
//! firing 3 ticks back-to-back, which would feed the model 3 times in
//! the same frame and defeat the coalescing.

use std::sync::Arc;
use std::time::Duration;

use gpui::{AsyncApp, Context, Task, WeakEntity};
use tokio::sync::broadcast;
use tokio::time::{self, MissedTickBehavior};

use crate::term::{TermChunk, TermSession};

/// Frame budget for the coalescing pump. 16ms ≈ 60fps; any faster and
/// we'd feed the model more than once per repaint, wasting work. Any
/// slower and the user-visible latency for typing / `ls` output grows
/// past the perceptible threshold (~50ms).
pub const FEED_TICK: Duration = Duration::from_millis(16);

/// Spawn the foreground coalescing pump for a `TermSession`.
///
/// The returned `Task` is detached by the caller (typically
/// `TermView::new`); it terminates naturally when the broadcast sender
/// closes (i.e. when the PTY's read thread exits because the child shell
/// died).
///
/// Generic over the *owner* context `U` because the caller is usually
/// `TermView::new` (which has `Context<TermView>`), not
/// `Context<TermSession>`. `Context::spawn` works for any owner type — it
/// just runs the future on the foreground executor; the `WeakEntity<U>`
/// it would hand back is unused here (we captured the session's
/// `WeakEntity` explicitly).
///
/// `dropped_chunks` is an `Arc<AtomicU64>` rather than a field on the
/// session because (a) it's read from the UI status bar on every frame
/// and (b) it's written from inside the pump's `update` closure — both
/// want `&self` access, which `AtomicU64` gives without going through
/// the entity borrow machinery.
pub fn spawn_term_feed<U: 'static>(
    cx: &mut Context<U>,
    session: WeakEntity<TermSession>,
    mut rx: broadcast::Receiver<TermChunk>,
    dropped_chunks: Arc<std::sync::atomic::AtomicU64>,
) -> Task<()> {
    cx.spawn(async move |_weak_self, cx: &mut AsyncApp| {
        let mut interval = time::interval(FEED_TICK);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        // Don't accumulate an immediate tick from t=0 — we want the first
        // feed to happen only after a real chunk arrives (otherwise we'd
        // fire `feed(&[])` once at startup, which is a no-op but wastes
        // a `cx.notify()`).
        interval.reset();

        // Coalescing buffer. Sized to the typical read-burst size of one
        // `find /` stanza; grows if needed via `extend_from_slice`.
        let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
        let mut pending_dropped: u64 = 0;

        loop {
            // `tokio::select!` polls both arms: as soon as either the
            // receiver yields a chunk OR the interval fires, we wake up.
            // Crucially, on each wake we drain **all** currently-ready
            // chunks before feeding the model — otherwise a fast burst
            // would still cause one mutation per chunk. We get "drain
            // all ready chunks" for free because `select!` returns after
            // the *first* ready arm; subsequent chunks are picked up in
            // the next loop iteration's `select!` (also fast, since the
            // receiver is already ready). The 16ms tick acts as a floor
            // — if chunks arrive faster than 16ms apart, we batch them
            // by skipping the tick arm via `biased`.
            tokio::select! {
                biased; // prioritize chunk arrival over tick
                recv = rx.recv() => {
                    match recv {
                        Ok(chunk) => {
                            buf.extend_from_slice(&chunk.bytes);
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            // Coalesce Lagged increments too — we only
                            // push the cumulative count to the AtomicU64
                            // on the next tick, avoiding contention on
                            // the atomic under sustained backpressure.
                            pending_dropped = pending_dropped.saturating_add(n);
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            // Shell exited. Do one last feed so any
                            // buffered final output (e.g. bash's exit
                            // message) lands in the model before the
                            // pump dies.
                            feed_once(&session, cx, &mut buf, &mut pending_dropped, &dropped_chunks);
                            break;
                        }
                    }
                }
                _ = interval.tick() => {
                    feed_once(&session, cx, &mut buf, &mut pending_dropped, &dropped_chunks);
                }
            }
        }
    })
}

/// Drain the coalescing buffer + pending drop counter into the session.
///
/// Factored out so both the tick arm and the `Closed` arm can flush the
/// final buffer. Synchronous (no `.await`) — `WeakEntity::update` on the
/// foreground executor resolves synchronously.
fn feed_once(
    session: &WeakEntity<TermSession>,
    cx: &mut AsyncApp,
    buf: &mut Vec<u8>,
    pending_dropped: &mut u64,
    dropped_chunks: &Arc<std::sync::atomic::AtomicU64>,
) {
    if buf.is_empty() && *pending_dropped == 0 {
        return;
    }

    // Commit the dropped count first so the status bar reflects the loss
    // in the same frame the user sees the (possibly torn) output land.
    let had_drops = *pending_dropped > 0;
    if had_drops {
        dropped_chunks.fetch_add(*pending_dropped, std::sync::atomic::Ordering::Relaxed);
        *pending_dropped = 0;
    }

    let bytes = std::mem::take(buf);
    let _ = session.update(cx, |s, cx| {
        if !bytes.is_empty() {
            s.feed(&bytes);
        }
        // If we lost chunks since the last feed, the model's view of the
        // stream is now ahead of what's on-screen in a non-contiguous
        // way — repaint from scratch so no torn rows survive.
        // (`mark_all_dirty` is cheap — it just sets bool flags per row.)
        if had_drops {
            s.model.mark_all_dirty();
        }
        cx.notify();
    });
}

// === Tests ===
//
// We don't unit-test the pump directly: driving `cx.spawn` + tokio's
// interval requires a foreground executor + a running tokio runtime,
// which is painful to set up in a `#[test]`. The integration coverage
// of the full pump (burst coalescing + dropped_chunks under `yes`)
// lives in the G-1.6 `term_bench` example, which runs the pump against
// a real PTY for 30s and inspects the metrics. This matches the spike
// skeleton's guidance: "G-1.5 验收: 跑 `yes` 命令，观察 `dropped_chunks`
// 计数是否非零（说明背压生效）".
