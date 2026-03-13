use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use ddc_lib::monitor::MonitorId;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::debug;

use crate::monitor_manager::MonitorManager;

pub struct FadeController {
    tasks: HashMap<MonitorId, JoinHandle<()>>,
}

impl FadeController {
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
        }
    }

    /// Start a fade from `from_percent` to `to_percent` for a single monitor.
    /// Cancels any in-progress fade for that monitor.
    pub fn start_fade(
        &mut self,
        monitor_id: MonitorId,
        from_percent: u8,
        to_percent: u8,
        duration_ms: u64,
        steps: u8,
        manager: Arc<Mutex<MonitorManager>>,
    ) {
        // Cancel previous fade for this monitor
        if let Some(handle) = self.tasks.remove(&monitor_id) {
            handle.abort();
        }

        let id = monitor_id.clone();
        let handle = tokio::spawn(async move {
            fade_task(id, from_percent, to_percent, duration_ms, steps, manager).await;
        });
        self.tasks.insert(monitor_id, handle);
    }

}

async fn fade_task(
    monitor_id: MonitorId,
    from: u8,
    to: u8,
    duration_ms: u64,
    steps: u8,
    manager: Arc<Mutex<MonitorManager>>,
) {
    if steps == 0 || from == to {
        return;
    }

    let step_duration = Duration::from_millis(duration_ms / steps as u64);
    let from_i = from as i16;
    let to_i = to as i16;

    let mut interval = tokio::time::interval(step_duration);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    for i in 1..=steps {
        interval.tick().await;

        let target = if i == steps {
            to
        } else {
            let progress = i as i16;
            let value = from_i + (to_i - from_i) * progress / steps as i16;
            value.clamp(0, 100) as u8
        };

        debug!("fade step {i}/{steps}: {monitor_id} -> {target}%");

        let id = monitor_id.clone();
        let mgr = Arc::clone(&manager);
        // Run the blocking DDC write off the async runtime
        let result = tokio::task::spawn_blocking(move || {
            let mut mgr = mgr.blocking_lock();
            mgr.set_brightness_direct(&id, target)
        })
        .await;

        if let Err(e) = result {
            tracing::warn!("fade step {i} failed: {e}");
            break;
        }
    }
}
