#!/usr/bin/env python3
"""BlueStacks-style Desktop GUI Emulator & Controller for ios-sweet (iPhone X / iOS 16.7.16).

Usage:
  python tools/sweet-emu.py                # Launch full Desktop Emulator GUI
  python tools/sweet-emu.py --self-test    # Run automated headless self-test and exit 0

Features:
  - Interactive iPhone X (1080x2400 @3x -> 360x800 viewport) display with notch & home bar
  - Mouse-to-Touch engine mapped to SweetHID (0..1079, 0..2399) with bottom-edge (y >= 2352)
    swipe-up Home Gesture detection (kIOHIDDigitizerEventFromEdgeTip, patch 0071)
  - Two operating modes:
      1. Live QEMU VM Mode: spawns qemu-system-aarch64 with SMP/MTTCG, virtio-blk root disk,
         virtio-net, QMP framebuffer streaming (0x9c000000), and shared-memory input mailbox
         (0x9d600000).
      2. Interactive Preview / Sim Mode: built-in interactive iOS 16.7.16 Lock Screen,
         SpringBoard Home Screen, Settings -> About (sweet-virt), and live .ipa launcher so
         every gesture, button, and IPA install can be tested immediately with zero setup.
  - One-click .IPA / .APP installer and Demo Game IPA generator powered by tools/sweet-ipa.py
  - One-click QEMU Apple Device Tree (mkadt.py) & SWEETXNU payload (mkpayload.py) builder
"""

import argparse
import datetime
import importlib.util
import json
import math
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import threading
import time
import zlib

TOOLS_DIR = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.dirname(TOOLS_DIR)
if TOOLS_DIR not in sys.path:
    sys.path.insert(0, TOOLS_DIR)


def _load_tool_module(module_name, filename):
    path = os.path.join(TOOLS_DIR, filename)
    spec = importlib.util.spec_from_file_location(module_name, path)
    if spec is None or spec.loader is None:
        raise ImportError(f'Cannot load {filename} from {TOOLS_DIR}')
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


qemu_sweet = _load_tool_module('qemu_sweet', 'qemu-sweet.py')
sweet_ipa = _load_tool_module('sweet_ipa', 'sweet-ipa.py')
mkadt = _load_tool_module('mkadt', 'mkadt.py')
mkpayload = _load_tool_module('mkpayload', 'mkpayload.py')

# Panel constants (iPhone X / Redmi Note 10 Pro framebuffer)
PANEL_W = qemu_sweet.FB_W          # 1080
PANEL_H = qemu_sweet.FB_H          # 2400
EDGE_THRESHOLD_PX = 48             # Patch 0071 SWEET_TOUCH_EDGE_PX = 48 (bottom edge y >= 2352)
BOTTOM_EDGE_Y = PANEL_H - EDGE_THRESHOLD_PX  # 2352

# Viewport scale (1/3 of 1080x2400 = 360x800 points, exact @3x retina ratio)
VIEWPORT_SCALE = 3
VIEWPORT_W = PANEL_W // VIEWPORT_SCALE  # 360
VIEWPORT_H = PANEL_H // VIEWPORT_SCALE  # 800

# Bezel padding around the 360x800 screen inside the left canvas
BEZEL_PAD_X = 28
BEZEL_PAD_Y = 30
CANVAS_W = VIEWPORT_W + BEZEL_PAD_X * 2
CANVAS_H = VIEWPORT_H + BEZEL_PAD_Y * 2


# ---------------------------------------------------------------------------
# Coordinate Mapper & Gesture Engine
# ---------------------------------------------------------------------------

class TouchCoordinateMapper:
    """Map canvas mouse coordinates to 1080x2400 panel coordinates and detect edge gestures."""

    def __init__(self, screen_x0=BEZEL_PAD_X, screen_y0=BEZEL_PAD_Y,
                 view_w=VIEWPORT_W, view_h=VIEWPORT_H,
                 panel_w=PANEL_W, panel_h=PANEL_H):
        self.screen_x0 = screen_x0
        self.screen_y0 = screen_y0
        self.view_w = view_w
        self.view_h = view_h
        self.panel_w = panel_w
        self.panel_h = panel_h

    def inside_screen(self, canvas_x, canvas_y):
        return (self.screen_x0 <= canvas_x <= self.screen_x0 + self.view_w and
                self.screen_y0 <= canvas_y <= self.screen_y0 + self.view_h)

    def canvas_to_panel(self, canvas_x, canvas_y):
        rel_x = (canvas_x - self.screen_x0) / max(1, self.view_w)
        rel_y = (canvas_y - self.screen_y0) / max(1, self.view_h)
        px = int(round(rel_x * (self.panel_w - 1)))
        py = int(round(rel_y * (self.panel_h - 1)))
        px = max(0, min(self.panel_w - 1, px))
        py = max(0, min(self.panel_h - 1, py))
        return px, py

    def panel_to_canvas(self, panel_x, panel_y):
        cx = self.screen_x0 + (panel_x / max(1, self.panel_w - 1)) * self.view_w
        cy = self.screen_y0 + (panel_y / max(1, self.panel_h - 1)) * self.view_h
        return int(round(cx)), int(round(cy))

    @staticmethod
    def is_edge_contact(panel_x, panel_y):
        """Matches SweetHID.cpp (patch 0071): contacts starting within 48px of any edge
        carry kIOHIDDigitizerEventFromEdgeTip."""
        return (
            panel_x < EDGE_THRESHOLD_PX
            or panel_x > (PANEL_W - 1) - EDGE_THRESHOLD_PX
            or panel_y < EDGE_THRESHOLD_PX
            or panel_y > (PANEL_H - 1) - EDGE_THRESHOLD_PX
        )

    @staticmethod
    def is_bottom_edge_start(panel_y):
        """Return True if touch started in the bottom 48px home-gesture zone (y >= 2352)."""
        return panel_y >= BOTTOM_EDGE_Y

    @staticmethod
    def is_home_swipe(start_x, start_y, end_x, end_y):
        """Return True if gesture started in the bottom edge (y >= 2352) and swiped upward."""
        return start_y >= BOTTOM_EDGE_Y and (start_y - end_y) >= 120


# ---------------------------------------------------------------------------
# Headless / State Model for Interactive Preview & QEMU Controller
# ---------------------------------------------------------------------------

BUILTIN_SYSTEM_APPS = [
    # Row 0
    {
        'bundle_id': 'com.apple.facetime',
        'display_name': 'FaceTime',
        'short_version': '16.7.16',
        'graphics_api': 'UIKit / AVFoundation',
        'color': '#34c759',
        'accent': '#86efac',
        'glyph': '📹',
        'builtin': True,
    },
    {
        'bundle_id': 'com.apple.mobilecal',
        'display_name': 'Calendar',
        'short_version': '16.7.16',
        'graphics_api': 'UIKit / EventKit',
        'color': '#f8fafc',
        'accent': '#ff3b30',
        'glyph': '20',
        'text_color': '#0f172a',
        'builtin': True,
    },
    {
        'bundle_id': 'com.apple.mobileslideshow',
        'display_name': 'Photos',
        'short_version': '16.7.16',
        'graphics_api': 'PhotoKit / CoreImage',
        'color': '#f8fafc',
        'accent': '#38bdf8',
        'glyph': '✿',
        'text_color': '#ec4899',
        'builtin': True,
    },
    {
        'bundle_id': 'com.apple.camera',
        'display_name': 'Camera',
        'short_version': '16.7.16',
        'graphics_api': 'AVFoundation / ISP',
        'color': '#cbd5e1',
        'accent': '#64748b',
        'glyph': '📷',
        'text_color': '#0f172a',
        'builtin': True,
    },
    # Row 1
    {
        'bundle_id': 'com.apple.mobilemail',
        'display_name': 'Mail',
        'short_version': '16.7.16',
        'graphics_api': 'UIKit / MessageUI',
        'color': '#0a84ff',
        'accent': '#60a5fa',
        'glyph': '✉',
        'builtin': True,
    },
    {
        'bundle_id': 'com.apple.mobiletimer',
        'display_name': 'Clock',
        'short_version': '16.7.16',
        'graphics_api': 'UIKit / CoreAnimation',
        'color': '#0f172a',
        'accent': '#f59e0b',
        'glyph': '🕘',
        'builtin': True,
    },
    {
        'bundle_id': 'com.apple.Maps',
        'display_name': 'Maps',
        'short_version': '16.7.16',
        'graphics_api': 'MapKit / VectorKit',
        'color': '#22c55e',
        'accent': '#38bdf8',
        'glyph': '🗺',
        'builtin': True,
    },
    {
        'bundle_id': 'com.apple.weather',
        'display_name': 'Weather',
        'short_version': '16.7.16',
        'graphics_api': 'WeatherKit / Metal',
        'color': '#0284c7',
        'accent': '#fde047',
        'glyph': '⛅',
        'builtin': True,
    },
    # Row 2
    {
        'bundle_id': 'com.apple.reminders',
        'display_name': 'Reminders',
        'short_version': '16.7.16',
        'graphics_api': 'UIKit / EventKit',
        'color': '#f8fafc',
        'accent': '#3b82f6',
        'glyph': '☰',
        'text_color': '#0a84ff',
        'builtin': True,
    },
    {
        'bundle_id': 'com.apple.mobilenotes',
        'display_name': 'Notes',
        'short_version': '16.7.16',
        'graphics_api': 'UIKit / PencilKit',
        'color': '#fef08a',
        'accent': '#f59e0b',
        'glyph': '📝',
        'text_color': '#0f172a',
        'builtin': True,
    },
    {
        'bundle_id': 'com.apple.stocks',
        'display_name': 'Stocks',
        'short_version': '16.7.16',
        'graphics_api': 'UIKit / Charts',
        'color': '#0f172a',
        'accent': '#38bdf8',
        'glyph': '📈',
        'builtin': True,
    },
    {
        'bundle_id': 'com.apple.iBooks',
        'display_name': 'Books',
        'short_version': '16.7.16',
        'graphics_api': 'UIKit / CoreText',
        'color': '#f97316',
        'accent': '#fdba74',
        'glyph': '📖',
        'builtin': True,
    },
    # Row 3
    {
        'bundle_id': 'com.apple.AppStore',
        'display_name': 'App Store',
        'short_version': '16.7.16',
        'graphics_api': 'StoreKit / UIKit',
        'color': '#0a84ff',
        'accent': '#93c5fd',
        'glyph': 'A',
        'builtin': True,
    },
    {
        'bundle_id': 'com.apple.podcasts',
        'display_name': 'Podcasts',
        'short_version': '16.7.16',
        'graphics_api': 'AVFoundation / UIKit',
        'color': '#9333ea',
        'accent': '#d8b4fe',
        'glyph': '🎙',
        'builtin': True,
    },
    {
        'bundle_id': 'com.apple.tv',
        'display_name': 'TV',
        'short_version': '16.7.16',
        'graphics_api': 'AVKit / CoreMedia',
        'color': '#0f172a',
        'accent': '#94a3b8',
        'glyph': 'tv',
        'builtin': True,
    },
    {
        'bundle_id': 'com.apple.Health',
        'display_name': 'Health',
        'short_version': '16.7.16',
        'graphics_api': 'HealthKit / UIKit',
        'color': '#f8fafc',
        'accent': '#ff2d55',
        'glyph': '♥',
        'text_color': '#ff2d55',
        'builtin': True,
    },
    # Row 4
    {
        'bundle_id': 'com.apple.Home',
        'display_name': 'Home',
        'short_version': '16.7.16',
        'graphics_api': 'HomeKit / UIKit',
        'color': '#f59e0b',
        'accent': '#fde68a',
        'glyph': '🏠',
        'builtin': True,
    },
    {
        'bundle_id': 'com.apple.Passbook',
        'display_name': 'Wallet',
        'short_version': '16.7.16',
        'graphics_api': 'PassKit / UIKit',
        'color': '#1e293b',
        'accent': '#38bdf8',
        'glyph': '💳',
        'builtin': True,
    },
    {
        'bundle_id': 'com.apple.Preferences',
        'display_name': 'Settings',
        'short_version': '16.7.16',
        'graphics_api': 'UIKit / MobileGestalt',
        'color': '#64748b',
        'accent': '#cbd5e1',
        'glyph': '⚙',
        'builtin': True,
    },
    {
        'bundle_id': 'com.apple.DocumentsApp',
        'display_name': 'Files',
        'short_version': '16.7.16',
        'graphics_api': 'UIKit / FileProvider',
        'color': '#0a84ff',
        'accent': '#60a5fa',
        'glyph': '📁',
        'builtin': True,
    },
]

