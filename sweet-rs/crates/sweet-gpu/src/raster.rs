//! High-speed 2D/3D software rasterizer primitives, exact 4x4 supersampled Apple Squircle
//! (`|x|^4.2 + |y|^4.2 <= r^4.2`), real frosted-glass backdrop blur, anti-aliased rounded
//! shapes, scanline/barycentric triangle rasterizer, and SWAR framebuffer downsampler.

/// Packs `(r, g, b, a)` into `0xAA_RR_GG_BB` (whose little-endian bytes are `[B, G, R, A]`).
#[inline]
pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

/// Packs `(r, g, b)` with opaque alpha `0xFF` into `0xFF_RR_GG_BB`.
#[inline]
pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    0xFF00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

/// Ensures a color has non-zero alpha (if alpha == 0, defaults to 0xFF).
#[inline]
pub const fn ensure_alpha(c: u32) -> u32 {
    if (c & 0xFF00_0000) == 0 {
        c | 0xFF00_0000
    } else {
        c
    }
}

/// Alpha-blends `src` (`0xAARRGGBB`) over `dst` (`0xFFRRGGBB`) using integer SWAR arithmetic.
#[inline]
pub fn blend_argb(dst: u32, src: u32) -> u32 {
    let a = (src >> 24) & 0xFF;
    if a == 255 {
        return src;
    }
    if a == 0 {
        return dst;
    }
    let inv_a = 256 - a;
    let a_plus = a + 1;
    let rb = (((src & 0x00FF_00FF) * a_plus + (dst & 0x00FF_00FF) * inv_a) >> 8) & 0x00FF_00FF;
    let g = (((src & 0x0000_FF00) * a_plus + (dst & 0x0000_FF00) * inv_a) >> 8) & 0x0000_FF00;
    0xFF00_0000 | rb | g
}

/// Linearly interpolates between two `0xFFRRGGBB` colors with `t` in `0..=256`.
#[inline]
pub fn lerp_color(c0: u32, c1: u32, t: u32) -> u32 {
    let t = t.min(256);
    let inv_t = 256 - t;
    let rb = (((c0 & 0x00FF_00FF) * inv_t + (c1 & 0x00FF_00FF) * t) >> 8) & 0x00FF_00FF;
    let g = (((c0 & 0x0000_FF00) * inv_t + (c1 & 0x0000_FF00) * t) >> 8) & 0x0000_FF00;
    0xFF00_0000 | rb | g
}

#[inline]
pub fn put_pixel_blended(fb: &mut [u32], fb_w: usize, fb_h: usize, x: i32, y: i32, color: u32) {
    if x >= 0 && y >= 0 && (x as usize) < fb_w && (y as usize) < fb_h {
        let idx = (y as usize) * fb_w + (x as usize);
        fb[idx] = blend_argb(fb[idx], color);
    }
}

/// Fills an axis-aligned rectangle `(x, y, w, h)` with alpha blending.
pub fn fill_rect(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    color: u32,
) {
    if w <= 0 || h <= 0 || fb_w == 0 || fb_h == 0 {
        return;
    }
    let x0 = x.max(0) as usize;
    let y0 = y.max(0) as usize;
    let x1 = (x + w).min(fb_w as i32).max(0) as usize;
    let y1 = (y + h).min(fb_h as i32).max(0) as usize;
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let a = (color >> 24) & 0xFF;
    if a == 255 {
        for py in y0..y1 {
            let row = &mut fb[py * fb_w + x0..py * fb_w + x1];
            row.fill(color);
        }
    } else if a > 0 {
        for py in y0..y1 {
            let row = &mut fb[py * fb_w + x0..py * fb_w + x1];
            for px in row.iter_mut() {
                *px = blend_argb(*px, color);
            }
        }
    }
}

