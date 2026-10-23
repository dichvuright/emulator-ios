pub mod adt;
pub mod ipa;
pub mod qemu;
pub mod sha;

pub use adt::{
    build_qemu_adt, build_sweet_adt, parse_adt, serialize_adt, AdtNode, AdtProperty,
};
pub use ipa::{
    analyze_macho_objc_metadata, build_demo_macho_executable, build_trustcache_v1, create_demo_ipa,
    decode_objc_type_encoding, extract_codesig_cdhash, inspect_app_or_ipa, install_app_to_root,
    list_installed_apps, parse_macho_binary, parse_plist_kv, parse_trustcache_v1, read_stored_zip,
    write_stored_zip, IpasimMachOAnalysis, IpasimObjcMethod,
};
pub use qemu::{
    build_qemu_args, direct_qemu_attachment_info, direct_qemu_enqueue_input,
    direct_qemu_invalidate_attachment, direct_qemu_read_bytes, direct_qemu_read_fb,
    direct_qemu_read_u64, qmp_execute_json, unpack_sweetxnu_payload,
};
pub use sha::{
    crc32_ieee, deterministic_uuid_string, hex_decode, hex_encode, sha1_digest, sha256_digest,
    sha384_digest,
};

pub const FB_BASE: u64 = 0x9c00_0000;
pub const FB_W: usize = 1080;
pub const FB_H: usize = 2400;
pub const FB_STRIDE: usize = FB_W * 4;
pub const INPUT_MBOX_PA: u64 = 0x9d60_0000;
pub const INPUT_MBOX_MAGIC: u32 = 0x5357_494e;
pub const GPU_RING_PA: u64 = 0x9d60_4000;
pub const GPU_RING_MAGIC: u32 = 0x5347_5055;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppMetadata {
    pub bundle_id: String,
    pub display_name: String,
    pub version: String,
    pub executable: String,
    pub graphics_api: String,
    pub is_arm64: bool,
    pub is_encrypted: bool,
    pub cdhash_hex: String,
    pub bundle_uuid: String,
    pub data_uuid: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SweetInputEvent {
    pub kind: u16,
    pub id: u16,
    pub down: i32,
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug)]
pub struct SweetInputMailbox {
    pub write_idx: u32,
    pub read_idx: u32,
    pub flags: u32,
    pub events: [SweetInputEvent; 64],
}

impl Default for SweetInputMailbox {
    fn default() -> Self {
        Self::new()
    }
}

impl SweetInputMailbox {
    pub fn new() -> Self {
        Self {
            write_idx: 0,
            read_idx: 0,
            flags: 0,
            events: [SweetInputEvent::default(); 64],
        }
    }

    pub fn push_touch(&mut self, finger: u16, down: bool, x: i32, y: i32) {
        let idx = (self.write_idx as usize) % 64;
        self.events[idx] = SweetInputEvent {
            kind: 1,
            id: finger,
            down: if down { 1 } else { 0 },
            x: x.clamp(0, (FB_W as i32) - 1),
            y: y.clamp(0, (FB_H as i32) - 1),
        };
        self.write_idx = self.write_idx.wrapping_add(1);
    }

    pub fn push_key(&mut self, key: u16, down: bool) {
        let idx = (self.write_idx as usize) % 64;
        self.events[idx] = SweetInputEvent {
            kind: 2,
            id: key,
            down: if down { 1 } else { 0 },
            x: 0,
            y: 0,
        };
        self.write_idx = self.write_idx.wrapping_add(1);
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(16 + 64 * 16);
        buf.extend_from_slice(&INPUT_MBOX_MAGIC.to_le_bytes());
        buf.extend_from_slice(&self.write_idx.to_le_bytes());
        buf.extend_from_slice(&self.read_idx.to_le_bytes());
        buf.extend_from_slice(&self.flags.to_le_bytes());
        for ev in &self.events {
            buf.extend_from_slice(&ev.kind.to_le_bytes());
            buf.extend_from_slice(&ev.id.to_le_bytes());
            buf.extend_from_slice(&ev.down.to_le_bytes());
            buf.extend_from_slice(&ev.x.to_le_bytes());
            buf.extend_from_slice(&ev.y.to_le_bytes());
        }
        buf
    }

