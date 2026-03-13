use serde::{Deserialize, Serialize};

/// Stable, human-readable monitor identifier.
/// Examples:
///   "ddc-DEL-U2722D-a1b2c3" for DDC/CI monitors (manufacturer-model-serial_hash)
///   "backlight-intel_backlight" for sysfs backlight panels
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MonitorId(pub String);

impl std::fmt::Display for MonitorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorInfo {
    pub id: MonitorId,
    pub name: String,
    pub kind: MonitorKind,
    /// Brightness 0-100 (cached)
    pub brightness_percent: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorKind {
    Ddc,
    Backlight,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorStatus {
    pub id: MonitorId,
    pub name: String,
    pub brightness_percent: u8,
}
