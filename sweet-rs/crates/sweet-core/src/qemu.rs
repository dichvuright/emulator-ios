//! QEMU `virt` (`cortex-a76`, `gic-version=3`, MTTCG) discovery, `SWEETXNU` payload loader,
//! and QMP TCP JSON client (`std::net::TcpStream`).

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::{
    QemuConfig, QemuRunner, SweetInputMailbox, FB_BASE, FB_H, FB_STRIDE, FB_W, INPUT_MBOX_MAGIC,
    INPUT_MBOX_PA,
};

pub const STUB_PA: u64 = 0x4020_0000;
pub const PAGE_SIZE_16K: u64 = 0x4000;
pub const BOOT_LINE_LENGTH: usize = 608;
pub const FB_SIZE_BYTES: usize = FB_STRIDE * FB_H;

static RUNNING_QEMU_PROCS: OnceLock<Mutex<HashMap<u16, Child>>> = OnceLock::new();
static QEMU_PROC_HANDLE: AtomicUsize = AtomicUsize::new(0);
static QEMU_RAM_HOST_BASE: AtomicUsize = AtomicUsize::new(0);
static QEMU_ATTACHED_PID: AtomicU32 = AtomicU32::new(0);
static QEMU_DISCOVERY_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static QEMU_LAST_SCAN: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
static QEMU_INPUT_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[cfg(windows)]
mod win32_ram {
    use super::*;
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};

    #[repr(C)]
    struct PROCESSENTRY32W {
        dw_size: u32,
        cnt_usage: u32,
        th32_process_id: u32,
        th32_default_heap_id: usize,
        th32_module_id: u32,
        cnt_threads: u32,
        th32_parent_process_id: u32,
        pc_pri_class_base: i32,
        dw_flags: u32,
        sz_exe_file: [u16; 260],
    }

    #[repr(C)]
    struct MEMORY_BASIC_INFORMATION {
        base_address: usize,
        allocation_base: usize,
        allocation_protect: u32,
        partition_id: u16,
        region_size: usize,
        state: u32,
        protect: u32,
        type_: u32,
    }

    type NtWriteVirtualMemoryFn = unsafe extern "system" fn(
        process_handle: *mut c_void,
        base_address: *mut c_void,
        buffer: *const c_void,
        number_of_bytes_to_write: usize,
        number_of_bytes_written: *mut usize,
    ) -> i32;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateToolhelp32Snapshot(dw_flags: u32, th32_process_id: u32) -> *mut c_void;
        fn Process32FirstW(h_snapshot: *mut c_void, lppe: *mut PROCESSENTRY32W) -> i32;
        fn Process32NextW(h_snapshot: *mut c_void, lppe: *mut PROCESSENTRY32W) -> i32;
        fn OpenProcess(
            dw_desired_access: u32,
            b_inherit_handle: i32,
            dw_process_id: u32,
        ) -> *mut c_void;
        fn CloseHandle(h_object: *mut c_void) -> i32;
        fn VirtualQueryEx(
            h_process: *mut c_void,
            lp_address: *const c_void,
            lp_buffer: *mut MEMORY_BASIC_INFORMATION,
            dw_length: usize,
        ) -> usize;
        fn ReadProcessMemory(
            h_process: *mut c_void,
            lp_base_address: *const c_void,
            lp_buffer: *mut c_void,
            n_size: usize,
            lp_number_of_bytes_read: *mut usize,
        ) -> i32;
        fn WriteProcessMemory(
            h_process: *mut c_void,
            lp_base_address: *mut c_void,
            lp_buffer: *const c_void,
            n_size: usize,
            lp_number_of_bytes_written: *mut usize,
        ) -> i32;
        fn GetModuleHandleA(lp_module_name: *const u8) -> *mut c_void;
        fn GetProcAddress(h_module: *mut c_void, lp_proc_name: *const u8) -> *mut c_void;
    }

    fn nt_write_fn() -> Option<NtWriteVirtualMemoryFn> {
        static NT_WRITE: OnceLock<Option<NtWriteVirtualMemoryFn>> = OnceLock::new();
        *NT_WRITE.get_or_init(|| unsafe {
            let h_ntdll = GetModuleHandleA(b"ntdll.dll\0".as_ptr());
            if h_ntdll.is_null() {
                return None;
            }
            let p = GetProcAddress(h_ntdll, b"NtWriteVirtualMemory\0".as_ptr());
            if p.is_null() {
                None
            } else {
                Some(std::mem::transmute::<*mut c_void, NtWriteVirtualMemoryFn>(p))
            }
        })
    }

    unsafe fn write_proc_mem(handle: *mut c_void, addr: usize, buf: &[u8]) -> bool {
        let mut n_written: usize = 0;
        if let Some(nt_write) = nt_write_fn() {
            let st = nt_write(
                handle,
                addr as *mut c_void,
                buf.as_ptr() as *const c_void,
                buf.len(),
                &mut n_written,
            );
            st == 0 && n_written == buf.len()
        } else {
            WriteProcessMemory(
                handle,
                addr as *mut c_void,
                buf.as_ptr() as *const c_void,
                buf.len(),
                &mut n_written,
            ) != 0
                && n_written == buf.len()
        }
    }

    unsafe fn verify_stub(handle: *mut c_void, base: usize) -> bool {
        if handle.is_null() || base == 0 {
            return false;
        }
        let mut words = [0u8; 16];
        let mut n_read: usize = 0;
        let addr = base.wrapping_add((STUB_PA - 0x4000_0000) as usize);
        let ok = ReadProcessMemory(
            handle,
            addr as *const c_void,
            words.as_mut_ptr() as *mut c_void,
            16,
            &mut n_read,
        );
        ok != 0
            && n_read == 16
            && u32::from_le_bytes([words[0], words[1], words[2], words[3]]) == 0x5800_0080
            && u32::from_le_bytes([words[4], words[5], words[6], words[7]]) == 0x5800_00a1
            && u32::from_le_bytes([words[8], words[9], words[10], words[11]]) == 0xd61f_0020
            && u32::from_le_bytes([words[12], words[13], words[14], words[15]]) == 0xd503_201f
    }

    fn ensure_attached() -> Option<(*mut c_void, usize)> {
        let cur_h = QEMU_PROC_HANDLE.load(Ordering::Relaxed) as *mut c_void;
        let cur_b = QEMU_RAM_HOST_BASE.load(Ordering::Relaxed);
        if !cur_h.is_null() && cur_b != 0 {
            return Some((cur_h, cur_b));
        }

        let lock = QEMU_DISCOVERY_LOCK.get_or_init(|| Mutex::new(()));
        let _guard = lock.lock().ok()?;

        let cur_h = QEMU_PROC_HANDLE.load(Ordering::Relaxed) as *mut c_void;
        let cur_b = QEMU_RAM_HOST_BASE.load(Ordering::Relaxed);
        unsafe {
            if verify_stub(cur_h, cur_b) {
                return Some((cur_h, cur_b));
            }
            if !cur_h.is_null() {
                CloseHandle(cur_h);
                QEMU_PROC_HANDLE.store(0, Ordering::Relaxed);
                QEMU_RAM_HOST_BASE.store(0, Ordering::Relaxed);
                QEMU_ATTACHED_PID.store(0, Ordering::Relaxed);
            }

            let scan_lock = QEMU_LAST_SCAN.get_or_init(|| Mutex::new(None));
            if let Ok(mut last) = scan_lock.lock() {
                if let Some(prev) = *last {
                    if prev.elapsed() < Duration::from_millis(200) {
                        return None;
                    }
                }
                *last = Some(Instant::now());
            }

            let h_snap = CreateToolhelp32Snapshot(0x2, 0);
            if h_snap.is_null() || h_snap == (-1isize as *mut c_void) {
                return None;
            }
            let mut pe: PROCESSENTRY32W = zeroed();
            pe.dw_size = size_of::<PROCESSENTRY32W>() as u32;
            let mut pids = Vec::new();
            if Process32FirstW(h_snap, &mut pe) != 0 {
                loop {
                    let nul = pe
                        .sz_exe_file
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(pe.sz_exe_file.len());
                    let name = String::from_utf16_lossy(&pe.sz_exe_file[..nul]).to_ascii_lowercase();
                    if name == "qemu-system-aarch64.exe" {
                        pids.push(pe.th32_process_id);
                    }
                    if Process32NextW(h_snap, &mut pe) == 0 {
                        break;
                    }
                }
            }
            CloseHandle(h_snap);

            for pid in pids {
                let h_proc = OpenProcess(0x0438, 0, pid);
                if h_proc.is_null() {
                    continue;
                }
                let mut addr: usize = 0x10000;
                let mut last_alloc_base: usize = 0;
                let mut mbi: MEMORY_BASIC_INFORMATION = zeroed();
                while addr < (1usize << 47) {
                    let ret = VirtualQueryEx(
                        h_proc,
                        addr as *const c_void,
                        &mut mbi,
                        size_of::<MEMORY_BASIC_INFORMATION>(),
                    );
                    if ret == 0 {
                        break;
                    }
                    let base = mbi.base_address;
                    let alloc_base = mbi.allocation_base;
                    let rsize = mbi.region_size;
                    if mbi.state == 0x1000 && alloc_base != 0 && alloc_base != last_alloc_base {
                        last_alloc_base = alloc_base;
                        if verify_stub(h_proc, alloc_base) {
                            QEMU_PROC_HANDLE.store(h_proc as usize, Ordering::Relaxed);
                            QEMU_RAM_HOST_BASE.store(alloc_base, Ordering::Relaxed);
                            QEMU_ATTACHED_PID.store(pid, Ordering::Relaxed);
                            return Some((h_proc, alloc_base));
                        }
                    }
                    if mbi.state == 0x1000 && base != alloc_base && rsize >= (512 * 1024 * 1024) {
                        if verify_stub(h_proc, base) {
                            QEMU_PROC_HANDLE.store(h_proc as usize, Ordering::Relaxed);
                            QEMU_RAM_HOST_BASE.store(base, Ordering::Relaxed);
                            QEMU_ATTACHED_PID.store(pid, Ordering::Relaxed);
                            return Some((h_proc, base));
                        }
                    }
                    addr = base.saturating_add(rsize.max(4096));
                }
                CloseHandle(h_proc);
            }
        }
        None
    }

    pub fn invalidate_attachment() {
        let lock = QEMU_DISCOVERY_LOCK.get_or_init(|| Mutex::new(()));
        if let Ok(_g) = lock.lock() {
            let cur_h = QEMU_PROC_HANDLE.swap(0, Ordering::Relaxed) as *mut c_void;
            QEMU_RAM_HOST_BASE.store(0, Ordering::Relaxed);
            QEMU_ATTACHED_PID.store(0, Ordering::Relaxed);
            if let Some(scan_lock) = QEMU_LAST_SCAN.get() {
                if let Ok(mut last) = scan_lock.lock() {
                    *last = None;
                }
            }
            if !cur_h.is_null() {
                unsafe {
                    CloseHandle(cur_h);
                }
            }
        }
    }

    pub fn attachment_info() -> Option<(u32, usize)> {
        let (_h, base) = ensure_attached()?;
        let pid = QEMU_ATTACHED_PID.load(Ordering::Relaxed);
        Some((pid, base))
    }

    pub fn read_u64(pa: u64) -> Option<u64> {
        if pa < 0x4000_0000 {
            return None;
        }
        let (handle, base) = ensure_attached()?;
        let host_va = base.wrapping_add((pa - 0x4000_0000) as usize);
        let mut buf = [0u8; 8];
        let mut n_read: usize = 0;
        let ok = unsafe {
            ReadProcessMemory(
                handle,
                host_va as *const c_void,
                buf.as_mut_ptr() as *mut c_void,
                8,
                &mut n_read,
            )
        };
        if ok != 0 && n_read == 8 {
            Some(u64::from_le_bytes(buf))
        } else {
            invalidate_attachment();
            None
        }
    }

    pub fn read_bytes(pa: u64, dst: &mut [u8]) -> bool {
        if pa < 0x4000_0000 || dst.is_empty() {
            return false;
        }
        let Some((handle, base)) = ensure_attached() else {
            return false;
        };
        let host_va = base.wrapping_add((pa - 0x4000_0000) as usize);
        let mut n_read: usize = 0;
        let ok = unsafe {
            ReadProcessMemory(
                handle,
                host_va as *const c_void,
                dst.as_mut_ptr() as *mut c_void,
                dst.len(),
                &mut n_read,
            )
        };
        if ok != 0 && n_read == dst.len() {
            true
        } else {
            invalidate_attachment();
            false
        }
    }

    pub fn read_fb(dst: &mut [u8]) -> bool {
        let want = dst.len().min(FB_SIZE_BYTES);
        if want == 0 {
            return false;
        }
        let Some((handle, base)) = ensure_attached() else {
            return false;
        };
        let host_va = base.wrapping_add((FB_BASE - 0x4000_0000) as usize);
        let mut n_read: usize = 0;
        let ok = unsafe {
            ReadProcessMemory(
                handle,
                host_va as *const c_void,
                dst.as_mut_ptr() as *mut c_void,
                want,
                &mut n_read,
            )
        };
        if ok != 0 && n_read == want {
            true
        } else {
            invalidate_attachment();
            false
        }
    }

    pub fn enqueue_input(ev_type: u16, ev_id: u16, down: bool, x: i32, y: i32) -> bool {
        let Some((handle, base)) = ensure_attached() else {
            return false;
        };
        let lock = QEMU_INPUT_LOCK.get_or_init(|| Mutex::new(()));
        let Ok(_guard) = lock.lock() else {
            return false;
        };
        let mbox_va = base.wrapping_add((INPUT_MBOX_PA - 0x4000_0000) as usize);
        unsafe {
            let mut hdr = [0u8; 16];
            let mut n_read: usize = 0;
            let ok = ReadProcessMemory(
                handle,
                mbox_va as *const c_void,
                hdr.as_mut_ptr() as *mut c_void,
                16,
                &mut n_read,
            );
            if ok == 0 || n_read != 16 {
                invalidate_attachment();
                return false;
            }
            let magic = u32::from_le_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
            let mut w_idx = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]);
            let r_idx = u32::from_le_bytes([hdr[8], hdr[9], hdr[10], hdr[11]]);
            if magic != INPUT_MBOX_MAGIC {
                let mut init_hdr = [0u8; 16];
                init_hdr[0..4].copy_from_slice(&INPUT_MBOX_MAGIC.to_le_bytes());
                if !write_proc_mem(handle, mbox_va, &init_hdr) {
                    return false;
                }
                w_idx = 0;
            }
            let mut ev = [0u8; 16];
            ev[0..2].copy_from_slice(&ev_type.to_le_bytes());
            ev[2..4].copy_from_slice(&ev_id.to_le_bytes());
            ev[4..8].copy_from_slice(&(if down { 1i32 } else { 0i32 }).to_le_bytes());
            ev[8..12].copy_from_slice(&x.to_le_bytes());
            ev[12..16].copy_from_slice(&y.to_le_bytes());

            // In-place move coalescing: if there is already >= 1 unread event in the ring
            // and both the new event and the tail unread event are touch-down/move (type=1, down=1),
            // overwrite the tail slot in-place so the ring buffer never builds a lag backlog!
            let pending = w_idx.wrapping_sub(r_idx);
            if ev_type == 1 && down && pending >= 1 && pending < 64 {
                let prev_slot = (w_idx.wrapping_sub(1) as usize) & 63;
                let prev_va = mbox_va + 16 + prev_slot * 16;
                let mut prev_ev = [0u8; 8];
                let mut nr: usize = 0;
                if ReadProcessMemory(
                    handle,
                    prev_va as *const c_void,
                    prev_ev.as_mut_ptr() as *mut c_void,
                    8,
                    &mut nr,
                ) != 0
                    && nr == 8
                {
                    let prev_type = u16::from_le_bytes([prev_ev[0], prev_ev[1]]);
                    let prev_down = i32::from_le_bytes([prev_ev[4], prev_ev[5], prev_ev[6], prev_ev[7]]);
                    if prev_type == 1 && prev_down != 0 {
                        return write_proc_mem(handle, prev_va, &ev);
                    }
                }
            }

            let slot = (w_idx as usize) & 63;
            let slot_va = mbox_va + 16 + slot * 16;
            if !write_proc_mem(handle, slot_va, &ev) {
                return false;
            }
            let new_w = w_idx.wrapping_add(1);
            write_proc_mem(handle, mbox_va + 4, &new_w.to_le_bytes())
        }
    }
}

