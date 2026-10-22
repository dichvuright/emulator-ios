//! Authentic Apple iOS 16.7.16 (20H392) Typography Engine.
//! On Windows, loads genuine Apple `SFUI.ttf` (`System Font Regular` / `System Font Semibold`),
//! `SFUIRounded.ttf` (`.SF UI Rounded`), and `SFUIMono.ttf` (`.SF UI Mono`) extracted from the
//! official iOS 16.7.16 IPSW via Win32 GDI `AddFontResourceExW` + anti-aliased DIB rasterization
//! with an O(1) in-memory alpha coverage cache.

use crate::raster::{blend_argb, put_pixel_blended, rgba};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[derive(Clone)]
pub struct CachedTextMask {
    pub width: i32,
    pub height: i32,
    pub alpha: Vec<u8>,
}

static TEXT_MASK_CACHE: OnceLock<Mutex<HashMap<(String, i32, u8), CachedTextMask>>> = OnceLock::new();

#[cfg(windows)]
mod win32_font {
    use super::CachedTextMask;
    use std::path::{Path, PathBuf};
    use std::ptr;
    use std::sync::OnceLock;

    #[repr(C)]
    struct BitmapInfoHeader {
        bi_size: u32,
        bi_width: i32,
        bi_height: i32,
        bi_planes: u16,
        bi_bit_count: u16,
        bi_compression: u32,
        bi_size_image: u32,
        bi_x_pels_per_meter: i32,
        bi_y_pels_per_meter: i32,
        bi_clr_used: u32,
        bi_clr_important: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct Size {
        cx: i32,
        cy: i32,
    }

    #[link(name = "gdi32")]
    extern "system" {
        fn AddFontResourceExW(name: *const u16, fl: u32, res: *mut core::ffi::c_void) -> i32;
        fn CreateCompatibleDC(hdc: isize) -> isize;
        fn DeleteDC(hdc: isize) -> i32;
        fn CreateDIBSection(
            hdc: isize,
            pbmi: *const BitmapInfoHeader,
            usage: u32,
            ppv_bits: *mut *mut u32,
            h_section: isize,
            offset: u32,
        ) -> isize;
        fn SelectObject(hdc: isize, h: isize) -> isize;
        fn DeleteObject(ho: isize) -> i32;
        fn CreateFontW(
            c_height: i32,
            c_width: i32,
            c_escapement: i32,
            c_orientation: i32,
            c_weight: i32,
            b_italic: u32,
            b_underline: u32,
            b_strike_out: u32,
            i_char_set: u32,
            i_out_precision: u32,
            i_clip_precision: u32,
            i_quality: u32,
            i_pitch_and_family: u32,
            psz_face_name: *const u16,
        ) -> isize;
        fn SetBkMode(hdc: isize, mode: i32) -> i32;
        fn SetTextColor(hdc: isize, color: u32) -> u32;
        fn GetTextExtentPoint32W(hdc: isize, lp_string: *const u16, c: i32, psizl: *mut Size) -> i32;
        fn TextOutW(hdc: isize, x: i32, y: i32, lp_string: *const u16, c: i32) -> i32;
        fn GdiFlush() -> i32;
    }

    static FONTS_INITIALIZED: OnceLock<bool> = OnceLock::new();

    fn find_fonts_dirs() -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        let rel_candidates = [
            "emu_workspace/apple_assets/fonts",
            "../emu_workspace/apple_assets/fonts",
            "../../emu_workspace/apple_assets/fonts",
            "research/ipsw-x/os_decompressed/System/Library/Fonts/CoreUI",
            "research/ipsw-x/os_decompressed/System/Library/Fonts/Core",
        ];
        for r in &rel_candidates {
            let p = PathBuf::from(r);
            if p.exists() {
                dirs.push(p);
            }
        }
        if let Ok(exe) = std::env::current_exe() {
            let mut cur: Option<&Path> = exe.parent();
            for _ in 0..5 {
                if let Some(d) = cur {
                    let cand = d.join("emu_workspace").join("apple_assets").join("fonts");
                    if cand.exists() {
                        dirs.push(cand);
                    }
                    cur = d.parent();
                }
            }
        }
        dirs
    }

