#!/usr/bin/env python3
"""Boot a SWEETXNU payload (tools/mkpayload.py) in QEMU's virt board.

  qemu-sweet.py PAYLOAD OUTDIR [--secs N] [--gdb] [--live-serial]
  qemu-sweet.py PAYLOAD OUTDIR --smp 6 --disk ios16-root.img --net --interactive

Does what shim/loader.c does on the phone, but on the host: places the
kernel segments, ADT and boot_args at their physical addresses and starts
the CPU at a 6-instruction stub that jumps to the entry with x0 = boot_args.
virt RAM starts at 0x40000000; with 4G it covers the phone's framebuffer
(0x9c000000), the shared-memory input mailbox (0x9d600000), and the kernel's
phys_base (0xb0000000) as plain memory.

After N seconds (or when the guest stops) it writes into OUTDIR:
  fb.png     the 1080x2400 framebuffer (what the phone screen would show)
  regs.txt   QEMU's view of CPU state
  qemu.log   exceptions with ESR/FAR/ELR (-d int,guest_errors)

With --kernel KERNEL (the matching unstripped Mach-O), also decode the
framebuffer using its iso_font into fb.txt and write result.json. This is a
snapshot of the circular screen, not a complete boot log. --fail-on-panic
returns 2 when a panic is visible; absence of a panic does not prove boot.
An undecodable or blank screen returns 3 with --fail-on-panic.
"""
import argparse
import json
import os
import shutil
import socket
import struct
import subprocess
import sys
import time
import zlib

FB_BASE, FB_W, FB_H = 0x9c000000, 1080, 2400
FB_STRIDE = FB_W * 4
FB_SIZE_BYTES = FB_STRIDE * FB_H
PAGE = 0x4000
STUB_PA = 0x40200000	# above the dtb QEMU places at RAM base
BOOT_LINE_LENGTH = 608

# Shared-memory input mailbox (matches bsd/dev/sweet_virtio.c)
INPUT_MBOX_PA = 0x9d600000
INPUT_MBOX_MAGIC = 0x5357494e	# 'SWIN'
INPUT_MBOX_RING_SIZE = 64
INPUT_MBOX_HEADER_SIZE = 16
INPUT_MBOX_ENTRY_SIZE = 16
INPUT_MBOX_TOTAL_SIZE = INPUT_MBOX_HEADER_SIZE + INPUT_MBOX_RING_SIZE * INPUT_MBOX_ENTRY_SIZE

INPUT_TYPE_TOUCH = 1
INPUT_TYPE_KEY = 2

KEY_POWER = 0
KEY_VOL_UP = 1
KEY_VOL_DOWN = 2
KEY_HOME = 3

WINDOWS_QEMU_PATHS = (
    os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), 'research', 'qemu', 'qemu-system-aarch64.exe'),
    r'C:\Program Files\qemu\qemu-system-aarch64.exe',
    r'C:\Program Files (x86)\qemu\qemu-system-aarch64.exe',
    os.path.expanduser(r'~\scoop\apps\qemu\current\qemu-system-aarch64.exe'),
    r'C:\msys64\mingw64\bin\qemu-system-aarch64.exe',
    r'C:\msys64\ucrt64\bin\qemu-system-aarch64.exe',
)


def find_qemu_binary():
    """Locate qemu-system-aarch64 in PATH or standard Windows install locations."""
    qemu_bin = shutil.which('qemu-system-aarch64')
    if qemu_bin:
        return qemu_bin
    for candidate in WINDOWS_QEMU_PATHS:
        if candidate and os.path.isfile(candidate):
            return candidate
    return None


def check_qemu_dependency(cpu='cortex-a76'):
    """Verify qemu-system-aarch64 binary and CPU model support."""
    qemu_bin = find_qemu_binary()
    if not qemu_bin:
        print('[!] Error: qemu-system-aarch64 not found in PATH.', file=sys.stderr)
        print('    Install instructions:', file=sys.stderr)
        print('      Windows:       winget install SoftwareFreedomConservancy.QEMU', file=sys.stderr)
        print('      Arch Linux:    sudo pacman -S qemu-system-aarch64', file=sys.stderr)
        print('      Ubuntu/Debian: sudo apt install qemu-system-arm', file=sys.stderr)
        print('      Fedora:        sudo dnf install qemu-system-aarch64', file=sys.stderr)
        print('      macOS:         brew install qemu', file=sys.stderr)
        return False
    try:
        ver = subprocess.check_output([qemu_bin, '--version'], text=True).splitlines()[0]
        print(f'[*] Found QEMU: {ver} ({qemu_bin})', file=sys.stderr)
    except Exception as e:
        print(f'[!] Warning: Failed to query QEMU version: {e}', file=sys.stderr)
        return False

    try:
        cpu_list = subprocess.check_output([qemu_bin, '-M', 'virt', '-cpu', 'help'], text=True)
        if cpu in cpu_list:
            print(f'[*] CPU model {cpu} supported', file=sys.stderr)
        else:
            print(f'[!] Warning: CPU model {cpu} not explicitly listed in qemu-system-aarch64 -cpu help', file=sys.stderr)
    except Exception:
        pass
    return True


def find_macho_symbol(kernel_path, sym_name=b'_iso_font'):
    """Find a symbol VA in an ARM64 Mach-O kernel using LC_SYMTAB (pure Python fallback for llvm-nm)."""
    data = open(kernel_path, 'rb').read()
    magic, _, _, _, ncmds, _, _, _ = struct.unpack_from('<8I', data, 0)
    if magic != 0xfeedfacf:
        return []
    off = 32
    for _ in range(ncmds):
        cmd, cmdsize = struct.unpack_from('<II', data, off)
        if cmd == 0x2:  # LC_SYMTAB
            symoff, nsyms, stroff, strsize = struct.unpack_from('<4I', data, off + 8)
            matches = []
            target = sym_name + b'\0'
            for i in range(nsyms):
                strx, ntype, _, _, value = struct.unpack_from('<IBBHQ', data, symoff + 16 * i)
                if (ntype & 0x0e) != 0 and data[stroff + strx:stroff + strx + len(target)] == target:
                    matches.append(value)
            return matches
        off += cmdsize
    return []


def check_kernel_tools(kernel_path):
    """Verify kernel file exists (uses llvm-nm if available, or pure-Python Mach-O LC_SYMTAB parser)."""
    if not os.path.isfile(kernel_path):
        print(f'[!] Error: kernel file does not exist: {kernel_path}', file=sys.stderr)
        return False
    return True


def get_avail_commit_bytes():
    """Return available host commit charge in bytes on Windows (or 0 on POSIX)."""
    if os.name != 'nt':
        return 0
    try:
        import ctypes
        class MEMORYSTATUSEX(ctypes.Structure):
            _fields_ = [
                ('dwLength', ctypes.c_ulong),
                ('dwMemoryLoad', ctypes.c_ulong),
                ('ullTotalPhys', ctypes.c_ulonglong),
                ('ullAvailPhys', ctypes.c_ulonglong),
                ('ullTotalPageFile', ctypes.c_ulonglong),
                ('ullAvailPageFile', ctypes.c_ulonglong),
                ('ullTotalVirtual', ctypes.c_ulonglong),
                ('ullAvailVirtual', ctypes.c_ulonglong),
                ('ullAvailExtendedVirtual', ctypes.c_ulonglong),
            ]
        st = MEMORYSTATUSEX()
        st.dwLength = ctypes.sizeof(st)
        if ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(st)):
            return int(st.ullAvailPageFile)
    except Exception:
        pass
    return 0


def build_image(payload, req_ram_gb=4, req_smp=None):
    """Return (phys_base, ram_image, boot_args_pa, entry_pa, mem_size)."""
    magic, ver, nsegs = struct.unpack_from('<8sII', payload, 0)
    assert magic == b'SWEETXNU' and ver == 1, 'not a SWEETXNU v1 payload'
    virt_base, entry_va, adt_off, adt_size, kernel_end, phys_base, mem_size = \
        struct.unpack_from('<7Q', payload, 0x10)
    if req_ram_gb and int(req_ram_gb) > 4:
        mem_size = int(req_ram_gb) << 30
    avail_commit = get_avail_commit_bytes()
    if avail_commit > 0:
        low_hole = max(0, phys_base - 0x40000000)
        safe_total = max(4 << 30, (avail_commit - (768 << 20)) & ~((64 << 20) - 1))
        if low_hole + mem_size > safe_total and safe_total > low_hole + (2 << 30):
            mem_size = safe_total - low_hole
    cmdline = bytearray(payload[0x48:0x48 + BOOT_LINE_LENGTH])
    if req_smp is not None:
        cmd_str = cmdline.split(b'\0', 1)[0].decode('ascii', errors='replace')
        import re
        cmd_str = re.sub(r'\bcpus=\d+', f'cpus={int(req_smp)}', cmd_str)
        cmd_bytes = cmd_str.encode('ascii')[:BOOT_LINE_LENGTH - 1]
        cmdline = cmd_bytes + b'\0' * (BOOT_LINE_LENGTH - len(cmd_bytes))
    delta = phys_base - virt_base
    adt_pa = kernel_end + delta
    ba_pa = (adt_pa + adt_size + PAGE - 1) & ~(PAGE - 1)
    top = ba_pa + PAGE
    img = bytearray(top - phys_base)
    for i in range(nsegs):
        vmaddr, vmsize, foff, fsize = struct.unpack_from('<4Q', payload, 0x2a8 + 32 * i)
        o = vmaddr + delta - phys_base
        img[o:o + fsize] = payload[foff:foff + fsize]
    adt_blob = bytearray(payload[adt_off:adt_off + adt_size])
    dram_pat = b'dram-size' + b'\x00' * 23 + struct.pack('<I', 8)
    dram_idx = adt_blob.find(dram_pat)
    if dram_idx != -1:
        struct.pack_into('<Q', adt_blob, dram_idx + 36, mem_size)
    img[adt_pa - phys_base:adt_pa - phys_base + adt_size] = adt_blob
    ba = struct.pack('<HH4x4Q6QI4xQI', 2, 2, virt_base, phys_base, mem_size, top,
                     FB_BASE, 1, FB_STRIDE, FB_W, FB_H, 32, 0, adt_pa - delta, adt_size)
    ba += bytes(cmdline) + b'\0' * 4 + struct.pack('<QQ', 0, mem_size)
    assert len(ba) == 736
    img[ba_pa - phys_base:ba_pa - phys_base + len(ba)] = ba
    return phys_base, bytes(img), ba_pa, entry_va + delta, mem_size


def stub(ba_pa, entry_pa):
    # ldr x0, =ba ; ldr x1, =entry ; br x1 ; nop ; .quad ba ; .quad entry
    return struct.pack('<4I2Q', 0x58000080, 0x580000a1, 0xd61f0020, 0xd503201f, ba_pa, entry_pa)


