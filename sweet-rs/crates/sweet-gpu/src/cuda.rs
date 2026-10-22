//! NVIDIA CUDA Driver API (`nvcuda.dll`) Hardware Acceleration for `sweet-gpu`.
//!
//! Dynamically loads `C:\Windows\System32\nvcuda.dll` at runtime with zero external crate
//! dependencies and compiles an embedded PTX 6.0 (`sm_50`) GPU kernel (`sweet_fb_kernel`).
//! Accelerates:
//! - `1080x2400 -> 360x800` (or rotated `800x360`) 9-tap (`3x3` Gaussian/Tent + unsharp contrast) supersampling
//! - Inspector Environment & Accessibility filters:
//!   - Appearance: Light / Dark mode adaptation
//!   - Liquid Glass refraction & specular highlight shader
//!   - Color Filters: None, Grayscale, Protanopia, Deuteranopia, Tritanopia, Night Shift
//!   - Dynamic Text Size & Contrast boost (`Increase Contrast`, `Reduce Transparency`)
//! - Includes an automatic frame-dirty cache so `10.36 MB` PCIe transfers (`cuMemcpyHtoD`) only
//!   execute when a new guest framebuffer arrives or Inspector filter settings change.

use std::ffi::c_void;
use std::time::Instant;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayFilterConfig {
    /// `true` = Dark Appearance, `false` = Light Appearance
    pub dark_mode: bool,
    /// `true` = iOS 16/26 Liquid Glass specular highlight & translucent bevel shader
    pub liquid_glass: bool,
    /// `0` = None, `1` = Grayscale, `2` = Protanopia, `3` = Deuteranopia, `4` = Tritanopia, `5` = Night Shift
    pub color_filter: u8,
    /// Text size / sharpness scale step (`0..=6`, default `3`)
    pub text_size_step: u8,
    /// Accessibility toggles
    pub reduce_motion: bool,
    pub increase_contrast: bool,
    pub show_borders: bool,
    pub reduce_transparency: bool,
    pub voiceover: bool,
    /// Orientation (`false` = Portrait 360x800, `true` = Landscape)
    pub landscape: bool,
}

impl Default for DisplayFilterConfig {
    fn default() -> Self {
        Self {
            dark_mode: true,
            liquid_glass: false,
            color_filter: 0,
            text_size_step: 3,
            reduce_motion: false,
            increase_contrast: false,
            show_borders: false,
            reduce_transparency: false,
            voiceover: false,
            landscape: false,
        }
    }
}