    pub fn from_bytes(raw: &[u8]) -> Option<Self> {
        if raw.len() < 16 + 64 * 16 {
            return None;
        }
        let magic = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
        if magic != INPUT_MBOX_MAGIC {
            return None;
        }
        let write_idx = u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]);
        let read_idx = u32::from_le_bytes([raw[8], raw[9], raw[10], raw[11]]);
        let flags = u32::from_le_bytes([raw[12], raw[13], raw[14], raw[15]]);
        let mut events = [SweetInputEvent::default(); 64];
        for (i, ev) in events.iter_mut().enumerate() {
            let off = 16 + i * 16;
            ev.kind = u16::from_le_bytes([raw[off], raw[off + 1]]);
            ev.id = u16::from_le_bytes([raw[off + 2], raw[off + 3]]);
            ev.down = i32::from_le_bytes([raw[off + 4], raw[off + 5], raw[off + 6], raw[off + 7]]);
            ev.x = i32::from_le_bytes([raw[off + 8], raw[off + 9], raw[off + 10], raw[off + 11]]);
            ev.y = i32::from_le_bytes([raw[off + 12], raw[off + 13], raw[off + 14], raw[off + 15]]);
        }
        Some(Self {
            write_idx,
            read_idx,
            flags,
            events,
        })
    }

    pub fn is_bottom_edge_swipe(y: i32) -> bool {
        y >= (FB_H as i32) - 48
    }
}

#[derive(Clone, Debug)]
pub struct QemuConfig {
    pub qemu_bin: String,
    pub payload_path: String,
    pub disk_path: String,
    pub out_dir: String,
    pub smp: u32,
    pub ram_gb: u32,
    pub qmp_port: u16,
    pub net_enabled: bool,
    pub display_window: bool,
}