def write_png(path, raw):
    rows = []
    for y in range(FB_H):
        line = raw[y * FB_STRIDE:(y + 1) * FB_STRIDE]
        rgb = bytearray(FB_W * 3)	# a8r8g8b8 little endian = B G R A in memory
        rgb[0::3], rgb[1::3], rgb[2::3] = line[2::4], line[1::4], line[0::4]
        rows.append(b'\0' + bytes(rgb))
    chunk = lambda t, d: struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d))
    png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', FB_W, FB_H, 8, 2, 0, 0, 0))
    png += chunk(b'IDAT', zlib.compress(b''.join(rows), 6)) + chunk(b'IEND', b'')
    open(path, 'wb').write(png)


def fb_to_ppm(raw, scale=3):
    """Convert raw 1080x2400 BGRA framebuffer bytes into a downscaled P6 binary PPM
    suitable for direct loading into Tkinter's PhotoImage(data=...)."""
    scale = max(1, int(scale))
    out_w = FB_W // scale
    out_h = FB_H // scale
    step = scale * 4
    row_limit = out_w * step
    rgb = bytearray(out_w * out_h * 3)
    row_rgb_len = out_w * 3
    if len(raw) < FB_SIZE_BYTES:
        raw = raw + b'\x00' * (FB_SIZE_BYTES - len(raw))
    for dst_y in range(out_h):
        src_y = dst_y * scale
        line = raw[src_y * FB_STRIDE:src_y * FB_STRIDE + row_limit]
        base = dst_y * row_rgb_len
        rgb[base + 0:base + row_rgb_len:3] = line[2::step]
        rgb[base + 1:base + row_rgb_len:3] = line[1::step]
        rgb[base + 2:base + row_rgb_len:3] = line[0::step]
    header = f'P6\n{out_w} {out_h}\n255\n'.encode('ascii')
    return header + bytes(rgb)


def payload_data(payload, va, size):
    """Read initialized segment bytes by VA (the payload has no slide)."""
    nsegs = struct.unpack_from('<I', payload, 12)[0]
    for i in range(nsegs):
        vmaddr, _, foff, fsize = struct.unpack_from('<4Q', payload, 0x2a8 + 32 * i)
        if vmaddr <= va and va + size <= vmaddr + fsize:
            offset = foff + va - vmaddr
            return payload[offset:offset + size]
    raise ValueError(f'VA {va:#x}+{size:#x} not in initialized payload segments')


