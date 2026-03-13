mod backlight;
mod config;
mod ddc_monitor;
mod fade;
mod monitor_manager;
mod server;
mod udev_watcher;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use ddc_lib::ipc::default_socket_path;
use fade::FadeController;
use monitor_manager::run_scan;
use monitor_manager::MonitorManager;
use tokio::net::UnixListener;
use tokio::sync::{watch, Mutex};
use tracing::{error, info, warn};
use udev_watcher::{run_udev_watcher, RescanReason};

#[derive(Debug, Parser)]
#[command(name = "ddcd", about = "DDC brightness control daemon")]
struct Args {
    /// Unix socket path (overrides config and XDG_RUNTIME_DIR).
    #[arg(long)]
    socket: Option<PathBuf>,

    /// Config file path.
    #[arg(long)]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialise tracing from RUST_LOG env var (default: info)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    let config = Arc::new(config::Config::load().unwrap_or_else(|e| {
        tracing::warn!("could not load config: {e}, using defaults");
        config::Config::default()
    }));

    // Determine socket path
    let socket_path = args
        .socket
        .unwrap_or_else(|| {
            if config.daemon.socket_path.is_empty() {
                default_socket_path()
            } else {
                PathBuf::from(&config.daemon.socket_path)
            }
        });

    // Remove stale socket file
    if socket_path.exists() {
        std::fs::remove_file(&socket_path)
            .with_context(|| format!("remove stale socket {}", socket_path.display()))?;
    }

    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("bind socket {}", socket_path.display()))?;
    // Allow any user to connect (brightness control is not a security boundary).
    std::fs::set_permissions(&socket_path, std::os::unix::fs::PermissionsExt::from_mode(0o666))
        .with_context(|| format!("chmod socket {}", socket_path.display()))?;
    info!("socket: {}", socket_path.display());

    // Manager starts empty — the initial scan runs in the background so the server
    // is immediately available (backlight control works as soon as the first scan
    // finishes, without waiting for DDC retries to complete).
    let manager = Arc::new(Mutex::new(MonitorManager::new(Arc::clone(&config))));

    let fade = Arc::new(Mutex::new(FadeController::new()));

    // Watch channel for coalesced "set all monitors" brightness (starts at 50;
    // updated to the actual value after the first scan completes).
    let (desired_tx, desired_rx) = watch::channel(50u8);
    let desired_tx = Arc::new(desired_tx);

    // Applier: always writes the latest desired brightness; never queues stale values.
    let applier_manager = Arc::clone(&manager);
    tokio::spawn(brightness_applier(desired_rx, applier_manager));

    // Channel for rescan triggers from udev watcher
    let (rescan_tx, mut rescan_rx) = tokio::sync::mpsc::channel::<RescanReason>(8);

    // Spawn udev watcher on a dedicated thread with its own runtime.
    // AsyncMonitorSocket wraps a raw libudev pointer and is not Send, so it
    // cannot run on the multi-threaded tokio pool.
    let udev_tx = rescan_tx.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .expect("build udev runtime");
        rt.block_on(async move {
            if let Err(e) = run_udev_watcher(udev_tx).await {
                error!("udev watcher error: {e}");
            }
        });
    });

    // Rescan handler: retries until the DDC monitor count stabilises.
    // run_scan() does all the slow I/O WITHOUT holding the manager lock, so
    // brightness commands are never blocked while a scan is in progress.
    let manager_for_rescan = Arc::clone(&manager);
    tokio::spawn(async move {
        while let Some(reason) = rescan_rx.recv().await {
            info!("rescanning monitors (reason: {:?})", reason);

            let mut last_found = 0usize;
            const MAX_ATTEMPTS: u32 = 6;
            for attempt in 0..MAX_ATTEMPTS {
                // Grab config without holding the lock for the scan itself.
                let config = manager_for_rescan.lock().await.config();

                // Heavy I/O: no lock held.
                let (monitors, backlight) = match tokio::task::spawn_blocking(move || run_scan(&config)).await {
                    Ok(r) => r,
                    Err(e) => { error!("rescan error: {e}"); break; }
                };

                // Brief lock: just swap in the new results.
                let found = manager_for_rescan.lock().await.apply_scan_results(monitors, backlight);

                if attempt + 1 == MAX_ATTEMPTS { break; }

                if found > last_found {
                    last_found = found;
                    info!("found {found} DDC monitor(s), rescanning in 2s to check for more...");
                    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                } else if found == 0 {
                    let delay = 1u64 << (attempt + 1).min(4);
                    info!("no DDC monitors found, retrying in {delay}s (attempt {}/{MAX_ATTEMPTS})", attempt + 1);
                    tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
                } else {
                    break; // found > 0 and stable
                }
            }
        }
    });

    // Spawn Unix socket server — starts before the initial scan so backlight
    // commands are accepted immediately (DDC retries happen in background).
    let server_manager = Arc::clone(&manager);
    let server_fade = Arc::clone(&fade);
    let server_config = Arc::clone(&config);
    let server_desired = Arc::clone(&desired_tx);
    let server_handle = tokio::spawn(async move {
        if let Err(e) = server::run_server(listener, server_manager, server_fade, server_config, server_desired).await {
            error!("server error: {e}");
        }
    });

    // Trigger initial scan through the same retry loop used for hotplug rescans.
    let _ = rescan_tx.send(RescanReason::Startup).await;

    // Wait for SIGTERM or SIGINT
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("received Ctrl+C, shutting down");
        }
        _ = server_handle => {
            error!("server task ended unexpectedly");
        }
    }

    // Clean up socket
    let _ = std::fs::remove_file(&socket_path);
    info!("goodbye");
    Ok(())
}

/// Applies the latest desired brightness to all monitors.
/// Coalesces rapid updates: if multiple values arrive while a DDC write is in
/// progress, only the most recent one is applied next — intermediates are dropped.
async fn brightness_applier(
    mut rx: watch::Receiver<u8>,
    manager: Arc<Mutex<MonitorManager>>,
) {
    loop {
        // Wait until a new desired value is posted.
        if rx.changed().await.is_err() {
            break;
        }
        // Read the LATEST value (coalesces any intermediates that arrived).
        let target = *rx.borrow_and_update();

        let mgr = Arc::clone(&manager);
        let result = tokio::task::spawn_blocking(move || {
            let mut mgr = mgr.blocking_lock();
            let ids = mgr.all_ids();
            for id in &ids {
                if let Err(e) = mgr.set_brightness_direct(id, target) {
                    warn!("apply brightness to {id}: {e}");
                }
            }
        })
        .await;

        if let Err(e) = result {
            error!("brightness applier task panicked: {e}");
        }
    }
}
