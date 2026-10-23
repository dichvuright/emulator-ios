pub mod apps;
pub mod cuda;
pub mod font;
pub mod png;
pub mod raster;
pub mod springboard;

pub use cuda::{CudaAccelerator, DisplayFilterConfig};

use std::path::Path;
use std::time::Instant;
use sweet_core::{AppMetadata, GPU_RING_MAGIC};

use crate::raster::{
    blend_argb, blit_procedural_texture, ensure_alpha, fill_rounded_rect, fill_triangle,
};

pub const GPU_CMD_NOP: u32 = 0;
pub const GPU_CMD_CLEAR: u32 = 1;
pub const GPU_CMD_RECT: u32 = 2;
pub const GPU_CMD_TRIANGLE: u32 = 3;
pub const GPU_CMD_BLIT_TEXTURE: u32 = 4;
pub const GPU_CMD_PRESENT: u32 = 5;

pub const GPU_RING_HEADER_SIZE: usize = 16;
pub const GPU_RING_ENTRY_SIZE: usize = 24;
pub const GPU_RING_MAX_ENTRIES: usize = 256;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuCommand {
    pub opcode: u32,
    pub arg0: u32,
    pub arg1: u32,
    pub arg2: u32,
    pub arg3: u32,
    pub color: u32,
}

impl GpuCommand {
    /// Packs signed 16-bit `(x, y)` coordinates into a single `u32` (`(x << 16) | (y & 0xFFFF)`).
    #[inline]
    pub const fn pack_xy(x: i32, y: i32) -> u32 {
        (((x as i16 as u16) as u32) << 16) | ((y as i16 as u16) as u32)
    }

    /// Unpacks a `u32` coordinate pair packed with [`Self::pack_xy`].
    #[inline]
    pub const fn unpack_xy(packed: u32) -> (i32, i32) {
        let x = ((packed >> 16) as u16 as i16) as i32;
        let y = ((packed & 0xFFFF) as u16 as i16) as i32;
        (x, y)
    }

    #[inline]
    pub const fn clear(color: u32) -> Self {
        Self {
            opcode: GPU_CMD_CLEAR,
            arg0: 0,
            arg1: 0,
            arg2: 0,
            arg3: 0,
            color,
        }
    }

    #[inline]
    pub const fn rect(x: u32, y: u32, w: u32, h: u32, color: u32) -> Self {
        Self {
            opcode: GPU_CMD_RECT,
            arg0: x,
            arg1: y,
            arg2: w,
            arg3: h,
            color,
        }
    }

    #[inline]
    pub const fn rounded_rect(x: u32, y: u32, w: u32, h: u32, radius: u32, color: u32) -> Self {
        Self {
            opcode: GPU_CMD_RECT,
            arg0: x,
            arg1: y,
            arg2: ((radius & 0xFFFF) << 16) | (w & 0xFFFF),
            arg3: h,
            color,
        }
    }

    #[inline]
    pub const fn triangle(
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        color: u32,
    ) -> Self {
        Self {
            opcode: GPU_CMD_TRIANGLE,
            arg0: Self::pack_xy(x0, y0),
            arg1: Self::pack_xy(x1, y1),
            arg2: Self::pack_xy(x2, y2),
            arg3: 0,
            color,
        }
    }

    #[inline]
    pub const fn blit_texture(x: u32, y: u32, w: u32, h: u32, color: u32) -> Self {
        Self {
            opcode: GPU_CMD_BLIT_TEXTURE,
            arg0: x,
            arg1: y,
            arg2: w,
            arg3: h,
            color,
        }
    }

    #[inline]
    pub const fn present() -> Self {
        Self {
            opcode: GPU_CMD_PRESENT,
            arg0: 0,
            arg1: 0,
            arg2: 0,
            arg3: 0,
            color: 0,
        }
    }

    /// Serializes this command into its 24-byte little-endian ring buffer representation.
    pub fn to_le_bytes(&self) -> [u8; 24] {
        let mut out = [0u8; 24];
        out[0..4].copy_from_slice(&self.opcode.to_le_bytes());
        out[4..8].copy_from_slice(&self.arg0.to_le_bytes());
        out[8..12].copy_from_slice(&self.arg1.to_le_bytes());
        out[12..16].copy_from_slice(&self.arg2.to_le_bytes());
        out[16..20].copy_from_slice(&self.arg3.to_le_bytes());
        out[20..24].copy_from_slice(&self.color.to_le_bytes());
        out
    }

    /// Parses a 24-byte slice from the shared-memory GPU ring buffer.
    pub fn from_le_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 24 {
            return None;
        }
        Some(Self {
            opcode: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            arg0: u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            arg1: u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
            arg2: u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]),
            arg3: u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]),
            color: u32::from_le_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]),
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct GpuStats {
    pub frames_rendered: u64,
    pub draw_calls: u64,
    pub triangles: u64,
    pub last_frame_us: u64,
    pub backend_name: String,
}

#[derive(Clone, Debug, Default)]
pub struct IosSimState {
    pub locked: bool,
    pub active_app: Option<String>,
    pub installed_apps: Vec<AppMetadata>,
    pub frame_tick: u64,
    pub touch_x: i32,
    pub touch_y: i32,
    pub touch_down: bool,
    pub edge_swipe_active: bool,

    pub initialized: bool,
    pub settings_page: String,
    pub scroll_y: i32,
    pub airplane_mode: bool,
    pub wifi_enabled: bool,
    pub wifi_network: String,
    pub bluetooth_enabled: bool,
    pub cellular_enabled: bool,
    pub dark_mode: bool,
    pub low_power_mode: bool,
    pub night_shift: bool,
    pub true_tone: bool,
    pub flashlight_on: bool,
    pub orientation_lock: bool,
    pub focus_dnd: bool,
    pub brightness: u8,
    pub volume: u8,

