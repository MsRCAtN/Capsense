//! Stress tests for reproducing the 48-hour exit bug.
//!
//! Simulates:
//! 1. Long-running focus change pattern (~equivalent to hours of usage)
//! 2. Rapid thread spawning with hung IME windows
//! 3. Memory/handle leak detection over extended simulated uptime

use std::ptr::null_mut;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW,
    SendMessageTimeoutW, SMTO_NORMAL, WM_USER, WNDCLASSW,
};

// ── Window helpers ──────────────────────────────────────────────────────────

fn encode_wide(s: &str) -> Vec<u16> {
    let mut res: Vec<u16> = s.encode_utf16().collect();
    res.push(0);
    res
}

fn create_hung_window(name: &str) -> HWND {
    let class_name = encode_wide(name);
    let h_instance = unsafe { GetModuleHandleW(null_mut()) };
    let wnd_class = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(DefWindowProcW),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: h_instance,
        hIcon: 0,
        hCursor: 0,
        hbrBackground: 0,
        lpszMenuName: null_mut(),
        lpszClassName: class_name.as_ptr(),
    };
    unsafe { RegisterClassW(&wnd_class) };

    unsafe {
        CreateWindowExW(
            0, class_name.as_ptr(), null_mut(), 0,
            0, 0, 0, 0, 0, 0, h_instance, null_mut(),
        )
    }
}

// ── Test: Simulated 48-hour usage pattern ───────────────────────────────────

/// Simulates ~48 hours of focus changes at an accelerated rate.
/// Each "hour" is compressed to 200ms (200ms * 48 = 9.6s total test time).
/// Focus changes happen at ~10/second (high but realistic during active use).
#[test]
fn test_simulated_extended_uptime() {
    // Create a few "hung" windows to simulate unresponsive IME windows
    let mut hung_windows: Vec<HWND> = Vec::new();
    for i in 0..5 {
        let hwnd = create_hung_window(&format!("StressHungWin{}", i));
        assert_ne!(hwnd, 0, "Failed to create hung window {}", i);
        hung_windows.push(hwnd);
    }

    let spawned = Arc::new(AtomicU64::new(0));
    let completed = Arc::new(AtomicU64::new(0));
    let timeouts = Arc::new(AtomicU64::new(0));
    let inflight = Arc::new(AtomicU32::new(0));
    let peak_inflight = Arc::new(AtomicU32::new(0));

    // Simulate 48 "hours", each compressed to 200ms
    let simulated_hours = 48;
    let ms_per_hour = 200u64;

    let start = Instant::now();

    for _hour in 0..simulated_hours {
        let batch_start = Instant::now();

        // Spawn focus-sync threads at ~10/second for this "hour" (200ms = 2 threads)
        // But to stress test: spawn 20 in rapid succession per hour
        let mut handles = Vec::new();
        for _ in 0..20 {
            let hwnd = hung_windows[(_hour as usize) % hung_windows.len()];
            let s = Arc::clone(&spawned);
            let c = Arc::clone(&completed);
            let t = Arc::clone(&timeouts);
            let inf = Arc::clone(&inflight);
            let pk = Arc::clone(&peak_inflight);

            let cur = inf.fetch_add(1, Ordering::SeqCst) + 1;
            s.fetch_add(1, Ordering::SeqCst);

            // Track peak
            let mut prev = pk.load(Ordering::SeqCst);
            while cur > prev {
                match pk.compare_exchange(prev, cur, Ordering::SeqCst, Ordering::SeqCst) {
                    Ok(_) => break,
                    Err(actual) => prev = actual,
                }
            }

            let h = thread::spawn(move || {
                // Simulate the 50ms delay in schedule_chinese_ime_mode_sync
                thread::sleep(Duration::from_millis(10));

                let mut result: usize = 0;
                let ret = unsafe {
                    SendMessageTimeoutW(
                        hwnd,
                        WM_USER + 1,
                        0,
                        0,
                        SMTO_NORMAL,
                        50, // 50ms timeout (shorter than real to speed up test)
                        &mut result,
                    )
                };

                if ret == 0 {
                    t.fetch_add(1, Ordering::SeqCst);
                }

                inf.fetch_sub(1, Ordering::SeqCst);
                c.fetch_add(1, Ordering::SeqCst);
            });
            handles.push(h);
        }

        // Wait for all threads in this batch to complete
        for h in handles {
            let _ = h.join();
        }

        // Simulate the idle time between focus change bursts
        let elapsed_this_hour = batch_start.elapsed();
        if elapsed_this_hour < Duration::from_millis(ms_per_hour) {
            thread::sleep(Duration::from_millis(ms_per_hour) - elapsed_this_hour);
        }
    }

    let total_elapsed = start.elapsed();

    // Additional wait for any stragglers
    thread::sleep(Duration::from_millis(500));

    let s = spawned.load(Ordering::SeqCst);
    let c = completed.load(Ordering::SeqCst);
    let t = timeouts.load(Ordering::SeqCst);
    let inf = inflight.load(Ordering::SeqCst);
    let peak = peak_inflight.load(Ordering::SeqCst);

    eprintln!("=== Stress test results ===");
    eprintln!("Total elapsed: {:?}", total_elapsed);
    eprintln!("Threads spawned:  {}", s);
    eprintln!("Threads completed: {}", c);
    eprintln!("SendMsg timeouts:  {}", t);
    eprintln!("In-flight now:     {}", inf);
    eprintln!("Peak in-flight:    {}", peak);

    // All spawned threads must have completed
    assert_eq!(s, c, "{} threads spawned but {} completed — leak detected!", s, c);

    // In-flight count must be zero
    assert_eq!(inf, 0, "{} threads still in flight after test", inf);

    // Peak concurrent threads should be reasonable (under 100 even with 20/interval)
    assert!(peak <= 100, "Peak in-flight threads was {} — should be under 100", peak);

    // All calls should have timed out (since windows are hung)
    assert_eq!(t, s, "Expected all {} calls to timeout, got {} timeouts", s, t);

    // Cleanup
    for hwnd in hung_windows {
        unsafe { DestroyWindow(hwnd) };
    }
}

