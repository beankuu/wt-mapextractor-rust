use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use image::DynamicImage;
use libloading::{Library, Symbol};

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

// ── Oodle decompression (DLL loaded once) ─────────────────────────

type OodleFn = unsafe extern "C" fn(
    comp_buf: *const u8,
    comp_buf_size: i64,
    raw_buf: *mut u8,
    raw_len: i64,
    fuzz_safe: i32,
    check_crc: i32,
    verbosity: i32,
    dec_buf_base: *mut u8,
    dec_buf_size: i64,
    fp_callback: i64,
    callback_user_data: i64,
    decoder_memory: *mut u8,
    decoder_memory_size: i64,
    thread_phase: i32,
) -> i64;

static OODLE_LIB: OnceLock<Option<Library>> = OnceLock::new();
/// Optional config-provided DLL path, set once before any decompression.
static OODLE_DLL_PATH: OnceLock<Option<String>> = OnceLock::new();

/// Register the oo2core DLL path from config before first use.
pub fn set_oodle_dll_path(path: Option<&std::path::Path>) {
    let _ = OODLE_DLL_PATH.get_or_init(|| path.map(|p| p.to_string_lossy().into_owned()));
}

fn load_oodle_lib() -> Option<&'static Library> {
    OODLE_LIB
        .get_or_init(|| {
            // Config-registered path has highest priority
            let cfg_path = OODLE_DLL_PATH.get().and_then(|o| o.clone());
            let candidates: Vec<Option<String>> = vec![
                cfg_path,
                std::env::var("OODLE_DLL").ok(),
                Some("src/oo2core_9_win64.dll".to_string()),
                Some("oo2core_9_win64.dll".to_string()),
            ];
            for c in candidates.into_iter().flatten() {
                let p = Path::new(&c);
                if p.exists() {
                    if let Ok(lib) = unsafe { Library::new(p) } {
                        return Some(lib);
                    }
                }
            }
            None
        })
        .as_ref()
}

pub fn oodle_decompress(comp: &[u8], expected_size: usize) -> Option<Vec<u8>> {
    let lib = load_oodle_lib()?;
    let func: Symbol<OodleFn> = unsafe { lib.get(b"OodleLZ_Decompress") }.ok()?;
    let mut out = vec![0u8; expected_size];
    let res = unsafe {
        func(
            comp.as_ptr(),
            comp.len() as i64,
            out.as_mut_ptr(),
            expected_size as i64,
            0,
            0,
            0,
            std::ptr::null_mut(),
            0,
            0,
            0,
            std::ptr::null_mut(),
            0,
            3,
        )
    };
    if res == expected_size as i64 {
        Some(out)
    } else {
        None
    }
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