    // Phone App
    pub phone_tab: u8,
    pub phone_dial_buf: String,
    pub phone_in_call: bool,
    pub phone_call_target: String,
    pub phone_call_start_tick: u64,
    pub phone_muted: bool,
    pub phone_speaker: bool,
    pub phone_recents: Vec<(String, String)>,

    // Calculator App
    pub calc_display: String,
    pub calc_acc: f64,
    pub calc_pending_op: Option<char>,
    pub calc_replace_next: bool,

    // Safari App
    pub safari_url: String,
    pub safari_editing_url: bool,
    pub safari_input_buf: String,
    pub safari_history: Vec<String>,

    // Messages (iMessage) App
    pub messages_active_chat: Option<usize>,
    pub messages_input_buf: String,
    pub messages_threads: Vec<(String, String, Vec<(bool, String)>)>,

    // Camera & Photos App
    pub camera_mode: u8,
    pub camera_front: bool,
    pub camera_flash_tick: u64,
    pub photos_count: usize,
    pub photos_selected: Option<usize>,
    pub photos_tab: u8,
    pub photos_favorites: u32,

    // Clock App
    pub clock_tab: u8,
    pub alarms_enabled: u8,
    pub stopwatch_running: bool,
    pub stopwatch_elapsed_ticks: u64,
    pub stopwatch_laps: Vec<u64>,
    pub timer_running: bool,
    pub timer_remaining_ticks: u64,

    // Notes & Reminders App
    pub notes_selected: Option<usize>,
    pub notes_items: Vec<(String, String)>,
    pub reminders_done: u16,
    pub reminders_items: Vec<String>,

    // Music, Weather, Files, App Store, Home, Calendar, Mail, Spotlight
    pub music_playing: bool,
    pub music_track_idx: usize,
    pub music_progress_ticks: u64,
    pub weather_city_idx: usize,
    pub files_current_dir: String,
    pub app_store_tab: u8,
    pub home_devices_on: u8,
    pub calendar_selected_day: u8,
    pub mail_selected_idx: Option<usize>,
    pub spotlight_open: bool,
    pub spotlight_query: String,

    // CoreAnimation Spring Physics & Transition State (60-120 FPS)
    pub lock_anim: f32,
    pub lock_vel: f32,
    pub home_flyin: f32,
    pub app_anim: f32,
    pub app_vel: f32,
    pub anim_app_name: Option<String>,
    pub anim_icon_rect: (f32, f32, f32, f32),
    pub drag_start_vy: i32,
    pub nav_anim: f32,
    pub nav_dir: i8,
    pub prev_settings_page: String,
    pub last_seen_settings_page: String,
    pub scroll_vel: f32,
    pub prev_locked: bool,
}

pub struct SweetGpuEngine {
    pub stats: GpuStats,
}

impl Default for SweetGpuEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SweetGpuEngine {
    pub fn new() -> Self {
        Self {
            stats: GpuStats {
                backend_name: "Rust SIMD + Win32 GDI/OpenGL Forwarding Ring".to_string(),
                ..Default::default()
            },
        }
    }

    pub fn stats(&self) -> &GpuStats {
        &self.stats
    }

    /// Submits a single GPU command to the host engine and updates telemetry counters.
    pub fn submit_command(&mut self, cmd: GpuCommand) {
        match cmd.opcode {
            GPU_CMD_NOP => {}
            GPU_CMD_CLEAR | GPU_CMD_RECT | GPU_CMD_BLIT_TEXTURE => {
                self.stats.draw_calls += 1;
            }
            GPU_CMD_TRIANGLE => {
                self.stats.draw_calls += 1;
                self.stats.triangles += 1;
            }
            GPU_CMD_PRESENT => {
                self.stats.frames_rendered += 1;
            }
            _ => {}
        }
    }

    /// Executes a `GpuCommand` directly onto a `&mut [u32]` framebuffer and updates stats.
    pub fn execute_command_on_u32(
        &mut self,
        cmd: GpuCommand,
        fb: &mut [u32],
        fb_w: usize,
        fb_h: usize,
    ) {
        self.submit_command(cmd);
        if fb_w == 0 || fb_h == 0 || fb.len() < fb_w * fb_h {
            return;
        }
        let buf = &mut fb[..fb_w * fb_h];
        let color = ensure_alpha(cmd.color);

        match cmd.opcode {
            GPU_CMD_NOP | GPU_CMD_PRESENT => {}
            GPU_CMD_CLEAR => {
                buf.fill(color);
            }
            GPU_CMD_RECT => {
                let x = cmd.arg0 as i32;
                let y = cmd.arg1 as i32;
                let w = (cmd.arg2 & 0xFFFF) as i32;
                let radius = ((cmd.arg2 >> 16) & 0xFFFF) as i32;
                let h = cmd.arg3 as i32;
                fill_rounded_rect(buf, fb_w, fb_h, x, y, w, h, radius, color);
            }
            GPU_CMD_TRIANGLE => {
                let (x0, y0, x1, y1, x2, y2) = decode_triangle_vertices(cmd);
                fill_triangle(buf, fb_w, fb_h, x0, y0, x1, y1, x2, y2, color);
            }
            GPU_CMD_BLIT_TEXTURE => {
                let x = cmd.arg0 as i32;
                let y = cmd.arg1 as i32;
                let w = cmd.arg2 as i32;
                let h = cmd.arg3 as i32;
                blit_procedural_texture(buf, fb_w, fb_h, x, y, w, h, color);
            }
            _ => {}
        }
    }

