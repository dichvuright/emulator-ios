//! Mach-O 64 parser, CodeDirectory SHA-256 extractor, Apple TrustCache v1 builder,
//! Stored-ZIP `.ipa` reader/writer, XML & binary plist parser, and rootfs installer.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::sha::{
    crc32_ieee, deterministic_uuid_string, hex_decode, hex_encode, sha1_digest, sha256_digest,
    sha384_digest,
};
use crate::AppMetadata;

pub const MH_MAGIC: u32 = 0xfeed_face;
pub const MH_CIGAM: u32 = 0xcefa_edfe;
pub const MH_MAGIC_64: u32 = 0xfeed_facf;
pub const MH_CIGAM_64: u32 = 0xcffa_edfe;
pub const FAT_MAGIC: u32 = 0xcafe_babe;
pub const FAT_CIGAM: u32 = 0xbeba_feca;
pub const FAT_MAGIC_64: u32 = 0xcafe_babf;
pub const FAT_CIGAM_64: u32 = 0xbfba_feca;

pub const CPU_TYPE_ARM64: u32 = 0x0100_000c;
pub const MH_EXECUTE: u32 = 0x2;
pub const MH_NOUNDEFS: u32 = 0x1;
pub const MH_DYLDLINK: u32 = 0x4;
pub const MH_TWOLEVEL: u32 = 0x80;
pub const MH_PIE: u32 = 0x0020_0000;

pub const LC_LOAD_DYLIB: u32 = 0x0c;
pub const LC_SEGMENT_64: u32 = 0x19;
pub const LC_UUID: u32 = 0x1b;
pub const LC_CODE_SIGNATURE: u32 = 0x1d;
pub const LC_REEXPORT_DYLIB: u32 = 0x8000_001f;
pub const LC_LAZY_LOAD_DYLIB: u32 = 0x20;
pub const LC_ENCRYPTION_INFO: u32 = 0x21;
pub const LC_LOAD_UPWARD_DYLIB: u32 = 0x8000_0023;
pub const LC_MAIN: u32 = 0x8000_0028;
pub const LC_ENCRYPTION_INFO_64: u32 = 0x2c;
pub const LC_WEAK_DYLIB: u32 = 0x8000_0018;

pub const CSMAGIC_CODEDIRECTORY: u32 = 0xfade_0c02;
pub const CSMAGIC_EMBEDDED_SIGNATURE: u32 = 0xfade_0cc0;
pub const CSMAGIC_EMBEDDED_ENTITLEMENTS: u32 = 0xfade_7171;

pub const CSSLOT_CODEDIRECTORY: u32 = 0;
pub const CSSLOT_ENTITLEMENTS: u32 = 5;

pub const CS_ADHOC: u32 = 0x0000_0002;
pub const CS_HASHTYPE_SHA1: u8 = 1;
pub const CS_HASHTYPE_SHA256: u8 = 2;
pub const CS_HASHTYPE_SHA256_TRUNCATED: u8 = 3;
pub const CS_HASHTYPE_SHA384: u8 = 4;

/// Deterministic UUID `53574545-5454-4331-8000-000000000001` used in ios-sweet TrustCache v1.
pub const SWEET_TRUSTCACHE_UUID: [u8; 16] = [
    0x53, 0x57, 0x45, 0x45, 0x54, 0x54, 0x43, 0x31, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
];

/// Build an Apple TrustCache v1 binary (`u32 version = 1`, `16-byte UUID`, `u32 count`,
/// followed by sorted, deduplicated `[20-byte cdhash + u8 hash_type(2) + u8 flags(0)]`).
pub fn build_trustcache_v1(cdhashes: &[[u8; 20]]) -> Vec<u8> {
    let mut sorted: Vec<[u8; 20]> = cdhashes.to_vec();
    sorted.sort_unstable();
    sorted.dedup();

    let count = sorted.len() as u32;
    let mut out = Vec::with_capacity(24 + sorted.len() * 22);
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&SWEET_TRUSTCACHE_UUID);
    out.extend_from_slice(&count.to_le_bytes());

    for hash in &sorted {
        out.extend_from_slice(hash);
        out.push(CS_HASHTYPE_SHA256);
        out.push(0u8);
    }
    out
}

/// Parse an Apple TrustCache v1 binary and return its 20-byte CDHashes.
pub fn parse_trustcache_v1(data: &[u8]) -> Result<Vec<[u8; 20]>, String> {
    if data.len() < 24 {
        return Err("TrustCache v1 buffer too small (< 24 bytes)".to_string());
    }
    let version = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if version != 1 {
        return Err(format!("Unsupported TrustCache version: {}", version));
    }
    let count = u32::from_le_bytes([data[20], data[21], data[22], data[23]]) as usize;
    let mut out = Vec::with_capacity(count);
    let mut off = 24;
    for _ in 0..count {
        if off + 22 > data.len() {
            break;
        }
        let mut h = [0u8; 20];
        h.copy_from_slice(&data[off..off + 20]);
        out.push(h);
        off += 22;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Stored-ZIP Writer & Reader
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct ZipEntryData {
    pub name: String,
    pub compression_method: u16,
    pub data: Vec<u8>,
}

/// Write an uncompressed (STORE method `0`) ZIP archive from `(entry_name, bytes)` pairs.
pub fn write_stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central_dir = Vec::new();

    for &(name, data) in entries {
        let name_bytes = name.as_bytes();
        let crc = crc32_ieee(data);
        let size = data.len() as u32;
        let local_header_offset = out.len() as u32;

        // Local file header (30 bytes + filename)
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes()); // signature
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed (2.0)
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // compression method = 0 (STORE)
        out.extend_from_slice(&0u16.to_le_bytes()); // mod time
        out.extend_from_slice(&0x5421u16.to_le_bytes()); // mod date
        out.extend_from_slice(&crc.to_le_bytes()); // crc32
        out.extend_from_slice(&size.to_le_bytes()); // compressed size
        out.extend_from_slice(&size.to_le_bytes()); // uncompressed size
        out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra field length
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(data);

        // Central directory file header (46 bytes + filename)
        central_dir.extend_from_slice(&0x0201_4b50u32.to_le_bytes()); // signature
        central_dir.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central_dir.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central_dir.extend_from_slice(&0u16.to_le_bytes()); // flags
        central_dir.extend_from_slice(&0u16.to_le_bytes()); // compression = 0 (STORE)
        central_dir.extend_from_slice(&0u16.to_le_bytes()); // mod time
        central_dir.extend_from_slice(&0x5421u16.to_le_bytes()); // mod date
        central_dir.extend_from_slice(&crc.to_le_bytes()); // crc32
        central_dir.extend_from_slice(&size.to_le_bytes()); // compressed size
        central_dir.extend_from_slice(&size.to_le_bytes()); // uncompressed size
        central_dir.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        central_dir.extend_from_slice(&0u16.to_le_bytes()); // extra len
        central_dir.extend_from_slice(&0u16.to_le_bytes()); // comment len
        central_dir.extend_from_slice(&0u16.to_le_bytes()); // disk number
        central_dir.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central_dir.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central_dir.extend_from_slice(&local_header_offset.to_le_bytes());
        central_dir.extend_from_slice(name_bytes);
    }

    let cd_offset = out.len() as u32;
    let cd_size = central_dir.len() as u32;
    let count = entries.len() as u16;
    out.extend_from_slice(&central_dir);

    // End of central directory record (22 bytes)
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // disk number
    out.extend_from_slice(&0u16.to_le_bytes()); // disk with CD
    out.extend_from_slice(&count.to_le_bytes()); // entries on this disk
    out.extend_from_slice(&count.to_le_bytes()); // total entries
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment length

    out
}

