//! JPEG optimisation built on MozJPEG.
//!
//! * `lossless` – jpegtran-style transcoding. The quantised DCT coefficients
//!   are copied bit-for-bit; only the entropy coding is rebuilt (optimised
//!   Huffman tables + MozJPEG's progressive scan search). Decoded pixels are
//!   identical to the original.
//! * `smart` – JPEGmini-style "visually lossless" re-encode with MozJPEG's
//!   trellis quantisation. Only used when the user enables Smart mode.

use anyhow::{anyhow, Result};
use mozjpeg_sys::*;
use std::os::raw::{c_int, c_ulong, c_void};
use std::{mem, panic, ptr, slice};

extern "C" {
    fn free(p: *mut c_void);
}

const APP0: c_int = 0xE0;
const APP1: c_int = 0xE1;
const APP2: c_int = 0xE2;
const APP14: c_int = 0xEE;
const COM: c_int = 0xFE;

unsafe extern "C-unwind" fn on_error(cinfo: &mut jpeg_common_struct) {
    let buf = [0u8; 80];
    if let Some(fmt) = (*cinfo.err).format_message {
        fmt(cinfo, &buf);
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let msg = String::from_utf8_lossy(&buf[..end]).to_string();
    // Unwinds back through libjpeg into catch_unwind below (mozjpeg-sys
    // is built with the `unwinding` feature for exactly this purpose).
    std::panic::resume_unwind(Box::new(format!("JPEG error: {msg}")));
}

unsafe extern "C-unwind" fn silent(_c: &mut jpeg_common_struct, _lvl: c_int) {}

fn panic_msg(e: Box<dyn std::any::Any + Send>) -> anyhow::Error {
    if let Some(s) = e.downcast_ref::<String>() {
        anyhow!(s.clone())
    } else if let Some(s) = e.downcast_ref::<&str>() {
        anyhow!(s.to_string())
    } else {
        anyhow!("JPEG decoder error")
    }
}

/// Lossless re-encode. Returns the new bytes (may be larger than input – the
/// caller decides whether to keep them).
pub fn lossless(data: &[u8], keep_metadata: bool) -> Result<Vec<u8>> {
    let data = data.to_vec();
    panic::catch_unwind(move || unsafe { lossless_inner(&data, keep_metadata) })
        .map_err(panic_msg)?
}

struct Guard {
    src: *mut jpeg_decompress_struct,
    dst: *mut jpeg_compress_struct,
    out: *mut u8,
}
impl Drop for Guard {
    fn drop(&mut self) {
        unsafe {
            if !self.dst.is_null() {
                jpeg_destroy_compress(&mut *self.dst);
            }
            if !self.src.is_null() {
                jpeg_destroy_decompress(&mut *self.src);
            }
            if !self.out.is_null() {
                free(self.out as *mut c_void);
            }
        }
    }
}

unsafe fn lossless_inner(data: &[u8], keep_metadata: bool) -> Result<Vec<u8>> {
    let mut err: Box<jpeg_error_mgr> = Box::new(mem::zeroed());
    jpeg_std_error(&mut err);
    err.error_exit = Some(on_error);
    err.emit_message = Some(silent);

    let mut src: Box<jpeg_decompress_struct> = Box::new(mem::zeroed());
    let mut dst: Box<jpeg_compress_struct> = Box::new(mem::zeroed());
    let mut outbuf: *mut u8 = ptr::null_mut();
    let mut outsize: c_ulong = 0;

    src.common.err = &mut *err;
    jpeg_create_decompress(&mut *src);
    let mut guard = Guard { src: &mut *src, dst: ptr::null_mut(), out: ptr::null_mut() };

    jpeg_mem_src(&mut src, data.as_ptr(), data.len() as c_ulong);
    // Always keep ICC profiles and EXIF (we filter EXIF later if stripping).
    jpeg_save_markers(&mut src, APP2, 0xFFFF);
    jpeg_save_markers(&mut src, APP1, 0xFFFF);
    if keep_metadata {
        jpeg_save_markers(&mut src, COM, 0xFFFF);
        for m in 0..16 {
            if APP0 + m != APP1 && APP0 + m != APP2 {
                jpeg_save_markers(&mut src, APP0 + m, 0xFFFF);
            }
        }
    }
    jpeg_read_header(&mut src, 1);
    let coefs = jpeg_read_coefficients(&mut src);

    dst.common.err = &mut *err;
    jpeg_create_compress(&mut *dst);
    guard.dst = &mut *dst;
    jpeg_copy_critical_parameters(&src, &mut dst);
    dst.optimize_coding = 1;
    jpeg_simple_progression(&mut dst);

    jpeg_mem_dest(&mut dst, &mut outbuf, &mut outsize);
    jpeg_write_coefficients(&mut dst, coefs);

    // Copy saved markers (same rules as jpegtran's jcopy_markers_execute).
    let mut m = src.marker_list;
    while !m.is_null() {
        let mk = &*m;
        let body = slice::from_raw_parts(mk.data, mk.data_length as usize);
        let code = mk.marker as c_int;
        let skip = (dst.write_JFIF_header != 0 && code == APP0 && body.starts_with(b"JFIF\0"))
            || (dst.write_Adobe_marker != 0 && code == APP14 && body.starts_with(b"Adobe"))
            || (!keep_metadata && code == APP1 && !exif_needs_keeping(body))
            || (!keep_metadata && code == APP2 && !body.starts_with(b"ICC_PROFILE\0"));
        if !skip {
            jpeg_write_marker(&mut dst, code, mk.data, mk.data_length);
        }
        m = mk.next;
    }

    jpeg_finish_compress(&mut dst);
    guard.out = outbuf;
    let out = slice::from_raw_parts(outbuf, outsize as usize).to_vec();
    jpeg_finish_decompress(&mut src);
    drop(guard);
    drop(err);
    Ok(out)
}

/// When stripping metadata we still keep EXIF if it carries a non-default
/// orientation – otherwise phone photos would suddenly display sideways.
fn exif_needs_keeping(app1: &[u8]) -> bool {
    if !app1.starts_with(b"Exif\0\0") {
        return false; // XMP etc.
    }
    exif_orientation(&app1[6..]).map(|o| o != 1).unwrap_or(false)
}

fn exif_orientation(tiff: &[u8]) -> Option<u16> {
    if tiff.len() < 8 {
        return None;
    }
    let le = match &tiff[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |o: usize| -> Option<u16> {
        let b = tiff.get(o..o + 2)?;
        Some(if le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) })
    };
    let u32_at = |o: usize| -> Option<u32> {
        let b = tiff.get(o..o + 4)?;
        let a = [b[0], b[1], b[2], b[3]];
        Some(if le { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) })
    };
    let ifd = u32_at(4)? as usize;
    let n = u16_at(ifd)? as usize;
    for i in 0..n {
        let e = ifd + 2 + i * 12;
        if u16_at(e)? == 0x0112 {
            return u16_at(e + 8);
        }
    }
    None
}