pub fn direct_qemu_attachment_info() -> Option<(u32, usize)> {
    #[cfg(windows)]
    {
        win32_ram::attachment_info()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

pub fn direct_qemu_invalidate_attachment() {
    #[cfg(windows)]
    {
        win32_ram::invalidate_attachment();
    }
}

pub fn direct_qemu_read_u64(pa: u64) -> Option<u64> {
    #[cfg(windows)]
    {
        win32_ram::read_u64(pa)
    }
    #[cfg(not(windows))]
    {
        let _ = pa;
        None
    }
}

pub fn direct_qemu_read_bytes(pa: u64, dst: &mut [u8]) -> bool {
    #[cfg(windows)]
    {
        win32_ram::read_bytes(pa, dst)
    }
    #[cfg(not(windows))]
    {
        let _ = (pa, dst);
        false
    }
}

pub fn direct_qemu_read_fb(dst: &mut [u8]) -> bool {
    #[cfg(windows)]
    {
        win32_ram::read_fb(dst)
    }
    #[cfg(not(windows))]
    {
        let _ = dst;
        false
    }
}

pub fn direct_qemu_enqueue_input(ev_type: u16, ev_id: u16, down: bool, x: i32, y: i32) -> bool {
    #[cfg(windows)]
    {
        win32_ram::enqueue_input(ev_type, ev_id, down, x, y)
    }
    #[cfg(not(windows))]
    {
        let _ = (ev_type, ev_id, down, x, y);
        false
    }
}

fn procs_map() -> &'static Mutex<HashMap<u16, Child>> {
    RUNNING_QEMU_PROCS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Locate `qemu-system-aarch64` in `PATH` or standard Windows installation directories.
pub fn find_qemu_binary_impl() -> Option<String> {
    let exe_names: &[&str] = if cfg!(windows) {
        &["qemu-system-aarch64.exe", "qemu-system-aarch64"]
    } else {
        &["qemu-system-aarch64"]
    };

    if let Some(path_var) = env::var_os("PATH") {
        for dir in env::split_paths(&path_var) {
            for name in exe_names {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(candidate.to_string_lossy().into_owned());
                }
            }
        }
    }

    let mut candidates = vec![
        PathBuf::from(r"research\qemu\qemu-system-aarch64.exe"),
        PathBuf::from(r"..\research\qemu\qemu-system-aarch64.exe"),
        PathBuf::from(r"C:\Program Files\qemu\qemu-system-aarch64.exe"),
        PathBuf::from(r"C:\Program Files (x86)\qemu\qemu-system-aarch64.exe"),
        PathBuf::from(r"C:\msys64\mingw64\bin\qemu-system-aarch64.exe"),
        PathBuf::from(r"C:\msys64\ucrt64\bin\qemu-system-aarch64.exe"),
    ];

    if let Some(home) = env::var_os("USERPROFILE").or_else(|| env::var_os("HOME")) {
        candidates.push(
            PathBuf::from(home)
                .join("scoop")
                .join("apps")
                .join("qemu")
                .join("current")
                .join("qemu-system-aarch64.exe"),
        );
    }

    for c in candidates {
        if c.is_file() {
            return Some(c.to_string_lossy().into_owned());
        }
    }

    None
}

/// Unpack a `SWEETXNU` v1 payload into `(phys_base, ram_image, boot_args_pa, entry_pa, stub_bytes)`.
pub fn unpack_sweetxnu_payload(payload: &[u8]) -> Result<(u64, Vec<u8>, u64, u64, [u8; 32]), String> {
    if payload.len() < 0x2a8 || &payload[0..8] != b"SWEETXNU" {
        return Err("Not a valid SWEETXNU payload".to_string());
    }
    let ver = u32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]);
    if ver != 1 {
        return Err(format!("Unsupported SWEETXNU payload version: {}", ver));
    }
    let nsegs = u32::from_le_bytes([payload[12], payload[13], payload[14], payload[15]]) as usize;

    let read_u64 = |off: usize| -> u64 {
        u64::from_le_bytes([
            payload[off],
            payload[off + 1],
            payload[off + 2],
            payload[off + 3],
            payload[off + 4],
            payload[off + 5],
            payload[off + 6],
            payload[off + 7],
        ])
    };

    let virt_base = read_u64(0x10);
    let entry_va = read_u64(0x18);
    let adt_off = read_u64(0x20) as usize;
    let adt_size = read_u64(0x28) as usize;
    let kernel_end = read_u64(0x30);
    let phys_base = read_u64(0x38);
    let mem_size = read_u64(0x40);
    let cmdline = &payload[0x48..0x48 + BOOT_LINE_LENGTH];

    let delta = phys_base.wrapping_sub(virt_base);
    let adt_pa = kernel_end.wrapping_add(delta);
    let ba_pa = (adt_pa + (adt_size as u64) + PAGE_SIZE_16K - 1) & !(PAGE_SIZE_16K - 1);
    let top = ba_pa + PAGE_SIZE_16K;

    if top < phys_base || (top - phys_base) > (512 * 1024 * 1024) {
        return Err("SWEETXNU payload image range invalid".to_string());
    }

    let img_len = (top - phys_base) as usize;
    let mut img = vec![0u8; img_len];

    for i in 0..nsegs {
        let seg_off = 0x2a8 + 32 * i;
        if seg_off + 32 > payload.len() {
            break;
        }
        let vmaddr = read_u64(seg_off);
        let _vmsize = read_u64(seg_off + 8);
        let foff = read_u64(seg_off + 16) as usize;
        let fsize = read_u64(seg_off + 24) as usize;
        let dst_off = vmaddr.wrapping_add(delta).wrapping_sub(phys_base) as usize;
        if foff + fsize <= payload.len() && dst_off + fsize <= img.len() {
            img[dst_off..dst_off + fsize].copy_from_slice(&payload[foff..foff + fsize]);
        }
    }

    let adt_dst = (adt_pa - phys_base) as usize;
    if adt_off + adt_size <= payload.len() && adt_dst + adt_size <= img.len() {
        img[adt_dst..adt_dst + adt_size].copy_from_slice(&payload[adt_off..adt_off + adt_size]);
    }

    // Build 736-byte XNU arm64 boot_args struct
    let mut ba = Vec::with_capacity(736);
    ba.extend_from_slice(&2u16.to_le_bytes()); // Revision = 2
    ba.extend_from_slice(&2u16.to_le_bytes()); // Version = 2
    ba.extend_from_slice(&[0u8; 4]); // pad
    ba.extend_from_slice(&virt_base.to_le_bytes());
    ba.extend_from_slice(&phys_base.to_le_bytes());
    ba.extend_from_slice(&mem_size.to_le_bytes());
    ba.extend_from_slice(&top.to_le_bytes());
    // Boot_Video: v_baseAddr, v_display(1), v_rowBytes, v_width, v_height, v_depth(32)
    ba.extend_from_slice(&FB_BASE.to_le_bytes());
    ba.extend_from_slice(&1u64.to_le_bytes());
    ba.extend_from_slice(&(FB_STRIDE as u64).to_le_bytes());
    ba.extend_from_slice(&(FB_W as u64).to_le_bytes());
    ba.extend_from_slice(&(FB_H as u64).to_le_bytes());
    ba.extend_from_slice(&32u64.to_le_bytes());
    ba.extend_from_slice(&0u32.to_le_bytes()); // machineType
    ba.extend_from_slice(&[0u8; 4]); // pad
    ba.extend_from_slice(&adt_pa.wrapping_sub(delta).to_le_bytes()); // deviceTreeP (VA)
    ba.extend_from_slice(&(adt_size as u32).to_le_bytes()); // deviceTreeLength
    ba.extend_from_slice(cmdline);
    ba.extend_from_slice(&[0u8; 4]); // pad
    ba.extend_from_slice(&0u64.to_le_bytes()); // bootFlags
    ba.extend_from_slice(&mem_size.to_le_bytes()); // memSizeActual

    let ba_dst = (ba_pa - phys_base) as usize;
    if ba_dst + ba.len() <= img.len() {
        img[ba_dst..ba_dst + ba.len()].copy_from_slice(&ba);
    }

    let entry_pa = entry_va.wrapping_add(delta);
    // 6-instruction stub: ldr x0, =ba ; ldr x1, =entry ; br x1 ; nop ; .quad ba ; .quad entry
    let mut stub = [0u8; 32];
    stub[0..4].copy_from_slice(&0x5800_0080u32.to_le_bytes());
    stub[4..8].copy_from_slice(&0x5800_00a1u32.to_le_bytes());
    stub[8..12].copy_from_slice(&0xd61f_0020u32.to_le_bytes());
    stub[12..16].copy_from_slice(&0xd503_201fu32.to_le_bytes());
    stub[16..24].copy_from_slice(&ba_pa.to_le_bytes());
    stub[24..32].copy_from_slice(&entry_pa.to_le_bytes());

    Ok((phys_base, img, ba_pa, entry_pa, stub))
}