/// Read a ZIP archive from raw bytes. Returns `Err("DEFLATE_COMPRESSED")` if a non-stored entry is encountered.
pub fn read_stored_zip(zip_bytes: &[u8]) -> Result<Vec<ZipEntryData>, String> {
    if zip_bytes.len() < 22 {
        return Err("File too small to be a ZIP archive".to_string());
    }

    // Locate End of Central Directory (0x06054b50) scanning backwards from the end
    let min_eocd_off = zip_bytes.len().saturating_sub(65557);
    let mut eocd_off = None;
    let mut idx = zip_bytes.len() - 22;
    loop {
        if zip_bytes[idx..idx + 4] == [0x50, 0x4b, 0x05, 0x06] {
            eocd_off = Some(idx);
            break;
        }
        if idx <= min_eocd_off {
            break;
        }
        idx -= 1;
    }

    let mut entries = Vec::new();

    if let Some(eocd) = eocd_off {
        let total_entries =
            u16::from_le_bytes([zip_bytes[eocd + 10], zip_bytes[eocd + 11]]) as usize;
        let cd_offset = u32::from_le_bytes([
            zip_bytes[eocd + 16],
            zip_bytes[eocd + 17],
            zip_bytes[eocd + 18],
            zip_bytes[eocd + 19],
        ]) as usize;

        let mut pos = cd_offset;
        for _ in 0..total_entries {
            if pos + 46 > zip_bytes.len() {
                break;
            }
            let sig = u32::from_le_bytes([
                zip_bytes[pos],
                zip_bytes[pos + 1],
                zip_bytes[pos + 2],
                zip_bytes[pos + 3],
            ]);
            if sig != 0x0201_4b50 {
                break;
            }
            let method = u16::from_le_bytes([zip_bytes[pos + 10], zip_bytes[pos + 11]]);
            let comp_size = u32::from_le_bytes([
                zip_bytes[pos + 20],
                zip_bytes[pos + 21],
                zip_bytes[pos + 22],
                zip_bytes[pos + 23],
            ]) as usize;
            let name_len =
                u16::from_le_bytes([zip_bytes[pos + 28], zip_bytes[pos + 29]]) as usize;
            let extra_len =
                u16::from_le_bytes([zip_bytes[pos + 30], zip_bytes[pos + 31]]) as usize;
            let comment_len =
                u16::from_le_bytes([zip_bytes[pos + 32], zip_bytes[pos + 33]]) as usize;
            let local_off = u32::from_le_bytes([
                zip_bytes[pos + 42],
                zip_bytes[pos + 43],
                zip_bytes[pos + 44],
                zip_bytes[pos + 45],
            ]) as usize;

            if pos + 46 + name_len > zip_bytes.len() {
                break;
            }
            let name =
                String::from_utf8_lossy(&zip_bytes[pos + 46..pos + 46 + name_len]).into_owned();
            pos += 46 + name_len + extra_len + comment_len;

            if name.ends_with('/') {
                continue;
            }

            if local_off + 30 > zip_bytes.len() {
                return Err(format!("ZIP local header out of bounds for {}", name));
            }
            let l_sig = u32::from_le_bytes([
                zip_bytes[local_off],
                zip_bytes[local_off + 1],
                zip_bytes[local_off + 2],
                zip_bytes[local_off + 3],
            ]);
            if l_sig != 0x0403_4b50 {
                return Err(format!("Invalid ZIP local header signature for {}", name));
            }
            let l_name_len =
                u16::from_le_bytes([zip_bytes[local_off + 26], zip_bytes[local_off + 27]])
                    as usize;
            let l_extra_len =
                u16::from_le_bytes([zip_bytes[local_off + 28], zip_bytes[local_off + 29]])
                    as usize;
            let data_start = local_off + 30 + l_name_len + l_extra_len;
            if data_start + comp_size > zip_bytes.len() {
                return Err(format!("ZIP entry data out of bounds for {}", name));
            }

            if method != 0 {
                return Err("DEFLATE_COMPRESSED".to_string());
            }

            entries.push(ZipEntryData {
                name,
                compression_method: method,
                data: zip_bytes[data_start..data_start + comp_size].to_vec(),
            });
        }
        return Ok(entries);
    }

    // Fallback: sequential scan of Local File Headers
    let mut pos = 0;
    while pos + 30 <= zip_bytes.len() {
        let sig = u32::from_le_bytes([
            zip_bytes[pos],
            zip_bytes[pos + 1],
            zip_bytes[pos + 2],
            zip_bytes[pos + 3],
        ]);
        if sig != 0x0403_4b50 {
            break;
        }
        let method = u16::from_le_bytes([zip_bytes[pos + 8], zip_bytes[pos + 9]]);
        let comp_size = u32::from_le_bytes([
            zip_bytes[pos + 18],
            zip_bytes[pos + 19],
            zip_bytes[pos + 20],
            zip_bytes[pos + 21],
        ]) as usize;
        let name_len = u16::from_le_bytes([zip_bytes[pos + 26], zip_bytes[pos + 27]]) as usize;
        let extra_len = u16::from_le_bytes([zip_bytes[pos + 28], zip_bytes[pos + 29]]) as usize;
        let name_start = pos + 30;
        let data_start = name_start + name_len + extra_len;
        if data_start + comp_size > zip_bytes.len() {
            break;
        }
        let name = String::from_utf8_lossy(&zip_bytes[name_start..name_start + name_len]).into_owned();
        if !name.ends_with('/') {
            if method != 0 {
                return Err("DEFLATE_COMPRESSED".to_string());
            }
            entries.push(ZipEntryData {
                name,
                compression_method: method,
                data: zip_bytes[data_start..data_start + comp_size].to_vec(),
            });
        }
        pos = data_start + comp_size;
    }

    if entries.is_empty() {
        return Err("No valid ZIP entries found".to_string());
    }
    Ok(entries)
}

// ---------------------------------------------------------------------------
// XML & Binary Plist (`bplist00`) Parser
// ---------------------------------------------------------------------------

/// Parse either an XML plist or an Apple `bplist00` binary plist into string key-value pairs.
pub fn parse_plist_kv(raw: &[u8]) -> HashMap<String, String> {
    if raw.starts_with(b"bplist00") {
        if let Some(map) = parse_bplist00(raw) {
            return map;
        }
    }
    parse_xml_plist(raw)
}

fn parse_xml_plist(raw: &[u8]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let text = String::from_utf8_lossy(raw);
    let mut rest = text.as_ref();

    while let Some(k_start) = rest.find("<key>") {
        let after_k = &rest[k_start + 5..];
        let Some(k_end) = after_k.find("</key>") else {
            break;
        };
        let key = xml_unescape(after_k[..k_end].trim());
        rest = &after_k[k_end + 6..];

        let trimmed = rest.trim_start();
        if let Some(after_s) = trimmed.strip_prefix("<string>") {
            if let Some(s_end) = after_s.find("</string>") {
                let val = xml_unescape(&after_s[..s_end]);
                map.insert(key, val);
                rest = &after_s[s_end + 9..];
                continue;
            }
        } else if let Some(after_i) = trimmed.strip_prefix("<integer>") {
            if let Some(i_end) = after_i.find("</integer>") {
                let val = after_i[..i_end].trim().to_string();
                map.insert(key, val);
                rest = &after_i[i_end + 10..];
                continue;
            }
        } else if let Some(after_t) = trimmed.strip_prefix("<true/>") {
            map.insert(key, "true".to_string());
            rest = after_t;
            continue;
        } else if let Some(after_f) = trimmed.strip_prefix("<false/>") {
            map.insert(key, "false".to_string());
            rest = after_f;
            continue;
        }
    }
    map
}

fn xml_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

fn read_be_uint(bytes: &[u8]) -> usize {
    let mut val = 0usize;
    for &b in bytes {
        val = (val << 8) | (b as usize);
    }
    val
}