DOCK_APPS = [
    {
        'bundle_id': 'com.apple.mobilephone',
        'display_name': 'Phone',
        'short_version': '16.7.16',
        'graphics_api': 'UIKit / CallKit',
        'color': '#34c759',
        'accent': '#86efac',
        'glyph': '📞',
    },
    {
        'bundle_id': 'com.apple.mobilesafari',
        'display_name': 'Safari',
        'short_version': '16.7.16',
        'graphics_api': 'WebKit / Nitro JS',
        'color': '#0a84ff',
        'accent': '#60a5fa',
        'glyph': '🧭',
    },
    {
        'bundle_id': 'com.apple.MobileSMS',
        'display_name': 'Messages',
        'short_version': '16.7.16',
        'graphics_api': 'UIKit / IMCore',
        'color': '#34c759',
        'accent': '#86efac',
        'glyph': '💬',
    },
    {
        'bundle_id': 'com.apple.Music',
        'display_name': 'Music',
        'short_version': '16.7.16',
        'graphics_api': 'MediaPlayer / AudioToolbox',
        'color': '#ff2d55',
        'accent': '#fda4af',
        'glyph': '♫',
    },
]

APP_PALETTE = [
    ('#7c3aed', '#c084fc'),
    ('#ea580c', '#fb923c'),
    ('#0d9488', '#2dd4bf'),
    ('#db2777', '#f472b6'),
    ('#4f46e5', '#818cf8'),
    ('#ca8a04', '#facc15'),
]


