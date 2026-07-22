# SpinCtrl Implementation Plan

> **Reconciled** against the actual codebase. `[x]` = done or substantially delivered; `[ ]` = not started. Items whose core is delivered but with notable gaps carry an inline `— partial: …` note flagging the remaining work.

## Implementation Tasks

### 1. Project Foundation Setup
- [x] 1.1 Create Cargo workspace with daemon and TUI binaries
  - Set up `Cargo.toml` workspace configuration
  - Create `spinctrl-service` directory for bash service files
  - Create `spinctrl-tui` directory for Rust TUI application
  - Add core dependencies: `serde`, `serde_json`, `tokio`, `clap`, `ratatui`, `crossterm`, `notify`
  - Configure workspace-level settings and shared dependencies
  - *References requirements: 1.1, 4.7, 7.1, 7.2*

- [x] 1.2 Implement configuration data structures and JSON schema
  - Create `Config` struct with battery, CPU, and thermal settings
  - Implement JSON serialization/deserialization with `serde`
  - Add configuration validation logic with error types
  - Create default configuration values and schema validation
  - Write unit tests for configuration parsing and validation
  - *References requirements: 5.1, 5.4, 5.5, 8.9*

- [x] 1.3 Create IPC directory structure and file management utilities
  - Implement atomic file operations using temporary files and moves
  - Create directory setup functions with proper permissions (0750 dirs, 0640 files, 0620 FIFO)
  - Add file permission checking and error handling
  - Write utility functions for safe JSON file reading/writing
  - Create test fixtures for IPC file operations
  - *References requirements: 5.6, 7.3, 7.7, 8.6*

### 2. Enhanced Bash Service Implementation
- [x] 2.1 Enhance existing bash script with configuration file loading
  - Add `load_config()` function (config at `/etc/spinctrl/config.json`)
  - Implement JSON parsing using `jq` with fallback to defaults
  - Add configuration validation in bash (threshold ranges, valid governors)
  - Backward compatibility when config file is missing (built-in defaults)
  - *References requirements: 1.4, 5.2, 5.4, 8.8*

- [x] 2.2 Add status reporting functionality to bash service
  - Implement `write_status()` function that creates JSON status files
  - Include battery capacity, AC status, CPU governor, charge control state
  - Add timestamp and service PID to status information
  - Use atomic file operations (write to .tmp, then move)
  - Create periodic status updates (every 30 seconds)
  - *References requirements: 1.7, 4.1, 4.8*

- [x] 2.3 Implement command processing via FIFO in bash service
  - Create and manage `/var/lib/spinctrl/commands.fifo` named pipe
  - Add `process_commands()` function to read from FIFO
  - Implement command parsing and validation (battery_threshold, force_charge, cpu_governor, apply_config)
  - Add command execution with error handling and logging
  - *References requirements: 1.8, 5.7, 7.5, 8.1*

- [x] 2.4 Add configuration file watching with inotify to bash service
  - Implement configuration file monitoring using `inotifywait`
  - Add automatic config reload when file changes detected
  - Add debouncing to prevent rapid reloads during file writes (0.5s)
  - Test configuration hot-reloading functionality
  - *References requirements: 1.4, 5.3*
  - — partial: explicit file-deletion/recreation edge-case handling is light (`-e modify,move,create` covers the common path)

- [x] 2.5 Add logging and event tracking in bash service
  - Add structured event logging to `/var/lib/spinctrl/events.log`
  - Include event types: config_changed, command_executed, hardware_action, error
  - Add timestamps, event details, and context information
  - Ensure logs are readable by TUI user group (0640 root:spinctrl)
  - *References requirements: 1.5, 5.9, 7.6, 8.4*
  - — partial: **log rotation is NOT implemented** (events.log grows unbounded)

- [x] 2.6 Add IPC file cleanup and graceful shutdown to bash service
  - Enhance existing `cleanup()` function to remove IPC files
  - Add proper signal handling for SIGTERM and SIGINT
  - Ensure hardware state restoration before exit
  - Verify IPC files are recreated on service restart (ExecStartPre)
  - *References requirements: 1.6, 8.3, 8.6*

