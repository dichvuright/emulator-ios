//! Authentic Apple iOS 16.7.16 (20H392) iPhone X (D22AP) SpringBoard, Lock Screen,
//! 20-icon Home Screen + 4-icon Frosted Glass Dock, Settings, Safari, Phone,
//! Control Center, Stock Apps, and Metal/GLES 3D Game Benchmark renderer.

use crate::font::{
    draw_ios16_lock_clock, draw_sdf_numeral, draw_text_centered, draw_text_scaled,
    draw_text_semibold, draw_text_shadow, measure_sdf_numerals, measure_text_px,
};
use crate::raster::{
    blend_argb, blur_rounded_rect, draw_line, draw_ring, fill_circle, fill_rect, fill_rounded_rect,
    fill_rounded_rect_gradient, fill_triangle, lerp_color, put_pixel_blended, render_squircle_icon,
    rgb, rgba,
};
use crate::{GpuCommand, IosSimState, SweetGpuEngine, GPU_CMD_TRIANGLE};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

struct RawBgraAsset {
    width: usize,
    height: usize,
    pixels: Vec<u32>,
}

struct AppleAssetsCache {
    icons: HashMap<String, RawBgraAsset>,
    wallpaper_light: Option<RawBgraAsset>,
    wallpaper_dark: Option<RawBgraAsset>,
}

static APPLE_ASSETS: OnceLock<AppleAssetsCache> = OnceLock::new();

fn load_bgra_file(path: &Path) -> Option<RawBgraAsset> {
    let bytes = fs::read(path).ok()?;
    if bytes.len() < 8 {
        return None;
    }
    let w = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    let h = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let count = w.checked_mul(h)?;
    if bytes.len() < 8 + count * 4 {
        return None;
    }
    let mut pixels = Vec::with_capacity(count);
    for chunk in bytes[8..8 + count * 4].chunks_exact(4) {
        let mut px = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        if (px >> 24) == 0 && px != 0 {
            px |= 0xFF00_0000;
        }
        pixels.push(px);
    }
    Some(RawBgraAsset {
        width: w,
        height: h,
        pixels,
    })
}

fn find_apple_assets_dir() -> Option<PathBuf> {
    let candidates = [
        "emu_workspace/apple_assets",
        "../emu_workspace/apple_assets",
        "../../emu_workspace/apple_assets",
    ];
    for c in &candidates {
        let p = PathBuf::from(c);
        if p.join("icons").exists() {
            return Some(p);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut cur = exe.parent();
        for _ in 0..5 {
            if let Some(d) = cur {
                let cand = d.join("emu_workspace").join("apple_assets");
                if cand.join("icons").exists() {
                    return Some(cand);
                }
                cur = d.parent();
            }
        }
    }
    None
}

fn get_apple_assets() -> &'static AppleAssetsCache {
    APPLE_ASSETS.get_or_init(|| {
        let mut icons = HashMap::new();
        let mut wallpaper_light = None;
        let mut wallpaper_dark = None;

        if let Some(base) = find_apple_assets_dir() {
            let icons_dir = base.join("icons");
            if let Ok(entries) = fs::read_dir(&icons_dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.extension().and_then(|s| s.to_str()) == Some("bgra") {
                        if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                            if let Some(asset) = load_bgra_file(&p) {
                                let display_key = stem.replace('_', " ");
                                icons.insert(display_key.to_lowercase(), RawBgraAsset {
                                    width: asset.width,
                                    height: asset.height,
                                    pixels: asset.pixels.clone(),
                                });
                                icons.insert(stem.to_lowercase(), asset);
                            }
                        }
                    }
                }
            }
            wallpaper_light = load_bgra_file(&base.join("wallpapers").join("SydneyLight.bgra"));
            wallpaper_dark = load_bgra_file(&base.join("wallpapers").join("SydneyDark.bgra"));
        }

        AppleAssetsCache {
            icons,
            wallpaper_light,
            wallpaper_dark,
        }
    })
}

fn sample_asset_box(asset: &RawBgraAsset, dst: &mut [u32], dst_w: usize, dst_h: usize) {
    if asset.width == 0 || asset.height == 0 || dst_w == 0 || dst_h == 0 {
        return;
    }
    for dy in 0..dst_h {
        let sy0 = (dy * asset.height) / dst_h;
        let sy1 = (((dy + 1) * asset.height) / dst_h).max(sy0 + 1).min(asset.height);
        for dx in 0..dst_w {
            let sx0 = (dx * asset.width) / dst_w;
            let sx1 = (((dx + 1) * asset.width) / dst_w).max(sx0 + 1).min(asset.width);
            let mut r_sum = 0u32;
            let mut g_sum = 0u32;
            let mut b_sum = 0u32;
            let mut cnt = 0u32;
            for sy in sy0..sy1 {
                let row = sy * asset.width;
                for sx in sx0..sx1 {
                    let px = asset.pixels[row + sx];
                    let a = (px >> 24) & 0xFF;
                    let r = (px >> 16) & 0xFF;
                    let g = (px >> 8) & 0xFF;
                    let b = px & 0xFF;
                    // Composite over white if icon edge has transparency inside squircle
                    let er = (r * a + 255 * (255 - a)) / 255;
                    let eg = (g * a + 255 * (255 - a)) / 255;
                    let eb = (b * a + 255 * (255 - a)) / 255;
                    r_sum += er;
                    g_sum += eg;
                    b_sum += eb;
                    cnt += 1;
                }
            }
            if cnt > 0 {
                dst[dy * dst_w + dx] = 0xFF00_0000
                    | ((r_sum / cnt) << 16)
                    | ((g_sum / cnt) << 8)
                    | (b_sum / cnt);
            }
        }
    }
}

/// Renders the authentic iOS 16 stock wallpaper (`SydneyWallpaper.jpg` or `SydneyDark.bgra`
/// extracted from the official iOS 16.7.16 IPSW when available, with bilinear center-crop scaling).
pub(crate) fn draw_ios16_wallpaper(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    dark_mode: bool,
    _tick: u64,
) {
    if fb_w == 0 || fb_h == 0 {
        return;
    }
    let wp_opt = if dark_mode {
        get_apple_assets()
            .wallpaper_dark
            .as_ref()
            .or(get_apple_assets().wallpaper_light.as_ref())
    } else {
        get_apple_assets().wallpaper_light.as_ref()
    };
    if let Some(wp) = wp_opt {
        if wp.width > 16 && wp.height > 16 {
            // Center-crop aspect-ratio fit from the 875x875 SydneyWallpaper into fb_w x fb_h
            let target_aspect = (fb_w as f32) / (fb_h as f32);
            let crop_h = wp.height as f32;
            let crop_w = (crop_h * target_aspect).min(wp.width as f32);
            let x_off = ((wp.width as f32) - crop_w) * 0.5;

            for y in 0..fb_h {
                let fy = ((y as f32) * (crop_h - 1.0)) / ((fb_h.max(2) - 1) as f32);
                let y0 = (fy.floor() as usize).min(wp.height - 1);
                let y1 = (y0 + 1).min(wp.height - 1);
                let ty = ((fy - (y0 as f32)) * 256.0) as u32;
                let row = &mut fb[y * fb_w..(y + 1) * fb_w];
                for (x, px) in row.iter_mut().enumerate() {
                    let fx = x_off + ((x as f32) * (crop_w - 1.0)) / ((fb_w.max(2) - 1) as f32);
                    let x0 = (fx.floor() as usize).min(wp.width - 1);
                    let x1 = (x0 + 1).min(wp.width - 1);
                    let tx = ((fx - (x0 as f32)) * 256.0) as u32;
                    let c00 = wp.pixels[y0 * wp.width + x0];
                    let c10 = wp.pixels[y0 * wp.width + x1];
                    let c01 = wp.pixels[y1 * wp.width + x0];
                    let c11 = wp.pixels[y1 * wp.width + x1];
                    let top = lerp_color(c00, c10, tx);
                    let bot = lerp_color(c01, c11, tx);
                    *px = lerp_color(top, bot, ty);
                }
            }
            return;
        }
    }
    let inv_w = 1.0 / (fb_w.max(1) as f32);
    let inv_h = 1.0 / (fb_h.max(1) as f32);
    let dark_mul = if dark_mode { 0.52f32 } else { 1.0f32 };

    // 2x2 Bayer dither matrix in [-0.375, +0.375] to eliminate 8-bit gradient banding
    const BAYER_2X2: [[f32; 2]; 2] = [[-0.375, 0.125], [0.375, -0.125]];

    for y in 0..fb_h {
        let v = (y as f32) * inv_h;
        let row = &mut fb[y * fb_w..(y + 1) * fb_w];
        let bayer_row = &BAYER_2X2[y & 1];

        // Precompute vertical wave parameters for this scanline
        let wave_center_x = 0.52 + 0.22 * (v * 3.14159 * 1.15 - 0.5).sin();
        for (x, px) in row.iter_mut().enumerate() {
            let u = (x as f32) * inv_w;

            // Base vertical/diagonal blend: Deep Midnight Blue (#0A1128) -> Royal Indigo (#1D3557)
            let diag = (v * 0.72 + u * 0.28).clamp(0.0, 1.0);
            let mut r = 10.0 + diag * (29.0 - 10.0);
            let mut g = 17.0 + diag * (53.0 - 17.0);
            let mut b = 40.0 + diag * (87.0 - 40.0);

            // Sweeping iOS 16 Teal / Sky wave (#0096C7 -> #48CAE4) across upper-middle S-curve
            let teal_curve_v = 0.42 - 0.18 * ((u - 0.15) * 3.0).cos();
            let d_teal = ((v - teal_curve_v) / 0.34).powi(2) + ((u - 0.48) / 0.75).powi(2);
            let w_teal = (1.0 - d_teal).clamp(0.0, 1.0);
            let w_teal_smooth = w_teal * w_teal * (3.0 - 2.0 * w_teal);
            let sky_mix = (1.0 - v).clamp(0.0, 1.0);
            let teal_r = 0.0 + sky_mix * 72.0;
            let teal_g = 150.0 + sky_mix * (202.0 - 150.0);
            let teal_b = 199.0 + sky_mix * (228.0 - 199.0);
            r = r + (teal_r - r) * (w_teal_smooth * 0.88);
            g = g + (teal_g - g) * (w_teal_smooth * 0.88);
            b = b + (teal_b - b) * (w_teal_smooth * 0.88);

            // Signature iOS 16 Warm Golden-Coral ribbon (#F4A261 / #E76F51) curving across lower-mid
            let coral_curve_v = 0.58 + 0.14 * ((u - 0.25) * 3.4).sin();
            let dist_coral = ((v - coral_curve_v) / 0.19).powi(2)
                + ((u - wave_center_x) / 0.56).powi(2);
            let w_coral = (1.0 - dist_coral).clamp(0.0, 1.0);
            let w_coral_smooth = w_coral * w_coral * (3.0 - 2.0 * w_coral);
            let coral_r = 231.0 + (1.0 - u) * (244.0 - 231.0);
            let coral_g = 111.0 + (1.0 - (v - 0.5).abs() * 2.0).clamp(0.0, 1.0) * (162.0 - 111.0);
            let coral_b = 81.0 + u * (97.0 - 81.0);
            r = r + (coral_r - r) * (w_coral_smooth * 0.78);
            g = g + (coral_g - g) * (w_coral_smooth * 0.78);
            b = b + (coral_b - b) * (w_coral_smooth * 0.78);

            // Deep ocean indigo vignette at the very bottom for high Dock contrast
            let bot_fade = ((v - 0.76) / 0.24).clamp(0.0, 1.0);
            let bot_smooth = bot_fade * bot_fade;
            r = r * (1.0 - bot_smooth * 0.52) + 14.0 * (bot_smooth * 0.52);
            g = g * (1.0 - bot_smooth * 0.45) + 34.0 * (bot_smooth * 0.45);
            b = b * (1.0 - bot_smooth * 0.25) + 78.0 * (bot_smooth * 0.25);

            let dither = bayer_row[x & 1];
            let r_u8 = (r * dark_mul + dither).clamp(0.0, 255.0) as u8;
            let g_u8 = (g * dark_mul + dither).clamp(0.0, 255.0) as u8;
            let b_u8 = (b * dark_mul + dither).clamp(0.0, 255.0) as u8;
            *px = rgb(r_u8, g_u8, b_u8);
        }
    }
}