fn parse_bplist00(data: &[u8]) -> Option<HashMap<String, String>> {
    if data.len() < 40 || !data.starts_with(b"bplist00") {
        return None;
    }
    let trailer = &data[data.len() - 32..];
    let offset_int_size = trailer[6] as usize;
    let object_ref_size = trailer[7] as usize;
    let num_objects = read_be_uint(&trailer[8..16]);
    let top_object = read_be_uint(&trailer[16..24]);
    let offset_table_offset = read_be_uint(&trailer[24..32]);

    if offset_int_size == 0
        || object_ref_size == 0
        || num_objects == 0
        || top_object >= num_objects
        || offset_table_offset + num_objects * offset_int_size > data.len() - 32
    {
        return None;
    }

    let get_obj_offset = |obj_idx: usize| -> Option<usize> {
        if obj_idx >= num_objects {
            return None;
        }
        let start = offset_table_offset + obj_idx * offset_int_size;
        let off = read_be_uint(&data[start..start + offset_int_size]);
        if off < data.len() - 32 {
            Some(off)
        } else {
            None
        }
    };

    fn read_bplist_len(data: &[u8], off: usize, low_nibble: u8) -> Option<(usize, usize)> {
        if low_nibble < 0x0f {
            return Some((low_nibble as usize, off + 1));
        }
        if off + 2 > data.len() {
            return None;
        }
        let int_marker = data[off + 1];
        if (int_marker & 0xf0) != 0x10 {
            return None;
        }
        let int_bytes = 1usize << (int_marker & 0x0f);
        if off + 2 + int_bytes > data.len() {
            return None;
        }
        let len = read_be_uint(&data[off + 2..off + 2 + int_bytes]);
        Some((len, off + 2 + int_bytes))
    }

    let decode_scalar = |obj_idx: usize| -> Option<String> {
        let off = get_obj_offset(obj_idx)?;
        let marker = data[off];
        let kind = marker & 0xf0;
        let info = marker & 0x0f;
        match kind {
            0x00 => match info {
                0x08 => Some("false".to_string()),
                0x09 => Some("true".to_string()),
                _ => None,
            },
            0x10 => {
                let nbytes = 1usize << info;
                if off + 1 + nbytes > data.len() {
                    return None;
                }
                Some(read_be_uint(&data[off + 1..off + 1 + nbytes]).to_string())
            }
            0x50 => {
                let (len, start) = read_bplist_len(data, off, info)?;
                if start + len > data.len() {
                    return None;
                }
                Some(String::from_utf8_lossy(&data[start..start + len]).into_owned())
            }
            0x60 => {
                let (char_len, start) = read_bplist_len(data, off, info)?;
                let byte_len = char_len * 2;
                if start + byte_len > data.len() {
                    return None;
                }
                let mut u16s = Vec::with_capacity(char_len);
                for i in 0..char_len {
                    let hi = data[start + i * 2];
                    let lo = data[start + i * 2 + 1];
                    u16s.push(u16::from_be_bytes([hi, lo]));
                }
                Some(String::from_utf16_lossy(&u16s))
            }
            _ => None,
        }
    };

    let root_off = get_obj_offset(top_object)?;
    let root_marker = data[root_off];
    if (root_marker & 0xf0) != 0xd0 {
        return None;
    }
    let (pair_count, refs_start) = read_bplist_len(data, root_off, root_marker & 0x0f)?;
    let total_refs_bytes = pair_count * 2 * object_ref_size;
    if refs_start + total_refs_bytes > data.len() {
        return None;
    }

    let mut map = HashMap::new();
    for i in 0..pair_count {
        let k_ref_off = refs_start + i * object_ref_size;
        let v_ref_off = refs_start + (pair_count + i) * object_ref_size;
        let k_idx = read_be_uint(&data[k_ref_off..k_ref_off + object_ref_size]);
        let v_idx = read_be_uint(&data[v_ref_off..v_ref_off + object_ref_size]);
        if let (Some(k), Some(v)) = (decode_scalar(k_idx), decode_scalar(v_idx)) {
            map.insert(k, v);
        }
    }
    Some(map)
}

// ---------------------------------------------------------------------------
// Mach-O 64 & CodeSignature Parser
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct MachOInspection {
    pub valid: bool,
    pub is_arm64: bool,
    pub is_encrypted: bool,
    pub cryptid: u32,
    pub dylibs: Vec<String>,
    pub frameworks: Vec<String>,
    pub cdhash: [u8; 20],
    pub cdhash_hex: String,
}

fn read_u32(data: &[u8], off: usize, le: bool) -> u32 {
    let b = [data[off], data[off + 1], data[off + 2], data[off + 3]];
    if le {
        u32::from_le_bytes(b)
    } else {
        u32::from_be_bytes(b)
    }
}

fn read_u64(data: &[u8], off: usize, le: bool) -> u64 {
    let b = [
        data[off],
        data[off + 1],
        data[off + 2],
        data[off + 3],
        data[off + 4],
        data[off + 5],
        data[off + 6],
        data[off + 7],
    ];
    if le {
        u64::from_le_bytes(b)
    } else {
        u64::from_be_bytes(b)
    }
}

/// Extract the 20-byte CDHash from an `LC_CODE_SIGNATURE` SuperBlob (`0xfade0cc0`).
pub fn extract_codesig_cdhash(sig_data: &[u8]) -> Option<[u8; 20]> {
    if sig_data.len() < 12 {
        return None;
    }
    let magic = read_u32(sig_data, 0, false);
    if magic != CSMAGIC_EMBEDDED_SIGNATURE {
        return None;
    }
    let count = read_u32(sig_data, 8, false) as usize;
    let mut best_hash: Option<(u8, [u8; 20])> = None;

    for i in 0..count {
        let idx_off = 12 + i * 8;
        if idx_off + 8 > sig_data.len() {
            break;
        }
        let _slot_type = read_u32(sig_data, idx_off, false);
        let blob_off = read_u32(sig_data, idx_off + 4, false) as usize;
        if blob_off + 8 > sig_data.len() {
            continue;
        }
        let b_magic = read_u32(sig_data, blob_off, false);
        let b_len = read_u32(sig_data, blob_off + 4, false) as usize;
        let blob_end = (blob_off + b_len).min(sig_data.len());
        let blob = &sig_data[blob_off..blob_end];

        if b_magic == CSMAGIC_CODEDIRECTORY && blob.len() >= 40 {
            let hash_type = blob[37];
            let mut cdhash = [0u8; 20];
            match hash_type {
                CS_HASHTYPE_SHA1 => {
                    cdhash = sha1_digest(blob);
                }
                CS_HASHTYPE_SHA384 => {
                    let d = sha384_digest(blob);
                    cdhash.copy_from_slice(&d[..20]);
                }
                _ => {
                    let d = sha256_digest(blob);
                    cdhash.copy_from_slice(&d[..20]);
                }
            }
            if best_hash.is_none()
                || (hash_type == CS_HASHTYPE_SHA256
                    && best_hash.map(|(ht, _)| ht) != Some(CS_HASHTYPE_SHA256))
            {
                best_hash = Some((hash_type, cdhash));
            }
        }
    }

    best_hash.map(|(_, h)| h)
}

const KNOWN_FRAMEWORKS: &[(&str, &str)] = &[
    ("Metal.framework", "Metal"),
    ("MetalKit.framework", "MetalKit"),
    ("OpenGLES.framework", "OpenGLES"),
    ("GLKit.framework", "GLKit"),
    ("SpriteKit.framework", "SpriteKit"),
    ("SceneKit.framework", "SceneKit"),
    ("UnityFramework.framework", "UnityFramework"),
    ("UIKit.framework", "UIKit"),
    ("QuartzCore.framework", "QuartzCore"),
    ("CoreGraphics.framework", "CoreGraphics"),
];

