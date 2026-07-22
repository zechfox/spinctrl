# SpinCtrl - System Control Tool Requirements

## Introduction

SpinCtrl is a hybrid system control tool designed for the Acer Spin 13 laptop that provides both autonomous battery care management and interactive system control through a Terminal User Interface (TUI). The system consists of an enhanced bash background service running via systemd that handles hardware interactions, and a Rust-based user-space TUI application that communicates with the service through file-based IPC.

The tool is built around an existing, proven bash script that manages battery charging, CPU scaling, and thermal control via ectool and cpupower. The architecture enhances this bash service with configuration file watching and status reporting capabilities, while adding a modern Rust TUI for real-time monitoring and settings management. This hybrid approach preserves the reliability of the existing bash implementation while adding advanced user interface capabilities.

## Requirements

### 1. Enhanced Bash Background Service
**User Story**: As a system administrator, I want a reliable bash-based background service that manages hardware autonomously while providing IPC capabilities, so that my laptop's battery and thermal performance are optimized without manual intervention and can be monitored by user applications.

**Acceptance Criteria**:
1.1. **When** the system boots up, **the** enhanced bash service **shall** start automatically via systemd with root privileges.
1.2. **When** the AC adapter is plugged in, **the** service **shall** monitor battery capacity and stop charging when the configured threshold is reached.
1.3. **When** the AC adapter is unplugged, **the** service **shall** restore normal charging behavior and update CPU scaling accordingly.
1.4. **When** the service detects configuration file changes via inotify, **the** service **shall** reload and apply new settings within 1 second.
1.5. **When** hardware commands fail, **the** service **shall** log errors to both systemd journal and structured event log and continue operating with degraded functionality.
1.6. **When** the service receives termination signals, **the** service **shall** gracefully shut down, restore normal hardware states, and clean up IPC files.
1.7. **When** the service runs, **it** **shall** periodically write status information to JSON files for TUI consumption.
1.8. **When** the service processes commands from the TUI, **it** **shall** read from a FIFO queue and execute hardware operations safely.

### 2. Battery Management
**User Story**: As a laptop user, I want intelligent battery charging control, so that my battery lifespan is maximized while ensuring adequate charge for my needs.

**Acceptance Criteria**:
2.1. **When** the battery reaches the configured threshold (default 80%), **the** system **shall** stop charging using ectool chargecontrol idle.
2.2. **When** force charging is enabled, **the** system **shall** override threshold settings and charge to 100%.
2.3. **When** the user modifies battery settings, **the** system **shall** validate threshold values are between 50-100%.
2.4. **When** battery capacity is below the threshold and AC is connected, **the** system **shall** resume normal charging.
2.5. **When** ectool commands fail, **the** system **shall** retry up to 3 times before logging an error.

### 3. CPU Performance Management
**User Story**: As a power-conscious user, I want to control CPU performance and power consumption, so that I can balance performance needs with battery life.

**Acceptance Criteria**:
3.1. **When** the user selects a CPU governor, **the** system **shall** apply the setting using cpupower frequency-set.
3.2. **When** the user sets CPU frequency limits, **the** system **shall** validate values against available frequencies.
3.3. **When** CPU scaling settings change, **the** system **shall** apply changes to all CPU cores simultaneously.
3.4. **When** cpupower commands fail, **the** system **shall** maintain current settings and log the error.
3.5. **When** the system starts, **the** service **shall** restore the last configured CPU performance settings.

### 4. Rust Terminal User Interface
**User Story**: As a user, I want an intuitive Rust-based terminal interface to view system status and modify settings, so that I can easily control my laptop's power management without complex commands while benefiting from a modern, responsive UI.

**Acceptance Criteria**:
4.1. **When** the TUI starts, **it** **shall** read status from JSON files and display current battery status, CPU governor, and thermal information in real-time.
4.2. **When** the user navigates the interface, **the** TUI **shall** provide keyboard shortcuts for all major functions and smooth, responsive navigation.
4.3. **When** the user modifies settings, **the** TUI **shall** provide immediate visual feedback, input validation, and preview of changes.
4.4. **When** the user saves changes, **the** TUI **shall** validate the configuration, then send it to the bash service via the `apply_config` FIFO command. The service applies pushed config at runtime; it is not persisted to `/etc` by either component — `/etc` is the boot-time defaults source, editable by hand or via `install.sh`.
4.5. **When** the background service is unavailable, **the** TUI **shall** display a clear error message, show last known status, and provide offline configuration editing.
4.6. **When** the user requests help, **the** TUI **shall** display context-sensitive help information with detailed explanations of hardware controls.
4.7. **When** the TUI runs, **it** **shall** operate with normal user privileges and communicate with the privileged bash service only through file-based IPC.
4.8. **When** status files are updated by the service, **the** TUI **shall** automatically refresh the display without user intervention.

### 5. File-Based IPC and Configuration Management
**User Story**: As a system user, I want my settings to persist across reboots and be easily configurable through a reliable file-based communication system, so that my preferences are maintained without manual reconfiguration and the TUI can safely communicate with the privileged service.