/// Renders the iPhone X (D22AP) top sensor housing notch and status bar (`9:41`, signal bars or airplane, `5G`/Wi-Fi, battery).
pub(crate) fn draw_iphone_x_status_bar(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    dark_text: bool,
    screen_state: &IosSimState,
) {
    let w = fb_w as i32;
    let h = fb_h as i32;
    let fg = if dark_text {
        rgb(18, 20, 26)
    } else {
        rgb(250, 252, 255)
    };

    // Authentic iPhone X Sensor Housing Notch
    let notch_w = (w * 48 / 100).max(96);
    let notch_h = (h * 34 / 1000).clamp(20, 32);
    let notch_x = (w - notch_w) / 2;
    fill_rounded_rect(
        fb,
        fb_w,
        fb_h,
        notch_x,
        -8,
        notch_w,
        notch_h + 8,
        14,
        rgb(5, 6, 8),
    );

    // Earpiece speaker slot & infrared/front camera lens inside notch
    let spk_w = (notch_w / 4).max(22);
    let spk_x = (w - spk_w) / 2;
    let spk_y = (notch_h / 2) - 2;
    fill_rounded_rect(fb, fb_w, fb_h, spk_x, spk_y, spk_w, 4, 2, rgb(26, 28, 36));
    fill_circle(
        fb,
        fb_w,
        fb_h,
        spk_x + spk_w + 12,
        spk_y + 2,
        3,
        rgb(20, 42, 92),
    );
    put_pixel_blended(
        fb,
        fb_w,
        fb_h,
        spk_x + spk_w + 11,
        spk_y + 1,
        rgba(120, 190, 255, 180),
    );

    // Status Bar Left: "9:41" centered in left ear
    let status_y = (notch_h - 15).max(7);
    let left_ear_w = notch_x.max(20);
    let time_w = measure_text_px("9:41", 1);
    let time_x = ((left_ear_w - time_w) / 2).max(12);
    draw_text_semibold(fb, fb_w, fb_h, time_x, status_y, "9:41", fg, 1, 2, false);

    // Status Bar Right: Cellular signal bars / Airplane Mode + "5G" + iOS Battery icon
    let right_ear_x = notch_x + notch_w;
    let right_start = right_ear_x + 8;

    if screen_state.airplane_mode {
        // Draw Airplane icon in status bar
        let ax = right_start + 8;
        let ay = status_y + 5;
        draw_line(fb, fb_w, fb_h, ax - 5, ay, ax + 5, ay, 2, fg);
        fill_triangle(fb, fb_w, fb_h, ax - 1, ay, ax - 3, ay - 5, ax + 2, ay, fg);
        fill_triangle(fb, fb_w, fb_h, ax - 1, ay, ax - 3, ay + 5, ax + 2, ay, fg);
    } else {
        // 4 ascending rounded cellular bars
        for bar in 0..4i32 {
            let bh = 4 + bar * 2;
            let bx = right_start + bar * 4;
            let by = status_y + 10 - bh;
            fill_rounded_rect(fb, fb_w, fb_h, bx, by, 2, bh, 1, fg);
        }
        // "5G" or Wi-Fi indicator
        let net_lbl = if !screen_state.initialized || screen_state.cellular_enabled {
            "5G"
        } else if screen_state.wifi_enabled {
            "WiFi"
        } else {
            ""
        };
        if !net_lbl.is_empty() {
            draw_text_scaled(fb, fb_w, fb_h, right_start + 18, status_y, net_lbl, fg, 1, 2);
        }
    }

    // Authentic iOS Battery icon at far right
    let bat_w = 22;
    let bat_h = 11;
    let bat_x = (w - bat_w - 12).max(right_start + 34);
    let bat_y = status_y;
    let outline_col = if dark_text {
        rgba(28, 30, 38, 130)
    } else {
        rgba(255, 255, 255, 145)
    };
    fill_rounded_rect(fb, fb_w, fb_h, bat_x, bat_y, bat_w, bat_h, 3, outline_col);
    // Inner cutout
    let inner_bg = if dark_text {
        rgba(242, 242, 247, 220)
    } else {
        rgba(15, 25, 48, 175)
    };
    fill_rounded_rect(
        fb,
        fb_w,
        fb_h,
        bat_x + 1,
        bat_y + 1,
        bat_w - 2,
        bat_h - 2,
        2,
        inner_bg,
    );
    // Battery charge level fill (yellow in Low Power Mode, else fg)
    let fill_col = if screen_state.low_power_mode {
        rgb(255, 204, 0)
    } else {
        fg
    };
    fill_rounded_rect(
        fb,
        fb_w,
        fb_h,
        bat_x + 2,
        bat_y + 2,
        bat_w - 4,
        bat_h - 4,
        2,
        fill_col,
    );
    // Battery positive terminal cap
    fill_rounded_rect(
        fb,
        fb_w,
        fb_h,
        bat_x + bat_w + 1,
        bat_y + 3,
        2,
        5,
        1,
        outline_col,
    );
}

/// Renders the clean iPhone X bottom Home Indicator pill (`128x5` at `y = 788` on `360x800`).
pub(crate) fn draw_home_indicator(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    screen_state: &IosSimState,
    dark_bar: bool,
) {
    let w = fb_w as i32;
    let h = fb_h as i32;
    let bar_w = (w * 128 / 360).max(88);
    let bar_h = 5;
    let bar_x = (w - bar_w) / 2;
    let swipe_lift = if screen_state.edge_swipe_active { 4 } else { 0 };
    let bar_y = (h - 12 - swipe_lift).max(0);
    let bar_color = if dark_bar {
        rgba(20, 22, 28, 220)
    } else {
        rgba(255, 255, 255, 235)
    };
    fill_rounded_rect(fb, fb_w, fb_h, bar_x, bar_y, bar_w, bar_h, 2, bar_color);
}

