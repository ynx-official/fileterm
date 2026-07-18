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
//! ## Architecture: split tokio / gpui halves
//!
//! The pump has two halves connected by a `std::sync::mpsc` channel:
//!
//! 1. **Tokio half** (`tokio::spawn`): owns the `broadcast::Receiver` and
//!    a `tokio::time::Interval`. On each tick (or chunk arrival via
//!    `select!`), it drains + coalesces into a `Vec<u8>` and sends
//!    `(bytes, dropped_count)` through the mpsc channel. This half runs
//!    on the tokio runtime's worker threads, where `broadcast::recv` and
//!    `time::interval` have a reactor to park against.
//!
//! 2. **gpui half** (`cx.spawn` on the foreground executor): polls the
//!    mpsc channel every `FEED_TICK` using `cx.background_executor().timer`
//!    (gpui's own timer, no tokio dependency). For each `(bytes, drops)`
//!    it receives, it calls `session.update(cx, |s, cx| { s.feed(&bytes);
//!    cx.notify(); })` — which must run on the foreground executor because
//!    `Entity::update` borrows the gpui `AppCell` (`!Sync`).
//!
//! This split is forced by two facts:
//!   * `tokio::time::interval` / `broadcast::recv` panic with "no reactor"
//!     when polled from gpui's foreground executor (which is not a tokio
//!     runtime, even if one is `enter()`ed on the main thread — the
//!     foreground executor runs on its own thread without the thread-local
//!     handle).
//!   * `Entity::update` must run on gpui's foreground executor; calling it
//!     from a tokio worker thread would borrow the `!Sync` `AppCell` from
//!     the wrong thread and panic.
//!
//! The mpsc channel is the boundary: tokio half produces `(bytes, drops)`,
//! gpui half consumes. `try_recv` is non-blocking so the gpui half never
//! parks on the channel — it only parks on `timer().await`, which is gpui's
//! own parking.
//!
//! ## Backpressure story
//!
//! The PTY read thread (in `pty.rs`) pushes `TermChunk`s into a
//! `broadcast::channel(256)`. If the tokio half falls behind by more than
//! 256 chunks between ticks, `recv()` returns `RecvError::Lagged(n)` and
//! the next `n` chunks are **lost**. This is intentional — under sustained
//! `yes` load the chunks are redundant (every line is `"y\n"`), so dropping
//! them is better than stalling the PTY read thread. The Lagged count is
//! accumulated as `pending_dropped` and sent through the mpsc channel
//! alongside the next batch of bytes; the gpui half adds it to the
//! `dropped_chunks` AtomicU64 and calls `mark_all_dirty()` so the next
//! frame repaints from scratch (no torn rows survive).
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

/// A coalesced batch of bytes + the count of chunks dropped due to
/// `Lagged` since the previous batch. Sent over the mpsc channel from
/// the tokio half to the gpui half.
type Batch = (Vec<u8>, u64);