/// Parse a thin or universal (Fat) Mach-O binary, preferring the `CPU_TYPE_ARM64` slice.
pub fn parse_macho_binary(data: &[u8]) -> MachOInspection {
    if data.len() < 28 {
        return MachOInspection::default();
    }
    let magic_le = read_u32(data, 0, true);
    if matches!(
        magic_le,
        FAT_MAGIC | FAT_CIGAM | FAT_MAGIC_64 | FAT_CIGAM_64
    ) {
        let is_fat64 = matches!(magic_le, FAT_MAGIC_64 | FAT_CIGAM_64);
        let n_be = read_u32(data, 4, false);
        let le = !(1..=32).contains(&n_be);
        let nfat = read_u32(data, 4, le) as usize;
        let mut off = 8usize;
        let mut chosen_slice: Option<&[u8]> = None;
        let mut has_arm64 = false;

        for _ in 0..nfat {
            if is_fat64 {
                if off + 32 > data.len() {
                    break;
                }
                let cputype = read_u32(data, off, le);
                let slice_off = read_u64(data, off + 8, le) as usize;
                let slice_size = read_u64(data, off + 16, le) as usize;
                off += 32;
                if cputype == CPU_TYPE_ARM64 {
                    has_arm64 = true;
                }
                if slice_off + slice_size <= data.len()
                    && (chosen_slice.is_none() || cputype == CPU_TYPE_ARM64)
                {
                    chosen_slice = Some(&data[slice_off..slice_off + slice_size]);
                }
            } else {
                if off + 20 > data.len() {
                    break;
                }
                let cputype = read_u32(data, off, le);
                let slice_off = read_u32(data, off + 8, le) as usize;
                let slice_size = read_u32(data, off + 12, le) as usize;
                off += 20;
                if cputype == CPU_TYPE_ARM64 {
                    has_arm64 = true;
                }
                if slice_off + slice_size <= data.len()
                    && (chosen_slice.is_none() || cputype == CPU_TYPE_ARM64)
                {
                    chosen_slice = Some(&data[slice_off..slice_off + slice_size]);
                }
            }
        }
        if let Some(s) = chosen_slice {
            let mut res = parse_macho_slice(s);
            if has_arm64 {
                res.is_arm64 = true;
            }
            return res;
        }
        return MachOInspection::default();
    }

    parse_macho_slice(data)
}

fn parse_macho_slice(slice: &[u8]) -> MachOInspection {
    let mut info = MachOInspection::default();
    if slice.len() < 28 {
        return info;
    }
    let magic = read_u32(slice, 0, true);
    let (le, hdr_size) = match magic {
        MH_MAGIC_64 => (true, 32usize),
        MH_CIGAM_64 => (false, 32usize),
        MH_MAGIC => (true, 28usize),
        MH_CIGAM => (false, 28usize),
        _ => return info,
    };
    if slice.len() < hdr_size {
        return info;
    }

    let cpu_type = read_u32(slice, 4, le);
    let ncmds = read_u32(slice, 16, le) as usize;
    let sizeofcmds = read_u32(slice, 20, le) as usize;

    info.valid = true;
    info.is_arm64 = cpu_type == CPU_TYPE_ARM64;

    let mut off = hdr_size;
    let max_off = slice.len().min(hdr_size + sizeofcmds);
    let mut sig_range: Option<(usize, usize)> = None;

    for _ in 0..ncmds {
        if off + 8 > max_off {
            break;
        }
        let cmd = read_u32(slice, off, le);
        let cmdsize = read_u32(slice, off + 4, le) as usize;
        if cmdsize < 8 || off + cmdsize > slice.len() {
            break;
        }

        if matches!(
            cmd,
            LC_LOAD_DYLIB
                | LC_WEAK_DYLIB
                | LC_REEXPORT_DYLIB
                | LC_LAZY_LOAD_DYLIB
                | LC_LOAD_UPWARD_DYLIB
        ) && cmdsize >= 24
        {
            let str_off = read_u32(slice, off + 8, le) as usize;
            if str_off >= 8 && str_off < cmdsize {
                let raw = &slice[off + str_off..off + cmdsize];
                let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
                let dylib = String::from_utf8_lossy(&raw[..end]).into_owned();
                for &(marker, fw_name) in KNOWN_FRAMEWORKS {
                    if dylib.contains(marker) && !info.frameworks.iter().any(|f| f == fw_name) {
                        info.frameworks.push(fw_name.to_string());
                    }
                }
                info.dylibs.push(dylib);
            }
        } else if matches!(cmd, LC_ENCRYPTION_INFO_64 | LC_ENCRYPTION_INFO) && cmdsize >= 20 {
            let cryptid = read_u32(slice, off + 16, le);
            info.cryptid = cryptid;
            if cryptid != 0 {
                info.is_encrypted = true;
            }
        } else if cmd == LC_CODE_SIGNATURE && cmdsize >= 16 {
            let dataoff = read_u32(slice, off + 8, le) as usize;
            let datasize = read_u32(slice, off + 12, le) as usize;
            if dataoff + datasize <= slice.len() {
                sig_range = Some((dataoff, datasize));
            }
        }

        off += cmdsize;
    }

    if let Some((doff, dsize)) = sig_range {
        if let Some(cdh) = extract_codesig_cdhash(&slice[doff..doff + dsize]) {
            info.cdhash = cdh;
            info.cdhash_hex = hex_encode(&cdh);
        }
    }

    if info.cdhash_hex.is_empty() {
        let digest = sha256_digest(slice);
        info.cdhash.copy_from_slice(&digest[..20]);
        info.cdhash_hex = hex_encode(&info.cdhash);
    }

    info
}

pub fn classify_graphics_api(frameworks: &[String]) -> String {
    let has_metal = frameworks.iter().any(|f| f == "Metal" || f == "MetalKit");
    let has_gles = frameworks.iter().any(|f| f == "OpenGLES" || f == "GLKit");
    let has_uikit = frameworks
        .iter()
        .any(|f| f == "UIKit" || f == "QuartzCore" || f == "CoreGraphics");

    if has_metal && has_gles {
        "OpenGL ES + Metal".to_string()
    } else if has_metal {
        if has_uikit {
            "Metal + UIKit".to_string()
        } else {
            "Metal".to_string()
        }
    } else if has_gles {
        if has_uikit {
            "OpenGL ES / 2D".to_string()
        } else {
            "OpenGL ES".to_string()
        }
    } else {
        "UIKit / CoreAnimation 2D".to_string()
    }
}

// ---------------------------------------------------------------------------
// Demo IPA Generator (`create_demo_ipa`)
// ---------------------------------------------------------------------------