/// Draws the genuine Apple `@3x` (`180x180`) icon extracted from the iOS 16.7.16 IPSW
/// inside a local `size x size` squircle canvas (with vector fallback if asset dir is absent).
pub(crate) fn draw_stock_icon_canvas(canvas: &mut [u32], size: usize, kind: &str) {
    let s = size as i32;
    let cx = s / 2;
    let cy = s / 2;

    if let Some(asset) = get_apple_assets().icons.get(&kind.to_lowercase()) {
        sample_asset_box(asset, canvas, size, size);
        if kind.eq_ignore_ascii_case("Clock") {
            // Draw live hour (9), minute (41), and orange second hands over the genuine Apple Clock face
            let r_face = (s * 43) / 100;
            draw_line(
                canvas,
                size,
                size,
                cx,
                cy,
                cx - (r_face * 48) / 100,
                cy - (r_face * 12) / 100,
                2,
                rgb(24, 24, 28),
            );
            draw_line(
                canvas,
                size,
                size,
                cx,
                cy,
                cx - (r_face * 55) / 100,
                cy + (r_face * 48) / 100,
                2,
                rgb(24, 24, 28),
            );
            draw_line(
                canvas,
                size,
                size,
                cx - 3,
                cy + 3,
                cx + (r_face * 68) / 100,
                cy - (r_face * 54) / 100,
                1,
                rgb(255, 149, 0),
            );
            fill_circle(canvas, size, size, cx, cy, 2, rgb(255, 149, 0));
        }
        return;
    }

    let fill_grad = |buf: &mut [u32], top_c: u32, bot_c: u32| {
        for y in 0..s {
            let t = ((y as u32) * 256) / (s.max(1) as u32);
            let c = lerp_color(top_c, bot_c, t);
            buf[(y as usize) * size..((y + 1) as usize) * size].fill(c);
        }
    };

    match kind {
        "FaceTime" => {
            // Bright green gradient (#34C759 -> #28A745) + white video camera silhouette
            fill_grad(canvas, rgb(52, 199, 89), rgb(36, 162, 66));
            let body_w = s * 44 / 100;
            let body_h = s * 32 / 100;
            let bx = cx - body_w / 2 - 4;
            let by = cy - body_h / 2;
            fill_rounded_rect(canvas, size, size, bx, by, body_w, body_h, 5, rgb(255, 255, 255));
            // Right trapezoid lens flare
            let lx0 = bx + body_w + 1;
            let lx1 = lx0 + s * 17 / 100;
            fill_triangle(
                canvas,
                size,
                size,
                lx0,
                cy - 3,
                lx1,
                by + 2,
                lx1,
                by + body_h - 2,
                rgb(255, 255, 255),
            );
            fill_triangle(
                canvas,
                size,
                size,
                lx0,
                cy - 3,
                lx1,
                by + body_h - 2,
                lx0,
                cy + 3,
                rgb(255, 255, 255),
            );
        }
        "Calendar" => {
            // Pure white squircle + red "TUE" top text + large crisp black "20"
            fill_grad(canvas, rgb(255, 255, 255), rgb(246, 246, 249));
            let tue_w = measure_text_px("TUE", 1);
            draw_text_semibold(
                canvas,
                size,
                size,
                cx - tue_w / 2,
                s * 14 / 100,
                "TUE",
                rgb(255, 59, 48),
                1,
                2,
                false,
            );
            let num_h = (s * 46 / 100).max(18);
            let num_w = measure_sdf_numerals("20", num_h, 2);
            let mut nx = cx - num_w / 2;
            let ny = s * 39 / 100;
            for b in b"20" {
                let cw = draw_sdf_numeral(
                    canvas,
                    size,
                    size,
                    nx,
                    ny,
                    *b,
                    num_h,
                    2.0,
                    rgb(24, 24, 28),
                );
                nx += cw + 2;
            }
        }
        "Photos" => {
            // White squircle + Apple's iconic 8 overlapping translucent color petals
            canvas.fill(rgb(255, 255, 255));
            let petals: [(f32, u32); 8] = [
                (-std::f32::consts::FRAC_PI_2, rgba(255, 149, 0, 210)), // Top: Orange
                (-std::f32::consts::FRAC_PI_4, rgba(255, 204, 0, 210)), // NE: Yellow
                (0.0, rgba(52, 199, 89, 205)),                          // Right: Green
                (std::f32::consts::FRAC_PI_4, rgba(0, 199, 190, 205)),  // SE: Mint
                (std::f32::consts::FRAC_PI_2, rgba(48, 176, 199, 205)), // Bottom: Cyan
                (3.0 * std::f32::consts::FRAC_PI_4, rgba(0, 122, 255, 205)), // SW: Blue
                (std::f32::consts::PI, rgba(175, 82, 222, 205)),        // Left: Purple
                (-3.0 * std::f32::consts::FRAC_PI_4, rgba(255, 45, 85, 205)), // NW: Magenta
            ];
            let petal_dist = (s as f32) * 0.21;
            let rx = (s as f32) * 0.16;
            let ry = (s as f32) * 0.105;
            for &(ang, col) in &petals {
                let (sin_a, cos_a) = ang.sin_cos();
                let pcx = (cx as f32) + cos_a * petal_dist;
                let pcy = (cy as f32) + sin_a * petal_dist;
                let base_a = (col >> 24) & 0xFF;
                let rgb_part = col & 0x00FF_FFFF;
                for py in 0..s {
                    for px in 0..s {
                        let dx = (px as f32 + 0.5) - pcx;
                        let dy = (py as f32 + 0.5) - pcy;
                        let u = (dx * cos_a + dy * sin_a) / rx;
                        let v = (-dx * sin_a + dy * cos_a) / ry;
                        let d = (u * u + v * v).sqrt();
                        let edge = 1.08 - d;
                        if edge > 0.0 {
                            let cov = (edge * 4.5).clamp(0.0, 1.0);
                            let eff_a = ((base_a as f32) * cov) as u32;
                            if eff_a > 0 {
                                let idx = (py as usize) * size + (px as usize);
                                canvas[idx] = blend_argb(canvas[idx], (eff_a << 24) | rgb_part);
                            }
                        }
                    }
                }
            }
        }
        "Camera" => {
            // Silver metallic gradient + dark charcoal camera body + lens rings + yellow mode dot
            fill_grad(canvas, rgb(224, 227, 232), rgb(162, 167, 176));
            let bw = s * 68 / 100;
            let bh = s * 48 / 100;
            let bx = cx - bw / 2;
            let by = cy - bh / 2 + 3;
            // Viewfinder top hump
            fill_rounded_rect(
                canvas,
                size,
                size,
                cx - bw / 4,
                by - 5,
                bw / 2,
                8,
                3,
                rgb(44, 45, 48),
            );
            // Main charcoal body
            fill_rounded_rect(canvas, size, size, bx, by, bw, bh, 6, rgb(44, 45, 48));
            // Metallic outer lens ring
            let lr = bh * 36 / 100;
            draw_ring(
                canvas,
                size,
                size,
                cx,
                by + bh / 2,
                lr - 2,
                lr + 1,
                rgb(175, 180, 188),
            );
            // Deep glass lens
            fill_circle(canvas, size, size, cx, by + bh / 2, lr - 2, rgb(20, 24, 34));
            fill_circle(
                canvas,
                size,
                size,
                cx - 2,
                by + bh / 2 - 2,
                2,
                rgba(100, 180, 255, 190),
            );
            // Yellow shutter/flash dot at top-right of body
            fill_circle(
                canvas,
                size,
                size,
                bx + bw - 6,
                by + 5,
                2,
                rgb(255, 204, 0),
            );
        }
        "Mail" => {
            // Blue gradient (#1A8CFF -> #0066CC) + white envelope with crisp V-fold
            fill_grad(canvas, rgb(26, 140, 255), rgb(0, 102, 204));
            let ew = s * 66 / 100;
            let eh = s * 46 / 100;
            let ex = cx - ew / 2;
            let ey = cy - eh / 2;
            fill_rounded_rect(canvas, size, size, ex, ey, ew, eh, 4, rgb(255, 255, 255));
            // Lower fold lines
            draw_line(
                canvas,
                size,
                size,
                ex + 2,
                ey + eh - 2,
                cx - 4,
                cy,
                1,
                rgba(0, 102, 204, 95),
            );
            draw_line(
                canvas,
                size,
                size,
                ex + ew - 2,
                ey + eh - 2,
                cx + 4,
                cy,
                1,
                rgba(0, 102, 204, 95),
            );
            // Crisp top V-flap fold
            draw_line(
                canvas,
                size,
                size,
                ex + 1,
                ey + 1,
                cx,
                cy + 3,
                2,
                rgb(18, 118, 232),
            );
            draw_line(
                canvas,
                size,
                size,
                ex + ew - 1,
                ey + 1,
                cx,
                cy + 3,
                2,
                rgb(18, 118, 232),
            );
        }
        "Clock" => {
            // Black squircle + white circle clock face + hour/minute hands + orange second hand
            canvas.fill(rgb(28, 28, 30));
            let cr = s * 41 / 100;
            fill_circle(canvas, size, size, cx, cy, cr, rgb(250, 250, 252));
            // 12 hour tick dots around perimeter
            for h_idx in 0..12 {
                let ang = (h_idx as f32) * (std::f32::consts::TAU / 12.0);
                let tx = cx + ((cr - 3) as f32 * ang.cos()).round() as i32;
                let ty = cy + ((cr - 3) as f32 * ang.sin()).round() as i32;
                put_pixel_blended(canvas, size, size, tx, ty, rgb(40, 40, 45));
            }
            // Hour hand (pointing ~10 o'clock)
            draw_line(
                canvas,
                size,
                size,
                cx,
                cy,
                cx - cr * 48 / 100,
                cy - cr * 32 / 100,
                2,
                rgb(20, 20, 22),
            );
            // Minute hand (pointing ~2 o'clock)
            draw_line(
                canvas,
                size,
                size,
                cx,
                cy,
                cx + cr * 62 / 100,
                cy - cr * 42 / 100,
                2,
                rgb(20, 20, 22),
            );
            // Orange second hand (pointing ~6:35)
            draw_line(
                canvas,
                size,
                size,
                cx + 2,
                cy - 3,
                cx - cr * 30 / 100,
                cy + cr * 70 / 100,
                1,
                rgb(255, 149, 0),
            );
            fill_circle(canvas, size, size, cx, cy, 2, rgb(255, 149, 0));
        }
        "Maps" => {
            // Apple Maps multi-quadrant road map + blue interstate shield + blue navigation arrow circle
            canvas.fill(rgb(242, 238, 230));
            // Top-left green park
            fill_rect(canvas, size, size, 0, 0, cx, cy + 4, rgb(108, 206, 103));
            // Bottom-left blue bay water
            fill_rect(canvas, size, size, 0, cy + 4, cx + 6, s, rgb(90, 200, 250));
            // Bottom-right green park strip
            fill_rect(canvas, size, size, cx + 8, cy + 8, s, s, rgb(162, 224, 130));
            // White local roads
            fill_rect(canvas, size, size, 0, cy + 1, s, 5, rgb(255, 255, 255));
            draw_line(canvas, size, size, 0, s - 8, s, 8, 5, rgb(255, 255, 255));
            // Golden highway running vertically
            fill_rect(canvas, size, size, cx - 2, 0, 8, s, rgb(235, 165, 20));
            fill_rect(canvas, size, size, cx - 1, 0, 6, s, rgb(255, 208, 45));
            // Miniature Interstate shield at top-left
            fill_rounded_rect(canvas, size, size, 7, 8, 12, 12, 4, rgb(0, 122, 255));
            fill_rect(canvas, size, size, 7, 8, 12, 4, rgb(255, 59, 48));
            // Blue navigation circle with white NE arrow at bottom-right
            let nav_cx = cx + 6;
            let nav_cy = cy + 4;
            fill_circle(canvas, size, size, nav_cx, nav_cy, 12, rgb(255, 255, 255));
            fill_circle(canvas, size, size, nav_cx, nav_cy, 10, rgb(0, 122, 255));
            fill_triangle(
                canvas,
                size,
                size,
                nav_cx + 4,
                nav_cy - 5,
                nav_cx - 5,
                nav_cy + 1,
                nav_cx + 1,
                nav_cy + 5,
                rgb(255, 255, 255),
            );
        }
        "Weather" => {
            // Sky blue gradient + glowing golden sun partially behind a soft white cloud
            fill_grad(canvas, rgb(28, 114, 242), rgb(72, 182, 255));
            // Golden sun at upper-left
            fill_circle(
                canvas,
                size,
                size,
                cx - 7,
                cy - 6,
                11,
                rgba(255, 214, 10, 90),
            );
            fill_circle(canvas, size, size, cx - 7, cy - 6, 9, rgb(255, 214, 10));
            // Puffy white cloud overlapping the sun
            let cl_y = cy + 4;
            fill_rounded_rect(
                canvas,
                size,
                size,
                cx - 14,
                cl_y - 2,
                30,
                12,
                6,
                rgb(255, 255, 255),
            );
            fill_circle(canvas, size, size, cx - 6, cl_y - 2, 8, rgb(255, 255, 255));
            fill_circle(canvas, size, size, cx + 4, cl_y - 4, 10, rgb(255, 255, 255));
            fill_circle(canvas, size, size, cx + 11, cl_y + 1, 6, rgb(255, 255, 255));
        }
        "Reminders" => {
            // White squircle + 3 colored circular dots (blue/red/orange) with gray list lines
            canvas.fill(rgb(255, 255, 255));
            let rows = [
                (s * 26 / 100, rgb(0, 122, 255)),
                (s * 50 / 100, rgb(255, 59, 48)),
                (s * 74 / 100, rgb(255, 149, 0)),
            ];
            for &(ry, dot_col) in &rows {
                fill_circle(canvas, size, size, 13, ry, 4, dot_col);
                fill_rounded_rect(
                    canvas,
                    size,
                    size,
                    22,
                    ry - 1,
                    s - 30,
                    3,
                    1,
                    rgb(205, 205, 212),
                );
            }
        }
        "Notes" => {
            // White squircle + rich yellow top header strip + subtle gray horizontal lines
            canvas.fill(rgb(255, 255, 255));
            let hdr_h = s * 28 / 100;
            fill_rect(canvas, size, size, 0, 0, s, hdr_h, rgb(255, 204, 0));
            fill_rect(canvas, size, size, 0, hdr_h, s, 2, rgb(228, 176, 0));
            for line_idx in 1..=3 {
                let ly = hdr_h + line_idx * ((s - hdr_h) / 4);
                fill_rect(canvas, size, size, 0, ly, s, 1, rgb(224, 224, 230));
            }
        }
        "Stocks" => {
            // Black squircle + glowing cyan jagged stock trend graph + vertical grid lines
            fill_grad(canvas, rgb(20, 22, 26), rgb(28, 32, 38));
            for gx in (10..s).step_by(11) {
                fill_rect(canvas, size, size, gx, 0, 1, s, rgba(255, 255, 255, 22));
            }
            let pts: [(i32, i32); 6] = [
                (4, s * 72 / 100),
                (s * 24 / 100, s * 56 / 100),
                (s * 42 / 100, s * 66 / 100),
                (s * 60 / 100, s * 38 / 100),
                (s * 76 / 100, s * 46 / 100),
                (s - 5, s * 24 / 100),
            ];
            // Soft cyan glow beneath trend line
            for w_seg in pts.windows(2) {
                let (x0, y0) = w_seg[0];
                let (x1, y1) = w_seg[1];
                for px in x0..=x1 {
                    let t = (px - x0) as f32 / ((x1 - x0).max(1) as f32);
                    let top_y = (y0 as f32 + t * (y1 - y0) as f32) as i32;
                    for py in top_y..s {
                        let a = ((1.0 - (py - top_y) as f32 / (s as f32)) * 85.0)
                            .clamp(0.0, 85.0) as u8;
                        put_pixel_blended(canvas, size, size, px, py, rgba(50, 195, 255, a));
                    }
                }
                draw_line(
                    canvas,
                    size,
                    size,
                    x0,
                    y0,
                    x1,
                    y1,
                    2,
                    rgb(90, 210, 255),
                );
            }
            fill_circle(
                canvas,
                size,
                size,
                pts[5].0,
                pts[5].1,
                3,
                rgb(255, 255, 255),
            );
        }
        "Books" => {
            // Warm orange gradient (#FF9500 -> #FF5E00) + white open book pages
            fill_grad(canvas, rgb(255, 155, 10), rgb(255, 94, 0));
            let bw = s * 30 / 100;
            let bh = s * 44 / 100;
            let by = cy - bh / 2;
            // Left page
            fill_rounded_rect(
                canvas,
                size,
                size,
                cx - bw - 1,
                by,
                bw,
                bh,
                3,
                rgb(255, 255, 255),
            );
            // Right page
            fill_rounded_rect(
                canvas,
                size,
                size,
                cx + 1,
                by,
                bw,
                bh,
                3,
                rgb(255, 255, 255),
            );
            // Spine crease
            fill_rect(
                canvas,
                size,
                size,
                cx - 1,
                by + 2,
                2,
                bh - 4,
                rgb(230, 110, 15),
            );
        }
        "App Store" => {
            // Apple blue gradient (#1D9BF0 -> #0061F2) + 3 white rounded crossed sticks forming 'A'
            fill_grad(canvas, rgb(29, 155, 240), rgb(0, 97, 242));
            let white = rgb(255, 255, 255);
            // Horizontal bar stick
            draw_line(
                canvas,
                size,
                size,
                cx - 13,
                cy + 5,
                cx + 13,
                cy + 5,
                4,
                white,
            );
            // Left diagonal stick
            draw_line(
                canvas,
                size,
                size,
                cx + 2,
                cy - 14,
                cx - 11,
                cy + 12,
                4,
                white,
            );
            // Right diagonal stick
            draw_line(
                canvas,
                size,
                size,
                cx - 2,
                cy - 14,
                cx + 11,
                cy + 12,
                4,
                white,
            );
        }
        "Podcasts" => {
            // Purple gradient (#AF52DE -> #7B1FA2) + white concentric radio rings & figure
            fill_grad(canvas, rgb(185, 85, 232), rgb(118, 28, 162));
            let white = rgb(255, 255, 255);
            draw_ring(canvas, size, size, cx, cy - 2, 14, 16, white);
            draw_ring(canvas, size, size, cx, cy - 2, 9, 11, white);
            // Clear bottom wedge for broadcast silhouette
            fill_triangle(
                canvas,
                size,
                size,
                cx,
                cy,
                cx - 12,
                s,
                cx + 12,
                s,
                rgb(132, 42, 178),
            );
            // Central broadcast person head & body
            fill_circle(canvas, size, size, cx, cy - 3, 4, white);
            fill_rounded_rect(canvas, size, size, cx - 4, cy + 3, 8, 12, 3, white);
        }
        "TV" => {
            // Black squircle + white Apple logo + "tv" letters
            fill_grad(canvas, rgb(28, 28, 32), rgb(8, 8, 10));
            let white = rgb(255, 255, 255);
            // Mini Apple silhouette on left
            fill_circle(canvas, size, size, cx - 9, cy + 2, 6, white);
            draw_line(
                canvas,
                size,
                size,
                cx - 9,
                cy - 6,
                cx - 7,
                cy - 8,
                2,
                white,
            );
            // "tv" letters on right
            draw_text_semibold(canvas, size, size, cx + 1, cy - 4, "tv", white, 1, 2, false);
        }
        "Health" => {
            // White squircle + glossy pink/red heart in the upper-right quadrant
            canvas.fill(rgb(255, 255, 255));
            let hx = cx + 5;
            let hy = cy - 5;
            let hr = 6;
            fill_circle(canvas, size, size, hx - 4, hy - 2, hr, rgb(255, 45, 85));
            fill_circle(canvas, size, size, hx + 4, hy - 2, hr, rgb(255, 59, 48));
            fill_triangle(
                canvas,
                size,
                size,
                hx - 10,
                hy,
                hx + 10,
                hy,
                hx,
                hy + 12,
                rgb(255, 45, 85),
            );
        }
        "Home" => {
            // Amber/orange gradient + white nested house outline with chimney
            fill_grad(canvas, rgb(255, 168, 12), rgb(255, 108, 0));
            let white = rgb(255, 255, 255);
            // Chimney
            fill_rect(canvas, size, size, cx + 7, cy - 12, 4, 8, white);
            // Outer house roof & body
            fill_triangle(
                canvas,
                size,
                size,
                cx,
                cy - 15,
                cx - 16,
                cy - 1,
                cx + 16,
                cy - 1,
                white,
            );
            fill_rounded_rect(canvas, size, size, cx - 12, cy - 2, 24, 16, 3, white);
            // Nested inner warm home cutout
            fill_triangle(
                canvas,
                size,
                size,
                cx,
                cy - 7,
                cx - 7,
                cy,
                cx + 7,
                cy,
                rgb(255, 138, 8),
            );
            fill_rounded_rect(
                canvas,
                size,
                size,
                cx - 6,
                cy,
                12,
                10,
                2,
                rgb(255, 138, 8),
            );
        }
        "Wallet" => {
            // Dark charcoal base + colorful blue/green/yellow/red card tops inside a beige pocket
            canvas.fill(rgb(28, 28, 30));
            let mx = 6;
            let cw = s - 2 * mx;
            // 4 stacked credit passes
            fill_rounded_rect(canvas, size, size, mx, 8, cw, 14, 3, rgb(0, 122, 255));
            fill_rounded_rect(canvas, size, size, mx, 13, cw, 14, 3, rgb(52, 199, 89));
            fill_rounded_rect(canvas, size, size, mx, 18, cw, 14, 3, rgb(255, 204, 0));
            fill_rounded_rect(canvas, size, size, mx, 23, cw, 14, 3, rgb(255, 59, 48));
            // Beige wallet pocket with curved top notch
            fill_rounded_rect(
                canvas,
                size,
                size,
                4,
                cy + 1,
                s - 8,
                s - cy - 5,
                5,
                rgb(218, 210, 198),
            );
            fill_circle(canvas, size, size, cx, cy + 1, 6, rgb(255, 59, 48));
        }
        "Settings" => {
            // Silver/slate gradient + authentic Apple 3-ring intricate mechanical gear with teeth & 6 inner spokes
            fill_grad(canvas, rgb(146, 152, 164), rgb(84, 88, 98));
            let gear_outer = rgb(232, 236, 242);
            let dark_cut = rgb(58, 62, 70);
            let r_outer = s * 36 / 100;
            // 12 outer cog teeth around perimeter
            for t_idx in 0..12 {
                let ang = (t_idx as f32) * (std::f32::consts::TAU / 12.0);
                let tx = cx + ((r_outer + 2) as f32 * ang.cos()).round() as i32;
                let ty = cy + ((r_outer + 2) as f32 * ang.sin()).round() as i32;
                draw_line(canvas, size, size, cx, cy, tx, ty, 4, gear_outer);
            }
            // Outer gear wheel & dark interior cutout
            fill_circle(canvas, size, size, cx, cy, r_outer, gear_outer);
            fill_circle(canvas, size, size, cx, cy, r_outer - 4, dark_cut);
            // 6 mechanical inner spokes
            for spk in 0..3 {
                let ang = (spk as f32) * (std::f32::consts::PI / 3.0);
                let dx = ((r_outer - 3) as f32 * ang.cos()).round() as i32;
                let dy = ((r_outer - 3) as f32 * ang.sin()).round() as i32;
                draw_line(
                    canvas,
                    size,
                    size,
                    cx - dx,
                    cy - dy,
                    cx + dx,
                    cy + dy,
                    3,
                    gear_outer,
                );
            }
            // Middle ring & center hub
            draw_ring(canvas, size, size, cx, cy, 5, 8, gear_outer);
            fill_circle(canvas, size, size, cx, cy, 4, dark_cut);
        }
        "Files" => {
            // White squircle + rich blue Apple folder icon
            canvas.fill(rgb(255, 255, 255));
            let fw = s * 68 / 100;
            let fh = s * 48 / 100;
            let fx = cx - fw / 2;
            let fy = cy - fh / 2 + 2;
            // Back folder tab
            fill_rounded_rect(
                canvas,
                size,
                size,
                fx,
                fy - 4,
                fw * 44 / 100,
                10,
                3,
                rgb(16, 118, 230),
            );
            // Front glossy blue folder body
            fill_rounded_rect_gradient(
                canvas,
                size,
                size,
                fx,
                fy,
                fw,
                fh,
                5,
                rgb(68, 174, 255),
                rgb(0, 122, 255),
            );
        }
        "Phone" => {
            // Green gradient + white phone receiver
            fill_grad(canvas, rgb(52, 208, 88), rgb(32, 152, 56));
            let white = rgb(255, 255, 255);
            // Curved handset earpieces & bridge
            fill_circle(canvas, size, size, cx - 9, cy - 8, 5, white);
            fill_circle(canvas, size, size, cx + 8, cy + 9, 5, white);
            draw_line(
                canvas,
                size,
                size,
                cx - 9,
                cy - 6,
                cx + 6,
                cy + 9,
                6,
                white,
            );
            // Cut inner notch of receiver
            fill_circle(canvas, size, size, cx + 3, cy - 3, 5, rgb(42, 180, 72));
        }
        "Safari" => {
            // White border + blue dial with degree ticks + red/white compass needle
            canvas.fill(rgb(255, 255, 255));
            let dial_r = s * 43 / 100;
            // Blue gradient compass dial
            for py in (cy - dial_r)..=(cy + dial_r) {
                let t = (((py - (cy - dial_r)) as u32) * 256) / ((2 * dial_r).max(1) as u32);
                let col = lerp_color(rgb(42, 182, 255), rgb(0, 102, 235), t);
                for px in (cx - dial_r)..=(cx + dial_r) {
                    let dx = px - cx;
                    let dy = py - cy;
                    if dx * dx + dy * dy <= dial_r * dial_r {
                        put_pixel_blended(canvas, size, size, px, py, col);
                    }
                }
            }
            // White degree tick marks around compass dial
            for deg_idx in 0..24 {
                let ang = (deg_idx as f32) * (std::f32::consts::TAU / 24.0);
                let r_in = if deg_idx % 2 == 0 {
                    dial_r - 4
                } else {
                    dial_r - 2
                };
                let x0 = cx + ((r_in as f32) * ang.cos()).round() as i32;
                let y0 = cy + ((r_in as f32) * ang.sin()).round() as i32;
                let x1 = cx + (((dial_r - 1) as f32) * ang.cos()).round() as i32;
                let y1 = cy + (((dial_r - 1) as f32) * ang.sin()).round() as i32;
                draw_line(
                    canvas,
                    size,
                    size,
                    x0,
                    y0,
                    x1,
                    y1,
                    1,
                    rgba(255, 255, 255, 210),
                );
            }
            // NE Red needle + SW White needle
            let nr = dial_r - 3;
            fill_triangle(
                canvas,
                size,
                size,
                cx + nr - 2,
                cy - nr + 2,
                cx - 3,
                cy - 3,
                cx + 3,
                cy + 3,
                rgb(255, 59, 48),
            );
            fill_triangle(
                canvas,
                size,
                size,
                cx - nr + 2,
                cy + nr - 2,
                cx - 3,
                cy - 3,
                cx + 3,
                cy + 3,
                rgb(255, 255, 255),
            );
        }
        "Messages" => {
            // Green gradient + white chat bubble
            fill_grad(canvas, rgb(52, 208, 88), rgb(32, 152, 56));
            let white = rgb(255, 255, 255);
            let bw = s * 64 / 100;
            let bh = s * 50 / 100;
            fill_rounded_rect(
                canvas,
                size,
                size,
                cx - bw / 2,
                cy - bh / 2 - 2,
                bw,
                bh,
                bh / 2,
                white,
            );
            fill_triangle(
                canvas,
                size,
                size,
                cx - bw / 4,
                cy + bh / 2 - 4,
                cx - bw / 3 - 2,
                cy + bh / 2 + 4,
                cx - 2,
                cy + bh / 2 - 3,
                white,
            );
        }
        "Music" => {
            // Pink-to-red gradient (#FF2D55 -> #FF3B30) + white beamed eighth notes
            fill_grad(canvas, rgb(255, 45, 85), rgb(255, 59, 48));
            let white = rgb(255, 255, 255);
            // Left and right note heads
            fill_circle(canvas, size, size, cx - 8, cy + 9, 5, white);
            fill_circle(canvas, size, size, cx + 6, cy + 6, 5, white);
            // Vertical stems
            fill_rect(canvas, size, size, cx - 5, cy - 10, 3, 19, white);
            fill_rect(canvas, size, size, cx + 9, cy - 13, 3, 19, white);
            // Slanted top beam connecting both stems
            draw_line(
                canvas,
                size,
                size,
                cx - 4,
                cy - 9,
                cx + 11,
                cy - 12,
                5,
                white,
            );
        }
        _ => {
            // Clean custom installed app squircle icon (NO red MTL/IPA text badge!)
            fill_grad(canvas, rgb(88, 86, 214), rgb(48, 42, 148));
            let r = s / 3;
            fill_triangle(
                canvas,
                size,
                size,
                cx,
                cy - r,
                cx + r,
                cy + r - 2,
                cx - r,
                cy + r - 2,
                rgba(255, 255, 255, 230),
            );
        }
    }
}

