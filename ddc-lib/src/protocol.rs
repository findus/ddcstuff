use serde::{Deserialize, Serialize};

use crate::monitor::{MonitorId, MonitorInfo, MonitorStatus};

/// Which monitors a command targets.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Target {
    All,
    ById { id: MonitorId },
    ByIndex { index: usize },
}

/// The brightness adjustment to apply.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrightnessOp {
    /// Set to absolute percentage (0-100).
    Set { percent: u8 },
    /// Increase by N percentage points.
    Increase { percent: u8 },
    /// Decrease by N percentage points.
    Decrease { percent: u8 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    SetBrightness {
        target: Target,
        op: BrightnessOp,
        /// Request animated fade transition.
        fade: bool,
    },
    GetBrightness {
        target: Target,
    },
    ListMonitors,
    Ping,
    /// Force an immediate monitor rescan.
    Rescan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    Ok,
    Brightness { monitors: Vec<MonitorStatus> },
    Monitors { list: Vec<MonitorInfo> },
    Pong { version: String },
    Error { message: String },
}