fn pack_segment_64(
    name: &str,
    vmaddr: u64,
    vmsize: u64,
    fileoff: u64,
    filesize: u64,
    maxprot: u32,
    initprot: u32,
    sections: &[Vec<u8>],
) -> Vec<u8> {
    let cmdsize = (72 + 80 * sections.len()) as u32;
    let mut out = Vec::with_capacity(cmdsize as usize);
    out.extend_from_slice(&LC_SEGMENT_64.to_le_bytes());
    out.extend_from_slice(&cmdsize.to_le_bytes());
    let mut segname = [0u8; 16];
    let nb = name.as_bytes();
    let clen = nb.len().min(16);
    segname[..clen].copy_from_slice(&nb[..clen]);
    out.extend_from_slice(&segname);
    out.extend_from_slice(&vmaddr.to_le_bytes());
    out.extend_from_slice(&vmsize.to_le_bytes());
    out.extend_from_slice(&fileoff.to_le_bytes());
    out.extend_from_slice(&filesize.to_le_bytes());
    out.extend_from_slice(&maxprot.to_le_bytes());
    out.extend_from_slice(&initprot.to_le_bytes());
    out.extend_from_slice(&(sections.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for s in sections {
        out.extend_from_slice(s);
    }
    out
}

fn pack_section_64(
    sectname: &str,
    segname: &str,
    addr: u64,
    size: u64,
    offset: u32,
    align: u32,
    flags: u32,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(80);
    let mut s_sect = [0u8; 16];
    let sb = sectname.as_bytes();
    s_sect[..sb.len().min(16)].copy_from_slice(&sb[..sb.len().min(16)]);
    let mut s_seg = [0u8; 16];
    let gb = segname.as_bytes();
    s_seg[..gb.len().min(16)].copy_from_slice(&gb[..gb.len().min(16)]);

    out.extend_from_slice(&s_sect);
    out.extend_from_slice(&s_seg);
    out.extend_from_slice(&addr.to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&offset.to_le_bytes());
    out.extend_from_slice(&align.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // reloff
    out.extend_from_slice(&0u32.to_le_bytes()); // nreloc
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // reserved1
    out.extend_from_slice(&0u32.to_le_bytes()); // reserved2
    out.extend_from_slice(&0u32.to_le_bytes()); // reserved3
    out
}

fn pack_load_dylib(path: &str) -> Vec<u8> {
    let raw_len = path.len() + 1;
    let total_len = (24 + raw_len + 7) & !7;
    let mut out = Vec::with_capacity(total_len);
    out.extend_from_slice(&LC_LOAD_DYLIB.to_le_bytes());
    out.extend_from_slice(&(total_len as u32).to_le_bytes());
    out.extend_from_slice(&24u32.to_le_bytes()); // name offset
    out.extend_from_slice(&2u32.to_le_bytes()); // timestamp
    out.extend_from_slice(&0x0001_0000u32.to_le_bytes()); // current_version
    out.extend_from_slice(&0x0001_0000u32.to_le_bytes()); // compatibility_version
    out.extend_from_slice(path.as_bytes());
    while out.len() < total_len {
        out.push(0);
    }
    out
}

fn build_codesig_superblob(
    image: &[u8],
    code_limit: usize,
    text_size: u64,
    ident_str: &str,
    entitlements_xml: &[u8],
) -> Vec<u8> {
    let page_shift: u8 = 12;
    let page_size: usize = 1 << page_shift;
    let mut ident = ident_str.as_bytes().to_vec();
    ident.push(0);

    let mut ent_blob = Vec::with_capacity(8 + entitlements_xml.len());
    ent_blob.extend_from_slice(&CSMAGIC_EMBEDDED_ENTITLEMENTS.to_be_bytes());
    ent_blob.extend_from_slice(&((8 + entitlements_xml.len()) as u32).to_be_bytes());
    ent_blob.extend_from_slice(entitlements_xml);

    let nspecial: u32 = 5;
    let mut all_hashes = Vec::new();
    let ent_hash = sha256_digest(&ent_blob);
    all_hashes.extend_from_slice(&ent_hash);
    all_hashes.extend_from_slice(&[0u8; 32 * 4]); // slots -4..-1

    let mut nslots: u32 = 0;
    let mut off = 0usize;
    while off < code_limit {
        let end = (off + page_size).min(code_limit);
        let page_hash = sha256_digest(&image[off..end]);
        all_hashes.extend_from_slice(&page_hash);
        nslots += 1;
        off += page_size;
    }

    let hdr_len: u32 = 88;
    let ident_off: u32 = hdr_len;
    let hash_off: u32 = ident_off + (ident.len() as u32) + 32 * nspecial;
    let cd_len: u32 = ident_off + (ident.len() as u32) + (all_hashes.len() as u32);

    let mut cd = Vec::with_capacity(cd_len as usize);
    cd.extend_from_slice(&CSMAGIC_CODEDIRECTORY.to_be_bytes());
    cd.extend_from_slice(&cd_len.to_be_bytes());
    cd.extend_from_slice(&0x0002_0400u32.to_be_bytes()); // version
    cd.extend_from_slice(&CS_ADHOC.to_be_bytes()); // flags
    cd.extend_from_slice(&hash_off.to_be_bytes());
    cd.extend_from_slice(&ident_off.to_be_bytes());
    cd.extend_from_slice(&nspecial.to_be_bytes());
    cd.extend_from_slice(&nslots.to_be_bytes());
    cd.extend_from_slice(&(code_limit as u32).to_be_bytes());
    cd.push(32); // hashSize
    cd.push(CS_HASHTYPE_SHA256); // hashType = 2
    cd.push(0); // platform
    cd.push(page_shift); // pageSize shift
    cd.extend_from_slice(&0u32.to_be_bytes()); // spare2
    cd.extend_from_slice(&0u32.to_be_bytes()); // scatterOffset
    cd.extend_from_slice(&0u32.to_be_bytes()); // teamOffset
    cd.extend_from_slice(&0u32.to_be_bytes()); // spare3
    cd.extend_from_slice(&0u64.to_be_bytes()); // codeLimit64
    cd.extend_from_slice(&0u64.to_be_bytes()); // execSegBase
    cd.extend_from_slice(&text_size.to_be_bytes()); // execSegLimit
    cd.extend_from_slice(&1u64.to_be_bytes()); // execSegFlags = CS_EXECSEG_MAIN_BINARY
    cd.extend_from_slice(&ident);
    cd.extend_from_slice(&all_hashes);

    let sb_hdr_len: u32 = 12 + 8 * 2;
    let total_len: u32 = sb_hdr_len + (cd.len() as u32) + (ent_blob.len() as u32);
    let mut sb = Vec::with_capacity(total_len as usize);
    sb.extend_from_slice(&CSMAGIC_EMBEDDED_SIGNATURE.to_be_bytes());
    sb.extend_from_slice(&total_len.to_be_bytes());
    sb.extend_from_slice(&2u32.to_be_bytes()); // 2 blobs
    sb.extend_from_slice(&CSSLOT_CODEDIRECTORY.to_be_bytes());
    sb.extend_from_slice(&sb_hdr_len.to_be_bytes());
    sb.extend_from_slice(&CSSLOT_ENTITLEMENTS.to_be_bytes());
    sb.extend_from_slice(&(sb_hdr_len + cd.len() as u32).to_be_bytes());
    sb.extend_from_slice(&cd);
    sb.extend_from_slice(&ent_blob);
    sb
}

/// Build a valid ARM64 Mach-O 64-bit executable (`MH_MAGIC_64 = 0xfeedfacf`, `CPU_TYPE_ARM64 = 0x0100000c`)
/// with `LC_LOAD_DYLIB`, `LC_ENCRYPTION_INFO_64 (cryptid=0)`, and `LC_CODE_SIGNATURE` (`CS_SuperBlob` + `CS_CodeDirectory`).
pub fn build_demo_macho_executable(ident_str: &str) -> Vec<u8> {
    let base_vm: u64 = 0x1_0000_0000;
    let seg_size: usize = 0x4000;
    let code_off: usize = 0x800;

    // Minimal ARM64 entry stub: mov w0, #0 (0x52800000); ret (0xd65f03c0)
    let mut arm64_code = Vec::with_capacity(8);
    arm64_code.extend_from_slice(&0x5280_0000u32.to_le_bytes());
    arm64_code.extend_from_slice(&0xd65f_03c0u32.to_le_bytes());

    let entitlements_xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
           <key>application-identifier</key>\n\
           <string>SWEETDEMO.{}</string>\n\
           <key>get-task-allow</key>\n\
           <true/>\n\
           <key>platform-application</key>\n\
           <true/>\n\
         </dict>\n\
         </plist>\n",
        ident_str
    );

    let dummy_sig = build_codesig_superblob(
        &vec![0u8; seg_size],
        seg_size,
        seg_size as u64,
        ident_str,
        entitlements_xml.as_bytes(),
    );
    let sig_size = dummy_sig.len();
    let linkedit_vmsize = ((sig_size + seg_size - 1) & !(seg_size - 1)) as u64;

    let text_sect = pack_section_64(
        "__text",
        "__TEXT",
        base_vm + (code_off as u64),
        arm64_code.len() as u64,
        code_off as u32,
        2,
        0x8000_0400,
    );

    let dylibs = [
        "/usr/lib/libSystem.B.dylib",
        "/System/Library/Frameworks/Foundation.framework/Foundation",
        "/System/Library/Frameworks/UIKit.framework/UIKit",
        "/System/Library/Frameworks/QuartzCore.framework/QuartzCore",
        "/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics",
        "/System/Library/Frameworks/OpenGLES.framework/OpenGLES",
        "/System/Library/Frameworks/Metal.framework/Metal",
    ];

    let uuid_digest = sha256_digest(ident_str.as_bytes());
    let mut lc_uuid = Vec::with_capacity(24);
    lc_uuid.extend_from_slice(&LC_UUID.to_le_bytes());
    lc_uuid.extend_from_slice(&24u32.to_le_bytes());
    lc_uuid.extend_from_slice(&uuid_digest[..16]);

    let mut lc_main = Vec::with_capacity(24);
    lc_main.extend_from_slice(&LC_MAIN.to_le_bytes());
    lc_main.extend_from_slice(&24u32.to_le_bytes());
    lc_main.extend_from_slice(&(code_off as u64).to_le_bytes());
    lc_main.extend_from_slice(&0u64.to_le_bytes());

    let mut lc_enc = Vec::with_capacity(24);
    lc_enc.extend_from_slice(&LC_ENCRYPTION_INFO_64.to_le_bytes());
    lc_enc.extend_from_slice(&24u32.to_le_bytes());
    lc_enc.extend_from_slice(&0u32.to_le_bytes()); // cryptoff
    lc_enc.extend_from_slice(&0u32.to_le_bytes()); // cryptsize
    lc_enc.extend_from_slice(&0u32.to_le_bytes()); // cryptid = 0
    lc_enc.extend_from_slice(&0u32.to_le_bytes()); // pad

    let mut lc_codesig = Vec::with_capacity(16);
    lc_codesig.extend_from_slice(&LC_CODE_SIGNATURE.to_le_bytes());
    lc_codesig.extend_from_slice(&16u32.to_le_bytes());
    lc_codesig.extend_from_slice(&(seg_size as u32).to_le_bytes());
    lc_codesig.extend_from_slice(&(sig_size as u32).to_le_bytes());

    let mut load_cmds: Vec<Vec<u8>> = vec![
        pack_segment_64("__PAGEZERO", 0, base_vm, 0, 0, 0, 0, &[]),
        pack_segment_64(
            "__TEXT",
            base_vm,
            seg_size as u64,
            0,
            seg_size as u64,
            5,
            5,
            &[text_sect],
        ),
        pack_segment_64(
            "__LINKEDIT",
            base_vm + (seg_size as u64),
            linkedit_vmsize,
            seg_size as u64,
            sig_size as u64,
            1,
            1,
            &[],
        ),
        lc_uuid,
        lc_main,
        lc_enc,
    ];
    for d in &dylibs {
        load_cmds.push(pack_load_dylib(d));
    }
    load_cmds.push(lc_codesig);

    let sizeofcmds: usize = load_cmds.iter().map(|c| c.len()).sum();
    let mut header = Vec::with_capacity(32);
    header.extend_from_slice(&MH_MAGIC_64.to_le_bytes());
    header.extend_from_slice(&CPU_TYPE_ARM64.to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes()); // cpusubtype
    header.extend_from_slice(&MH_EXECUTE.to_le_bytes());
    header.extend_from_slice(&(load_cmds.len() as u32).to_le_bytes());
    header.extend_from_slice(&(sizeofcmds as u32).to_le_bytes());
    header.extend_from_slice(&(MH_NOUNDEFS | MH_DYLDLINK | MH_TWOLEVEL | MH_PIE).to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes()); // reserved

    let mut image = vec![0u8; seg_size];
    image[..32].copy_from_slice(&header);
    let mut off = 32;
    for cmd in &load_cmds {
        image[off..off + cmd.len()].copy_from_slice(cmd);
        off += cmd.len();
    }
    image[code_off..code_off + arm64_code.len()].copy_from_slice(&arm64_code);

    let real_sig = build_codesig_superblob(
        &image,
        seg_size,
        seg_size as u64,
        ident_str,
        entitlements_xml.as_bytes(),
    );
    image.extend_from_slice(&real_sig);
    image
}

/// Create a valid, self-contained demo `.ipa` (`SweetBenchmark.ipa`) at `path` and return its `AppMetadata`.
pub fn create_demo_ipa(path: &Path) -> Result<AppMetadata, String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create parent dir: {}", e))?;
        }
    }

    let info_plist_xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
  <key>CFBundleDevelopmentRegion</key>\n\
  <string>en</string>\n\
  <key>CFBundleDisplayName</key>\n\
  <string>Sweet 3D Bench</string>\n\
  <key>CFBundleExecutable</key>\n\
  <string>SweetBenchmark</string>\n\
  <key>CFBundleIdentifier</key>\n\
  <string>com.sweet.benchmark</string>\n\
  <key>CFBundleInfoDictionaryVersion</key>\n\
  <string>6.0</string>\n\
  <key>CFBundleName</key>\n\
  <string>SweetBenchmark</string>\n\
  <key>CFBundlePackageType</key>\n\
  <string>APPL</string>\n\
  <key>CFBundleShortVersionString</key>\n\
  <string>1.0.0</string>\n\
  <key>CFBundleVersion</key>\n\
  <string>1</string>\n\
  <key>LSRequiresIPhoneOS</key>\n\
  <true/>\n\
  <key>MinimumOSVersion</key>\n\
  <string>16.0</string>\n\
