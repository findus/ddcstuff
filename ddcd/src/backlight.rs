use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ddc_lib::monitor::{MonitorId, MonitorInfo, MonitorKind, MonitorStatus};
use tracing::{debug, warn};

pub struct Backlight {
    pub id: MonitorId,
    pub name: String,
    path: PathBuf,
    max_brightness: u32,
    cached_percent: u8,
}

impl Backlight {
    /// Auto-detect the best backlight device in /sys/class/backlight/.
    /// Priority: type=="raw" > type=="platform" > type=="firmware"
    pub fn detect(override_path: &str) -> Option<Self> {
        if !override_path.is_empty() {
            let path = PathBuf::from(override_path);
            return Self::from_path(&path).ok();
        }

        let backlight_dir = Path::new("/sys/class/backlight");
        if !backlight_dir.exists() {
            return None;
        }

        let entries = std::fs::read_dir(backlight_dir).ok()?;

        let mut best: Option<(u8, PathBuf)> = None; // (priority, path)
        for entry in entries.flatten() {
            let path = entry.path();
            let priority = backlight_type_priority(&path);
            if let Some((current_priority, _)) = &best {
                if priority > *current_priority {
                    best = Some((priority, path));
                }
            } else if priority > 0 {
                best = Some((priority, path));
            }
        }

        let (_, path) = best?;
        match Self::from_path(&path) {
            Ok(bl) => {
                debug!("detected backlight: {}", bl.name);
                Some(bl)
            }
            Err(e) => {
                warn!("failed to open backlight at {}: {e}", path.display());
                None
            }
        }
    }

    fn from_path(path: &Path) -> Result<Self> {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("backlight")
            .to_string();

        let max_brightness: u32 = std::fs::read_to_string(path.join("max_brightness"))
            .context("read max_brightness")?
            .trim()
            .parse()
            .context("parse max_brightness")?;

        anyhow::ensure!(max_brightness > 0, "max_brightness is 0");

        let mut bl = Self {
            id: MonitorId(format!("backlight-{name}")),
            name: name.clone(),
            path: path.to_owned(),
            max_brightness,
            cached_percent: 0,
        };
        // Populate the initial cache
        bl.cached_percent = bl.read_percent().unwrap_or(0);
        Ok(bl)
    }

    fn read_percent(&self) -> Result<u8> {
        let raw: u32 = std::fs::read_to_string(self.path.join("brightness"))
            .context("read brightness")?
            .trim()
            .parse()
            .context("parse brightness")?;
        Ok(((raw * 100) / self.max_brightness).min(100) as u8)
    }

    pub fn get_percent(&mut self) -> Result<u8> {
        let p = self.read_percent()?;
        self.cached_percent = p;
        Ok(p)
    }

    pub fn set_percent(&mut self, percent: u8) -> Result<()> {
        let percent = percent.min(100);
        let raw = (percent as u32 * self.max_brightness) / 100;
        std::fs::write(self.path.join("brightness"), raw.to_string())
            .with_context(|| {
                format!(
                    "write brightness to {}; make sure you are in the 'video' group \
                     or have a udev rule granting write access",
                    self.path.display()
                )
            })?;
        self.cached_percent = percent;
        Ok(())
    }

    pub fn info(&self) -> MonitorInfo {
        MonitorInfo {
            id: self.id.clone(),
            name: self.name.clone(),
            kind: MonitorKind::Backlight,
            brightness_percent: self.cached_percent,
        }
    }

    pub fn status(&self) -> MonitorStatus {
        MonitorStatus {
            id: self.id.clone(),
            name: self.name.clone(),
            brightness_percent: self.cached_percent,
        }
    }
}

/// Returns a priority score for a backlight type: raw=3, platform=2, firmware=1, unknown=0.
fn backlight_type_priority(path: &Path) -> u8 {
    let type_path = path.join("type");
    let bl_type = std::fs::read_to_string(type_path)
        .map(|s| s.trim().to_lowercase())
        .unwrap_or_default();
    match bl_type.as_str() {
        "raw" => 3,
        "platform" => 2,
        "firmware" => 1,
        _ => 0,
    }
}
