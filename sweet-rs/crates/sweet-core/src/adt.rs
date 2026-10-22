//! Apple Device Tree (ADT) serializer, parser, and builder for `xiaomi,sweet` and QEMU `virt`.
//! Matches `tools/adt.py` and `tools/mkadt.py` binary format.

use crate::sha::sha384_digest;
use crate::{FB_BASE, FB_H, FB_STRIDE, FB_W, INPUT_MBOX_PA};

pub const PHYS_BASE: u64 = 0xb000_0000;
pub const MEM_SIZE: u64 = 0x4d00_0000;
pub const HIGH_PHYS_BASE: u64 = 0x1_0000_0000;
pub const HIGH_MEM_SIZE: u64 = 0xfcc0_0000;
pub const SOC_BASE: u64 = 0x0100_0000;

pub const GICD: u64 = 0x17a0_0000;
pub const GICD_SIZE: u64 = 0x0001_0000;
pub const GICR: u64 = 0x17a6_0000;
pub const GICR_SIZE: u64 = 0x0010_0000;
pub const FB_SIZE: u64 = 0x0170_0000;
pub const SWEET_ECID: u64 = 0x5357_4545_5400_0001;
pub const UFS_BASE: u64 = 0x01d8_4000;
pub const UFS_SIZE: u64 = 0x0000_3000;
pub const TIMEBASE: u32 = 19_200_000;
pub const VTIMER_INTID: u32 = 19;