/// Fills an anti-aliased rounded rectangle `(x, y, w, h)` with corner radius `r`.
pub fn fill_rounded_rect(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    r: i32,
    color: u32,
) {
    if w <= 0 || h <= 0 {
        return;
    }
    let r = r.clamp(0, w.min(h) / 2);
    if r == 0 {
        fill_rect(fb, fb_w, fb_h, x, y, w, h, color);
        return;
    }
    let base_a = (color >> 24) & 0xFF;
    if base_a == 0 {
        return;
    }
    let rgb_part = color & 0x00FF_FFFF;
    let r_sub = r * 2;
    let r_in_sq = (r_sub - 1).max(0) * (r_sub - 1).max(0);
    let r_out_sq = (r_sub + 1) * (r_sub + 1);
    let span = (r_out_sq - r_in_sq).max(1);

    let y0 = y.max(0);
    let y1 = (y + h).min(fb_h as i32);
    let x0 = x.max(0);
    let x1 = (x + w).min(fb_w as i32);

    for py in y0..y1 {
        let ly = py - y;
        let cy_dist = if ly < r {
            r - 1 - ly
        } else if ly >= h - r {
            ly - (h - r)
        } else {
            0
        };

        let row_off = (py as usize) * fb_w;
        if cy_dist == 0 {
            if base_a == 255 {
                fb[row_off + (x0 as usize)..row_off + (x1 as usize)].fill(color);
            } else {
                for px in x0..x1 {
                    let idx = row_off + (px as usize);
                    fb[idx] = blend_argb(fb[idx], color);
                }
            }
            continue;
        }

        let dy2 = (cy_dist * 2 + 1) * (cy_dist * 2 + 1);
        for px in x0..x1 {
            let lx = px - x;
            let cx_dist = if lx < r {
                r - 1 - lx
            } else if lx >= w - r {
                lx - (w - r)
            } else {
                0
            };

            let idx = row_off + (px as usize);
            if cx_dist == 0 {
                fb[idx] = blend_argb(fb[idx], color);
            } else {
                let dx2 = (cx_dist * 2 + 1) * (cx_dist * 2 + 1);
                let d2 = dx2 + dy2;
                if d2 <= r_in_sq {
                    fb[idx] = blend_argb(fb[idx], color);
                } else if d2 < r_out_sq {
                    let cov = ((r_out_sq - d2) * 255 / span) as u32;
                    let eff_a = (base_a * cov) >> 8;
                    if eff_a > 0 {
                        fb[idx] = blend_argb(fb[idx], (eff_a << 24) | rgb_part);
                    }
                }
            }
        }
    }
}

/// Fills a vertical gradient rounded rectangle.
#[allow(clippy::too_many_arguments)]
pub fn fill_rounded_rect_gradient(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    r: i32,
    top_color: u32,
    bottom_color: u32,
) {
    if w <= 0 || h <= 0 {
        return;
    }
    let r = r.clamp(0, w.min(h) / 2);
    let r_sub = r * 2;
    let r_in_sq = (r_sub - 1).max(0) * (r_sub - 1).max(0);
    let r_out_sq = (r_sub + 1) * (r_sub + 1);
    let span = (r_out_sq - r_in_sq).max(1);

    let y0 = y.max(0);
    let y1 = (y + h).min(fb_h as i32);
    let x0 = x.max(0);
    let x1 = (x + w).min(fb_w as i32);

    for py in y0..y1 {
        let ly = py - y;
        let t = ((ly as u32) * 256) / (h.max(1) as u32);
        let row_color = lerp_color(top_color, bottom_color, t);
        let rgb_part = row_color & 0x00FF_FFFF;

        let cy_dist = if ly < r {
            r - 1 - ly
        } else if ly >= h - r {
            ly - (h - r)
        } else {
            0
        };

        let row_off = (py as usize) * fb_w;
        if cy_dist == 0 {
            fb[row_off + (x0 as usize)..row_off + (x1 as usize)].fill(row_color);
            continue;
        }

        let dy2 = (cy_dist * 2 + 1) * (cy_dist * 2 + 1);
        for px in x0..x1 {
            let lx = px - x;
            let cx_dist = if lx < r {
                r - 1 - lx
            } else if lx >= w - r {
                lx - (w - r)
            } else {
                0
            };

            let idx = row_off + (px as usize);
            if cx_dist == 0 {
                fb[idx] = row_color;
            } else {
                let dx2 = (cx_dist * 2 + 1) * (cx_dist * 2 + 1);
                let d2 = dx2 + dy2;
                if d2 <= r_in_sq {
                    fb[idx] = row_color;
                } else if d2 < r_out_sq {
                    let cov = ((r_out_sq - d2) * 255 / span) as u32;
                    if cov > 0 {
                        fb[idx] = blend_argb(fb[idx], (cov << 24) | rgb_part);
                    }
                }
            }
        }
    }
}

