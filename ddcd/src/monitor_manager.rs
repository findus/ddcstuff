use std::sync::Arc;

use anyhow::{bail, Result};
use ddc_hi::Display;
use ddc_lib::{
    monitor::{MonitorId, MonitorInfo, MonitorStatus},
    protocol::{BrightnessOp, Target},
};
use tracing::info;

use crate::{
    backlight::Backlight,
    config::Config,
    ddc_monitor::DdcMonitor,
};

pub struct MonitorManager {
    pub ddc_monitors: Vec<DdcMonitor>,
    pub backlight: Option<Backlight>,
    config: Arc<Config>,
}

impl MonitorManager {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            ddc_monitors: Vec::new(),
            backlight: None,
            config,
        }
    }

    /// Scan for all monitors. Replaces the current cache.
    /// This is slow (I2C enumeration) and should not be called on every brightness change.
    pub fn scan(&mut self) {
        info!("scanning monitors...");

        // Enumerate DDC/CI monitors
        let displays = Display::enumerate();
        let mut new_ddc: Vec<DdcMonitor> = Vec::new();
        for display in displays {
            let raw_id = display.info.id.clone();
            match DdcMonitor::from_display(display) {
                Ok(m) => {
                    info!("found DDC monitor: {} ({})", m.name, m.id);
                    new_ddc.push(m);
                }
                Err(e) => {
                    tracing::debug!("skipping display {raw_id}: {e}");
                }
            }
        }
        self.ddc_monitors = new_ddc;

        // Detect backlight
        self.backlight = Backlight::detect(&self.config.backlight.path);
        if let Some(bl) = &self.backlight {
            info!("found backlight: {} ({})", bl.name, bl.id);
        }

        info!(
            "scan complete: {} DDC monitor(s), {} backlight",
            self.ddc_monitors.len(),
            if self.backlight.is_some() { "1" } else { "0" }
        );
    }

    /// Set brightness on a specific monitor by ID (used by the fade controller).
    /// Operates synchronously — call from spawn_blocking or a sync context.
    pub fn set_brightness_direct(&mut self, id: &MonitorId, percent: u8) -> Result<()> {
        // Check DDC monitors
        for m in &mut self.ddc_monitors {
            if &m.id == id {
                return m.set_percent(percent);
            }
        }
        // Check backlight
        if let Some(bl) = &mut self.backlight {
            if &bl.id == id {
                return bl.set_percent(percent);
            }
        }
        bail!("monitor not found: {id}");
    }

    /// Apply a BrightnessOp to a single monitor (by ID), returns updated status.
    pub fn apply_to_monitor(&mut self, id: &MonitorId, op: &BrightnessOp) -> Result<MonitorStatus> {
        // Find current value
        let current = self.get_current_percent(id)?;
        let target = apply_op(op, current);

        self.set_brightness_direct(id, target)?;

        Ok(MonitorStatus {
            id: id.clone(),
            name: self.monitor_name(id),
            brightness_percent: target,
        })
    }

    /// Apply BrightnessOp to all monitors, returns status of each.
    pub fn apply_to_all(&mut self, op: &BrightnessOp) -> Vec<Result<MonitorStatus>> {
        let ids: Vec<MonitorId> = self.all_ids();
        ids.iter()
            .map(|id| self.apply_to_monitor(id, op))
            .collect()
    }

    pub fn get_status(&mut self, target: &Target) -> Result<Vec<MonitorStatus>> {
        match target {
            Target::All => {
                let ids = self.all_ids();
                ids.iter()
                    .map(|id| {
                        let percent = self.get_current_percent(id)?;
                        Ok(MonitorStatus {
                            id: id.clone(),
                            name: self.monitor_name(id),
                            brightness_percent: percent,
                        })
                    })
                    .collect()
            }
            Target::ById { id } => Ok(vec![MonitorStatus {
                id: id.clone(),
                name: self.monitor_name(id),
                brightness_percent: self.get_current_percent(id)?,
            }]),
            Target::ByIndex { index } => {
                let ids = self.all_ids();
                let id = ids
                    .get(*index)
                    .ok_or_else(|| anyhow::anyhow!("monitor index {index} out of range"))?
                    .clone();
                Ok(vec![MonitorStatus {
                    id: id.clone(),
                    name: self.monitor_name(&id),
                    brightness_percent: self.get_current_percent(&id)?,
                }])
            }
        }
    }

    pub fn list_monitors(&self) -> Vec<MonitorInfo> {
        let mut list: Vec<MonitorInfo> = self.ddc_monitors.iter().map(|m| m.info()).collect();
        if let Some(bl) = &self.backlight {
            list.push(bl.info());
        }
        list
    }

    /// Resolve a target to a list of monitor IDs.
    pub fn resolve_target(&self, target: &Target) -> Result<Vec<MonitorId>> {
        match target {
            Target::All => Ok(self.all_ids()),
            Target::ById { id } => {
                if self.find_ddc(id).is_some()
                    || self.backlight.as_ref().map(|b| &b.id == id).unwrap_or(false)
                {
                    Ok(vec![id.clone()])
                } else {
                    bail!("monitor not found: {id}");
                }
            }
            Target::ByIndex { index } => {
                let ids = self.all_ids();
                let id = ids
                    .get(*index)
                    .ok_or_else(|| anyhow::anyhow!("monitor index {index} out of range"))?;
                Ok(vec![id.clone()])
            }
        }
    }

    fn all_ids(&self) -> Vec<MonitorId> {
        let mut ids: Vec<MonitorId> = self.ddc_monitors.iter().map(|m| m.id.clone()).collect();
        if let Some(bl) = &self.backlight {
            ids.push(bl.id.clone());
        }
        ids
    }

    fn get_current_percent(&mut self, id: &MonitorId) -> Result<u8> {
        for m in &mut self.ddc_monitors {
            if &m.id == id {
                // Use cached value for speed; the daemon owns all writes
                return Ok(m.cached_percent());
            }
        }
        if let Some(bl) = &mut self.backlight {
            if &bl.id == id {
                return bl.get_percent();
            }
        }
        bail!("monitor not found: {id}");
    }

    fn monitor_name(&self, id: &MonitorId) -> String {
        for m in &self.ddc_monitors {
            if &m.id == id {
                return m.name.clone();
            }
        }
        if let Some(bl) = &self.backlight {
            if &bl.id == id {
                return bl.name.clone();
            }
        }
        id.0.clone()
    }

    fn find_ddc(&self, id: &MonitorId) -> Option<&DdcMonitor> {
        self.ddc_monitors.iter().find(|m| &m.id == id)
    }
}

pub fn apply_op(op: &BrightnessOp, current: u8) -> u8 {
    match op {
        BrightnessOp::Set { percent } => (*percent).min(100),
        BrightnessOp::Increase { percent } => current.saturating_add(*percent).min(100),
        BrightnessOp::Decrease { percent } => current.saturating_sub(*percent),
    }
}
