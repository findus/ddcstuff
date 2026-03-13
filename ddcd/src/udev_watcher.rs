use std::time::Duration;

use futures::StreamExt;
use tokio::sync::mpsc;
use tokio_udev::{AsyncMonitorSocket, MonitorBuilder};
use tracing::{debug, info, warn};

#[derive(Debug)]
pub enum RescanReason {
    Startup,
    UdevDrm,
}

/// Watch for DRM hotplug events and send rescan triggers with debouncing.
pub async fn run_udev_watcher(trigger: mpsc::Sender<RescanReason>) -> anyhow::Result<()> {
    let builder = MonitorBuilder::new()
        .map_err(|e| anyhow::anyhow!("create udev monitor builder: {e}"))?;
    let builder = builder
        .match_subsystem("drm")
        .map_err(|e| anyhow::anyhow!("match drm subsystem: {e}"))?;
    let socket = builder
        .listen()
        .map_err(|e| anyhow::anyhow!("listen udev: {e}"))?;
    let mut monitor = AsyncMonitorSocket::new(socket)
        .map_err(|e| anyhow::anyhow!("async monitor socket: {e}"))?;

    // Debounce state: optional pending rescan timer handle
    let (debounce_tx, mut debounce_rx) = mpsc::channel::<()>(1);

    let trigger_clone = trigger.clone();
    tokio::spawn(async move {
        loop {
            // Wait for the first debounce signal
            if debounce_rx.recv().await.is_none() {
                break;
            }
            // Drain any additional signals that arrive within the debounce window
            loop {
                match tokio::time::timeout(Duration::from_millis(1500), debounce_rx.recv()).await {
                    Ok(Some(())) => {} // another event, keep waiting
                    _ => break,       // timeout or channel closed
                }
            }
            info!("DRM event debounced, triggering rescan");
            let _ = trigger_clone.send(RescanReason::UdevDrm).await;
        }
    });

    while let Some(event) = monitor.next().await {
        match event {
            Ok(event) => {
                let action = event.action().and_then(|a| a.to_str().map(|s| s.to_owned()));
                debug!("udev drm event: {:?}", action);
                match action.as_deref() {
                    Some("add") | Some("remove") | Some("change") => {
                        let _ = debounce_tx.try_send(());
                    }
                    _ => {}
                }
            }
            Err(e) => {
                warn!("udev event error: {e}");
            }
        }
    }

    Ok(())
}
