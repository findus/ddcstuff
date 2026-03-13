mod waybar;

use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use ddc_lib::{
    ipc::{default_socket_path, read_message, write_message},
    monitor::MonitorId,
    protocol::{BrightnessOp, Request, Response, Target},
};
use tokio::net::UnixStream;

/// Parse a brightness argument: absolute ("80"), relative increase ("+10"), relative decrease ("-10").
#[derive(Debug, Clone)]
pub enum BrightnessArg {
    Absolute(u8),
    Increase(u8),
    Decrease(u8),
}

impl FromStr for BrightnessArg {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        if let Some(rest) = s.strip_prefix('+') {
            let n: u8 = rest.parse().context("invalid brightness delta")?;
            Ok(BrightnessArg::Increase(n))
        } else if let Some(rest) = s.strip_prefix('-') {
            let n: u8 = rest.parse().context("invalid brightness delta")?;
            Ok(BrightnessArg::Decrease(n))
        } else {
            let n: u8 = s.parse().context("invalid brightness value (0-100)")?;
            Ok(BrightnessArg::Absolute(n))
        }
    }
}

impl From<BrightnessArg> for BrightnessOp {
    fn from(arg: BrightnessArg) -> Self {
        match arg {
            BrightnessArg::Absolute(p) => BrightnessOp::Set { percent: p },
            BrightnessArg::Increase(p) => BrightnessOp::Increase { percent: p },
            BrightnessArg::Decrease(p) => BrightnessOp::Decrease { percent: p },
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "ddcctl", about = "Control monitor brightness via ddcd")]
struct Cli {
    /// Unix socket path (overrides XDG_RUNTIME_DIR default).
    #[arg(long, global = true)]
    socket: Option<PathBuf>,

    #[command(subcommand)]
    command: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Set brightness: absolute (80), increase (+10), decrease (-10).
    Set {
        /// Brightness value: "80" for absolute, "+10" to increase, "-10" to decrease.
        value: BrightnessArg,
        /// Target monitor ID or index (default: all monitors).
        #[arg(long)]
        monitor: Option<String>,
        /// Animate the transition with a fade effect.
        #[arg(long)]
        fade: bool,
    },
    /// Get current brightness.
    Get {
        /// Target monitor ID or index (default: all monitors).
        #[arg(long)]
        monitor: Option<String>,
    },
    /// List connected monitors with their IDs.
    List,
    /// Output brightness as Waybar custom module JSON.
    Waybar,
    /// Force the daemon to rescan monitors.
    Rescan,
    /// Check if the daemon is running.
    Ping,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let socket_path = cli.socket.clone().unwrap_or_else(default_socket_path);
    let is_waybar = matches!(cli.command, Cmd::Waybar);

    let request = build_request(cli.command);
    let response = send_request(&socket_path, request).await;

    match response {
        Ok(resp) => {
            if is_waybar {
                print_waybar(resp);
            } else {
                print_response(resp);
            }
        }
        Err(e) => {
            if is_waybar {
                // Never break waybar with a non-JSON error
                let output = waybar::WaybarOutput {
                    text: "N/A".to_string(),
                    tooltip: format!("ddcd not running: {e}"),
                    class: "brightness-error".to_string(),
                    percentage: 0,
                };
                println!("{}", serde_json::to_string(&output).unwrap());
            } else {
                if e.to_string().contains("No such file") || e.to_string().contains("Connection refused") {
                    eprintln!("error: could not connect to ddcd at {}", socket_path.display());
                    eprintln!("  Is ddcd running? Start it with: ddcd");
                } else {
                    eprintln!("error: {e}");
                }
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

fn build_request(cmd: Cmd) -> Request {
    match cmd {
        Cmd::Set { value, monitor, fade } => Request::SetBrightness {
            target: resolve_target(monitor),
            op: BrightnessOp::from(value),
            fade,
        },
        Cmd::Get { monitor } => Request::GetBrightness {
            target: resolve_target(monitor),
        },
        Cmd::List => Request::ListMonitors,
        Cmd::Waybar => Request::GetBrightness {
            target: Target::All,
        },
        Cmd::Rescan => Request::Rescan,
        Cmd::Ping => Request::Ping,
    }
}

fn resolve_target(monitor: Option<String>) -> Target {
    match monitor {
        None => Target::All,
        Some(id) => {
            if let Ok(index) = id.parse::<usize>() {
                Target::ByIndex { index }
            } else {
                Target::ById { id: MonitorId(id) }
            }
        }
    }
}

async fn send_request(socket_path: &std::path::Path, request: Request) -> Result<Response> {
    let stream = UnixStream::connect(socket_path)
        .await
        .with_context(|| format!("connect to {}", socket_path.display()))?;
    let (mut reader, mut writer) = stream.into_split();
    write_message(&mut writer, &request).await?;
    let response: Response = read_message(&mut reader).await?;
    Ok(response)
}

fn print_waybar(response: Response) {
    let output = match response {
        Response::Brightness { monitors } => waybar::format_waybar(&monitors),
        Response::Error { message } => waybar::WaybarOutput {
            text: "N/A".to_string(),
            tooltip: message,
            class: "brightness-error".to_string(),
            percentage: 0,
        },
        _ => waybar::WaybarOutput {
            text: "N/A".to_string(),
            tooltip: "unexpected response".to_string(),
            class: "brightness-error".to_string(),
            percentage: 0,
        },
    };
    println!("{}", serde_json::to_string(&output).unwrap());
}

fn print_response(response: Response) {
    match response {
        Response::Ok => {}
        Response::Pong { version } => {
            println!("ddcd {version} is running");
        }
        Response::Error { message } => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Response::Brightness { monitors } => {
            for m in &monitors {
                println!("{}: {}%  ({})", m.name, m.brightness_percent, m.id);
            }
        }
        Response::Monitors { list } => {
            if list.is_empty() {
                println!("no monitors found");
                return;
            }
            println!("{:<4} {:<44} {:<10} {:>6}", "idx", "id", "kind", "bri");
            println!("{}", "-".repeat(68));
            for (i, m) in list.iter().enumerate() {
                println!(
                    "{:<4} {:<44} {:<10} {:>5}%",
                    i,
                    m.id,
                    format!("{:?}", m.kind).to_lowercase(),
                    m.brightness_percent
                );
            }
        }
    }
}