### 3. Rust TUI Core Implementation
- [x] 3.1 Create basic TUI application structure and navigation
  - Implement `App` struct with application state management
  - Create tab-based navigation (Status, Battery, CPU, Thermal, Events)
  - Add keyboard event handling and navigation controls
  - Implement basic UI layout with header, content, and footer
  - Add application mode handling (Monitoring, Editing, Help, Popup)
  - *References requirements: 4.2, 4.6*

- [x] 3.2 Implement status monitoring and real-time display
  - Create status data structures matching JSON schema
  - Implement status file reading and parsing
  - Add real-time status display with battery, AC, CPU information
  - Create visual indicators for charging state and thresholds
  - Handle missing or corrupted status files gracefully
  - *References requirements: 4.1, 4.5, 8.7*

- [x] 3.3 Add file watching for automatic UI updates
  - Implement file system watching using `notify` crate
  - Watch `/var/lib/spinctrl/` directory for file changes
  - Add automatic UI refresh when status files update
  - Keep 2s polling as fallback
  - *References requirements: 4.8*
  - — partial: watcher-error reconnection not explicitly handled (degrades to the polling fallback)

- [x] 3.4 Create configuration editing interface
  - Implement interactive forms for battery, CPU, and thermal settings
  - Add input validation with immediate feedback
  - Create confirmation dialogs for configuration changes
  - Implement undo/cancel functionality for editing sessions
  - Add visual indicators for modified but unsaved settings
  - *References requirements: 4.3, 4.4, 8.9*
  - — partial: editing infrastructure (mode, `input_buffer`, cursor, save→`ApplyConfig`) exists, but `input_buffer` is **not wired to config fields**; no undo/cancel/preview/modified-indicators

### 4. Battery Management Features
- [x] 4.1 Implement battery threshold control in TUI
  - Display current threshold status and charging behavior (`draw_battery_tab`)
  - Threshold validation (50-100%) in `BatteryConfig::validate` + `save_current_edit` gate
  - Immediate threshold changes via `ApplyConfig` FIFO command
  - *References requirements: 2.1, 2.3, 2.4*
  - — partial: no slider/input widget; editing UX incomplete (see 3.4)

- [x] 4.2 Add force charging functionality
  - Send `force_charge` command (key `f`)
  - Service-side force-charge handling with auto-disable at 100%
  - Display force-charge status
  - *References requirements: 2.2*

- [x] 4.3 Enhance battery monitoring logic in bash service
  - Threshold-based stop via `ectool chargecontrol idle`
  - AC plug/unplug monitoring + threshold-change handling
  - *References requirements: 2.1, 2.4, 2.5*
  - — partial: no battery health/SOH indicators or extended status

### 5. CPU Performance Management
- [x] 5.1 Implement CPU governor selection in TUI
  - Display governor_ac/governor_battery (`draw_cpu_tab`)
  - Separate AC and battery governor settings
  - Governor change via `ApplyConfig`
  - *References requirements: 3.1, 3.5*
  - — partial: available governors are a hardcoded list (`Config::get_available_governors`), not read from `/sys`; no dropdown widget

- [x] 5.2 Add CPU frequency management interface
  - `min_freq_khz`/`max_freq_khz` fields in `CpuConfig`
  - Validate against available frequencies (`available_cpu_frequencies` reads sysfs) — req 3.2
  - *References requirements: 3.2*
  - — partial: **service does not apply freq limits** (no `cpupower frequency-set -u/-d`); TUI fields + validation exist but aren't enforced on hardware

- [x] 5.3 Enhance CPU scaling in bash service
  - AC/battery-specific governor application (`handle_ac_plugged`/`unplugged`)
  - All-core verification via sysfs `scaling_governor` (req 3.3)
  - Retry with exponential backoff (req 8.1)
  - *References requirements: 3.1, 3.3, 3.4*
  - — partial: frequency-limit application not implemented (see 5.2)

### 6. Thermal Management Implementation
- [x] 6.1 Create thermal configuration interface in TUI
  - Display warn/high/shutdown/fan temps + profile (`draw_thermal_tab`)
  - Temperature validation + range checking (`ThermalConfig::validate`)
  - Thermal profile selection (`apply_thermal_profile`)
  - *References requirements: 6.1, 6.2*
  - — partial: no interactive form widget