/// Build the `qemu-system-aarch64` command-line arguments for a given `QemuConfig`.
pub fn build_qemu_args(cfg: &QemuConfig, phys_base_opt: Option<u64>) -> Vec<String> {
    let smp = cfg.smp.clamp(1, 8);
    let ram_gb = cfg.ram_gb.max(4);
    let out_dir = Path::new(&cfg.out_dir);
    let serial_path = out_dir.join("serial.txt");
    let log_path = out_dir.join("qemu.log");
    let mbox_path = out_dir.join("mbox.bin");

    let mut args = vec![
        "-M".to_string(),
        "virt,gic-version=3".to_string(),
        "-cpu".to_string(),
        "cortex-a76".to_string(),
        "-smp".to_string(),
        smp.to_string(),
        "-accel".to_string(),
        "tcg,thread=multi".to_string(),
        "-m".to_string(),
        format!("{}G", ram_gb),
    ];

    if cfg.display_window {
        args.extend([
            "-device".to_string(),
            "ramfb".to_string(),
            "-device".to_string(),
            "virtio-tablet-device".to_string(),
            "-device".to_string(),
            "virtio-keyboard-device".to_string(),
        ]);
    } else {
        args.push("-nographic".to_string());
    }

    args.extend([
        "-serial".to_string(),
        format!("file:{}", serial_path.to_string_lossy()),
        "-qmp".to_string(),
        format!("tcp:127.0.0.1:{},server,nowait", cfg.qmp_port),
        "-d".to_string(),
        "guest_errors".to_string(),
        "-D".to_string(),
        log_path.to_string_lossy().into_owned(),
        "-no-reboot".to_string(),
    ]);

    if let Some(phys_base) = phys_base_opt {
        let ram_bin = out_dir.join("ram.bin");
        let stub_bin = out_dir.join("stub.bin");
        args.extend([
            "-device".to_string(),
            format!(
                "loader,file={},addr={:#x},force-raw=on",
                ram_bin.to_string_lossy(),
                phys_base
            ),
            "-device".to_string(),
            format!(
                "loader,file={},addr={:#x},force-raw=on",
                stub_bin.to_string_lossy(),
                STUB_PA
            ),
            "-device".to_string(),
            format!("loader,addr={:#x},cpu-num=0", STUB_PA),
        ]);
    }

    if mbox_path.is_file() {
        args.extend([
            "-device".to_string(),
            format!(
                "loader,file={},addr={:#x},force-raw=on",
                mbox_path.to_string_lossy(),
                INPUT_MBOX_PA
            ),
        ]);
    }

    if !cfg.disk_path.is_empty() && Path::new(&cfg.disk_path).is_file() {
        args.extend([
            "-drive".to_string(),
            format!("if=none,id=iosroot,format=raw,file={}", cfg.disk_path),
            "-device".to_string(),
            "virtio-blk-device,drive=iosroot".to_string(),
        ]);
    }

    if cfg.net_enabled {
        args.extend([
            "-netdev".to_string(),
            "user,id=net0,hostfwd=tcp::2222-:22,hostfwd=tcp::8080-:80".to_string(),
            "-device".to_string(),
            "virtio-net-device,netdev=net0".to_string(),
        ]);
    }

    args
}