    pub fn ensure_apple_fonts_loaded() -> bool {
        *FONTS_INITIALIZED.get_or_init(|| {
            let mut loaded_any = false;
            let font_files = [
                "SFUI.ttf",
                "SFUIRounded.ttf",
                "SFUIMono.ttf",
                "AvenirNext.ttc",
                "HelveticaNeue.ttc",
            ];
            for dir in find_fonts_dirs() {
                for fname in &font_files {
                    let fpath = dir.join(fname);
                    if fpath.exists() {
                        if let Ok(abs) = fpath.canonicalize() {
                            let s = abs.to_string_lossy();
                            let clean = s.strip_prefix(r"\\?\").unwrap_or(&s);
                            let mut wide: Vec<u16> = clean.encode_utf16().collect();
                            wide.push(0);
                            let added = unsafe { AddFontResourceExW(wide.as_ptr(), 0x10, ptr::null_mut()) };
                            if added > 0 {
                                loaded_any = true;
                            }
                        }
                    }
                }
            }
            loaded_any
        })
    }

    pub fn rasterize_truetype_mask(text: &str, px_height: i32, style: u8) -> Option<CachedTextMask> {
        if text.is_empty() || px_height <= 0 {
            return Some(CachedTextMask {
                width: 0,
                height: px_height.max(1),
                alpha: Vec::new(),
            });
        }
        ensure_apple_fonts_loaded();

        // Style 0 = SF Pro Regular, 1 = SF Pro Semibold, 2 = SF Pro Rounded Heavy (Lock Clock), 3 = SF Mono
        let (face_name, weight) = match style {
            1 => ("System Font Semibold", 600),
            2 => (".SF UI Rounded", 700),
            3 => (".SF UI Mono", 500),
            _ => ("System Font Regular", 400),
        };
        let mut face_wide: Vec<u16> = face_name.encode_utf16().collect();
        face_wide.push(0);
        let text_wide: Vec<u16> = text.encode_utf16().collect();

        unsafe {
            let hdc = CreateCompatibleDC(0);
            if hdc == 0 {
                return None;
            }
            // Render directly at 1x so GDI TrueType grid-fitting snaps horizontal/vertical stems to exact pixels
            let hfont = CreateFontW(
                -px_height,
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                1, // DEFAULT_CHARSET
                0,
                0,
                4, // ANTIALIASED_QUALITY (grid-fitted grayscale anti-aliasing)
                0,
                face_wide.as_ptr(),
            );
            if hfont == 0 {
                DeleteDC(hdc);
                return None;
            }
            let old_font = SelectObject(hdc, hfont);

            let mut sz = Size::default();
            GetTextExtentPoint32W(hdc, text_wide.as_ptr(), text_wide.len() as i32, &mut sz);
            let bmp_w = (sz.cx + 4).max(4);
            let bmp_h = (sz.cy + 2).max(4);

            let bmi = BitmapInfoHeader {
                bi_size: core::mem::size_of::<BitmapInfoHeader>() as u32,
                bi_width: bmp_w,
                bi_height: -bmp_h, // top-down DIB
                bi_planes: 1,
                bi_bit_count: 32,
                bi_compression: 0,
                bi_size_image: 0,
                bi_x_pels_per_meter: 0,
                bi_y_pels_per_meter: 0,
                bi_clr_used: 0,
                bi_clr_important: 0,
            };
            let mut bits_ptr: *mut u32 = ptr::null_mut();
            let hbmp = CreateDIBSection(hdc, &bmi, 0, &mut bits_ptr, 0, 0);
            if hbmp == 0 || bits_ptr.is_null() {
                SelectObject(hdc, old_font);
                DeleteObject(hfont);
                DeleteDC(hdc);
                return None;
            }
            let old_bmp = SelectObject(hdc, hbmp);

            let total_px = (bmp_w * bmp_h) as usize;
            let slice = core::slice::from_raw_parts_mut(bits_ptr, total_px);
            slice.fill(0);

            SetBkMode(hdc, 1); // TRANSPARENT
            SetTextColor(hdc, 0x00FF_FFFF); // Pure white
            TextOutW(hdc, 0, 0, text_wide.as_ptr(), text_wide.len() as i32);
            GdiFlush();

            let out_w = bmp_w.max(1);
            let out_h = bmp_h.max(1);
            let mut alpha = vec![0u8; (out_w * out_h) as usize];

            for i in 0..(out_w * out_h) as usize {
                let cov = slice[i] & 0xFF;
                alpha[i] = ((cov * 310) >> 8).min(255) as u8;
            }

            SelectObject(hdc, old_bmp);
            DeleteObject(hbmp);
            SelectObject(hdc, old_font);
            DeleteObject(hfont);
            DeleteDC(hdc);

            Some(CachedTextMask {
                width: sz.cx.max(1),
                height: out_h,
                alpha,
            })
        }
    }
}

#[inline]
fn scale_to_px_height(scale_x: i32, scale_y: i32) -> i32 {
    if scale_x <= 1 && scale_y <= 1 {
        13
    } else if scale_x == 1 && scale_y == 2 {
        15
    } else if scale_x == 2 && scale_y == 2 {
        22
    } else if scale_x == 2 && scale_y == 4 {
        26
    } else {
        (8 * scale_y).max(13)
    }
}

pub fn get_or_rasterize_mask(text: &str, px_height: i32, style: u8) -> Option<CachedTextMask> {
    let cache_mutex = TEXT_MASK_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = (text.to_string(), px_height, style);
    if let Ok(guard) = cache_mutex.lock() {
        if let Some(found) = guard.get(&key) {
            return Some(found.clone());
        }
    }
    #[cfg(windows)]
    {
        if let Some(mask) = win32_font::rasterize_truetype_mask(text, px_height, style) {
            if let Ok(mut guard) = cache_mutex.lock() {
                if guard.len() > 2048 {
                    guard.clear();
                }
                guard.insert(key, mask.clone());
            }
            return Some(mask);
        }
    }
    None
}

fn blit_text_mask(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    x: i32,
    y: i32,
    mask: &CachedTextMask,
    color: u32,
) {
    if mask.alpha.is_empty() || fb_w == 0 || fb_h == 0 {
        return;
    }
    let base_a = ((color >> 24) & 0xFF).max(if color != 0 { 0xFF } else { 0 });
    let rgb_part = color & 0x00FF_FFFF;
    let stride = (mask.alpha.len() as i32) / mask.height.max(1);

    for dy in 0..mask.height {
        let py = y + dy;
        if py < 0 || py >= fb_h as i32 {
            continue;
        }
        let fb_row = (py as usize) * fb_w;
        let mask_row = (dy * stride) as usize;
        for dx in 0..stride {
            let px = x + dx;
            if px < 0 || px >= fb_w as i32 {
                continue;
            }
            let cov = mask.alpha[mask_row + (dx as usize)] as u32;
            if cov > 0 {
                let eff_a = (base_a * cov) >> 8;
                if eff_a > 0 {
                    let idx = fb_row + (px as usize);
                    fb[idx] = blend_argb(fb[idx], (eff_a << 24) | rgb_part);
                }
            }
        }
    }
}

/// Returns an 8x8 row bitmap (MSB = leftmost pixel) for printable ASCII `c`.
#[inline]
pub const fn glyph_8x8(c: u8) -> [u8; 8] {
    match c {
        b' ' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        b'!' => [0x18, 0x18, 0x18, 0x18, 0x18, 0x00, 0x18, 0x00],
        b'"' => [0x66, 0x66, 0x24, 0x00, 0x00, 0x00, 0x00, 0x00],
        b'#' => [0x6C, 0x6C, 0xFE, 0x6C, 0xFE, 0x6C, 0x6C, 0x00],
        b'$' => [0x18, 0x3E, 0x60, 0x3C, 0x06, 0x7C, 0x18, 0x00],
        b'%' => [0x00, 0xC6, 0xCC, 0x18, 0x30, 0x66, 0xC6, 0x00],
        b'&' => [0x38, 0x6C, 0x38, 0x76, 0xDC, 0xCC, 0x76, 0x00],
        b'\'' => [0x18, 0x18, 0x30, 0x00, 0x00, 0x00, 0x00, 0x00],
        b'(' => [0x0C, 0x18, 0x30, 0x30, 0x30, 0x18, 0x0C, 0x00],
        b')' => [0x30, 0x18, 0x0C, 0x0C, 0x0C, 0x18, 0x30, 0x00],
        b'*' => [0x00, 0x66, 0x3C, 0xFF, 0x3C, 0x66, 0x00, 0x00],
        b'+' => [0x00, 0x18, 0x18, 0x7E, 0x18, 0x18, 0x00, 0x00],
        b',' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x30],
        b'-' => [0x00, 0x00, 0x00, 0x7E, 0x00, 0x00, 0x00, 0x00],
        b'.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x00],
        b'/' => [0x06, 0x0C, 0x18, 0x30, 0x60, 0xC0, 0x80, 0x00],
        b'0' => [0x3C, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x00],
        b'1' => [0x18, 0x38, 0x18, 0x18, 0x18, 0x18, 0x3C, 0x00],
        b'2' => [0x3C, 0x66, 0x06, 0x1C, 0x30, 0x60, 0x7E, 0x00],
        b'3' => [0x3C, 0x66, 0x06, 0x1C, 0x06, 0x66, 0x3C, 0x00],
        b'4' => [0x0C, 0x1C, 0x3C, 0x6C, 0x7E, 0x0C, 0x0C, 0x00],
        b'5' => [0x7E, 0x60, 0x7C, 0x06, 0x06, 0x66, 0x3C, 0x00],
        b'6' => [0x1C, 0x30, 0x60, 0x7C, 0x66, 0x66, 0x3C, 0x00],
        b'7' => [0x7E, 0x06, 0x0C, 0x18, 0x30, 0x30, 0x30, 0x00],
        b'8' => [0x3C, 0x66, 0x66, 0x3C, 0x66, 0x66, 0x3C, 0x00],
        b'9' => [0x3C, 0x66, 0x66, 0x3E, 0x06, 0x0C, 0x38, 0x00],
        b':' => [0x00, 0x18, 0x18, 0x00, 0x00, 0x18, 0x18, 0x00],
        b';' => [0x00, 0x18, 0x18, 0x00, 0x00, 0x18, 0x18, 0x30],
        b'<' => [0x0C, 0x18, 0x30, 0x60, 0x30, 0x18, 0x0C, 0x00],
        b'=' => [0x00, 0x00, 0x7E, 0x00, 0x00, 0x7E, 0x00, 0x00],
        b'>' => [0x60, 0x30, 0x18, 0x0C, 0x18, 0x30, 0x60, 0x00],
        b'?' => [0x3C, 0x66, 0x06, 0x1C, 0x18, 0x00, 0x18, 0x00],
        b'@' => [0x3C, 0x66, 0x6E, 0x6E, 0x60, 0x62, 0x3C, 0x00],
        b'A' => [0x18, 0x3C, 0x66, 0x66, 0x7E, 0x66, 0x66, 0x00],
        b'B' => [0x7C, 0x66, 0x66, 0x7C, 0x66, 0x66, 0x7C, 0x00],
        b'C' => [0x3C, 0x66, 0x60, 0x60, 0x60, 0x66, 0x3C, 0x00],
        b'D' => [0x78, 0x6C, 0x66, 0x66, 0x66, 0x6C, 0x78, 0x00],
        b'E' => [0x7E, 0x60, 0x60, 0x7C, 0x60, 0x60, 0x7E, 0x00],
        b'F' => [0x7E, 0x60, 0x60, 0x7C, 0x60, 0x60, 0x60, 0x00],
        b'G' => [0x3C, 0x66, 0x60, 0x6E, 0x66, 0x66, 0x3E, 0x00],
        b'H' => [0x66, 0x66, 0x66, 0x7E, 0x66, 0x66, 0x66, 0x00],
        b'I' => [0x3C, 0x18, 0x18, 0x18, 0x18, 0x18, 0x3C, 0x00],
        b'J' => [0x1E, 0x0C, 0x0C, 0x0C, 0x6C, 0x6C, 0x38, 0x00],
        b'K' => [0x66, 0x6C, 0x78, 0x70, 0x78, 0x6C, 0x66, 0x00],
        b'L' => [0x60, 0x60, 0x60, 0x60, 0x60, 0x60, 0x7E, 0x00],
        b'M' => [0xC6, 0xEE, 0xFE, 0xD6, 0xC6, 0xC6, 0xC6, 0x00],
        b'N' => [0x66, 0x76, 0x7E, 0x7E, 0x6E, 0x66, 0x66, 0x00],
        b'O' => [0x3C, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x00],
        b'P' => [0x7C, 0x66, 0x66, 0x7C, 0x60, 0x60, 0x60, 0x00],
        b'Q' => [0x3C, 0x66, 0x66, 0x66, 0x6E, 0x3C, 0x0E, 0x00],
        b'R' => [0x7C, 0x66, 0x66, 0x7C, 0x6C, 0x66, 0x66, 0x00],
        b'S' => [0x3C, 0x66, 0x60, 0x3C, 0x06, 0x66, 0x3C, 0x00],
        b'T' => [0x7E, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x00],
        b'U' => [0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x00],
        b'V' => [0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x18, 0x00],
        b'W' => [0xC6, 0xC6, 0xC6, 0xD6, 0xFE, 0xEE, 0xC6, 0x00],
        b'X' => [0x66, 0x66, 0x3C, 0x18, 0x3C, 0x66, 0x66, 0x00],
        b'Y' => [0x66, 0x66, 0x66, 0x3C, 0x18, 0x18, 0x18, 0x00],
        b'Z' => [0x7E, 0x06, 0x0C, 0x18, 0x30, 0x60, 0x7E, 0x00],
        b'[' => [0x3C, 0x30, 0x30, 0x30, 0x30, 0x30, 0x3C, 0x00],
        b'\\' => [0xC0, 0x60, 0x30, 0x18, 0x0C, 0x06, 0x02, 0x00],
        b']' => [0x3C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x3C, 0x00],
        b'^' => [0x10, 0x38, 0x6C, 0xC6, 0x00, 0x00, 0x00, 0x00],
        b'_' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF],
        b'`' => [0x30, 0x18, 0x0C, 0x00, 0x00, 0x00, 0x00, 0x00],
        b'a' => [0x00, 0x00, 0x3C, 0x06, 0x3E, 0x66, 0x3E, 0x00],
        b'b' => [0x60, 0x60, 0x7C, 0x66, 0x66, 0x66, 0x7C, 0x00],
        b'c' => [0x00, 0x00, 0x3C, 0x66, 0x60, 0x66, 0x3C, 0x00],
        b'd' => [0x06, 0x06, 0x3E, 0x66, 0x66, 0x66, 0x3E, 0x00],
        b'e' => [0x00, 0x00, 0x3C, 0x66, 0x7E, 0x60, 0x3C, 0x00],
        b'f' => [0x1C, 0x36, 0x30, 0x78, 0x30, 0x30, 0x30, 0x00],
        b'g' => [0x00, 0x00, 0x3E, 0x66, 0x66, 0x3E, 0x06, 0x3C],
        b'h' => [0x60, 0x60, 0x7C, 0x66, 0x66, 0x66, 0x66, 0x00],
        b'i' => [0x18, 0x00, 0x18, 0x18, 0x18, 0x18, 0x18, 0x00],
        b'j' => [0x06, 0x00, 0x06, 0x06, 0x06, 0x66, 0x66, 0x3C],
        b'k' => [0x60, 0x60, 0x66, 0x6C, 0x78, 0x6C, 0x66, 0x00],
        b'l' => [0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x00],
        b'm' => [0x00, 0x00, 0xEC, 0xFE, 0xD6, 0xC6, 0xC6, 0x00],
        b'n' => [0x00, 0x00, 0x7C, 0x66, 0x66, 0x66, 0x66, 0x00],
        b'o' => [0x00, 0x00, 0x3C, 0x66, 0x66, 0x66, 0x3C, 0x00],
        b'p' => [0x00, 0x00, 0x7C, 0x66, 0x66, 0x7C, 0x60, 0x60],
        b'q' => [0x00, 0x00, 0x3E, 0x66, 0x66, 0x3E, 0x06, 0x06],
        b'r' => [0x00, 0x00, 0x7C, 0x66, 0x60, 0x60, 0x60, 0x00],
        b's' => [0x00, 0x00, 0x3E, 0x60, 0x3C, 0x06, 0x7C, 0x00],
        b't' => [0x30, 0x30, 0x7C, 0x30, 0x30, 0x36, 0x1C, 0x00],
        b'u' => [0x00, 0x00, 0x66, 0x66, 0x66, 0x66, 0x3E, 0x00],
        b'v' => [0x00, 0x00, 0x66, 0x66, 0x66, 0x3C, 0x18, 0x00],
        b'w' => [0x00, 0x00, 0xC6, 0xC6, 0xD6, 0xFE, 0x6C, 0x00],
        b'x' => [0x00, 0x00, 0x66, 0x3C, 0x18, 0x3C, 0x66, 0x00],
        b'y' => [0x00, 0x00, 0x66, 0x66, 0x66, 0x3E, 0x06, 0x3C],
        b'z' => [0x00, 0x00, 0x7E, 0x0C, 0x18, 0x30, 0x7E, 0x00],
        b'{' => [0x0E, 0x18, 0x18, 0x70, 0x18, 0x18, 0x0E, 0x00],
        b'|' => [0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x00],
        b'}' => [0x70, 0x18, 0x18, 0x0E, 0x18, 0x18, 0x70, 0x00],
        b'~' => [0x76, 0xDC, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        _ => [0x00, 0x00, 0x18, 0x3C, 0x3C, 0x18, 0x00, 0x00],
    }
}