/// Computes exact 4x4 subpixel anti-aliased coverage (`0..=255`) for the continuous-curvature
/// Apple Squircle (`|x|^4.2 + |y|^4.2 <= r^4.2`) at local pixel `(lx, ly)` within a `size x size` icon.
#[inline]
pub fn squircle_coverage_4x4(lx: i32, ly: i32, size: i32) -> u32 {
    if lx < 0 || ly < 0 || lx >= size || ly >= size || size <= 0 {
        return 0;
    }
    let half = (size as f32) * 0.5;
    let inv_r = 1.0 / half.max(1.0);

    // Quick interior check at pixel center
    let cx_norm = (((lx as f32) + 0.5) - half).abs() * inv_r;
    let cy_norm = (((ly as f32) + 0.5) - half).abs() * inv_r;
    if cx_norm <= 0.72 && cy_norm <= 0.72 {
        return 255;
    }

    let mut inside = 0u32;
    for sy in 0..4 {
        let py = (ly as f32) + ((sy as f32) + 0.5) * 0.25;
        let dy = (py - half).abs() * inv_r;
        let dy_pow = dy.powf(4.2);
        if dy_pow > 1.0 {
            continue;
        }
        for sx in 0..4 {
            let px = (lx as f32) + ((sx as f32) + 0.5) * 0.25;
            let dx = (px - half).abs() * inv_r;
            let val = dx.powf(4.2) + dy_pow;
            if val <= 1.0 {
                inside += 1;
            }
        }
    }
    (inside * 255) / 16
}

/// Fills an Apple continuous-curvature squircle (`|x|^4.2 + |y|^4.2 <= r^4.2`) with vertical gradient.
pub fn fill_squircle_gradient(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    x: i32,
    y: i32,
    size: i32,
    top_color: u32,
    bottom_color: u32,
) {
    if size <= 0 {
        return;
    }
    let y0 = y.max(0);
    let y1 = (y + size).min(fb_h as i32);
    let x0 = x.max(0);
    let x1 = (x + size).min(fb_w as i32);

    for py in y0..y1 {
        let ly = py - y;
        let t = ((ly as u32) * 256) / (size.max(1) as u32);
        let row_color = lerp_color(top_color, bottom_color, t);
        let rgb_part = row_color & 0x00FF_FFFF;
        let row_off = (py as usize) * fb_w;

        for px in x0..x1 {
            let lx = px - x;
            let cov = squircle_coverage_4x4(lx, ly, size);
            if cov == 255 {
                fb[row_off + (px as usize)] = row_color;
            } else if cov > 0 {
                let idx = row_off + (px as usize);
                fb[idx] = blend_argb(fb[idx], (cov << 24) | rgb_part);
            }
        }
    }
}