/// Spawn the foreground coalescing pump for a `TermSession`.
///
/// The returned `Task` is the gpui half; it is detached by the caller
/// (typically `TermView::new`). It terminates naturally when the mpsc
/// channel's sender is dropped, which happens when the tokio half exits
/// (i.e. the PTY's read thread exited because the child shell died).
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
/// and (b) it's written from the gpui half's `update` closure — both
/// want `&self` access, which `AtomicU64` gives without going through
/// the entity borrow machinery.
///
/// **Caller responsibility**: a tokio runtime must be running and
/// `enter()`ed on the calling thread, so `tokio::spawn` can find it.
/// Examples do this by constructing a `tokio::runtime::Runtime` in
/// `main()` and holding the `enter()` guard for the process lifetime.
pub fn spawn_term_feed<U: 'static>(
    cx: &mut Context<U>,
    session: WeakEntity<TermSession>,
    mut rx: broadcast::Receiver<TermChunk>,
    dropped_chunks: Arc<std::sync::atomic::AtomicU64>,
) -> Task<()> {
    // Channel between tokio half (producer) and gpui half (consumer).
    // Bounded to 64 so a stalled gpui half doesn't let the tokio half
    // accumulate unbounded pending batches — if the gpui half is more
    // than 64 frames behind (≈1s at 60fps), the tokio half will block
    // on send, which back-pressures the broadcast receiver, which
    // eventually causes `Lagged` (the count surfaces in `dropped_chunks`).
    let (batch_tx, batch_rx) = std::sync::mpsc::sync_channel::<Batch>(64);

    // ---- Tokio half: broadcast recv + interval tick → coalesce → mpsc send
    //
    // `tokio::spawn` requires a running runtime on the current thread
    // (via `enter()`); the examples set this up in `main()`. The spawned
    // task runs on a tokio worker thread, where `time::interval` and
    // `broadcast::recv` have a reactor to park against.
    tokio::spawn(async move {
        let mut interval = time::interval(FEED_TICK);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        // Don't accumulate an immediate tick from t=0 — we want the first
        // feed to happen only after a real chunk arrives (otherwise we'd
        // fire an empty batch once at startup).
        interval.reset();

        // Coalescing buffer. Sized to the typical read-burst size of one
        // `find /` stanza; grows if needed via `extend_from_slice`.
        let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
        let mut pending_dropped: u64 = 0;

        loop {
            // `tokio::select!` polls both arms: as soon as either the
            // receiver yields a chunk OR the interval fires, we wake up.
            // `biased` prioritizes chunk arrival over tick, so all chunks
            // that arrived during the previous tick window are drained
            // into `buf` before the tick arm fires and flushes.
            tokio::select! {
                biased;
                recv = rx.recv() => {
                    match recv {
                        Ok(chunk) => {
                            buf.extend_from_slice(&chunk.bytes);
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            pending_dropped = pending_dropped.saturating_add(n);
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            // Shell exited. Flush the final batch so any
                            // buffered output (e.g. bash's exit message)
                            // lands in the model before the pump dies.
                            flush_batch(&batch_tx, &mut buf, &mut pending_dropped);
                            break;
                        }
                    }
                }
                _ = interval.tick() => {
                    flush_batch(&batch_tx, &mut buf, &mut pending_dropped);
                }
            }
        }
        // `batch_tx` drops here; the gpui half's `try_recv` will start
        // returning `Err(Disconnected)` and exit its loop.
    });

    // ---- gpui half: timer tick → drain mpsc → session.update
    //
    // Runs on the foreground executor (via `cx.spawn`). Uses gpui's own
    // `background_executor().timer()` for the 16ms tick — no tokio
    // dependency. Each tick, drain all pending batches via `try_recv`
    // (non-blocking) and feed each to the session.
    cx.spawn(async move |_weak_self, cx: &mut AsyncApp| {
        loop {
            cx.background_executor().timer(FEED_TICK).await;

            // Drain all batches that accumulated since the last tick.
            // `try_recv` returns immediately if nothing is pending, so
            // we never block the foreground executor on the channel.
            let mut had_any = false;
            loop {
                match batch_rx.try_recv() {
                    Ok((bytes, drops)) => {
                        had_any = true;
                        let _ = session.update(cx, |s, cx| {
                            if !bytes.is_empty() {
                                s.feed(&bytes);
                            }
                            // If we lost chunks since the last feed, the
                            // model's view of the stream is now ahead of
                            // what's on-screen in a non-contiguous way —
                            // repaint from scratch so no torn rows survive.
                            if drops > 0 {
                                s.model.mark_all_dirty();
                            }
                            cx.notify();
                        });
                        if drops > 0 {
                            dropped_chunks.fetch_add(drops, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        // Tokio half exited (shell died). We're done.
                        return;
                    }
                }
            }
            let _ = had_any; // currently unused; kept for future stats
        }
    })
}

/// Drain the coalescing buffer + pending drop counter into the mpsc
/// channel as a single `Batch`. No-op if both are empty.
///
/// Factored out so both the tick arm and the `Closed` arm can flush the
/// final buffer. Synchronous (no `.await`) — `sync_channel::send` blocks
/// only if the channel is full (64 pending batches), which back-pressures
/// the tokio half until the gpui half catches up.
fn flush_batch(
    batch_tx: &std::sync::mpsc::SyncSender<Batch>,
    buf: &mut Vec<u8>,
    pending_dropped: &mut u64,
) {
    if buf.is_empty() && *pending_dropped == 0 {
        return;
    }
    let bytes = std::mem::take(buf);
    let drops = std::mem::take(pending_dropped);
    // `send` blocks if the channel is full (64 batches). That's the
    // intended backpressure: if the gpui half is stalled, the tokio half
    // stops draining the broadcast, which eventually causes `Lagged`.
    // The error path (Disconnected) means the gpui half already exited;
    // we drop the batch silently — the shell is dying anyway.
    let _ = batch_tx.send((bytes, drops));
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