pub const QEMU_GICD: u64 = 0x0800_0000;
pub const QEMU_GICR: u64 = 0x080a_0000;
pub const QEMU_TIMEBASE: u32 = 62_500_000;
pub const QEMU_VTIMER_INTID: u32 = 27;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdtProperty {
    pub name: String,
    pub flags: u32,
    pub value: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AdtNode {
    pub props: Vec<AdtProperty>,
    pub children: Vec<AdtNode>,
}

impl AdtNode {
    pub fn new(name: &str, props: Vec<(&str, Vec<u8>)>, children: Vec<AdtNode>) -> Self {
        let mut all_props = Vec::with_capacity(props.len() + 1);
        all_props.push(AdtProperty {
            name: "name".to_string(),
            flags: 0,
            value: cstr(name),
        });
        for (k, v) in props {
            all_props.push(AdtProperty {
                name: k.to_string(),
                flags: 0,
                value: v,
            });
        }
        Self {
            props: all_props,
            children,
        }
    }

    pub fn name(&self) -> &str {
        for p in &self.props {
            if p.name == "name" {
                let end = p.value.iter().position(|&b| b == 0).unwrap_or(p.value.len());
                if let Ok(s) = std::str::from_utf8(&p.value[..end]) {
                    return s;
                }
            }
        }
        "?"
    }

    pub fn get_prop(&self, prop_name: &str) -> Option<&[u8]> {
        for p in &self.props {
            if p.name == prop_name {
                return Some(&p.value);
            }
        }
        None
    }

    pub fn find_child(&self, child_name: &str) -> Option<&AdtNode> {
        self.children.iter().find(|c| c.name() == child_name)
    }
}

#[inline]
fn u32_bytes(vals: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vals.len() * 4);
    for &v in vals {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

#[inline]
fn u64_bytes(vals: &[u64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vals.len() * 8);
    for &v in vals {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

#[inline]
fn cstr(s: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(s.len() + 1);
    v.extend_from_slice(s.as_bytes());
    v.push(0);
    v
}

#[inline]
fn cstr_list(items: &[&str]) -> Vec<u8> {
    let mut v = Vec::new();
    for s in items {
        v.extend_from_slice(s.as_bytes());
        v.push(0);
    }
    v
}

/// Serialize an `AdtNode` tree into Apple Device Tree binary format.
pub fn serialize_adt(node: &AdtNode) -> Vec<u8> {
    let mut out = Vec::new();
    serialize_into(node, &mut out);
    out
}

fn serialize_into(node: &AdtNode, out: &mut Vec<u8>) {
    out.extend_from_slice(&(node.props.len() as u32).to_le_bytes());
    out.extend_from_slice(&(node.children.len() as u32).to_le_bytes());
    for prop in &node.props {
        let mut name_buf = [0u8; 32];
        let name_bytes = prop.name.as_bytes();
        let copy_len = name_bytes.len().min(32);
        name_buf[..copy_len].copy_from_slice(&name_bytes[..copy_len]);
        out.extend_from_slice(&name_buf);

        let len_field = (prop.value.len() as u32) | (prop.flags << 31);
        out.extend_from_slice(&len_field.to_le_bytes());
        out.extend_from_slice(&prop.value);

        let pad = (4 - (prop.value.len() & 3)) & 3;
        for _ in 0..pad {
            out.push(0);
        }
    }
    for child in &node.children {
        serialize_into(child, out);
    }
}

/// Parse an Apple Device Tree binary blob into an `AdtNode` tree and return `(root, bytes_consumed)`.
pub fn parse_adt(buf: &[u8]) -> Result<(AdtNode, usize), String> {
    parse_adt_at(buf, 0)
}

fn parse_adt_at(buf: &[u8], mut off: usize) -> Result<(AdtNode, usize), String> {
    if off + 8 > buf.len() {
        return Err("ADT buffer truncated at node header".to_string());
    }
    let n_props = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]) as usize;
    let n_children =
        u32::from_le_bytes([buf[off + 4], buf[off + 5], buf[off + 6], buf[off + 7]]) as usize;
    off += 8;

    let mut props = Vec::with_capacity(n_props);
    for _ in 0..n_props {
        if off + 36 > buf.len() {
            return Err("ADT buffer truncated at property header".to_string());
        }
        let name_slice = &buf[off..off + 32];
        let name_end = name_slice.iter().position(|&b| b == 0).unwrap_or(32);
        let name = String::from_utf8_lossy(&name_slice[..name_end]).into_owned();
        let raw_size = u32::from_le_bytes([
            buf[off + 32],
            buf[off + 33],
            buf[off + 34],
            buf[off + 35],
        ]);
        let flags = raw_size >> 31;
        let ln = (raw_size & 0x7fff_ffff) as usize;
        off += 36;
        if off + ln > buf.len() {
            return Err(format!("ADT property '{}' value out of bounds", name));
        }
        let value = buf[off..off + ln].to_vec();
        let aligned = (ln + 3) & !3;
        if off + aligned > buf.len() {
            return Err(format!("ADT property '{}' padding out of bounds", name));
        }
        off += aligned;
        props.push(AdtProperty { name, flags, value });
    }

    let mut children = Vec::with_capacity(n_children);
    for _ in 0..n_children {
        let (child, next_off) = parse_adt_at(buf, off)?;
        off = next_off;
        children.push(child);
    }

    Ok((AdtNode { props, children }, off))
}

fn load_d22ap_product_props() -> Option<Vec<AdtProperty>> {
    let mut candidates = vec![
        std::path::PathBuf::from("research/ipsw-x/d22ap.adt"),
        std::path::PathBuf::from("../research/ipsw-x/d22ap.adt"),
        std::path::PathBuf::from("../../research/ipsw-x/d22ap.adt"),
        std::path::PathBuf::from("../../../research/ipsw-x/d22ap.adt"),
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../research/ipsw-x/d22ap.adt"),
    ];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("research/ipsw-x/d22ap.adt"));
            candidates.push(dir.join("../research/ipsw-x/d22ap.adt"));
            candidates.push(dir.join("../../research/ipsw-x/d22ap.adt"));
            candidates.push(dir.join("../../../research/ipsw-x/d22ap.adt"));
        }
    }

    for path in &candidates {
        if let Ok(bytes) = std::fs::read(path) {
            if let Ok((root, _)) = parse_adt(&bytes) {
                if let Some(prod) = root.find_child("product") {
                    let filtered: Vec<AdtProperty> = prod
                        .props
                        .iter()
                        .filter(|p| p.flags == 0 && p.name != "name" && p.name != "AAPL,phandle")
                        .cloned()
                        .collect();
                    if !filtered.is_empty() {
                        return Some(filtered);
                    }
                }
            }
        }
    }
    None
}