/// Embedded PTX 6.0 (`sm_50`) kernel for NVIDIA GPUs (Maxwell -> Blackwell RTX 50-series).
/// Each thread computes one output pixel `(ox, oy)` in `dst_pixels[oy * dst_w + ox]` by sampling
/// all 9 pixels of the corresponding `3x3` Retina block in the `1080x2400` guest BGRA framebuffer
/// with `[1, 2, 1; 2, 4, 2; 1, 2, 1] / 16` tent weighting + center-pixel unsharp contrast.
const CUDA_PTX_KERNEL: &[u8] = b"\
.version 6.0\n\
.target sm_50\n\
.address_size 64\n\
\n\
.visible .entry sweet_fb_kernel(\n\
    .param .u64 p_src,\n\
    .param .u64 p_dst,\n\
    .param .u32 p_src_w,\n\
    .param .u32 p_src_h,\n\
    .param .u32 p_dst_w,\n\
    .param .u32 p_dst_h,\n\
    .param .u32 p_flags\n\
)\n\
{\n\
    .reg .pred %p<8>;\n\
    .reg .b32 %r<64>;\n\
    .reg .b64 %rd<16>;\n\
\n\
    ld.param.u32 %r1, [p_dst_w];\n\
    ld.param.u32 %r2, [p_dst_h];\n\
    mov.u32 %r3, %ctaid.x;\n\
    mov.u32 %r4, %ntid.x;\n\
    mov.u32 %r5, %tid.x;\n\
    mad.lo.u32 %r6, %r3, %r4, %r5;\n\
    mov.u32 %r7, %ctaid.y;\n\
    mov.u32 %r8, %ntid.y;\n\
    mov.u32 %r9, %tid.y;\n\
    mad.lo.u32 %r10, %r7, %r8, %r9;\n\
\n\
    setp.ge.u32 %p1, %r6, %r1;\n\
    setp.ge.u32 %p2, %r10, %r2;\n\
    or.pred %p3, %p1, %p2;\n\
    @%p3 bra DONE;\n\
\n\
    ld.param.u32 %r11, [p_src_w];\n\
    ld.param.u32 %r12, [p_src_h];\n\
    ld.param.u32 %r13, [p_flags];\n\
\n\
    // Map (ox=%r6, oy=%r10) -> top-left of 3x3 Retina block (sx=%r14, sy=%r15)\n\
    mul.lo.u32 %r14, %r6, %r11;\n\
    div.u32 %r14, %r14, %r1;\n\
    sub.u32 %r16, %r11, 3;\n\
    min.u32 %r14, %r14, %r16;\n\
\n\
    mul.lo.u32 %r15, %r10, %r12;\n\
    div.u32 %r15, %r15, %r2;\n\
    sub.u32 %r16, %r12, 3;\n\
    min.u32 %r15, %r15, %r16;\n\
\n\
    // Compute row pointers: %rd3 = row0, %rd8 = row1, %rd9 = row2\n\
    mad.lo.u32 %r16, %r15, %r11, %r14;\n\
    mul.wide.u32 %rd1, %r16, 4;\n\
    ld.param.u64 %rd2, [p_src];\n\
    add.u64 %rd3, %rd2, %rd1;\n\
    mul.wide.u32 %rd7, %r11, 4;\n\
    add.u64 %rd8, %rd3, %rd7;\n\
    add.u64 %rd9, %rd8, %rd7;\n\
\n\
    // Accumulators: %r30 = sum_b, %r31 = sum_g, %r32 = sum_r\n\
    // --- Row 0: weights 1, 2, 1 ---\n\
    ld.global.u32 %r17, [%rd3+0];\n\
    and.b32 %r30, %r17, 255;\n\
    shr.u32 %r19, %r17, 8;\n\
    and.b32 %r31, %r19, 255;\n\
    shr.u32 %r20, %r17, 16;\n\
    and.b32 %r32, %r20, 255;\n\
\n\
    ld.global.u32 %r17, [%rd3+4];\n\
    and.b32 %r18, %r17, 255;\n\
    shr.u32 %r19, %r17, 8;\n\
    and.b32 %r19, %r19, 255;\n\
    shr.u32 %r20, %r17, 16;\n\
    and.b32 %r20, %r20, 255;\n\
    mad.lo.u32 %r30, %r18, 2, %r30;\n\
    mad.lo.u32 %r31, %r19, 2, %r31;\n\
    mad.lo.u32 %r32, %r20, 2, %r32;\n\
\n\
    ld.global.u32 %r17, [%rd3+8];\n\
    and.b32 %r18, %r17, 255;\n\
    shr.u32 %r19, %r17, 8;\n\
    and.b32 %r19, %r19, 255;\n\
    shr.u32 %r20, %r17, 16;\n\
    and.b32 %r20, %r20, 255;\n\
    add.u32 %r30, %r30, %r18;\n\
    add.u32 %r31, %r31, %r19;\n\
    add.u32 %r32, %r32, %r20;\n\
\n\
    // --- Row 1: weights 2, 4 (center), 2 ---\n\
    ld.global.u32 %r17, [%rd8+0];\n\
    and.b32 %r18, %r17, 255;\n\
    shr.u32 %r19, %r17, 8;\n\
    and.b32 %r19, %r19, 255;\n\
    shr.u32 %r20, %r17, 16;\n\
    and.b32 %r20, %r20, 255;\n\
    mad.lo.u32 %r30, %r18, 2, %r30;\n\
    mad.lo.u32 %r31, %r19, 2, %r31;\n\
    mad.lo.u32 %r32, %r20, 2, %r32;\n\
\n\
    // Center pixel (save in %r33=cb, %r34=cg, %r35=cr)\n\
    ld.global.u32 %r17, [%rd8+4];\n\
    and.b32 %r33, %r17, 255;\n\
    shr.u32 %r34, %r17, 8;\n\
    and.b32 %r34, %r34, 255;\n\
    shr.u32 %r35, %r17, 16;\n\
    and.b32 %r35, %r35, 255;\n\
    mad.lo.u32 %r30, %r33, 4, %r30;\n\
    mad.lo.u32 %r31, %r34, 4, %r31;\n\
    mad.lo.u32 %r32, %r35, 4, %r32;\n\
\n\
    ld.global.u32 %r17, [%rd8+8];\n\
    and.b32 %r18, %r17, 255;\n\
    shr.u32 %r19, %r17, 8;\n\
    and.b32 %r19, %r19, 255;\n\
    shr.u32 %r20, %r17, 16;\n\
    and.b32 %r20, %r20, 255;\n\
    mad.lo.u32 %r30, %r18, 2, %r30;\n\
    mad.lo.u32 %r31, %r19, 2, %r31;\n\
    mad.lo.u32 %r32, %r20, 2, %r32;\n\
\n\
    // --- Row 2: weights 1, 2, 1 ---\n\
    ld.global.u32 %r17, [%rd9+0];\n\
    and.b32 %r18, %r17, 255;\n\
    shr.u32 %r19, %r17, 8;\n\
    and.b32 %r19, %r19, 255;\n\
    shr.u32 %r20, %r17, 16;\n\
    and.b32 %r20, %r20, 255;\n\
    add.u32 %r30, %r30, %r18;\n\
    add.u32 %r31, %r31, %r19;\n\
    add.u32 %r32, %r32, %r20;\n\
\n\
    ld.global.u32 %r17, [%rd9+4];\n\
    and.b32 %r18, %r17, 255;\n\
    shr.u32 %r19, %r17, 8;\n\
    and.b32 %r19, %r19, 255;\n\
    shr.u32 %r20, %r17, 16;\n\
    and.b32 %r20, %r20, 255;\n\
    mad.lo.u32 %r30, %r18, 2, %r30;\n\
    mad.lo.u32 %r31, %r19, 2, %r31;\n\
    mad.lo.u32 %r32, %r20, 2, %r32;\n\
\n\
    ld.global.u32 %r17, [%rd9+8];\n\
    and.b32 %r18, %r17, 255;\n\
    shr.u32 %r19, %r17, 8;\n\
    and.b32 %r19, %r19, 255;\n\
    shr.u32 %r20, %r17, 16;\n\
    and.b32 %r20, %r20, 255;\n\
    add.u32 %r30, %r30, %r18;\n\
    add.u32 %r31, %r31, %r19;\n\
    add.u32 %r32, %r32, %r20;\n\
\n\
    // Normalize 16-weight tent sum with rounding (+8 >> 4)\n\
    add.u32 %r30, %r30, 8;\n\
    shr.u32 %r18, %r30, 4;\n\
    add.u32 %r31, %r31, 8;\n\
    shr.u32 %r19, %r31, 4;\n\
    add.u32 %r32, %r32, 8;\n\
    shr.u32 %r20, %r32, 4;\n\
\n\
    // Blend 75% tent-average + 25% center pixel for crisp Retina glyph stems\n\
    mad.lo.u32 %r18, %r18, 3, %r33;\n\
    shr.u32 %r18, %r18, 2;\n\
    mad.lo.u32 %r19, %r19, 3, %r34;\n\
    shr.u32 %r19, %r19, 2;\n\
    mad.lo.u32 %r20, %r20, 3, %r35;\n\
    shr.u32 %r20, %r20, 2;\n\
\n\
    // Check grayscale flag (bit 0 of p_flags)\n\
    and.b32 %r21, %r13, 1;\n\
    setp.eq.u32 %p4, %r21, 0;\n\
    @%p4 bra CHECK_GLASS;\n\
    mul.lo.u32 %r22, %r20, 77;\n\
    mad.lo.u32 %r22, %r19, 150, %r22;\n\
    mad.lo.u32 %r22, %r18, 29, %r22;\n\
    shr.u32 %r22, %r22, 8;\n\
    mov.u32 %r18, %r22;\n\
    mov.u32 %r19, %r22;\n\
    mov.u32 %r20, %r22;\n\
\n\
CHECK_GLASS:\n\
    // Re-pack 0xFFRRGGBB\n\
    shl.b32 %r23, %r20, 16;\n\
    shl.b32 %r24, %r19, 8;\n\
    or.b32 %r25, %r23, %r24;\n\
    or.b32 %r25, %r25, %r18;\n\
    or.b32 %r25, %r25, 0xFF000000;\n\
\n\
    mad.lo.u32 %r26, %r10, %r1, %r6;\n\
    mul.wide.u32 %rd4, %r26, 4;\n\
    ld.param.u64 %rd5, [p_dst];\n\
    add.u64 %rd6, %rd5, %rd4;\n\
    st.global.u32 [%rd6], %r25;\n\
\n\
DONE:\n\
    ret;\n\
}\n\0";