</dict>\n\
</plist>\n";

    let macho_bin = build_demo_macho_executable("com.sweet.benchmark");
    let pkg_info = b"APPL????";

    let zip_bytes = write_stored_zip(&[
        ("Payload/SweetBenchmark.app/Info.plist", info_plist_xml.as_bytes()),
        ("Payload/SweetBenchmark.app/PkgInfo", pkg_info),
        ("Payload/SweetBenchmark.app/SweetBenchmark", &macho_bin),
    ]);

    fs::write(path, &zip_bytes).map_err(|e| format!("Failed to write demo IPA: {}", e))?;
    inspect_app_or_ipa(path)
}

// ---------------------------------------------------------------------------
// App / IPA Inspector (`inspect_app_or_ipa`)
// ---------------------------------------------------------------------------

fn find_app_dir(path: &Path) -> Result<PathBuf, String> {
    if path.join("Info.plist").is_file() {
        return Ok(path.to_path_buf());
    }
    let payload = path.join("Payload");
    if payload.is_dir() {
        if let Ok(rd) = fs::read_dir(&payload) {
            for entry in rd.flatten() {
                let p = entry.path();
                if p.is_dir()
                    && p.extension().and_then(|s| s.to_str()) == Some("app")
                    && p.join("Info.plist").is_file()
                {
                    return Ok(p);
                }
            }
        }
    }
    if let Ok(rd) = fs::read_dir(path) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir()
                && p.extension().and_then(|s| s.to_str()) == Some("app")
                && p.join("Info.plist").is_file()
            {
                return Ok(p);
            }
        }
    }
    Err(format!("No .app bundle with Info.plist found in {:?}", path))
}

fn build_metadata_from_parts(
    app_folder_stem: &str,
    info_plist_bytes: &[u8],
    exec_bytes: Option<&[u8]>,
) -> Result<AppMetadata, String> {
    let kv = parse_plist_kv(info_plist_bytes);
    let bundle_id = kv
        .get("CFBundleIdentifier")
        .cloned()
        .unwrap_or_default();
    if bundle_id.is_empty() {
        return Err("Info.plist is missing CFBundleIdentifier".to_string());
    }
    let display_name = kv
        .get("CFBundleDisplayName")
        .or_else(|| kv.get("CFBundleName"))
        .cloned()
        .unwrap_or_else(|| app_folder_stem.to_string());
    let version = kv
        .get("CFBundleShortVersionString")
        .or_else(|| kv.get("CFBundleVersion"))
        .cloned()
        .unwrap_or_else(|| "1.0.0".to_string());
    let executable = kv
        .get("CFBundleExecutable")
        .cloned()
        .unwrap_or_else(|| app_folder_stem.to_string());

    let (is_arm64, is_encrypted, graphics_api, cdhash_hex) = if let Some(bin) = exec_bytes {
        let m = parse_macho_binary(bin);
        let gfx = classify_graphics_api(&m.frameworks);
        (m.is_arm64, m.is_encrypted, gfx, m.cdhash_hex)
    } else {
        (
            false,
            false,
            "UIKit / CoreAnimation 2D".to_string(),
            String::new(),
        )
    };

    let bundle_uuid = deterministic_uuid_string(&format!("sweet.bundle.{}", bundle_id));
    let data_uuid = deterministic_uuid_string(&format!("sweet.data.{}", bundle_id));

    Ok(AppMetadata {
        bundle_id,
        display_name,
        version,
        executable,
        graphics_api,
        is_arm64,
        is_encrypted,
        cdhash_hex,
        bundle_uuid,
        data_uuid,
    })
}

