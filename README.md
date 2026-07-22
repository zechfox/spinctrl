# SpinCtrl

A hybrid system control tool for the Acer Spin 13 laptop featuring intelligent battery management, CPU performance scaling, and thermal control through a modern Terminal User Interface.

## Overview

SpinCtrl provides autonomous laptop power management through a background service that automatically optimizes battery charging, CPU performance, and thermal settings. Users can monitor and configure the system through an intuitive terminal interface, making advanced hardware control accessible without complex command-line operations.

## Quick Start

1. **Install**: `sudo ./install.sh`
2. **Start service**: `sudo systemctl start spinctrl`
3. **Add user to group**: `sudo usermod -a -G spinctrl $USER` (logout/login required)
4. **Launch TUI**: `spinctrl`

## Key Benefits

- **Extends battery life** by preventing overcharging with configurable thresholds
- **Optimizes performance** by automatically switching CPU governors based on power source
- **Manages thermals** to keep your laptop cool and quiet
- **Easy to use** - no complex configuration files to edit manually

## Features

### Battery Management
- Configurable charge thresholds (50-100%) to extend battery lifespan
- Force charging override for full charges when needed
- Automatic charge control based on AC adapter events
- Real-time battery status monitoring

### CPU Performance Management  
- Automatic CPU governor switching (AC vs battery power)
- Configurable governors: performance, powersave, ondemand, conservative
- Optional frequency limit controls
- Power-aware performance profiles

### Thermal Management
- EC thermal threshold configuration (warning, high, shutdown temperatures)
- Fan control curve management
- Thermal profiles: conservative, balanced, performance
- Real-time temperature monitoring

### User Interface
- Modern terminal UI with tabbed navigation
- Real-time hardware status display
- Interactive configuration editing with validation
- Comprehensive event log viewer
- Context-sensitive help system
- Keyboard shortcuts for power users

## Requirements

### System Dependencies
- `ectool` - Embedded Controller interface (chromium-ec package)
- `cpupower` - CPU frequency scaling utilities
- `jq` - JSON processing for configuration
- `inotifywait` - File system event monitoring (inotify-tools)
- `udevadm` - Hardware event monitoring

### Runtime Requirements
- Linux system with systemd
- Root access for service installation
- Acer Spin 13 hardware with compatible EC
- Rust toolchain for building (cargo)

## Installation

### Quick Install
```bash
git clone https://github.com/user/spinctrl.git
cd spinctrl
sudo ./install.sh
```

### Manual Installation
```bash
# Install dependencies (Ubuntu/Debian)
sudo apt install coreutils cpupower jq inotify-tools udev

# Build Rust components
cargo build --release

# Install binaries
sudo cp target/release/spinctrl /usr/local/bin/
sudo cp spinctrl-service/spinctrl-service.sh /usr/local/bin/spinctrl-service
sudo chmod +x /usr/local/bin/spinctrl-service

# Install systemd service
sudo cp spinctrl-service/spinctrl.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable spinctrl

# Create system group
sudo groupadd -r spinctrl

# Create system config directory (boot-time defaults live here)
sudo mkdir -p /etc/spinctrl
sudo chown root:spinctrl /etc/spinctrl
sudo chmod 750 /etc/spinctrl
# Create /etc/spinctrl/config.json using the schema/example in Configuration Reference.

# Create runtime IPC directory
sudo mkdir -p /var/lib/spinctrl
sudo chown root:spinctrl /var/lib/spinctrl
sudo chmod 750 /var/lib/spinctrl

# Add user to spinctrl group (logout/login required)
sudo usermod -a -G spinctrl $USER
```

## Usage

### Starting the Service
```bash
# Start the background service
sudo systemctl start spinctrl

# Check service status
sudo systemctl status spinctrl

# View service logs
sudo journalctl -u spinctrl -f
```

### Using the TUI

Launch the interface:
```bash
spinctrl
```

**Common Tasks:**
- **Set battery threshold**: Go to Battery tab → Enter → Adjust threshold → Save
- **Change CPU governors**: Go to CPU tab → Enter → Select AC/battery governors → Save  
- **View system status**: Status tab shows real-time battery, AC, and CPU information
- **Force charge to 100%**: Battery tab → Press `f` (useful before trips)
- **Check service health**: `spinctrl --check` (quick status without TUI)