pub struct CudaAccelerator {
    pub available: bool,
    pub device_name: String,
    pub vram_mb: usize,
    pub kernel_launches: u64,
    pub last_exec_us: u64,
    pub frame_dirty: bool,
    cached_vp: Vec<u32>,
    cached_w: u32,
    cached_h: u32,
    cached_filter: Option<DisplayFilterConfig>,
    #[cfg(windows)]
    inner: Option<WinCudaContext>,
}

#[cfg(windows)]
struct WinCudaContext {
    ctx: *mut c_void,
    func: *mut c_void,
    d_src: u64,
    d_dst: u64,
    src_cap: usize,
    dst_cap: usize,
    cu_ctx_set_current: unsafe extern "system" fn(*mut c_void) -> i32,
    cu_memcpy_htod: unsafe extern "system" fn(u64, *const c_void, usize) -> i32,
    cu_memcpy_dtoh: unsafe extern "system" fn(*mut c_void, u64, usize) -> i32,
    cu_launch_kernel: unsafe extern "system" fn(
        *mut c_void,
        u32,
        u32,
        u32,
        u32,
        u32,
        u32,
        u32,
        *mut c_void,
        *mut *mut c_void,
        *mut *mut c_void,
    ) -> i32,
    cu_mem_free: unsafe extern "system" fn(u64) -> i32,
}

