//! Zero-dependency PNG encoder (IHDR + zlib uncompressed IDAT + IEND with CRC-32 & Adler-32).

use std::fs;
use std::path::Path;

const PNG_SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Computes IEEE 802.3 CRC-32 used by PNG chunks.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

/// Computes RFC 1950 Adler-32 checksum used by zlib streams.
pub fn adler32(data: &[u8]) -> u32 {
    const MOD_ADLER: u32 = 65521;
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for chunk in data.chunks(5552) {
        for &byte in chunk {
            a += byte as u32;
            b += a;
        }
        a %= MOD_ADLER;
        b %= MOD_ADLER;
    }
    (b << 16) | a
}

fn write_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    let crc_start = out.len();
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(payload);
    let crc = crc32(&out[crc_start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// Encodes a 32-bit BGRA byte buffer (`width * height * 4` bytes) into a valid PNG byte vector.
pub fn encode_bgra_png_bytes(width: u32, height: u32, bgra: &[u8]) -> Result<Vec<u8>, String> {
    if width == 0 || height == 0 {
        return Err("PNG dimensions must be non-zero".to_string());
    }
    let expected_len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|px| px.checked_mul(4))
        .ok_or_else(|| "PNG dimensions overflow".to_string())?;

    if bgra.len() < expected_len {
        return Err(format!(
            "BGRA buffer too small: got {} bytes, expected at least {} bytes for {}x{}",
            bgra.len(),
            expected_len,
            width,
            height
        ));
    }

    let row_bytes = (width as usize) * 4;
    let raw_len = (height as usize) * (1 + row_bytes);
    let mut raw_scanlines = Vec::with_capacity(raw_len);

    for y in 0..(height as usize) {
        raw_scanlines.push(0u8); // Filter type 0 (None)
        let row_start = y * row_bytes;
        let row = &bgra[row_start..row_start + row_bytes];
        for px in row.chunks_exact(4) {
            let b = px[0];
            let g = px[1];
            let r = px[2];
            let a = if px[3] == 0 { 0xFF } else { px[3] };
            raw_scanlines.push(r);
            raw_scanlines.push(g);
            raw_scanlines.push(b);
            raw_scanlines.push(a);
        }
    }

    // Build RFC 1950 zlib stream with uncompressed deflate blocks (BTYPE=00)
    let num_blocks = (raw_scanlines.len() + 65534) / 65535;
    let mut zlib = Vec::with_capacity(2 + num_blocks.max(1) * 5 + raw_scanlines.len() + 4);
    // CMF = 0x78 (Deflate, 32K window), FLG = 0x01 (no dict, FCHECK so 0x7801 % 31 == 0)
    zlib.push(0x78);
    zlib.push(0x01);

    let chunks: Vec<&[u8]> = if raw_scanlines.is_empty() {
        vec![&[]]
    } else {
        raw_scanlines.chunks(65535).collect()
    };

    for (i, block) in chunks.iter().enumerate() {
        let is_final = i + 1 == chunks.len();
        zlib.push(if is_final { 0x01 } else { 0x00 });
        let len = block.len() as u16;
        let nlen = !len;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&nlen.to_le_bytes());
        zlib.extend_from_slice(block);
    }

    let adler = adler32(&raw_scanlines);
    zlib.extend_from_slice(&adler.to_be_bytes());

    let mut png = Vec::with_capacity(64 + zlib.len());
    png.extend_from_slice(&PNG_SIG);

    // IHDR: width(4), height(4), bit_depth(1)=8, color_type(1)=6 (RGBA),
    // compression(1)=0, filter(1)=0, interlace(1)=0
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8;
    ihdr[9] = 6;
    ihdr[10] = 0;
    ihdr[11] = 0;
    ihdr[12] = 0;
    write_chunk(&mut png, b"IHDR", &ihdr);
    write_chunk(&mut png, b"IDAT", &zlib);
    write_chunk(&mut png, b"IEND", &[]);

    Ok(png)
}

/// Writes a 32-bit BGRA buffer to disk at `path` as a standard RGBA PNG image.
pub fn save_bgra_png(path: &Path, width: u32, height: u32, bgra: &[u8]) -> Result<(), String> {
    let bytes = encode_bgra_png_bytes(width, height, bgra)?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = fs::create_dir_all(parent);
        }
    }
    fs::write(path, bytes).map_err(|e| format!("Failed to write PNG {}: {}", path.display(), e))
}