/// Renders an app icon into a local `size x size` canvas via `draw_fn`, then composites it
/// onto `fb` at `(ix, iy)` using the exact 4x4 supersampled Apple Squircle mask (`|x|^4.2 + |y|^4.2 <= r^4.2`)
/// and an optional subtle drop shadow underneath.
pub fn render_squircle_icon<F>(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    ix: i32,
    iy: i32,
    size: i32,
    shadow: bool,
    draw_fn: F,
) where
    F: FnOnce(&mut [u32], usize, usize),
{
    if size <= 0 || fb_w == 0 || fb_h == 0 {
        return;
    }
    // Soft drop shadow beneath Home Screen icons
    if shadow {
        let sy_off = 2;
        let y0 = (iy + sy_off).max(0);
        let y1 = (iy + sy_off + size).min(fb_h as i32);
        let x0 = ix.max(0);
        let x1 = (ix + size).min(fb_w as i32);
        for py in y0..y1 {
            let ly = py - (iy + sy_off);
            let row_off = (py as usize) * fb_w;
            for px in x0..x1 {
                let lx = px - ix;
                let cov = squircle_coverage_4x4(lx, ly, size);
                if cov > 0 {
                    let sa = (58 * cov) >> 8;
                    if sa > 0 {
                        let idx = row_off + (px as usize);
                        fb[idx] = blend_argb(fb[idx], (sa << 24) | 0x0004_0814);
                    }
                }
            }
        }
    }

    let sz = size as usize;
    let mut canvas = vec![0xFFFF_FFFFu32; sz * sz];
    draw_fn(&mut canvas, sz, sz);

    let y0 = iy.max(0);
    let y1 = (iy + size).min(fb_h as i32);
    let x0 = ix.max(0);
    let x1 = (ix + size).min(fb_w as i32);

    for py in y0..y1 {
        let ly = py - iy;
        let row_off = (py as usize) * fb_w;
        let c_row = (ly as usize) * sz;
        for px in x0..x1 {
            let lx = px - ix;
            let cov = squircle_coverage_4x4(lx, ly, size);
            if cov > 0 {
                let src_px = canvas[c_row + (lx as usize)] & 0x00FF_FFFF;
                let idx = row_off + (px as usize);
                if cov == 255 {
                    fb[idx] = 0xFF00_0000 | src_px;
                } else {
                    fb[idx] = blend_argb(fb[idx], (cov << 24) | src_px);
                }
            }
        }
    }
}

