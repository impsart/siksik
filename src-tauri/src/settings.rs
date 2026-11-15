use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// "system" | "light" | "dark"
    pub theme: String,
    /// "lossless" | "smart"
    pub mode: String,
    /// JPEG quality used in smart mode (60-95)
    pub jpeg_quality: u8,
    /// oxipng preset 0-6
    pub png_level: u8,
    /// Use Zopfli for PNG (slower, a few % smaller)
    pub png_zopfli: bool,
    pub keep_metadata: bool,
    /// "same" | "folder"
    pub output_mode: String,
    pub output_dir: String,
    /// Overwrite originals / existing files instead of writing a copy
    pub overwrite: bool,
    /// Suffix for copies written next to the original, e.g. "-siksik"
    pub suffix: String,
    pub preserve_dates: bool,
    pub recursive: bool,
    pub watch_enabled: bool,
    pub watch_folders: Vec<String>,
    /// Optional PNG used as the window / taskbar icon at runtime
    pub custom_icon: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            mode: "lossless".into(),
            jpeg_quality: 85,
            png_level: 4,
            png_zopfli: false,
            keep_metadata: true,
            output_mode: "same".into(),
            output_dir: String::new(),
            overwrite: false,
            suffix: "-siksik".into(),
            preserve_dates: true,
            recursive: true,
            watch_enabled: true,
            watch_folders: vec![],
            custom_icon: String::new(),
        }
    }
}

pub fn data_dir() -> PathBuf {
    let d = dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("Siksik");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn path() -> PathBuf {
    data_dir().join("settings.json")
}

impl Settings {
    pub fn load() -> Self {
        std::fs::read_to_string(path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) -> anyhow::Result<()> {
        std::fs::write(path(), serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Stats {
    pub files_optimized: u64,
    pub files_failed: u64,
    pub files_skipped: u64,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub first_used: u64,
}

fn stats_path() -> PathBuf {
    data_dir().join("stats.json")
}

impl Stats {
    pub fn load() -> Self {
        let mut s: Stats = std::fs::read_to_string(stats_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if s.first_used == 0 {
            s.first_used = now();
        }
        s
    }
    pub fn save(&self) {
        if let Ok(j) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(stats_path(), j);
        }
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