**Acceptance Criteria**:
5.1. **When** settings are changed, **the** TUI **shall** push the full configuration to the bash service via the `apply_config` FIFO command. The service persists the pushed config to `/var/lib/spinctrl/config_status.json` (root:spinctrl, 0640) — the **persistent runtime config** (FHS `/var/lib` variable state). The TUI reads `config_status.json` for display (never reads `/etc`). `/etc/spinctrl/config.json` is the read-only factory default (read by the service at boot only); on boot, the service reads `/etc` then overrides with `config_status` if it exists, so pushed changes survive restarts.
5.2. **When** the bash service starts, **it** **shall** load configuration using jq with fallback to built-in defaults for missing values.
5.3. **When** configuration is updated, **the** bash service **shall** detect changes via inotifywait and reload settings automatically.
5.4. **When** invalid configuration is detected, **the** bash service **shall** use default values, log a warning, and continue operation.
5.5. **When** the configuration file is missing at service start, **the** bash service **shall** fall back to built-in defaults and log a warning. `/etc/spinctrl/config.json` (factory defaults, 0640) is created at install time by `install.sh`. On first boot (no `config_status.json`), the service seeds it from `/etc` so the TUI has a config to display; subsequent `apply_config` pushes update `config_status.json`, which overrides `/etc` on later boots.
5.6. **When** multiple processes access configuration, **the** system **shall** prevent race conditions through atomic file operations using temporary files and moves.
5.7. **When** the TUI sends commands, **it** **shall** write to /var/lib/spinctrl/commands.fifo for the bash service to process.
5.8. **When** the bash service updates status, **it** **shall** write JSON status information to /var/lib/spinctrl/status.json for TUI consumption.
5.9. **When** events occur, **the** bash service **shall** append structured log entries to /var/lib/spinctrl/events.log for TUI history viewing.

### 6. Thermal Management
**User Story**: As a user concerned about laptop thermals, I want the system to manage fan curves and thermal thresholds, so that my laptop runs cool and quiet while preventing overheating.

**Acceptance Criteria**:
6.1. **When** the service starts, **it** **shall** configure thermal thresholds using ectool thermalset for warning, high, shutdown, fan-off, and fan-max temperatures.
6.2. **When** thermal profiles change, **the** system **shall** update EC thermal settings appropriately.
6.3. **When** thermal sensors are unavailable, **the** system **shall** use conservative default values.
6.4. **When** thermal settings are applied, **the** system **shall** verify the changes took effect.

### 7. Security and Privilege Separation
**User Story**: As a security-conscious user, I want the hybrid system to follow strict privilege separation principles, so that security risks are minimized while maintaining functionality through safe file-based communication.

**Acceptance Criteria**:
7.1. **When** the Rust TUI runs, **it** **shall** operate with normal user privileges without requiring sudo or elevated permissions.
7.2. **When** hardware operations are needed, **only** the bash background service **shall** have root privileges and execute ectool/cpupower commands.
7.3. **When** IPC files are accessed, **the** system **shall** enforce appropriate file permissions (0640 for config/status/events, 0750 for directories, 0620 for the FIFO), scoped to the dedicated `spinctrl` group with no world access.
7.4. **When** user input is processed by the TUI, **it** **shall** validate and sanitize all inputs before writing to IPC files to prevent injection attacks.
7.5. **When** the bash service reads commands from FIFO, **it** **shall** validate command format and parameters before execution.
7.6. **When** the service handles privileged operations, **it** **shall** log all hardware state changes to both systemd journal and structured event log for audit purposes.
7.7. **When** file-based IPC is used, **the** system **shall** ensure the bash service owns IPC files and the TUI user has appropriate group access.

### 8. Error Handling and Reliability
**User Story**: As a user, I want the hybrid bash-rust system to be robust and handle errors gracefully, so that hardware failures don't cause system instability or data loss and both components can recover from various failure modes.

**Acceptance Criteria**:
8.1. **When** hardware commands fail in the bash service, **it** **shall** implement retry logic with maximum 3 attempts and exponential backoff.
8.2. **When** critical errors occur, **the** bash service **shall** restore safe default hardware states before exiting.
8.3. **When** the bash service crashes, **systemd** **shall** automatically restart it within 5 seconds and recreate IPC files.
8.4. **When** file operations fail, **both** the bash service and Rust TUI **shall** provide clear error messages indicating the specific failure.
8.5. **When** dependencies are missing (ectool, cpupower, jq, inotifywait), **the** bash service **shall** detect this at startup and log helpful installation guidance.
8.6. **When** IPC files become corrupted or inaccessible, **the** system **shall** recreate them with appropriate defaults and permissions.
8.7. **When** the TUI cannot communicate with the bash service, **it** **shall** continue operating in read-only mode with cached data.
8.8. **When** JSON parsing fails in the bash service, **it** **shall** fall back to built-in defaults and log the parsing error.
8.9. **When** the Rust TUI encounters file permission errors, **it** **shall** provide clear guidance about group membership and file permissions.