/// Applies a real 2-pass separable box blur over the rectangular region `(x, y, w, h)` in-place,
/// masked to an anti-aliased rounded rectangle of radius `corner_r`, and blended with `tint_color`
/// and a subtle `border_color` specular rim.
#[allow(clippy::too_many_arguments)]
pub fn blur_rounded_rect(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    corner_r: i32,
    blur_radius: i32,
    tint_color: u32,
    border_color: u32,
) {
    if w <= 0 || h <= 0 || fb_w == 0 || fb_h == 0 {
        return;
    }
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + w).min(fb_w as i32);
    let y1 = (y + h).min(fb_h as i32);
    if x0 >= x1 || y0 >= y1 {
        return;
    }

    let rw = (x1 - x0) as usize;
    let rh = (y1 - y0) as usize;
    let br = blur_radius.clamp(1, 16) as usize;

    // Copy region with clamped padding from fb
    let mut temp_a = vec![0u32; rw * rh];
    let mut temp_b = vec![0u32; rw * rh];

    for ry in 0..rh {
        let sy = (y0 as usize + ry).min(fb_h - 1);
        let src_row = &fb[sy * fb_w..sy * fb_w + fb_w];
        let dst_row = &mut temp_a[ry * rw..(ry + 1) * rw];
        for rx in 0..rw {
            let sx = (x0 as usize + rx).min(fb_w - 1);
            dst_row[rx] = src_row[sx];
        }
    }

    // 2 iterations of horizontal + vertical box blur for smooth Gaussian-like frosted glass
    for _ in 0..2 {
        // Horizontal pass: temp_a -> temp_b
        for ry in 0..rh {
            let row_in = &temp_a[ry * rw..(ry + 1) * rw];
            let row_out = &mut temp_b[ry * rw..(ry + 1) * rw];
            for rx in 0..rw {
                let mut r_sum = 0u32;
                let mut g_sum = 0u32;
                let mut b_sum = 0u32;
                let mut count = 0u32;
                let k0 = rx.saturating_sub(br);
                let k1 = (rx + br).min(rw - 1);
                for sample in &row_in[k0..=k1] {
                    r_sum += (sample >> 16) & 0xFF;
                    g_sum += (sample >> 8) & 0xFF;
                    b_sum += sample & 0xFF;
                    count += 1;
                }
                row_out[rx] = rgb(
                    (r_sum / count) as u8,
                    (g_sum / count) as u8,
                    (b_sum / count) as u8,
                );
            }
        }
        // Vertical pass: temp_b -> temp_a
        for rx in 0..rw {
            for ry in 0..rh {
                let mut r_sum = 0u32;
                let mut g_sum = 0u32;
                let mut b_sum = 0u32;
                let mut count = 0u32;
                let k0 = ry.saturating_sub(br);
                let k1 = (ry + br).min(rh - 1);
                for ky in k0..=k1 {
                    let sample = temp_b[ky * rw + rx];
                    r_sum += (sample >> 16) & 0xFF;
                    g_sum += (sample >> 8) & 0xFF;
                    b_sum += sample & 0xFF;
                    count += 1;
                }
                let blurred = rgb(
                    (r_sum / count) as u8,
                    (g_sum / count) as u8,
                    (b_sum / count) as u8,
                );
                temp_a[ry * rw + rx] = blend_argb(blurred, tint_color);
            }
        }
    }

    // Composite blurred + tinted pixels into fb with rounded-corner anti-aliasing
    let r = corner_r.clamp(0, w.min(h) / 2);
    let r_sub = r * 2;
    let r_in_sq = (r_sub - 1).max(0) * (r_sub - 1).max(0);
    let r_out_sq = (r_sub + 1) * (r_sub + 1);
    let span = (r_out_sq - r_in_sq).max(1);

    for py in y0..y1 {
        let ly = py - y;
        let cy_dist = if ly < r {
            r - 1 - ly
        } else if ly >= h - r {
            ly - (h - r)
        } else {
            0
        };
        let dy2 = (cy_dist * 2 + 1) * (cy_dist * 2 + 1);
        let row_off = (py as usize) * fb_w;
        let t_row = ((py - y0) as usize) * rw;

        for px in x0..x1 {
            let lx = px - x;
            let cx_dist = if lx < r {
                r - 1 - lx
            } else if lx >= w - r {
                lx - (w - r)
            } else {
                0
            };

            let mut glass_px = temp_a[t_row + ((px - x0) as usize)];
            // Subtle 1px inner border highlight along edges
            let is_border = lx == 0 || lx == w - 1 || ly == 0 || ly == h - 1;
            if is_border && (border_color >> 24) > 0 {
                glass_px = blend_argb(glass_px, border_color);
            }

            let idx = row_off + (px as usize);
            if cx_dist == 0 && cy_dist == 0 {
                fb[idx] = glass_px;
            } else {
                let dx2 = if cx_dist > 0 {
                    (cx_dist * 2 + 1) * (cx_dist * 2 + 1)
                } else {
                    0
                };
                let dy2_eff = if cy_dist > 0 { dy2 } else { 0 };
                let d2 = dx2 + dy2_eff;
                if d2 <= r_in_sq {
                    if d2 >= (r_sub - 3).max(0) * (r_sub - 3).max(0) && (border_color >> 24) > 0 {
                        glass_px = blend_argb(glass_px, border_color);
                    }
                    fb[idx] = glass_px;
                } else if d2 < r_out_sq {
                    let cov = ((r_out_sq - d2) * 255 / span) as u32;
                    if cov > 0 {
                        fb[idx] = blend_argb(fb[idx], (cov << 24) | (glass_px & 0x00FF_FFFF));
                    }
                }
            }
        }
    }
}

/// Fills an anti-aliased circle centered at `(cx, cy)` with radius `r`.
pub fn fill_circle(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    cx: i32,
    cy: i32,
    r: i32,
    color: u32,
) {
    if r <= 0 {
        return;
    }
    let base_a = (color >> 24) & 0xFF;
    if base_a == 0 {
        return;
    }
    let rgb_part = color & 0x00FF_FFFF;
    let r_f = r as f32;
    let y0 = (cy - r - 1).max(0);
    let y1 = (cy + r + 2).min(fb_h as i32);
    let x0 = (cx - r - 1).max(0);
    let x1 = (cx + r + 2).min(fb_w as i32);

    for py in y0..y1 {
        let dy = (py - cy) as f32;
        let dy2 = dy * dy;
        let row_off = (py as usize) * fb_w;
        for px in x0..x1 {
            let dx = (px - cx) as f32;
            let d = (dx * dx + dy2).sqrt();
            let edge = r_f + 0.5 - d;
            if edge > 0.0 {
                let cov = if edge >= 1.0 {
                    255u32
                } else {
                    (edge * 255.0) as u32
                };
                let eff_a = (base_a * cov) >> 8;
                if eff_a > 0 {
                    let idx = row_off + (px as usize);
                    fb[idx] = blend_argb(fb[idx], (eff_a << 24) | rgb_part);
                }
            }
        }
    }
}

