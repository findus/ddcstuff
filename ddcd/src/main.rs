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
    info!("socket: {}", socket_path.display());

    // Initial monitor scan (blocking, before entering async loop)
    let manager = {
        let mut mgr = MonitorManager::new(Arc::clone(&config));
        // Run the scan in a blocking thread so we don't block tokio
        let mgr = tokio::task::spawn_blocking(move || {
            mgr.scan();
            mgr
        })
        .await?;
        Arc::new(Mutex::new(mgr))
    };

    let fade = Arc::new(Mutex::new(FadeController::new()));

    // Watch channel for coalesced "set all monitors" brightness.
    // Each SetBrightness{All} request just updates this value; a single applier
    // task drains it as fast as DDC allows. Rapid scroll events are coalesced so
    // only the latest pending value is written — no queue buildup.
    let initial_brightness = {
        let mgr = manager.lock().await;
        let ids = mgr.all_ids();
        if ids.is_empty() {
            50u8
        } else {
            let sum: u32 = mgr
                .list_monitors()
                .iter()
                .map(|m| m.brightness_percent as u32)
                .sum();
            (sum / ids.len() as u32).clamp(1, 100) as u8
        }
    };
    let (desired_tx, desired_rx) = watch::channel(initial_brightness);
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

    // Spawn rescan handler
    let manager_for_rescan = Arc::clone(&manager);
    tokio::spawn(async move {
        while let Some(reason) = rescan_rx.recv().await {
            info!("rescanning monitors (reason: {:?})", reason);
            let mgr = Arc::clone(&manager_for_rescan);
            if let Err(e) = tokio::task::spawn_blocking(move || {
                mgr.blocking_lock().scan();
            })
            .await
            {
                error!("rescan error: {e}");
            }
        }
    });

    // Spawn Unix socket server
    let server_manager = Arc::clone(&manager);
    let server_fade = Arc::clone(&fade);
    let server_config = Arc::clone(&config);
    let server_desired = Arc::clone(&desired_tx);
    let server_handle = tokio::spawn(async move {
        if let Err(e) = server::run_server(listener, server_manager, server_fade, server_config, server_desired).await {
            error!("server error: {e}");
        }
    });

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