// ── Test: Thread accumulation with rapidly-spawning short-lived threads ─────

#[test]
fn test_no_thread_accumulation_under_load() {
    // Spawn 1000 threads rapidly, each living for 10-50ms
    // Verify that all threads complete and no resources leak

    let total = 1000;
    let done = Arc::new(AtomicU32::new(0));

    let mut handles = Vec::with_capacity(total);
    for i in 0..total {
        let d = Arc::clone(&done);
        let h = thread::spawn(move || {
            thread::sleep(Duration::from_millis((i % 40 + 10) as u64));
            d.fetch_add(1, Ordering::SeqCst);
        });
        handles.push(h);

        // Don't let the handle list grow too large — join completed ones periodically
        if handles.len() >= 100 {
            let mut new_handles = Vec::new();
            for h in handles.drain(..) {
                if h.is_finished() {
                    let _ = h.join();
                } else {
                    new_handles.push(h);
                }
            }
            handles = new_handles;
        }
    }

    // Wait for remaining
    for h in handles {
        let _ = h.join();
    }

    assert_eq!(done.load(Ordering::SeqCst), total as u32, "Not all threads completed");
}

// ── Test: SendMessageTimeoutW latency under load ────────────────────────────

#[test]
fn test_sendmessage_timeout_latency() {
    let hwnd = create_hung_window("StressLatencyWin");
    assert_ne!(hwnd, 0, "Failed to create test window");

    let mut total_latency = Duration::ZERO;
    let iterations = 100;

    for _ in 0..iterations {
        let start = Instant::now();
        let mut result: usize = 0;
        unsafe {
            SendMessageTimeoutW(
                hwnd,
                WM_USER + 1,
                0,
                0,
                SMTO_NORMAL,
                100,
                &mut result,
            );
        }
        total_latency += start.elapsed();
    }

    let avg_latency = total_latency / iterations as u32;

    eprintln!("Average SendMessageTimeoutW latency: {:?} over {} calls", avg_latency, iterations);

    // Average latency should be under 200ms (the timeout value)
    assert!(
        avg_latency < Duration::from_millis(200),
        "Average latency {:?} exceeds expected ceiling",
        avg_latency
    );

    unsafe { DestroyWindow(hwnd) };
}

// ── Test: Memory stability under extended SendMessageTimeoutW calls ─────────

#[test]
fn test_memory_stability() {
    // This is an approximate test. We track that the program doesn't crash
    // and that peak thread count stays bounded under sustained load.

    let hwnd = create_hung_window("StressMemWin");
    assert_ne!(hwnd, 0, "Failed to create test window");

    let rounds = 10;
    let calls_per_round = 50;

    for _round in 0..rounds {
        let mut handles = Vec::new();
        for _ in 0..calls_per_round {
            let h = thread::spawn(move || {
                thread::sleep(Duration::from_millis(5));
                let mut result: usize = 0;
                unsafe {
                    SendMessageTimeoutW(
                        hwnd,
                        WM_USER + 1,
                        0,
                        0,
                        SMTO_NORMAL,
                        50,
                        &mut result,
                    );
                }
            });
            handles.push(h);
        }
        for h in handles {
            let _ = h.join();
        }
        // Brief pause between rounds
        thread::sleep(Duration::from_millis(20));
    }

    // If we got here without crashing, the test passes.
    // The real test is running this under a memory profiler (e.g., Dr. Memory)
    // to detect handle leaks that aren't visible from user-space Rust.

    unsafe { DestroyWindow(hwnd) };
}