    /// Processes pending GPU commands from the guest shared-memory ring buffer (`GPU_RING_PA = 0x9d60_4000`)
    /// and rasterizes them into `fb_bgra` (`&mut [u8]`).
    ///
    /// Ring format:
    /// - Header (`16 bytes`): `u32 magic (0x53475055 'SGPU')`, `u32 write_idx`, `u32 read_idx`, `u32 flags`
    /// - Entries (`24 bytes` each, up to 256 entries): `GpuCommand { opcode, arg0, arg1, arg2, arg3, color }`
    pub fn process_ring_buffer(&mut self, ring_bytes: &[u8], fb_bgra: &mut [u8]) -> usize {
        if ring_bytes.len() < GPU_RING_HEADER_SIZE {
            return 0;
        }
        let magic = u32::from_le_bytes([ring_bytes[0], ring_bytes[1], ring_bytes[2], ring_bytes[3]]);
        if magic != GPU_RING_MAGIC {
            return 0;
        }

        let write_idx =
            u32::from_le_bytes([ring_bytes[4], ring_bytes[5], ring_bytes[6], ring_bytes[7]]);
        let read_idx =
            u32::from_le_bytes([ring_bytes[8], ring_bytes[9], ring_bytes[10], ring_bytes[11]]);

        let entry_cap = ((ring_bytes.len() - GPU_RING_HEADER_SIZE) / GPU_RING_ENTRY_SIZE)
            .min(GPU_RING_MAX_ENTRIES);
        if entry_cap == 0 || write_idx == read_idx {
            return 0;
        }

        let t0 = Instant::now();
        let (fb_w, fb_h) = infer_fb_dimensions(fb_bgra.len() / 4);
        let max_to_process = (write_idx.wrapping_sub(read_idx) as usize).min(entry_cap);

        let mut processed = 0usize;
        for step in 0..max_to_process {
            let slot = (read_idx.wrapping_add(step as u32) as usize) % entry_cap;
            let off = GPU_RING_HEADER_SIZE + slot * GPU_RING_ENTRY_SIZE;
            if let Some(cmd) = GpuCommand::from_le_bytes(&ring_bytes[off..off + GPU_RING_ENTRY_SIZE])
            {
                self.submit_command(cmd);
                if fb_w > 0 && fb_h > 0 {
                    rasterize_command_on_u8(cmd, fb_bgra, fb_w, fb_h);
                }
                processed += 1;
            }
        }

        self.stats.last_frame_us = t0.elapsed().as_micros().max(1) as u64;
        processed
    }

    /// Downsamples the guest's `1080x2400` 32-bit BGRA framebuffer (`&[u8]`)
    /// into any viewport buffer (`&mut [u32]`, e.g., `360x800` or `405x900`).
    pub fn downsample_1080x2400_to_viewport(
        src_bgra: &[u8],
        dst_bgra: &mut [u32],
        dst_w: usize,
        dst_h: usize,
    ) {
        raster::downsample_1080x2400_to_viewport(src_bgra, dst_bgra, dst_w, dst_h);
    }

    /// Renders a complete interactive iOS 16.7.16 (20H392) SpringBoard frame (Lock Screen,
    /// Home Screen with squircle app icons, Settings -> General -> About, or Real-Time 3D
    /// rotating cube & starfield game benchmark) into `dst_bgra`.
    pub fn render_ios_springboard_frame(
        &mut self,
        fb_w: usize,
        fb_h: usize,
        dst_bgra: &mut [u32],
        screen_state: &IosSimState,
    ) {
        let t0 = Instant::now();
        springboard::render_springboard(self, fb_w, fb_h, dst_bgra, screen_state);
        self.stats.frames_rendered += 1;
        self.stats.last_frame_us = t0.elapsed().as_micros().max(1) as u64;
    }

    /// Encodes a 32-bit BGRA buffer to a valid PNG file on disk with zero external dependencies.
    pub fn save_bgra_png(
        path: &Path,
        width: u32,
        height: u32,
        bgra: &[u8],
    ) -> Result<(), String> {
        png::save_bgra_png(path, width, height, bgra)
    }

    /// Helper to build a serialized shared-memory GPU command ring buffer from a slice of commands.
    pub fn encode_ring_buffer(commands: &[GpuCommand], read_idx: u32, flags: u32) -> Vec<u8> {
        let cap = commands.len().clamp(1, GPU_RING_MAX_ENTRIES);
        let mut buf = vec![0u8; GPU_RING_HEADER_SIZE + cap * GPU_RING_ENTRY_SIZE];
        let write_idx = read_idx.wrapping_add(commands.len().min(cap) as u32);
        buf[0..4].copy_from_slice(&GPU_RING_MAGIC.to_le_bytes());
        buf[4..8].copy_from_slice(&write_idx.to_le_bytes());
        buf[8..12].copy_from_slice(&read_idx.to_le_bytes());
        buf[12..16].copy_from_slice(&flags.to_le_bytes());

        for (i, cmd) in commands.iter().take(cap).enumerate() {
            let slot = ((read_idx as usize) + i) % cap;
            let off = GPU_RING_HEADER_SIZE + slot * GPU_RING_ENTRY_SIZE;
            buf[off..off + GPU_RING_ENTRY_SIZE].copy_from_slice(&cmd.to_le_bytes());
        }
        buf
    }
}