/// Draws a smooth anti-aliased circle ring centered at `(cx, cy)` with inner radius `r_inner` and outer radius `r_outer`.
#[allow(clippy::too_many_arguments)]
pub fn draw_ring(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    cx: i32,
    cy: i32,
    r_inner: i32,
    r_outer: i32,
    color: u32,
) {
    if r_outer <= 0 || r_outer <= r_inner {
        return;
    }
    let base_a = (color >> 24) & 0xFF;
    if base_a == 0 {
        return;
    }
    let rgb_part = color & 0x00FF_FFFF;
    let r_in = r_inner.max(0) as f32;
    let r_out = r_outer as f32;
    let y0 = (cy - r_outer - 1).max(0);
    let y1 = (cy + r_outer + 2).min(fb_h as i32);
    let x0 = (cx - r_outer - 1).max(0);
    let x1 = (cx + r_outer + 2).min(fb_w as i32);

    for py in y0..y1 {
        let dy = (py - cy) as f32;
        let dy2 = dy * dy;
        let row_off = (py as usize) * fb_w;
        for px in x0..x1 {
            let dx = (px - cx) as f32;
            let d = (dx * dx + dy2).sqrt();
            let cov_out = (r_out + 0.5 - d).clamp(0.0, 1.0);
            let cov_in = if r_inner > 0 {
                (d - (r_in - 0.5)).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let cov = (cov_out * cov_in * 255.0) as u32;
            let eff_a = (base_a * cov) >> 8;
            if eff_a > 0 {
                let idx = row_off + (px as usize);
                fb[idx] = blend_argb(fb[idx], (eff_a << 24) | rgb_part);
            }
        }
    }
}

/// Draws a smooth anti-aliased rounded-cap line with thickness `thickness` between `(x0, y0)` and `(x1, y1)`.
#[allow(clippy::too_many_arguments)]
pub fn draw_line(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    thickness: i32,
    color: u32,
) {
    if fb_w == 0 || fb_h == 0 || thickness <= 0 {
        return;
    }
    let base_a = (color >> 24) & 0xFF;
    if base_a == 0 {
        return;
    }
    let rgb_part = color & 0x00FF_FFFF;
    let r = (thickness as f32) * 0.5;
    let pad = (thickness / 2) + 2;
    let min_x = x0.min(x1).saturating_sub(pad).max(0);
    let max_x = (x0.max(x1) + pad).min((fb_w as i32) - 1);
    let min_y = y0.min(y1).saturating_sub(pad).max(0);
    let max_y = (y0.max(y1) + pad).min((fb_h as i32) - 1);

    let ax = x0 as f32;
    let ay = y0 as f32;
    let bx = x1 as f32;
    let by = y1 as f32;
    let vx = bx - ax;
    let vy = by - ay;
    let len_sq = vx * vx + vy * vy;

    for py in min_y..=max_y {
        let py_f = py as f32;
        let row_off = (py as usize) * fb_w;
        for px in min_x..=max_x {
            let px_f = px as f32;
            let d = if len_sq <= 1e-4 {
                ((px_f - ax).powi(2) + (py_f - ay).powi(2)).sqrt()
            } else {
                let t = (((px_f - ax) * vx + (py_f - ay) * vy) / len_sq).clamp(0.0, 1.0);
                let proj_x = ax + t * vx;
                let proj_y = ay + t * vy;
                ((px_f - proj_x).powi(2) + (py_f - proj_y).powi(2)).sqrt()
            };
            let edge = r + 0.5 - d;
            if edge > 0.0 {
                let cov = (edge.clamp(0.0, 1.0) * 255.0) as u32;
                let eff_a = (base_a * cov) >> 8;
                if eff_a > 0 {
                    let idx = row_off + (px as usize);
                    fb[idx] = blend_argb(fb[idx], (eff_a << 24) | rgb_part);
                }
            }
        }
    }
}

/// Fast scanline / half-space triangle rasterizer (`GPU_CMD_TRIANGLE`).
#[allow(clippy::too_many_arguments)]
pub fn fill_triangle(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    color: u32,
) {
    if fb_w == 0 || fb_h == 0 {
        return;
    }
    let min_x = x0.min(x1).min(x2).max(0);
    let max_x = x0.max(x1).max(x2).min((fb_w as i32) - 1);
    let min_y = y0.min(y1).min(y2).max(0);
    let max_y = y0.max(y1).max(y2).min((fb_h as i32) - 1);
    if min_x > max_x || min_y > max_y {
        return;
    }

    let area = (x1 - x0) as i64 * (y2 - y0) as i64 - (y1 - y0) as i64 * (x2 - x0) as i64;
    if area == 0 {
        return;
    }
    let (ax, ay, bx, by, cx, cy) = if area > 0 {
        (x0, y0, x1, y1, x2, y2)
    } else {
        (x0, y0, x2, y2, x1, y1)
    };

    let c = ensure_alpha(color);
    let alpha = (c >> 24) & 0xFF;

    let dx01 = (bx - ax) as i64;
    let dy01 = (by - ay) as i64;
    let dx12 = (cx - bx) as i64;
    let dy12 = (cy - by) as i64;
    let dx20 = (ax - cx) as i64;
    let dy20 = (ay - cy) as i64;

    for py in min_y..=max_y {
        let row_off = (py as usize) * fb_w;
        let py64 = py as i64;
        for px in min_x..=max_x {
            let px64 = px as i64;
            let w0 = dx01 * (py64 - ay as i64) - dy01 * (px64 - ax as i64);
            let w1 = dx12 * (py64 - by as i64) - dy12 * (px64 - bx as i64);
            let w2 = dx20 * (py64 - cy as i64) - dy20 * (px64 - cx as i64);
            if w0 >= 0 && w1 >= 0 && w2 >= 0 {
                let idx = row_off + (px as usize);
                if alpha == 255 {
                    fb[idx] = c;
                } else {
                    fb[idx] = blend_argb(fb[idx], c);
                }
            }
        }
    }
}

/// Blits a procedural checkerboard/gradient texture (`GPU_CMD_BLIT_TEXTURE`) into `(x, y, w, h)`.
#[allow(clippy::too_many_arguments)]
pub fn blit_procedural_texture(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    tint_color: u32,
) {
    if w <= 0 || h <= 0 || fb_w == 0 || fb_h == 0 {
        return;
    }
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + w).min(fb_w as i32);
    let y1 = (y + h).min(fb_h as i32);
    let base = ensure_alpha(tint_color);
    let alt = lerp_color(base, 0xFFFF_FFFF, 72);

    for py in y0..y1 {
        let row_off = (py as usize) * fb_w;
        let ty = ((py - y) >> 3) & 1;
        for px in x0..x1 {
            let tx = ((px - x) >> 3) & 1;
            let texel = if (tx ^ ty) == 0 { base } else { alt };
            let idx = row_off + (px as usize);
            fb[idx] = blend_argb(fb[idx], texel);
        }
    }
}

