# Từ ios-sweet tới một "giả lập iOS" kiểu BlueStacks

Tài liệu này trả lời ba câu hỏi, theo thứ tự: **dự án hiện tại là gì**, **muốn có giả lập kiểu
BlueStacks thì phải làm những gì**, và **những repo open-source nào có thể vay code/học cách làm**.
Mọi kết luận về code đều trỏ tới file trong repo này; mọi kết luận về repo ngoài đều trỏ tới link
và được ghi rõ là "quan sát từ tài liệu công khai".

> Đọc kèm: [`NOTES.md`](../NOTES.md) (nhật ký bring-up chi tiết) và
> [`QEMU_TESTING.md`](QEMU_TESTING.md) (cách chạy QEMU hiện có).

---

## 1. Hiện trạng: dự án này KHÔNG phải một emulator

`ios-sweet` đưa XNU (kernel iOS 16, nguồn mở) lên **một chiếc điện thoại thật** — Redmi Note 10 Pro
(`sweet`, SM7150) — với userland thật của iPhone X (iOS 16.7.16). Xem [README.md](../README.md).

Đường boot hiện tại:

```
ABL (bootloader Xiaomi, đã unlock)
  -> fastboot boot shim-boot.img                 (tạm thời, không flash gì vào boot)
     -> shim: tự setup MMU, đọc FDT sống của máy, nạp payload từ ramdisk
        -> XNU Mach-O + ADT (Apple Device Tree do mkadt.py sinh) + boot-args
           -> XNU -> driver UFS -> HFS+ root trong phân vùng userdata
              -> dyld + shared cache -> launchd -> backboardd + SpringBoard
```

Ba đặc điểm quyết định mọi thứ phía sau:

1. **Phần cứng là thật.** UFS, Goodix GT9896, PMIC/SPMI, GENI I²C, ABL framebuffer — driver viết
   cho đúng các khối SM7150 đó (patch 0028–0034, 0060–0063 trong [xnu/patches](../xnu/patches)).