/// Expands an 8x8 glyph into a crisp 8x16 glyph bitmap.
#[inline]
pub const fn glyph_8x16(c: u8) -> [u8; 16] {
    let g = glyph_8x8(c);
    [
        g[0], g[0], g[1], g[1], g[2], g[2], g[3], g[3], g[4], g[4], g[5], g[5], g[6], g[6], g[7],
        g[7],
    ]
}

/// Returns `(left_col, advance_cols)` in 8x8 glyph space for proportional fallback spacing.
#[inline]
pub const fn glyph_metrics(c: u8) -> (i32, i32) {
    match c {
        b' ' => (0, 4),
        b'i' | b'l' | b'!' | b'|' | b'\'' | b'`' => (2, 4),
        b'.' | b',' | b':' | b';' => (2, 4),
        b'I' | b'(' | b')' | b'[' | b']' => (1, 5),
        b'f' | b't' | b'r' | b'j' | b'1' => (1, 6),
        b'm' | b'w' | b'M' | b'W' => (0, 8),
        _ => (1, 6),
    }
}

/// Measures the rendered pixel width of `text` using genuine Apple `SFUI.ttf` when available.
pub fn measure_text_px(text: &str, scale_x: i32) -> i32 {
    if scale_x <= 0 || text.is_empty() {
        return 0;
    }
    let px_h = scale_to_px_height(scale_x, scale_x * 2);
    if let Some(mask) = get_or_rasterize_mask(text, px_h, 0) {
        return mask.width;
    }
    let mut w = 0i32;
    for b in text.bytes() {
        let (_, adv) = glyph_metrics(b);
        w += adv * scale_x;
    }
    w
}

