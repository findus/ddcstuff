use std::sync::Arc;

use anyhow::Result;
use ddc_lib::{
    ipc::{read_message, write_message},
    protocol::{Request, Response, Target},
};
use tokio::{
    net::UnixListener,
    sync::{watch, Mutex},
};
use tracing::{debug, error, info, warn};

use crate::{
    config::Config,
    fade::FadeController,
    monitor_manager::{apply_op, run_scan, MonitorManager},
};

pub async fn run_server(
    listener: UnixListener,
    manager: Arc<Mutex<MonitorManager>>,
    fade: Arc<Mutex<FadeController>>,
    config: Arc<Config>,
    desired_tx: Arc<watch::Sender<u8>>,
) -> Result<()> {
    info!("listening for connections");
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let manager = Arc::clone(&manager);
                let fade = Arc::clone(&fade);
                let config = Arc::clone(&config);
                let desired_tx = Arc::clone(&desired_tx);
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream, manager, fade, config, desired_tx).await {
                        warn!("connection error: {e}");
                    }
                });
            }
            Err(e) => {
                error!("accept error: {e}");
            }
        }
    }
}

async fn handle_connection(
    stream: tokio::net::UnixStream,
    manager: Arc<Mutex<MonitorManager>>,
    fade: Arc<Mutex<FadeController>>,
    config: Arc<Config>,
    desired_tx: Arc<watch::Sender<u8>>,
) -> Result<()> {
    let (mut reader, mut writer) = stream.into_split();

    let request: Request = read_message(&mut reader).await?;
    debug!("request: {:?}", request);

    let response = dispatch(request, manager, fade, config, desired_tx).await;
    write_message(&mut writer, &response).await?;
    Ok(())
}

async fn dispatch(
    request: Request,
    manager: Arc<Mutex<MonitorManager>>,
    fade: Arc<Mutex<FadeController>>,
    config: Arc<Config>,
    desired_tx: Arc<watch::Sender<u8>>,
) -> Response {
    match request {
        Request::Ping => Response::Pong {
            version: env!("CARGO_PKG_VERSION").to_string(),
        },

        Request::Rescan => {
            let config = manager.lock().await.config();
            let result = tokio::task::spawn_blocking(move || run_scan(&config)).await;
            match result {
                Ok((monitors, backlight)) => {
                    manager.lock().await.apply_scan_results(monitors, backlight);
                    Response::Ok
                }
                Err(e) => Response::Error { message: e.to_string() },
            }
        }

        Request::ListMonitors => {
            let list = manager.lock().await.list_monitors();
            Response::Monitors { list }
        }

        Request::GetBrightness { target } => {
            let manager = Arc::clone(&manager);
            let result = tokio::task::spawn_blocking(move || {
                manager.blocking_lock().get_status(&target)
            })
            .await;
            match result {
                Ok(Ok(monitors)) => Response::Brightness { monitors },
                Ok(Err(e)) => Response::Error {
                    message: e.to_string(),
                },
                Err(e) => Response::Error {
                    message: e.to_string(),
                },
            }
        }

        Request::SetBrightness { target, op, fade: use_fade } => {
            // Determine actual fade setting
            let should_fade = use_fade || config.fade.enabled;

            if should_fade {
                // Resolve targets and start fade tasks
                let ids = {
                    let mgr = manager.lock().await;
                    match mgr.resolve_target(&target) {
                        Ok(ids) => ids,
                        Err(e) => {
                            return Response::Error {
                                message: e.to_string(),
                            };
                        }
                    }
                };

                let duration_ms = config.fade.duration_ms;
                let steps = config.fade.steps;
                let mut fade_ctrl = fade.lock().await;

                for id in ids {
                    let current = {
                        let mgr = manager.lock().await;
                        mgr.ddc_monitors
                            .iter()
                            .find(|m| m.id == id)
                            .map(|m| m.cached_percent())
                            .or_else(|| {
                                mgr.backlight
                                    .as_ref()
                                    .filter(|b| b.id == id)
                                    .map(|b| b.status().brightness_percent)
                            })
                            .unwrap_or(50)
                    };
                    let to = apply_op(&op, current);
                    fade_ctrl.start_fade(
                        id,
                        current,
                        to,
                        duration_ms,
                        steps,
                        Arc::clone(&manager),
                    );
                }
                Response::Ok
            } else if matches!(target, Target::All) {
                // Coalesced fast path for "all monitors": update the desired brightness
                // watch and return immediately. The applier task writes the latest value,
                // dropping any intermediates that piled up during rapid scroll events.
                let current = *desired_tx.borrow();
                let new_val = apply_op(&op, current);
                let _ = desired_tx.send(new_val);
                Response::Ok
            } else {
                // Single-monitor path: blocking write, used infrequently.
                let manager_clone = Arc::clone(&manager);
                let result = tokio::task::spawn_blocking(move || {
                    let mut mgr = manager_clone.blocking_lock();
                    match &target {
                        Target::All => unreachable!(),
                        Target::ById { id } => {
                            mgr.apply_to_monitor(id, &op).map(|s| vec![s])
                        }
                        Target::ByIndex { .. } => {
                            let ids = mgr.resolve_target(&target)?;
                            if let Some(id) = ids.first() {
                                mgr.apply_to_monitor(id, &op).map(|s| vec![s])
                            } else {
                                anyhow::bail!("no monitors found")
                            }
                        }
                    }
                })
                .await;

                match result {
                    Ok(Ok(monitors)) => Response::Brightness { monitors },
                    Ok(Err(e)) => Response::Error { message: e.to_string() },
                    Err(e) => Response::Error { message: e.to_string() },
                }
            }
        }
    }
}