/// Ultra-fast integer box-filter downsampler from `1080x2400` 32-bit BGRA (`&[u8]`)
/// into any viewport buffer (`&mut [u32]`, e.g., `360x800` or `405x900`).
pub fn downsample_1080x2400_to_viewport(
    src_bgra: &[u8],
    dst_bgra: &mut [u32],
    dst_w: usize,
    dst_h: usize,
) {
    if dst_w == 0 || dst_h == 0 || dst_bgra.is_empty() || src_bgra.len() < 4 {
        return;
    }
    let out_len = (dst_w * dst_h).min(dst_bgra.len());
    let dst = &mut dst_bgra[..out_len];

    let total_src_pixels = src_bgra.len() / 4;
    let (src_w, src_h) = if total_src_pixels >= 1080 * 2400 {
        (1080usize, 2400usize)
    } else if total_src_pixels >= dst_w * dst_h && dst_w > 0 {
        let ratio_h = (total_src_pixels / dst_w).max(1);
        (dst_w, ratio_h)
    } else {
        let side = (total_src_pixels as f64).sqrt().max(1.0) as usize;
        (side, (total_src_pixels / side).max(1))
    };

    #[inline(always)]
    fn read_px(src: &[u8], idx: usize) -> u32 {
        let off = idx * 4;
        u32::from_le_bytes([src[off], src[off + 1], src[off + 2], src[off + 3]])
    }

    if src_w == 1080 && src_h == 2400 && dst_w == 360 && dst_h == 800 && out_len == 360 * 800 {
        for dy in 0..800usize {
            let sy0 = dy * 3;
            let r0 = sy0 * 1080;
            let r1 = (sy0 + 1) * 1080;
            let r2 = (sy0 + 2) * 1080;
            let dst_row = &mut dst[dy * 360..(dy + 1) * 360];
            for dx in 0..360usize {
                let sx0 = dx * 3;
                let mut b_sum = 0u32;
                let mut g_sum = 0u32;
                let mut r_sum = 0u32;
                for &row_base in &[r0, r1, r2] {
                    for col in sx0..sx0 + 3 {
                        let off = (row_base + col) * 4;
                        b_sum += src_bgra[off] as u32;
                        g_sum += src_bgra[off + 1] as u32;
                        r_sum += src_bgra[off + 2] as u32;
                    }
                }
                let b = (b_sum + 4) / 9;
                let g = (g_sum + 4) / 9;
                let r = (r_sum + 4) / 9;
                dst_row[dx] = 0xFF00_0000 | (r << 16) | (g << 8) | b;
            }
        }
        return;
    }

    let step_x_fp = ((src_w << 16) / dst_w.max(1)).max(1);
    let step_y_fp = ((src_h << 16) / dst_h.max(1)).max(1);
    let half_x_fp = (step_x_fp >> 1).min(((src_w.saturating_sub(1)) << 16).max(1));
    let half_y_fp = (step_y_fp >> 1).min(((src_h.saturating_sub(1)) << 16).max(1));

    for dy in 0..dst_h {
        let sy0 = ((dy * step_y_fp) >> 16).min(src_h - 1);
        let sy1 = ((dy * step_y_fp + half_y_fp) >> 16).min(src_h - 1);
        let row0_base = sy0 * src_w;
        let row1_base = sy1 * src_w;
        let dst_row_start = dy * dst_w;
        if dst_row_start >= out_len {
            break;
        }
        let dst_row_end = (dst_row_start + dst_w).min(out_len);
        for (dx, out_px) in dst[dst_row_start..dst_row_end].iter_mut().enumerate() {
            let sx0 = ((dx * step_x_fp) >> 16).min(src_w - 1);
            let sx1 = ((dx * step_x_fp + half_x_fp) >> 16).min(src_w - 1);
            let p0 = read_px(src_bgra, (row0_base + sx0).min(total_src_pixels - 1));
            let p1 = read_px(src_bgra, (row0_base + sx1).min(total_src_pixels - 1));
            let p2 = read_px(src_bgra, (row1_base + sx0).min(total_src_pixels - 1));
            let p3 = read_px(src_bgra, (row1_base + sx1).min(total_src_pixels - 1));

            let rb = (((p0 & 0x00FF_00FF)
                + (p1 & 0x00FF_00FF)
                + (p2 & 0x00FF_00FF)
                + (p3 & 0x00FF_00FF))
                >> 2)
                & 0x00FF_00FF;
            let g = (((p0 & 0x0000_FF00)
                + (p1 & 0x0000_FF00)
                + (p2 & 0x0000_FF00)
                + (p3 & 0x0000_FF00))
                >> 2)
                & 0x0000_FF00;
            *out_px = 0xFF00_0000 | rb | g;
        }
    }
}