fn find_sweet_ipa_py() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from("tools/sweet-ipa.py"),
        PathBuf::from("../tools/sweet-ipa.py"),
        PathBuf::from("../../tools/sweet-ipa.py"),
        PathBuf::from("../../../tools/sweet-ipa.py"),
    ];
    for c in &candidates {
        if c.is_file() {
            return Some(c.clone());
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut cur = exe.parent();
        for _ in 0..6 {
            if let Some(dir) = cur {
                let candidate = dir.join("tools").join("sweet-ipa.py");
                if candidate.is_file() {
                    return Some(candidate);
                }
                cur = dir.parent();
            } else {
                break;
            }
        }
    }
    None
}

fn json_extract_string(json: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{}\"", key);
    let idx = json.find(&pattern)?;
    let after = &json[idx + pattern.len()..];
    let colon = after.find(':')?;
    let val_part = after[colon + 1..].trim_start();
    if !val_part.starts_with('"') {
        return None;
    }
    let s = &val_part[1..];
    let end = s.find('"')?;
    Some(s[..end].to_string())
}

fn json_extract_bool(json: &str, key: &str) -> Option<bool> {
    let pattern = format!("\"{}\"", key);
    let idx = json.find(&pattern)?;
    let after = &json[idx + pattern.len()..];
    let colon = after.find(':')?;
    let val_part = after[colon + 1..].trim_start();
    if val_part.starts_with("true") {
        Some(true)
    } else if val_part.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn inspect_via_python_fallback(path: &Path) -> Result<AppMetadata, String> {
    let Some(script) = find_sweet_ipa_py() else {
        return Err("DEFLATE-compressed IPA requires tools/sweet-ipa.py fallback".to_string());
    };
    let output = Command::new("python")
        .arg(&script)
        .arg("info")
        .arg(path)
        .arg("--json")
        .output()
        .map_err(|e| format!("Failed to invoke python tools/sweet-ipa.py: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "sweet-ipa.py failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let json = String::from_utf8_lossy(&output.stdout);
    let bundle_id = json_extract_string(&json, "bundle_id").unwrap_or_default();
    let display_name = json_extract_string(&json, "display_name").unwrap_or_default();
    let version = json_extract_string(&json, "short_version").unwrap_or_else(|| "1.0".to_string());
    let executable = json_extract_string(&json, "executable").unwrap_or_default();
    let graphics_api = json_extract_string(&json, "primary_api")
        .unwrap_or_else(|| "UIKit / CoreAnimation 2D".to_string());
    let is_arm64 = json_extract_bool(&json, "arm64_supported").unwrap_or(false);
    let is_encrypted = json_extract_bool(&json, "encrypted").unwrap_or(false);
    let cdhash_hex = json_extract_string(&json, "cdhash").unwrap_or_default();

    let bundle_uuid = deterministic_uuid_string(&format!("sweet.bundle.{}", bundle_id));
    let data_uuid = deterministic_uuid_string(&format!("sweet.data.{}", bundle_id));

    Ok(AppMetadata {
        bundle_id,
        display_name,
        version,
        executable,
        graphics_api,
        is_arm64,
        is_encrypted,
        cdhash_hex,
        bundle_uuid,
        data_uuid,
    })
}

/// Inspect an `.app` directory or `.ipa` archive and return its `AppMetadata`.
pub fn inspect_app_or_ipa(path: &Path) -> Result<AppMetadata, String> {
    if !path.exists() {
        return Err(format!("Path does not exist: {:?}", path));
    }

    if path.is_dir() {
        let app_dir = find_app_dir(path)?;
        let stem = app_dir
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("App");
        let plist_bytes = fs::read(app_dir.join("Info.plist"))
            .map_err(|e| format!("Failed to read Info.plist: {}", e))?;
        let kv = parse_plist_kv(&plist_bytes);
        let exec_name = kv
            .get("CFBundleExecutable")
            .cloned()
            .unwrap_or_else(|| stem.to_string());
        let exec_path = app_dir.join(&exec_name);
        let exec_bytes = fs::read(&exec_path).ok();
        return build_metadata_from_parts(stem, &plist_bytes, exec_bytes.as_deref());
    }

    let raw_zip = fs::read(path).map_err(|e| format!("Failed to read file {:?}: {}", path, e))?;
    match read_stored_zip(&raw_zip) {
        Ok(entries) => {
            let mut app_prefix = String::new();
            let mut app_stem = "App".to_string();
            for e in &entries {
                let parts: Vec<&str> = e.name.trim_matches('/').split('/').collect();
                if parts.len() >= 2 && parts[0] == "Payload" && parts[1].ends_with(".app") {
                    app_prefix = format!("{}/{}/", parts[0], parts[1]);
                    app_stem = parts[1].trim_end_matches(".app").to_string();
                    break;
                } else if !parts.is_empty() && parts[0].ends_with(".app") {
                    app_prefix = format!("{}/", parts[0]);
                    app_stem = parts[0].trim_end_matches(".app").to_string();
                    break;
                }
            }
            if app_prefix.is_empty() {
                return Err("No Payload/*.app bundle found in IPA archive".to_string());
            }

            let plist_target = format!("{}Info.plist", app_prefix);
            let plist_entry = entries
                .iter()
                .find(|e| e.name.eq_ignore_ascii_case(&plist_target))
                .ok_or_else(|| format!("{} not found in IPA archive", plist_target))?;

            let kv = parse_plist_kv(&plist_entry.data);
            let exec_name = kv
                .get("CFBundleExecutable")
                .cloned()
                .unwrap_or_else(|| app_stem.clone());
            let exec_target = format!("{}{}", app_prefix, exec_name);
            let exec_entry = entries.iter().find(|e| e.name == exec_target);

            build_metadata_from_parts(
                &app_stem,
                &plist_entry.data,
                exec_entry.map(|e| e.data.as_slice()),
            )
        }
        Err(reason) if reason == "DEFLATE_COMPRESSED" => inspect_via_python_fallback(path),
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// Rootfs Installer & Installed Apps Registry (`install_app_to_root`, `list_installed_apps`)
// ---------------------------------------------------------------------------

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("Failed to create dir {:?}: {}", dst, e))?;
    let rd = fs::read_dir(src).map_err(|e| format!("Failed to read dir {:?}: {}", src, e))?;
    for entry in rd.flatten() {
        let p = entry.path();
        let target = dst.join(entry.file_name());
        if p.is_dir() {
            copy_dir_recursive(&p, &target)?;
        } else if p.is_file() {
            fs::copy(&p, &target).map_err(|e| format!("Failed to copy {:?}: {}", p, e))?;
        }
    }
    Ok(())
}

fn write_mcm_metadata_plist(
    path: &Path,
    bundle_id: &str,
    uuid: &str,
    content_class: u32,
    short_ver: &str,
) -> Result<(), String> {
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
  <key>MCMMetadataContentClass</key>\n\
  <integer>{}</integer>\n\
  <key>MCMMetadataIdentifier</key>\n\
  <string>{}</string>\n\
  <key>MCMMetadataPersona</key>\n\
  <integer>0</integer>\n\
  <key>MCMMetadataUUID</key>\n\
  <string>{}</string>\n\
  <key>MCMMetadataVersion</key>\n\
  <integer>1</integer>\n\
  <key>MCMMetadataInfo</key>\n\
  <dict>\n\
    <key>CFBundleShortVersionString</key>\n\
    <string>{}</string>\n\
    <key>CFBundleVersion</key>\n\
    <string>{}</string>\n\
  </dict>\n\
</dict>\n\
</plist>\n",
        content_class, bundle_id, uuid, short_ver, short_ver
    );
    fs::write(path, xml).map_err(|e| format!("Failed to write MCM metadata plist: {}", e))
}

/// Install an `.ipa` or `.app` bundle into `<root_dir>/private/var/containers/` and update
/// `<root_dir>/private/var/db/sweet_trustcache.bin` and `<root_dir>/private/var/db/sweet_installed_apps.txt`.
pub fn install_app_to_root(path: &Path, root_dir: &Path) -> Result<AppMetadata, String> {
    let meta = inspect_app_or_ipa(path)?;
    if meta.bundle_id.is_empty() {
        return Err("Cannot install app with empty CFBundleIdentifier".to_string());
    }

    let bundle_container = root_dir
        .join("private")
        .join("var")
        .join("containers")
        .join("Bundle")
        .join("Application")
        .join(&meta.bundle_uuid);
    let data_container = root_dir
        .join("private")
        .join("var")
        .join("containers")
        .join("Data")
        .join("Application")
        .join(&meta.data_uuid);

    fs::create_dir_all(&bundle_container)
        .map_err(|e| format!("Failed to create bundle container: {}", e))?;
    fs::create_dir_all(&data_container)
        .map_err(|e| format!("Failed to create data container: {}", e))?;

    for sub in &[
        "Documents",
        "Library/Caches",
        "Library/Preferences",
        "SystemData",
        "tmp",
    ] {
        let p = data_container.join(sub);
        fs::create_dir_all(&p).map_err(|e| format!("Failed to create data subdir {:?}: {}", p, e))?;
    }

    let app_folder_name = if meta.executable.is_empty() {
        "App.app".to_string()
    } else {
        format!("{}.app", meta.executable)
    };
    let dest_app_dir = bundle_container.join(&app_folder_name);
    fs::create_dir_all(&dest_app_dir)
        .map_err(|e| format!("Failed to create dest .app dir: {}", e))?;

    if path.is_dir() {
        let src_app = find_app_dir(path)?;
        copy_dir_recursive(&src_app, &dest_app_dir)?;
    } else {
        let raw_zip = fs::read(path).map_err(|e| format!("Failed to read IPA: {}", e))?;
        match read_stored_zip(&raw_zip) {
            Ok(entries) => {
                let mut app_prefix = String::new();
                for e in &entries {
                    let parts: Vec<&str> = e.name.trim_matches('/').split('/').collect();
                    if parts.len() >= 2 && parts[0] == "Payload" && parts[1].ends_with(".app") {
                        app_prefix = format!("{}/{}/", parts[0], parts[1]);
                        break;
                    } else if !parts.is_empty() && parts[0].ends_with(".app") {
                        app_prefix = format!("{}/", parts[0]);
                        break;
                    }
                }
                for e in &entries {
                    if let Some(rel) = e.name.strip_prefix(&app_prefix) {
                        if rel.is_empty() || rel.contains("..") {
                            continue;
                        }
                        let target = dest_app_dir.join(rel);
                        if let Some(p) = target.parent() {
                            let _ = fs::create_dir_all(p);
                        }
                        fs::write(&target, &e.data)
                            .map_err(|err| format!("Failed to extract {:?}: {}", target, err))?;
                    }
                }
            }
            Err(r) if r == "DEFLATE_COMPRESSED" => {
                if let Some(script) = find_sweet_ipa_py() {
                    let _ = Command::new("python")
                        .arg(&script)
                        .arg("install")
                        .arg(path)
                        .arg("--root")
                        .arg(root_dir)
                        .output();
                }
            }
            Err(e) => return Err(e),
        }
    }

    write_mcm_metadata_plist(
        &bundle_container.join(".com.apple.mobile_container_manager.metadata.plist"),
        &meta.bundle_id,
        &meta.bundle_uuid,
        1,
        &meta.version,
    )?;
    write_mcm_metadata_plist(
        &data_container.join(".com.apple.mobile_container_manager.metadata.plist"),
        &meta.bundle_id,
        &meta.data_uuid,
        2,
        &meta.version,
    )?;

    // Update <root_dir>/private/var/db/sweet_trustcache.bin
    let db_dir = root_dir.join("private").join("var").join("db");
    fs::create_dir_all(&db_dir).map_err(|e| format!("Failed to create var/db: {}", e))?;
    let tc_path = db_dir.join("sweet_trustcache.bin");

    let mut all_hashes: Vec<[u8; 20]> = Vec::new();
    if let Ok(existing_tc) = fs::read(&tc_path) {
        if let Ok(parsed) = parse_trustcache_v1(&existing_tc) {
            all_hashes.extend(parsed);
        }
    }
    if let Some(decoded) = hex_decode(&meta.cdhash_hex) {
        if decoded.len() >= 20 {
            let mut h = [0u8; 20];
            h.copy_from_slice(&decoded[..20]);
            all_hashes.push(h);
        }
    }
    let tc_blob = build_trustcache_v1(&all_hashes);
    fs::write(&tc_path, &tc_blob)
        .map_err(|e| format!("Failed to write sweet_trustcache.bin: {}", e))?;

    // Update <root_dir>/private/var/db/sweet_installed_apps.txt
    let reg_txt_path = db_dir.join("sweet_installed_apps.txt");
    let mut apps = list_installed_apps(root_dir);
    apps.retain(|a| a.bundle_id != meta.bundle_id);
    apps.push(meta.clone());

    let mut lines = String::new();
    for a in &apps {
        lines.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            a.bundle_id,
            a.display_name,
            a.version,
            a.executable,
            a.graphics_api,
            if a.is_arm64 { "1" } else { "0" },
            if a.is_encrypted { "1" } else { "0" },
            a.cdhash_hex,
            a.bundle_uuid,
            a.data_uuid,
        ));
    }
    fs::write(&reg_txt_path, lines)
        .map_err(|e| format!("Failed to write sweet_installed_apps.txt: {}", e))?;

    // Also mirror sweet_trustcache.bin to workspace root and sync HFSX image if unlocked
    if let Some(ws_dir) = root_dir.parent() {
        let _ = fs::write(ws_dir.join("sweet_trustcache.bin"), &tc_blob);
        let hfsx_img = ws_dir.join("ios16-root-20H392.img");
        if hfsx_img.is_file() {
            let py_code = format!(
                "import sys; sys.path.insert(0, 'tools'); import qemu_sweet; qemu_sweet.sync_hfsx_image(r'{}')",
                hfsx_img.display()
            );
            let _ = Command::new("python")
                .args(["-c", &py_code])
                .output();
        }
    }

    Ok(meta)
}