2. **Không có GPU.** CoreAnimation render bằng CPU (~20–30 fps), frame được copy vào framebuffer mà
   ABL để lại ở `0x9c000000` (patch 0046, 0056; [README.md](../README.md#L20)).
3. **QEMU chỉ là công cụ test kernel.** [tools/qemu-sweet.py](../tools/qemu-sweet.py) chạy board
   `virt` với:
   - `-nographic`, `-smp 1` ([dòng 276–281](../tools/qemu-sweet.py#L276-L281));
   - boot_args dựng thủ công trong [`build_image()`](../tools/qemu-sweet.py#L92-L95): framebuffer là
     **vùng RAM** `0x9c000000` 1080×2400, cuối run dump ra `fb.png`
     ([dòng 307](../tools/qemu-sweet.py#L307));
   - boot-args `rd=md0` → root là **ramdisk**, và script **không attach `-drive` nào**;
   - ADT sinh bởi `mkadt.py --qemu` chỉ khác GIC/timer, còn lại vẫn mô tả phần cứng sweet.

Nói cách khác: QEMU hiện tại chứng minh **kernel chạy được trên nền ảo hoá** ở mức "boot + console",
nhưng chưa có storage thật, chưa có hiển thị sống, chưa có input. Đó chính là 3 khoảng trống lớn nhất.

---

## 2. "Giống BlueStacks" thực sự nghĩa là gì

| BlueStacks (Android) | iOS | Ghi chú |
| --- | --- | --- |
| Android x86/ARM image chạy trên hypervisor của Windows | XNU **chỉ có arm64**, không có bản x86 | Trên PC x86 buộc phải giả lập ARM (TCG) → rất chậm, hoặc không chạy |
| GPU ảo + DirectX/OpenGL passthrough | iOS 16 cần Metal; repo chỉ có đường CPU render | Không có Metal trên Windows → phải viết lớp dịch (rất lớn) |
| Cài APK, Play Store | Không có App Store/Apple ID; AMFI và sandbox đã tắt | "Cài app" hiện chỉ là thêm binary vào root + trustcache |
| AOSP công khai, phát hành image | Userland iOS là bản sao lưu từ IPSW, có bản quyền | Không được phân phối kèm; người dùng phải tự trích IPSW (repo cũng làm vậy) |
| Boot 5–20 giây | ~4 phút trên máy thật (CPU render, UFS) | Trong QEMU TCG sẽ còn chậm hơn nhiều |

**Kết luận định hướng:**

- **PC x86 (Windows/Linux):** không có đường "native". Chỉ có QEMU ARM64 + TCG. Mục tiêu thực tế là
  *một cửa sổ QEMU boot được SpringBoard*, không phải sản phẩm dùng hằng ngày.
- **PC ARM64 (Snapdragon X / Apple Silicon):** QEMU aarch64 với HVF có thể nhanh gần native. Đây là
  nền tảng hợp lý nếu muốn "dùng được".
- **macOS Apple Silicon:** nếu mục tiêu chỉ là *dùng* iOS ảo, `Virtualization.framework` (vphone-cli)
  ngắn hơn nhiều so với tự viết emulator — xem mục 5.

---

## 3. Khoảng trống kỹ thuật và cách lấp

Bảng dưới là "definition of done" theo từng mốc. Cột **Hiện có** chỉ file/patch trong repo này.

### Mốc A — QEMU boot được root HFS+ thật (không còn ramdisk)

Đây là mốc rẻ nhất và mở khoá mọi thứ sau nó: có đĩa ghi được thì launchd mới chạy được userland.

| Hạng mục | Hiện có | Cần làm |
| --- | --- | --- |
| Block device | `sweet_ufs.c` (patch 0028–0034): UFS host thật, mailbox/DMA/descriptor | Viết driver BSD mới `sweet_vblk.c` cùng contract `bdevsw`/`disk`, đăng ký tên `ufsroot0` để boot-arg `rd=ufsroot0` không phải đổi |
| Transport | — | Cách 1: virtio-blk (MMIO, có sẵn trong QEMU `virt`). Cách 2: một MMIO device tối giản tự viết trong QEMU fork (dễ debug hơn) |
| Attach ảnh | [qemu-sweet.py](../tools/qemu-sweet.py) không có `-drive` | Thêm `-drive if=none,format=raw,file=ios16-root.img` + `-device virtio-blk-device,drive=...` |
| ADT | `mkadt.py --qemu` vẫn mô tả `/arm-io/ufs*` | Thêm node ảo (compatible riêng) + trỏ `ufsroot0` tới đó |
| Root image | `mkiosroot.sh` tạo HFS+ từ IPSW | Dùng lại; kiểm tra block size/HFS+ journal mà driver ảo phải hiểu (repo đã dùng HFS+ có journal, patch 0026) |

Ghi chú thực dụng: driver ảo **không cần** mô phỏng UFS. Chỉ cần đọc/ghi sector đúng thứ tự và tôn
trọng flush/barrier; phần HFS+ trong kernel không quan tâm bên dưới là gì.

### Mốc B — Hiển thị ra cửa sổ QEMU (thay vì dump `fb.png`)

| Hạng mục | Hiện có | Cần làm |
| --- | --- | --- |
| Nguồn frame | IOSurface + AppleCLCD (patch 0046, 0056): CPU render, copy vào `0x9c000000` mỗi 10 Hz | Giữ nguyên logic; chỉ đổi **đích** |
| Đích | RAM thường | RAM của một QEMU device (shared memory) hoặc framebuffer do virtio-gpu/ramfb cấp |
| QEMU side | Chỉ `pmemsave` cuối run | Thêm device hiển thị (ramfb là cách nhanh nhất) hoặc virtio-gpu + `-display gtk/sdl` |
| Kích thước | 1080×2400 (panel thật), iOS thấy 1125×2436 | Scale trong host window; giữ nguyên hình học để touch map đúng |

Cách nhanh nhất để "thấy gì đó": dùng `-device ramfb` với vùng RAM lấy từ boot-args, rồi sửa
`phys_base_fb` trong shim/ADT trỏ vào đó. Cách "đúng" lâu dài: virtio-gpu-gl (dù không có tăng tốc 3D
cho CoreAnimation, nhưng có scanout và input đi kèm).

### Mốc C — Touch/chuột/bàn phím

| Hạng mục | Hiện có | Cần làm |
| --- | --- | --- |
| Đường vào iOS | `sweet_hid_touch()`, `sweet_hid_key()` → `SweetHIDTouch`/`SweetHIDButtons` (patch 0058) | **Giữ nguyên** — đây là phần khớp userland thật, rất có giá trị |
| Driver nguồn | Goodix qua GENI I²C (patch 0060, 0063), PMIC PON (0060) | Thay bằng virtio-input (tablet/keyboard) hoặc device ảo gọi thẳng `sweet_hid_touch/key` |
| Mapping | 1:1 pixel, có xử lý vùng biên (patch 0071) | Map toạ độ con trỏ → toạ độ panel 1080×2400 |

Bài học từ patch 0071: toạ độ và `kIOHIDDigitizerEventFromEdgeTip` ảnh hưởng trực tiếp tới gesture
home của iPhone X — giả lập phải phát đúng bit này, nếu không "vuốt về home" sẽ không hoạt động.

### Mốc D — Bỏ/thay driver phần cứng không có trong VM

Danh sách phải stub hoặc thay (mỗi cái đều có nguy cơ daemon crash-loop):

| Khối | Patch hiện tại | Thay bằng |
| --- | --- | --- |
| RPMh regulator | 0062 | Vote giả luôn "on" |
| SPMI/PMIC, PON keys | 0060 | virtio-input / bàn phím ảo |
| GENI I²C (touch, PMIC phụ) | 0060, 0063 | Bỏ hẳn |
| USB DWC3 device mode | 0072–0077 (CDC-ECM + `ue0`) | virtio-net, và map ifnet sang `en0` để configd nhận |
| UFS | 0028–0034 | Mốc A |
| Panel/DSI | 0061 | Mốc B |
| Battery | 0062 (`SweetBattery`) | Giữ, số liệu tĩnh |
| Watchdog | 0035–0045 | Giữ (`wdt=-1` đang dùng) |

### Mốc E — SMP, timer, interrupt trên `virt`

- Patch 0065 (PSCI) và 0078 (8 core) dùng PSCI + GICv3 — QEMU `virt,gic-version=3` cũng vậy, nên
  phần lớn tái dùng được. **Khác biệt đã ghi trong [NOTES.md](../NOTES.md#L340-L348):** máy thật dùng
  conduit `smc`, QEMU `virt` dùng `hvc` → kernel phải chọn theo ADT (hoặc boot-arg), đây là thứ đầu
  tiên sẽ chặn `-smp 6` nếu bỏ qua. Nhưng [qemu-sweet.py](../tools/qemu-sweet.py#L276) đang `-smp 1`; muốn
  hết "input latency ~1 phút" phải chạy `-smp 6/8` và cho `mkadt.py` phát đủ CPU node + GICR.
- Timer: máy thật 19.2 MHz; QEMU `virt` dùng CNTFRQ 62.5 MHz. Kiểm tra `sweet` timer code có đọc
  CNTFRQ từ ADT hay hard-code.
- Hướng dẫn hiện tại ([QEMU_TESTING.md](QEMU_TESTING.md)) vẫn ghi `cpus=1` — cập nhật khi Mốc E xong.

### Mốc F — Đóng gói "app" (phần sản phẩm, chưa có trong repo)

Nếu thực sự muốn trải nghiệm kiểu BlueStacks:

1. **GUI host**: Electron/Qt bọc `qemu-system-aarch64`, nút Play/Stop, snapshot (`savevm`), chọn ảnh,
   mapping phím → touch, scale cửa sổ.
2. **Onboarding**: người dùng tự cung cấp IPSW (không bundle iOS). Repo đã có `mkiosroot.sh`,
   `sweet-root-tweaks.py` để tự động hoá; cần thêm luồng "chọn IPSW → tạo root image".
3. **Hợp pháp**: không phân phối rootfs, dyld cache, hay firmware Apple. Ghi rõ trong UI.
4. **Kỳ vọng hiệu năng**: nêu rõ "chạy được để nghiên cứu", không hứa 60 fps / App Store.

---

## 4. Trình tự đề xuất (theo giá trị/chi phí)

| Bước | Việc | Vì sao trước |
| --- | --- | --- |
| 1 | `-drive` + virtio-blk driver ảo, boot HFS+ thật | Không có đĩa thì không có userland trong QEMU; mọi thứ khác vô nghĩa |
| 2 | `-smp 6` + ADT đủ CPU | Boot nhanh hơn nhiều lần, giảm thời gian debug (hiện 1 CPU → input latency ~1 phút trên máy thật) |
| 3 | ramfb/virtio-gpu → cửa sổ QEMU | Nhìn thấy SpringBoard trong QEMU; verify Mốc A đúng |
| 4 | virtio-input → `sweet_hid_touch/key` | Điều khiển được, test gesture |
| 5 | Stub RPMh/SPMI/GENI, virtio-net thay `ue0` | Dọn crash-loop, có mạng |
| 6 | GUI host + onboarding IPSW | Sản phẩm hoá |

Mỗi bước nên đi kèm "smoke test" trong CI: boot QEMU 60–90 s, `--fail-on-panic`, chụp frame, so sánh
kích thước `serial.txt`. `qemu-sweet.py` đã có sẵn khung cho việc này (`--fail-on-panic`, `result.json`).

---

## 5. Repo ngoài đáng đọc (đã kiểm tra tài liệu, không phải đoán)

### 5.1 `jprx/darwin-vm` (+ `jprx/qemu-sptm`) — *khuyến nghị đọc đầu tiên*

- https://github.com/jprx/darwin-vm — MIT, ~1.2k sao, commit gần nhất 10/2026.
- Giả lập **A14–A19 / M1–M5** trong QEMU, boot iOS/macOS tới **root shell trong vài giây**, GDB được,
  hỗ trợ kernel SPTM/TXM và CPU có MIE. Chạy được trên host không phải ARM.
- Cấu trúc rất giống repo này: `get_files.sh` (trích IPSW, patch ramdisk) + `fix_perms.sh` +
  `build_tc.py` (trustcache) + `run.sh` (QEMU). Có luôn `TCP_TUNNELING_SSH.md`.
- Boot-args họ dùng: `rd=md0 serial=3 -v -noprogress wdt=-1` — so sánh với boot-args của sweet.
- **Tác giả nói rõ: không màn hình, không Wi-Fi/BT, không GPU, không SpringBoard.** Đây là "Linux +
  busybox của Darwin", đúng để nghiên cứu kernel.
- Vay được: cách dựng `boot_args`, machine model giả Apple SoC, cách patch ramdisk để launchd chạy
  bash, cách build trustcache, cách debug kernel bằng GDB.
- Lưu ý: bước chuẩn bị firmware cần **một máy Mac** + `ipsw` tool (không cần tải cả IPSW, chỉ URL).

### 5.2 `TrungNguyen1909/qemu-t8030` — iPhone 11 trên QEMU

- https://github.com/TrungNguyen1909/qemu-t8030 — ~2.2k sao, **Public archive** (commit cuối 10/2022).
- Emulator iPhone 11 (t8030) chạy iBoot + iOS 14–16, có các file `hw/apple_*.c` (SPMI, I2C…) là tài
  liệu quý về device model kiểu Apple. Có cả USB companion phục vụ restore.
- Vay được: cách mô phỏng thiết bị Apple (SPMI arbiter, I2C, PMU), cấu trúc machine model.
- Hạn chế: base QEMU 7.1, không còn bảo trì; QEMU upstream cũng đóng issue "Add iphone emulated device"
  mà không merge.

### 5.3 `alephsecurity/xnu-qemu-arm64` (+ `-tools`) — kinh điển

- https://github.com/alephsecurity/xnu-qemu-arm64 và https://github.com/alephsecurity/xnu-qemu-arm64-tools
- iOS 12.1 / iPhone 6s Plus trên QEMU, vào **bash shell**, root ramdisk **read-only**, chỉ UART.
  Có loạt bài giải thích chi tiết: https://alephsecurity.com/2019/06/17/xnu-qemu-arm64-1/
- Giá trị lớn nhất cho người mới: tài liệu mô tả **từng bước** chuẩn bị kernelcache (asn1 decode, lzss),
  device tree, ramdisk, trustcache, plist launchd — trùng khớp với `mkpayload.py`/`mkiosroot.sh` của repo này.
- Nhược: dừng từ ~2020, không device nào ngoài UART, ramdisk 2 GB read-only.
- Docker sẵn có: `sickcodes/Docker-eyeOS` (dựa trên repo này).

### 5.4 `devos50/qemu-ios` (nhánh `ipod_touch_1g`, `ipod_touch_2g`) + `samhenrigold/LightTouchMac`

- Nhánh QEMU: https://github.com/devos50/qemu/tree/ipod_touch_1g — bài viết:
  https://devos50.github.io/blog/2022/ipod-touch-qemu/
- Emulate iPod touch 1G/2G, iPhone 2G→4, iPad 1 (iPhone OS 1 → iOS 7), có **display + multitouch**,
  chạy SpringBoard. `LightTouchMac` là bản đóng gói GUI cho macOS.
- Vay được: đây là ví dụ hoàn chỉnh nhất về **LCD + framebuffer + đa điểm + NOR/NAND** trong QEMU.
  Đặc biệt hữu ích khi làm Mốc B/C.
- Nhược: firmware rất cũ, không liên quan trực tiếp tới iOS 16.

### 5.5 `Lakr233/vphone-cli` — không phải emulator

- https://github.com/Lakr233/vphone-cli
- Dùng `Virtualization.framework` trên **macOS Apple Silicon** + firmware PV=3 để chạy iOS thật trong
  máy ảo. Nhanh, có GUI, jailbreak được; nhưng cần macOS 15+/Apple Silicon, nới SIP/AMFI, và firmware
  lấy từ Apple.
- Chỉ dùng khi mục tiêu là **dùng** iOS ảo, không phải tự xây emulator trên Windows/Linux.

### 5.6 Tài liệu nền

- https://worthdoingbadly.com/xnuqemu/ — chuỗi bài gốc của zhuowei (2017) về XNU trên QEMU; đọc để
  hiểu "tại sao XNU cần device tree + boot-args + trustcache".
- https://github.com/blacktop/ipsw — tool trích IPSW/dyld; repo này đã dùng.
- https://github.com/blacktop/darwin-xnu-build — dựng XNU từ nguồn (repo này dùng làm gốc CI).
- https://github.com/apple-oss-distributions/xnu — nguồn XNU chính thức.

---

## 6. Giới hạn cần chấp nhận (đừng hứa hơn)

| Điều | Thực tế |
| --- | --- |
| Chạy trên Windows x86 | Chỉ QEMU TCG; chậm; không GPU; không Metal |
| "Cài app như BlueStacks" | Không có App Store; chỉ thêm binary + trustcache; nhiều app cần entitlement/SEP |
| Đăng nhập Apple ID | Cần AKS/SEP thật hoặc patch sâu; repo đang dùng stub AppleKeyStore |
| 60 fps | CoreAnimation CPU render; trên máy thật 20–30 fps, trong QEMU TCG thấp hơn |
| Phân phối | Không bundle rootfs/firmware Apple (bản quyền) |
| An toàn | AMFI/sandbox tắt, không data protection → không dùng cho dữ liệu cá nhân |

---

## 7. Bảng tra nhanh: patch nào dùng lại được trong QEMU

| Patch | Dùng lại? | Ghi chú |
| --- | --- | --- |
| 0001–0027 (toolchain, console, ADT platform, RAM disk, AMFI, HFS+) | Có | Giữ nguyên |
| 0028–0034 (UFS) | Không | Thay bằng block device ảo (Mốc A) |
| 0035–0045 (entitlement, keystore, watchdog, crash report) | Có | Cần cho userland |
| 0046, 0056, 0061 (IOSurface/IOMFB, panel copy) | Có, đổi đích | Mốc B |
| 0047–0055 (diagnostics, sandbox) | Có | Rất hữu ích khi debug QEMU |
| 0058 (HID event services) | Có | Giữ; thay nguồn sự kiện (Mốc C) |
| 0059 (iPhone X identity) | Có | ADT phải giữ `/product` D22AP |
| 0060, 0062, 0063 (SPMI/GENI/RPMh/touch/battery) | Không | Stub hoặc thay (Mốc D) |
| 0064–0069 (work interval, SMP, IOSurface semantics, log flush) | Có | SMP cần `-smp` (Mốc E) |
| 0071 (edge touch) | Có | Ảnh hưởng gesture home |
| 0072–0078 (USB DWC3, SMP 8 core) | Một phần | USB thay bằng virtio-net; SMP giữ |

---

## 8. Checklist bắt đầu ngay

- [ ] Chạy smoke test hiện tại: `python3 tools/qemu-sweet.py --check-env`
- [ ] Dựng lại payload QEMU theo [QEMU_TESTING.md](QEMU_TESTING.md) và xác nhận `result.json`
- [ ] Đọc `darwin-vm/run.sh` + `get_files.sh` để so sánh cách dựng `boot_args` và ramdisk
- [ ] Viết `sweet_vblk.c` (virtio-blk MMIO) + thêm `-drive` vào `qemu-sweet.py`
- [ ] Đổi `-smp 1` → `-smp 6`, sinh ADT đủ CPU node
- [ ] Thêm ramfb, trỏ `phys_base_fb` vào vùng RAM của nó, bỏ chụp `fb.png` (giữ làm artifact CI)
- [ ] Thêm virtio-input, nối vào `sweet_hid_touch()` / `sweet_hid_key()`
- [ ] Viết CI job: boot 90 s, `--fail-on-panic`, upload `fb.png` + `serial.txt`

Cập nhật `NOTES.md` sau mỗi mốc (theo quy ước handoff của dự án).