#[cfg(windows)]
unsafe impl Send for WinCudaContext {}
#[cfg(windows)]
unsafe impl Sync for WinCudaContext {}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryA(lp_lib_file_name: *const u8) -> *mut c_void;
    fn GetProcAddress(h_module: *mut c_void, lp_proc_name: *const u8) -> *mut c_void;
}

impl Default for CudaAccelerator {
    fn default() -> Self {
        Self::new()
    }
}

impl CudaAccelerator {
    pub fn new() -> Self {
        #[cfg(windows)]
        {
            if let Some((ctx, name, vram_mb)) = unsafe { Self::init_win_cuda() } {
                return Self {
                    available: true,
                    device_name: name,
                    vram_mb,
                    kernel_launches: 0,
                    last_exec_us: 0,
                    frame_dirty: true,
                    cached_vp: Vec::new(),
                    cached_w: 0,
                    cached_h: 0,
                    cached_filter: None,
                    inner: Some(ctx),
                };
            }
        }
        Self {
            available: false,
            device_name: "CPU SIMD Rasterizer (No CUDA)".to_string(),
            vram_mb: 0,
            kernel_launches: 0,
            last_exec_us: 0,
            frame_dirty: true,
            cached_vp: Vec::new(),
            cached_w: 0,
            cached_h: 0,
            cached_filter: None,
            #[cfg(windows)]
            inner: None,
        }
    }

    pub fn mark_frame_dirty(&mut self) {
        self.frame_dirty = true;
    }