impl Default for QemuConfig {
    fn default() -> Self {
        Self {
            qemu_bin: String::new(),
            payload_path: String::new(),
            disk_path: String::new(),
            out_dir: "emu_workspace/qemu_run".to_string(),
            smp: 8,
            ram_gb: 6,
            qmp_port: 4444,
            net_enabled: true,
            display_window: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct QemuRunner {
    pub cfg: QemuConfig,
    pub mbox: SweetInputMailbox,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), nanos));
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_sha256_and_sha384_nist_vectors() {
        let d256 = sha256_digest(b"abc");
        assert_eq!(
            hex_encode(&d256),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );

        let empty256 = sha256_digest(b"");
        assert_eq!(
            hex_encode(&empty256),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );

        let multi256 =
            sha256_digest(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq");
        assert_eq!(
            hex_encode(&multi256),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );

        let d384 = sha384_digest(b"abc");
        assert_eq!(
            hex_encode(&d384),
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"
        );

        let s1 = sha1_digest(b"abc");
        assert_eq!(
            hex_encode(&s1),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
    }

    #[test]
    fn test_build_qemu_and_sweet_adt() {
        let qemu_blob = build_qemu_adt(6, false);
        assert!(qemu_blob.len() > 1000);
        let (root, consumed) = parse_adt(&qemu_blob).expect("parse qemu adt");
        assert_eq!(consumed, qemu_blob.len());
        assert_eq!(root.name(), "device-tree");
        assert_eq!(root.get_prop("model"), Some(b"iPhone10,3\0".as_slice()));

        let chosen = root.find_child("chosen").expect("/chosen node");
        assert_eq!(
            chosen.get_prop("display-scale"),
            Some(&3u32.to_le_bytes()[..])
        );
        assert_eq!(
            chosen.get_prop("unique-chip-id"),
            Some(&0x5357_4545_5400_0001u64.to_le_bytes()[..])
        );
        assert_eq!(
            chosen.get_prop("chip-id"),
            Some(&0x8015u32.to_le_bytes()[..])
        );
        assert_eq!(chosen.get_prop("qemu-virt"), Some(&1u32.to_le_bytes()[..]));

        let cpus = root.find_child("cpus").expect("/cpus node");
        assert_eq!(cpus.get_prop("psci-conduit"), Some(b"hvc\0".as_slice()));
        assert_eq!(cpus.children.len(), 6);
        let cpu1 = cpus.find_child("cpu1").expect("cpu1");
        assert_eq!(cpu1.get_prop("reg"), Some(&1u32.to_le_bytes()[..]));

        let arm_io = root.find_child("arm-io").expect("/arm-io node");
        let disp0 = arm_io.find_child("disp0").expect("disp0");
        assert_eq!(
            disp0.get_prop("canvas-width"),
            Some(&1125u32.to_le_bytes()[..])
        );
        assert_eq!(
            disp0.get_prop("canvas-height"),
            Some(&2436u32.to_le_bytes()[..])
        );

        let fbcon = root.find_child("sweet-fbcon").expect("/sweet-fbcon node");
        assert_eq!(fbcon.get_prop("width"), Some(&1080u32.to_le_bytes()[..]));
        assert_eq!(fbcon.get_prop("height"), Some(&2400u32.to_le_bytes()[..]));

        let buttons = root.find_child("buttons").expect("/buttons node");
        assert_eq!(
            buttons.get_prop("home-button-type"),
            Some(&2u32.to_le_bytes()[..])
        );

        let virt = root.find_child("sweet-virt").expect("/sweet-virt node");
        assert!(virt.get_prop("input-mailbox").is_some());
        assert!(root.find_child("sweet-ufs").is_none());

        // Test physical sweet ADT
        let sweet_blob = build_sweet_adt(true);
        let (sweet_root, _) = parse_adt(&sweet_blob).expect("parse sweet adt");
        assert!(sweet_root.find_child("sweet-ufs").is_some());
        assert!(sweet_root.find_child("sweet-virt").is_none());
        let sweet_cpus = sweet_root.find_child("cpus").expect("/cpus");
        assert_eq!(sweet_cpus.children.len(), 8);
        let sweet_cpu1 = sweet_cpus.find_child("cpu1").expect("cpu1");
        assert_eq!(
            sweet_cpu1.get_prop("reg"),
            Some(&(1u32 << 8).to_le_bytes()[..])
        );
    }

    #[test]
    fn test_trustcache_v1_builder() {
        let h1 = [0xbbu8; 20];
        let h2 = [0x11u8; 20];
        let blob = build_trustcache_v1(&[h1, h2, h1]);
        // 24-byte header + 2 * 22-byte entries = 68 bytes
        assert_eq!(blob.len(), 24 + 2 * 22);
        let parsed = parse_trustcache_v1(&blob).expect("parse trustcache");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0], h2);
        assert_eq!(parsed[1], h1);
        assert_eq!(blob[24 + 20], 2); // CS_HASHTYPE_SHA256
        assert_eq!(blob[24 + 21], 0); // flags = 0
    }