fn build_product_node(ph_prop: (&'static str, Vec<u8>)) -> AdtNode {
    if let Some(real_props) = load_d22ap_product_props() {
        let mut props = Vec::with_capacity(real_props.len() + 2);
        props.push(AdtProperty {
            name: "name".to_string(),
            flags: 0,
            value: cstr("product"),
        });
        props.push(AdtProperty {
            name: ph_prop.0.to_string(),
            flags: 0,
            value: ph_prop.1,
        });
        props.extend(real_props);
        AdtNode {
            props,
            children: Vec::new(),
        }
    } else {
        AdtNode::new(
            "product",
            vec![
                ph_prop,
                ("display-mirroring", u32_bytes(&[1])),
                ("product-name", cstr("iPhone X")),
                ("product-description", cstr("iPhone X")),
                ("artwork-scale-factor", u32_bytes(&[3])),
                ("artwork-device-idiom", cstr("phone")),
                ("artwork-dynamic-displaymode", cstr("0")),
                ("artwork-display-gamut", cstr("P3")),
                ("oled-display", u32_bytes(&[1])),
                ("artwork-device-subtype", u32_bytes(&[2436])),
            ],
            vec![],
        )
    }
}

fn build_adt_internal(qemu: bool, highmem: bool, cpus: u32) -> Vec<u8> {
    let (phys_base, mem_size) = if highmem {
        (HIGH_PHYS_BASE, HIGH_MEM_SIZE)
    } else {
        (PHYS_BASE, MEM_SIZE)
    };
    let (gicd, gicr, timebase) = if qemu {
        (QEMU_GICD, QEMU_GICR, QEMU_TIMEBASE)
    } else {
        (GICD, GICR, TIMEBASE)
    };
    let num_cpus = cpus.clamp(1, 16);

    let mut next_phandle: u32 = 1;
    let mut ph = || -> (&'static str, Vec<u8>) {
        let id = next_phandle;
        next_phandle += 1;
        ("AAPL,phandle", u32_bytes(&[id]))
    };

    // Deterministic 64-byte random-seed
    let mut random_seed = vec![0u8; 64];
    for (i, b) in random_seed.iter_mut().enumerate() {
        *b = (0x53u8).wrapping_add((i as u8).wrapping_mul(37));
    }

    let boot_manifest_hash = sha384_digest(b"sweet boot manifest").to_vec();

    let mut chosen_props = vec![
        ph(),
        ("dram-base", u64_bytes(&[phys_base])),
        ("dram-size", u64_bytes(&[mem_size])),
        ("random-seed", random_seed),
        ("debug-enabled", u32_bytes(&[1])),
        ("firmware-version", cstr("sweet-shim")),
        ("system-firmware-version", cstr("sweet-shim")),
        ("boot-manifest-hash", boot_manifest_hash),
        ("crypto-hash-method", cstr("sha2-384")),
        ("display-scale", u32_bytes(&[3])),
        ("unique-chip-id", u64_bytes(&[SWEET_ECID])),
        ("chip-id", u32_bytes(&[0x8015])),
        ("certificate-production-status", u32_bytes(&[0])),
        ("effective-production-status-ap", u32_bytes(&[0])),
        ("certificate-security-mode", u32_bytes(&[1])),
        ("effective-security-mode-sep", u32_bytes(&[1])),
        ("mac-address-wifi0", vec![0x02, 0x00, 0x5e, 0x77, 0xee, 0x02]),
        ("mac-address-bluetooth0", vec![0x02, 0x00, 0x5e, 0x77, 0xee, 0x03]),
        ("mac-address-ethernet0", vec![0x02, 0x00, 0x5e, 0x77, 0xee, 0x01]),
        ("mac-address-ethernet1", vec![0x02, 0x00, 0x5e, 0x77, 0xee, 0x04]),
        ("software-behavior", {
            let mut sb = vec![0u8; 16];
            sb[0] = 1;
            sb
        }),
        ("software-bundle-version", cstr("20H392")),
    ];
    if qemu {
        chosen_props.push(("qemu-virt", u32_bytes(&[1])));
    }

    let memory_map = AdtNode::new("memory-map", vec![ph()], vec![]);
    let asmb = AdtNode::new(
        "asmb",
        vec![
            ph(),
            ("lp-sip0", u64_bytes(&[0x6f])),
            ("lp-sip1", u32_bytes(&[1])),
        ],
        vec![],
    );
    let chosen = AdtNode::new("chosen", chosen_props, vec![memory_map, asmb]);

    let mut cpu_nodes = Vec::with_capacity(num_cpus as usize);
    for core in 0..num_cpus {
        let reg_val = if qemu { core } else { core << 8 };
        let state_str = if core == 0 { "running" } else { "stopped" };
        let clk_freq: u32 = if core >= 6 { 2_304_000_000 } else { 1_804_800_000 };
        let cpu_name = format!("cpu{}", core);
        cpu_nodes.push(AdtNode::new(
            &cpu_name,
            vec![
                ph(),
                ("device_type", cstr("cpu")),
                ("compatible", cstr_list(&["qcom,kryo470", "ARM,v8"])),
                ("reg", u32_bytes(&[reg_val])),
                ("cpu-id", u32_bytes(&[core])),
                ("cluster-id", u32_bytes(&[0])),
                ("cluster-type", u32_bytes(&[0])),
                ("die-cluster-id", u32_bytes(&[0])),
                ("cluster-core-id", u32_bytes(&[core])),
                ("l2-cache-id", u32_bytes(&[core])),
                ("l3-cache-id", u32_bytes(&[0])),
                ("state", cstr(state_str)),
                ("timebase-frequency", u32_bytes(&[timebase])),
                ("clock-frequency", u32_bytes(&[clk_freq])),
                ("bus-frequency", u32_bytes(&[100_000_000])),
                ("memory-frequency", u32_bytes(&[1_866_000_000])),
                ("peripheral-frequency", u32_bytes(&[timebase])),
                ("fixed-frequency", u32_bytes(&[timebase])),
            ],
            vec![],
        ));
    }

    let mut cpus_props = vec![
        ph(),
        ("#address-cells", u32_bytes(&[1])),
        ("#size-cells", u32_bytes(&[0])),
    ];
    if qemu {
        cpus_props.push(("psci-conduit", cstr("hvc")));
    }
    let cpus_node = AdtNode::new("cpus", cpus_props, cpu_nodes);

    let gic = AdtNode::new(
        "gic",
        vec![
            ph(),
            ("compatible", cstr("arm,gic-v3")),
            ("interrupt-controller", Vec::new()),
            (
                "virtual-timer-intid",
                u32_bytes(&[if qemu { QEMU_VTIMER_INTID } else { VTIMER_INTID }]),
            ),
            (
                "reg",
                u64_bytes(&[gicd - SOC_BASE, GICD_SIZE, gicr - SOC_BASE, GICR_SIZE]),
            ),
        ],
        vec![],
    );
    let arm_io_ph = ph();
    let wdt = AdtNode::new("wdt", vec![ph(), ("no-pmu", u32_bytes(&[1]))], vec![]);
    let disp0 = AdtNode::new(
        "disp0",
        vec![
            ph(),
            ("device_type", cstr("display-subsystem")),
            ("compatible", cstr("sweet,display")),
            ("canvas-width", u32_bytes(&[1125])),
            ("canvas-height", u32_bytes(&[2436])),
        ],
        vec![],
    );
    let wlan = AdtNode::new(
        "wlan",
        vec![
            ph(),
            ("compatible", cstr("wlan")),
            ("local-mac-address", vec![0x52, 0x54, 0x00, 0x12, 0x34, 0x56]),
        ],
        vec![],
    );
    let bluetooth = AdtNode::new(
        "bluetooth",
        vec![
            ph(),
            ("compatible", cstr("bluetooth")),
            ("local-mac-address", vec![0x52, 0x54, 0x00, 0x12, 0x34, 0x57]),
        ],
        vec![],
    );
    let arm_io = AdtNode::new(
        "arm-io",
        vec![
            arm_io_ph,
            ("device_type", cstr("sweet-io")),
            ("compatible", cstr_list(&["sweet-io", "arm-io"])),
            ("#address-cells", u32_bytes(&[2])),
            ("#size-cells", u32_bytes(&[2])),
            ("ranges", u64_bytes(&[0, SOC_BASE, 0x1_0000_0000 - SOC_BASE])),
        ],
        vec![gic, wdt, disp0, wlan, bluetooth],
    );

    let fbcon = AdtNode::new(
        "sweet-fbcon",
        vec![
            ph(),
            ("reg", u64_bytes(&[FB_BASE, FB_SIZE])),
            ("width", u32_bytes(&[FB_W as u32])),
            ("height", u32_bytes(&[FB_H as u32])),
            ("stride", u32_bytes(&[FB_STRIDE as u32])),
        ],
        vec![],
    );

    let ufs = AdtNode::new(
        "sweet-ufs",
        vec![
            ph(),
            (
                "compatible",
                cstr(if qemu { "qemu,virtio-mmio" } else { "qcom,ufshc" }),
            ),
            (
                "reg",
                u64_bytes(&[
                    if qemu { 0x0a00_0000 } else { UFS_BASE },
                    if qemu { 0x4000 } else { UFS_SIZE },
                ]),
            ),
        ],
        vec![],
    );

    let virt = AdtNode::new(
        "sweet-virt",
        vec![
            ph(),
            ("compatible", cstr("qemu,virtio-mmio")),
            ("reg", u64_bytes(&[0x0a00_0000, 0x4000])),
            ("input-mailbox", u64_bytes(&[INPUT_MBOX_PA, 0x4000])),
        ],
        vec![],
    );

    let defaults = AdtNode::new(
        "defaults",
        vec![ph(), ("no-effaceable-storage", u32_bytes(&[1]))],
        vec![],
    );

    let buttons = AdtNode::new(
        "buttons",
        vec![
            ph(),
            ("button-names", cstr_list(&["volup", "voldown", "hold"])),
            ("home-button-type", u32_bytes(&[2])),
            ("opposed-power-vol-buttons", u32_bytes(&[1])),
        ],
        vec![],
    );

    let product = build_product_node(ph());
    let mut children = vec![
        chosen,
        defaults,
        cpus_node,
        arm_io,
        fbcon,
        buttons,
        product,
    ];
    if qemu {
        children.push(virt);
    } else {
        children.push(ufs);
    }

    let root = AdtNode::new(
        "device-tree",
        vec![
            ph(),
            (
                "compatible",
                cstr_list(&[
                    "D22AP",
                    "iPhone10,3",
                    "xiaomi,sweet",
                    "qcom,sm7150",
                    "AppleARM",
                ]),
            ),
            ("model", cstr("iPhone10,3")),
            ("target-type", cstr("D22")),
            ("target-sub-type", cstr("D22AP")),
            ("manufacturer", cstr("Apple Inc.")),
            ("serial-number", cstr("F2LV809XJCLH")),
            ("mlb-serial-number", cstr("F2L74210012JCLH1A")),
            ("model-number", cstr("MQAC2")),
            ("region-info", cstr("LL/A")),
            ("regulatory-model-number", cstr("A1865")),
            ("config-number", cstr("D22AP")),
            ("#address-cells", u32_bytes(&[2])),
            ("#size-cells", u32_bytes(&[2])),
        ],
        children,
    );

    serialize_adt(&root)
}

/// Build an Apple Device Tree binary for QEMU `virt` (`-M virt,gic-version=3 -cpu cortex-a76`).
pub fn build_qemu_adt(cpus: u32, highmem: bool) -> Vec<u8> {
    let n = if cpus == 0 { 6 } else { cpus };
    build_adt_internal(true, highmem, n)
}

/// Build an Apple Device Tree binary for physical `xiaomi,sweet` (Redmi Note 10 Pro / SM7150).
pub fn build_sweet_adt(highmem: bool) -> Vec<u8> {
    build_adt_internal(false, highmem, 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_d22ap_product_node_in_adt() {
        let blob = build_qemu_adt(6, false);
        let (root, _) = parse_adt(&blob).expect("parse adt");
        let product = root.find_child("product").expect("/product node must exist");
        assert_eq!(product.get_prop("product-name"), Some(b"iPhone X\0".as_slice()));
        assert_eq!(product.get_prop("product-description"), Some(b"iPhone X\0".as_slice()));
        assert_eq!(product.get_prop("oled-display"), Some(&1u32.to_le_bytes()[..]));
        assert_eq!(product.get_prop("artwork-scale-factor"), Some(&3u32.to_le_bytes()[..]));
        if load_d22ap_product_props().is_some() {
            assert!(product.props.len() >= 60, "expected full d22ap.adt /product node");
        }
    }
}