/// Execute a single QMP JSON command over TCP (`127.0.0.1:port`) after negotiating `qmp_capabilities`.
pub fn qmp_execute_json(port: u16, command_json: &str, timeout_ms: u64) -> Result<String, String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let timeout = Duration::from_millis(timeout_ms.max(100));
    let mut stream = TcpStream::connect_timeout(&addr, timeout)
        .map_err(|e| format!("QMP connect to 127.0.0.1:{} failed: {}", port, e))?;
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    let cloned = stream
        .try_clone()
        .map_err(|e| format!("Failed to clone QMP TcpStream: {}", e))?;
    let mut reader = BufReader::new(cloned);

    // 1. Read QMP greeting
    let mut greeting = String::new();
    reader
        .read_line(&mut greeting)
        .map_err(|e| format!("Failed to read QMP greeting: {}", e))?;
    if greeting.trim().is_empty() {
        return Err("Empty QMP greeting from QEMU".to_string());
    }

    // 2. Negotiate capabilities
    stream
        .write_all(b"{\"execute\":\"qmp_capabilities\"}\n")
        .map_err(|e| format!("Failed to send qmp_capabilities: {}", e))?;
    stream.flush().ok();

    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .map_err(|e| format!("QMP read error during capabilities: {}", e))?;
        if n == 0 {
            return Err("QMP closed during capabilities".to_string());
        }
        if line.contains("\"return\"") || line.contains("\"error\"") {
            break;
        }
    }

    // 3. Send actual command
    let mut payload = command_json.trim().to_string();
    payload.push('\n');
    stream
        .write_all(payload.as_bytes())
        .map_err(|e| format!("Failed to send QMP command: {}", e))?;
    stream.flush().ok();

    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .map_err(|e| format!("QMP read error waiting for response: {}", e))?;
        if n == 0 {
            return Err("QMP connection closed before response".to_string());
        }
        if line.contains("\"error\"") {
            return Err(format!("QMP returned error: {}", line.trim()));
        }
        if line.contains("\"return\"") {
            return Ok(line.trim().to_string());
        }
    }
}