**Navigation:**
- **Tab/1-5**: Switch between Status, Battery, CPU, Thermal, Events
- **Enter/e**: Edit settings in current tab
- **r**: Refresh data  
- **h**: Help
- **q**: Quit

## Configuration Reference

SpinCtrl splits storage by privilege and lifecycle:

| Path | Owner:Group | Mode | Purpose |
|---|---|---|---|
| `/etc/spinctrl/` | `root:spinctrl` | `0750` | System config directory |
| `/etc/spinctrl/config.json` | `root:spinctrl` | `0640` | **Factory defaults** (read-only; service reads at boot only) |
| `/var/lib/spinctrl/` | `root:spinctrl` | `0750` | Runtime state directory |
| `/var/lib/spinctrl/config_status.json` | `root:spinctrl` | `0640` | **Persistent runtime config** (service writes, TUI reads) |
| `/var/lib/spinctrl/status.json` | `root:spinctrl` | `0640` | Live status (service writes, TUI reads) |
| `/var/lib/spinctrl/events.log` | `root:spinctrl` | `0640` | Audit log |
| `/var/lib/spinctrl/commands.fifo` | `root:spinctrl` | `0620` | TUI → service command channel (group write-only) |

The single `spinctrl` system group is the access boundary: TUI users join it to read config_status/status and push commands.

### Write model

- `/etc/spinctrl/config.json` is the **read-only factory default** (FHS static config). The service reads it at boot only; it never writes `/etc` (`ProtectSystem=strict`). The TUI never reads `/etc`.
- `/var/lib/spinctrl/config_status.json` is the **persistent runtime config** (FHS variable state). The service writes it on every `apply_config` push (persisting changes across restarts). The TUI reads this file for display — never `/etc`.
- **Boot order**: the service reads `/etc` (defaults), then overrides with `config_status` if it exists (the persisted runtime config from prior pushes). On first boot (no `config_status`), the service seeds it from `/etc`.
- TUI edits are pushed at **runtime** via the `apply_config:<json>` FIFO command. The service applies them immediately (thermal, governor, threshold) **and** persists them to `config_status.json` — so they survive restarts.
- To reset to factory defaults, delete `/var/lib/spinctrl/config_status.json` (the service re-reads `/etc` defaults on next boot).

### Schema

```jsonc
{
  "battery": {
    "threshold": 80,          // u8, 50..=100
    "force_charge": false     // bool
  },
  "cpu": {
    "governor_ac": "performance",      // performance|powersave|ondemand|conservative|schedutil
    "governor_battery": "powersave",  // same set
    "min_freq_khz": null,              // optional u32, kHz
    "max_freq_khz": null               // optional u32, kHz; must be > min_freq_khz
  },
  "thermal": {
    "warn_temp": 70,         // u8, 40..=100, must be > high_temp
    "high_temp": 55,         // u8, must be < shutdown_temp
    "shutdown_temp": 80,     // u8, 50..=110
    "fan_off_temp": 50,      // u8, must be < fan_max_temp
    "fan_max_temp": 75,      // u8
    "profile": "balanced"    // conservative|balanced|performance|custom
  },
  "version": 1               // u32, defaults to 1 if absent
}
```

### Defaults

| Setting | Default |
|---|---|
| `battery.threshold` | 80 |
| `battery.force_charge` | false |
| `cpu.governor_ac` | `performance` |
| `cpu.governor_battery` | `powersave` |
| `cpu.min_freq_khz` / `max_freq_khz` | none |
| `thermal.warn_temp` / `high_temp` / `shutdown_temp` | 70 / 55 / 80 |
| `thermal.fan_off_temp` / `fan_max_temp` | 50 / 75 |
| `thermal.profile` | `balanced` |

### Validation rules

- `battery.threshold`: 50–100
- `thermal.warn_temp`: 40–100, and `warn_temp > high_temp`
- `thermal.high_temp < shutdown_temp`; `shutdown_temp`: 50–110
- `thermal.fan_off_temp < fan_max_temp`
- `cpu.governor_ac` / `governor_battery`: non-empty; recognized: `performance`, `powersave`, `ondemand`, `conservative`, `schedutil`
- `cpu.min_freq_khz < max_freq_khz` when both set