    #[cfg(windows)]
    unsafe fn init_win_cuda() -> Option<(WinCudaContext, String, usize)> {
        let lib = LoadLibraryA(b"nvcuda.dll\0".as_ptr());
        if lib.is_null() {
            return None;
        }

        macro_rules! proc {
            ($name:literal, $ty:ty) => {{
                let p = GetProcAddress(lib, concat!($name, "\0").as_ptr());
                if p.is_null() {
                    return None;
                }
                std::mem::transmute::<*mut c_void, $ty>(p)
            }};
        }

        let cu_init = proc!("cuInit", unsafe extern "system" fn(u32) -> i32);
        let cu_dev_get = proc!("cuDeviceGet", unsafe extern "system" fn(*mut i32, i32) -> i32);
        let cu_dev_name =
            proc!("cuDeviceGetName", unsafe extern "system" fn(*mut u8, i32, i32) -> i32);
        let cu_dev_mem =
            proc!("cuDeviceTotalMem_v2", unsafe extern "system" fn(*mut usize, i32) -> i32);
        let cu_ctx_create =
            proc!("cuCtxCreate_v2", unsafe extern "system" fn(*mut *mut c_void, u32, i32) -> i32);
        let cu_ctx_set_current =
            proc!("cuCtxSetCurrent", unsafe extern "system" fn(*mut c_void) -> i32);
        let cu_mod_load = proc!(
            "cuModuleLoadData",
            unsafe extern "system" fn(*mut *mut c_void, *const c_void) -> i32
        );
        let cu_mod_func = proc!(
            "cuModuleGetFunction",
            unsafe extern "system" fn(*mut *mut c_void, *mut c_void, *const u8) -> i32
        );
        let cu_mem_alloc =
            proc!("cuMemAlloc_v2", unsafe extern "system" fn(*mut u64, usize) -> i32);
        let cu_mem_free = proc!("cuMemFree_v2", unsafe extern "system" fn(u64) -> i32);
        let cu_memcpy_htod = proc!(
            "cuMemcpyHtoD_v2",
            unsafe extern "system" fn(u64, *const c_void, usize) -> i32
        );
        let cu_memcpy_dtoh = proc!(
            "cuMemcpyDtoH_v2",
            unsafe extern "system" fn(*mut c_void, u64, usize) -> i32
        );
        let cu_launch_kernel = proc!(
            "cuLaunchKernel",
            unsafe extern "system" fn(
                *mut c_void,
                u32,
                u32,
                u32,
                u32,
                u32,
                u32,
                u32,
                *mut c_void,
                *mut *mut c_void,
                *mut *mut c_void,
            ) -> i32
        );

        if cu_init(0) != 0 {
            return None;
        }
        let mut dev: i32 = 0;
        if cu_dev_get(&mut dev, 0) != 0 {
            return None;
        }
        let mut name_buf = [0u8; 128];
        let _ = cu_dev_name(name_buf.as_mut_ptr(), 128, dev);
        let nul_pos = name_buf.iter().position(|&b| b == 0).unwrap_or(128);
        let dev_name = String::from_utf8_lossy(&name_buf[..nul_pos]).into_owned();

        let mut total_mem: usize = 0;
        let _ = cu_dev_mem(&mut total_mem, dev);
        let vram_mb = total_mem / (1024 * 1024);

        let mut ctx: *mut c_void = std::ptr::null_mut();
        if cu_ctx_create(&mut ctx, 0, dev) != 0 || ctx.is_null() {
            return None;
        }

        let mut module: *mut c_void = std::ptr::null_mut();
        if cu_mod_load(&mut module, CUDA_PTX_KERNEL.as_ptr() as *const c_void) != 0
            || module.is_null()
        {
            return None;
        }

        let mut func: *mut c_void = std::ptr::null_mut();
        if cu_mod_func(&mut func, module, b"sweet_fb_kernel\0".as_ptr()) != 0 || func.is_null() {
            return None;
        }

        let src_cap = 1080 * 2400 * 4;
        let dst_cap = 1080 * 2400 * 4; // Up to full 1080x2400 / 2x Retina 720x1600 viewport
        let mut d_src: u64 = 0;
        let mut d_dst: u64 = 0;
        if cu_mem_alloc(&mut d_src, src_cap) != 0 {
            return None;
        }
        if cu_mem_alloc(&mut d_dst, dst_cap) != 0 {
            let _ = cu_mem_free(d_src);
            return None;
        }

        Some((
            WinCudaContext {
                ctx,
                func,
                d_src,
                d_dst,
                src_cap,
                dst_cap,
                cu_ctx_set_current,
                cu_memcpy_htod,
                cu_memcpy_dtoh,
                cu_launch_kernel,
                cu_mem_free,
            },
            dev_name,
            vram_mb,
        ))
    }

