# emulator-ios

**Trình giả lập Apple iOS 16.7.16 (`20H392` — iPhone X `iPhone10,3` / `D22AP`) chạy trực tiếp trên Windows PC (QEMU ARM64 + Rust Device Hub GUI + NVIDIA CUDA + Direct-RAM Zero-Copy Bridge).**

Dự án này được phát triển dựa trên nền tảng nghiên cứu port nhân XNU (`xnu-8796`) từ dự án `ios-sweet` (vốn ban đầu chạy trên phần cứng Xiaomi Redmi Note 10 Pro), sau đó được **viết lại, mở rộng và tối ưu hóa toàn diện để khởi động và vận hành trực tiếp hệ điều hành iOS 16.7.16 nguyên bản trên máy tính Windows**.

Toàn bộ không gian người dùng (userland) của **iPhone X iOS 16.7.16 (`20H392`)** — bao gồm `launchd`, hệ thống XPC daemons, `backboardd`, `SpringBoard`, Lock Screen, Home Screen, Today View, Control Center và các ứng dụng gốc (`Settings / Preferences.app`, `App Store`, `Safari`, `Messages`, `Photos`) — khởi động trực tiếp vào giao diện **Device Hub Studio (`sweet-emu.exe`)** trên Windows với tăng tốc phần cứng **NVIDIA CUDA (`nvcuda.dll`)** và cơ chế đọc/ghi bộ nhớ RAM trực tiếp không qua QMP (`sweet-bridge`).

---

## Tính năng hoạt động trên Windows

| Thành phần | Trạng thái hoạt động |
| --- | --- |
| **Giao diện Studio (`sweet-emu.exe`)** | Giao diện `Device Hub` 3 cột (`1240×900`, backbuffer 2x Retina `2480×1800`, khung hiển thị iPhone X `720×1600`) viết bằng Rust (`Win32`), tự động khởi chạy máy ảo QEMU khi mở `.exe` |
| **Tăng tốc GPU (`sweet-gpu`)** | Kernel PTX 6.0 (`sm_50`) nạp trực tiếp qua NVIDIA Driver API (`nvcuda.dll`), xử lý nội suy 9-tap tent filter `1080×2400 -> 720×1600`, unsharp contrast, chế độ Sáng/Tối, Liquid Glass, Color Filters trực tiếp trên GPU NVIDIA (đạt `~1.5 ms`/khung hình trên RTX 5050) |
| **Zero-Copy Direct-RAM Bridge (`sweet-bridge`)** | Đọc biến đếm `swap_count` (`0.46 µs`), đọc trực tiếp `10.36 MB` framebuffer (`0x9c000000`) từ tiến trình `qemu-system-aarch64.exe` (`~1.9 ms`) bằng `ReadProcessMemory` và bơm sự kiện cảm ứng trực tiếp vào ring buffer `'SWIN'` (`0x9d600000`) bằng `ntdll!NtWriteVirtualMemory` (`~3 µs`) — không tốn bất kỳ lệnh QMP hay khóa Big QEMU Lock (BQL) nào |
| **CPU & Đa nhân SMP** | Khởi chạy đủ **8/8 nhân ARM64** (`cpu0..cpu7`, `SWEET CPU: 8/8 CPUs initialized, SGI1 verified`) trên QEMU 11.1.0 `virt` (`-accel tcg,thread=multi,tb-size=1024`) |
| **Bộ nhớ RAM (`phys_base = 0xa0000000`)** | Tối ưu ánh xạ địa chỉ vật lý từ `0xa0000000`, cung cấp **5.17+ GB RAM thực tế** cho XNU (`mem_size = 0x134000000`) cùng giới hạn bộ nhớ `SweetSurface` lên tới **4.0 GB** |
| **Lưu trữ (`VirtIO-blk` + HFS+)** | Driver `SweetVirtIOBlk` trong XNU đọc/ghi trực tiếp phân vùng gốc `ios16-root-20H392.img` (HFS+ case-sensitive), kèm bộ đồng bộ B-Tree & Allocation Bitmap tự động trong `tools/qemu-sweet.py` |
| **Đồ họa (`IOSurface` + `IOMobileFramebuffer`)** | Toàn bộ `SpringBoard`, `backboardd`, `CABackdropLayer`, `CAMetalLayer` được patch trong `QuartzCore` (`dyld_shared_cache_arm64.03`) để xuất trực tiếp định dạng **1-plane 8-bit `'BGRA'` (`0x42475241`, stride căn lề 256-byte)**, loại bỏ hoàn toàn bước chuyển đổi phần mềm 2-plane `'b3a8'` và 10-bit `'w30r'` trên CPU |
| **Cảm ứng, Bàn phím PC & Cử chỉ** | Hỗ trợ chạm, cuộn 120Hz real-time preview, vuốt cạnh dưới về Home / App Switcher (`kIOHIDDigitizerEventFromEdgeTip`), nút Back điều hướng `<`, cuộn con lăn chuột và gõ phím trực tiếp từ bàn phím PC sang bàn phím ảo iOS 16 |
| **Cài đặt `.IPA` & TrustCache (`sweet-core`)** | Giải nén `.ipa`, tự động băm CDHash vào `sweet_trustcache.bin` và đồng bộ thẳng vào image HFS+ trên Windows |