def decode_screen(raw, font):
    """Exact glyph matching for pe_serial.c's 2x, white-on-black fbcon."""
    glyphs = {font[c * 16:(c + 1) * 16]: chr(c) for c in range(32, 127)}
    rows, unknown = [], 0
    for row in range(FB_H // 32):
        chars = []
        for col in range(FB_W // 16):
            glyph = bytearray(16)
            for y in range(16):
                offset = (row * 32 + y * 2) * FB_STRIDE + col * 16 * 4
                for x in range(8):
                    if raw[offset + x * 8:offset + x * 8 + 4] == b'\xff' * 4:
                        glyph[y] |= 1 << x
            key = bytes(glyph)
            char = glyphs.get(key, '?')
            unknown += key not in glyphs
            chars.append(char)
        rows.append(''.join(chars).rstrip())
    return '\n'.join(rows).rstrip() + '\n', unknown


def screen_result(screen, unknown):
    if 'panic(cpu ' in screen or 'SWEET EARLY PANIC:' in screen:
        status = 'panic'
    elif unknown:
        status = 'incomplete_decode'
    elif screen.strip():
        status = 'no_panic_observed'
    else:
        status = 'blank_screen'
    return {'status': status, 'unknown_glyphs': unknown, 'snapshot_only': True}


def find_free_port(default=4444):
    """Find an available local TCP port for QMP on Windows or when requested."""
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
            s.bind(('127.0.0.1', 0))
            return s.getsockname()[1]
    except Exception:
        return default


def _parse_qmp_target(target):
    """Return ('tcp', (host, port)) or ('unix', sock_path)."""
    if isinstance(target, int):
        return 'tcp', ('127.0.0.1', target)
    if isinstance(target, (tuple, list)) and len(target) == 2:
        return 'tcp', (str(target[0]), int(target[1]))
    if isinstance(target, str):
        t = target.strip()
        if t.startswith('tcp:'):
            t = t[4:].split(',', 1)[0]
        if ':' in t and not os.path.isabs(t) and not t.startswith('/') and not t.startswith('.'):
            host, port_s = t.rsplit(':', 1)
            if port_s.isdigit():
                return 'tcp', (host or '127.0.0.1', int(port_s))
    return 'unix', str(target)


class QMPClient:
    """Cross-platform QEMU Machine Protocol (QMP) client supporting TCP (Windows/any OS)
    and AF_UNIX domain sockets (POSIX)."""

    def __init__(self, target, timeout=5.0):
        self.target = target
        self.timeout = timeout
        self.sock = None
        self.file = None

    def connect(self, retries=15, delay=0.1):
        mode, addr = _parse_qmp_target(self.target)
        last_err = None
        for _ in range(max(1, retries)):
            try:
                if mode == 'tcp':
                    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
                    try:
                        s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                    except Exception:
                        pass
                else:
                    if not hasattr(socket, 'AF_UNIX'):
                        raise RuntimeError('AF_UNIX sockets are not supported on this platform; use TCP QMP')
                    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                s.settimeout(self.timeout)
                s.connect(addr)
                self.sock = s
                self.file = s.makefile('rw', encoding='utf-8', errors='replace')
                greeting = self.file.readline()
                if not greeting:
                    raise ConnectionError('Empty QMP greeting from QEMU')
                self.execute('qmp_capabilities')
                return self
            except Exception as e:
                last_err = e
                try:
                    s.close()
                except Exception:
                    pass
                time.sleep(delay)
        raise ConnectionError(f'Failed to connect to QMP ({self.target}): {last_err}')

    def execute(self, cmd, **arguments):
        if not self.file:
            self.connect()
        payload = {'execute': cmd}
        if arguments:
            payload['arguments'] = arguments
        self.file.write(json.dumps(payload) + '\n')
        self.file.flush()
        while True:
            line = self.file.readline()
            if not line:
                raise ConnectionError('QMP connection closed by QEMU')
            resp = json.loads(line)
            if 'return' in resp or 'error' in resp:
                return resp

    def hmp(self, command_line):
        return self.execute('human-monitor-command', **{'command-line': command_line})

    def read_u64(self, pa):
        r = self.hmp(f'xp /1gx 0x{int(pa):x}')
        txt = r.get('return', '')
        if ':' in txt:
            toks = txt.split(':', 1)[1].strip().split()
            if toks:
                try:
                    return int(toks[0], 16)
                except ValueError:
                    pass
        return None

    def pmemsave(self, val, size, filename):
        return self.execute('pmemsave', val=int(val), size=int(size), filename=str(filename))

    def close(self):
        if self.file:
            try:
                self.file.close()
            except Exception:
                pass
            self.file = None
        if self.sock:
            try:
                self.sock.close()
            except Exception:
                pass
            self.sock = None

    def __enter__(self):
        return self.connect()

    def __exit__(self, exc_type, exc_val, exc_tb):
        self.close()


def qmp(sock_path, *cmds, pmemsave=None):
    """Run QEMU human-monitor commands over QMP (supports both AF_UNIX path and TCP target)."""
    out = []
    with QMPClient(sock_path) as client:
        for c in cmds:
            r = client.hmp(c)
            if 'error' in r:
                print(f'qmp {c!r}: {r["error"]}', file=sys.stderr)
            out.append(r.get('return', ''))
        if pmemsave:	# native QMP: HMP's argument parser chokes on '-' in paths
            r = client.pmemsave(pmemsave[0], pmemsave[1], pmemsave[2])
            if 'error' in r:
                print(f'qmp pmemsave: {r["error"]}', file=sys.stderr)
    return out


class QemuProcessRamReader:
    """Zero-BQL, Zero-Disk-I/O direct guest RAM reader on Windows via ReadProcessMemory.
    Locates QEMU's contiguous guest RAM allocation (PA 0x40000000) by matching the stub
    at PA 0x40200000 (offset +0x200000 == 0x58000080) and reads FB_BASE (0x9c000000) in ~2.8 ms
    without stalling QEMU vCPUs under the Big QEMU Lock."""

    def __init__(self, pid, ram_gib=4):
        self.pid = pid
        self.ram_bytes = max(1, int(ram_gib)) << 30
        self.handle = None
        self.ram_host_base = None
        self._fb_buf = None
        self._fb_pkts = None
        self._fb_pkt_idx = 0
        self._mbox_hdr_buf = None
        self._init_win32()

    def _init_win32(self):
        if os.name != 'nt' or not self.pid:
            return
        try:
            import ctypes
            from ctypes import wintypes
            self._ctypes = ctypes
            self._k32 = ctypes.windll.kernel32
            self._ntdll = ctypes.windll.ntdll

            class MEMORY_BASIC_INFORMATION(ctypes.Structure):
                _fields_ = [
                    ('BaseAddress', ctypes.c_void_p),
                    ('AllocationBase', ctypes.c_void_p),
                    ('AllocationProtect', wintypes.DWORD),
                    ('PartitionId', wintypes.WORD),
                    ('RegionSize', ctypes.c_size_t),
                    ('State', wintypes.DWORD),
                    ('Protect', wintypes.DWORD),
                    ('Type', wintypes.DWORD),
                ]

            self._MBI = MEMORY_BASIC_INFORMATION
            # 0x0438 = PROCESS_VM_READ | PROCESS_VM_WRITE | PROCESS_VM_OPERATION | PROCESS_QUERY_INFORMATION
            self.handle = self._k32.OpenProcess(0x0438, False, int(self.pid))
            if not self.handle:
                self.handle = self._k32.OpenProcess(0x0410, False, int(self.pid))
            self._fb_buf = ctypes.create_string_buffer(FB_SIZE_BYTES)
            hdr = b'SWFB' + struct.pack('<I', FB_SIZE_BYTES)
            pkt_a = bytearray(8 + FB_SIZE_BYTES)
            pkt_b = bytearray(8 + FB_SIZE_BYTES)
            pkt_a[:8] = hdr
            pkt_b[:8] = hdr
            self._fb_pkts = (pkt_a, pkt_b)
            self._mbox_hdr_buf = ctypes.create_string_buffer(16)
        except Exception:
            self.handle = None

    def ensure_base(self):
        if self.ram_host_base is not None:
            return True
        if not self.handle:
            return False
        try:
            ctypes = self._ctypes
            mbi = self._MBI()
            mbi_sz = ctypes.sizeof(mbi)
            addr = 0
            last_alloc_base = 0
            chk = ctypes.create_string_buffer(16)
            nread = ctypes.c_size_t(0)
            want_stub = (0x58000080, 0x580000a1, 0xd61f0020, 0xd503201f)
            while self._k32.VirtualQueryEx(self.handle, ctypes.c_void_p(addr), ctypes.byref(mbi), mbi_sz):
                base = mbi.BaseAddress or 0
                alloc_base = mbi.AllocationBase or 0
                rsize = mbi.RegionSize or 0
                if mbi.State == 0x1000 and rsize >= (3 << 30):
                    for cand in (base, alloc_base):
                        if not cand:
                            continue
                        if self._k32.ReadProcessMemory(
                            self.handle,
                            ctypes.c_void_p(cand + (STUB_PA - 0x40000000)),
                            chk,
                            16,
                            ctypes.byref(nread),
                        ) and nread.value == 16:
                            if struct.unpack('<4I', chk.raw) == want_stub:
                                self.ram_host_base = cand
                                return True
                addr = base + max(4096, rsize)
                if addr >= (1 << 47):
                    break
        except Exception:
            pass
        return False

    def read_fb_packet(self):
        """Read FB_BASE directly into a double-buffered pre-framed SWFB packet buffer (zero-alloc)."""
        if not self.ensure_base() or self._fb_pkts is None:
            return None, None
        try:
            ctypes = self._ctypes
            host_va = self.ram_host_base + (FB_BASE - 0x40000000)
            nread = ctypes.c_size_t(0)
            pkt_buf = self._fb_pkts[self._fb_pkt_idx]
            c_target = (ctypes.c_char * FB_SIZE_BYTES).from_buffer(pkt_buf, 8)
            ok = self._k32.ReadProcessMemory(
                self.handle,
                ctypes.c_void_p(host_va),
                ctypes.byref(c_target),
                FB_SIZE_BYTES,
                ctypes.byref(nread),
            )
            if ok and nread.value == FB_SIZE_BYTES:
                self._fb_pkt_idx ^= 1
                mv = memoryview(pkt_buf)
                return mv, mv[8:]
        except Exception:
            pass
        return None, None

    def read_pa(self, pa, size):
        if not self.ensure_base() or pa < 0x40000000:
            return None
        try:
            ctypes = self._ctypes
            host_va = self.ram_host_base + (pa - 0x40000000)
            nread = ctypes.c_size_t(0)
            if size == FB_SIZE_BYTES and self._fb_buf is not None:
                ok = self._k32.ReadProcessMemory(
                    self.handle,
                    ctypes.c_void_p(host_va),
                    self._fb_buf,
                    size,
                    ctypes.byref(nread),
                )
                if ok and nread.value == size:
                    return memoryview(self._fb_buf)[:size]
                return None
            buf = ctypes.create_string_buffer(size)
            ok = self._k32.ReadProcessMemory(
                self.handle,
                ctypes.c_void_p(host_va),
                buf,
                size,
                ctypes.byref(nread),
            )
            if ok and nread.value == size:
                return buf.raw
        except Exception:
            pass
        return None

    def write_pa(self, pa, data_bytes):
        if not self.ensure_base() or pa < 0x40000000:
            return False
        try:
            ctypes = self._ctypes
            host_va = self.ram_host_base + (pa - 0x40000000)
            nwritten = ctypes.c_size_t(0)
            n = len(data_bytes)
            st = self._ntdll.NtWriteVirtualMemory(
                self.handle,
                ctypes.c_void_p(host_va),
                data_bytes,
                n,
                ctypes.byref(nwritten),
            )
            return bool(st == 0 and nwritten.value == n)
        except Exception:
            return False

    def enqueue_input(self, ev_type, ev_id, down, x=0, y=0):
        """Directly enqueue an input event into guest RAM at INPUT_MBOX_PA (0x9d600000) in ~4us."""
        if not self.ensure_base() or self._mbox_hdr_buf is None:
            return False
        try:
            ctypes = self._ctypes
            host_va = self.ram_host_base + (INPUT_MBOX_PA - 0x40000000)
            nread = ctypes.c_size_t(0)
            if not self._k32.ReadProcessMemory(
                self.handle,
                ctypes.c_void_p(host_va),
                self._mbox_hdr_buf,
                16,
                ctypes.byref(nread),
            ) or nread.value != 16:
                return False
            magic, w_idx, r_idx, _ = struct.unpack_from('<4I', self._mbox_hdr_buf.raw, 0)
            if magic != INPUT_MBOX_MAGIC:
                return False
            if ((w_idx - r_idx) & 0xffffffff) >= INPUT_MBOX_RING_SIZE:
                return False
            cx = max(0, min(FB_W - 1, int(x)))
            cy = max(0, min(FB_H - 1, int(y)))
            ev_bytes = struct.pack(
                '<HHiii',
                int(ev_type) & 0xffff,
                int(ev_id) & 0xffff,
                1 if down else 0,
                cx,
                cy,
            )
            pending = (w_idx - r_idx) & 0xffffffff
            if int(ev_type) == 1 and down and 8 <= pending < INPUT_MBOX_RING_SIZE:
                prev_slot = (w_idx - 1) % INPUT_MBOX_RING_SIZE
                prev_off = INPUT_MBOX_HEADER_SIZE + prev_slot * INPUT_MBOX_ENTRY_SIZE
                prev_raw = self.read_pa(INPUT_MBOX_PA + prev_off, 8)
                if prev_raw and len(prev_raw) == 8:
                    p_type, _, p_down = struct.unpack_from('<HHi', prev_raw, 0)
                    if p_type == 1 and p_down != 0:
                        return self.write_pa(INPUT_MBOX_PA + prev_off, ev_bytes)
            slot = w_idx % INPUT_MBOX_RING_SIZE
            off = INPUT_MBOX_HEADER_SIZE + slot * INPUT_MBOX_ENTRY_SIZE
            if not self.write_pa(INPUT_MBOX_PA + off, ev_bytes):
                return False
            new_w = struct.pack('<I', (w_idx + 1) & 0xffffffff)
            return self.write_pa(INPUT_MBOX_PA + 4, new_w)
        except Exception:
            return False

    def read_u64(self, pa):
        raw = self.read_pa(pa, 8)
        if raw and len(raw) == 8:
            return struct.unpack('<Q', raw)[0]
        return None

    def close(self):
        if self.handle:
            try:
                self._k32.CloseHandle(self.handle)
            except Exception:
                pass
            self.handle = None


class SweetInputMailbox:
    """Shared-memory input mailbox encoder & QMP injector at physical address 0x9d600000.

    Matches bsd/dev/sweet_virtio.c:
      0x00: uint32_t magic      (0x5357494e 'SWIN')
      0x04: uint32_t write_idx
      0x08: uint32_t read_idx
      0x0c: uint32_t flags
      0x10 + (idx % 64) * 16:
        struct { uint16_t type; uint16_t id; int32_t down; int32_t x; int32_t y; }
    """

    def __init__(self, qmp_target=None, base_pa=INPUT_MBOX_PA):
        self.qmp_target = qmp_target
        self.base_pa = base_pa
        self.write_idx = 0
        self.read_idx = 0
        self.flags = 0
        self.buffer = bytearray(INPUT_MBOX_TOTAL_SIZE)
        self._sync_header()

    def _sync_header(self):
        struct.pack_into(
            '<4I',
            self.buffer,
            0,
            INPUT_MBOX_MAGIC,
            self.write_idx & 0xffffffff,
            self.read_idx & 0xffffffff,
            self.flags & 0xffffffff,
        )

    def write_initial_file(self, path):
        """Write the initial 1040-byte mailbox image for QEMU -device loader."""
        self._sync_header()
        with open(path, 'wb') as f:
            f.write(self.buffer)
        return path

    @staticmethod
    def clamp_coords(x, y):
        return max(0, min(FB_W - 1, int(round(x)))), max(0, min(FB_H - 1, int(round(y))))

    @staticmethod
    def pack_event(ev_type, ev_id, down, x=0, y=0):
        return struct.pack('<HHiii', int(ev_type) & 0xffff, int(ev_id) & 0xffff,
                           1 if down else 0, int(x), int(y))

    @staticmethod
    def unpack_event(buf, offset=0):
        ev_type, ev_id, down, x, y = struct.unpack_from('<HHiii', buf, offset)
        return {'type': ev_type, 'id': ev_id, 'down': down, 'x': x, 'y': y}

    def read_header(self):
        magic, w_idx, r_idx, flags = struct.unpack_from('<4I', self.buffer, 0)
        return {'magic': magic, 'write_idx': w_idx, 'read_idx': r_idx, 'flags': flags}

    def read_slot(self, idx):
        slot = int(idx) % INPUT_MBOX_RING_SIZE
        off = INPUT_MBOX_HEADER_SIZE + slot * INPUT_MBOX_ENTRY_SIZE
        return self.unpack_event(self.buffer, off)

    def _enqueue_event(self, ev_type, ev_id, down, x=0, y=0):
        slot = self.write_idx % INPUT_MBOX_RING_SIZE
        off = INPUT_MBOX_HEADER_SIZE + slot * INPUT_MBOX_ENTRY_SIZE
        ev_bytes = self.pack_event(ev_type, ev_id, down, x, y)
        self.buffer[off:off + INPUT_MBOX_ENTRY_SIZE] = ev_bytes
        self.write_idx = (self.write_idx + 1) & 0xffffffff
        self._sync_header()

        slot_pa = self.base_pa + off
        if self.qmp_target is not None:
            self._push_to_qmp(ev_type, ev_id, down, x, y, slot_pa, ev_bytes)

        return {
            'slot': slot,
            'slot_pa': slot_pa,
            'write_idx': self.write_idx,
            'type': ev_type,
            'id': ev_id,
            'down': 1 if down else 0,
            'x': int(x),
            'y': int(y),
        }

    def _push_to_qmp(self, ev_type, ev_id, down, x, y, slot_pa, ev_bytes):
        """Send input event to a running QEMU instance over QMP."""
        try:
            with QMPClient(self.qmp_target, timeout=1.5) as client:
                # Probe physical mailbox header via HMP xp and also dispatch QMP input-send-event
                # when virtio-tablet / virtio-keyboard devices are attached.
                client.hmp(f'xp /4wx {self.base_pa:#x}')
                if ev_type == INPUT_TYPE_TOUCH:
                    abs_x = int(x * 32767 // max(1, FB_W - 1))
                    abs_y = int(y * 32767 // max(1, FB_H - 1))
                    events = [
                        {'type': 'abs', 'data': {'axis': 'x', 'value': abs_x}},
                        {'type': 'abs', 'data': {'axis': 'y', 'value': abs_y}},
                        {'type': 'btn', 'data': {'down': bool(down), 'button': 'left'}},
                    ]
                    client.execute('input-send-event', events=events)
                elif ev_type == INPUT_TYPE_KEY:
                    qcode_map = {
                        KEY_POWER: 'power',
                        KEY_VOL_UP: 'volumeup',
                        KEY_VOL_DOWN: 'volumedown',
                        KEY_HOME: 'ac_home',
                    }
                    qcode = qcode_map.get(ev_id, 'esc')
                    events = [{
                        'type': 'key',
                        'data': {'down': bool(down), 'key': {'type': 'qcode', 'data': qcode}},
                    }]
                    client.execute('input-send-event', events=events)
        except Exception:
            pass

    def send_touch(self, finger, down, x, y):
        cx, cy = self.clamp_coords(x, y)
        return self._enqueue_event(INPUT_TYPE_TOUCH, int(finger) % 10, down, cx, cy)

    def send_key(self, key_id, down):
        return self._enqueue_event(INPUT_TYPE_KEY, int(key_id), down, 0, 0)

    def press_key(self, key_id):
        down_ev = self.send_key(key_id, 1)
        up_ev = self.send_key(key_id, 0)
        return [down_ev, up_ev]

    def tap(self, x, y, finger=0):
        down_ev = self.send_touch(finger, 1, x, y)
        up_ev = self.send_touch(finger, 0, x, y)
        return [down_ev, up_ev]

    def swipe(self, x0, y0, x1, y1, steps=8, finger=0):
        steps = max(2, int(steps))
        events = []
        for i in range(steps):
            t = i / (steps - 1)
            xi = int(round(x0 + (x1 - x0) * t))
            yi = int(round(y0 + (y1 - y0) * t))
            events.append(self.send_touch(finger, 1, xi, yi))
        events.append(self.send_touch(finger, 0, x1, y1))
        return events

    def home_gesture(self, finger=0):
        """Trigger the iPhone X bottom-edge swipe-up home gesture (starts at y=2380 >= 2352
        so patch 0071 sets kIOHIDDigitizerEventFromEdgeTip)."""
        return self.swipe(FB_W // 2, 2380, FB_W // 2, 1650, steps=6, finger=finger)


def send_touch_event(qmp_target, finger, down, x, y, mailbox=None):
    """Helper to write a touch event into the mailbox and dispatch via QMP."""
    mbox = mailbox or SweetInputMailbox(qmp_target=qmp_target)
    if qmp_target is not None:
        mbox.qmp_target = qmp_target
    return mbox.send_touch(finger, down, x, y)


def send_key_event(qmp_target, key_id, down, mailbox=None):
    """Helper to write a hardware key event into the mailbox and dispatch via QMP."""
    mbox = mailbox or SweetInputMailbox(qmp_target=qmp_target)
    if qmp_target is not None:
        mbox.qmp_target = qmp_target
    return mbox.send_key(key_id, down)


def capture_framebuffer(qmp_target, raw_path, png_path=None):
    """Capture the live 1080x2400 framebuffer from FB_BASE (0x9c000000) via QMP pmemsave."""
    raw_path = os.path.abspath(raw_path)
    with QMPClient(qmp_target, timeout=3.0) as client:
        r = client.pmemsave(FB_BASE, FB_SIZE_BYTES, raw_path)
        if 'error' in r:
            raise RuntimeError(f'QMP pmemsave failed: {r["error"]}')
    with open(raw_path, 'rb') as f:
        raw = f.read()
    if png_path:
        write_png(png_path, raw)
    return raw


def build_accel_args(accel_mode, smp):
    """Determine QEMU -accel arguments based on --accel mode and --smp count."""
    mode = (accel_mode or 'auto').lower()
    if mode == 'auto':
        if smp > 1:
            return ['-accel', 'tcg,thread=multi,tb-size=1024']
        return ['-accel', 'tcg,tb-size=1024']
    if mode == 'mttcg':
        return ['-accel', 'tcg,thread=multi,tb-size=1024']
    if mode == 'tcg':
        return ['-accel', 'tcg,tb-size=1024']
    if mode in ('whpx', 'hvf', 'kvm'):
        return ['-accel', mode]
    return []


def stream_serial_file(serial_path, file_pos, out_stream=sys.stdout):
    """Read newly appended content from serial.txt and write directly to stream."""
    if not os.path.exists(serial_path):
        return file_pos
    try:
        with open(serial_path, 'r', errors='replace') as f:
            f.seek(file_pos)
            data = f.read()
            if data:
                out_stream.write(data)
                out_stream.flush()
                return f.tell()
    except Exception:
        pass
    return file_pos


def main():
    ap = argparse.ArgumentParser(
        description="Boot a SWEETXNU payload in QEMU's virt board.",
        epilog="Examples:\n"
               "  qemu-sweet.py payload.bin out/\n"
               "  qemu-sweet.py payload.bin out/ --live-serial\n"
               "  qemu-sweet.py payload.bin out/ --smp 6 --disk ios16-root.img --net --interactive\n"
               "  qemu-sweet.py payload.bin out/ --kernel kernel.macho --fail-on-panic\n"
               "  qemu-sweet.py --check-env\n",
        formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument('payload', nargs='?', help='SWEETXNU payload binary (from mkpayload.py)')
    ap.add_argument('outdir', nargs='?', help='Output directory for fb.png, regs.txt, and logs')
    ap.add_argument('--secs', type=float, default=20, help='Duration to run QEMU in seconds (default: 20)')
    ap.add_argument('--gdb', action='store_true', help='start halted, gdb stub on :1234, no timeout')
    ap.add_argument('--hmp', action='append', default=[], help='extra monitor command run after stop (repeatable), e.g. "x /8gx 0xfffffe..."')
    ap.add_argument('--cpu', default='cortex-a76', help='QEMU CPU model (default: cortex-a76)')
    ap.add_argument('--smp', type=int, default=1, help='Number of SMP CPU cores, 1..8 (default: 1)')
    ap.add_argument('--ram-gb', type=int, default=4, help='Guest RAM size in GiB, e.g. 4, 6, 8 (default: 4)')
    ap.add_argument('--accel', choices=['auto', 'tcg', 'mttcg', 'whpx', 'hvf', 'kvm'], default='auto',
                    help='QEMU acceleration backend (default: auto -> mttcg when smp > 1)')
    ap.add_argument('--disk', help='Path to raw HFS+ or GPT root disk image (attaches virtio-blk-device)')
    ap.add_argument('--net', action='store_true', help='Attach virtio-net-device with user networking (SSH :2222->:22, HTTP :8080->:80)')
    ap.add_argument('--display-window', action='store_true', help='Open native QEMU graphical window (ramfb + virtio-tablet/keyboard) instead of -nographic')
    ap.add_argument('--interactive', action='store_true', help='Run interactively until closed or Ctrl+C and initialize input mailbox at 0x9d600000')
    ap.add_argument('--qmp-port', type=int, default=None, help='Use TCP port on 127.0.0.1 for QMP instead of a Unix domain socket')
    ap.add_argument('--kernel', help='matching unstripped kernel, for exact framebuffer text decoding (uses llvm-nm)')
    ap.add_argument('--fail-on-panic', action='store_true', help='exit 2 for a visible panic, 3 for incomplete/blank decoding; requires --kernel')
    ap.add_argument('--live-serial', action='store_true', help='stream serial console output to stdout in real time')
    ap.add_argument('--check-env', action='store_true', help='check QEMU installation and dependencies without running a payload')
    ap.add_argument('--sync-only', action='store_true', help='synchronize HFSX plists/prefs into --disk without launching QEMU')
    a = ap.parse_args()

    if a.sync_only:
        disk_target = a.disk or os.path.join('emu_workspace', 'ios16-root-20H392.img')
        if not os.path.isfile(disk_target):
            print(f'[!] Error: disk image not found: {disk_target}', file=sys.stderr)
            return 1
        sync_hfsx_image(disk_target)
        print(f'[+] HFSX image synced: {disk_target}')
        return 0

    if a.check_env:
        ok = check_qemu_dependency(a.cpu)
        if shutil.which('llvm-nm'):
            print('[*] llvm-nm found in PATH', file=sys.stderr)
        else:
            print('[!] Optional: llvm-nm not found (needed only when using --kernel for text decoding)', file=sys.stderr)
        return 0 if ok else 1

    if not a.payload or not a.outdir:
        ap.error('payload and outdir are required unless --check-env is specified')

    if a.fail_on_panic and not a.kernel:
        ap.error('--fail-on-panic requires --kernel')

    if a.disk and not os.path.isfile(a.disk):
        ap.error(f'--disk file does not exist: {a.disk}')

    if not check_qemu_dependency(a.cpu):
        return 1

    if a.kernel and not check_kernel_tools(a.kernel):
        return 1

    qemu_bin = find_qemu_binary() or 'qemu-system-aarch64'
    payload = open(a.payload, 'rb').read()
    font = None
    if a.kernel:
        if shutil.which('llvm-nm'):
            symbols = subprocess.check_output(['llvm-nm', '--defined-only', a.kernel], text=True)
            fonts = [int(line.split()[0], 16) for line in symbols.splitlines()
                     if line.split()[-1:] == ['_iso_font']]
        else:
            fonts = find_macho_symbol(a.kernel, b'_iso_font')
        if len(fonts) != 1:
            ap.error('kernel must contain exactly one _iso_font symbol')
        font = payload_data(payload, fonts[0], 256 * 16)
    os.makedirs(a.outdir, exist_ok=True)
    out = os.path.abspath(a.outdir)

    # Remove stale artifacts from previous runs so live boot frames stream from t=0s
    for stale_name in ('fb.bin', 'fb_tmp.bin', 'fb_live.raw', 'sweet_log.txt', 'sweet_log_tmp.txt', 'result.json'):
        stale_path = os.path.join(out, stale_name)
        if os.path.exists(stale_path):
            try:
                os.unlink(stale_path)
            except OSError:
                pass

    req_ram = max(4, min(16, int(a.ram_gb)))
    smp = max(1, min(8, int(a.smp)))
    phys_base, img, ba_pa, entry_pa, mem_size = build_image(payload, req_ram_gb=req_ram, req_smp=smp)
    open(f'{out}/ram.bin', 'wb').write(img)
    open(f'{out}/stub.bin', 'wb').write(stub(ba_pa, entry_pa))
    print(f'phys_base={phys_base:#x} mem_size={mem_size:#x} boot_args={ba_pa:#x} entry={entry_pa:#x} smp={smp}', file=sys.stderr)

    # Configure QMP endpoint (TCP on Windows or when --qmp-port is specified; AF_UNIX on POSIX)
    if a.qmp_port is not None or os.name == 'nt':
        port = a.qmp_port if a.qmp_port is not None else find_free_port()
        qmp_arg = f'tcp:127.0.0.1:{port},server,nowait'
        qmp_target = ('127.0.0.1', port)
        mon = f'127.0.0.1:{port}'
    else:
        mon = f'{out}/mon.sock'
        if os.path.exists(mon):
            os.unlink(mon)
        qmp_arg = f'unix:{mon},server,nowait'
        qmp_target = mon

    # virt RAM starts at 0x40000000: cover the payload's RAM (phys_base + mem_size - 0x40000000).
    ram_mib = max(4096, -(-(phys_base + mem_size - 0x40000000) // (1 << 20)))
    ram_gib = -(-ram_mib // 1024)

    cmd = [qemu_bin, '-M', 'virt,gic-version=3', '-cpu', a.cpu, '-smp', str(smp)]
    cmd += build_accel_args(a.accel, smp)
    cmd += ['-m', f'{ram_mib}M']

    if a.display_window:
        cmd += ['-device', 'ramfb', '-device', 'virtio-tablet-device', '-device', 'virtio-keyboard-device']
    else:
        cmd += ['-nographic']

    if a.disk:
        try:
            sync_hfsx_image(a.disk)
        except Exception:
            pass

    ser_port = find_free_port(default=4445)
    open(f'{out}/serial.txt', 'wb').close()

    cmd += [
        '-serial', f'tcp:127.0.0.1:{ser_port},server,nowait',
        '-qmp', qmp_arg,
    ]
    if not a.interactive:
        cmd += [
            '-d', 'guest_errors',
            '-D', f'{out}/qemu.log',
        ]
    cmd += [
        '-no-reboot',
        '-device', f'loader,file={out}/ram.bin,addr={phys_base:#x},force-raw=on',
        '-device', f'loader,file={out}/stub.bin,addr={STUB_PA:#x},force-raw=on',
        '-device', f'loader,addr={STUB_PA:#x},cpu-num=0',
    ]

    mbox = SweetInputMailbox(qmp_target=qmp_target)
    mbox_file = f'{out}/mbox.bin'
    mbox.write_initial_file(mbox_file)
    cmd += ['-device', f'loader,file={mbox_file},addr={INPUT_MBOX_PA:#x},force-raw=on']

    if a.disk:
        disk_abs = os.path.abspath(a.disk)
        cmd += [
            '-drive', f'if=none,id=iosroot,format=raw,cache=unsafe,aio=threads,file={disk_abs}',
            '-device', 'virtio-blk-device,drive=iosroot',
        ]

    if a.net:
        cmd += [
            '-netdev', 'user,id=net0,hostfwd=tcp::2222-:22,hostfwd=tcp::8080-:80',
            '-device', 'virtio-net-device,netdev=net0',
        ]

    if a.gdb:
        cmd += ['-S', '-gdb', 'tcp::1234']

    p = None
    serial_file = f'{out}/serial.txt'
    serial_pos = 0
    last_nonblank_fb = None

    tot_pa = None
    ring_pa = None
    swap_pa = None
    if a.kernel:
        try:
            virt_base = struct.unpack_from('<Q', payload, 0x10)[0]
            log_tot_vas = find_macho_symbol(a.kernel, b'_sweet_log_total')
            log_ring_vas = find_macho_symbol(a.kernel, b'_sweet_log_ring')
            if log_tot_vas and log_ring_vas:
                tot_pa = log_tot_vas[0] - virt_base + phys_base
                ring_pa = log_ring_vas[0] - virt_base + phys_base
            swap_vas = find_macho_symbol(a.kernel, b'__ZL19sweet_fb_swap_count')
            if swap_vas:
                swap_pa = swap_vas[0] - virt_base + phys_base
        except Exception:
            pass

    try:
        p = subprocess.Popen(cmd, stdin=subprocess.DEVNULL)
        if a.gdb:
            print('gdb stub on :1234; QMP:', mon, file=sys.stderr)
            p.wait()
            return 0

        t0 = time.time()
        last_snap = t0
        last_print = t0
        ser_sock = None
        mbox_forwarded_idx = 0
        last_unlock_attempt = 0
        spotlight_seen_at = 0
        tapped_settings = False
        last_input_time = 0.0

        def ensure_ser_sock():
            nonlocal ser_sock
            if ser_sock is None:
                try:
                    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
                    try:
                        s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                    except Exception:
                        pass
                    s.settimeout(0.2)
                    s.connect(('127.0.0.1', ser_port))
                    s.setblocking(False)
                    ser_sock = s
                except Exception:
                    ser_sock = None

        def send_uart_pkt(ev_type, ev_id, down, x=0, y=0):
            ensure_ser_sock()
            if ser_sock is not None:
                pkt = struct.pack(
                    '<BBBBHH',
                    0xa5,
                    int(ev_type) & 0xff,
                    int(ev_id) & 0xff,
                    1 if down else 0,
                    max(0, min(FB_W - 1, int(x))),
                    max(0, min(FB_H - 1, int(y))),
                )
                try:
                    ser_sock.sendall(pkt)
                except Exception:
                    pass

        import select
        import threading
        qmp_live = None
        ram_reader = QemuProcessRamReader(p.pid, ram_gib)
        last_written_fb = None
        last_written_pkt = None
        last_touch_down_time = 0.0
        touch_down_start = (0, 0)
        last_sent_pos = (0, 0)
        move_steps_sent = 0
        stop_input_thread = False

        def dispatch_input(ev_type, ev_id, down, x=0, y=0):
            if not ram_reader.enqueue_input(ev_type, ev_id, down, x, y):
                send_uart_pkt(ev_type, ev_id, down, x, y)

        def input_worker():
            nonlocal mbox_forwarded_idx, last_input_time, last_touch_down_time, touch_down_start, last_sent_pos, move_steps_sent
            udp_sock = None
            try:
                udp_sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                udp_sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                udp_sock.bind(('127.0.0.1', 4446))
                udp_sock.setblocking(False)
            except Exception:
                udp_sock = None

            last_mbox_check = 0.0
            last_mbox_mtime = 0.0

            while not stop_input_thread and p.poll() is None:
                if udp_sock is not None:
                    try:
                        select.select([udp_sock], [], [], 0.020)
                    except Exception:
                        time.sleep(0.005)
                else:
                    time.sleep(0.010)

                events = []
                # 1. Drain zero-latency UDP input packets from sweet-emu.exe (127.0.0.1:4446)
                if udp_sock is not None:
                    while True:
                        try:
                            pkt, _ = udp_sock.recvfrom(64)
                            if len(pkt) >= 8 and pkt[0] == 0xa5:
                                _, ev_t, ev_id, dn, ex, ey = struct.unpack_from('<BBBBHH', pkt, 0)
                                events.append((ev_t, ev_id, dn, ex, ey))
                        except Exception:
                            break

                # 2. Fallback: check mbox.bin at most 10x/sec when modified by external CLI tools
                now_inp = time.time()
                if now_inp - last_mbox_check >= 0.10:
                    last_mbox_check = now_inp
                    if os.path.exists(mbox_file):
                        try:
                            mt = os.path.getmtime(mbox_file)
                            if mt != last_mbox_mtime:
                                last_mbox_mtime = mt
                                mb_raw = open(mbox_file, 'rb').read(INPUT_MBOX_TOTAL_SIZE)
                                if len(mb_raw) >= 16:
                                    mg, w_idx, _, _ = struct.unpack_from('<4I', mb_raw, 0)
                                    if mg == INPUT_MBOX_MAGIC and w_idx != mbox_forwarded_idx:
                                        if ((w_idx - mbox_forwarded_idx) & 0xffffffff) > INPUT_MBOX_RING_SIZE:
                                            mbox_forwarded_idx = (w_idx - INPUT_MBOX_RING_SIZE) & 0xffffffff
                                        while mbox_forwarded_idx != w_idx:
                                            slot = mbox_forwarded_idx % INPUT_MBOX_RING_SIZE
                                            off = INPUT_MBOX_HEADER_SIZE + slot * INPUT_MBOX_ENTRY_SIZE
                                            if off + 16 <= len(mb_raw):
                                                events.append(struct.unpack_from('<HHiii', mb_raw, off))
                                            mbox_forwarded_idx = (mbox_forwarded_idx + 1) & 0xffffffff
                                        try:
                                            with open(mbox_file, 'r+b') as mf:
                                                mf.seek(8)
                                                mf.write(struct.pack('<I', mbox_forwarded_idx))
                                            last_mbox_mtime = os.path.getmtime(mbox_file)
                                        except Exception:
                                            pass
                        except Exception:
                            pass

                if events:
                    last_input_time = time.time()
                    for idx_e, (ev_t, ev_id, dn, ex, ey) in enumerate(events):
                        # Coalesce only sub-6px micro-jitter within the same burst
                        if (
                            ev_t == INPUT_TYPE_TOUCH
                            and dn
                            and last_touch_down_time > 0.0
                            and idx_e + 1 < len(events)
                            and events[idx_e + 1][0] == INPUT_TYPE_TOUCH
                            and events[idx_e + 1][2] != 0
                            and abs(ex - events[idx_e + 1][3]) + abs(ey - events[idx_e + 1][4]) < 6
                        ):
                            continue

                        if ev_t == INPUT_TYPE_TOUCH:
                            if dn:
                                if last_touch_down_time == 0.0:
                                    last_touch_down_time = time.time()
                                    touch_down_start = (ex, ey)
                                    last_sent_pos = (ex, ey)
                                    move_steps_sent = 0
                                    dispatch_input(ev_t, ev_id, 1, ex, ey)
                                else:
                                    last_sent_pos = (ex, ey)
                                    move_steps_sent += 1
                                    dispatch_input(ev_t, ev_id, 1, ex, ey)
                            else:
                                total_dist = abs(ex - touch_down_start[0]) + abs(ey - touch_down_start[1])
                                if total_dist < 24:
                                    held = time.time() - last_touch_down_time
                                    if 0.0 < held < 0.012:
                                        time.sleep(0.012 - held)
                                last_touch_down_time = 0.0
                                move_steps_sent = 0
                                dispatch_input(ev_t, ev_id, 0, ex, ey)
                        else:
                            dispatch_input(ev_t, ev_id, dn, ex, ey)
            if udp_sock is not None:
                try:
                    udp_sock.close()
                except Exception:
                    pass

        inp_thr = threading.Thread(target=input_worker, daemon=True)
        inp_thr.start()

        fb_srv_sock = None
        fb_clients = []
        try:
            fb_srv_sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            fb_srv_sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            fb_srv_sock.bind(('127.0.0.1', 4447))
            fb_srv_sock.listen(4)
            fb_srv_sock.setblocking(False)
        except Exception:
            fb_srv_sock = None

        def broadcast_stream_packet(tag4, payload_bytes, preframed_pkt=None):
            if fb_srv_sock is not None:
                while True:
                    try:
                        c_sock, _ = fb_srv_sock.accept()
                        c_sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                        try:
                            c_sock.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, 16 * 1024 * 1024)
                        except Exception:
                            pass
                        c_sock.settimeout(0.60)
                        fb_clients.append(c_sock)
                        if last_written_pkt is not None:
                            try:
                                c_sock.sendall(last_written_pkt)
                            except Exception:
                                pass
                        elif last_written_fb is not None:
                            try:
                                c_sock.sendall(b'SWFB' + struct.pack('<I', len(last_written_fb)) + last_written_fb)
                            except Exception:
                                pass
                    except Exception:
                        break
            if not fb_clients or (not payload_bytes and preframed_pkt is None):
                return
            pkt = preframed_pkt if preframed_pkt is not None else (tag4 + struct.pack('<I', len(payload_bytes)) + bytes(payload_bytes))
            dead = []
            for c_sock in fb_clients:
                try:
                    c_sock.sendall(pkt)
                except Exception:
                    dead.append(c_sock)
            for d in dead:
                try:
                    d.close()
                except Exception:
                    pass
                fb_clients.remove(d)

        last_swap_count = -1
        last_written_hash = None
        last_log_total = 0
        last_log_data = b''
        last_pmemsave_time = 0.0
        last_disk_fb_time = 0.0
        pending_swap_capture = True

        while p.poll() is None and (a.interactive or time.time() - t0 < a.secs):
            if fb_srv_sock is not None:
                broadcast_stream_packet(b'', b'')
            if ser_sock is None and time.time() - t0 > 0.5:
                ensure_ser_sock()
            if ser_sock is not None:
                try:
                    rx = ser_sock.recv(4096)
                    if rx:
                        with open(serial_file, 'ab') as sf:
                            sf.write(rx)
                except Exception:
                    pass
            if a.live_serial:
                serial_pos = stream_serial_file(serial_file, serial_pos)
            now = time.time()
            # When SweetBridgeService (Rust Direct-RAM) is active in interactive mode after SpringBoard unlock
            # and no legacy TCP :4447 clients are connected, do not burn CPU/BQL polling 10.36MB frames in Python!
            if a.interactive and spotlight_seen_at > 0 and not fb_clients:
                time.sleep(0.25)
                continue
            snap_interval = 0.016 if (now - last_input_time < 5.0) else (0.033 if spotlight_seen_at > 0 else 0.12)
            if now - last_snap >= snap_interval:
                last_snap = now
                elapsed = int(now - t0)
                do_png = (now - last_print >= 10.0)
                if do_png:
                    last_print = now
                    print(f'[*] QEMU running: {elapsed}s / {a.secs}s ...', file=sys.stderr, flush=True)
                try:
                    tmp_fb = f'{out}/fb_live.raw'
                    need_log = (spotlight_seen_at == 0) or do_png
                    if swap_pa:
                        cur_swap = ram_reader.read_u64(swap_pa)
                        if cur_swap is None:
                            if qmp_live is None:
                                qmp_live = QMPClient(qmp_target, timeout=2.0)
                                qmp_live.connect()
                            cur_swap = qmp_live.read_u64(swap_pa)
                        if cur_swap is not None and cur_swap != last_swap_count:
                            last_swap_count = cur_swap
                            pending_swap_capture = True
                    else:
                        pending_swap_capture = True

                    # In interactive GUI mode, wait until IOMobileFramebuffer swap >= 1 so early 8x16 fbcon dump is skipped
                    gui_ready = (not a.interactive) or (not swap_pa) or (last_swap_count >= 1)
                    do_fb_capture = gui_ready and (
                        last_written_fb is None
                        or do_png
                        or (pending_swap_capture and (now - last_pmemsave_time >= (0.014 if fb_clients else 1.5)))
                        or (not a.interactive and spotlight_seen_at == 0 and (now - last_pmemsave_time >= 0.35))
                    )
                    raw_live = b''
                    live_pkt = None
                    if do_fb_capture:
                        live_pkt, raw_live_mv = ram_reader.read_fb_packet()
                        if raw_live_mv is not None:
                            raw_live = raw_live_mv
                        else:
                            if qmp_live is None:
                                qmp_live = QMPClient(qmp_target, timeout=2.0)
                                qmp_live.connect()
                            qmp_live.pmemsave(FB_BASE, FB_STRIDE * FB_H, tmp_fb)
                            if os.path.exists(tmp_fb):
                                try:
                                    raw_live = open(tmp_fb, 'rb').read()
                                    os.unlink(tmp_fb)
                                except OSError:
                                    raw_live = b''
                        pending_swap_capture = False
                        last_pmemsave_time = time.time()

                    active_fb = last_nonblank_fb
                    if len(raw_live) == FB_STRIDE * FB_H:
                        if max(raw_live[::256], default=0) > 16:
                            last_nonblank_fb = raw_live
                        active_fb = last_nonblank_fb or raw_live
                        fb_hash = zlib.crc32(active_fb)
                        if fb_hash != last_written_hash or not os.path.exists(f'{out}/fb.bin'):
                            last_written_hash = fb_hash
                            last_written_fb = active_fb
                            if live_pkt is not None and active_fb is raw_live:
                                last_written_pkt = live_pkt
                                broadcast_stream_packet(b'SWFB', active_fb, preframed_pkt=live_pkt)
                            else:
                                last_written_pkt = None
                                broadcast_stream_packet(b'SWFB', active_fb)
                            if (not fb_clients) or do_png or (now - last_disk_fb_time >= 2.5) or (not os.path.exists(f'{out}/fb.bin')):
                                last_disk_fb_time = now
                                try:
                                    tmp_bin = f'{out}/fb_tmp.bin'
                                    open(tmp_bin, 'wb').write(active_fb)
                                    os.replace(tmp_bin, f'{out}/fb.bin')
                                except OSError:
                                    pass
                        if do_png:
                            try:
                                write_png(f'{out}/fb.png', active_fb)
                            except OSError:
                                pass
                    rdata = b''
                    if need_log and tot_pa and ring_pa:
                        tot_b = ram_reader.read_u64(tot_pa)
                        if tot_b is None:
                            if qmp_live is None:
                                qmp_live = QMPClient(qmp_target, timeout=2.0)
                                qmp_live.connect()
                            tot_b = qmp_live.read_u64(tot_pa)
                        tot_b = tot_b or 0
                        if 0 < tot_b <= 64 * 1024 * 1024 and tot_b != last_log_total:
                            last_log_total = tot_b
                            if spotlight_seen_at > 0 and tot_b > 8192 and tot_b <= 4 * 1024 * 1024:
                                tail_off = tot_b - 8192
                                rdata = ram_reader.read_pa(ring_pa + tail_off, 8192) or b''
                                if not rdata:
                                    qmp_live.pmemsave(ring_pa + tail_off, 8192, f'{out}/log_ring_live.raw')
                            else:
                                save_sz = min(tot_b, 4 * 1024 * 1024)
                                rdata = ram_reader.read_pa(ring_pa, save_sz) or b''
                                if not rdata:
                                    qmp_live.pmemsave(ring_pa, save_sz, f'{out}/log_ring_live.raw')
                            if not rdata and os.path.exists(f'{out}/log_ring_live.raw'):
                                try:
                                    rdata = open(f'{out}/log_ring_live.raw', 'rb').read()
                                    os.unlink(f'{out}/log_ring_live.raw')
                                except OSError:
                                    rdata = b''
                            if rdata:
                                last_log_data = rdata
                                broadcast_stream_packet(b'SWLG', rdata[-2048:])
                                try:
                                    tmp_log = f'{out}/sweet_log_tmp.txt'
                                    open(tmp_log, 'wb').write(rdata)
                                    os.replace(tmp_log, f'{out}/sweet_log.txt')
                                except OSError:
                                    pass
                    log_chk = rdata or last_log_data
                    if b'SpringBoard create id 9' in log_chk:
                        if last_unlock_attempt == 0.0:
                            last_unlock_attempt = elapsed - 5
                    if spotlight_seen_at == 0 and (b'SpringBoard create id 9' in log_chk) and (b'exec Spotlight' not in log_chk) and elapsed - last_unlock_attempt >= 7:
                        if not os.path.exists(f'{out}/lockscreen_fb.png') and active_fb is not None:
                            write_png(f'{out}/lockscreen_fb.png', active_fb)
                            open(f'{out}/lockscreen_fb.bin', 'wb').write(active_fb)
                        last_unlock_attempt = elapsed
                        if log_chk.rfind(b'SWEET FB: display off') > log_chk.rfind(b'SWEET FB: display on'):
                            dispatch_input(INPUT_TYPE_KEY, KEY_POWER, 1, 0, 0)
                            time.sleep(0.06)
                            dispatch_input(INPUT_TYPE_KEY, KEY_POWER, 0, 0, 0)
                            time.sleep(1.5)
                        # Smooth bottom-edge swipe-up without KEY_HOME interruption
                        for sy in (2385, 2240, 2000, 1700, 1350, 980, 620, 320, 180):
                            dispatch_input(INPUT_TYPE_TOUCH, 0, 1, 540, sy)
                            time.sleep(0.025)
                        dispatch_input(INPUT_TYPE_TOUCH, 0, 0, 540, 180)
                    if b'exec Spotlight' in log_chk:
                        if spotlight_seen_at == 0:
                            spotlight_seen_at = elapsed
                        elif elapsed - spotlight_seen_at >= 10 and not tapped_settings and not a.interactive:
                            if active_fb is not None:
                                write_png(f'{out}/homescreen_fb.png', active_fb)
                                open(f'{out}/homescreen_fb.bin', 'wb').write(active_fb)
                            tapped_settings = True
                            # Tap Settings icon on Home Screen at (655, 290) in 1080x2400 space
                            for _ in range(3):
                                dispatch_input(INPUT_TYPE_TOUCH, 0, 1, 655, 290)
                                time.sleep(0.04)
                            dispatch_input(INPUT_TYPE_TOUCH, 0, 0, 655, 290)
                except Exception:
                    if qmp_live is not None:
                        try:
                            qmp_live.close()
                        except Exception:
                            pass
                        qmp_live = None
            time.sleep(0.004)

        stop_input_thread = True
        ram_reader.close()
        if qmp_live is not None:
            try:
                qmp_live.close()
            except Exception:
                pass
            qmp_live = None

        if a.live_serial:
            stream_serial_file(serial_file, serial_pos)

        result = {'status': 'qemu_exited', 'qemu_returncode': p.returncode}
        if p.poll() is None:
            _, regs, *extra = qmp(qmp_target, 'stop', 'info registers', *a.hmp,
                                  pmemsave=(FB_BASE, FB_STRIDE * FB_H, f'{out}/fb.raw'))
            if a.kernel:
                try:
                    log_tot_vas = find_macho_symbol(a.kernel, b'_sweet_log_total')
                    log_ring_vas = find_macho_symbol(a.kernel, b'_sweet_log_ring')
                    if log_tot_vas and log_ring_vas:
                        virt_base = struct.unpack_from('<Q', payload, 0x10)[0]
                        tot_pa = log_tot_vas[0] - virt_base + phys_base
                        ring_pa = log_ring_vas[0] - virt_base + phys_base
                        with QMPClient(qmp_target) as client:
                            client.pmemsave(tot_pa, 8, f'{out}/log_tot.raw')
                            client.pmemsave(ring_pa, 4 * 1024 * 1024, f'{out}/log_ring.raw')
                        tot_bytes = struct.unpack('<Q', open(f'{out}/log_tot.raw', 'rb').read(8))[0]
                        ring_data = open(f'{out}/log_ring.raw', 'rb').read()
                        ring_sz = len(ring_data)
                        if tot_bytes <= ring_sz:
                            log_text = ring_data[:tot_bytes]
                        else:
                            pos = tot_bytes % ring_sz
                            log_text = ring_data[pos:] + ring_data[:pos]
                        open(f'{out}/sweet_log.txt', 'wb').write(log_text)
                        os.unlink(f'{out}/log_tot.raw')
                        os.unlink(f'{out}/log_ring.raw')
                except Exception as e:
                    print(f'[!] Note: could not dump sweet_log_ring: {e}', file=sys.stderr)
            open(f'{out}/regs.txt', 'w').write(regs + ''.join(f'\n(qemu) {c}\n{r}' for c, r in zip(a.hmp, extra)))
            p.kill()
            p.wait()
            raw = open(f'{out}/fb.raw', 'rb').read()
            if max(raw[::256], default=0) <= 64 and last_nonblank_fb is not None:
                raw = last_nonblank_fb
            open(f'{out}/fb.bin', 'wb').write(raw)
            write_png(f'{out}/fb.png', raw)
            result = {'status': 'not_decoded'}
            if font is not None:
                screen, unknown = decode_screen(raw, font)
                open(f'{out}/fb.txt', 'w').write(screen)
                result = screen_result(screen, unknown)
            os.unlink(f'{out}/fb.raw')
        else:
            print(f'qemu exited early: {p.returncode}', file=sys.stderr)

        open(f'{out}/result.json', 'w').write(json.dumps(result, indent=2) + '\n')
        print(json.dumps(result), file=sys.stderr)
        print(f'{out}/fb.png  {out}/regs.txt  {out}/qemu.log', file=sys.stderr)
        if a.fail_on_panic and result['status'] == 'panic':
            return 2
        if a.fail_on_panic and result['status'] in ('incomplete_decode', 'blank_screen'):
            return 3
        if result['status'] == 'qemu_exited':
            return 1
        return 0

    except KeyboardInterrupt:
        print('\n[!] Interrupted by user, stopping QEMU...', file=sys.stderr)
        if p and p.poll() is None:
            try:
                _, regs = qmp(qmp_target, 'stop', 'info registers',
                              pmemsave=(FB_BASE, FB_STRIDE * FB_H, f'{out}/fb.raw'))
                open(f'{out}/regs.txt', 'w').write(regs)
                if os.path.exists(f'{out}/fb.raw'):
                    write_png(f'{out}/fb.png', open(f'{out}/fb.raw', 'rb').read())
                    os.unlink(f'{out}/fb.raw')
            except Exception:
                pass
            p.kill()
            p.wait()
        return 130
    finally:
        ram_file = f'{out}/ram.bin'
        if os.path.exists(ram_file):
            try:
                os.unlink(ram_file)
            except Exception:
                pass
        if a.disk and os.path.exists(a.disk):
            try:
                sync_hfsx_image(a.disk)
            except Exception as e:
                print(f'[!] Note: HFSX post-run sync warning: {e}', file=sys.stderr)


def sync_hfsx_image(disk_path):
    import plistlib
    ws_dir = os.path.dirname(os.path.abspath(disk_path))
    hub_prefs = {}
    prefs_file = os.path.join(ws_dir, 'device_hub_prefs.json')
    if os.path.isfile(prefs_file):
        try:
            hub_prefs = json.loads(open(prefs_file, 'r', encoding='utf-8').read())
        except Exception:
            hub_prefs = {}
    with open(disk_path, 'r+b') as f:
        f.seek(1024)
        vh = bytearray(f.read(512))
        if len(vh) < 512 or vh[:2] not in (b'HX', b'H+'):
            return
        total_blocks = struct.unpack('>I', vh[44:48])[0]
        alloc_start, alloc_blocks = struct.unpack('>II', vh[128:136])
        cat_blocks = struct.unpack('>I', vh[284:288])[0]
        cat_start = struct.unpack('>I', vh[288:292])[0]
        f.seek(cat_start * 4096)
        cat = bytearray(f.read(cat_blocks * 4096))
        node_size = 8192
        num_nodes = len(cat) // node_size
        max_cnid = struct.unpack('>I', vh[64:68])[0]
        max_block = max(cat_start + cat_blocks, alloc_start + alloc_blocks, struct.unpack('>I', vh[52:56])[0])
        cat_mod = False
        for nid in range(num_nodes):
            node = cat[nid * node_size:(nid + 1) * node_size]
            f_link, b_link, kind, height, num_recs = struct.unpack('>IIbBH', node[:12])
            if kind != -1:
                continue
            for r in range(num_recs):
                off = struct.unpack('>H', node[node_size - 2 * (r + 1):node_size - 2 * r])[0]
                next_off = struct.unpack('>H', node[node_size - 2 * (r + 2):node_size - 2 * (r + 1)])[0]
                rec = node[off:next_off]
                if len(rec) < 12:
                    continue
                key_len = struct.unpack('>H', rec[:2])[0]
                name_chars = struct.unpack('>H', rec[6:8])[0] if len(rec) >= 8 else 0
                name = rec[8:8 + name_chars * 2].decode('utf-16be', errors='replace') if len(rec) >= 8 + name_chars * 2 else ''
                val_off = 2 + key_len + ((2 + key_len) & 1)
                if val_off + 12 > len(rec):
                    continue
                rec_type = struct.unpack('>h', rec[val_off:val_off + 2])[0]
                if rec_type in (1, 2):
                    cnid = struct.unpack('>I', rec[val_off + 8:val_off + 12])[0]
                    if cnid > max_cnid:
                        max_cnid = cnid
                    if cnid != 321325 and val_off + 44 <= len(rec):
                        f_val = bytearray(rec[val_off:])
                        uid, gid, mode = struct.unpack('>III', f_val[32:44])
                        if uid == 4294967294 or (mode & 0o777) == 0:
                            new_uid = 501 if uid == 4294967294 else uid
                            new_gid = 501 if new_uid == 501 else gid
                            new_mode = (mode & ~0o777) | (0o777 if rec_type == 1 else 0o666)
                            struct.pack_into('>III', f_val, 32, new_uid, new_gid, new_mode)
                            cat[nid * node_size + off + val_off:nid * node_size + next_off] = f_val
                            rec = cat[nid * node_size + off:nid * node_size + next_off]
                            cat_mod = True
                    if rec_type == 2 and val_off + 168 <= len(rec):
                        f_val = bytearray(rec[val_off:])
                        for e in range(8):
                            sb, bc = struct.unpack('>II', f_val[104 + e * 8:112 + e * 8])
                            if bc > 0 and sb + bc > max_block:
                                max_block = sb + bc
                        if name in (
                            '.com.apple.mobile_container_manager.metadata.plist',
                            'data_ark.plist',
                            'com.apple.springboard.plist',
                            'com.apple.Accessibility.plist',
                            '.GlobalPreferences.plist',
                            'com.apple.uikitservices.userInterfaceStyleMode.plist',
                            'IconState.plist',
                            'DefaultIconState.plist',
                            'DefaultIconState~iphone.plist',
                            'launchd.plist',
                        ):
                            lsize = struct.unpack('>Q', f_val[88:96])[0]
                            sblk, bcnt = struct.unpack('>II', f_val[104:112])
                            if name == 'launchd.plist' and lsize < 100000:
                                continue
                            if 0 < lsize <= bcnt * 4096 and sblk > 0:
                                f.seek(sblk * 4096)
                                data = f.read(lsize)
                                try:
                                    pl = plistlib.loads(data)
                                    changed = False
                                    if name == 'launchd.plist' and isinstance(pl, dict):
                                        daemons = pl.get('LaunchDaemons')
                                        if isinstance(daemons, dict):
                                            bg_throttle_tokens = (
                                                'com.apple.wifid.plist',
                                                'com.apple.wifiFirmwareLoader.plist',
                                                'com.apple.WiFiCloudAssetsXPCService.plist',
                                                'com.apple.WirelessRadioManager',
                                                'com.apple.factory.NFQRCoded.plist',
                                                'com.apple.assistantd.plist',
                                                'com.apple.routined.plist',
                                                'com.apple.sharingd.plist',
                                                'com.apple.sleepd.plist',
                                                'com.apple.sntpd.plist',
                                                'com.apple.dhcp6d.plist',
                                                'com.apple.OTACrashCopier.plist',
                                                'com.apple.DumpPanic.plist',
                                                'com.apple.accessoryupdaterd.plist',
                                                'com.apple.askpermissiond.plist',
                                                'com.apple.familynotificationd.plist',
                                                'com.apple.peakpowermanagerd.plist',
                                                'com.apple.safetyalertsd.plist',
                                                'com.apple.seld.plist',
                                                'com.apple.seserviced.plist',
                                                'com.apple.wcd.plist',
                                                'com.apple.nanoregistryd.plist',
                                                'com.apple.nanoprefsyncd.plist',
                                                'com.apple.companionappd.plist',
                                                'com.apple.chronod.plist',
                                                'com.apple.liveactivitiesd.plist',
                                                'com.apple.cloudpaird.plist',
                                                'com.apple.nearbyd.plist',
                                                'com.apple.ptpd.plist',
                                                'com.apple.remoted.plist',
                                                'com.apple.remotepairingdeviced.plist',
                                                'com.apple.revisiond.plist',
                                                'com.apple.bluetoothuserd.plist',
                                                'com.apple.PerfPowerServicesExtended.plist',
                                                'com.apple.CAReportingService.plist',
                                                'com.apple.triald.plist',
                                                'com.apple.biomed.plist',
                                                'com.apple.biomesyncd.plist',
                                                'com.apple.duetexpertd.plist',
                                                'com.apple.suggestd.plist',
                                                'com.apple.parsecd.plist',
                                                'com.apple.searchd.plist',
                                                'com.apple.searchpartyd.plist',
                                                'com.apple.findmydeviced',
                                                'com.apple.cloudd.plist',
                                                'com.apple.bird.plist',
                                                'com.apple.analyticsd.plist',
                                                'com.apple.symptomsd.plist',
                                                'com.apple.powerlogHelperd.plist',
                                                'com.apple.locationd.plist',
                                                'com.apple.corespotlightd.plist',
                                                'com.apple.appstored.plist',
                                                'com.apple.itunesstored.plist',
                                                'com.apple.bookassetd.plist',
                                                'com.apple.amsengagementd.plist',
                                                'com.apple.promotedcontentd.plist',
                                                'com.apple.imagent.plist',
                                                'com.apple.IMDPersistenceAgent.plist',
                                                'com.apple.identityservicesd.plist',
                                                'com.apple.callservicesd.plist',
                                                'com.apple.CallHistorySyncHelper.plist',
                                                'com.apple.FaceTime.',
                                                'com.apple.apsd.plist',
                                                'com.apple.ScreenTimeAgent.plist',
                                                'com.apple.familycircled.plist',
                                                'com.apple.CircleJoinRequested.plist',
                                                'com.apple.cdpd.plist',
                                                'com.apple.pfd.plist',
                                                'com.apple.ind.plist',
                                                'com.apple.akd.plist',
                                                'com.apple.appleaccountd.plist',
                                                'com.apple.coreidvd.plist',
                                                'com.apple.dmd.plist',
                                                'com.apple.passd.plist',
                                                'com.apple.assetsd.plist',
                                                'com.apple.axassetsd.plist',
                                                'com.apple.languageassetd.plist',
                                                'com.apple.voiced.plist',
                                                'com.apple.contextstored.plist',
                                                'com.apple.transparencyd.plist',
                                                'com.apple.sociallayerd.plist',
                                                'com.apple.useractivityd.plist',
                                                'com.apple.syncdefaultsd.plist',
                                                'com.apple.webbookmarksd.plist',
                                                'com.apple.SafariBookmarksSyncAgent.plist',
                                                'com.apple.ndoagent.plist',
                                                'com.apple.mobilerepaird.plist',
                                                'com.apple.announced.plist',
                                                'com.apple.donotdisturbd.plist',
                                                'com.apple.AppSSODaemon.plist',
                                                'com.apple.GSSCred.plist',
                                                'com.apple.VideoSubscriberAccount.',
                                                'com.apple.videosubscriptionsd.plist',
                                                'com.apple.FileProvider.plist',
                                                'com.apple.filesystems.livefileproviderd.plist',
                                                'com.apple.ManagedSettingsAgent.plist',
                                                'com.apple.privacyaccountingd.plist',
                                                'com.apple.nfcd.plist',
                                                'com.apple.rapportd.plist',
                                                'com.apple.CommCenter',
                                                'com.apple.AccessibilityUIServer.plist',
                                                'com.apple.accessibility.axassetsd.plist',
                                                'com.apple.accessibility.axremoted.plist',
                                                'com.apple.accessibility.heard.plist',
                                                'com.apple.accessibility.motiontrackingd.plist',
                                                'com.apple.SiriTTSService.',
                                                'com.apple.siri.',
                                                '/System/Library/NanoLaunchDaemons/',
                                            )
                                            no_ipc_bloat_tokens = (
                                                'com.apple.WiFiCloudAssetsXPCService.plist',
                                                'com.apple.WirelessRadioManager',
                                                'com.apple.assistantd.plist',
                                                'com.apple.routined.plist',
                                                'com.apple.sharingd.plist',
                                                'com.apple.wcd.plist',
                                                'com.apple.nanoregistryd.plist',
                                                'com.apple.nanoprefsyncd.plist',
                                                'com.apple.companionappd.plist',
                                                'com.apple.chronod.plist',
                                                'com.apple.liveactivitiesd.plist',
                                                'com.apple.triald.plist',
                                                'com.apple.biomed.plist',
                                                'com.apple.biomesyncd.plist',
                                                'com.apple.duetexpertd.plist',
                                                'com.apple.suggestd.plist',
                                                'com.apple.parsecd.plist',
                                                'com.apple.searchd.plist',
                                                'com.apple.searchpartyd.plist',
                                                'com.apple.findmydeviced',
                                                'com.apple.cloudd.plist',
                                                'com.apple.bird.plist',
                                                'com.apple.analyticsd.plist',
                                                'com.apple.symptomsd.plist',
                                                'com.apple.powerlogHelperd.plist',
                                                'com.apple.PerfPowerServicesExtended.plist',
                                                'com.apple.CAReportingService.plist',
                                                'com.apple.corespotlightd.plist',
                                                'com.apple.appstored.plist',
                                                'com.apple.itunesstored.plist',
                                                'com.apple.bookassetd.plist',
                                                'com.apple.amsengagementd.plist',
                                                'com.apple.promotedcontentd.plist',
                                                'com.apple.imagent.plist',
                                                'com.apple.IMDPersistenceAgent.plist',
                                                'com.apple.identityservicesd.plist',
                                                'com.apple.callservicesd.plist',
                                                'com.apple.CallHistorySyncHelper.plist',
                                                'com.apple.FaceTime.',
                                                'com.apple.apsd.plist',
                                                'com.apple.ScreenTimeAgent.plist',
                                                'com.apple.ManagedSettingsAgent.plist',
                                                'com.apple.privacyaccountingd.plist',
                                                'com.apple.familycircled.plist',
                                                'com.apple.CircleJoinRequested.plist',
                                                'com.apple.cdpd.plist',
                                                'com.apple.pfd.plist',
                                                'com.apple.ind.plist',
                                                'com.apple.akd.plist',
                                                'com.apple.appleaccountd.plist',
                                                'com.apple.coreidvd.plist',
                                                'com.apple.dmd.plist',
                                                'com.apple.passd.plist',
                                                'com.apple.nfcd.plist',
                                                'com.apple.seld.plist',
                                                'com.apple.seserviced.plist',
                                                'com.apple.rapportd.plist',
                                                'com.apple.CommCenter',
                                                'com.apple.FileProvider.plist',
                                                'com.apple.filesystems.livefileproviderd.plist',
                                                'com.apple.VideoSubscriberAccount.',
                                                'com.apple.videosubscriptionsd.plist',
                                                'com.apple.AppSSODaemon.plist',
                                                'com.apple.GSSCred.plist',
                                                'com.apple.AccessibilityUIServer.plist',
                                                'com.apple.accessibility.axassetsd.plist',
                                                'com.apple.accessibility.axremoted.plist',
                                                'com.apple.accessibility.heard.plist',
                                                'com.apple.accessibility.motiontrackingd.plist',
                                                'com.apple.voiced.plist',
                                                'com.apple.contextstored.plist',
                                                'com.apple.transparencyd.plist',
                                                'com.apple.sociallayerd.plist',
                                                'com.apple.useractivityd.plist',
                                                'com.apple.syncdefaultsd.plist',
                                                'com.apple.webbookmarksd.plist',
                                                'com.apple.SafariBookmarksSyncAgent.plist',
                                                'com.apple.ndoagent.plist',
                                                'com.apple.mobilerepaird.plist',
                                                'com.apple.announced.plist',
                                                'com.apple.donotdisturbd.plist',
                                                'com.apple.SiriTTSService.',
                                                'com.apple.siri.',
                                                '/System/Library/NanoLaunchDaemons/',
                                            )
                                            for dk, dv in daemons.items():
                                                if not isinstance(dv, dict):
                                                    continue
                                                if 'gamecontrollerd.plist' in dk.lower():
                                                    if not dv.get('RunAtLoad'):
                                                        dv['RunAtLoad'] = True
                                                        changed = True
                                                    continue
                                                if any(tok in dk for tok in bg_throttle_tokens):
                                                    for k_strip in ('KeepAlive', 'LaunchEvents', 'Events', 'StartInterval', 'StartCalendarInterval'):
                                                        if k_strip in dv:
                                                            dv.pop(k_strip, None)
                                                            changed = True
                                                    if dv.get('RunAtLoad'):
                                                        dv['RunAtLoad'] = False
                                                        changed = True
                                                    if dv.get('ThrottleInterval') != 3600:
                                                        dv['ThrottleInterval'] = 3600
                                                        changed = True
                                                if any(tok in dk for tok in no_ipc_bloat_tokens):
                                                    for k_strip in ('MachServices', 'LaunchEvents', 'Events', 'KeepAlive', 'StartInterval', 'StartCalendarInterval'):
                                                        if k_strip in dv:
                                                            dv.pop(k_strip, None)
                                                            changed = True
                                    elif name == '.com.apple.mobile_container_manager.metadata.plist':
                                        if not isinstance(pl.get('MCMMetadataInfo'), dict):
                                            pl['MCMMetadataInfo'] = {}
                                            changed = True
                                    elif name == 'data_ark.plist' and isinstance(pl, dict):
                                        if pl.get('-ActivationState') != 'Activated' or not pl.get('-DataMigrationCompleted'):
                                            pl['-ActivationState'] = 'Activated'
                                            pl['-BrickState'] = False
                                            pl['-BuildVersion'] = '20H392'
                                            pl['-ProductVersion'] = '16.7.16'
                                            pl['-DataMigrationCompleted'] = True
                                            pl['-PreSydroDataMigrationCompleted'] = True
                                            changed = True
                                    elif name == 'com.apple.uikitservices.userInterfaceStyleMode.plist' and isinstance(pl, dict):
                                        want_style = 1 if (hub_prefs and hub_prefs.get('dark_mode') is False) else 2
                                        if pl.get('UserInterfaceStyleMode') != want_style:
                                            pl['UserInterfaceStyleMode'] = want_style
                                            changed = True
                                    elif name == 'com.apple.Accessibility.plist' and isinstance(pl, dict):
                                        want_rm = 1 if (hub_prefs and hub_prefs.get('reduce_motion') is True) else 0
                                        if pl.get('ReduceMotionEnabled') != want_rm or pl.get('EnhancedBackgroundContrastEnabled') != 1:
                                            pl['ReduceMotionEnabled'] = want_rm
                                            pl['EnhancedBackgroundContrastEnabled'] = 1
                                            changed = True
                                    elif name == '.GlobalPreferences.plist' and isinstance(pl, dict):
                                        if pl.get('UIAnimationDragCoefficient') != 0.05:
                                            pl['UIAnimationDragCoefficient'] = 0.05
                                            changed = True
                                    elif name == 'com.apple.springboard.plist' and isinstance(pl, dict):
                                        want_style = 1 if (hub_prefs and hub_prefs.get('dark_mode') is False) else 2
                                        want_rm_sb = bool(hub_prefs and hub_prefs.get('reduce_motion') is True)
                                        if pl.get('SBUserInterfaceStyle') != want_style:
                                            pl['SBUserInterfaceStyle'] = want_style
                                            changed = True
                                        if pl.get('SBReduceMotion') is not want_rm_sb:
                                            pl['SBReduceMotion'] = want_rm_sb
                                            changed = True
                                        if not pl.get('BKNoWatchdogs'):
                                            pl['BKNoWatchdogs'] = True
                                            changed = True
                                        for wp_key in ('SBLockScreenWallpapers', 'SBHomeScreenWallpapers'):
                                            wp = pl.get(wp_key)
                                            if isinstance(wp, dict) and isinstance(wp.get('none'), dict):
                                                if wp['none'].get('SBWallpaperParallaxFactorKey') != 0.0:
                                                    wp['none']['SBWallpaperParallaxFactorKey'] = 0.0
                                                    changed = True
                                    elif name in ('IconState.plist', 'DefaultIconState.plist', 'DefaultIconState~iphone.plist') and isinstance(pl, dict):
                                        if pl.get('today'):
                                            pl['today'] = []
                                            changed = True
                                        if pl.get('favoriteToday'):
                                            pl['favoriteToday'] = []
                                            changed = True
                                        ilists = pl.get('iconLists')
                                        if isinstance(ilists, list):
                                            new_ilists = []
                                            for page in ilists:
                                                if isinstance(page, list):
                                                    filtered = [item for item in page if not isinstance(item, dict)]
                                                    if len(filtered) != len(page):
                                                        changed = True
                                                    new_ilists.append(filtered)
                                                else:
                                                    new_ilists.append(page)
                                            if changed:
                                                pl['iconLists'] = new_ilists
                                    if changed:
                                        new_b = plistlib.dumps(pl, fmt=plistlib.FMT_BINARY)
                                        if len(new_b) <= bcnt * 4096:
                                            f.seek(sblk * 4096)
                                            f.write(new_b + b'\x00' * (bcnt * 4096 - len(new_b)))
                                            struct.pack_into('>Q', f_val, 88, len(new_b))
                                            cat[nid * node_size + off + val_off:nid * node_size + next_off] = f_val
                                            cat_mod = True
                                except Exception:
                                    pass
        if cat_mod:
            f.seek(cat_start * 4096)
            f.write(cat)
        next_alloc = max_block + 16
        f.seek(alloc_start * 4096)
        bm = bytearray(f.read(alloc_blocks * 4096))
        full_bytes = min(len(bm), next_alloc // 8)
        bm[:full_bytes] = b'\xff' * full_bytes
        rem_bits = next_alloc % 8
        if rem_bits and full_bytes < len(bm):
            bm[full_bytes] |= (0xFF << (8 - rem_bits)) & 0xFF
        f.seek(alloc_start * 4096)
        f.write(bm)
        free_blocks = max(0, total_blocks - next_alloc - 1)
        vh[4:8] = struct.pack('>I', 0x80000100)
        vh[48:52] = struct.pack('>I', free_blocks)
        vh[52:56] = struct.pack('>I', next_alloc)
        vh[64:68] = struct.pack('>I', max_cnid + 100)
        f.seek(1024)
        f.write(vh)
        f.seek((total_blocks - 1) * 4096 + 3072)
        f.write(vh)
        f.flush()


if __name__ == '__main__':
    sys.exit(main())
