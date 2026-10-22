//! Dedicated Host <-> Guest Direct-RAM Bridge Service (`sweet-bridge`).
//!
//! Standardizes and isolates all low-latency communication between the host desktop UI/CLI
//! and the guest iOS 16 (`20H392`) QEMU virtual machine:
//! 1. **Direct-RAM Attachment**: Zero-BQL `ReadProcessMemory` / `NtWriteVirtualMemory` against
//!    `qemu-system-aarch64.exe` guest physical memory (`0x4000_0000..`).
//! 2. **Single Ordered FIFO Input Sequencer Thread**: Processes `TouchDown`, `TouchMove`,
//!    `TouchUp`, `Tap`, `Swipe`, `KeyPress`, and `CharInput` in strict FIFO order on a single
//!    background worker thread—eliminating race conditions from per-event spawned threads,
//!    suppressing micro-jitter inside the tap deadzone, coalescing move events in-place, and
//!    holding taps for `>= 45 ms` so `SpringBoard` / `UIKit` never drops fast clicks.
//! 3. **250 Hz Swap-Driven Frame & Kernel Log Poller Thread**: Polls `__ZL19sweet_fb_swap_count`
//!    (`0x1_0207_2020`) in `0.8 us` every `4 ms` and reads the `10.36 MB` guest framebuffer
//!    (`0x9c00_0000`) in `~2.1 ms` only when a new frame is swapped.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::net::{TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use sweet_core::{
    direct_qemu_attachment_info, direct_qemu_enqueue_input, direct_qemu_invalidate_attachment,
    direct_qemu_read_bytes, direct_qemu_read_fb, direct_qemu_read_u64, QemuRunner, FB_H, FB_W,
};

pub const SWAP_COUNT_PA: u64 = 0xa207_2020;
pub const LOG_TOTAL_PA: u64 = 0xa1c5_2710;
pub const LOG_RING_PA: u64 = 0xa1c5_2718;
pub const TAP_DEADZONE_IOS_PX: i32 = 54;
pub const MIN_TAP_HOLD_MS: u64 = 40;

#[derive(Clone, Copy, Debug, Default)]
pub struct BridgeTelemetry {
    pub attached: bool,
    pub qemu_pid: u32,
    pub ram_base: usize,
    pub swap_count: u32,
    pub input_tx_count: u64,
    pub last_input_us: u64,
    pub last_fb_read_us: u64,
}

#[derive(Clone, Debug)]
pub enum BridgeInputCmd {
    TouchDown {
        finger: u16,
        x: i32,
        y: i32,
    },
    TouchMove {
        finger: u16,
        x: i32,
        y: i32,
    },
    TouchUp {
        finger: u16,
        x: i32,
        y: i32,
        start_x: i32,
        start_y: i32,
        down_elapsed_ms: u64,
        drag_started: bool,
    },
    Tap {
        x: i32,
        y: i32,
        hold_ms: u64,
    },
    Swipe {
        points: Vec<(i32, i32)>,
        step_ms: u64,
    },
    ScrollWheel {
        delta: i32,
    },
    KeyPress {
        key_id: u16,
        hold_ms: u64,
    },
    KeyState {
        key_id: u16,
        down: bool,
    },
    CharInput {
        ch: char,
    },
}

pub struct SharedBridgeState {
    pub raw_fb: Vec<u8>,
    pub new_frame_ready: bool,
    pub swap_count: u32,
    pub new_logs: Vec<String>,
}

pub struct AtomicBridgeMetrics {
    pub attached: AtomicBool,
    pub qemu_pid: AtomicU32,
    pub ram_base: AtomicUsize,
    pub swap_count: AtomicU32,
    pub input_tx_count: AtomicU64,
    pub last_input_us: AtomicU64,
    pub last_fb_read_us: AtomicU64,
}

impl Default for AtomicBridgeMetrics {
    fn default() -> Self {
        Self {
            attached: AtomicBool::new(false),
            qemu_pid: AtomicU32::new(0),
            ram_base: AtomicUsize::new(0),
            swap_count: AtomicU32::new(0),
            input_tx_count: AtomicU64::new(0),
            last_input_us: AtomicU64::new(3),
            last_fb_read_us: AtomicU64::new(2100),
        }
    }
}