/// Draws text using genuine Apple `SFUI.ttf` (`System Font Regular`) when available.
pub fn draw_text_scaled(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    x: i32,
    y: i32,
    text: &str,
    color: u32,
    scale_x: i32,
    scale_y: i32,
) {
    if scale_x <= 0 || scale_y <= 0 || fb_w == 0 || fb_h == 0 || text.is_empty() {
        return;
    }
    let px_h = scale_to_px_height(scale_x, scale_y);
    if let Some(mask) = get_or_rasterize_mask(text, px_h, 0) {
        blit_text_mask(fb, fb_w, fb_h, x, y - 1, &mask, color);
        return;
    }
    // Fallback bitmap path
    let base_a = ((color >> 24) & 0xFF).max(if color != 0 { 0xFF } else { 0 });
    let rgb_part = color & 0x00FF_FFFF;
    let mut cx = x;
    for b in text.bytes() {
        let (left_col, adv_cols) = glyph_metrics(b);
        let rows = glyph_8x8(b);
        for (ry, &row) in rows.iter().enumerate() {
            for rx in left_col..8 {
                if (row & (0x80 >> rx)) != 0 {
                    let px = cx + (rx - left_col) * scale_x;
                    let py = y + (ry as i32) * scale_y;
                    if px >= 0 && px < fb_w as i32 && py >= 0 && py < fb_h as i32 {
                        let idx = (py as usize) * fb_w + (px as usize);
                        fb[idx] = blend_argb(fb[idx], (base_a << 24) | rgb_part);
                    }
                }
            }
        }
        cx += adv_cols * scale_x;
    }
}

