use std::fs;
use std::io::Cursor;
use std::path::Path;

use anyhow::{Context, Result};
use image::DynamicImage;

// ── Byte-reading helpers ──────────────────────────────────────────

#[inline]
pub fn read_u32_le(data: &[u8], off: usize) -> Option<u32> {
    let s = data.get(off..off + 4)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

#[inline]
pub fn read_i32_le(data: &[u8], off: usize) -> Option<i32> {
    let s = data.get(off..off + 4)?;
    Some(i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

#[inline]
pub fn read_f32_le(data: &[u8], off: usize) -> Option<f32> {
    let s = data.get(off..off + 4)?;
    Some(f32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

// ── Oodle decompression ───────────────────────────────────────────

pub fn oodle_decompress(comp: &[u8], expected_size: usize) -> Option<Vec<u8>> {
    let mut out = vec![0u8; expected_size];
    let mut extractor = oozextract::Extractor::new();
    extractor.read_from_slice(comp, &mut out).ok()?;
    Some(out)
}

// ── Native DDS decoder ───────────────────────────────────────────

/// Decode a DDS file on disk to a `DynamicImage`.
#[allow(dead_code)]
pub fn decode_dds_file(path: &Path) -> Result<DynamicImage> {
    let data = fs::read(path)
        .with_context(|| format!("Failed to read DDS file {}", path.display()))?;
    decode_dds_bytes(&data)
        .with_context(|| format!("Failed to decode DDS {}", path.display()))
}

/// Decode DDS data in memory to a `DynamicImage`.
pub fn decode_dds_bytes(data: &[u8]) -> Result<DynamicImage> {
    let dds = image_dds::ddsfile::Dds::read(&mut Cursor::new(data))
        .context("Failed to parse DDS header")?;
    let rgba = image_dds::image_from_dds(&dds, 0)
        .context("Failed to decode DDS pixel data")?;
    Ok(DynamicImage::ImageRgba8(rgba))
}