/// List all installed apps in `root_dir` from `private/var/db/sweet_installed_apps.txt`
/// and `private/var/containers/Bundle/Application/`.
pub fn list_installed_apps(root_dir: &Path) -> Vec<AppMetadata> {
    let mut apps: Vec<AppMetadata> = Vec::new();
    let reg_txt_path = root_dir
        .join("private")
        .join("var")
        .join("db")
        .join("sweet_installed_apps.txt");

    if let Ok(content) = fs::read_to_string(&reg_txt_path) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 10 {
                apps.push(AppMetadata {
                    bundle_id: parts[0].to_string(),
                    display_name: parts[1].to_string(),
                    version: parts[2].to_string(),
                    executable: parts[3].to_string(),
                    graphics_api: parts[4].to_string(),
                    is_arm64: parts[5] == "1" || parts[5].eq_ignore_ascii_case("true"),
                    is_encrypted: parts[6] == "1" || parts[6].eq_ignore_ascii_case("true"),
                    cdhash_hex: parts[7].to_string(),
                    bundle_uuid: parts[8].to_string(),
                    data_uuid: parts[9].to_string(),
                });
            }
        }
    }

    // Also scan <root_dir>/private/var/containers/Bundle/Application/ for bundles installed via Python
    let bundle_base = root_dir
        .join("private")
        .join("var")
        .join("containers")
        .join("Bundle")
        .join("Application");
    if let Ok(rd) = fs::read_dir(&bundle_base) {
        for entry in rd.flatten() {
            let cdir = entry.path();
            if !cdir.is_dir() {
                continue;
            }
            if let Ok(mut meta) = inspect_app_or_ipa(&cdir) {
                if let Some(uuid_os) = cdir.file_name().and_then(|s| s.to_str()) {
                    meta.bundle_uuid = uuid_os.to_string();
                }
                if !apps.iter().any(|a| a.bundle_id == meta.bundle_id) {
                    apps.push(meta);
                }
            }
        }
    }

    apps
}