/// Renders the authentic iOS 16.7.16 Lock Screen (`screen_state.locked == true`).
fn render_lock_screen(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    screen_state: &IosSimState,
) {
    draw_ios16_wallpaper(
        fb,
        fb_w,
        fb_h,
        screen_state.dark_mode,
        screen_state.frame_tick,
    );
    draw_iphone_x_status_bar(fb, fb_w, fb_h, false, screen_state);

    let w = fb_w as i32;
    let h = fb_h as i32;

    // Tiny crisp white padlock icon below notch
    let lock_y = (h * 56 / 1000).max(40);
    draw_ring(
        fb,
        fb_w,
        fb_h,
        w / 2,
        lock_y,
        3,
        5,
        rgb(250, 252, 255),
    );
    fill_rounded_rect(
        fb,
        fb_w,
        fb_h,
        w / 2 - 5,
        lock_y + 3,
        11,
        8,
        2,
        rgb(250, 252, 255),
    );

    // Date in clean white SF Pro Semibold: "Tuesday, October 20"
    let date_y = lock_y + 18;
    let date_text = "Tuesday, October 20";
    let date_w = measure_text_px(date_text, 1);
    draw_text_semibold(
        fb,
        fb_w,
        fb_h,
        (w - date_w) / 2,
        date_y,
        date_text,
        rgb(245, 248, 255),
        1,
        2,
        true,
    );

    // Large, smooth, heavy-rounded iOS 16 Lock Screen Clock "9:41" at 72px height
    let clock_h = (h * 72 / 800).max(54);
    let clock_y = date_y + 20;
    draw_ios16_lock_clock(
        fb,
        fb_w,
        fb_h,
        clock_y,
        "9:41",
        clock_h,
        rgba(248, 252, 255, 242),
    );

    // Bottom left & right: Authentic iOS 16 circular translucent frosted-glass
    // Flashlight and Camera quick-action buttons (46x46 circle)
    let btn_size = 46;
    let btn_r = btn_size / 2;
    let btn_y = h - 86;
    let fl_x = 32;
    let cam_x = w - 32 - btn_size;

    // Flashlight quick-action button (frosted glass blur + white/active flashlight icon)
    let fl_tint = if screen_state.flashlight_on {
        rgba(255, 255, 255, 235)
    } else {
        rgba(18, 24, 38, 110)
    };
    let fl_fg = if screen_state.flashlight_on {
        rgb(0, 122, 255)
    } else {
        rgb(255, 255, 255)
    };
    blur_rounded_rect(
        fb,
        fb_w,
        fb_h,
        fl_x,
        btn_y,
        btn_size,
        btn_size,
        btn_r,
        7,
        fl_tint,
        rgba(255, 255, 255, 45),
    );
    let fl_cx = fl_x + btn_r;
    let fl_cy = btn_y + btn_r;
    fill_rounded_rect(fb, fb_w, fb_h, fl_cx - 5, fl_cy - 10, 10, 4, 1, fl_fg);
    fill_rounded_rect(fb, fb_w, fb_h, fl_cx - 3, fl_cy - 5, 6, 15, 2, fl_fg);
    fill_circle(
        fb,
        fb_w,
        fb_h,
        fl_cx,
        fl_cy + 1,
        1,
        if screen_state.flashlight_on {
            rgb(255, 204, 0)
        } else {
            rgb(30, 36, 52)
        },
    );

    // Camera quick-action button (frosted glass blur + white camera icon)
    blur_rounded_rect(
        fb,
        fb_w,
        fb_h,
        cam_x,
        btn_y,
        btn_size,
        btn_size,
        btn_r,
        7,
        rgba(18, 24, 38, 110),
        rgba(255, 255, 255, 45),
    );
    let cam_cx = cam_x + btn_r;
    let cam_cy = btn_y + btn_r;
    fill_rounded_rect(
        fb,
        fb_w,
        fb_h,
        cam_cx - 4,
        cam_cy - 8,
        8,
        3,
        1,
        rgb(255, 255, 255),
    );
    fill_rounded_rect(
        fb,
        fb_w,
        fb_h,
        cam_cx - 9,
        cam_cy - 6,
        18,
        13,
        3,
        rgb(255, 255, 255),
    );
    fill_circle(fb, fb_w, fb_h, cam_cx, cam_cy + 1, 4, rgb(28, 36, 54));
    fill_circle(fb, fb_w, fb_h, cam_cx, cam_cy + 1, 2, rgb(255, 255, 255));

    // Subtle "Swipe up to open" above Home Indicator
    let prompt_y = h - 30;
    draw_text_centered(
        fb,
        fb_w,
        fb_h,
        prompt_y,
        "Swipe up to open",
        rgba(245, 248, 255, 220),
        1,
        2,
        true,
    );

    draw_home_indicator(fb, fb_w, fb_h, screen_state, false);
}