/// Decodes triangle coordinates from `GpuCommand`.
/// Supports both packed 16-bit vertex pairs `(arg0, arg1, arg2)` and `(x, y, w, h)` bounding-box args.
fn decode_triangle_vertices(cmd: GpuCommand) -> (i32, i32, i32, i32, i32, i32) {
    if (cmd.arg0 >> 16) == 0 && (cmd.arg1 >> 16) == 0 && (cmd.arg2 >> 16) == 0 && cmd.arg3 > 0 {
        let x = cmd.arg0 as i32;
        let y = cmd.arg1 as i32;
        let w = cmd.arg2 as i32;
        let h = cmd.arg3 as i32;
        (x + w / 2, y, x + w, y + h, x, y + h)
    } else {
        let (x0, y0) = GpuCommand::unpack_xy(cmd.arg0);
        let (x1, y1) = GpuCommand::unpack_xy(cmd.arg1);
        let (x2, y2) = GpuCommand::unpack_xy(cmd.arg2);
        (x0, y0, x1, y1, x2, y2)
    }
}

/// Infers `(width, height)` from a pixel count for `process_ring_buffer`.
fn infer_fb_dimensions(pixels: usize) -> (usize, usize) {
    if pixels == 0 {
        return (0, 0);
    }
    match pixels {
        2_592_000 => return (1080, 2400),
        648_000 => return (540, 1200),
        364_500 => return (405, 900),
        288_000 => return (360, 800),
        162_000 => return (270, 600),
        72_000 => return (180, 400),
        18_000 => return (90, 200),
        _ => {}
    }
    let side = (pixels as f64).sqrt().round() as usize;
    if side > 0 && side * side == pixels {
        return (side, side);
    }
    for &w in &[1080usize, 720, 640, 540, 405, 360, 320, 256, 128, 100, 64, 32, 16, 8, 4] {
        if pixels >= w && pixels % w == 0 {
            return (w, pixels / w);
        }
    }
    let s = side.max(1);
    (s, (pixels / s).max(1))
}

/// Rasterizes a single `GpuCommand` directly into a `&mut [u8]` BGRA framebuffer.
fn rasterize_command_on_u8(cmd: GpuCommand, fb_bgra: &mut [u8], fb_w: usize, fb_h: usize) {
    let total_bytes = fb_w * fb_h * 4;
    if fb_bgra.len() < total_bytes {
        return;
    }
    let color = ensure_alpha(cmd.color);

    match cmd.opcode {
        GPU_CMD_NOP | GPU_CMD_PRESENT => {}
        GPU_CMD_CLEAR => {
            let le = color.to_le_bytes();
            for px in fb_bgra[..total_bytes].chunks_exact_mut(4) {
                px.copy_from_slice(&le);
            }
        }
        GPU_CMD_RECT => {
            let x = cmd.arg0 as i32;
            let y = cmd.arg1 as i32;
            let w = (cmd.arg2 & 0xFFFF) as i32;
            let radius = ((cmd.arg2 >> 16) & 0xFFFF) as i32;
            let h = cmd.arg3 as i32;
            rasterize_tile_u8(fb_bgra, fb_w, fb_h, x, y, w, h, |tile, tw, th, ox, oy| {
                fill_rounded_rect(tile, tw, th, x - ox, y - oy, w, h, radius, color);
            });
        }
        GPU_CMD_TRIANGLE => {
            let (x0, y0, x1, y1, x2, y2) = decode_triangle_vertices(cmd);
            let min_x = x0.min(x1).min(x2);
            let max_x = x0.max(x1).max(x2);
            let min_y = y0.min(y1).min(y2);
            let max_y = y0.max(y1).max(y2);
            let w = (max_x - min_x + 1).max(1);
            let h = (max_y - min_y + 1).max(1);
            rasterize_tile_u8(
                fb_bgra,
                fb_w,
                fb_h,
                min_x,
                min_y,
                w,
                h,
                |tile, tw, th, ox, oy| {
                    fill_triangle(
                        tile,
                        tw,
                        th,
                        x0 - ox,
                        y0 - oy,
                        x1 - ox,
                        y1 - oy,
                        x2 - ox,
                        y2 - oy,
                        color,
                    );
                },
            );
        }
        GPU_CMD_BLIT_TEXTURE => {
            let x = cmd.arg0 as i32;
            let y = cmd.arg1 as i32;
            let w = cmd.arg2 as i32;
            let h = cmd.arg3 as i32;
            rasterize_tile_u8(fb_bgra, fb_w, fb_h, x, y, w, h, |tile, tw, th, ox, oy| {
                blit_procedural_texture(tile, tw, th, x - ox, y - oy, w, h, color);
            });
        }
        _ => {
            // Ignore unknown opcodes gracefully
            let _ = blend_argb(0, color);
        }
    }
}

