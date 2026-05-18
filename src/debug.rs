//! Debug diagnostics for long-running Capsense instances.
//!
//! Tracks thread spawns, panics, GetMessageW errors, and shutdown signals.
//! Use `dbg_dump()` to print a snapshot of all counters.
//!
//! Enabled via `--features debug` or when built in test mode.

use std::sync::atomic::{AtomicU64, AtomicU32, Ordering};

// ── Thread tracking ─────────────────────────────────────────────────

/// Total threads spawned by hook callbacks (CapsLock timer threads)
pub static HOOK_THREAD_SPAWNED: AtomicU64 = AtomicU64::new(0);
/// Total focus-sync threads spawned
pub static FOCUS_SYNC_THREAD_SPAWNED: AtomicU64 = AtomicU64::new(0);
/// Total mouse-sync threads spawned
pub static MOUSE_SYNC_THREAD_SPAWNED: AtomicU64 = AtomicU64::new(0);
/// Peak concurrent thread estimate (incremented on spawn, atomically tracked)
pub static THREAD_PEAK: AtomicU64 = AtomicU64::new(0);
// Coarse in-flight counter — incremented on spawn, decremented on exit
pub static THREADS_IN_FLIGHT: AtomicU32 = AtomicU32::new(0);

// ── Panic tracking ──────────────────────────────────────────────────

pub static KEYBOARD_HOOK_PANICS: AtomicU64 = AtomicU64::new(0);
pub static MOUSE_HOOK_PANICS: AtomicU64 = AtomicU64::new(0);
pub static FOCUS_EVENT_PANICS: AtomicU64 = AtomicU64::new(0);
pub static MESSAGE_WINDOW_PANICS: AtomicU64 = AtomicU64::new(0);

// ── Message loop tracking ───────────────────────────────────────────

pub static GETMESSAGE_ERRORS_MAIN: AtomicU64 = AtomicU64::new(0);
pub static GETMESSAGE_ERRORS_MSG_WINDOW: AtomicU64 = AtomicU64::new(0);
pub static WM_QUIT_RECEIVED: AtomicU64 = AtomicU64::new(0);
pub static WM_CLOSE_RECEIVED: AtomicU64 = AtomicU64::new(0);

// ── SendMessageTimeout tracking ─────────────────────────────────────

pub static SENDMSG_TIMEOUTS: AtomicU64 = AtomicU64::new(0);
pub static SENDMSG_SUCCESS: AtomicU64 = AtomicU64::new(0);

// ── Helper functions ────────────────────────────────────────────────

/// Mark a hook thread as spawned. Called before `thread::spawn` in hook callbacks.
pub fn track_hook_thread_spawn() {
    HOOK_THREAD_SPAWNED.fetch_add(1, Ordering::Relaxed);
    let inflight = THREADS_IN_FLIGHT.fetch_add(1, Ordering::Relaxed) + 1;
    let prev_peak = THREAD_PEAK.load(Ordering::Relaxed);
    if inflight as u64 > prev_peak {
        THREAD_PEAK.store(inflight as u64, Ordering::Relaxed);
    }
}

/// Mark a hook thread as completed. Called when the timer thread exits.
pub fn track_hook_thread_done() {
    THREADS_IN_FLIGHT.fetch_sub(1, Ordering::Relaxed);
}

/// Mark a focus-sync thread as spawned.
pub fn track_focus_sync_spawn() {
    FOCUS_SYNC_THREAD_SPAWNED.fetch_add(1, Ordering::Relaxed);
    let inflight = THREADS_IN_FLIGHT.fetch_add(1, Ordering::Relaxed) + 1;
    let prev_peak = THREAD_PEAK.load(Ordering::Relaxed);
    if inflight as u64 > prev_peak {
        THREAD_PEAK.store(inflight as u64, Ordering::Relaxed);
    }
}

/// Mark a focus-sync thread as completed.
pub fn track_focus_sync_done() {
    THREADS_IN_FLIGHT.fetch_sub(1, Ordering::Relaxed);
}