impl QemuRunner {
    pub fn find_qemu_binary() -> Option<String> {
        find_qemu_binary_impl()
    }

    pub fn start(cfg: &QemuConfig) -> Result<Self, String> {
        direct_qemu_invalidate_attachment();
        let out_dir = Path::new(&cfg.out_dir);
        fs::create_dir_all(out_dir)
            .map_err(|e| format!("Failed to create out_dir {:?}: {}", out_dir, e))?;

        // 1. Stop any existing python wrapper process FIRST so its finally: block finishes
        //    before we touch ram.bin or spawn the new QEMU VM.
        if let Ok(mut map) = procs_map().lock() {
            if let Some(mut prev) = map.remove(&cfg.qmp_port) {
                let _ = prev.kill();
                let _ = prev.wait();
            }
        }

        let mbox_path = out_dir.join("mbox.bin");
        let fresh = SweetInputMailbox::new();
        fs::write(&mbox_path, fresh.to_bytes())
            .map_err(|e| format!("Failed to write initial mbox.bin: {}", e))?;
        let mbox = fresh;

        let qemu_bin = if !cfg.qemu_bin.is_empty() && Path::new(&cfg.qemu_bin).is_file() {
            cfg.qemu_bin.clone()
        } else {
            Self::find_qemu_binary().unwrap_or_default()
        };

        let py_script = Path::new("tools/qemu-sweet.py");
        let patched_kernel = Path::new("emu_workspace/kernel.qemu.sweet");
        if !qemu_bin.is_empty()
            && !cfg.payload_path.is_empty()
            && Path::new(&cfg.payload_path).is_file()
        {
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                let _ = Command::new("taskkill")
                    .args(["/F", "/IM", "qemu-system-aarch64.exe"])
                    .creation_flags(0x0800_0000)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            // Wait briefly for OS file handles on the disk image to release
            std::thread::sleep(Duration::from_millis(120));

            let _ = fs::remove_file(out_dir.join("sweet_log.txt"));
            let _ = fs::remove_file(out_dir.join("fb.bin"));
            let _ = fs::remove_file(out_dir.join("result.json"));

            if py_script.is_file() && patched_kernel.is_file() {
                let mut cmd = Command::new("python");
                cmd.args([
                    "tools/qemu-sweet.py",
                    &cfg.payload_path,
                    &cfg.out_dir,
                    "--smp",
                    &cfg.smp.clamp(1, 8).to_string(),
                    "--ram-gb",
                    &cfg.ram_gb.clamp(4, 16).to_string(),
                    "--secs",
                    "86400",
                    "--interactive",
                    "--net",
                    "--kernel",
                    "emu_workspace/kernel.qemu.sweet",
                ]);
                if !cfg.disk_path.is_empty() && Path::new(&cfg.disk_path).is_file() {
                    cmd.args(["--disk", &cfg.disk_path]);
                }
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    cmd.creation_flags(0x0800_0000);
                }
                let child = cmd
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|e| format!("Failed to spawn tools/qemu-sweet.py: {}", e))?;

                if let Ok(mut map) = procs_map().lock() {
                    if let Some(mut prev) = map.insert(cfg.qmp_port, child) {
                        let _ = prev.kill();
                        let _ = prev.wait();
                    }
                }
            } else {
                let mut phys_base_opt = None;
                if let Ok(payload_bytes) = fs::read(&cfg.payload_path) {
                    if let Ok((phys_base, ram_img, _ba_pa, _entry_pa, stub_bytes)) =
                        unpack_sweetxnu_payload(&payload_bytes)
                    {
                        let _ = fs::write(out_dir.join("ram.bin"), &ram_img);
                        let _ = fs::write(out_dir.join("stub.bin"), &stub_bytes);
                        phys_base_opt = Some(phys_base);
                    }
                }
                let args = build_qemu_args(cfg, phys_base_opt);
                let child = Command::new(&qemu_bin)
                    .args(&args)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|e| format!("Failed to spawn QEMU ({}): {}", qemu_bin, e))?;

