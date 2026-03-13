use ddc_lib::monitor::MonitorStatus;
use serde::Serialize;

#[derive(Serialize)]
pub struct WaybarOutput {
    pub text: String,
    pub tooltip: String,
    pub class: String,
    pub percentage: u8,
}

pub fn format_waybar(monitors: &[MonitorStatus]) -> WaybarOutput {
    if monitors.is_empty() {
        return WaybarOutput {
            text: "N/A".to_string(),
            tooltip: "No monitors found".to_string(),
            class: "brightness-error".to_string(),
            percentage: 0,
        };
    }

    let avg = monitors.iter().map(|m| m.brightness_percent as u32).sum::<u32>()
        / monitors.len() as u32;

    let tooltip = monitors
        .iter()
        .map(|m| format!("{}: {}%", m.name, m.brightness_percent))
        .collect::<Vec<_>>()
        .join("\n");

    let class = if avg < 30 {
        "brightness-low"
    } else {
        "brightness"
    }
    .to_string();

    WaybarOutput {
        text: format!("{avg}%"),
        tooltip,
        class,
        percentage: avg.min(100) as u8,
    }
}
