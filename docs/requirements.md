# SpinCtrl - System Control Tool Requirements

## Introduction

SpinCtrl is a system control tool designed for the Acer Spin 13 laptop that provides both autonomous battery care management and interactive system control through a Terminal User Interface (TUI). The system consists of a privileged background service running via systemd that handles hardware interactions, and a user-space TUI application that communicates with the service through configuration file-based IPC.

The tool extends existing autonomous battery care functionality (based on ectool with root privileges) to include CPU scaling management via cpupower, while providing an intuitive interface for users to adjust settings like charging thresholds, force charging, and CPU performance profiles.

## Requirements

### 1. Background Service Management
**User Story**: As a system administrator, I want a reliable background service that manages hardware autonomously, so that my laptop's battery and thermal performance are optimized without manual intervention.

**Acceptance Criteria**:
1.1. **When** the system boots up, **the** background service **shall** start automatically via systemd with root privileges.
1.2. **When** the AC adapter is plugged in, **the** service **shall** monitor battery capacity and stop charging when the configured threshold is reached.
1.3. **When** the AC adapter is unplugged, **the** service **shall** restore normal charging behavior.
1.4. **When** the service detects configuration file changes, **the** service **shall** reload and apply new settings within 1 second.
1.5. **When** hardware commands fail, **the** service **shall** log errors and continue operating with degraded functionality.
1.6. **When** the service receives termination signals, **the** service **shall** gracefully shut down and restore normal hardware states.

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

### 4. Terminal User Interface
**User Story**: As a user, I want an intuitive terminal interface to view system status and modify settings, so that I can easily control my laptop's power management without complex commands.

**Acceptance Criteria**:
4.1. **When** the TUI starts, **it** **shall** display current battery status, CPU governor, and thermal information in real-time.
4.2. **When** the user navigates the interface, **the** TUI **shall** provide keyboard shortcuts for all major functions.
4.3. **When** the user modifies settings, **the** TUI **shall** provide immediate visual feedback and validation.
4.4. **When** the user saves changes, **the** TUI **shall** write configuration atomically to prevent corruption.
4.5. **When** the background service is unavailable, **the** TUI **shall** display a clear error message and allow viewing cached status.
4.6. **When** the user requests help, **the** TUI **shall** display context-sensitive help information.

### 5. Configuration Management
**User Story**: As a system user, I want my settings to persist across reboots and be easily configurable, so that my preferences are maintained without manual reconfiguration.

**Acceptance Criteria**:
5.1. **When** settings are changed, **the** system **shall** persist configuration in JSON format at /etc/spinctrl/config.json.
5.2. **When** configuration is updated, **the** system **shall** include metadata with timestamps and checksums for validation.
5.3. **When** invalid configuration is detected, **the** system **shall** use default values and log a warning.
5.4. **When** configuration files are missing, **the** system **shall** create default configuration automatically.
5.5. **When** multiple processes access configuration, **the** system **shall** prevent race conditions through atomic file operations.

### 6. Thermal Management
**User Story**: As a user concerned about laptop thermals, I want the system to manage fan curves and thermal thresholds, so that my laptop runs cool and quiet while preventing overheating.

**Acceptance Criteria**:
6.1. **When** the service starts, **it** **shall** configure thermal thresholds using ectool thermalset for warning, high, shutdown, fan-off, and fan-max temperatures.
6.2. **When** thermal profiles change, **the** system **shall** update EC thermal settings appropriately.
6.3. **When** thermal sensors are unavailable, **the** system **shall** use conservative default values.
6.4. **When** thermal settings are applied, **the** system **shall** verify the changes took effect.

### 7. Security and Privileges
**User Story**: As a security-conscious user, I want the system to follow privilege separation principles, so that security risks are minimized while maintaining functionality.

**Acceptance Criteria**:
7.1. **When** the TUI runs, **it** **shall** operate with normal user privileges without requiring sudo.
7.2. **When** hardware operations are needed, **only** the background service **shall** have root privileges.
7.3. **When** configuration files are accessed, **the** system **shall** enforce appropriate file permissions (644 for config, 755 for directories).
7.4. **When** user input is processed, **the** system **shall** validate and sanitize all inputs to prevent injection attacks.
7.5. **When** the service handles privileged operations, **it** **shall** log all hardware state changes for audit purposes.

### 8. Error Handling and Reliability
**User Story**: As a user, I want the system to be robust and handle errors gracefully, so that hardware failures don't cause system instability or data loss.

**Acceptance Criteria**:
8.1. **When** hardware commands fail, **the** system **shall** implement exponential backoff retry logic with maximum 3 attempts.
8.2. **When** critical errors occur, **the** system **shall** restore safe default hardware states before exiting.
8.3. **When** the service crashes, **systemd** **shall** automatically restart it within 5 seconds.
8.4. **When** file operations fail, **the** system **shall** provide clear error messages indicating the specific failure.
8.5. **When** dependencies are missing (ectool, cpupower), **the** system **shall** detect this at startup and provide helpful installation guidance.