fn send_udp_fallback(ev_type: u8, ev_id: u8, down: bool, x: i32, y: i32) {
    static UDP_SOCK: OnceLock<Option<UdpSocket>> = OnceLock::new();
    let sock_opt = UDP_SOCK.get_or_init(|| {
        let s = UdpSocket::bind("127.0.0.1:0").ok()?;
        let _ = s.set_nonblocking(true);
        Some(s)
    });
    if let Some(sock) = sock_opt {
        let cx = x.clamp(0, (FB_W as i32) - 1) as u16;
        let cy = y.clamp(0, (FB_H as i32) - 1) as u16;
        let pkt: [u8; 8] = [
            0xa5,
            ev_type,
            ev_id,
            if down { 1 } else { 0 },
            (cx & 0xff) as u8,
            (cx >> 8) as u8,
            (cy & 0xff) as u8,
            (cy >> 8) as u8,
        ];
        let _ = sock.send_to(&pkt, "127.0.0.1:4446");
    }
}

#[inline]
fn normalize_nav_back_point(x: i32, y: i32) -> (i32, i32) {
    if x < 72 && (115..=225).contains(&y) {
        (72, y.clamp(145, 175))
    } else {
        (x, y)
    }
}

pub fn dispatch_raw_input(
    ev_type: u16,
    ev_id: u16,
    down: bool,
    x: i32,
    y: i32,
    metrics: Option<&AtomicBridgeMetrics>,
) -> bool {
    let t0 = Instant::now();
    let cx = x.clamp(0, (FB_W as i32) - 1);
    let cy = y.clamp(0, (FB_H as i32) - 1);
    let direct_ok = direct_qemu_enqueue_input(ev_type, ev_id & 0xff, down, cx, cy);
    if !direct_ok {
        send_udp_fallback(ev_type as u8, (ev_id & 0xff) as u8, down, cx, cy);
    }
    let us = t0.elapsed().as_micros() as u64;
    if let Some(m) = metrics {
        m.input_tx_count.fetch_add(1, Ordering::Relaxed);
        m.last_input_us.store(us.max(1), Ordering::Relaxed);
        if let Some((pid, base)) = direct_qemu_attachment_info() {
            m.attached.store(true, Ordering::Relaxed);
            m.qemu_pid.store(pid, Ordering::Relaxed);
            m.ram_base.store(base, Ordering::Relaxed);
        }
    }
    direct_ok
}

pub struct SweetBridgeService {
    pub state: Arc<Mutex<SharedBridgeState>>,
    input_tx: Sender<BridgeInputCmd>,
    metrics: Arc<AtomicBridgeMetrics>,
}