---

## Kiến trúc khởi động & truyền tải khung hình trên Windows

```text
sweet-emu.exe (Rust Win32 Device Hub Studio GUI + NVIDIA CUDA PTX Renderer)
  │
  ├── Khởi chạy tự động tools/qemu-sweet.py -> qemu-system-aarch64.exe (-M virt, 8 vCPUs, 5.17+ GB Usable RAM)
  │     └── Boot stub tại 0x40200000 -> payload_virtio.bin (XNU 8796 + qemu.adt D22AP + boot-args)
  │           └── XNU Kernel -> SweetVirtIOBlk -> ios16-root-20H392.img (HFS+ R/W)
  │                 └── launchd -> backboardd + SpringBoard + Preferences / Safari / AppStore
  │
  └── sweet-bridge (Direct-RAM Zero-BQL I/O):
        ├── Frame Poller (250 Hz): ReadProcessMemory(SWAP_COUNT_PA @ 0xa2072020)
        │     └── Khi có frame mới -> ReadProcessMemory(FB_BASE @ 0x9c000000, 10.36 MB) -> CUDA PTX Blit
        ├── Log Reader: ReadProcessMemory(LOG_RING_PA @ 0xa1c52718) -> Live XNU Console
        └── Input Writer: NtWriteVirtualMemory('SWIN' Ring @ 0x9d600000) -> _touch_thread (6ms / 16 events) -> SweetHIDTouch
```

---

## Hướng dẫn chạy trên Windows