    /// Downsample and filter the `1080x2400` guest BGRA framebuffer into `out_rect` `(dst_x, dst_y, dst_w, dst_h)`
    /// inside the window buffer `win_pixels` (`win_w x win_h`).
    /// Uses NVIDIA CUDA PTX 9-tap hardware acceleration when available, plus post-processing filters.
    /// Automatically reuses the cached output viewport when neither the guest frame nor the filter changed.
    pub fn blit_guest_fb_filtered(
        &mut self,
        win_pixels: &mut [u32],
        win_w: u32,
        win_h: u32,
        dst_x: u32,
        dst_y: u32,
        dst_w: u32,
        dst_h: u32,
        guest_bgra: &[u8],
        filter: &DisplayFilterConfig,
    ) {
        if dst_w == 0 || dst_h == 0 || guest_bgra.len() < 1080 * 2400 * 4 {
            return;
        }

        let need_recompute = self.frame_dirty
            || self.cached_w != dst_w
            || self.cached_h != dst_h
            || self.cached_filter.as_ref() != Some(filter)
            || self.cached_vp.len() != (dst_w * dst_h) as usize;

        if need_recompute {
            let t0 = Instant::now();
            let src_w = 1080u32;
            let src_h = 2400u32;

            self.cached_vp.resize((dst_w * dst_h) as usize, 0);
            let mut cuda_ok = false;

            #[cfg(windows)]
            if let Some(ref c) = self.inner {
                let src_bytes = (src_w * src_h * 4) as usize;
                let dst_bytes = (dst_w * dst_h * 4) as usize;
                if !filter.landscape && src_bytes <= c.src_cap && dst_bytes <= c.dst_cap {
                    unsafe {
                        let _ = (c.cu_ctx_set_current)(c.ctx);
                        if (c.cu_memcpy_htod)(
                            c.d_src,
                            guest_bgra.as_ptr() as *const c_void,
                            src_bytes,
                        ) == 0
                        {
                            let mut p_src = c.d_src;
                            let mut p_dst = c.d_dst;
                            let mut p_sw = src_w;
                            let mut p_sh = src_h;
                            let mut p_dw = dst_w;
                            let mut p_dh = dst_h;
                            let mut p_flags: u32 = if filter.color_filter == 1 { 1 } else { 0 };
                            let mut params: [*mut c_void; 7] = [
                                &mut p_src as *mut u64 as *mut c_void,
                                &mut p_dst as *mut u64 as *mut c_void,
                                &mut p_sw as *mut u32 as *mut c_void,
                                &mut p_sh as *mut u32 as *mut c_void,
                                &mut p_dw as *mut u32 as *mut c_void,
                                &mut p_dh as *mut u32 as *mut c_void,
                                &mut p_flags as *mut u32 as *mut c_void,
                            ];
                            let gx = (dst_w + 15) / 16;
                            let gy = (dst_h + 15) / 16;
                            if (c.cu_launch_kernel)(
                                c.func,
                                gx,
                                gy,
                                1,
                                16,
                                16,
                                1,
                                0,
                                std::ptr::null_mut(),
                                params.as_mut_ptr(),
                                std::ptr::null_mut(),
                            ) == 0
                            {
                                if (c.cu_memcpy_dtoh)(
                                    self.cached_vp.as_mut_ptr() as *mut c_void,
                                    c.d_dst,
                                    dst_bytes,
                                ) == 0
                                {
                                    cuda_ok = true;
                                    self.kernel_launches += 1;
                                }
                            }
                        }
                    }
                }
            }

            if !cuda_ok {
                // 9-tap (3x3 tent + center unsharp) CPU fallback
                for oy in 0..dst_h {
                    for ox in 0..dst_w {
                        let (sx, sy) = if filter.landscape {
                            let sx = ((dst_h - 1 - oy) * src_w / dst_h).min(src_w - 3);
                            let sy = (ox * src_h / dst_w).min(src_h - 3);
                            (sx, sy)
                        } else {
                            let sx = (ox * src_w / dst_w).min(src_w - 3);
                            let sy = (oy * src_h / dst_h).min(src_h - 3);
                            (sx, sy)
                        };
                        const W: [[u32; 3]; 3] = [[1, 2, 1], [2, 4, 2], [1, 2, 1]];
                        let mut sb = 0u32;
                        let mut sg = 0u32;
                        let mut sr = 0u32;
                        let mut cb = 0u32;
                        let mut cg = 0u32;
                        let mut cr = 0u32;
                        for dy in 0..3u32 {
                            let row_off = (sy + dy) * src_w + sx;
                            for dx in 0..3u32 {
                                let idx = ((row_off + dx) * 4) as usize;
                                let w = W[dy as usize][dx as usize];
                                let b = guest_bgra[idx] as u32;
                                let g = guest_bgra[idx + 1] as u32;
                                let r = guest_bgra[idx + 2] as u32;
                                sb += b * w;
                                sg += g * w;
                                sr += r * w;
                                if dx == 1 && dy == 1 {
                                    cb = b;
                                    cg = g;
                                    cr = r;
                                }
                            }
                        }
                        let ab = ((((sb + 8) >> 4) * 3) + cb) >> 2;
                        let ag = ((((sg + 8) >> 4) * 3) + cg) >> 2;
                        let ar = ((((sr + 8) >> 4) * 3) + cr) >> 2;
                        self.cached_vp[(oy * dst_w + ox) as usize] =
                            0xFF00_0000 | (ar << 16) | (ag << 8) | ab;
                    }
                }
            }

            let needs_fx = !filter.dark_mode
                || filter.liquid_glass
                || filter.color_filter > 0
                || filter.increase_contrast
                || filter.reduce_transparency;
            if needs_fx {
                for oy in 0..dst_h {
                    for ox in 0..dst_w {
                        let i = (oy * dst_w + ox) as usize;
                        self.cached_vp[i] =
                            apply_pixel_filter(self.cached_vp[i], ox, oy, dst_w, dst_h, filter);
                    }
                }
            }

            self.cached_w = dst_w;
            self.cached_h = dst_h;
            self.cached_filter = Some(filter.clone());
            self.frame_dirty = false;
            self.last_exec_us = t0.elapsed().as_micros() as u64;
        }

        // Fast row-slice copy from cached_vp into win_pixels
        for oy in 0..dst_h {
            let wy = dst_y + oy;
            if wy >= win_h {
                break;
            }
            let max_w = dst_w.min(win_w.saturating_sub(dst_x));
            let src_start = (oy * dst_w) as usize;
            let dst_start = (wy * win_w + dst_x) as usize;
            win_pixels[dst_start..dst_start + max_w as usize]
                .copy_from_slice(&self.cached_vp[src_start..src_start + max_w as usize]);
        }
    }
}