/// Renders the authentic iOS 16.7.16 iPhone X Home Screen (`4x5` stock icons + Search Pill + 4-icon Dock).
fn render_home_screen(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    screen_state: &IosSimState,
) {
    draw_ios16_wallpaper(
        fb,
        fb_w,
        fb_h,
        screen_state.dark_mode,
        screen_state.frame_tick,
    );
    draw_iphone_x_status_bar(fb, fb_w, fb_h, false, screen_state);

    let w = fb_w as i32;
    let h = fb_h as i32;

    // 20 Authentic Apple stock iOS 16 Home Screen apps (4 columns x 5 rows)
    const STOCK_APPS: [&str; 20] = [
        // Row 1
        "FaceTime",
        "Calendar",
        "Photos",
        "Camera",
        // Row 2
        "Mail",
        "Clock",
        "Maps",
        "Weather",
        // Row 3
        "Reminders",
        "Notes",
        "Stocks",
        "Books",
        // Row 4
        "App Store",
        "Podcasts",
        "TV",
        "Health",
        // Row 5
        "Home",
        "Wallet",
        "Settings",
        "Files",
    ];

    let mut icons: Vec<String> = STOCK_APPS.iter().map(|&s| s.to_string()).collect();

    // Include any user-installed .ipa apps (excluding synthetic "Sweet 3D Bench")
    for app in &screen_state.installed_apps {
        let name = app.display_name.trim();
        if !name.is_empty() && !name.eq_ignore_ascii_case("Sweet 3D Bench") {
            icons.push(name.to_string());
        }
    }

    let cols = 4i32;
    let grid_margin_x = 18;
    let usable_w = (w - 2 * grid_margin_x).max(200);
    let cell_w = usable_w / cols;
    let icon_size = (w * 54 / 360).clamp(44, 60);
    let has_row6 = icons.len() > 20;
    let grid_top = if has_row6 { 48 } else { 54 };
    let row_h = if has_row6 {
        icon_size + 24
    } else {
        icon_size + 30
    };

    let flyin = if screen_state.initialized {
        screen_state.home_flyin.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let touch_vx = screen_state.touch_x / 3;
    let touch_vy = screen_state.touch_y / 3;
    let pressing_home = screen_state.touch_down
        && !screen_state.edge_swipe_active
        && screen_state.active_app.is_none()
        && !screen_state.locked;
    let morphing_app = if screen_state.app_anim > 0.02 && screen_state.app_anim < 0.98 {
        screen_state.anim_app_name.as_deref()
    } else {
        None
    };

    for (i, label) in icons.iter().take(24).enumerate() {
        if morphing_app.map_or(false, |m| m.eq_ignore_ascii_case(label)) {
            continue;
        }
        let col = (i as i32) % cols;
        let row = (i as i32) / cols;
        let base_cx = grid_margin_x + col * cell_w + cell_w / 2;
        let base_iy = grid_top + row * row_h;

        // Staggered radial fly-in on Lock Screen unlock
        let dx_norm = (col as f32) - 1.5;
        let dy_norm = (row as f32) - 2.0;
        let dist = (dx_norm * dx_norm + dy_norm * dy_norm).sqrt();
        let delay = ((2.8 - dist).max(0.0) * 0.08).clamp(0.0, 0.32);
        let local_t = ((flyin - delay) / (1.0 - delay)).clamp(0.0, 1.0);
        let ease = 1.0 - (1.0 - local_t).powi(3);

        let radial_x = (dx_norm * 26.0 * (1.0 - ease)).round() as i32;
        let radial_y = (dy_norm * 32.0 * (1.0 - ease)).round() as i32;
        let cx = base_cx + radial_x;
        let cy = base_iy + icon_size / 2 + radial_y;

        let is_pressed = pressing_home
            && (touch_vx - base_cx).abs() <= icon_size / 2 + 4
            && (touch_vy - (base_iy + icon_size / 2)).abs() <= icon_size / 2 + 4;
        let press_scale = if is_pressed { 0.88f32 } else { 1.0f32 };
        let scale = (0.55 + 0.45 * ease) * press_scale;
        let cur_size = ((icon_size as f32) * scale).round().clamp(24.0, 64.0) as i32;
        let ix = cx - cur_size / 2;
        let iy = cy - cur_size / 2;

        render_squircle_icon(fb, fb_w, fb_h, ix, iy, cur_size, true, |canvas, cw, _ch| {
            draw_stock_icon_canvas(canvas, cw, label);
            if is_pressed {
                for px in canvas.iter_mut() {
                    let r = (((*px >> 16) & 0xFF) * 195) >> 8;
                    let g = (((*px >> 8) & 0xFF) * 195) >> 8;
                    let b = ((*px & 0xFF) * 195) >> 8;
                    *px = (*px & 0xFF00_0000) | (r << 16) | (g << 8) | b;
                }
            }
        });

        if ease > 0.35 {
            // Centered crisp white label underneath
            let lw = measure_text_px(label, 1);
            let lx = cx - lw / 2;
            let ly = base_iy + icon_size + 5 + radial_y;
            draw_text_shadow(
                fb,
                fb_w,
                fb_h,
                lx,
                ly,
                label,
                rgb(250, 252, 255),
                1,
                2,
            );
        }
    }

    // Authentic iOS 16 Frosted Glass Search Pill (74x24) centered above the Dock
    let dock_slide = ((1.0 - (1.0 - (1.0 - flyin).powi(3))) * 70.0).round() as i32;
    let pill_w = 74;
    let pill_h = 24;
    let pill_x = (w - pill_w) / 2;
    let pill_y = h - 142 + dock_slide;
    blur_rounded_rect(
        fb,
        fb_w,
        fb_h,
        pill_x,
        pill_y,
        pill_w,
        pill_h,
        12,
        8,
        rgba(255, 255, 255, 56),
        rgba(255, 255, 255, 42),
    );
    // Tiny magnifying glass icon + "Search" inside pill
    let mg_cx = pill_x + 15;
    let mg_cy = pill_y + 11;
    draw_ring(
        fb,
        fb_w,
        fb_h,
        mg_cx,
        mg_cy,
        3,
        4,
        rgba(255, 255, 255, 235),
    );
    draw_line(
        fb,
        fb_w,
        fb_h,
        mg_cx + 3,
        mg_cy + 3,
        mg_cx + 5,
        mg_cy + 5,
        1,
        rgba(255, 255, 255, 235),
    );
    draw_text_scaled(
        fb,
        fb_w,
        fb_h,
        pill_x + 24,
        pill_y + 7,
        "Search",
        rgb(250, 252, 255),
        1,
        2,
    );

    // Authentic iOS 16 Frosted Glass Dock (y = 696..776 at 360x800, corner radius 26px)
    let dock_margin = 12;
    let dock_w = w - 2 * dock_margin;
    let dock_h = 80;
    let dock_y = h - 104 + dock_slide;
    blur_rounded_rect(
        fb,
        fb_w,
        fb_h,
        dock_margin,
        dock_y,
        dock_w,
        dock_h,
        26,
        12,
        rgba(240, 246, 255, 64),
        rgba(255, 255, 255, 48),
    );

    // Exact 4 iPhone X default Dock icons (no labels inside the dock!)
    const DOCK_ICONS: [&str; 4] = ["Phone", "Safari", "Messages", "Music"];
    let dock_cell_w = dock_w / 4;
    for (i, &dock_kind) in DOCK_ICONS.iter().enumerate() {
        if morphing_app.map_or(false, |m| m.eq_ignore_ascii_case(dock_kind)) {
            continue;
        }
        let cx = dock_margin + (i as i32) * dock_cell_w + dock_cell_w / 2;
        let base_iy = dock_y + (dock_h - icon_size) / 2;
        let is_pressed = pressing_home
            && (touch_vx - cx).abs() <= icon_size / 2 + 4
            && (touch_vy - (base_iy + icon_size / 2)).abs() <= icon_size / 2 + 4;
        let cur_size = if is_pressed {
            (icon_size * 88) / 100
        } else {
            icon_size
        };
        let ix = cx - cur_size / 2;
        let iy = base_iy + (icon_size - cur_size) / 2;
        render_squircle_icon(fb, fb_w, fb_h, ix, iy, cur_size, false, |canvas, cw, _ch| {
            draw_stock_icon_canvas(canvas, cw, dock_kind);
            if is_pressed {
                for px in canvas.iter_mut() {
                    let r = (((*px >> 16) & 0xFF) * 195) >> 8;
                    let g = (((*px >> 8) & 0xFF) * 195) >> 8;
                    let b = ((*px & 0xFF) * 195) >> 8;
                    *px = (*px & 0xFF00_0000) | (r << 16) | (g << 8) | b;
                }
            }
        });
    }

    if screen_state.spotlight_open {
        crate::apps::render_spotlight_overlay(fb, fb_w, fb_h, screen_state);
    }

    draw_home_indicator(fb, fb_w, fb_h, screen_state, false);
}

/// Renders the real-time 3D rotating cube + starfield game benchmark using `GPU_CMD_TRIANGLE`.
fn render_3d_game_benchmark(
    engine: &mut SweetGpuEngine,
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    screen_state: &IosSimState,
    _app_name: &str,
) {
    let w = fb_w as i32;
    let h = fb_h as i32;
    let tick = screen_state.frame_tick;

    for y in 0..fb_h {
        let t = ((y * 256) / fb_h.max(1)) as u32;
        let row_c = lerp_color(rgb(8, 10, 26), rgb(22, 12, 48), t);
        fb[y * fb_w..(y + 1) * fb_w].fill(row_c);
    }

    for i in 0..42u64 {
        let seed = i.wrapping_mul(1103515245).wrapping_add(12345);
        let sx = ((seed >> 16) as i32).abs() % w.max(1);
        let speed = ((i % 4) + 1) as u64;
        let sy = (((seed >> 8) as u64 + tick * speed * 3) % (h.max(1) as u64)) as i32;
        fill_circle(
            fb,
            fb_w,
            fb_h,
            sx,
            sy,
            if i % 5 == 0 { 2 } else { 1 },
            rgb(220, 240, 255),
        );
    }

    let base_angle = (tick as f32) * 0.055;
    let ay = base_angle;
    let ax = base_angle * 0.72 + 0.45;
    let (sin_y, cos_y) = ay.sin_cos();
    let (sin_x, cos_x) = ax.sin_cos();

    let raw_verts: [(f32, f32, f32); 8] = [
        (-1.0, -1.0, -1.0),
        (1.0, -1.0, -1.0),
        (1.0, 1.0, -1.0),
        (-1.0, 1.0, -1.0),
        (-1.0, -1.0, 1.0),
        (1.0, -1.0, 1.0),
        (1.0, 1.0, 1.0),
        (-1.0, 1.0, 1.0),
    ];

    let cx = (w / 2) as f32;
    let cy = (h / 2) as f32;
    let fov_scale = (w.min(h) as f32) * 0.82;

    let mut proj: [(i32, i32, f32); 8] = [(0, 0, 0.0); 8];
    for (i, &(vx, vy, vz)) in raw_verts.iter().enumerate() {
        let rx = vx * cos_y + vz * sin_y;
        let rz0 = -vx * sin_y + vz * cos_y;
        let ry = vy * cos_x - rz0 * sin_x;
        let rz = vy * sin_x + rz0 * cos_x;

        let z_cam = rz + 4.2;
        let inv_z = 1.0 / z_cam.max(0.5);
        let sx = (cx + rx * fov_scale * inv_z) as i32;
        let sy = (cy + ry * fov_scale * inv_z) as i32;
        proj[i] = (sx, sy, z_cam);
    }

    let faces: [([usize; 4], (u8, u8, u8)); 6] = [
        ([0, 1, 2, 3], (0, 220, 255)),
        ([5, 4, 7, 6], (255, 65, 145)),
        ([4, 0, 3, 7], (130, 80, 255)),
        ([1, 5, 6, 2], (45, 235, 135)),
        ([3, 2, 6, 7], (255, 195, 45)),
        ([4, 5, 1, 0], (70, 145, 255)),
    ];

    let mut face_order: [(usize, f32); 6] = [(0, 0.0); 6];
    for (fi, (idx, _)) in faces.iter().enumerate() {
        let avg_z = (proj[idx[0]].2 + proj[idx[1]].2 + proj[idx[2]].2 + proj[idx[3]].2) * 0.25;
        face_order[fi] = (fi, avg_z);
    }
    face_order.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    for &(fi, avg_z) in &face_order {
        let (idx, (br, bg, bb)) = faces[fi];
        let (x0, y0, _) = proj[idx[0]];
        let (x1, y1, _) = proj[idx[1]];
        let (x2, y2, _) = proj[idx[2]];
        let (x3, y3, _) = proj[idx[3]];

        let shade = ((5.4 - avg_z).clamp(0.45, 1.15) * 220.0) as u32;
        let r1 = ((br as u32 * shade) >> 8).min(255) as u8;
        let g1 = ((bg as u32 * shade) >> 8).min(255) as u8;
        let b1 = ((bb as u32 * shade) >> 8).min(255) as u8;
        let c1 = rgb(r1, g1, b1);
        let c2 = rgb(
            (r1 as u16 * 88 / 100) as u8,
            (g1 as u16 * 88 / 100) as u8,
            (b1 as u16 * 88 / 100) as u8,
        );

        let cmd_t1 = GpuCommand::triangle(x0, y0, x1, y1, x2, y2, c1);
        let cmd_t2 = GpuCommand::triangle(x0, y0, x2, y2, x3, y3, c2);
        engine.execute_command_on_u32(cmd_t1, fb, fb_w, fb_h);
        engine.execute_command_on_u32(cmd_t2, fb, fb_w, fb_h);
    }

    for s in 0..4usize {
        let orb_ang = base_angle * 1.4 + (s as f32) * std::f32::consts::FRAC_PI_2;
        let ox = cx as i32 + (orb_ang.cos() * (w as f32 * 0.34)) as i32;
        let oy = cy as i32 + (orb_ang.sin() * (h as f32 * 0.10)) as i32;
        let shard_cmd = GpuCommand {
            opcode: GPU_CMD_TRIANGLE,
            arg0: GpuCommand::pack_xy(ox, oy - 9),
            arg1: GpuCommand::pack_xy(ox + 8, oy + 7),
            arg2: GpuCommand::pack_xy(ox - 8, oy + 7),
            arg3: 0,
            color: rgb(80, 245, 255),
        };
        engine.execute_command_on_u32(shard_cmd, fb, fb_w, fb_h);
    }

    draw_iphone_x_status_bar(fb, fb_w, fb_h, false, screen_state);
    draw_home_indicator(fb, fb_w, fb_h, screen_state, false);
}

/// Renders a single app buffer (`app_name`) into `dst` (`fb_w x fb_h`).
fn render_app_buffer(
    engine: &mut SweetGpuEngine,
    dst: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    screen_state: &IosSimState,
    app_name: &str,
) {
    if app_name.eq_ignore_ascii_case("Sweet 3D Bench")
        || screen_state
            .installed_apps
            .iter()
            .any(|a| a.display_name == app_name || a.bundle_id == app_name)
    {
        render_3d_game_benchmark(engine, dst, fb_w, fb_h, screen_state, app_name);
    } else {
        crate::apps::render_active_app(dst, fb_w, fb_h, screen_state, app_name);
        engine.stats.draw_calls += 18;
    }
}

/// Top-level entry point called by `SweetGpuEngine::render_ios_springboard_frame`.
pub fn render_springboard(
    engine: &mut SweetGpuEngine,
    fb_w: usize,
    fb_h: usize,
    dst_bgra: &mut [u32],
    screen_state: &IosSimState,
) {
    if fb_w == 0 || fb_h == 0 || dst_bgra.len() < fb_w * fb_h {
        return;
    }
    let fb = &mut dst_bgra[..fb_w * fb_h];

    let lock_p = if !screen_state.locked && screen_state.lock_anim >= 0.999 {
        0.0
    } else if screen_state.initialized {
        screen_state.lock_anim.clamp(0.0, 1.0)
    } else if screen_state.locked {
        1.0
    } else {
        0.0
    };

    let app_p = if screen_state.active_app.is_some() && screen_state.app_anim <= 0.001 {
        1.0
    } else if screen_state.initialized {
        screen_state.app_anim.clamp(0.0, 1.0)
    } else if screen_state.active_app.is_some() {
        1.0
    } else {
        0.0
    };

    if lock_p >= 0.995 {
        render_lock_screen(fb, fb_w, fb_h, screen_state);
        engine.stats.draw_calls += 8;
    } else if app_p >= 0.995 && screen_state.active_app.is_some() {
        if let Some(ref app_name) = screen_state.active_app {
            render_app_buffer(engine, fb, fb_w, fb_h, screen_state, app_name);
        }
    } else {
        // Render Home Screen base layer
        render_home_screen(fb, fb_w, fb_h, screen_state);
        engine.stats.draw_calls += 24;

        // If an app is morphing open or closing back into its icon (0.005 < app_p < 0.995)
        let morph_name = screen_state
            .active_app
            .as_deref()
            .or(screen_state.anim_app_name.as_deref());
        if app_p > 0.005 {
            if let Some(app_name) = morph_name {
                // Dim Home Screen background proportionally to app_p
                let dim = (256.0 - app_p * 78.0).clamp(160.0, 256.0) as u32;
                for px in fb.iter_mut() {
                    let r = (((*px >> 16) & 0xFF) * dim) >> 8;
                    let g = (((*px >> 8) & 0xFF) * dim) >> 8;
                    let b = ((*px & 0xFF) * dim) >> 8;
                    *px = 0xFF00_0000 | (r << 16) | (g << 8) | b;
                }

                let mut app_buf = vec![0u32; fb_w * fb_h];
                render_app_buffer(engine, &mut app_buf, fb_w, fb_h, screen_state, app_name);

                let (ix, iy, iw, ih) = if screen_state.anim_icon_rect.2 > 1.0 {
                    screen_state.anim_icon_rect
                } else {
                    screen_state.icon_rect_for_app(app_name)
                };

                let (card_x, card_y, card_w, card_h) =
                    if screen_state.touch_down && screen_state.edge_swipe_active {
                        let cw = (fb_w as f32) * app_p;
                        let ch = (fb_h as f32) * app_p;
                        let finger_dx = ((screen_state.touch_x / 3) - (fb_w as i32 / 2)) as f32 * 0.35;
                        let cx = ((fb_w as f32) - cw) * 0.5 + finger_dx;
                        let cy = ((screen_state.touch_y / 3) as f32 - ch * 0.88)
                            .clamp(24.0, (fb_h as f32) - ch - 28.0);
                        (cx, cy, cw, ch)
                    } else {
                        let ease = 1.0 - (1.0 - app_p).powi(3);
                        let cx = ix + (0.0 - ix) * ease;
                        let cy = iy + (0.0 - iy) * ease;
                        let cw = iw + ((fb_w as f32) - iw) * ease;
                        let ch = ih + ((fb_h as f32) - ih) * ease;
                        (cx, cy, cw, ch)
                    };

                let x0 = card_x.round() as i32;
                let y0 = card_y.round() as i32;
                let cw = card_w.round().max(16.0) as i32;
                let ch = card_h.round().max(16.0) as i32;
                let rad = (13.0 + 18.0 * (app_p * std::f32::consts::PI).sin()).round() as i32;

                // Soft drop shadow beneath floating card
                fill_rounded_rect(
                    fb,
                    fb_w,
                    fb_h,
                    x0 - 3,
                    y0 + 5,
                    cw + 6,
                    ch + 6,
                    rad + 3,
                    rgba(0, 0, 0, 110),
                );

                // Prepare icon canvas for smooth icon <-> app content crossfade at small card sizes
                let icon_sz = 64usize;
                let mut icon_canvas = vec![0u32; icon_sz * icon_sz];
                draw_stock_icon_canvas(&mut icon_canvas, icon_sz, app_name);
                let icon_blend = ((0.32 - app_p) / 0.32).clamp(0.0, 1.0);
                let icon_blend_u32 = (icon_blend * 256.0) as u32;

                let r_clamped = rad.clamp(4, cw.min(ch) / 2);
                let r2 = r_clamped * r_clamped;
                for dy in 0..ch {
                    let py = y0 + dy;
                    if py < 0 || py >= fb_h as i32 {
                        continue;
                    }
                    let mut inset = 0;
                    if dy < r_clamped {
                        let cy_d = r_clamped - 1 - dy;
                        while inset < r_clamped
                            && (r_clamped - 1 - inset) * (r_clamped - 1 - inset) + cy_d * cy_d > r2
                        {
                            inset += 1;
                        }
                    } else if dy >= ch - r_clamped {
                        let cy_d = dy - (ch - r_clamped);
                        while inset < r_clamped
                            && (r_clamped - 1 - inset) * (r_clamped - 1 - inset) + cy_d * cy_d > r2
                        {
                            inset += 1;
                        }
                    }
                    let sy = ((dy as usize) * fb_h / (ch as usize)).min(fb_h - 1);
                    let isy = ((dy as usize) * icon_sz / (ch as usize)).min(icon_sz - 1);
                    for dx in inset..(cw - inset) {
                        let px = x0 + dx;
                        if px < 0 || px >= fb_w as i32 {
                            continue;
                        }
                        let sx = ((dx as usize) * fb_w / (cw as usize)).min(fb_w - 1);
                        let mut col = app_buf[sy * fb_w + sx];
                        if icon_blend_u32 > 0 {
                            let isx = ((dx as usize) * icon_sz / (cw as usize)).min(icon_sz - 1);
                            let ic = icon_canvas[isy * icon_sz + isx];
                            col = lerp_color(col, ic, icon_blend_u32);
                        }
                        fb[(py as usize) * fb_w + (px as usize)] = col | 0xFF00_0000;
                    }
                }
            }
        }

        // If Lock Screen curtain is lifting or dropping (0.005 < lock_p < 0.995)
        if lock_p > 0.005 {
            let mut lock_buf = vec![0u32; fb_w * fb_h];
            render_lock_screen(&mut lock_buf, fb_w, fb_h, screen_state);
            let slide_y = (((1.0 - lock_p) * (fb_h as f32)).round() as i32).clamp(0, fb_h as i32);
            let alpha = (lock_p * 255.0).clamp(0.0, 255.0) as u32;
            for dst_y in 0..((fb_h as i32) - slide_y) {
                let src_y = (dst_y + slide_y) as usize;
                let dst_row = (dst_y as usize) * fb_w;
                let src_row = src_y * fb_w;
                for x in 0..fb_w {
                    let bg = fb[dst_row + x];
                    let fg = lock_buf[src_row + x];
                    fb[dst_row + x] = lerp_color(bg, fg, alpha);
                }
            }
        }
    }

    // Apply screen brightness dimming if brightness < 100
    if screen_state.initialized && screen_state.brightness < 100 {
        let factor = (screen_state.brightness as u32).clamp(25, 100);
        for px in fb.iter_mut() {
            let r = (((*px >> 16) & 0xFF) * factor) / 100;
            let g = (((*px >> 8) & 0xFF) * factor) / 100;
            let b = ((*px & 0xFF) * factor) / 100;
            *px = 0xFF00_0000 | (r << 16) | (g << 8) | b;
        }
    }
}
