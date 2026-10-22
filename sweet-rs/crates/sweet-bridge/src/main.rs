//! Standalone `sweet-bridge` CLI (`adb`-style direct guest control & latency benchmark).
//!
//! Usage:
//!   sweet-bridge status
//!   sweet-bridge tap <x> <y>
//!   sweet-bridge swipe <x0> <y0> <x1> <y1> [steps]
//!   sweet-bridge key <key_id>
//!   sweet-bridge home
//!   sweet-bridge bench

use std::env;
use std::thread;
use std::time::{Duration, Instant};

use sweet_bridge::{dispatch_raw_input, MIN_TAP_HOLD_MS, SWAP_COUNT_PA};
use sweet_core::{
    direct_qemu_attachment_info, direct_qemu_read_fb, direct_qemu_read_u64, FB_BASE, FB_H, FB_W,
    INPUT_MBOX_PA,
};

fn print_status() {
    match direct_qemu_attachment_info() {
        Some((pid, base)) => {
            let swap_cnt = direct_qemu_read_u64(SWAP_COUNT_PA).unwrap_or(0);
            let mbox_hdr = direct_qemu_read_u64(INPUT_MBOX_PA).unwrap_or(0);
            let magic = (mbox_hdr & 0xffff_ffff) as u32;
            let w_idx = (mbox_hdr >> 32) as u32;
            let r_word = direct_qemu_read_u64(INPUT_MBOX_PA + 8).unwrap_or(0);
            let r_idx = (r_word & 0xffff_ffff) as u32;
            println!("SweetBridge Status: ATTACHED (Direct-RAM Zero-BQL)");
            println!("  QEMU Process PID : {}", pid);
            println!("  Host RAM Base VA : 0x{:016x}", base);
            println!("  Guest FB PA      : 0x{:08x} ({}x{} BGRA)", FB_BASE, FB_W, FB_H);
            println!("  Guest Swap Count : {} (PA 0x{:x})", swap_cnt, SWAP_COUNT_PA);
            println!(
                "  Input Mailbox    : 0x{:08x} (magic=0x{:08x}, w={}, r={})",
                INPUT_MBOX_PA, magic, w_idx, r_idx
            );
        }
        None => {
            println!("SweetBridge Status: UNATTACHED (waiting for qemu-system-aarch64.exe)");
        }
    }
}

fn run_bench() {
    print_status();
    let Some((_pid, _base)) = direct_qemu_attachment_info() else {
        println!("[!] Cannot run benchmark: QEMU VM is not running.");
        return;
    };

    let t_swap = Instant::now();
    for _ in 0..100 {
        let _ = direct_qemu_read_u64(SWAP_COUNT_PA);
    }
    let avg_swap_ns = t_swap.elapsed().as_nanos() / 100;

    let mut fb = vec![0u8; FB_W * FB_H * 4];
    let t_fb = Instant::now();
    let ok = direct_qemu_read_fb(&mut fb);
    let fb_us = t_fb.elapsed().as_micros();

    println!(
        "  8-byte Swap Poll : {} ns ({:.2} us)",
        avg_swap_ns,
        (avg_swap_ns as f64) / 1000.0
    );
    println!(
        "  10.36 MB FB Read : {} us ({:.2} ms, ok={})",
        fb_us,
        (fb_us as f64) / 1000.0,
        ok
    );
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("status");

    match cmd {
        "status" => print_status(),
        "bench" => run_bench(),
        "tap" => {
            let x: i32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(540);
            let y: i32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1200);
            let t0 = Instant::now();
            let ok1 = dispatch_raw_input(1, 0, true, x, y, None);
            let d_us = t0.elapsed().as_micros();
            thread::sleep(Duration::from_millis(MIN_TAP_HOLD_MS));
            let ok2 = dispatch_raw_input(1, 0, false, x, y, None);
            println!(
                "[SweetBridge] Tap ({}, {}) injected in {} us (direct={})",
                x,
                y,
                d_us,
                ok1 && ok2
            );
        }
        "swipe" => {
            let x0: i32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(540);
            let y0: i32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1800);
            let x1: i32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(540);
            let y1: i32 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(800);
            let steps: i32 = args
                .get(6)
                .and_then(|s| s.parse().ok())
                .unwrap_or(6)
                .max(2);
            for i in 0..steps {
                let px = x0 + ((x1 - x0) * i) / (steps - 1);
                let py = y0 + ((y1 - y0) * i) / (steps - 1);
                dispatch_raw_input(1, 0, true, px, py, None);
                thread::sleep(Duration::from_millis(12));
            }
            dispatch_raw_input(1, 0, false, x1, y1, None);
            println!(
                "[SweetBridge] Swipe ({}, {}) -> ({}, {}) ({} steps) completed",
                x0, y0, x1, y1, steps
            );
        }
        "home" => {
            for &sy in &[2385, 2240, 2020, 1760, 1480, 1220] {
                dispatch_raw_input(1, 0, true, 540, sy, None);
                thread::sleep(Duration::from_millis(12));
            }
            dispatch_raw_input(1, 0, false, 540, 1220, None);
            println!("[SweetBridge] Home swipe gesture completed");
        }
        "key" => {
            let key_id: u16 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            dispatch_raw_input(2, key_id, true, 0, 0, None);
            thread::sleep(Duration::from_millis(40));
            dispatch_raw_input(2, key_id, false, 0, 0, None);
            println!("[SweetBridge] Key {} press completed", key_id);
        }
        _ => {
            println!("Usage: sweet-bridge <status|bench|tap X Y|swipe X0 Y0 X1 Y1 [steps]|home|key ID>");
        }
    }
}