### 1. Yêu cầu môi trường
* **Hệ điều hành**: Windows 10 / Windows 11 (64-bit).
* **QEMU**: `qemu-system-aarch64.exe` (khuyến nghị QEMU 11.x trở lên tại `C:\Program Files\qemu\`).
* **Rust Toolchain**: `cargo` / `rustc` (`x86_64-pc-windows-msvc`) để biên dịch workspace `sweet-rs`.
* **Python 3.10+**: Dùng cho script khởi tạo payload/ADT và đồng bộ HFS+ (`tools/qemu-sweet.py`).
* **GPU (Tùy chọn nhưng khuyến nghị)**: Card đồ họa NVIDIA hỗ trợ CUDA (`nvcuda.dll`) để kích hoạt bộ dựng hình PTX phần cứng (tự động fallback sang bộ dựng đa luồng SIMD/CPU nếu không có GPU NVIDIA).

### 2. Chuẩn bị `emu_workspace/`
Đặt các thành phần runtime vào thư mục `emu_workspace/` (thư mục này được loại trừ khỏi Git theo `.gitignore` vì chứa dữ liệu hệ điều hành của Apple):
* `emu_workspace/kernel.qemu.sweet`: Nhân XNU đã áp dụng chuỗi patch `0001..0082` và các bản vá tối ưu QEMU `virt` / Direct-RAM.
* `emu_workspace/ios16-root-20H392.img`: Phân vùng gốc HFS+ được tạo từ IPSW iPhone X (`iPhone10,3`, iOS `16.7.16` build `20H392`).

Tạo lại Apple Device Tree (`qemu.adt`) và `payload_virtio.bin` (8 nhân, `phys_base = 0xa0000000`):
```powershell
python tools/mkadt.py emu_workspace/qemu.adt --qemu --cpus 8 --highmem
python tools/mkpayload.py emu_workspace/kernel.qemu.sweet emu_workspace/qemu.adt emu_workspace/payload_virtio.bin --phys-base 0xa0000000 --cmdline "debug=0xc serial=3 cpus=8 tlto=0 keepsyms=1 -enable_kprintf_spam -noprogress rd=ufsroot0 unentitled_ios_sim_launch=1 ignition_level=1 wdt_test=no_user_checkin ufsrw=1 rootrw=1 launchd_unsecure_cache=1 wdt=-1 sweetlog=0 sweetfb_noconsole=1 sweetfb_hz=60 sweetiolog=- sweetsample=- sweetnull=backboardd,SpringBoard"
```

### 3. Biên dịch và khởi chạy `sweet-emu.exe`
Chạy trực tiếp bằng file `.bat` ở thư mục gốc:
```powershell
.\run-emulator.bat
```
Hoặc biên dịch bản `release` tối ưu hóa từ `sweet-rs`:
```powershell
cargo build --manifest-path sweet-rs/Cargo.toml --release --bins
.\sweet-rs\target\release\sweet-emu.exe
```

### 4. Công cụ CLI điều khiển & benchmark (`sweet-bridge.exe`)
Ngoài giao diện đồ họa `sweet-emu.exe`, có thể kiểm tra tốc độ đọc RAM trực tiếp hoặc gửi thao tác chạm/vuốt qua CLI:
```powershell
.\sweet-rs\target\release\sweet-bridge.exe status
.\sweet-rs\target\release\sweet-bridge.exe bench
.\sweet-rs\target\release\sweet-bridge.exe tap 655 290
.\sweet-rs\target\release\sweet-bridge.exe swipe 540 2385 540 180 280
.\sweet-rs\target\release\sweet-bridge.exe home
```

---

## Cấu trúc thư mục dự án

| Đường dẫn | Mô tả |
| --- | --- |
| `sweet-rs/crates/sweet-emu/` | Ứng dụng GUI chính `sweet-emu.exe` trên Windows (giao diện `Device Hub` 3 cột, quản lý vòng đời máy ảo, xử lý sự kiện chuột/bàn phím và hiển thị màn hình iPhone X 120Hz) |
| `sweet-rs/crates/sweet-gpu/` | Engine dựng hình tăng tốc phần cứng **NVIDIA CUDA (`nvcuda.dll` + PTX 6.0 kernel)**, bộ lọc màu, Liquid Glass và kết xuất font TrueType |
| `sweet-rs/crates/sweet-bridge/` | Dịch vụ cầu nối tốc độ cao Host <-> Guest (`ReadProcessMemory` / `NtWriteVirtualMemory`) giao tiếp trực tiếp với RAM của QEMU không qua QMP |
| `sweet-rs/crates/sweet-core/` | Quản lý tiến trình QEMU trên Windows (`QemuRunner`), trình tạo Apple Device Tree (`adt.rs`), bộ cài đặt `.ipa` & TrustCache (`ipa.rs`) |
| `tools/qemu-sweet.py` | Trình khởi chạy QEMU `virt` trên Windows, đồng bộ hóa B-Tree/Bitmap phân vùng HFS+, tinh chỉnh `launchd.plist` và quản lý cấu hình máy ảo |
| `tools/mkadt.py`, `tools/mkpayload.py` | Công cụ đóng gói Apple Device Tree (`D22AP`) và tạo payload khởi động cho QEMU `virt` |
| `xnu/patches/` | Chuỗi **82 bản vá nhân XNU (`0001..0082`)**, bao gồm `0046-sweet-iosurface-iomfb.patch` (`SweetSurface` + `AppleCLCD`), `0058-sweet-hid-event-services.patch` (`IOHIDFamily`) và `0082-sweet-qemu-virt-emulator.patch` (hỗ trợ máy ảo QEMU `virt`, `SweetVirtIOBlk`, shared-memory `'SWIN'` touch ring) |
| `shim/` | Bootloader shim nguyên bản từ nhánh phần cứng `sweet` (lưu giữ để tham khảo và tương thích ngược) |
| `NOTES.md` | Nhật ký kỹ thuật chi tiết từng giai đoạn phát triển, địa chỉ bản vá nhị phân (`kernel.qemu.sweet`, `dyld_shared_cache`) và kết quả đo kiểm |

---

## Các tối ưu hóa kỹ thuật trọng tâm trên bản Windows

1. **Loại bỏ nghẽn QMP & Big QEMU Lock (BQL)**: Thay thế hoàn toàn cơ chế chụp màn hình qua QMP `pmemsave` và gửi cảm ứng qua UART bằng kỹ thuật ánh xạ trực tiếp vùng nhớ ảo của tiến trình `qemu-system-aarch64.exe` trên Windows (`VirtualQueryEx` + `ReadProcessMemory` + `NtWriteVirtualMemory`).
2. **Ép định dạng `'BGRA'` (`0x42475241`) toàn cục trong `QuartzCore`**: Vá 29 điểm trong `dyld_shared_cache_arm64.03` (kèm cập nhật mã băm SHA-256 `CS_CodeDirectory` cho từng trang 16 KiB) để vô hiệu hóa `HasExtendedColorDisplay` và nén AGX, giúp `CARenderServer` xuất thẳng bộ đệm 1-plane 8-bit `'BGRA'` thay vì tốn CPU giải nén 2-plane `'b3a8'` / 10-bit `'w30r'`.
3. **Căn lề dòng 256-byte (`SWEET_SURFACE_ROW_ALIGN = 256`) & vá `CoreMaterial`**: Khắc phục triệt để lỗi sọc chéo 5 dải trên Today View và khối xám `CABackdropLayer` khi không có shader GPU Metal.
4. **Sửa lỗi `Ticket spinlock timeout` dưới môi trường giả lập TCG**: Vô hiệu hóa bộ đếm timeout `48 ms` (`3,000,000` ticks @ `62.5 MHz`) trong XNU để nhân không bị panic khi luồng vCPU trên Windows bị lập lịch trễ.
5. **Tối ưu hóa hàng đợi sự kiện cảm ứng `'SWIN'`**: Gộp (`coalesce`) các sự kiện `TouchMove` ngay trong ring buffer `0x9d600000` và nâng deadzone nhận diện chạm icon (`TAP_DEADZONE_IOS_PX = 54`), kết hợp bộ xem trước cuộn 120Hz (`apply_live_scroll_preview`) ngay trên cửa sổ Windows.

---

## Ghi nhận (Credits)

* Nền tảng nghiên cứu port nhân XNU ban đầu được học hỏi từ dự án **`ios-sweet`** (port XNU 8796 lên Xiaomi Redmi Note 10 Pro).
* Phát triển dựa trên mã nguồn mở của Apple (`xnu`, `IOHIDFamily`, `libpthread`, `hfs`), [BearSSL](https://bearssl.org), [blacktop/darwin-xnu-build](https://github.com/blacktop/darwin-xnu-build), [ipsw](https://github.com/blacktop/ipsw) và [QEMU](https://www.qemu.org).
* Hệ điều hành iOS và các tệp韌體 (IPSW / `dyld_shared_cache`) thuộc bản quyền của Apple Inc. và không được phân phối kèm trong kho mã nguồn này.
