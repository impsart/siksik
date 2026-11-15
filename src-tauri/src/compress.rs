use crate::jpeg;
use crate::settings::Settings;
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use walkdir::WalkDir;

pub const EXTENSIONS: &[&str] = &["jpg", "jpeg", "jfif", "png", "webp"];

/// Files Siksik itself just wrote – the folder watcher ignores these so it
/// doesn't loop on its own output.
static RECENT: LazyLock<Mutex<HashMap<PathBuf, Instant>>> = LazyLock::new(Default::default);

pub fn recently_written(p: &Path) -> bool {
    let mut m = RECENT.lock().unwrap();
    m.retain(|_, t| t.elapsed() < Duration::from_secs(10));
    m.contains_key(p)
}

fn mark_written(p: &Path) {
    RECENT.lock().unwrap().insert(p.to_path_buf(), Instant::now());
}

pub fn is_supported(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// True for copies Siksik produced (e.g. `photo-siksik.jpg`).
pub fn is_own_copy(p: &Path, s: &Settings) -> bool {
    !s.suffix.is_empty()
        && p.file_stem()
            .and_then(|x| x.to_str())
            .map(|x| x.ends_with(&s.suffix))
            .unwrap_or(false)
}

#[derive(Clone, Debug)]
pub struct Job {
    pub file: PathBuf,
    /// Folder the user dropped – used to mirror sub-folders in the output dir.
    pub base: Option<PathBuf>,
}

pub fn collect(paths: &[String], s: &Settings) -> Vec<Job> {
    let mut jobs = vec![];
    for p in paths {
        let p = PathBuf::from(p);
        if p.is_dir() {
            let depth = if s.recursive { usize::MAX } else { 1 };
            for e in WalkDir::new(&p).max_depth(depth).into_iter().flatten() {
                let f = e.path();
                if e.file_type().is_file() && is_supported(f) && !is_own_copy(f, s) {
                    jobs.push(Job { file: f.to_path_buf(), base: Some(p.clone()) });
                }
            }
        } else if p.is_file() && is_supported(&p) {
            jobs.push(Job { file: p, base: None });
        }
    }
    jobs.sort_by(|a, b| a.file.cmp(&b.file));
    jobs.dedup_by(|a, b| a.file == b.file);
    jobs
}

#[derive(Clone, Debug, Serialize)]
pub struct FileResult {
    pub path: String,
    pub output: String,
    pub name: String,
    /// "optimized" | "skipped" | "failed"
    pub status: String,
    pub before: u64,
    pub after: u64,
    pub message: String,
    pub source: String,
}

fn unique(p: PathBuf) -> PathBuf {
    if !p.exists() {
        return p;
    }
    let stem = p.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let ext = p.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
    let dir = p.parent().map(Path::to_path_buf).unwrap_or_default();
    (1..)
        .map(|i| dir.join(format!("{stem} ({i}).{ext}")))
        .find(|c| !c.exists())
        .unwrap()
}

pub fn target_path(job: &Job, s: &Settings) -> PathBuf {
    let f = &job.file;
    let name = f.file_name().unwrap_or_default();
    if s.output_mode == "folder" && !s.output_dir.is_empty() {
        let rel_dir = job
            .base
            .as_ref()
            .and_then(|b| f.parent()?.strip_prefix(b).ok().map(|r| {
                PathBuf::from(b.file_name().unwrap_or_default()).join(r)
            }))
            .unwrap_or_default();
        let t = PathBuf::from(&s.output_dir).join(rel_dir).join(name);
        if s.overwrite || t == *f { t } else { unique(t) }
    } else if s.overwrite {
        f.clone()
    } else {
        let stem = f.file_stem().unwrap_or_default().to_string_lossy();
        let ext = f.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
        unique(f.with_file_name(format!("{stem}{}.{ext}", s.suffix)))
    }
}

fn optimize_png(data: &[u8], s: &Settings) -> Result<Vec<u8>> {
    let mut o = oxipng::Options::from_preset(s.png_level.min(6));
    o.strip = if s.keep_metadata {
        oxipng::StripChunks::None
    } else {
        oxipng::StripChunks::Safe
    };
    if s.png_zopfli {
        let mut z = oxipng::ZopfliOptions::default();
        z.iteration_count = NonZeroU64::new(15).unwrap();
        o.deflater = oxipng::Deflater::Zopfli(z);
    }
    oxipng::optimize_from_memory(data, &o).map_err(|e| anyhow!("PNG: {e}"))
}

fn optimize_jpeg(data: &[u8], s: &Settings) -> Result<Vec<u8>> {
    let lossless = jpeg::lossless(data, s.keep_metadata)?;
    if s.mode == "smart" {
        if let Ok(lossy) = jpeg::smart(data, s.jpeg_quality as f32, s.keep_metadata) {
            if lossy.len() < lossless.len() {
                return Ok(lossy);
            }
        }
    }
    Ok(lossless)
}

fn webp_chunks(data: &[u8]) -> Vec<[u8; 4]> {
    let mut out = vec![];
    let mut i = 12;
    while i + 8 <= data.len() {
        let id = [data[i], data[i + 1], data[i + 2], data[i + 3]];
        let len = u32::from_le_bytes([data[i + 4], data[i + 5], data[i + 6], data[i + 7]]) as usize;
        out.push(id);
        i += 8 + len + (len & 1);
    }
    out
}

fn optimize_webp(data: &[u8], s: &Settings) -> Result<Option<Vec<u8>>> {
    let chunks = webp_chunks(data);
    if chunks.iter().any(|c| c == b"ANIM") {
        return Ok(None); // animated – leave alone
    }
    if !chunks.iter().any(|c| c == b"VP8L") {
        return Ok(None); // lossy WebP – re-encoding would lose quality
    }
    if s.keep_metadata && chunks.iter().any(|c| c == b"ICCP" || c == b"EXIF" || c == b"XMP ") {
        return Ok(None);
    }
    let img = webp::Decoder::new(data).decode().ok_or_else(|| anyhow!("Unreadable WebP"))?;
    let enc = if img.is_alpha() {
        webp::Encoder::from_rgba(&img, img.width(), img.height())
    } else {
        webp::Encoder::from_rgb(&img, img.width(), img.height())
    };
    let mut cfg = webp::WebPConfig::new().map_err(|_| anyhow!("WebP config"))?;
    cfg.lossless = 1;
    cfg.exact = 1; // keep RGB under transparent pixels: truly lossless
    cfg.method = 6;
    cfg.quality = 100.0;
    let out = enc.encode_advanced(&cfg).map_err(|e| anyhow!("WebP: {e:?}"))?;
    Ok(Some(out.to_vec()))
}

fn write_atomic(target: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(d) = target.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = target.with_extension("siksik.tmp");
    std::fs::write(&tmp, bytes)?;
    mark_written(target);
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

pub fn process(job: &Job, s: &Settings, source: &str) -> FileResult {
    let f = &job.file;
    let mut r = FileResult {
        path: f.to_string_lossy().into(),
        output: String::new(),
        name: f.file_name().unwrap_or_default().to_string_lossy().into(),
        status: "failed".into(),
        before: 0,
        after: 0,
        message: String::new(),
        source: source.into(),
    };
    match run(job, s, &mut r) {
        Ok(()) => {}
        Err(e) => {
            r.status = "failed".into();
            r.message = e.to_string();
        }
    }
    r
}

fn run(job: &Job, s: &Settings, r: &mut FileResult) -> Result<()> {
    let f = &job.file;
    let data = std::fs::read(f)?;
    let meta = std::fs::metadata(f)?;
    r.before = data.len() as u64;
    let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();

    let optimized: Option<Vec<u8>> = match ext.as_str() {
        "png" => Some(optimize_png(&data, s)?),
        "jpg" | "jpeg" | "jfif" => Some(optimize_jpeg(&data, s)?),
        "webp" => optimize_webp(&data, s)?,
        _ => return Err(anyhow!("Unsupported format")),
    };

    let target = target_path(job, s);
    let smaller = optimized.as_ref().filter(|o| (o.len() as u64) < r.before);

    match smaller {
        Some(bytes) => {
            write_atomic(&target, bytes)?;
            r.after = bytes.len() as u64;
            r.status = "optimized".into();
        }
        None => {
            r.after = r.before;
            r.status = "skipped".into();
            r.message = if optimized.is_none() {
                "Left as-is (lossy, animated or has metadata)".into()
            } else {
                "Already optimized".into()
            };
            // When exporting to a separate folder, still deliver a copy so the
            // output folder is complete.
            if target != *f && s.output_mode == "folder" {
                write_atomic(&target, &data)?;
            } else {
                r.output = r.path.clone();
                return Ok(());
            }
        }
    }
    r.output = target.to_string_lossy().into();
    if s.preserve_dates {
        let mtime = filetime::FileTime::from_last_modification_time(&meta);
        let _ = filetime::set_file_mtime(&target, mtime);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Run with: SIKSIK_TEST_DIR=/path cargo test -- --nocapture
    #[test]
    fn compress_dir() {
        let Ok(dir) = std::env::var("SIKSIK_TEST_DIR") else { return };
        for mode in ["lossless", "smart"] {
            let s = Settings {
                mode: mode.into(),
                output_mode: "folder".into(),
                output_dir: format!("{dir}-out-{mode}"),
                overwrite: true,
                ..Default::default()
            };
            for job in collect(&[dir.clone()], &s) {
                let r = process(&job, &s, "test");
                println!("[{mode}] {:<28} {:>9} -> {:>9} {} {}", r.name, r.before, r.after, r.status, r.message);
                if r.name != "broken.jpg" { assert_ne!(r.status, "failed", "{}", r.message); }
            }
        }
    }
}