/// Draws text with a subtle dark drop shadow using genuine Apple `SFUI.ttf`.
pub fn draw_text_shadow(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    x: i32,
    y: i32,
    text: &str,
    color: u32,
    scale_x: i32,
    scale_y: i32,
) {
    draw_text_scaled(
        fb,
        fb_w,
        fb_h,
        x,
        y + 1,
        text,
        rgba(0, 4, 14, 155),
        scale_x,
        scale_y,
    );
    draw_text_scaled(fb, fb_w, fb_h, x, y, text, color, scale_x, scale_y);
}

/// Draws horizontally centered text at vertical coordinate `y`.
pub fn draw_text_centered(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    y: i32,
    text: &str,
    color: u32,
    scale_x: i32,
    scale_y: i32,
    shadow: bool,
) {
    let width = measure_text_px(text, scale_x);
    let x = ((fb_w as i32) - width) / 2;
    if shadow {
        draw_text_shadow(fb, fb_w, fb_h, x, y, text, color, scale_x, scale_y);
    } else {
        draw_text_scaled(fb, fb_w, fb_h, x, y, text, color, scale_x, scale_y);
    }
}

/// Draws right-aligned text ending at `right_x`.
pub fn draw_text_right(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    right_x: i32,
    y: i32,
    text: &str,
    color: u32,
    scale_x: i32,
    scale_y: i32,
) {
    let width = measure_text_px(text, scale_x);
    draw_text_scaled(fb, fb_w, fb_h, right_x - width, y, text, color, scale_x, scale_y);
}