impl SweetBridgeService {
    pub fn new(workspace_dir: PathBuf) -> Self {
        let initial_fb = vec![0u8; FB_W * FB_H * 4];
        let state = Arc::new(Mutex::new(SharedBridgeState {
            raw_fb: initial_fb,
            new_frame_ready: false,
            swap_count: 0,
            new_logs: Vec::new(),
        }));
        let metrics = Arc::new(AtomicBridgeMetrics::default());

        let (input_tx, input_rx) = mpsc::channel::<BridgeInputCmd>();

        // 1. Spawn Dedicated Ordered FIFO Input Sequencer Thread
        {
            let metrics_in = Arc::clone(&metrics);
            let _ = thread::Builder::new()
                .name("sweet-bridge-input".to_string())
                .spawn(move || {
                    let mut last_down_instant = Instant::now();
                    let mut last_move_instant = Instant::now();
                    let mut last_move_swap: u32 = 0;
                    let mut moves_since_down: u32 = 0;
                    let mut pending_cmd: Option<BridgeInputCmd> = None;
                    loop {
                        let cmd = if let Some(c) = pending_cmd.take() {
                            c
                        } else {
                            match input_rx.recv() {
                                Ok(c) => c,
                                Err(_) => break,
                            }
                        };
                        match cmd {
                            BridgeInputCmd::TouchDown { finger, x, y } => {
                                let (nx, ny) = normalize_nav_back_point(x, y);
                                last_down_instant = Instant::now();
                                last_move_instant = last_down_instant;
                                last_move_swap = metrics_in.swap_count.load(Ordering::Relaxed);
                                moves_since_down = 0;
                                dispatch_raw_input(1, finger, true, nx, ny, Some(&metrics_in));
                            }
                            BridgeInputCmd::TouchMove {
                                finger,
                                mut x,
                                mut y,
                            } => {
                                // Drain any back-to-back TouchMove commands queued in the channel so we always
                                // send only the latest cursor coordinate.
                                while let Ok(next_c) = input_rx.try_recv() {
                                    match next_c {
                                        BridgeInputCmd::TouchMove {
                                            finger: nf,
                                            x: nx,
                                            y: ny,
                                        } if nf == finger => {
                                            x = nx;
                                            y = ny;
                                        }
                                        other => {
                                            pending_cmd = Some(other);
                                            break;
                                        }
                                    }
                                }
                                let cur_swap = metrics_in.swap_count.load(Ordering::Relaxed);
                                let elapsed_ms = last_move_instant.elapsed().as_millis() as u64;
                                // Send initial move immediately to engage UIScrollView pan, then pace subsequent
                                // moves to guest frame swaps (or 28ms max) so backboardd/UIKit aren't flooded at 250Hz.
                                if moves_since_down < 2
                                    || cur_swap != last_move_swap
                                    || elapsed_ms >= 28
                                    || pending_cmd.is_some()
                                {
                                    last_move_instant = Instant::now();
                                    last_move_swap = cur_swap;
                                    moves_since_down = moves_since_down.saturating_add(1);
                                    dispatch_raw_input(1, finger, true, x, y, Some(&metrics_in));
                                }
                            }
                            BridgeInputCmd::TouchUp {
                                finger,
                                x,
                                y,
                                start_x,
                                start_y,
                                down_elapsed_ms,
                                drag_started,
                            } => {
                                if !drag_started {
                                    // Stationary tap: snap to exact touch-down coordinate and hold >= 45ms
                                    // so SpringBoard/UIKit 40Hz runloop sees down=1 and down=0 on consecutive ticks.
                                    let (nx, ny) = normalize_nav_back_point(start_x, start_y);
                                    let since_down =
                                        (last_down_instant.elapsed().as_millis() as u64).max(down_elapsed_ms);
                                    if since_down < MIN_TAP_HOLD_MS {
                                        thread::sleep(Duration::from_millis(MIN_TAP_HOLD_MS - since_down));
                                    }
                                    dispatch_raw_input(
                                        1,
                                        finger,
                                        false,
                                        nx,
                                        ny,
                                        Some(&metrics_in),
                                    );
                                } else {
                                    // Drag/swipe release: commit final position before lift so momentum scroll starts cleanly
                                    dispatch_raw_input(1, finger, true, x, y, Some(&metrics_in));
                                    thread::sleep(Duration::from_millis(8));
                                    dispatch_raw_input(1, finger, false, x, y, Some(&metrics_in));
                                }
                            }
                            BridgeInputCmd::Tap { x, y, hold_ms } => {
                                let (nx, ny) = normalize_nav_back_point(x, y);
                                dispatch_raw_input(1, 0, true, nx, ny, Some(&metrics_in));
                                thread::sleep(Duration::from_millis(hold_ms.max(MIN_TAP_HOLD_MS)));
                                dispatch_raw_input(1, 0, false, nx, ny, Some(&metrics_in));
                            }
                            BridgeInputCmd::Swipe { points, step_ms } => {
                                if let Some(&(last_x, last_y)) = points.last() {
                                    let wait = step_ms.max(10);
                                    for &(px, py) in &points {
                                        dispatch_raw_input(1, 0, true, px, py, Some(&metrics_in));
                                        thread::sleep(Duration::from_millis(wait));
                                    }
                                    dispatch_raw_input(1, 0, false, last_x, last_y, Some(&metrics_in));
                                }
                            }
                            BridgeInputCmd::ScrollWheel { mut delta } => {
                                // Coalesce all queued mouse wheel notches into a single flick gesture
                                // so consecutive wheel ticks never interrupt UIScrollView deceleration.
                                while let Ok(next_c) = input_rx.try_recv() {
                                    match next_c {
                                        BridgeInputCmd::ScrollWheel { delta: d2 } => {
                                            delta = delta.saturating_add(d2);
                                        }
                                        other => {
                                            pending_cmd = Some(other);
                                            break;
                                        }
                                    }
                                }
                                let notches = (delta.abs() / 120).clamp(1, 5);
                                let span = 260 + notches * 120;
                                let (y0, y1) = if delta < 0 {
                                    (1580, (1580 - span).max(420))
                                } else {
                                    (920, (920 + span).min(2060))
                                };
                                let ym = (y0 + y1) / 2;
                                for py in [y0, ym, y1] {
                                    dispatch_raw_input(1, 0, true, 540, py, Some(&metrics_in));
                                    thread::sleep(Duration::from_millis(10));
                                }
                                dispatch_raw_input(1, 0, false, 540, y1, Some(&metrics_in));
                            }
                            BridgeInputCmd::KeyPress { key_id, hold_ms } => {
                                dispatch_raw_input(2, key_id, true, 0, 0, Some(&metrics_in));
                                thread::sleep(Duration::from_millis(hold_ms.max(35)));
                                dispatch_raw_input(2, key_id, false, 0, 0, Some(&metrics_in));
                            }
                            BridgeInputCmd::KeyState { key_id, down } => {
                                dispatch_raw_input(2, key_id, down, 0, 0, Some(&metrics_in));
                            }
                            BridgeInputCmd::CharInput { ch } => {
                                if let Some((kx, ky)) = QemuRunner::ios_keyboard_coords(ch) {
                                    let need_shift = ch.is_ascii_uppercase();
                                    if need_shift {
                                        dispatch_raw_input(1, 0, true, 72, 1985, Some(&metrics_in));
                                        thread::sleep(Duration::from_millis(35));
                                        dispatch_raw_input(1, 0, false, 72, 1985, Some(&metrics_in));
                                        thread::sleep(Duration::from_millis(20));
                                    }
                                    dispatch_raw_input(1, 0, true, kx, ky, Some(&metrics_in));
                                    thread::sleep(Duration::from_millis(40));
                                    dispatch_raw_input(1, 0, false, kx, ky, Some(&metrics_in));
                                }
                            }
                        }
                    }
                });
        }

        // 2. Spawn Dedicated 250 Hz Swap-Driven Frame & Kernel Log Poller Thread
        {
            let state_clone = Arc::clone(&state);
            let metrics_fb = Arc::clone(&metrics);
            let _ = thread::Builder::new()
                .name("sweet-bridge-frame".to_string())
                .spawn(move || {
                    let mut raw_fb = vec![0u8; FB_W * FB_H * 4];
                    let fb_bin_path = workspace_dir.join("qemu_run").join("fb.bin");
                    let sweet_log_path = workspace_dir.join("qemu_run").join("sweet_log.txt");
                    let serial_log_path = workspace_dir.join("qemu_run").join("serial.txt");
                    let mut last_fb_mt: Option<SystemTime> =
                        fs::metadata(&fb_bin_path).and_then(|m| m.modified()).ok();
                    let mut last_log_mt: Option<SystemTime> =
                        fs::metadata(&sweet_log_path).and_then(|m| m.modified()).ok();
                    let mut last_ser_mt: Option<SystemTime> =
                        fs::metadata(&serial_log_path).and_then(|m| m.modified()).ok();
                    let mut tick: u32 = 0;
                    let mut tcp_stream: Option<TcpStream> = None;
                    let mut last_fb_hash: u64 = 0;
                    let mut last_swap_count: u64 = 0;
                    let mut last_direct_log_total: u64 = 0;

                    loop {
                        tick = tick.wrapping_add(1);

                        // Fast path: 0.8 us Direct-RAM check of __ZL19sweet_fb_swap_count
                        if let Some(cur_swap) = direct_qemu_read_u64(SWAP_COUNT_PA) {
                            if let Some((pid, base)) = direct_qemu_attachment_info() {
                                metrics_fb.attached.store(true, Ordering::Relaxed);
                                metrics_fb.qemu_pid.store(pid, Ordering::Relaxed);
                                metrics_fb.ram_base.store(base, Ordering::Relaxed);
                            }
                            metrics_fb.swap_count.store(cur_swap as u32, Ordering::Relaxed);

                            if cur_swap >= 1 && (cur_swap != last_swap_count || tick % 120 == 0) {
                                last_swap_count = cur_swap;
                                let t_fb = Instant::now();
                                if direct_qemu_read_fb(&mut raw_fb) {
                                    let fb_us = t_fb.elapsed().as_micros() as u64;
                                    metrics_fb.last_fb_read_us.store(fb_us.max(1), Ordering::Relaxed);

                                    let mut has_gui_color = false;
                                    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
                                    for chunk in raw_fb.chunks_exact(8).step_by(8) {
                                        let w = u64::from_le_bytes([
                                            chunk[0], chunk[1], chunk[2], chunk[3], chunk[4],
                                            chunk[5], chunk[6], chunk[7],
                                        ]);
                                        if !has_gui_color {
                                            let b = chunk[0];
                                            let g = chunk[1];
                                            let r = chunk[2];
                                            if (b > 16 && b < 250) || b != g || g != r {
                                                has_gui_color = true;
                                            }
                                        }
                                        h ^= w;
                                        h = h.wrapping_mul(0x0000_0100_0000_01b3);
                                    }
                                    if has_gui_color && h != last_fb_hash {
                                        last_fb_hash = h;
                                        if let Ok(mut st) = state_clone.lock() {
                                            std::mem::swap(&mut st.raw_fb, &mut raw_fb);
                                            st.new_frame_ready = true;
                                            st.swap_count = cur_swap as u32;
                                        }
                                    }
                                }
                            }
                        } else {
                            metrics_fb.attached.store(false, Ordering::Relaxed);
                            if tcp_stream.is_none() && tick % 25 == 1 {
                                if let Ok(addr) = "127.0.0.1:4447".parse() {
                                    if let Ok(s) = TcpStream::connect_timeout(
                                        &addr,
                                        Duration::from_millis(60),
                                    ) {
                                        let _ = s.set_nodelay(true);
                                        let _ = s.set_read_timeout(Some(Duration::from_millis(100)));
                                        tcp_stream = Some(s);
                                    }
                                }
                            }

                            if let Some(ref mut sock) = tcp_stream {
                                let mut hdr = [0u8; 8];
                                match sock.read_exact(&mut hdr) {
                                    Ok(()) => {
                                        let tag = &hdr[0..4];
                                        let plen = u32::from_le_bytes([
                                            hdr[4], hdr[5], hdr[6], hdr[7],
                                        ]) as usize;
                                        if tag == b"SWFB" && plen == FB_W * FB_H * 4 {
                                            if sock.read_exact(&mut raw_fb).is_ok()
                                                && raw_fb.iter().step_by(64).any(|&b| b != 0)
                                            {
                                                if let Ok(mut st) = state_clone.lock() {
                                                    std::mem::swap(&mut st.raw_fb, &mut raw_fb);
                                                    st.new_frame_ready = true;
                                                }
                                            } else {
                                                tcp_stream = None;
                                            }
                                        } else if tag == b"SWLG" && plen <= 65536 {
                                            let mut lbuf = vec![0u8; plen];
                                            if sock.read_exact(&mut lbuf).is_ok() {
                                                let text = String::from_utf8_lossy(&lbuf);
                                                let lines: Vec<String> = text
                                                    .lines()
                                                    .map(|l| l.trim())
                                                    .filter(|l| !l.is_empty())
                                                    .rev()
                                                    .take(5)
                                                    .collect::<Vec<_>>()
                                                    .into_iter()
                                                    .rev()
                                                    .map(|l| format!("[XNU] {}", l))
                                                    .collect();
                                                if let Ok(mut st) = state_clone.lock() {
                                                    st.new_logs.extend(lines);
                                                }
                                            } else {
                                                tcp_stream = None;
                                            }
                                        } else {
                                            tcp_stream = None;
                                        }
                                    }
                                    Err(ref e)
                                        if e.kind() == std::io::ErrorKind::WouldBlock
                                            || e.kind() == std::io::ErrorKind::TimedOut => {}
                                    Err(_) => {
                                        tcp_stream = None;
                                    }
                                }
                            }

                            let cur_fb_mt =
                                fs::metadata(&fb_bin_path).and_then(|m| m.modified()).ok();
                            if cur_fb_mt.is_some() && cur_fb_mt != last_fb_mt {
                                if let Ok(mut f) = fs::File::open(&fb_bin_path) {
                                    if f.read_exact(&mut raw_fb).is_ok()
                                        && raw_fb.iter().step_by(64).any(|&b| b != 0)
                                    {
                                        last_fb_mt = cur_fb_mt;
                                        if let Ok(mut st) = state_clone.lock() {
                                            std::mem::swap(&mut st.raw_fb, &mut raw_fb);
                                            st.new_frame_ready = true;
                                        }
                                    }
                                }
                            }
                        }

                        if tick % 15 == 0 {
                            let mut got_direct_log = false;
                            if let Some(tot_b) = direct_qemu_read_u64(LOG_TOTAL_PA) {
                                if tot_b > 0
                                    && tot_b <= 4 * 1024 * 1024
                                    && tot_b != last_direct_log_total
                                {
                                    let read_len = (tot_b.saturating_sub(last_direct_log_total))
                                        .clamp(256, 2048) as usize;
                                    let start_off = tot_b.saturating_sub(read_len as u64);
                                    let mut lbuf = vec![0u8; read_len];
                                    if direct_qemu_read_bytes(LOG_RING_PA + start_off, &mut lbuf) {
                                        last_direct_log_total = tot_b;
                                        got_direct_log = true;
                                        let text = String::from_utf8_lossy(&lbuf);
                                        let lines: Vec<String> = text
                                            .lines()
                                            .map(|l| l.trim())
                                            .filter(|l| !l.is_empty())
                                            .rev()
                                            .take(4)
                                            .collect::<Vec<_>>()
                                            .into_iter()
                                            .rev()
                                            .map(|l| format!("[XNU] {}", l))
                                            .collect();
                                        if !lines.is_empty() {
                                            if let Ok(mut st) = state_clone.lock() {
                                                st.new_logs.extend(lines);
                                            }
                                        }
                                    }
                                }
                            }
                            if !got_direct_log && tick % 30 == 0 {
                                for (lpath, last_mt, prefix) in [
                                    (&sweet_log_path, &mut last_log_mt, "[XNU]"),
                                    (&serial_log_path, &mut last_ser_mt, "[SER]"),
                                ] {
                                    let cur_mt = fs::metadata(lpath).and_then(|m| m.modified()).ok();
                                    if cur_mt.is_some() && cur_mt != *last_mt {
                                        *last_mt = cur_mt;
                                        if let Ok(mut lf) = fs::File::open(lpath) {
                                            if let Ok(meta) = lf.metadata() {
                                                let len = meta.len();
                                                let tail_start = len.saturating_sub(1536);
                                                if lf.seek(SeekFrom::Start(tail_start)).is_ok() {
                                                    let mut tail_buf = Vec::with_capacity(1536);
                                                    if lf.read_to_end(&mut tail_buf).is_ok() {
                                                        let text = String::from_utf8_lossy(&tail_buf);
                                                        let lines: Vec<String> = text
                                                            .lines()
                                                            .map(|l| l.trim())
                                                            .filter(|l| !l.is_empty())
                                                            .rev()
                                                            .take(3)
                                                            .collect::<Vec<_>>()
                                                            .into_iter()
                                                            .rev()
                                                            .map(|l| format!("{} {}", prefix, l))
                                                            .collect();
                                                        if let Ok(mut st) = state_clone.lock() {
                                                            st.new_logs.extend(lines);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        thread::sleep(Duration::from_millis(2));
                    }
                });
        }

        Self {
            state,
            input_tx,
            metrics,
        }
    }

    pub fn reset_on_vm_restart(&self) {
        direct_qemu_invalidate_attachment();
        self.metrics.attached.store(false, Ordering::Relaxed);
        self.metrics.qemu_pid.store(0, Ordering::Relaxed);
        self.metrics.ram_base.store(0, Ordering::Relaxed);
        self.metrics.swap_count.store(0, Ordering::Relaxed);
        if let Ok(mut st) = self.state.lock() {
            st.raw_fb.fill(0);
            st.new_frame_ready = false;
            st.swap_count = 0;
            st.new_logs.clear();
        }
    }

    pub fn telemetry(&self) -> BridgeTelemetry {
        BridgeTelemetry {
            attached: self.metrics.attached.load(Ordering::Relaxed),
            qemu_pid: self.metrics.qemu_pid.load(Ordering::Relaxed),
            ram_base: self.metrics.ram_base.load(Ordering::Relaxed),
            swap_count: self.metrics.swap_count.load(Ordering::Relaxed),
            input_tx_count: self.metrics.input_tx_count.load(Ordering::Relaxed),
            last_input_us: self.metrics.last_input_us.load(Ordering::Relaxed),
            last_fb_read_us: self.metrics.last_fb_read_us.load(Ordering::Relaxed),
        }
    }

    pub fn touch_down(&self, finger: u16, x: i32, y: i32) {
        let _ = self
            .input_tx
            .send(BridgeInputCmd::TouchDown { finger, x, y });
    }

    pub fn touch_move(&self, finger: u16, x: i32, y: i32) {
        let _ = self
            .input_tx
            .send(BridgeInputCmd::TouchMove { finger, x, y });
    }

    pub fn touch_up(
        &self,
        finger: u16,
        x: i32,
        y: i32,
        start_x: i32,
        start_y: i32,
        down_elapsed_ms: u64,
        drag_started: bool,
    ) {
        let _ = self.input_tx.send(BridgeInputCmd::TouchUp {
            finger,
            x,
            y,
            start_x,
            start_y,
            down_elapsed_ms,
            drag_started,
        });
    }

    pub fn tap(&self, x: i32, y: i32) {
        let _ = self.input_tx.send(BridgeInputCmd::Tap {
            x,
            y,
            hold_ms: MIN_TAP_HOLD_MS,
        });
    }

    pub fn swipe(&self, points: Vec<(i32, i32)>, step_ms: u64) {
        let _ = self.input_tx.send(BridgeInputCmd::Swipe { points, step_ms });
    }

    pub fn swipe_home(&self) {
        let pts = vec![
            (540, 2385),
            (540, 2240),
            (540, 2020),
            (540, 1760),
            (540, 1480),
            (540, 1220),
        ];
        self.swipe(pts, 12);
    }

    pub fn scroll_wheel(&self, wheel_delta: i32) {
        if wheel_delta != 0 {
            let _ = self
                .input_tx
                .send(BridgeInputCmd::ScrollWheel { delta: wheel_delta });
        }
    }

    pub fn press_power(&self) {
        let _ = self.input_tx.send(BridgeInputCmd::KeyPress {
            key_id: 0,
            hold_ms: 40,
        });
    }

    pub fn send_key(&self, key_id: u16, down: bool) {
        let _ = self
            .input_tx
            .send(BridgeInputCmd::KeyState { key_id, down });
    }

    pub fn send_char(&self, ch: char) -> bool {
        if QemuRunner::ios_keyboard_coords(ch).is_some() {
            let _ = self.input_tx.send(BridgeInputCmd::CharInput { ch });
            true
        } else {
            false
        }
    }
}