### Thermal profile presets

| Profile | warn | high | shutdown | fan_off | fan_max |
|---|---|---|---|---|---|
| `conservative` | 65 | 50 | 75 | 45 | 70 |
| `balanced` (default) | 70 | 55 | 80 | 50 | 75 |
| `performance` | 80 | 65 | 90 | 60 | 85 |
| `custom` | leaves current values unchanged | | | | |

### Example

```json
{
  "battery": { "threshold": 60, "force_charge": false },
  "cpu": { "governor_ac": "performance", "governor_battery": "powersave" },
  "thermal": { "warn_temp": 70, "high_temp": 55, "shutdown_temp": 80, "fan_off_temp": 50, "fan_max_temp": 75, "profile": "balanced" },
  "version": 1
}
```

### Recommended settings

- **Daily use**: threshold 80 (default) — good balance of convenience and battery health
- **Desk setup**: threshold 60 — maximize battery lifespan for mostly AC-powered use
- **Travel**: press `f` in the Battery tab before trips for a one-shot full charge to 100%

## Project Structure

```
spinctrl/
├── shared/                    # Shared Rust library
│   ├── src/
│   │   ├── config.rs         # Configuration data structures
│   │   ├── ipc.rs            # IPC management and file operations
│   │   ├── error.rs          # Error types
│   │   └── lib.rs            # Library entry point
│   └── Cargo.toml
├── spinctrl-tui/             # Rust TUI application
│   ├── src/
│   │   ├── app.rs            # Main application logic
│   │   ├── error.rs          # TUI-specific errors
│   │   └── main.rs           # Entry point
│   └── Cargo.toml
├── spinctrl-service/         # Bash service
│   ├── spinctrl-service.sh   # Enhanced service script
│   └── spinctrl.service      # Systemd unit file
├── .claude/specs/spinctrl/   # Design specifications
│   ├── requirements.md       # System requirements
│   ├── design.md            # Architecture design
│   └── tasks.md             # Implementation tasks
├── install.sh               # Installation script
├── Cargo.toml              # Workspace configuration
└── README.md               # This file
```

## Security

The system follows strict privilege separation:

- **Bash service**: Runs as root with minimal systemd security restrictions
- **TUI application**: Runs as normal user, communicates via file-based IPC
- **File permissions**: `0750` for directories, `0640` for config/status/events, `0620` for the commands FIFO; single dedicated `spinctrl` group as the access boundary
- **Input validation**: All user inputs validated before hardware operations
- **Audit logging**: All hardware state changes logged to systemd journal

## Troubleshooting

### Common Issues

**"Service not running" error:**
```bash
sudo systemctl start spinctrl
sudo systemctl status spinctrl  # Check for errors
```

**"Permission denied" when using TUI:**
```bash
sudo usermod -a -G spinctrl $USER  # Add user to group
# Then logout and login again
```

**TUI shows no data:**
```bash
spinctrl --check  # Quick service status
groups $USER      # Verify group membership includes 'spinctrl'
```

**Service won't start:**
```bash
# Check dependencies are installed
which ectool cpupower jq inotifywait

# View service logs for errors
sudo journalctl -u spinctrl -f
```

**Battery threshold not working:**
- Ensure ectool is properly installed and working: `sudo ectool battery`
- Check if your laptop's EC supports charge control
- Some laptops require specific ectool versions or kernel modules

## Contributing

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Make your changes following the existing patterns
4. Test with both the service and TUI components
5. Commit your changes (`git commit -m 'Add amazing feature'`)
6. Push to the branch (`git push origin feature/amazing-feature`)
7. Open a Pull Request

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Hardware Compatibility

Designed and tested for:
- **Acer Spin 13 Chromebook** (CP713-3W series)
- **ChromeOS** and **Linux** environments
- **Embedded Controller** with ectool support
- **Intel CPU** with cpupower governor support

## Acknowledgments

- Inspired by community-developed Acer Spin 13 EC control tooling
- Inspired by the need for better laptop power management on Linux
- Uses the ratatui library for the terminal interface
- Follows systemd best practices for service management