/// Draws semibold text using genuine Apple `SFUI.ttf` (`System Font Semibold`).
pub fn draw_text_semibold(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    x: i32,
    y: i32,
    text: &str,
    color: u32,
    scale_x: i32,
    scale_y: i32,
    shadow: bool,
) {
    if scale_x <= 0 || scale_y <= 0 || fb_w == 0 || fb_h == 0 || text.is_empty() {
        return;
    }
    let px_h = scale_to_px_height(scale_x, scale_y);
    if let Some(mask) = get_or_rasterize_mask(text, px_h, 1) {
        if shadow {
            blit_text_mask(fb, fb_w, fb_h, x, y, &mask, rgba(0, 4, 14, 150));
        }
        blit_text_mask(fb, fb_w, fb_h, x, y - 1, &mask, color);
        return;
    }
    if shadow {
        draw_text_scaled(
            fb,
            fb_w,
            fb_h,
            x,
            y + 1,
            text,
            rgba(0, 4, 14, 130),
            scale_x,
            scale_y,
        );
    }
    draw_text_scaled(fb, fb_w, fb_h, x, y, text, color, scale_x, scale_y);
}

/// Renders a single numeral (`0..9` or `:`) at `(x, y)` with height `height` using genuine Apple `.SF UI Rounded`.
pub fn draw_sdf_numeral(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    x: i32,
    y: i32,
    ch: u8,
    height: i32,
    _stroke_r: f32,
    color: u32,
) -> i32 {
    let s = [ch];
    let text = core::str::from_utf8(&s).unwrap_or("0");
    if let Some(mask) = get_or_rasterize_mask(text, height.max(12), 2) {
        blit_text_mask(fb, fb_w, fb_h, x, y - 4, &mask, color);
        return mask.width;
    }
    let _ = put_pixel_blended;
    (height * 60 / 100).max(12)
}