    #[test]
    fn test_demo_ipa_inspect_and_install() {
        let dir = unique_temp_dir("sweet_core_ipa");
        let ipa_path = dir.join("SweetBenchmark.ipa");
        let root_dir = dir.join("rootfs");

        let meta = create_demo_ipa(&ipa_path).expect("create_demo_ipa");
        assert_eq!(meta.bundle_id, "com.sweet.benchmark");
        assert_eq!(meta.display_name, "Sweet 3D Bench");
        assert_eq!(meta.executable, "SweetBenchmark");
        assert_eq!(meta.version, "1.0.0");
        assert!(meta.is_arm64);
        assert!(!meta.is_encrypted);
        assert_eq!(meta.cdhash_hex.len(), 40);
        assert!(meta.graphics_api.contains("Metal"));
        assert!(meta.graphics_api.contains("OpenGL"));

        let installed = install_app_to_root(&ipa_path, &root_dir).expect("install_app_to_root");
        assert_eq!(installed.bundle_id, "com.sweet.benchmark");
        assert!(!installed.bundle_uuid.is_empty());
        assert!(!installed.data_uuid.is_empty());

        let bundle_app = root_dir
            .join("private/var/containers/Bundle/Application")
            .join(&installed.bundle_uuid)
            .join("SweetBenchmark.app");
        assert!(bundle_app.join("Info.plist").is_file());
        assert!(bundle_app.join("SweetBenchmark").is_file());

        // Inspect directly from installed .app directory
        let dir_meta = inspect_app_or_ipa(&bundle_app).expect("inspect .app dir");
        assert_eq!(dir_meta.bundle_id, "com.sweet.benchmark");
        assert_eq!(dir_meta.cdhash_hex, meta.cdhash_hex);

        let tc_file = root_dir.join("private/var/db/sweet_trustcache.bin");
        assert!(tc_file.is_file());
        let tc_bytes = fs::read(&tc_file).unwrap();
        let tc_hashes = parse_trustcache_v1(&tc_bytes).unwrap();
        assert_eq!(tc_hashes.len(), 1);
        assert_eq!(hex_encode(&tc_hashes[0]), meta.cdhash_hex);

        let apps = list_installed_apps(&root_dir);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].bundle_id, "com.sweet.benchmark");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_input_mailbox_and_qmp_runner() {
        let mut mbox = SweetInputMailbox::new();
        assert!(!SweetInputMailbox::is_bottom_edge_swipe(2000));
        assert!(SweetInputMailbox::is_bottom_edge_swipe(2352));
        assert!(SweetInputMailbox::is_bottom_edge_swipe(2399));

        mbox.push_touch(0, true, 540, 2380);
        mbox.push_key(0, true);
        let raw = mbox.to_bytes();
        assert_eq!(raw.len(), 1040);
        assert_eq!(
            u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]),
            INPUT_MBOX_MAGIC
        );
        assert_eq!(u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]), 2);

        // Spin up a local mock QMP server on an ephemeral port to test QemuRunner QMP client
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock qmp");
        let port = listener.local_addr().unwrap().port();
        let dir = unique_temp_dir("sweet_core_qmp");
        fs::write(dir.join("serial.txt"), b"iBoot-sweet serial ready\nWelcome to XNU!\n").unwrap();

        let server_thread = thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let _ = stream.write_all(b"{\"QMP\":{\"version\":{\"qemu\":{\"micro\":0,\"minor\":2,\"major\":8}},\"capabilities\":[]}}\r\n");
                let cloned = stream.try_clone().unwrap();
                let mut reader = BufReader::new(cloned);
                let mut cap_line = String::new();
                let _ = reader.read_line(&mut cap_line);
                let _ = stream.write_all(b"{\"return\":{}}\r\n");

                let mut cmd_line = String::new();
                let _ = reader.read_line(&mut cmd_line);
                if cmd_line.contains("pmemsave") {
                    // Extract filename and write synthetic BGRA bytes
                    if let Some(idx) = cmd_line.find("\"filename\":\"") {
                        let rest = &cmd_line[idx + 12..];
                        if let Some(end) = rest.find('"') {
                            let fname = &rest[..end];
                            let _ = fs::write(fname, vec![0x42u8; 64]);
                        }
                    }
                }
                let _ = stream.write_all(b"{\"return\":{}}\r\n");
            }
        });

        let cfg = QemuConfig {
            qemu_bin: String::new(),
            payload_path: String::new(),
            disk_path: String::new(),
            out_dir: dir.to_string_lossy().into_owned(),
            smp: 6,
            ram_gb: 6,
            qmp_port: port,
            net_enabled: true,
            display_window: false,
        };

        let mut runner = QemuRunner::start(&cfg).expect("QemuRunner::start");
        runner.send_touch(0, true, 500, 1200).expect("send_touch");
        runner.send_key(0, true).expect("send_key");

        let mut fb_buf = [0u8; 64];
        runner
            .capture_framebuffer(&mut fb_buf)
            .expect("capture_framebuffer");
        assert_eq!(fb_buf[0], 0x42);

        let tail = runner.read_serial_tail(16);
        assert_eq!(tail, "Welcome to XNU!\n");

        let _ = server_thread.join();
        let _ = fs::remove_dir_all(&dir);
    }
}