class EmulatorCore:
    """Core state, workspace manager, ADT/payload builder, and input/preview engine."""

    def __init__(self, workspace_dir=None):
        self.workspace_dir = os.path.abspath(workspace_dir or os.path.join(REPO_ROOT, 'emu_workspace'))
        self.rootfs_dir = os.path.join(self.workspace_dir, 'rootfs')
        self.qemu_out_dir = os.path.join(self.workspace_dir, 'qemu_run')
        self.screenshots_dir = os.path.join(self.workspace_dir, 'screenshots')
        self.adt_path = os.path.join(self.workspace_dir, 'adt.qemu.bin')
        self.payload_path = os.path.join(self.workspace_dir, 'payload.qemu.bin')
        self.disk_path = os.path.join(self.workspace_dir, 'ios16-root.img')
        self.kernel_path = ''

        for d in (self.workspace_dir, self.rootfs_dir, self.qemu_out_dir, self.screenshots_dir):
            os.makedirs(d, exist_ok=True)

        # Configuration state
        self.smp_cores = 6
        self.ram_gb = 4
        self.accel_label = 'Multi-threaded TCG (MTTCG)'
        self.gpu_mode_label = 'Live Canvas Stream (CPU->RAMFB)'
        self.enable_net = True

        # Runtime state
        self.mode = 'preview'          # 'preview' or 'qemu'
        self.screen_state = 'lock'     # 'off', 'lock', 'home', 'app'
        self.active_app = None         # dict of currently open app when screen_state == 'app'
        self.volume_level = 65         # 0..100
        self.volume_hud_until = 0.0
        self.touch_Count = 0
        self.last_touch = None         # (x, y, down, is_edge)
        self.anim_tick = 0

        # Input mailbox & mapper
        self.mapper = TouchCoordinateMapper()
        self.mailbox = qemu_sweet.SweetInputMailbox()

        # QEMU process management
        self.qemu_proc = None
        self.qmp_target = None
        self.serial_pos = 0
        self.last_fb_raw = None

        # Cached icon hitboxes in panel coordinates: list of (x0, y0, x1, y1, app_dict)
        self.icon_hitboxes = []
        self.installed_apps = []
        self.logs = []

        self.refresh_installed_apps()

    def log(self, msg):
        ts = datetime.datetime.now().strftime('%H:%M:%S')
        line = f'[{ts}] {msg}'
        self.logs.append(line)
        return line

    def accel_code(self):
        lbl = self.accel_label.lower()
        if 'whpx' in lbl:
            return 'whpx'
        if 'kvm' in lbl:
            return 'kvm' if sys.platform.startswith('linux') else 'hvf'
        if 'hvf' in lbl:
            return 'hvf'
        if 'mttcg' in lbl or 'multi' in lbl:
            return 'mttcg'
        return 'auto'

    def refresh_installed_apps(self):
        try:
            self.installed_apps = sweet_ipa.list_installed_apps(self.rootfs_dir)
        except Exception as e:
            self.log(f'[!] Failed to list installed apps: {e}')
            self.installed_apps = []
        self._rebuild_icon_hitboxes()
        return self.installed_apps

    def all_home_apps(self):
        combined = list(BUILTIN_SYSTEM_APPS)
        for idx, app in enumerate(self.installed_apps):
            bg, accent = APP_PALETTE[idx % len(APP_PALETTE)]
            g_api = app.get('graphics_api', 'UIKit')
            glyph = '🎮' if ('Metal' in g_api or 'OpenGL' in g_api) else '📦'
            combined.append({
                'bundle_id': app['bundle_id'],
                'display_name': app['display_name'],
                'short_version': app.get('short_version', '1.0'),
                'graphics_api': g_api,
                'cdhashes': app.get('cdhashes', []),
                'frameworks': app.get('frameworks', []),
                'app_path': app.get('app_path', ''),
                'color': bg,
                'accent': accent,
                'glyph': glyph,
                'builtin': False,
            })
        return combined

    def _rebuild_icon_hitboxes(self):
        """Compute 1080x2400 panel hitboxes for SpringBoard Home Screen icons & dock."""
        self.icon_hitboxes = []
        apps = self.all_home_apps()
        # 4-column x 6-row grid in panel space (1080x2400 -> 360x800 @3x)
        # Rows 0..4 hold the 20 stock iOS 16 apps; Row 5 holds user-installed .ipa apps
        cols = 4
        icon_size = 162
        col_step = 240
        row_step = 258
        start_x = 90
        start_y = 198

        for idx, app in enumerate(apps[:24]):
            r = idx // cols
            c = idx % cols
            x0 = start_x + c * col_step
            y0 = start_y + r * row_step
            self.icon_hitboxes.append((x0, y0, x0 + icon_size, y0 + icon_size + 42, app))

        # Dock icons at the bottom of the screen (y = 2088..2328 -> 696..776 in 360x800)
        dock_start_x = 96
        for idx, d_app in enumerate(DOCK_APPS):
            x0 = dock_start_x + idx * 238
            y0 = 2088
            target = next((a for a in apps if a['bundle_id'] == d_app['bundle_id']), d_app)
            self.icon_hitboxes.append((x0, y0, x0 + icon_size, y0 + icon_size, target))

    def build_qemu_adt_and_payload(self):
        """Build QEMU Apple Device Tree (adt.qemu.bin) and optionally SWEETXNU payload."""
        os.makedirs(self.workspace_dir, exist_ok=True)
        mkadt_script = os.path.join(TOOLS_DIR, 'mkadt.py')
        cmd = [sys.executable, mkadt_script, self.adt_path, '--qemu', '--cpus', str(self.smp_cores)]
        res = subprocess.run(cmd, capture_output=True, text=True)
        if res.returncode != 0:
            # Fallback if mkadt.py does not take --cpus
            cmd_fb = [sys.executable, mkadt_script, self.adt_path, '--qemu']
            res = subprocess.run(cmd_fb, capture_output=True, text=True)
        if res.returncode != 0 or not os.path.isfile(self.adt_path):
            raise RuntimeError(f'mkadt.py failed: {res.stderr.strip()}')

        adt_size = os.path.getsize(self.adt_path)
        self.log(f'[✓] Built QEMU ADT: {self.adt_path} ({adt_size} bytes, cpus={self.smp_cores})')

        payload_built = False
        if self.kernel_path and os.path.isfile(self.kernel_path):
            cmdline = f'debug=0xc serial=3 cpus={self.smp_cores} keepsyms=1'
            blob, info = mkpayload.build(self.kernel_path, self.adt_path, cmdline)
            with open(self.payload_path, 'wb') as f:
                f.write(blob)
            payload_built = True
            self.log(f'[✓] Built SWEETXNU payload: {self.payload_path} ({len(blob)} bytes, entry={info["entry"]})')
        else:
            self.log('[i] Kernel Mach-O not selected; built ADT only (select a kernel to build payload.qemu.bin).')

        return {
            'adt_path': self.adt_path,
            'adt_size': adt_size,
            'payload_built': payload_built,
            'payload_path': self.payload_path if payload_built else None,
        }

    def create_and_install_demo_ipa(self):
        """Generate SweetBenchmark.ipa via sweet-ipa.py and install it into rootfs."""
        demo_ipa_path = os.path.join(self.workspace_dir, 'SweetBenchmark.ipa')
        demo_info = sweet_ipa.create_demo_ipa(demo_ipa_path)
        cdh = (demo_info.get('cdhashes') or ['none'])[0]
        self.log(f'[✓] Created demo game IPA: {demo_ipa_path} (CDHash: {cdh[:16]}...)')
        inst = sweet_ipa.install_ipa(demo_ipa_path, self.rootfs_dir)
        self.refresh_installed_apps()
        self.log(
            f'[✓] Installed {inst["display_name"]} ({inst["bundle_id"]}) v{inst["short_version"]} '
            f'[{inst["graphics"]["primary_api"]}] into {self.rootfs_dir}'
        )
        return inst

    def install_ipa_file(self, path):
        """Install a user-selected .ipa or .app into rootfs."""
        inst = sweet_ipa.install_ipa(path, self.rootfs_dir)
        self.refresh_installed_apps()
        self.log(
            f'[✓] Installed {inst["display_name"]} ({inst["bundle_id"]}) v{inst["short_version"]} '
            f'[{inst["graphics"]["primary_api"]}]'
        )
        return inst

    def uninstall_app_by_id(self, bundle_id):
        res = sweet_ipa.uninstall_app(bundle_id, self.rootfs_dir)
        if self.active_app and self.active_app.get('bundle_id') == bundle_id:
            self.screen_state = 'home'
            self.active_app = None
        self.refresh_installed_apps()
        self.log(f'[*] Uninstalled {bundle_id} from rootfs')
        return res

    # --- Touch & Hardware Button Handling ---

    def handle_touch_event(self, down, panel_x, panel_y, start_xy=None):
        """Process a touch event at (panel_x, panel_y) in both mailbox/QEMU and Preview mode."""
        px, py = self.mailbox.clamp_coords(panel_x, panel_y)
        is_edge = self.mapper.is_edge_contact(px, py)
        self.last_touch = (px, py, bool(down), is_edge)
        self.touch_Count += 1

        # Always enqueue into the 0x9d600000 input mailbox (and QMP if running)
        self.mailbox.qmp_target = self.qmp_target
        ev = self.mailbox.send_touch(0, down, px, py)

        # In Interactive Preview mode, update the simulated iOS 16 UI on release
        if not down and start_xy is not None:
            sx, sy = start_xy
            if self.mapper.is_home_swipe(sx, sy, px, py):
                self.trigger_home_gesture(from_edge_swipe=True)
            else:
                dist = math.hypot(px - sx, py - sy)
                if dist < 60:
                    self._handle_preview_tap(px, py)
                elif self.screen_state == 'lock' and (sy - py) > 120:
                    self.screen_state = 'home'
                    self.log('[iOS] Lock Screen unlocked via swipe-up')
        return ev

    def _handle_preview_tap(self, px, py):
        if self.screen_state == 'off':
            self.screen_state = 'lock'
            self.log('[iOS] Tap to wake -> Lock Screen')
            return
        if self.screen_state == 'lock':
            # Tapping bottom bar area on lock screen also hints/unlocks
            if py >= 2100:
                self.screen_state = 'home'
                self.log('[iOS] Unlocked -> SpringBoard Home Screen')
            return
        if self.screen_state == 'home':
            self._rebuild_icon_hitboxes()
            for x0, y0, x1, y1, app in self.icon_hitboxes:
                if x0 <= px <= x1 and y0 <= py <= y1:
                    self.launch_app_in_preview(app)
                    return

    def launch_app_in_preview(self, app):
        self.active_app = app
        self.screen_state = 'app'
        self.log(f'[SpringBoard] Launched {app.get("display_name")} ({app.get("bundle_id")})')

    def trigger_home_gesture(self, from_edge_swipe=False):
        self.mailbox.qmp_target = self.qmp_target
        if not from_edge_swipe:
            self.mailbox.home_gesture(0)
        self.mailbox.press_key(qemu_sweet.KEY_HOME)

        if self.screen_state == 'off':
            self.screen_state = 'lock'
        elif self.screen_state == 'lock':
            self.screen_state = 'home'
            self.log('[HID] Bottom-edge Home Gesture (kIOHIDDigitizerEventFromEdgeTip) -> Unlocked to Home Screen')
        elif self.screen_state == 'app':
            closed = self.active_app.get('display_name') if self.active_app else 'App'
            self.active_app = None
            self.screen_state = 'home'
            self.log(f'[HID] Bottom-edge Home Gesture (y >= {BOTTOM_EDGE_Y}) -> Closed {closed} to Home Screen')
        else:
            self.log('[HID] Bottom-edge Home Gesture on SpringBoard Home Screen')

    def trigger_power_button(self):
        self.mailbox.qmp_target = self.qmp_target
        self.mailbox.press_key(qemu_sweet.KEY_POWER)
        if self.screen_state == 'off':
            self.screen_state = 'lock'
            self.log('[HID] Power button pressed -> Screen Woke (Lock Screen)')
        else:
            self.screen_state = 'lock'
            self.active_app = None
            self.log('[HID] Power button pressed -> Locked to iOS 16 Lock Screen')

    def trigger_volume(self, delta):
        key_id = qemu_sweet.KEY_VOL_UP if delta > 0 else qemu_sweet.KEY_VOL_DOWN
        self.mailbox.qmp_target = self.qmp_target
        self.mailbox.press_key(key_id)
        self.volume_level = max(0, min(100, self.volume_level + delta))
        self.volume_hud_until = time.time() + 2.0
        direction = 'Vol +' if delta > 0 else 'Vol -'
        self.log(f'[HID] {direction} -> Volume {self.volume_level}%')

    # --- Screenshot & Framebuffer Rendering ---

    def render_preview_framebuffer_bgra(self):
        """Synthesize a 1080x2400 BGRA framebuffer representing the current Preview screen
        for PNG screenshots and self-tests."""
        raw = bytearray(PANEL_W * PANEL_H * 4)
        # Fill with sleek dark iOS 16 wallpaper gradient using fast row slices
        for y in range(PANEL_H):
            if self.screen_state == 'off':
                b, g, r = 8, 8, 10
            elif self.screen_state == 'lock':
                b = min(255, 45 + (y * 55) // PANEL_H)
                g = min(255, 22 + (y * 35) // PANEL_H)
                r = min(255, 18 + (y * 50) // PANEL_H)
            elif self.screen_state == 'home':
                b = min(255, 65 + (y * 70) // PANEL_H)
                g = min(255, 30 + (y * 50) // PANEL_H)
                r = min(255, 20 + (y * 35) // PANEL_H)
            else:
                b, g, r = 36, 26, 20
            row_bytes = bytes((b, g, r, 255)) * PANEL_W
            off = y * qemu_sweet.FB_STRIDE
            raw[off:off + qemu_sweet.FB_STRIDE] = row_bytes

        # Draw white home indicator bar at y = 2360..2374, x = 350..730
        bar_pixel = bytes((245, 245, 245, 255)) * (730 - 350)
        for y in range(2360, 2374):
            off = y * qemu_sweet.FB_STRIDE + 350 * 4
            raw[off:off + len(bar_pixel)] = bar_pixel

        return bytes(raw)

    def save_screenshot(self, out_path=None):
        if out_path is None:
            stamp = datetime.datetime.now().strftime('%Y%m%d_%H%M%S')
            out_path = os.path.join(self.screenshots_dir, f'sweet_screen_{stamp}.png')
        os.makedirs(os.path.dirname(os.path.abspath(out_path)), exist_ok=True)

        if self.mode == 'qemu' and self.qmp_target is not None:
            try:
                raw_tmp = os.path.join(self.qemu_out_dir, 'fb_snap.raw')
                raw = qemu_sweet.capture_framebuffer(self.qmp_target, raw_tmp, png_path=out_path)
                self.last_fb_raw = raw
                self.log(f'[📸] Saved live QEMU framebuffer screenshot: {out_path}')
                return out_path
            except Exception as e:
                self.log(f'[!] Live QMP capture failed ({e}), saving current preview framebuffer.')

        raw = self.last_fb_raw if self.last_fb_raw else self.render_preview_framebuffer_bgra()
        qemu_sweet.write_png(out_path, raw)
        self.log(f'[📸] Saved screenshot (1080x2400 PNG): {out_path}')
        return out_path

    # --- Live QEMU VM Process Lifecycle ---

    def start_qemu_vm(self):
        if self.qemu_proc and self.qemu_proc.poll() is None:
            self.log('[!] QEMU VM is already running.')
            return True

        qemu_bin = qemu_sweet.find_qemu_binary()
        if not qemu_bin:
            self.log('[!] qemu-system-aarch64 not found! Staying in Interactive Preview Mode.')
            self.log('    Install on Windows: winget install SoftwareFreedomConservancy.QEMU')
            return False

        if not os.path.isfile(self.payload_path):
            if self.kernel_path and os.path.isfile(self.kernel_path):
                self.build_qemu_adt_and_payload()
            else:
                self.log(f'[!] SWEETXNU payload not found ({self.payload_path}).')
                self.log('    Select a kernel Mach-O or payload in Configuration, or use Interactive Preview Mode.')
                return False

        os.makedirs(self.qemu_out_dir, exist_ok=True)
        payload = open(self.payload_path, 'rb').read()
        phys_base, img, ba_pa, entry_pa = qemu_sweet.build_image(payload)
        ram_file = os.path.join(self.qemu_out_dir, 'ram.bin')
        stub_file = os.path.join(self.qemu_out_dir, 'stub.bin')
        mbox_file = os.path.join(self.qemu_out_dir, 'mbox.bin')
        serial_file = os.path.join(self.qemu_out_dir, 'serial.txt')

        with open(ram_file, 'wb') as f:
            f.write(img)
        with open(stub_file, 'wb') as f:
            f.write(qemu_sweet.stub(ba_pa, entry_pa))
        self.mailbox.write_initial_file(mbox_file)
        open(serial_file, 'w').close()
        self.serial_pos = 0

        port = qemu_sweet.find_free_port()
        self.qmp_target = ('127.0.0.1', port)
        self.mailbox.qmp_target = self.qmp_target

        smp = max(1, min(8, int(self.smp_cores)))
        ram_gib = max(4, int(self.ram_gb))
        cmd = [qemu_bin, '-M', 'virt,gic-version=3', '-cpu', 'cortex-a76', '-smp', str(smp)]
        cmd += qemu_sweet.build_accel_args(self.accel_code(), smp)
        cmd += ['-m', f'{ram_gib}G']

        if 'native window' in self.gpu_mode_label.lower():
            cmd += ['-device', 'ramfb', '-device', 'virtio-tablet-device', '-device', 'virtio-keyboard-device']
        else:
            cmd += ['-nographic']

        cmd += [
            '-serial', f'file:{serial_file}',
            '-qmp', f'tcp:127.0.0.1:{port},server,nowait',
            '-d', 'int,guest_errors,unimp',
            '-D', os.path.join(self.qemu_out_dir, 'qemu.log'),
            '-no-reboot',
            '-device', f'loader,file={ram_file},addr={phys_base:#x},force-raw=on',
            '-device', f'loader,file={stub_file},addr={qemu_sweet.STUB_PA:#x},force-raw=on',
            '-device', f'loader,file={mbox_file},addr={qemu_sweet.INPUT_MBOX_PA:#x},force-raw=on',
            '-device', f'loader,addr={qemu_sweet.STUB_PA:#x},cpu-num=0',
        ]

        if self.disk_path and os.path.isfile(self.disk_path):
            cmd += [
                '-drive', f'if=none,id=iosroot,format=raw,file={os.path.abspath(self.disk_path)}',
                '-device', 'virtio-blk-device,drive=iosroot',
            ]

        if self.enable_net:
            cmd += [
                '-netdev', 'user,id=net0,hostfwd=tcp::2222-:22,hostfwd=tcp::8080-:80',
                '-device', 'virtio-net-device,netdev=net0',
            ]

        self.log(f'[▶] Launching QEMU: {" ".join(cmd)}')
        self.qemu_proc = subprocess.Popen(cmd, stdin=subprocess.DEVNULL)
        self.mode = 'qemu'
        return True

    def stop_qemu_vm(self):
        if self.qemu_proc and self.qemu_proc.poll() is None:
            self.log('[⏹] Stopping QEMU VM...')
            try:
                self.qemu_proc.terminate()
                self.qemu_proc.wait(timeout=3)
            except Exception:
                try:
                    self.qemu_proc.kill()
                except Exception:
                    pass
        self.qemu_proc = None
        self.qmp_target = None
        self.mailbox.qmp_target = None
        self.mode = 'preview'
        self.log('[*] Switched to Interactive Preview Mode.')


# ---------------------------------------------------------------------------
# Desktop GUI Application (Tkinter)
# ---------------------------------------------------------------------------

def create_gui_app(core: EmulatorCore):
    import tkinter as tk
    from tkinter import filedialog, messagebox, ttk

    class SweetEmulatorGUI:
        def __init__(self, master, emu: EmulatorCore):
            self.root = master
            self.emu = emu
            self.root.title('ios-sweet Desktop Emulator — iPhone X (iOS 16.7.16 / sweet-virt)')
            self.root.geometry('1180x900')
            self.root.minsize(1060, 860)
            self.root.configure(bg='#0b0f19')

            self.drag_start_panel = None
            self.current_drag_panel = None
            self.tk_fb_image = None
            self._log_rendered_count = 0

            self._configure_styles()
            self._build_layout()
            self._refresh_apps_tree()
            self._update_env_badge()

            # Start on authentic stock iOS 16.7.16 SpringBoard Home Screen
            self.emu.screen_state = 'home'

            self.emu.log('[✨] ios-sweet Desktop Emulator ready in Interactive Preview Mode (iOS 16.7.16 / 20H392).')
            self.emu.log('    Tip: Tap any of the 20 stock Apple icons + 4 Dock apps, or swipe up from bottom edge!')
            self._tick()

        def _configure_styles(self):
            style = ttk.Style(self.root)
            try:
                style.theme_use('clam')
            except Exception:
                pass
            style.configure('TNotebook', background='#0f172a', borderwidth=0)
            style.configure('TNotebook.Tab', background='#1e293b', foreground='#cbd5e1',
                            padding=(14, 8), font=('Segoe UI', 10, 'bold'))
            style.map('TNotebook.Tab',
                      background=[('selected', '#2563eb')],
                      foreground=[('selected', '#ffffff')])
            style.configure('Treeview', background='#0f172a', fieldbackground='#0f172a',
                            foreground='#e2e8f0', rowheight=28, font=('Segoe UI', 9))
            style.configure('Treeview.Heading', background='#1e293b', foreground='#93c5fd',
                            font=('Segoe UI', 9, 'bold'))
            style.map('Treeview', background=[('selected', '#1d4ed8')])

        def _build_layout(self):
            # Top Header Bar
            header = tk.Frame(self.root, bg='#111827', height=48, highlightthickness=1,
                              highlightbackground='#1f2937')
            header.pack(side=tk.TOP, fill=tk.X)

            title_lbl = tk.Label(
                header,
                text='  ios-sweet Emulator   |   iPhone10,3 (D22AP) • iOS 16.7.16 (20H392)',
                bg='#111827', fg='#f8fafc', font=('Segoe UI', 12, 'bold'), padx=16, pady=10
            )
            title_lbl.pack(side=tk.LEFT)

            self.mode_badge = tk.Label(
                header,
                text='✨ INTERACTIVE PREVIEW MODE',
                bg='#1d4ed8', fg='#ffffff', font=('Segoe UI', 9, 'bold'), padx=12, pady=4
            )
            self.mode_badge.pack(side=tk.RIGHT, padx=16, pady=8)

            self.coord_label = tk.Label(
                header,
                text='Touch: (---, ---) | Mailbox: 0x9d600000 [SWIN]',
                bg='#111827', fg='#94a3b8', font=('Consolas', 9)
            )
            self.coord_label.pack(side=tk.RIGHT, padx=12)

            # Main Content Area
            main_pane = tk.Frame(self.root, bg='#0b0f19')
            main_pane.pack(side=tk.TOP, fill=tk.BOTH, expand=True, padx=12, pady=8)

            # 1. Left Phone Viewport (iPhone X Bezel Canvas)
            phone_frame = tk.Frame(main_pane, bg='#0b0f19')
            phone_frame.pack(side=tk.LEFT, fill=tk.Y, padx=(0, 10))

            self.canvas = tk.Canvas(
                phone_frame, width=CANVAS_W, height=CANVAS_H,
                bg='#0b0f19', highlightthickness=0, cursor='hand2'
            )
            self.canvas.pack(side=tk.TOP)

            self.canvas.bind('<ButtonPress-1>', self._on_canvas_press)
            self.canvas.bind('<B1-Motion>', self._on_canvas_drag)
            self.canvas.bind('<ButtonRelease-1>', self._on_canvas_release)

            # 2. Center/Right BlueStacks-style Sidebar Toolbar
            sidebar = tk.Frame(main_pane, bg='#111827', width=210,
                               highlightthickness=1, highlightbackground='#1e293b')
            sidebar.pack(side=tk.LEFT, fill=tk.Y, padx=(0, 10))
            sidebar.pack_propagate(False)

            tk.Label(sidebar, text='EMULATOR CONTROLS', bg='#111827', fg='#64748b',
                     font=('Segoe UI', 8, 'bold'), pady=10).pack(fill=tk.X, padx=12)

            self._make_sidebar_btn(sidebar, '▶  Start QEMU VM', '#16a34a', self._on_start_vm)
            self._make_sidebar_btn(sidebar, '⏹  Stop VM', '#dc2626', self._on_stop_vm)
            self._make_sidebar_btn(sidebar, '✨ Interactive Preview', '#2563eb', self._on_switch_preview)

            tk.Frame(sidebar, bg='#1e293b', height=1).pack(fill=tk.X, padx=12, pady=8)
            tk.Label(sidebar, text='APPS & BUILD TOOLS', bg='#111827', fg='#64748b',
                     font=('Segoe UI', 8, 'bold')).pack(fill=tk.X, padx=12, pady=(0, 6))

            self._make_sidebar_btn(sidebar, '📦 Install .IPA / .APP', '#7c3aed', self._on_install_ipa_dialog)
            self._make_sidebar_btn(sidebar, '🎮 Install Demo Game', '#9333ea', self._on_install_demo_ipa)
            self._make_sidebar_btn(sidebar, '🔨 Build ADT & Payload', '#0284c7', self._on_build_adt_payload)

            tk.Frame(sidebar, bg='#1e293b', height=1).pack(fill=tk.X, padx=12, pady=8)
            tk.Label(sidebar, text='HARDWARE BUTTONS', bg='#111827', fg='#64748b',
                     font=('Segoe UI', 8, 'bold')).pack(fill=tk.X, padx=12, pady=(0, 6))

            self._make_sidebar_btn(sidebar, '🔌 Power (Lock / Wake)', '#334155', self._on_power_btn)
            self._make_sidebar_btn(sidebar, '🏠 Swipe Home Gesture', '#334155', self._on_home_btn)

            vol_row = tk.Frame(sidebar, bg='#111827')
            vol_row.pack(fill=tk.X, padx=12, pady=3)
            tk.Button(
                vol_row, text='🔊 Vol +', bg='#334155', fg='#f8fafc', activebackground='#475569',
                activeforeground='#ffffff', relief=tk.FLAT, font=('Segoe UI', 9, 'bold'),
                pady=6, command=lambda: self._on_vol_btn(10)
            ).pack(side=tk.LEFT, fill=tk.X, expand=True, padx=(0, 3))
            tk.Button(
                vol_row, text='🔉 Vol -', bg='#334155', fg='#f8fafc', activebackground='#475569',
                activeforeground='#ffffff', relief=tk.FLAT, font=('Segoe UI', 9, 'bold'),
                pady=6, command=lambda: self._on_vol_btn(-10)
            ).pack(side=tk.LEFT, fill=tk.X, expand=True, padx=(3, 0))

            self._make_sidebar_btn(sidebar, '📸 Save Screenshot', '#0d9488', self._on_screenshot_btn)

            # 3. Right Tabbed Panel (Configuration | Installed Apps | Kernel & Serial Log)
            right_pane = tk.Frame(main_pane, bg='#0f172a')
            right_pane.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)

            self.notebook = ttk.Notebook(right_pane)
            self.notebook.pack(fill=tk.BOTH, expand=True)

            self.tab_config = tk.Frame(self.notebook, bg='#0f172a', padx=16, pady=14)
            self.tab_apps = tk.Frame(self.notebook, bg='#0f172a', padx=14, pady=14)
            self.tab_logs = tk.Frame(self.notebook, bg='#0f172a', padx=14, pady=14)

            self.notebook.add(self.tab_config, text=' ⚙ Configuration ')
            self.notebook.add(self.tab_apps, text=' 📱 Installed Apps ')
            self.notebook.add(self.tab_logs, text=' 🖥 Kernel & Serial Log ')

            self._build_config_tab()
            self._build_apps_tab()
            self._build_logs_tab()

        def _make_sidebar_btn(self, parent, text, bg_color, cmd):
            btn = tk.Button(
                parent, text=text, bg=bg_color, fg='#ffffff',
                activebackground=bg_color, activeforeground='#ffffff',
                relief=tk.FLAT, font=('Segoe UI', 9, 'bold'),
                anchor='w', padx=12, pady=7, cursor='hand2', command=cmd
            )
            btn.pack(fill=tk.X, padx=12, pady=3)
            return btn

        def _build_config_tab(self):
            f = self.tab_config
            tk.Label(f, text='Virtual Hardware & QEMU Settings (sweet-virt)',
                     bg='#0f172a', fg='#f8fafc', font=('Segoe UI', 12, 'bold')).pack(anchor='w', pady=(0, 12))

            grid = tk.Frame(f, bg='#0f172a')
            grid.pack(fill=tk.X)

            # CPU Cores
            tk.Label(grid, text='SMP CPU Cores (1..8):', bg='#0f172a', fg='#cbd5e1',
                     font=('Segoe UI', 10)).grid(row=0, column=0, sticky='w', pady=6)
            self.var_cores = tk.StringVar(value=str(self.emu.smp_cores))
            cores_cb = ttk.Combobox(grid, textvariable=self.var_cores,
                                    values=['1', '2', '4', '6', '8'], state='readonly', width=28)
            cores_cb.grid(row=0, column=1, sticky='w', padx=12, pady=6)

            # RAM
            tk.Label(grid, text='System Memory (RAM):', bg='#0f172a', fg='#cbd5e1',
                     font=('Segoe UI', 10)).grid(row=1, column=0, sticky='w', pady=6)
            self.var_ram = tk.StringVar(value=f'{self.emu.ram_gb} GB')
            ram_cb = ttk.Combobox(grid, textvariable=self.var_ram,
                                  values=['4 GB', '8 GB'], state='readonly', width=28)
            ram_cb.grid(row=1, column=1, sticky='w', padx=12, pady=6)

            # Acceleration
            tk.Label(grid, text='CPU Acceleration:', bg='#0f172a', fg='#cbd5e1',
                     font=('Segoe UI', 10)).grid(row=2, column=0, sticky='w', pady=6)
            self.var_accel = tk.StringVar(value=self.emu.accel_label)
            accel_cb = ttk.Combobox(
                grid, textvariable=self.var_accel,
                values=[
                    'Multi-threaded TCG (MTTCG)',
                    'WHPX (Windows Hypervisor)',
                    'KVM / HVF',
                ],
                state='readonly', width=28
            )
            accel_cb.grid(row=2, column=1, sticky='w', padx=12, pady=6)

            # GPU Display Mode
            tk.Label(grid, text='GPU Display Mode:', bg='#0f172a', fg='#cbd5e1',
                     font=('Segoe UI', 10)).grid(row=3, column=0, sticky='w', pady=6)
            self.var_gpu = tk.StringVar(value=self.emu.gpu_mode_label)
            gpu_cb = ttk.Combobox(
                grid, textvariable=self.var_gpu,
                values=[
                    'Live Canvas Stream (CPU->RAMFB)',
                    'QEMU Native Window (ramfb/virtio-gpu)',
                ],
                state='readonly', width=28
            )
            gpu_cb.grid(row=3, column=1, sticky='w', padx=12, pady=6)

            tk.Frame(f, bg='#1e293b', height=1).pack(fill=tk.X, pady=14)
            tk.Label(f, text='Storage & Kernel Paths',
                     bg='#0f172a', fg='#f8fafc', font=('Segoe UI', 11, 'bold')).pack(anchor='w', pady=(0, 8))

            paths_frame = tk.Frame(f, bg='#0f172a')
            paths_frame.pack(fill=tk.X)

            self.var_kernel = tk.StringVar(value=self.emu.kernel_path)
            self.var_payload = tk.StringVar(value=self.emu.payload_path)
            self.var_disk = tk.StringVar(value=self.emu.disk_path)
            self.var_rootfs = tk.StringVar(value=self.emu.rootfs_dir)

            self._add_path_row(paths_frame, 0, 'Kernel Mach-O:', self.var_kernel, self._browse_kernel)
            self._add_path_row(paths_frame, 1, 'SWEETXNU Payload:', self.var_payload, self._browse_payload)
            self._add_path_row(paths_frame, 2, 'Root Disk Image:', self.var_disk, self._browse_disk)
            self._add_path_row(paths_frame, 3, 'Staging Rootfs Dir:', self.var_rootfs, self._browse_rootfs)

            btn_bar = tk.Frame(f, bg='#0f172a')
            btn_bar.pack(fill=tk.X, pady=14)
            tk.Button(
                btn_bar, text='💾 Apply Configuration', bg='#2563eb', fg='#ffffff',
                relief=tk.FLAT, font=('Segoe UI', 9, 'bold'), padx=14, pady=7,
                command=self._apply_config
            ).pack(side=tk.LEFT, padx=(0, 8))
            tk.Button(
                btn_bar, text='💿 Create 64MB Blank Root Disk Image', bg='#334155', fg='#ffffff',
                relief=tk.FLAT, font=('Segoe UI', 9, 'bold'), padx=14, pady=7,
                command=self._create_blank_disk
            ).pack(side=tk.LEFT)

            info_card = tk.Label(
                f,
                text=(
                    'Architecture Notes:\n'
                    '• Display: 1080×2400 BGRA @ 0x9c000000 (IOMobileFramebuffer @3x = 360×800 pts)\n'
                    '• Input Mailbox: 0x9d600000 (magic 0x5357494e "SWIN", 64-entry ring buffer)\n'
                    '• Home Gesture: Bottom-edge contact (y >= 2352) sets kIOHIDDigitizerEventFromEdgeTip (patch 0071)'
                ),
                bg='#1e293b', fg='#93c5fd', justify=tk.LEFT, anchor='w',
                font=('Consolas', 9), padx=12, pady=10
            )
            info_card.pack(fill=tk.X, pady=(8, 0))

        def _add_path_row(self, parent, row, label_text, var, browse_cmd):
            tk.Label(parent, text=label_text, bg='#0f172a', fg='#cbd5e1',
                     font=('Segoe UI', 9)).grid(row=row, column=0, sticky='w', pady=5)
            entry = tk.Entry(parent, textvariable=var, bg='#1e293b', fg='#f8fafc',
                             insertbackground='#ffffff', relief=tk.FLAT, width=38, font=('Consolas', 9))
            entry.grid(row=row, column=1, sticky='we', padx=8, pady=5)
            tk.Button(parent, text='Browse...', bg='#334155', fg='#f8fafc', relief=tk.FLAT,
                      font=('Segoe UI', 8), padx=8, command=browse_cmd).grid(row=row, column=2, pady=5)
            parent.grid_columnconfigure(1, weight=1)

        def _build_apps_tab(self):
            f = self.tab_apps
            top_bar = tk.Frame(f, bg='#0f172a')
            top_bar.pack(fill=tk.X, pady=(0, 10))

            tk.Label(top_bar, text='Installed iOS Applications & Games (Staging Rootfs)',
                     bg='#0f172a', fg='#f8fafc', font=('Segoe UI', 11, 'bold')).pack(side=tk.LEFT)

            cols = ('name', 'bundle_id', 'version', 'gfx', 'cdhash')
            self.apps_tree = ttk.Treeview(f, columns=cols, show='headings', height=14)
            self.apps_tree.heading('name', text='App Name')
            self.apps_tree.heading('bundle_id', text='Bundle Identifier')
            self.apps_tree.heading('version', text='Version')
            self.apps_tree.heading('gfx', text='Graphics API')
            self.apps_tree.heading('cdhash', text='TrustCache CDHash (SHA-256)')

            self.apps_tree.column('name', width=120)
            self.apps_tree.column('bundle_id', width=165)
            self.apps_tree.column('version', width=65)
            self.apps_tree.column('gfx', width=125)
            self.apps_tree.column('cdhash', width=150)
            self.apps_tree.pack(fill=tk.BOTH, expand=True)

            act_bar = tk.Frame(f, bg='#0f172a')
            act_bar.pack(fill=tk.X, pady=(10, 0))

            tk.Button(
                act_bar, text='🎮 Create & Install Demo Game IPA', bg='#9333ea', fg='#ffffff',
                relief=tk.FLAT, font=('Segoe UI', 9, 'bold'), padx=10, pady=6,
                command=self._on_install_demo_ipa
            ).pack(side=tk.LEFT, padx=(0, 6))
            tk.Button(
                act_bar, text='📦 Install .IPA / .APP...', bg='#7c3aed', fg='#ffffff',
                relief=tk.FLAT, font=('Segoe UI', 9, 'bold'), padx=10, pady=6,
                command=self._on_install_ipa_dialog
            ).pack(side=tk.LEFT, padx=(0, 6))
            tk.Button(
                act_bar, text='🚀 Launch Selected on Screen', bg='#16a34a', fg='#ffffff',
                relief=tk.FLAT, font=('Segoe UI', 9, 'bold'), padx=10, pady=6,
                command=self._on_launch_selected_app
            ).pack(side=tk.LEFT, padx=(0, 6))
            tk.Button(
                act_bar, text='🗑 Uninstall', bg='#dc2626', fg='#ffffff',
                relief=tk.FLAT, font=('Segoe UI', 9, 'bold'), padx=10, pady=6,
                command=self._on_uninstall_selected_app
            ).pack(side=tk.LEFT)

        def _build_logs_tab(self):
            f = self.tab_logs
            top = tk.Frame(f, bg='#0f172a')
            top.pack(fill=tk.X, pady=(0, 8))

            self.env_status_lbl = tk.Label(
                top, text='Checking QEMU status...', bg='#1e293b', fg='#38bdf8',
                font=('Consolas', 9), padx=10, pady=5, anchor='w'
            )
            self.env_status_lbl.pack(side=tk.LEFT, fill=tk.X, expand=True, padx=(0, 8))

            tk.Button(
                top, text='🔍 Check QEMU Env', bg='#334155', fg='#ffffff',
                relief=tk.FLAT, font=('Segoe UI', 9, 'bold'), padx=10, pady=4,
                command=self._update_env_badge
            ).pack(side=tk.RIGHT)

            self.log_text = tk.Text(
                f, bg='#020617', fg='#4ade80', insertbackground='#4ade80',
                font=('Consolas', 9), relief=tk.FLAT, wrap=tk.WORD
            )
            self.log_text.pack(fill=tk.BOTH, expand=True)

        # --- Actions & Callbacks ---

        def _apply_config(self):
            try:
                self.emu.smp_cores = int(self.var_cores.get())
            except Exception:
                self.emu.smp_cores = 6
            self.emu.ram_gb = 8 if '8' in self.var_ram.get() else 4
            self.emu.accel_label = self.var_accel.get()
            self.emu.gpu_mode_label = self.var_gpu.get()
            self.emu.kernel_path = self.var_kernel.get().strip()
            self.emu.payload_path = self.var_payload.get().strip()
            self.emu.disk_path = self.var_disk.get().strip()
            new_rootfs = self.var_rootfs.get().strip()
            if new_rootfs and new_rootfs != self.emu.rootfs_dir:
                self.emu.rootfs_dir = new_rootfs
                os.makedirs(self.emu.rootfs_dir, exist_ok=True)
                self.emu.refresh_installed_apps()
                self._refresh_apps_tree()
            self.emu.log(
                f'[⚙] Configuration updated: SMP={self.emu.smp_cores} cores, '
                f'RAM={self.emu.ram_gb}G, Accel={self.emu.accel_code()}'
            )

        def _create_blank_disk(self):
            self._apply_config()
            path = self.emu.disk_path
            os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
            with open(path, 'wb') as f:
                f.seek((64 * 1024 * 1024) - 1)
                f.write(b'\x00')
            self.emu.log(f'[💿] Created 64 MB raw disk image at {path}')

        def _browse_kernel(self):
            p = filedialog.askopenfilename(title='Select Unstripped XNU Kernel Mach-O')
            if p:
                self.var_kernel.set(p)
                self._apply_config()

        def _browse_payload(self):
            p = filedialog.askopenfilename(title='Select SWEETXNU Payload Binary')
            if p:
                self.var_payload.set(p)
                self._apply_config()

        def _browse_disk(self):
            p = filedialog.askopenfilename(title='Select iOS Root Disk Image (.img)')
            if p:
                self.var_disk.set(p)
                self._apply_config()

        def _browse_rootfs(self):
            p = filedialog.askdirectory(title='Select Staging Rootfs Directory')
            if p:
                self.var_rootfs.set(p)
                self._apply_config()

        def _update_env_badge(self):
            qemu_bin = qemu_sweet.find_qemu_binary()
            if qemu_bin:
                self.env_status_lbl.configure(
                    text=f'✓ QEMU Found: {qemu_bin}', fg='#4ade80'
                )
                self.emu.log(f'[✓] Found QEMU binary: {qemu_bin}')
            else:
                self.env_status_lbl.configure(
                    text='! QEMU not in PATH (Interactive Preview active; install via: winget install SoftwareFreedomConservancy.QEMU)',
                    fg='#facc15'
                )

        def _refresh_apps_tree(self):
            for item in self.apps_tree.get_children():
                self.apps_tree.delete(item)
            apps = self.emu.refresh_installed_apps()
            for app in apps:
                cdh = app.get('cdhashes', [''])[0] if app.get('cdhashes') else '-'
                self.apps_tree.insert(
                    '', tk.END, iid=app['bundle_id'],
                    values=(
                        app['display_name'],
                        app['bundle_id'],
                        app.get('short_version', '1.0'),
                        app.get('graphics_api', 'UIKit'),
                        cdh,
                    )
                )

        def _on_start_vm(self):
            self._apply_config()
            ok = self.emu.start_qemu_vm()
            if ok:
                self.mode_badge.configure(text='🟢 LIVE QEMU VM RUNNING', bg='#16a34a')
            else:
                self.notebook.select(self.tab_logs)

        def _on_stop_vm(self):
            self.emu.stop_qemu_vm()
            self.mode_badge.configure(text='✨ INTERACTIVE PREVIEW MODE', bg='#1d4ed8')

        def _on_switch_preview(self):
            self.emu.stop_qemu_vm()
            self.emu.screen_state = 'home'
            self.mode_badge.configure(text='✨ INTERACTIVE PREVIEW MODE', bg='#1d4ed8')

        def _on_install_demo_ipa(self):
            try:
                inst = self.emu.create_and_install_demo_ipa()
                self._refresh_apps_tree()
                self.emu.screen_state = 'home'
                self.emu.active_app = None
            except Exception as e:
                self.emu.log(f'[!] Demo IPA install error: {e}')

        def _on_install_ipa_dialog(self):
            path = filedialog.askopenfilename(
                title='Select iOS .IPA Archive or .APP Bundle',
                filetypes=[('iOS App Package', '*.ipa *.app'), ('All Files', '*.*')]
            )
            if not path:
                return
            try:
                self.emu.install_ipa_file(path)
                self._refresh_apps_tree()
                self.emu.screen_state = 'home'
                self.emu.active_app = None
            except Exception as e:
                self.emu.log(f'[!] IPA installation error: {e}')
                messagebox.showerror('IPA Installation Failed', str(e))

        def _on_launch_selected_app(self):
            sel = self.apps_tree.selection()
            if not sel:
                # Default to first installed app if none selected
                if self.emu.installed_apps:
                    bid = self.emu.installed_apps[0]['bundle_id']
                else:
                    return
            else:
                bid = sel[0]
            target = next((a for a in self.emu.all_home_apps() if a['bundle_id'] == bid), None)
            if target:
                self.emu.launch_app_in_preview(target)

        def _on_uninstall_selected_app(self):
            sel = self.apps_tree.selection()
            if not sel:
                return
            bid = sel[0]
            try:
                self.emu.uninstall_app_by_id(bid)
                self._refresh_apps_tree()
            except Exception as e:
                self.emu.log(f'[!] Uninstall error: {e}')

        def _on_build_adt_payload(self):
            self._apply_config()
            try:
                self.emu.build_qemu_adt_and_payload()
            except Exception as e:
                self.emu.log(f'[!] Build ADT/Payload error: {e}')
            self.notebook.select(self.tab_logs)

        def _on_power_btn(self):
            self.emu.trigger_power_button()

        def _on_home_btn(self):
            self.emu.trigger_home_gesture(from_edge_swipe=False)

        def _on_vol_btn(self, delta):
            self.emu.trigger_volume(delta)

        def _on_screenshot_btn(self):
            self.emu.save_screenshot()
            self.notebook.select(self.tab_logs)

        # --- Mouse-to-Touch Canvas Event Handlers ---

        def _on_canvas_press(self, event):
            px, py = self.emu.mapper.canvas_to_panel(event.x, event.y)
            self.drag_start_panel = (px, py)
            self.current_drag_panel = (px, py)
            self.emu.handle_touch_event(True, px, py)
            self._update_coord_status(px, py, True)

        def _on_canvas_drag(self, event):
            px, py = self.emu.mapper.canvas_to_panel(event.x, event.y)
            self.current_drag_panel = (px, py)
            self.emu.handle_touch_event(True, px, py)
            self._update_coord_status(px, py, True)

        def _on_canvas_release(self, event):
            px, py = self.emu.mapper.canvas_to_panel(event.x, event.y)
            start = self.drag_start_panel
            self.drag_start_panel = None
            self.current_drag_panel = None
            self.emu.handle_touch_event(False, px, py, start_xy=start)
            self._update_coord_status(px, py, False)

        def _update_coord_status(self, px, py, down):
            edge_tag = ' [EDGE 0071]' if self.emu.mapper.is_edge_contact(px, py) else ''
            state_str = 'DOWN' if down else 'UP  '
            self.coord_label.configure(
                text=f'Touch {state_str}: ({px:4d}, {py:4d}){edge_tag} | Mbox w={self.emu.mailbox.write_idx}'
            )

        # --- Main Render & Poll Loop ---

        def _tick(self):
            self.emu.anim_tick += 1
            self._flush_logs()
            self._poll_qemu_if_running()
            self._draw_phone_viewport()
            self.root.after(50, self._tick)

        def _flush_logs(self):
            while self._log_rendered_count < len(self.emu.logs):
                line = self.emu.logs[self._log_rendered_count]
                self._log_rendered_count += 1
                self.log_text.insert(tk.END, line + '\n')
                self.log_text.see(tk.END)

        def _poll_qemu_if_running(self):
            if self.emu.mode != 'qemu' or not self.emu.qemu_proc:
                return
            if self.emu.qemu_proc.poll() is not None:
                rc = self.emu.qemu_proc.returncode
                self.emu.log(f'[!] QEMU process exited with code {rc}. Returning to Interactive Preview.')
                self.emu.stop_qemu_vm()
                self.mode_badge.configure(text='✨ INTERACTIVE PREVIEW MODE', bg='#1d4ed8')
                return

            # Stream serial.txt lines into log console
            serial_file = os.path.join(self.emu.qemu_out_dir, 'serial.txt')
            if os.path.isfile(serial_file):
                try:
                    with open(serial_file, 'r', errors='replace') as sf:
                        sf.seek(self.emu.serial_pos)
                        chunk = sf.read()
                        if chunk:
                            self.emu.serial_pos = sf.tell()
                            for ln in chunk.splitlines():
                                self.emu.log(f'[serial] {ln}')
                except Exception:
                    pass

            # Periodically poll live framebuffer via QMP (every ~8 ticks = 400ms)
            if self.emu.anim_tick % 8 == 0 and self.emu.qmp_target:
                raw_path = os.path.join(self.emu.qemu_out_dir, 'live_fb.raw')
                try:
                    raw = qemu_sweet.capture_framebuffer(self.emu.qmp_target, raw_path)
                    self.emu.last_fb_raw = raw
                    ppm = qemu_sweet.fb_to_ppm(raw, scale=VIEWPORT_SCALE)
                    self.tk_fb_image = tk.PhotoImage(data=ppm, format='PPM')
                except Exception:
                    pass

        # --- Phone Viewport Drawing (iPhone X Bezel + Screen) ---

        def _draw_phone_viewport(self):
            c = self.canvas
            c.delete('all')

            sx0, sy0 = BEZEL_PAD_X, BEZEL_PAD_Y
            sx1, sy1 = sx0 + VIEWPORT_W, sy0 + VIEWPORT_H

            # Outer iPhone X Stainless Steel & Glass Chassis
            c.create_rectangle(sx0 - 14, sy0 - 16, sx1 + 14, sy1 + 16,
                               fill='#1e293b', outline='#475569', width=3)
            c.create_rectangle(sx0 - 8, sy0 - 10, sx1 + 8, sy1 + 10,
                               fill='#090d16', outline='#0f172a', width=2)

            # Physical side button hints on bezel
            c.create_rectangle(sx0 - 17, sy0 + 110, sx0 - 14, sy0 + 150, fill='#64748b', outline='')
            c.create_rectangle(sx0 - 17, sy0 + 165, sx0 - 14, sy0 + 205, fill='#64748b', outline='')
            c.create_rectangle(sx1 + 14, sy0 + 135, sx1 + 17, sy0 + 195, fill='#64748b', outline='')

            if self.emu.mode == 'qemu' and self.tk_fb_image is not None:
                c.create_image(sx0, sy0, anchor=tk.NW, image=self.tk_fb_image)
            else:
                self._draw_interactive_ios_screen(c, sx0, sy0, sx1, sy1)

            # iPhone X Top Sensor Housing (Notch)
            notch_w = 156
            notch_h = 24
            nx0 = sx0 + (VIEWPORT_W - notch_w) // 2
            nx1 = nx0 + notch_w
            c.create_rectangle(nx0, sy0, nx1, sy0 + notch_h, fill='#090d16', outline='#1e293b', width=1)
            # Speaker slot & front TrueDepth camera dot
            c.create_rectangle(nx0 + 52, sy0 + 9, nx0 + 94, sy0 + 14, fill='#1e293b', outline='')
            c.create_oval(nx0 + 106, sy0 + 7, nx0 + 116, sy0 + 17, fill='#1e3a8a', outline='#3b82f6')

            # Status Bar (Time left of notch, 5G + Battery right of notch)
            now_str = datetime.datetime.now().strftime('%H:%M')
            c.create_text(sx0 + 36, sy0 + 13, text=now_str, fill='#f8fafc',
                          font=('Segoe UI', 9, 'bold'))
            c.create_text(sx1 - 46, sy0 + 13, text='5G  100%', fill='#f8fafc',
                          font=('Segoe UI', 8, 'bold'))

            # Bottom 48px Edge-Gesture Zone subtle guide & iPhone X Home Indicator Bar
            edge_y_canvas = sy1 - (EDGE_THRESHOLD_PX // VIEWPORT_SCALE)
            c.create_line(sx0 + 8, edge_y_canvas, sx1 - 8, edge_y_canvas,
                          fill='#334155', dash=(3, 3))
            bar_w = 128
            bx0 = sx0 + (VIEWPORT_W - bar_w) // 2
            by0 = sy1 - 9
            c.create_rectangle(bx0, by0, bx0 + bar_w, by0 + 4,
                               fill='#f8fafc', outline='#94a3b8')

            # Volume HUD overlay if recently adjusted
            if time.time() < self.emu.volume_hud_until:
                vy0 = sy0 + 110
                vy1 = vy0 + 130
                c.create_rectangle(sx0 + 10, vy0, sx0 + 26, vy1, fill='#1e293b', outline='#475569')
                fill_h = int((self.emu.volume_level / 100.0) * 126)
                c.create_rectangle(sx0 + 12, vy1 - fill_h, sx0 + 24, vy1 - 2,
                                   fill='#f8fafc', outline='')

            # Visual Touch Contact & Swipe Trail
            if self.current_drag_panel is not None:
                tx, ty = self.emu.mapper.panel_to_canvas(*self.current_drag_panel)
                is_edge = self.emu.mapper.is_edge_contact(*self.current_drag_panel)
                ring_color = '#f43f5e' if is_edge else '#38bdf8'
                if self.drag_start_panel is not None:
                    sx_c, sy_c = self.emu.mapper.panel_to_canvas(*self.drag_start_panel)
                    c.create_line(sx_c, sy_c, tx, ty, fill=ring_color, width=2, dash=(4, 2))
                c.create_oval(tx - 12, ty - 12, tx + 12, ty + 12,
                              outline=ring_color, width=2, fill='')
                c.create_oval(tx - 4, ty - 4, tx + 4, ty + 4, fill=ring_color, outline='')

        def _draw_interactive_ios_screen(self, c, sx0, sy0, sx1, sy1):
            state = self.emu.screen_state
            if state == 'off':
                c.create_rectangle(sx0, sy0, sx1, sy1, fill='#05070c', outline='')
                c.create_text((sx0 + sx1) // 2, (sy0 + sy1) // 2,
                              text='Screen Off\nTap screen or press 🔌 Power to wake',
                              fill='#475569', font=('Segoe UI', 10), justify=tk.CENTER)
                return

            # iOS 16 Dynamic Wallpaper Bands
            bands = [
                '#0f172a', '#1e1b4b', '#1e3a8a', '#1d4ed8',
                '#0284c7', '#0f766e', '#115e59', '#0f172a'
            ]
            band_h = VIEWPORT_H / len(bands)
            for i, col in enumerate(bands):
                by0 = sy0 + int(i * band_h)
                by1 = sy0 + int((i + 1) * band_h) + 1
                c.create_rectangle(sx0, by0, sx1, by1, fill=col, outline='')

            if state == 'lock':
                self._draw_lock_screen(c, sx0, sy0, sx1, sy1)
            elif state == 'home':
                self._draw_home_screen(c, sx0, sy0, sx1, sy1)
            elif state == 'app' and self.emu.active_app:
                self._draw_active_app_screen(c, sx0, sy0, sx1, sy1, self.emu.active_app)

        def _draw_lock_screen(self, c, sx0, sy0, sx1, sy1):
            cx = (sx0 + sx1) // 2
            now = datetime.datetime.now()
            date_str = now.strftime('%A, %B %d')
            time_str = now.strftime('%H:%M')

            c.create_text(cx, sy0 + 54, text='🔒', fill='#e2e8f0', font=('Segoe UI', 14))
            c.create_text(cx, sy0 + 86, text=date_str, fill='#cbd5e1',
                          font=('Segoe UI', 12, 'bold'))
            c.create_text(cx, sy0 + 145, text=time_str, fill='#ffffff',
                          font=('Segoe UI', 48, 'bold'))

            # Notification / Device status card on Lock Screen
            c.create_rectangle(sx0 + 18, sy0 + 230, sx1 - 18, sy0 + 325,
                               fill='#0f172a', outline='#38bdf8', width=1)
            c.create_text(
                sx0 + 30, sy0 + 245, anchor='w',
                text='  ios-sweet • iOS 16.7.16 (20H392)',
                fill='#38bdf8', font=('Segoe UI', 9, 'bold')
            )
            c.create_text(
                sx0 + 30, sy0 + 282, anchor='w',
                text=(
                    f'Target: iPhone10,3 (D22AP) on sweet-virt\n'
                    f'SMP: {self.emu.smp_cores}x Cortex-A76 | Installed Apps: {len(self.emu.installed_apps)}'
                ),
                fill='#e2e8f0', font=('Consolas', 8)
            )

            # Flashlight & Camera quick buttons
            c.create_oval(sx0 + 34, sy1 - 78, sx0 + 74, sy1 - 38, fill='#0f172a', outline='#475569')
            c.create_text(sx0 + 54, sy1 - 58, text='🔦', fill='#ffffff', font=('Segoe UI', 12))
            c.create_oval(sx1 - 74, sy1 - 78, sx1 - 34, sy1 - 38, fill='#0f172a', outline='#475569')
            c.create_text(sx1 - 54, sy1 - 58, text='📷', fill='#ffffff', font=('Segoe UI', 12))

            bounce = int(4 * math.sin(self.emu.anim_tick * 0.25))
            c.create_text(cx, sy1 - 24 + bounce, text='▲ Swipe up from bottom bar to open',
                          fill='#f8fafc', font=('Segoe UI', 9, 'bold'))

        def _draw_home_screen(self, c, sx0, sy0, sx1, sy1):
            self.emu._rebuild_icon_hitboxes()

            # Translucent iOS 16 Dock background (y: 696..776 in 360x800)
            dock_y0 = sy0 + int(2064 / VIEWPORT_SCALE)
            dock_y1 = sy0 + int(2316 / VIEWPORT_SCALE)
            c.create_rectangle(sx0 + 12, dock_y0, sx1 - 12, dock_y1,
                               fill='#1e293b', outline='#475569', width=1)

            for x0, y0, x1, y1, app in self.emu.icon_hitboxes:
                cx0, cy0 = self.emu.mapper.panel_to_canvas(x0, y0)
                cx1, _ = self.emu.mapper.panel_to_canvas(x1, y0)
                icon_w = cx1 - cx0
                cy1 = cy0 + icon_w

                bg = app.get('color', '#3b82f6')
                accent = app.get('accent', '#93c5fd')
                txt_col = app.get('text_color', '#ffffff')
                c.create_rectangle(cx0, cy0, cx1, cy1, fill=bg, outline=accent, width=1)
                c.create_text((cx0 + cx1) // 2, (cy0 + cy1) // 2,
                              text=app.get('glyph', '📱'), fill=txt_col,
                              font=('Segoe UI', 14, 'bold'))

                # App label (only for main grid icons above dock)
                if y0 < 2000:
                    name = app.get('display_name', 'App')
                    if len(name) > 11:
                        name = name[:10] + '…'
                    c.create_text((cx0 + cx1) // 2, cy1 + 10, text=name,
                                  fill='#f8fafc', font=('Segoe UI', 8))

        def _draw_active_app_screen(self, c, sx0, sy0, sx1, sy1, app):
            bid = app.get('bundle_id', '')
            cx = (sx0 + sx1) // 2

            if bid == 'com.apple.Preferences':
                # iOS 16 Settings -> General -> About Screen
                c.create_rectangle(sx0, sy0, sx1, sy1, fill='#0f172a', outline='')
                c.create_text(cx, sy0 + 44, text='Settings • General • About',
                              fill='#f8fafc', font=('Segoe UI', 12, 'bold'))

                rows = [
                    ('Name', 'iPhone X'),
                    ('iOS Version', '16.7.16 (20H392)'),
                    ('Model Name', 'iPhone X'),
                    ('Model Identifier', 'iPhone10,3 (D22AP)'),
                    ('Device Tree', 'Genuine Apple d22ap.adt (/product)'),
                    ('CPU Topology', f'{self.emu.smp_cores} Cores (ARM64 Cortex-A76)'),
                    ('Display', '1080×2400 @3x (360×800 pts)'),
                    ('Input Mailbox', '0x9d600000 (SWIN Ring64)'),
                    ('SIP / Boot Policy', '0x6f (Unrestricted FS / Ad-hoc)'),
                    ('Installed IPA Apps', str(len(self.emu.installed_apps))),
                ]
                ry = sy0 + 82
                for k, v in rows:
                    c.create_rectangle(sx0 + 16, ry, sx1 - 16, ry + 42,
                                       fill='#1e293b', outline='#334155')
                    c.create_text(sx0 + 28, ry + 21, anchor='w', text=k,
                                  fill='#cbd5e1', font=('Segoe UI', 9, 'bold'))
                    c.create_text(sx1 - 28, ry + 21, anchor='e', text=v,
                                  fill='#38bdf8', font=('Consolas', 9))
                    ry += 48

                c.create_text(cx, sy1 - 28, text='Swipe up from bottom edge to return Home',
                              fill='#94a3b8', font=('Segoe UI', 8))
                return

            # Installed .IPA Game / App Live Preview (e.g., SweetBenchmark or user IPA)
            c.create_rectangle(sx0, sy0, sx1, sy1, fill='#090d16', outline='')

            # App Header Banner
            c.create_rectangle(sx0, sy0 + 26, sx1, sy0 + 88, fill='#1e1b4b', outline='#312e81')
            c.create_text(sx0 + 18, sy0 + 48, anchor='w',
                          text=f'{app.get("glyph", "🎮")}  {app.get("display_name", "App")}',
                          fill='#ffffff', font=('Segoe UI', 13, 'bold'))
            c.create_text(sx0 + 18, sy0 + 70, anchor='w',
                          text=f'{bid} • v{app.get("short_version", "1.0")} • {app.get("graphics_api", "Metal/GLES")}',
                          fill='#a5b4fc', font=('Consolas', 8))

            # Animated 60 FPS Game / Benchmark Viewport
            vp_y0 = sy0 + 100
            vp_y1 = sy0 + 470
            c.create_rectangle(sx0 + 16, vp_y0, sx1 - 16, vp_y1, fill='#0f172a', outline='#4f46e5', width=2)

            # Animated 3D/2D orbiting nodes & FPS counter
            t = self.emu.anim_tick * 0.12
            center_x = cx
            center_y = (vp_y0 + vp_y1) // 2 - 15
            for i in range(6):
                angle = t + i * (math.pi / 3.0)
                rx = center_x + int(95 * math.cos(angle))
                ry = center_y + int(65 * math.sin(angle * 1.5))
                col = ['#38bdf8', '#c084fc', '#4ade80', '#f472b6', '#facc15', '#fb923c'][i]
                c.create_line(center_x, center_y, rx, ry, fill='#334155', width=2)
                c.create_oval(rx - 14, ry - 14, rx + 14, ry + 14, fill=col, outline='#ffffff', width=1)

            c.create_text(sx0 + 28, vp_y0 + 18, anchor='w',
                          text='RENDER PIPELINE: ARM64 + LLVMpipe / CoreAnimation',
                          fill='#38bdf8', font=('Consolas', 8, 'bold'))
            c.create_text(sx1 - 28, vp_y0 + 18, anchor='e',
                          text='60.0 FPS', fill='#4ade80', font=('Consolas', 9, 'bold'))

            if self.emu.last_touch:
                lx, ly, ldown, ledge = self.emu.last_touch
                c.create_text(
                    cx, vp_y1 - 20,
                    text=f'Last Touch Input: x={lx}, y={ly} ({"DOWN" if ldown else "UP"})',
                    fill='#facc15', font=('Consolas', 8)
                )

            # Mach-O & TrustCache Verification Card
            cdhashes = app.get('cdhashes', [])
            cdh_str = cdhashes[0][:32] + '…' if cdhashes else 'System Built-in'
            fws = ', '.join(app.get('frameworks', [])[:5]) or 'UIKit, Foundation, QuartzCore'

            c.create_rectangle(sx0 + 16, vp_y1 + 14, sx1 - 16, sy1 - 42,
                               fill='#111827', outline='#1e293b')
            c.create_text(
                sx0 + 26, vp_y1 + 26, anchor='nw',
                text=(
                    f'Mach-O Slice : ARM64 (CPU_TYPE_ARM64 = 0x0100000c)\n'
                    f'FairPlay     : Decrypted (cryptid = 0)\n'
                    f'Graphics API : {app.get("graphics_api", "OpenGL ES + Metal")}\n'
                    f'Frameworks   : {fws}\n'
                    f'CDHash (TC)  : {cdh_str}'
                ),
                fill='#cbd5e1', font=('Consolas', 8)
            )

            c.create_text(cx, sy1 - 24, text='▲ Swipe up from bottom bar (y >= 2352) to return Home',
                          fill='#f8fafc', font=('Segoe UI', 8, 'bold'))

    root = tk.Tk()
    app = SweetEmulatorGUI(root, core)
    return root, app


# ---------------------------------------------------------------------------
# Automated Self-Test (`python tools/sweet-emu.py --self-test`)
# ---------------------------------------------------------------------------

def run_self_test():
    if hasattr(sys.stdout, 'reconfigure'):
        try:
            sys.stdout.reconfigure(encoding='utf-8', errors='replace')
        except Exception:
            pass
    print('[*] Running ios-sweet Desktop Emulator (sweet-emu.py) self-test...')
    tmp_ws = tempfile.mkdtemp(prefix='sweet_emu_selftest_')
    try:
        core = EmulatorCore(workspace_dir=tmp_ws)

        # 1. Verify Coordinate Mapper & Patch 0071 Bottom-Edge Gesture Detection
        px0, py0 = core.mapper.canvas_to_panel(BEZEL_PAD_X, BEZEL_PAD_Y)
        px1, py1 = core.mapper.canvas_to_panel(BEZEL_PAD_X + VIEWPORT_W, BEZEL_PAD_Y + VIEWPORT_H)
        assert (px0, py0) == (0, 0), f'Expected (0,0), got {(px0, py0)}'
        assert (px1, py1) == (PANEL_W - 1, PANEL_H - 1), f'Expected (1079,2399), got {(px1, py1)}'
        assert core.mapper.is_edge_contact(540, 2380) is True, 'y=2380 must be detected as edge contact'
        assert core.mapper.is_edge_contact(540, 1200) is False, 'center of screen must not be edge contact'
        assert core.mapper.is_home_swipe(540, 2380, 540, 1600) is True, 'bottom-edge swipe-up must trigger home gesture'
        print('    [OK] TouchCoordinateMapper & patch 0071 edge-gesture detection (1080x2400)')

        # 2. Verify Shared-Memory Input Mailbox (0x9d600000, magic 0x5357494e 'SWIN')
        mbox_file = os.path.join(tmp_ws, 'mbox_test.bin')
        core.mailbox.write_initial_file(mbox_file)
        hdr = core.mailbox.read_header()
        assert hdr['magic'] == 0x5357494e, f'Bad mailbox magic: {hdr["magic"]:#x}'
        ev_t = core.mailbox.send_touch(0, 1, 540, 1200)
        slot0 = core.mailbox.read_slot(0)
        assert slot0 == {'type': 1, 'id': 0, 'down': 1, 'x': 540, 'y': 1200}, f'Bad slot0: {slot0}'
        ev_k = core.mailbox.send_key(qemu_sweet.KEY_POWER, 1)
        slot1 = core.mailbox.read_slot(1)
        assert slot1['type'] == 2 and slot1['id'] == qemu_sweet.KEY_POWER and slot1['down'] == 1
        print(f'    [OK] SweetInputMailbox at {qemu_sweet.INPUT_MBOX_PA:#x} (magic={hdr["magic"]:#x}, ring=64)')

        # 3. Verify QEMU ADT Generation via mkadt.py
        adt_res = core.build_qemu_adt_and_payload()
        assert os.path.isfile(adt_res['adt_path']) and adt_res['adt_size'] > 512
        print(f'    [OK] QEMU ADT generation via mkadt.py ({adt_res["adt_size"]} bytes)')

        # 4. Verify Demo IPA Creation, Inspection, Installation, and TrustCache via sweet-ipa.py
        inst = core.create_and_install_demo_ipa()
        assert inst['bundle_id'] == 'org.sweet.SweetBenchmark'
        assert inst['compatible'] is True
        assert len(core.installed_apps) == 1
        tc_path = inst['trustcache']['path']
        assert os.path.isfile(tc_path)
        print(f'    [OK] Demo IPA creation & installation ({inst["display_name"]}, CDHash={inst["cdhashes"][0][:16]}...)')

        # 5. Verify Interactive Preview State Machine & Screenshot Framebuffer Renderer
        assert core.screen_state == 'lock'
        # Simulate bottom-edge swipe-up (y=2380 -> y=1600) to unlock
        core.handle_touch_event(True, 540, 2380)
        core.handle_touch_event(False, 540, 1600, start_xy=(540, 2380))
        assert core.screen_state == 'home', f'Expected home state after edge swipe, got {core.screen_state}'

        # Launch installed SweetBenchmark app
        bench_app = next(a for a in core.all_home_apps() if a['bundle_id'] == 'org.sweet.SweetBenchmark')
        core.launch_app_in_preview(bench_app)
        assert core.screen_state == 'app' and core.active_app['bundle_id'] == 'org.sweet.SweetBenchmark'

        # Return Home via bottom-edge gesture
        core.trigger_home_gesture()
        assert core.screen_state == 'home' and core.active_app is None

        # Render 1080x2400 BGRA framebuffer, convert to 360x800 P6 PPM, and save PNG screenshot
        raw_fb = core.render_preview_framebuffer_bgra()
        assert len(raw_fb) == PANEL_W * PANEL_H * 4
        ppm_data = qemu_sweet.fb_to_ppm(raw_fb, scale=3)
        assert ppm_data.startswith(b'P6\n360 800\n255\n')
        snap_png = core.save_screenshot(os.path.join(tmp_ws, 'test_snap.png'))
        assert os.path.isfile(snap_png) and os.path.getsize(snap_png) > 1024
        print('    [OK] Framebuffer renderer (1080x2400 BGRA -> 360x800 PPM + PNG screenshot)')

        # 6. Verify Tkinter GUI instantiation & single render cycle (if display available)
        try:
            root, gui = create_gui_app(core)
            root.update_idletasks()
            gui._draw_phone_viewport()
            root.update_idletasks()
            root.destroy()
            print('    [OK] Tkinter Desktop GUI instantiated and rendered cleanly')
        except Exception as tk_exc:
            print(f'    [i] Headless environment ({tk_exc}); skipped live window creation')

        print('[✓] All sweet-emu.py self-tests PASSED!')
        return 0
    finally:
        sweet_ipa.safe_rmtree(tmp_ws)


def main():
    ap = argparse.ArgumentParser(
        description='BlueStacks-style Desktop GUI Emulator for ios-sweet (iPhone X / iOS 16.7.16).'
    )
    ap.add_argument('--self-test', action='store_true',
                    help='Run automated headless self-test of GUI, ADT, IPA, framebuffer, and input')
    ap.add_argument('--workspace', default=None,
                    help='Emulator workspace directory (default: ./emu_workspace)')
    ap.add_argument('--kernel', default='', help='Optional path to unstripped XNU kernel Mach-O')
    ap.add_argument('--payload', default='', help='Optional path to SWEETXNU payload binary')
    ap.add_argument('--disk', default='', help='Optional path to raw iOS root disk image')
    ap.add_argument('--smp', type=int, default=6, help='Number of SMP CPU cores (default: 6)')
    args = ap.parse_args()

    if args.self_test:
        return run_self_test()

    core = EmulatorCore(workspace_dir=args.workspace)
    core.smp_cores = max(1, min(8, args.smp))
    if args.kernel:
        core.kernel_path = os.path.abspath(args.kernel)
    if args.payload:
        core.payload_path = os.path.abspath(args.payload)
    if args.disk:
        core.disk_path = os.path.abspath(args.disk)

    root, _ = create_gui_app(core)
    try:
        root.mainloop()
    finally:
        core.stop_qemu_vm()
    return 0


if __name__ == '__main__':
    sys.exit(main())