- [x] 6.2 Enhance thermal management in bash service
  - `configure_thermal` with `ectool thermalset`, config-driven
  - Thermal profiles with predefined temperature sets
  - Validation before applying (`validate_config`)
  - EC-availability probe + conservative-default fallback (req 6.3)
  - Read-back verification via `ectool thermalget` (req 6.4)
  - *References requirements: 6.1, 6.3, 6.4*

### 7. Event Logging and History
- [x] 7.1 Implement event log viewer in TUI
  - Event log display widget with scrolling (`draw_events_tab`, `List` + `ListState`)
  - Parse and display structured event log entries (type tags)
  - *References requirements: 5.9*
  - — partial: no filtering by type/time, no search/highlight, no export

- [x] 7.2 Add comprehensive event tracking to bash service
  - Structured event format (`write_event`, JSON)
  - Events for config/command/hardware/error
  - Event severity via `event_type` categorization
  - Readable by TUI group (0640)
  - *References requirements: 1.5, 7.6*
  - — partial: **log rotation not implemented**

### 8. Error Handling and Recovery
- [x] 8.1 Implement comprehensive error handling in TUI
  - Error display via popup (`draw_popup`)
  - Permission-error → group-membership guidance (`explain_error`, req 8.9)
  - Offline indicator (footer Service: Online/Offline)
  - Degraded operation with cached last status
  - *References requirements: 4.5, 8.7, 8.9*
  - — partial: no auto-retry for failed sends; no explicit `ServiceOffline` mode (degrades implicitly)

- [x] 8.2 Add robust error handling to bash service
  - Retry logic with exponential backoff (delay doubling, req 8.1)
  - Hardware command failure recovery (3 attempts)
  - Dependency checking at startup (`check_dependencies`)
  - Safe default restoration on critical errors (`cleanup`)
  - *References requirements: 8.1, 8.2, 8.5, 8.8*

### 9. Help System and Documentation
- [x] 9.1 Create context-sensitive help system in TUI
  - Help overlay with keyboard shortcuts (`draw_help`)
  - *References requirements: 4.6*
  - — partial: no detailed per-option explanations, no troubleshooting guide, no help search

- [x] 9.2 Add user guidance and status explanations
  - Permission-issue guidance (req 8.9)
  - User manual + configuration examples (README)
  - *References requirements: 8.9*
  - — partial: no tooltips, no setup wizard, no hardware-compatibility checking

### 10. Testing and Integration
- [x] 10.1 Create comprehensive test suite for Rust components
  - Unit tests for config/ipc/error/app (66 tests pass)
  - `TestBackend` render smoke test (all tabs + help + popups)
  - *References requirements: All TUI and configuration requirements*
  - — partial: no integration tests for IPC, no mock-service tests, no property-based tests, no automated UI-input simulation

- [ ] 10.2 Add testing framework for bash service
  - No bash test harness exists
  - *References requirements: All service and IPC requirements*

- [x] 10.3 Create installation and deployment scripts
  - `install.sh` (builds, installs binaries, systemd unit, `/etc` config, `/var/lib` runtime, `spinctrl` group)
  - `spinctrl.service` (systemd unit with security restrictions, `Group=spinctrl`)
  - User group management (`spinctrl` group)
  - *References requirements: 1.1, 7.3, 7.7*
  - — partial: **no uninstallation script**; no multi-distro testing

### 11. Final Integration and Polish
- [x] 11.1 Integrate all components and test end-to-end functionality
  - TUI ↔ service via FIFO + JSON IPC
  - Config change workflow (TUI → `ApplyConfig` → service applies at runtime)
  - Service restart recovery (systemd `Restart=always` + `ExecStartPre`)
  - *References requirements: All requirements across all categories*
  - — partial: no formal e2e test suite; hardware-through-TUI not verified on real hardware

- [x] 11.2 Add performance optimization and polish
  - Event-driven refresh via `notify` (reduced polling reliance, req 4.8)
  - Keyboard shortcuts
  - TUI rendering (5 tabs + help + popups)
  - Thermal profiles (presets)
  - *References requirements: 4.2, 4.3, 4.8*
  - — partial: no user profiles; animations minimal

- [x] 11.3 Create comprehensive documentation and examples
  - README with Configuration Reference (paths, schema, defaults, validation, examples)
  - Troubleshooting section
  - *References requirements: All requirements for user guidance and help*
  - — partial: no developer/extending docs; no video/tutorial