                if let Ok(mut map) = procs_map().lock() {
                    if let Some(mut prev) = map.insert(cfg.qmp_port, child) {
                        let _ = prev.kill();
                        let _ = prev.wait();
                    }
                }
            }
        }

        let mut final_cfg = cfg.clone();
        if final_cfg.qemu_bin.is_empty() {
            final_cfg.qemu_bin = qemu_bin;
        }

        Ok(Self {
            cfg: final_cfg,
            mbox,
        })
    }

    pub fn stop(&mut self) {
        direct_qemu_invalidate_attachment();
        if let Ok(mut map) = procs_map().lock() {
            if let Some(mut child) = map.remove(&self.cfg.qmp_port) {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let _ = Command::new("taskkill")
                .args(["/F", "/IM", "qemu-system-aarch64.exe"])
                .creation_flags(0x0800_0000)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }

    pub fn is_running(&mut self) -> bool {
        if let Ok(mut map) = procs_map().lock() {
            if let Some(child) = map.get_mut(&self.cfg.qmp_port) {
                match child.try_wait() {
                    Ok(None) => return true,
                    Ok(Some(_)) | Err(_) => {
                        map.remove(&self.cfg.qmp_port);
                    }
                }
            }
        }
        false
    }

    fn send_udp_hid_packet(ev_type: u8, ev_id: u8, down: bool, x: i32, y: i32) {
        static UDP_SOCK: OnceLock<Option<std::net::UdpSocket>> = OnceLock::new();
        let sock_opt = UDP_SOCK.get_or_init(|| {
            let s = std::net::UdpSocket::bind("127.0.0.1:0").ok()?;
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

    pub fn send_touch(&mut self, finger: u16, down: bool, x: i32, y: i32) -> Result<(), String> {
        let cx = x.clamp(0, (FB_W as i32) - 1);
        let cy = y.clamp(0, (FB_H as i32) - 1);
        if !direct_qemu_enqueue_input(1, finger & 0xff, down, cx, cy) {
            Self::send_udp_hid_packet(1, (finger & 0xff) as u8, down, cx, cy);
        }
        let is_live_run = self.cfg.out_dir.ends_with("qemu_run");
        if !self.cfg.out_dir.is_empty() && !is_live_run {
            let mbox_path = Path::new(&self.cfg.out_dir).join("mbox.bin");
            if let Ok(raw) = fs::read(&mbox_path) {
                if let Some(existing) = SweetInputMailbox::from_bytes(&raw) {
                    self.mbox = existing;
                }
            }
            self.mbox.push_touch(finger, down, cx, cy);
            if let Some(parent) = mbox_path.parent() {
                if parent.is_dir() {
                    let _ = fs::write(&mbox_path, self.mbox.to_bytes());
                }
            }
        } else {
            self.mbox.push_touch(finger, down, cx, cy);
            if is_live_run {
                self.mbox.read_idx = self.mbox.write_idx;
            }
        }
        Ok(())
    }

    pub fn send_key(&mut self, key: u16, down: bool) -> Result<(), String> {
        if !direct_qemu_enqueue_input(2, key & 0xff, down, 0, 0) {
            Self::send_udp_hid_packet(2, (key & 0xff) as u8, down, 0, 0);
        }
        let is_live_run = self.cfg.out_dir.ends_with("qemu_run");
        if !self.cfg.out_dir.is_empty() && !is_live_run {
            let mbox_path = Path::new(&self.cfg.out_dir).join("mbox.bin");
            if let Ok(raw) = fs::read(&mbox_path) {
                if let Some(existing) = SweetInputMailbox::from_bytes(&raw) {
                    self.mbox = existing;
                }
            }
            self.mbox.push_key(key, down);
            if let Some(parent) = mbox_path.parent() {
                if parent.is_dir() {
                    let _ = fs::write(&mbox_path, self.mbox.to_bytes());
                }
            }
        } else {
            self.mbox.push_key(key, down);
            if is_live_run {
                self.mbox.read_idx = self.mbox.write_idx;
            }
        }
        Ok(())
    }

    /// Map a PC keyboard character to iOS 16 on-screen QWERTY keyboard coordinates in 1080x2400 space.
    pub fn ios_keyboard_coords(ch: char) -> Option<(i32, i32)> {
        let lower = ch.to_ascii_lowercase();
        match lower {
            // Row 1: Q W E R T Y U I O P (y = 1665)
            'q' => Some((54, 1665)),
            'w' => Some((162, 1665)),
            'e' => Some((270, 1665)),
            'r' => Some((378, 1665)),
            't' => Some((486, 1665)),
            'y' => Some((594, 1665)),
            'u' => Some((702, 1665)),
            'i' => Some((810, 1665)),
            'o' => Some((918, 1665)),
            'p' => Some((1026, 1665)),
            // Row 2: A S D F G H J K L (y = 1825)
            'a' => Some((108, 1825)),
            's' => Some((216, 1825)),
            'd' => Some((324, 1825)),
            'f' => Some((432, 1825)),
            'g' => Some((540, 1825)),
            'h' => Some((648, 1825)),
            'j' => Some((756, 1825)),
            'k' => Some((864, 1825)),
            'l' => Some((972, 1825)),
            // Row 3: Z X C V B N M + Backspace (y = 1985)
            'z' => Some((216, 1985)),
            'x' => Some((324, 1985)),
            'c' => Some((432, 1985)),
            'v' => Some((540, 1985)),
            'b' => Some((648, 1985)),
            'n' => Some((756, 1985)),
            'm' => Some((864, 1985)),
            '\x08' | '\x7f' => Some((1005, 1985)),
            // Row 4: Space + Return/Search (y = 2145)
            ' ' => Some((540, 2145)),
            '\r' | '\n' => Some((945, 2145)),
            _ => None,
        }
    }

    pub fn send_char(&mut self, ch: char) -> Result<(), String> {
        if let Some((kx, ky)) = Self::ios_keyboard_coords(ch) {
            let is_live_run = self.cfg.out_dir.ends_with("qemu_run");
            let _ = self.send_touch(0, true, kx, ky);
            if is_live_run {
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(30));
                    if !direct_qemu_enqueue_input(1, 0, false, kx, ky) {
                        Self::send_udp_hid_packet(1, 0, false, kx, ky);
                    }
                });
            } else {
                let _ = self.send_touch(0, false, kx, ky);
            }
            Ok(())
        } else {
            Err(format!("Unsupported character for iOS keyboard: {:?}", ch))
        }
    }

    pub fn capture_framebuffer(&mut self, out_bgra: &mut [u8]) -> Result<(), String> {
        let want_len = out_bgra.len().min(FB_SIZE_BYTES);
        if direct_qemu_read_fb(&mut out_bgra[..want_len])
            && out_bgra[..want_len].iter().step_by(64).any(|&b| b != 0)
        {
            return Ok(());
        }
        let out_dir = Path::new(&self.cfg.out_dir);
        let fb_bin = out_dir.join("fb.bin");
        if fb_bin.is_file() {
            if let Ok(mut f) = fs::File::open(&fb_bin) {
                if f.read_exact(&mut out_bgra[..want_len]).is_ok()
                    && out_bgra[..want_len].iter().step_by(64).any(|&b| b != 0)
                {
                    return Ok(());
                }
            }
        }
        // Direct QMP pmemsave fallback when talking directly to a QMP endpoint
        let raw_tmp = out_dir.join("fb_qmp_live.raw");
        let escaped = raw_tmp.to_string_lossy().replace('\\', "/");
        let qmp_cmd = format!(
            "{{\"execute\":\"pmemsave\",\"arguments\":{{\"val\":{},\"size\":{},\"filename\":\"{}\"}}}}",
            FB_BASE, want_len, escaped
        );
        if qmp_execute_json(self.cfg.qmp_port, &qmp_cmd, 500).is_ok() && raw_tmp.is_file() {
            if let Ok(mut f) = fs::File::open(&raw_tmp) {
                let ok = f.read_exact(&mut out_bgra[..want_len]).is_ok();
                let _ = fs::remove_file(&raw_tmp);
                if ok {
                    return Ok(());
                }
            }
        }
        Err("No live guest framebuffer available".to_string())
    }

    pub fn read_serial_tail(&self, max_bytes: usize) -> String {
        if self.cfg.out_dir.is_empty() || max_bytes == 0 {
            return String::new();
        }
        let serial_path = Path::new(&self.cfg.out_dir).join("serial.txt");
        let Ok(mut file) = fs::File::open(&serial_path) else {
            return String::new();
        };
        let Ok(meta) = file.metadata() else {
            return String::new();
        };
        let len = meta.len() as usize;
        let read_len = len.min(max_bytes);
        let start = (len - read_len) as u64;
        if file.seek(SeekFrom::Start(start)).is_err() {
            return String::new();
        }
        let mut buf = vec![0u8; read_len];
        if file.read_exact(&mut buf).is_err() {
            return String::new();
        }
        String::from_utf8_lossy(&buf).into_owned()
    }
}