/// Mark a mouse-sync thread as spawned.
pub fn track_mouse_sync_spawn() {
    MOUSE_SYNC_THREAD_SPAWNED.fetch_add(1, Ordering::Relaxed);
    let inflight = THREADS_IN_FLIGHT.fetch_add(1, Ordering::Relaxed) + 1;
    let prev_peak = THREAD_PEAK.load(Ordering::Relaxed);
    if inflight as u64 > prev_peak {
        THREAD_PEAK.store(inflight as u64, Ordering::Relaxed);
    }
}

/// Mark a mouse-sync thread as completed.
pub fn track_mouse_sync_done() {
    THREADS_IN_FLIGHT.fetch_sub(1, Ordering::Relaxed);
}

// ── Dump ─────────────────────────────────────────────────────────────

/// Print a diagnostic snapshot to stderr.
pub fn dbg_dump() {
    eprintln!("=== Capsense Debug Snapshot ===");
    eprintln!("Hook threads spawned:   {}", HOOK_THREAD_SPAWNED.load(Ordering::Relaxed));
    eprintln!("Focus sync spawned:     {}", FOCUS_SYNC_THREAD_SPAWNED.load(Ordering::Relaxed));
    eprintln!("Mouse sync spawned:     {}", MOUSE_SYNC_THREAD_SPAWNED.load(Ordering::Relaxed));
    eprintln!("Threads in flight:      {}", THREADS_IN_FLIGHT.load(Ordering::Relaxed));
    eprintln!("Thread peak:            {}", THREAD_PEAK.load(Ordering::Relaxed));
    eprintln!("---");
    eprintln!("Kbd hook panics:        {}", KEYBOARD_HOOK_PANICS.load(Ordering::Relaxed));
    eprintln!("Mouse hook panics:      {}", MOUSE_HOOK_PANICS.load(Ordering::Relaxed));
    eprintln!("Focus event panics:     {}", FOCUS_EVENT_PANICS.load(Ordering::Relaxed));
    eprintln!("Msg window panics:      {}", MESSAGE_WINDOW_PANICS.load(Ordering::Relaxed));
    eprintln!("---");
    eprintln!("GetMessage errors (main):     {}", GETMESSAGE_ERRORS_MAIN.load(Ordering::Relaxed));
    eprintln!("GetMessage errors (msg win):  {}", GETMESSAGE_ERRORS_MSG_WINDOW.load(Ordering::Relaxed));
    eprintln!("WM_QUIT received:             {}", WM_QUIT_RECEIVED.load(Ordering::Relaxed));
    eprintln!("WM_CLOSE received:            {}", WM_CLOSE_RECEIVED.load(Ordering::Relaxed));
    eprintln!("---");
    eprintln!("SendMsg timeouts:       {}", SENDMSG_TIMEOUTS.load(Ordering::Relaxed));
    eprintln!("SendMsg success:        {}", SENDMSG_SUCCESS.load(Ordering::Relaxed));
    eprintln!("===============================");
}

/// Run a checker that prints warnings when counters exceed safe thresholds.
/// Call periodically (e.g., every N key presses) from the hook callback.
pub fn dbg_check_sanity() {
    let inflight = THREADS_IN_FLIGHT.load(Ordering::Relaxed);
    let peak = THREAD_PEAK.load(Ordering::Relaxed);
    let panics = KEYBOARD_HOOK_PANICS.load(Ordering::Relaxed)
        + MOUSE_HOOK_PANICS.load(Ordering::Relaxed)
        + FOCUS_EVENT_PANICS.load(Ordering::Relaxed);
    let timeouts = SENDMSG_TIMEOUTS.load(Ordering::Relaxed);

    // Warn if threads are accumulating (indicating SendMessageTimeoutW is returning
    // but threads aren't exiting fast enough)
    if inflight > 50 {
        eprintln!(
            "WARNING: {} threads in flight (peak: {}) — possible thread leak",
            inflight, peak
        );
    }
    if panics > 0 {
        eprintln!("WARNING: {} hook callback panics caught", panics);
    }
    if timeouts > 100 {
        eprintln!(
            "WARNING: {} SendMessage timeouts — IME window may be hung",
            timeouts
        );
    }
}