/// Measures the total width of a numeral string (`"9:41"`, `"20"`, etc.) at `height`.
pub fn measure_sdf_numerals(text: &str, height: i32, _tracking: i32) -> i32 {
    if let Some(mask) = get_or_rasterize_mask(text, height.max(12), 2) {
        return mask.width;
    }
    (text.len() as i32) * (height * 60 / 100).max(12)
}

/// Draws the signature iOS 16 Lock Screen Clock (`"9:41"`) centered horizontally at `y`
/// using genuine Apple `SFUIRounded.ttf` (`.SF UI Rounded`) from the iOS 16.7.16 IPSW.
pub fn draw_ios16_lock_clock(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    y: i32,
    text: &str,
    height: i32,
    color: u32,
) {
    let eff_h = (height * 118 / 100).max(64);
    if let Some(mask) = get_or_rasterize_mask(text, eff_h, 2) {
        let start_x = ((fb_w as i32) - mask.width) / 2;
        blit_text_mask(fb, fb_w, fb_h, start_x, y - 6, &mask, rgba(6, 12, 28, 75));
        blit_text_mask(fb, fb_w, fb_h, start_x, y - 8, &mask, color);
        return;
    }
    let stroke_r = (height as f32 * 0.088).max(1.8);
    let tracking = (height / 18).max(2);
    let total_w = measure_sdf_numerals(text, height, tracking);
    let start_x = ((fb_w as i32) - total_w) / 2;
    let mut cx = start_x;
    for b in text.bytes() {
        let cw = draw_sdf_numeral(fb, fb_w, fb_h, cx, y, b, height, stroke_r, color);
        cx += cw + tracking;
    }
}