/// Clips a bounding box `(x, y, w, h)` to `(fb_w, fb_h)`, copies that sub-rectangle from `fb_bgra`
/// into a temporary `u32` tile, runs the rasterizer closure `f`, and writes the modified tile back.
fn rasterize_tile_u8<F>(
    fb_bgra: &mut [u8],
    fb_w: usize,
    fb_h: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    f: F,
) where
    F: FnOnce(&mut [u32], usize, usize, i32, i32),
{
    if w <= 0 || h <= 0 {
        return;
    }
    let x0 = x.max(0) as usize;
    let y0 = y.max(0) as usize;
    let x1 = (x + w).min(fb_w as i32).max(0) as usize;
    let y1 = (y + h).min(fb_h as i32).max(0) as usize;
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let tw = x1 - x0;
    let th = y1 - y0;
    let mut tile = vec![0u32; tw * th];

    for ty in 0..th {
        let src_row_off = ((y0 + ty) * fb_w + x0) * 4;
        let dst_row_off = ty * tw;
        for tx in 0..tw {
            let off = src_row_off + tx * 4;
            tile[dst_row_off + tx] = u32::from_le_bytes([
                fb_bgra[off],
                fb_bgra[off + 1],
                fb_bgra[off + 2],
                fb_bgra[off + 3],
            ]);
        }
    }

    f(&mut tile, tw, th, x0 as i32, y0 as i32);

    for ty in 0..th {
        let dst_row_off = ((y0 + ty) * fb_w + x0) * 4;
        let src_row_off = ty * tw;
        for tx in 0..tw {
            let off = dst_row_off + tx * 4;
            fb_bgra[off..off + 4].copy_from_slice(&tile[src_row_off + tx].to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gpu_ring_buffer_processing() {
        let mut engine = SweetGpuEngine::new();
        let mut fb = vec![0u8; 64 * 64 * 4];

        let cmds = [
            GpuCommand::clear(0xFF10_2030),
            GpuCommand::rounded_rect(8, 8, 32, 24, 6, 0xFF00_AAFF),
            GpuCommand::triangle(10, 10, 50, 15, 25, 55, 0xFFFF_3366),
            GpuCommand::blit_texture(32, 32, 16, 16, 0xFF44_FF88),
            GpuCommand::present(),
        ];
        let ring = SweetGpuEngine::encode_ring_buffer(&cmds, 0, 0);
        let processed = engine.process_ring_buffer(&ring, &mut fb);

        assert_eq!(processed, 5);
        assert_eq!(engine.stats().frames_rendered, 1);
        assert_eq!(engine.stats().draw_calls, 4);
        assert_eq!(engine.stats().triangles, 1);
        assert!(engine.stats().last_frame_us >= 1);

        // Verify top-left pixel was cleared to 0xFF10_2030 (BGRA = [0x30, 0x20, 0x10, 0xFF])
        assert_eq!(&fb[0..4], &[0x30, 0x20, 0x10, 0xFF]);
        // Verify triangle interior pixel (25, 25) was painted to 0xFFFF_3366
        let tri_off = (25 * 64 + 25) * 4;
        assert_eq!(&fb[tri_off..tri_off + 4], &[0x66, 0x33, 0xFF, 0xFF]);
    }

    #[test]
    fn test_downsample_1080x2400_to_360x800() {
        let src_len = 1080 * 2400 * 4;
        let mut src = vec![0u8; src_len];
        for px in src.chunks_exact_mut(4) {
            px[0] = 0x40; // B
            px[1] = 0x80; // G
            px[2] = 0xC0; // R
            px[3] = 0xFF; // A
        }
        let mut dst = vec![0u32; 360 * 800];
        SweetGpuEngine::downsample_1080x2400_to_viewport(&src, &mut dst, 360, 800);
        assert_eq!(dst[0], 0xFFC0_8040);
        assert_eq!(dst[360 * 800 - 1], 0xFFC0_8040);
    }

    #[test]
    fn test_springboard_lock_home_settings_and_3d_bench() {
        let mut engine = SweetGpuEngine::new();
        let (w, h) = (360usize, 800usize);
        let mut fb = vec![0u32; w * h];

        // 1. Lock screen
        let mut state = IosSimState {
            locked: true,
            active_app: None,
            installed_apps: Vec::new(),
            frame_tick: 10,
            touch_x: 540,
            touch_y: 2370,
            touch_down: true,
            edge_swipe_active: true,
            ..Default::default()
        };
        state.ensure_initialized();
        engine.render_ios_springboard_frame(w, h, &mut fb, &state);
        assert_eq!(engine.stats().frames_rendered, 1);
        assert!(fb.iter().any(|&px| px != 0));

        // 2. Home screen with installed app
        state.locked = false;
        state.installed_apps.push(AppMetadata {
            bundle_id: "com.sweet.demo3d".to_string(),
            display_name: "Demo 3D IPA".to_string(),
            version: "1.0".to_string(),
            executable: "Demo3D".to_string(),
            graphics_api: "Metal".to_string(),
            is_arm64: true,
            is_encrypted: false,
            cdhash_hex: "8f4e2a91c0d37b15aa11223344556677889900aa".to_string(),
            bundle_uuid: "00000000-1111-2222-3333-444444444444".to_string(),
            data_uuid: "00000000-1111-2222-3333-555555555555".to_string(),
        });
        engine.render_ios_springboard_frame(w, h, &mut fb, &state);
        assert_eq!(engine.stats().frames_rendered, 2);

        // 3. Settings -> General -> About
        state.active_app = Some("Settings".to_string());
        engine.render_ios_springboard_frame(w, h, &mut fb, &state);
        assert_eq!(engine.stats().frames_rendered, 3);

        // 4. 3D Game Benchmark (rasterizes cube triangles via GPU_CMD_TRIANGLE)
        let tris_before = engine.stats().triangles;
        state.active_app = Some("Sweet 3D Bench".to_string());
        state.frame_tick = 25;
        engine.render_ios_springboard_frame(w, h, &mut fb, &state);
        assert_eq!(engine.stats().frames_rendered, 4);
        assert!(engine.stats().triangles >= tris_before + 12);

        // 5. Stock iOS 16 apps: Safari, Phone, Control Center, Calculator
        for app in ["Safari", "Phone", "Control Center", "Calculator"] {
            state.active_app = Some(app.to_string());
            engine.render_ios_springboard_frame(w, h, &mut fb, &state);
        }
        assert_eq!(engine.stats().frames_rendered, 8);
    }

    #[test]
    fn test_zero_dep_png_encoder() {
        let (w, h) = (32u32, 24u32);
        let mut bgra = vec![0u8; (w * h * 4) as usize];
        for (i, px) in bgra.chunks_exact_mut(4).enumerate() {
            px[0] = (i & 0xFF) as u8;
            px[1] = ((i >> 2) & 0xFF) as u8;
            px[2] = 0x90;
            px[3] = 0xFF;
        }
        let temp_path = std::env::temp_dir().join("sweet_gpu_test_out.png");
        SweetGpuEngine::save_bgra_png(&temp_path, w, h, &bgra).expect("PNG save should succeed");
        let bytes = std::fs::read(&temp_path).expect("PNG file should exist");
        let _ = std::fs::remove_file(&temp_path);

        assert_eq!(&bytes[0..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        assert!(bytes.windows(4).any(|w| w == b"IHDR"));
        assert!(bytes.windows(4).any(|w| w == b"IDAT"));
        assert!(bytes.windows(4).any(|w| w == b"IEND"));
    }

    #[test]
    fn test_cuda_accelerator_and_filters() {
        let mut cuda = CudaAccelerator::new();
        println!(
            "CUDA status: available={}, pinned_dma={}, device={}, vram={} MB",
            cuda.available, cuda.pinned_dma, cuda.device_name, cuda.vram_mb
        );
        let guest_bgra = vec![0x80u8; 1080 * 2400 * 4];
        let mut win_px = vec![0u32; 400 * 800];
        let mut filter = DisplayFilterConfig::default();
        filter.liquid_glass = true;
        filter.increase_contrast = true;
        cuda.blit_guest_fb_filtered(&mut win_px, 400, 800, 20, 20, 324, 720, &guest_bgra, &filter);
        assert_ne!(win_px[100 * 400 + 100], 0);
    }

    #[test]
    fn test_cuda_motion_and_pinned_dma() {
        let mut cuda = CudaAccelerator::new();
        let w = 1080usize;
        let h = 2400usize;
        let mut frame0 = vec![0x1Cu8; w * h * 4];
        let mut frame1 = vec![0x1Cu8; w * h * 4];

        // Draw high-contrast Settings-like rows at y0 = 600..624 in frame0
        // and scrolled up by 48 px to y1 = 552..576 in frame1 (mv_y = -48).
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                frame0[i + 3] = 0xFF;
                frame1[i + 3] = 0xFF;
                if (600..624).contains(&y) && (120..960).contains(&x) {
                    let v = if ((x / 16) & 1) == 0 { 0xF0 } else { 0x38 };
                    frame0[i] = v;
                    frame0[i + 1] = v;
                    frame0[i + 2] = v;
                }
                if (552..576).contains(&y) && (120..960).contains(&x) {
                    let v = if ((x / 16) & 1) == 0 { 0xF0 } else { 0x38 };
                    frame1[i] = v;
                    frame1[i + 1] = v;
                    frame1[i + 2] = v;
                }
            }
        }

        let mut win_px = vec![0u32; 720 * 1600];
        let filter = DisplayFilterConfig::default();

        // Upload frame 0 (initializes d_curr)
        cuda.mark_frame_dirty();
        cuda.set_temporal_alpha(256);
        cuda.blit_guest_fb_filtered(&mut win_px, 720, 1600, 0, 0, 720, 1600, &frame0, &filter);

        // Upload frame 1 (swaps d_prev <-> d_curr, runs sweet_motion_kernel + sweet_fb_kernel at alpha=128)
        cuda.mark_frame_dirty();
        cuda.set_temporal_alpha(128);
        cuda.blit_guest_fb_filtered(&mut win_px, 720, 1600, 0, 0, 720, 1600, &frame1, &filter);
        let upload_and_motion_us = cuda.last_exec_us;
        // At alpha=128 (halfway between y0=600..624 and y1=552..576 -> sy=588 -> oy=392, ox=152 -> sx=228):
        // Pure cross-fade without optical flow would yield background 0x1C; optical flow yields ~0xF0!
        let mid_flow_luma = win_px[392 * 720 + 152] & 0xFF;

        // Synthesize intermediate 120Hz frames straight from VRAM (zero 10.36 MB PCIe HtoD upload!)
        for alpha in [64u32, 96, 128, 160, 192, 224, 256] {
            cuda.set_temporal_alpha(alpha);
            cuda.blit_guest_fb_filtered(&mut win_px, 720, 1600, 0, 0, 720, 1600, &frame1, &filter);
        }
        let vram_interp_us = cuda.last_interp_us;

        println!(
            "Hybrid CUDA 120Hz bench: avail={}, pinned={}, motion_launches={}, kernel_launches={}, upload+flow={} us, vram_120hz_step={} us, mid_flow_luma=0x{:02x}",
            cuda.available,
            cuda.pinned_dma,
            cuda.motion_launches,
            cuda.kernel_launches,
            upload_and_motion_us,
            vram_interp_us,
            mid_flow_luma
        );

        if cuda.available {
            assert!(cuda.pinned_dma, "RTX 5050 should enable pinned PCIe DMA");
            assert!(cuda.motion_launches >= 1, "sweet_motion_kernel must execute");
            assert!(cuda.kernel_launches >= 8, "sweet_fb_kernel must execute for each alpha step");
            assert!(
                mid_flow_luma > 0xC0,
                "Optical flow at alpha=128 must warp scrolled row to halfway oy=392 (got 0x{:02x})",
                mid_flow_luma
            );
        }
        // At alpha=256, scrolled bar should sit cleanly around oy = 564 * 1600 / 2400 = 376, ox = 152
        let settled_luma = win_px[376 * 720 + 152] & 0xFF;
        assert!(settled_luma > 0xC0, "Settled frame at alpha=256 must match frame1 row (got 0x{:02x})", settled_luma);
    }

    #[test]
    fn test_no_red_stripe_corruption_at_low_alpha() {
        let mut cuda = CudaAccelerator::new();
        let w = 1080usize;
        let h = 2400usize;
        let mut filter = DisplayFilterConfig::default();
        filter.dark_mode = true; // pack_gpu_flags() == 0 so raw RGB comes straight out of sweet_fb_kernel
        let mut win_px = vec![0u32; 720 * 1600];

        // Case A: Non-rigid UI transition / tap (valid == 0) at alpha = 40, 64, 96, 120 (< 128)
        // Previously, SAMPLE_CURR_ONLY left %p4 = false and %r60..%r62 uninitialized, overwriting
        // the entire screen with 4.2 vertical red/teal register-file stripes whenever alpha < 128!
        let mut cut0 = vec![0x18u8; w * h * 4];
        let mut cut1 = vec![0x24u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                cut0[i + 3] = 0xFF;
                cut1[i + 3] = 0xFF;
                // Opposite-moving modal blocks so no global rigid (dx, dy) exists -> valid == 0
                if (500..900).contains(&y) && (200..880).contains(&x) {
                    cut0[i] = 0xE0;
                    cut0[i + 1] = 0xE0;
                    cut0[i + 2] = 0xE0;
                }
                if (1200..1700).contains(&y) && (140..940).contains(&x) && ((x + y) % 19 < 7) {
                    cut1[i] = 0xD8;
                    cut1[i + 1] = 0xD8;
                    cut1[i + 2] = 0xD8;
                }
            }
        }

        cuda.mark_frame_dirty();
        cuda.set_temporal_alpha(40); // Even first frame with alpha=40 must force run_alpha=256 cleanly!
        cuda.blit_guest_fb_filtered(&mut win_px, 720, 1600, 0, 0, 720, 1600, &cut0, &filter);
        for oy in 0..300usize {
            for ox in 0..720usize {
                let px = win_px[oy * 720 + ox];
                let r = ((px >> 16) & 0xFF) as i32;
                let g = ((px >> 8) & 0xFF) as i32;
                let b = (px & 0xFF) as i32;
                assert!(
                    (r - 0x18).abs() <= 1 && (g - 0x18).abs() <= 1 && (b - 0x18).abs() <= 1,
                    "First frame corruption at ({}, {}): 0x{:08x}",
                    ox,
                    oy,
                    px
                );
            }
        }

        cuda.mark_frame_dirty();
        cuda.set_temporal_alpha(40);
        cuda.blit_guest_fb_filtered(&mut win_px, 720, 1600, 0, 0, 720, 1600, &cut1, &filter);
        if cuda.available {
            assert!(!cuda.last_motion_valid, "Non-rigid cut must report last_motion_valid == false");
        }
        for alpha in [40u32, 64, 96, 120] {
            cuda.set_temporal_alpha(alpha);
            cuda.blit_guest_fb_filtered(&mut win_px, 720, 1600, 0, 0, 720, 1600, &cut1, &filter);
            for oy in 0..750usize {
                for ox in 0..720usize {
                    let px = win_px[oy * 720 + ox];
                    let r = ((px >> 16) & 0xFF) as i32;
                    let g = ((px >> 8) & 0xFF) as i32;
                    let b = (px & 0xFF) as i32;
                    assert!(
                        (r - 0x24).abs() <= 1 && (g - 0x24).abs() <= 1 && (b - 0x24).abs() <= 1,
                        "Red-stripe corruption on non-rigid cut at alpha={}, ({}, {}): 0x{:08x}",
                        alpha,
                        ox,
                        oy,
                        px
                    );
                }
            }
        }

        // Case B: Coherent vertical scroll (valid == 1, mv_y = -48) at alpha = 40, 64, 96 (< 128)
        // Verify Status Bar (sy < 128), Nav Bar (sy < 268), Nav seam, and Home Indicator (sy >= 2300)
        // have ZERO red stripes while the scrolled content band warps cleanly at alpha = 64!
        cuda.reset_motion_history();
        let mut sc0 = vec![0x1Cu8; w * h * 4];
        let mut sc1 = vec![0x1Cu8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                sc0[i + 3] = 0xFF;
                sc1[i + 3] = 0xFF;
                if (600..624).contains(&y) && (120..960).contains(&x) {
                    let v = if ((x / 16) & 1) == 0 { 0xF0 } else { 0x38 };
                    sc0[i] = v;
                    sc0[i + 1] = v;
                    sc0[i + 2] = v;
                }
                if (552..576).contains(&y) && (120..960).contains(&x) {
                    let v = if ((x / 16) & 1) == 0 { 0xF0 } else { 0x38 };
                    sc1[i] = v;
                    sc1[i + 1] = v;
                    sc1[i + 2] = v;
                }
            }
        }

        cuda.mark_frame_dirty();
        cuda.set_temporal_alpha(256);
        cuda.blit_guest_fb_filtered(&mut win_px, 720, 1600, 0, 0, 720, 1600, &sc0, &filter);

        cuda.mark_frame_dirty();
        cuda.set_temporal_alpha(40);
        cuda.blit_guest_fb_filtered(&mut win_px, 720, 1600, 0, 0, 720, 1600, &sc1, &filter);
        if cuda.available {
            assert!(cuda.last_motion_valid, "Coherent scroll must set last_motion_valid == true");
            assert_eq!(cuda.last_mv_y, -48, "Coherent scroll must detect mv_y == -48");
        }
        // Check static chrome bands (oy = 0..220 -> sy = 0..330, and oy = 1520..1600 -> sy = 2280..2400)
        // plus interior dark region (oy = 600..1200) at alpha = 40 (< 128)!
        for oy in (0..220usize).chain(600..1200usize).chain(1520..1600usize) {
            for ox in 0..720usize {
                let px = win_px[oy * 720 + ox];
                let r = ((px >> 16) & 0xFF) as i32;
                let g = ((px >> 8) & 0xFF) as i32;
                let b = (px & 0xFF) as i32;
                assert!(
                    (r - 0x1C).abs() <= 1 && (g - 0x1C).abs() <= 1 && (b - 0x1C).abs() <= 1,
                    "Red-stripe corruption during scroll at alpha=40, ({}, {}): 0x{:08x}",
                    ox,
                    oy,
                    px
                );
            }
        }

        // At alpha = 64 (25% along -48 px scroll -> y = 612 - 12 = 600 -> oy = 400, ox = 152):
        cuda.set_temporal_alpha(64);
        cuda.blit_guest_fb_filtered(&mut win_px, 720, 1600, 0, 0, 720, 1600, &sc1, &filter);
        if cuda.available {
            let quarter_luma = win_px[400 * 720 + 152] & 0xFF;
            assert!(
                quarter_luma > 0xC0,
                "Optical flow at alpha=64 (<128) must warp cleanly to oy=400 (got 0x{:02x})",
                quarter_luma
            );
        }
    }

    #[test]
    fn test_wide_range_fast_swipe_and_live_warp() {
        let mut cuda = CudaAccelerator::new();
        let w = 1080usize;
        let h = 2400usize;
        let mut filter = DisplayFilterConfig::default();
        filter.dark_mode = true;
        let mut win_px = vec![0u32; 720 * 1600];

        // Fast swipe: content row jumps by -363 px (well beyond old i8 [-120..+120] limit, and non-multiple of 4!)
        // Frame 0: row at y = 1200..1228; Frame 1: row at y = 837..865 (mv_y = -363).
        let mut fast0 = vec![0x1Cu8; w * h * 4];
        let mut fast1 = vec![0x1Cu8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                fast0[i + 3] = 0xFF;
                fast1[i + 3] = 0xFF;
                if (1200..1228).contains(&y) && (120..960).contains(&x) {
                    let v = if ((x / 16) & 1) == 0 { 0xF2 } else { 0x3A };
                    fast0[i] = v;
                    fast0[i + 1] = v;
                    fast0[i + 2] = v;
                }
                if (837..865).contains(&y) && (120..960).contains(&x) {
                    let v = if ((x / 16) & 1) == 0 { 0xF2 } else { 0x3A };
                    fast1[i] = v;
                    fast1[i + 1] = v;
                    fast1[i + 2] = v;
                }
            }
        }

        // 1. Upload fast0 & verify zero-latency live_shift_y = -36 shifts y=1212 UP to sy=1176 (oy=784),
        //    keeps Large Title across y=268 unbroken (bar y=250..290 shifts cleanly to sy=214..254 -> oy=160),
        //    AND keeps bottom row across y=2300 unbroken without vertical barcode smearing!
        for y in (250..290).chain(2288..2304) {
            for x in 200..400 {
                let i = (y * w + x) * 4;
                fast0[i] = 0xE8;
                fast0[i + 1] = 0xE8;
                fast0[i + 2] = 0xE8;
            }
        }
        cuda.mark_frame_dirty();
        cuda.set_temporal_alpha(256);
        cuda.set_live_shift(0, -36);
        cuda.blit_guest_fb_filtered(&mut win_px, 720, 1600, 0, 0, 720, 1600, &fast0, &filter);
        if cuda.available {
            let live_warped_luma = win_px[784 * 720 + 152] & 0xFF;
            assert!(
                live_warped_luma > 0xC0,
                "Zero-latency live_shift_y=-36 must warp y=1212 up to oy=784 immediately (got 0x{:02x})",
                live_warped_luma
            );
            // Large Title spanning y=250..290 must shift smoothly across y=268 to sy=240 (oy=160) with ZERO y=268 cut!
            assert!(
                (win_px[160 * 720 + 200] & 0xFF) > 0xC0,
                "Large Title across y=268 must NOT be horizontally sliced during live_shift_y!"
            );
            // Below the shifted 2288..2304 bar (sy=2280 -> src 2316 -> oy=1520, and sy=2310 -> oy=1540 across y=2300),
            // pixels must be 0x1C, with ZERO duplicated row at y=2300 and ZERO barcode smearing!
            assert!(
                ((win_px[1520 * 720 + 200] & 0xFF) as i32 - 0x1C).abs() <= 1
                    && ((win_px[1540 * 720 + 200] & 0xFF) as i32 - 0x1C).abs() <= 1,
                "Bottom viewport rows must NOT duplicate at y=2300 or smear into vertical barcode stripes!"
            );
            // Status bar + Notch (oy = 50 -> sy = 75 < 126) must remain unshifted 0x1C
            assert!(
                ((win_px[50 * 720 + 152] & 0xFF) as i32 - 0x1C).abs() <= 1,
                "Status bar (sy < 126) must stay static during live_shift_y"
            );
        }

        // 2. Upload fast1 with live_shift=0 and alpha=128 -> verify 1px-exact mv_y == -363 & midpoint sy = 1032 (oy = 688)
        cuda.set_live_shift(0, 0);
        cuda.mark_frame_dirty();
        cuda.set_temporal_alpha(128);
        cuda.blit_guest_fb_filtered(&mut win_px, 720, 1600, 0, 0, 720, 1600, &fast1, &filter);
        println!(
            "Wide-range CUDA swipe test: valid={}, mv=({}, {}), upload+flow={} us",
            cuda.last_motion_valid, cuda.last_mv_x, cuda.last_mv_y, cuda.last_exec_us
        );
        if cuda.available {
            assert!(cuda.last_motion_valid, "Fast -363px swipe must be detected as valid motion");
            assert!(
                (cuda.last_mv_y - (-363)).abs() <= 1,
                "2-stage motion kernel must recover exact mv_y=-363 (got {})",
                cuda.last_mv_y
            );
            let mid_fast_luma = win_px[688 * 720 + 152] & 0xFF;
            assert!(
                mid_fast_luma > 0xC0,
                "Wide-range optical flow at alpha=128 must warp -363px swipe to midpoint oy=688 (got 0x{:02x})",
                mid_fast_luma
            );
        }
    }
}
