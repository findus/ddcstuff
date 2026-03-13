use anyhow::{bail, Context, Result};
use ddc_hi::{Ddc, Display};
use ddc_lib::monitor::{MonitorId, MonitorInfo, MonitorKind};

pub struct DdcMonitor {
    pub id: MonitorId,
    pub name: String,
    display: Display,
    cached_percent: u8,
    /// The DDC max value for VCP 0x10 (brightness). Usually 100.
    max_value: u16,
}

impl DdcMonitor {
    pub fn from_display(mut display: Display) -> Result<Self> {
        // Probe the monitor with a real DDC read. If this fails the monitor is
        // not DDC-capable (laptop eDP panel, ghost I2C bus from Thunderbolt dock, etc.)
        // and should be skipped entirely.
        let val = display
            .handle
            .get_vcp_feature(0x10)
            .context("DDC probe failed")?;
        let (current, max) = (val.value(), val.maximum().max(1));

        let name = build_monitor_name(&display.info);
        let id = build_monitor_id(&display.info);
        let cached_percent = ((current as u32 * 100) / max as u32).min(100) as u8;

        Ok(Self {
            id,
            name,
            display,
            cached_percent,
            max_value: max,
        })
    }

    /// Set brightness by percentage (0-100).
    /// Must be called from a blocking context (spawn_blocking or sync thread).
    pub fn set_percent(&mut self, percent: u8) -> Result<()> {
        let percent = percent.min(100);
        let raw = (percent as u32 * self.max_value as u32 / 100) as u16;
        self.display
            .handle
            .set_vcp_feature(0x10, raw)
            .context("DDC set brightness")?;
        self.cached_percent = percent;
        Ok(())
    }

    pub fn cached_percent(&self) -> u8 {
        self.cached_percent
    }

    pub fn info(&self) -> MonitorInfo {
        MonitorInfo {
            id: self.id.clone(),
            name: self.name.clone(),
            kind: MonitorKind::Ddc,
            brightness_percent: self.cached_percent,
        }
    }
}

fn build_monitor_name(info: &ddc_hi::DisplayInfo) -> String {
    match (&info.model_name, &info.manufacturer_id) {
        (Some(model), Some(mfr)) => format!("{mfr} {model}"),
        (Some(model), None) => model.clone(),
        (None, Some(mfr)) => mfr.clone(),
        (None, None) => format!("Monitor ({})", info.id),
    }
}

fn build_monitor_id(info: &ddc_hi::DisplayInfo) -> MonitorId {
    let mfr = info.manufacturer_id.as_deref().unwrap_or("UNK");
    let model = info
        .model_name
        .as_deref()
        .unwrap_or("")
        .replace(' ', "_");
    let serial = info.serial_number.as_deref().unwrap_or("");

    // Hash serial/id to a short stable hex string
    let serial_hash = if serial.is_empty() {
        let h: u32 = info.id.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
        format!("{h:08x}")
    } else {
        let h: u32 = serial.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
        format!("{h:08x}")
    };

    MonitorId(format!("ddc-{mfr}-{model}-{serial_hash}"))
}
