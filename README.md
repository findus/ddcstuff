# ddcstuff

DDC/CI brightness control daemon for Linux with CLI and Waybar support.

## Architecture

- **`ddcd`** — daemon that caches connected monitors and serves brightness commands over a Unix socket
- **`ddcctl`** — CLI client for controlling brightness

The daemon handles all hardware communication. The client connects, sends a command, and returns immediately — no slow I2C probing on every invocation.

## Building

```sh
cargo build --release
```

## Permissions

### External monitors (DDC/CI via I2C)

Your user needs to be in the `i2c` group:

```sh
sudo usermod -aG i2c $USER
```

### Internal laptop display (backlight)

Your user needs to be in the `video` group:

```sh
sudo usermod -aG video $USER
```

Or add a udev rule that grants group-write access to the backlight on hotplug:

```
# /etc/udev/rules.d/99-backlight.rules
ACTION=="add", SUBSYSTEM=="backlight", RUN+="/bin/chgrp video /sys/class/backlight/%k/brightness", RUN+="/bin/chmod g+w /sys/class/backlight/%k/brightness"
```

## Usage

### Start the daemon

```sh
ddcd
```

Set `RUST_LOG=debug` for verbose output.

### CLI

```sh
ddcctl set 80          # set all monitors to 80%
ddcctl set +10         # increase all by 10%
ddcctl set -10         # decrease all by 10%
ddcctl set 80 --fade   # animated fade transition
ddcctl set 80 --monitor ddc-BNQ-BenQ_PD2700U-30cb5d40   # single monitor by ID
ddcctl set 80 --monitor 0                                 # single monitor by index
ddcctl get             # current brightness of all monitors
ddcctl list            # list monitors with IDs and current brightness
ddcctl rescan          # force the daemon to re-detect monitors
ddcctl ping            # check if daemon is running
ddcctl waybar          # print Waybar JSON output
```

Brightness floor is 1% — monitors will never go completely dark.


## Waybar

### Module config

```json
"custom/brightness": {
    "exec": "ddcctl waybar",
    "interval": 30,
    "return-type": "json",
    "on-scroll-up": "ddcctl set +5",
    "on-scroll-down": "ddcctl set -5",
    "on-click": "ddcctl set 50"
}
```

### Style

The module emits one of two CSS classes:

| Class | Condition |
|---|---|
| `brightness` | average brightness ≥ 30% |
| `brightness-low` | average brightness < 30% |
| `brightness-error` | daemon not running |

```css
#custom-brightness {
    padding: 0 10px;
}
#custom-brightness.brightness-low {
    color: #888;
}
#custom-brightness.brightness-error {
    color: #f38ba8;
}
```

## Configuration

Optional config file at `~/.config/ddcd/config.toml`:

```toml
[daemon]
socket_path = ""   # default: $XDG_RUNTIME_DIR/ddcd.sock

[fade]
enabled = false    # enable fade by default (can also be requested per-command with --fade)
duration_ms = 400
steps = 20

[backlight]
path = ""          # default: auto-detect; override e.g. "/sys/class/backlight/intel_backlight"
```

## Thunderbolt / dock notes

When connecting a Thunderbolt dock, the daemon detects the hotplug via udev and rescans. If the I2C buses are not yet ready (common with Thunderbolt), it retries up to 5 times with exponential backoff (2 s, 4 s, 8 s, 16 s, 16 s) before giving up.

If monitors are not detected after docking, run `ddcctl rescan` to trigger a manual rescan.

## Nixos Usage Example

```nix
# flake.nix inputs
inputs.ddcstuff.url = "path:/home/findus/repos/ddc";  # or a git URL later

# nixosConfigurations
nixpkgs.overlays = [
  (import /home/findus/repos/ddc/overlay.nix)
  # or if using flake input:
  # ddcstuff.overlays.default
];

environment.systemPackages = [ pkgs.ddcstuff ];
```