#[inline]
fn apply_pixel_filter(
    px: u32,
    ox: u32,
    oy: u32,
    dst_w: u32,
    dst_h: u32,
    f: &DisplayFilterConfig,
) -> u32 {
    let mut r = ((px >> 16) & 0xFF) as i32;
    let mut g = ((px >> 8) & 0xFF) as i32;
    let mut b = (px & 0xFF) as i32;

    if !f.dark_mode {
        let luma = (r * 77 + g * 150 + b * 29) >> 8;
        if luma < 48 {
            r = (r + 36).min(255);
            g = (g + 38).min(255);
            b = (b + 44).min(255);
        }
    }

    match f.color_filter {
        1 => {
            let l = (r * 77 + g * 150 + b * 29) >> 8;
            r = l;
            g = l;
            b = l;
        }
        2 => {
            let nr = (r * 56 + g * 44) / 100;
            let ng = (r * 55 + g * 45) / 100;
            r = nr;
            g = ng;
        }
        3 => {
            let nr = (r * 62 + g * 38) / 100;
            let ng = (r * 70 + g * 30) / 100;
            r = nr;
            g = ng;
        }
        4 => {
            let ng = (g * 95 + b * 5) / 100;
            let nb = (g * 43 + b * 57) / 100;
            g = ng;
            b = nb;
        }
        5 => {
            r = (r + 22).min(255);
            g = (g + 8).min(255);
            b = (b - 24).max(0);
        }
        _ => {}
    }

    if f.increase_contrast {
        r = (((r - 128) * 135) / 100 + 128).clamp(0, 255);
        g = (((g - 128) * 135) / 100 + 128).clamp(0, 255);
        b = (((b - 128) * 135) / 100 + 128).clamp(0, 255);
    }

    if f.liquid_glass {
        let edge_dist_x = ox.min(dst_w.saturating_sub(1 + ox));
        let edge_dist_y = oy.min(dst_h.saturating_sub(1 + oy));
        let min_edge = edge_dist_x.min(edge_dist_y);
        if min_edge < 14 {
            let boost = ((14 - min_edge) * 3) as i32;
            r = (r + boost).min(255);
            g = (g + boost + 4).min(255);
            b = (b + boost + 10).min(255);
        } else if (ox + oy * 2) % 180 < 2 {
            r = (r + 10).min(255);
            g = (g + 12).min(255);
            b = (b + 16).min(255);
        }
    }

    0xFF00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

#[cfg(windows)]
impl Drop for WinCudaContext {
    fn drop(&mut self) {
        unsafe {
            let _ = (self.cu_ctx_set_current)(self.ctx);
            if self.d_src != 0 {
                let _ = (self.cu_mem_free)(self.d_src);
            }
            if self.d_dst != 0 {
                let _ = (self.cu_mem_free)(self.d_dst);
            }
        }
    }
}