/// Visually-lossless re-encode (Smart mode).
pub fn smart(data: &[u8], quality: f32, keep_metadata: bool) -> Result<Vec<u8>> {
    let data = data.to_vec();
    panic::catch_unwind(move || -> Result<Vec<u8>> {
        let d = mozjpeg::Decompress::with_markers(mozjpeg::ALL_MARKERS).from_mem(&data)?;
        if matches!(d.color_space(), mozjpeg::ColorSpace::JCS_CMYK | mozjpeg::ColorSpace::JCS_YCCK) {
            return Err(anyhow!("CMYK JPEG – smart mode not supported"));
        }
        let markers: Vec<(mozjpeg::Marker, Vec<u8>)> =
            d.markers().map(|m| (m.marker, m.data.to_vec())).collect();
        let mut img = d.rgb()?;
        let (w, h) = (img.width(), img.height());
        let pixels: Vec<u8> = img.read_scanlines::<u8>()?;
        img.finish()?;

        let mut c = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
        c.set_size(w, h);
        c.set_quality(quality);
        c.set_optimize_coding(true);
        c.set_optimize_scans(true);
        c.set_progressive_mode();
        let mut c = c.start_compress(Vec::new())?;
        for (marker, body) in &markers {
            let keep = match marker {
                mozjpeg::Marker::APP(0) => false, // JFIF written automatically
                mozjpeg::Marker::APP(1) => keep_metadata || exif_needs_keeping(body),
                mozjpeg::Marker::APP(2) => keep_metadata || body.starts_with(b"ICC_PROFILE\0"),
                mozjpeg::Marker::APP(14) => false,
                _ => keep_metadata,
            };
            if keep {
                c.write_marker(*marker, body);
            }
        }
        c.write_scanlines(&pixels)?;
        Ok(c.finish()?)
    })
    .map_err(panic_msg)?
}
