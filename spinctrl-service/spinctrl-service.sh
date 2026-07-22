#!/bin/bash

# SpinCtrl Enhanced Service - IPC + hardware control for the TUI
# Enhanced version with IPC support for communication with TUI

SCRIPT_NAME="spinctrl-service"
LOG_TAG="$SCRIPT_NAME"

# Paths
# /etc/spinctrl/config.json — read-only factory defaults (FHS static config).
#   The service reads it at boot + on inotify reload; never writes it.
# /var/lib/spinctrl/config_status.json — persistent runtime config (FHS variable
#   state). On boot the service reads /etc defaults, then overrides with this
#   file if it exists (the persisted config from prior apply_config pushes).
#   The service writes this file on every apply_config, persisting pushes
#   across restarts. The TUI reads this file (never /etc).
# Runtime state (status/fifo/events) also lives in /var/lib/spinctrl.
IPC_DIR="/var/lib/spinctrl"
CONFIG_FILE="/etc/spinctrl/config.json"
CONFIG_STATUS_FILE="$IPC_DIR/config_status.json"
STATUS_FILE="$IPC_DIR/status.json"
COMMANDS_FIFO="$IPC_DIR/commands.fifo"
EVENTS_LOG="$IPC_DIR/events.log"

# Bare default variables. validate_config resets to these on bad input, so
# they must be defined (previously referenced but never assigned, which made
# validation-on-error wipe values to empty strings).
DEFAULT_BATTERY_THRESHOLD=80
DEFAULT_WARN_TEMP=70
DEFAULT_HIGH_TEMP=55
DEFAULT_SHUTDOWN_TEMP=80
DEFAULT_FAN_OFF_TEMP=50
DEFAULT_FAN_MAX_TEMP=75
DEFAULT_CPU_GOVERNOR_AC="performance"
DEFAULT_CPU_GOVERNOR_BATTERY="powersave"

# Configuration Variables with Defaults
declare -A CONFIG=(
    [BATTERY_THRESHOLD]=80
    [WARN_TEMP]=70
    [HIGH_TEMP]=55
    [SHUTDOWN_TEMP]=80
    [FAN_OFF_TEMP]=50
    [FAN_MAX_TEMP]=75
    [CPU_GOVERNOR_AC]="performance"
    [CPU_GOVERNOR_BATTERY]="powersave"
    [FORCE_CHARGE]=false
    [MIN_FREQ_KHZ]=""
    [MAX_FREQ_KHZ]=""
)

# Runtime State
MONITOR_PID=""
CONFIG_WATCHER_PID=""
COMMAND_PROCESSOR_PID=""
SHUTDOWN_REQUESTED=false

# Hardware Paths
AC_ADAPTER_PATH="/sys/class/power_supply/AC"
BATTERY_PATH="/sys/class/power_supply/BAT0"

# Helper function to get config values
get_config() { echo "${CONFIG[$1]}"; }

# Logging function with structured output
log() {
    local level="$1"
    local message="$2"
    local timestamp=$(date -Iseconds)
    
    # Log to systemd journal
    logger -t "$LOG_TAG" "[$level] $message"
    
    # Also log to stderr for systemd
    echo "[$timestamp] [$LOG_TAG] [$level] $message" >&2
}

# Rotate events log when it exceeds 1MB (cap 3 rotated copies)
rotate_events_log() {
    local log="$EVENTS_LOG"
    local max_size=1048576  # 1MB

    if [[ -f "$log" ]]; then
        local size
        size=$(stat -c%s "$log" 2>/dev/null || echo 0)
        if [[ "$size" -gt "$max_size" ]]; then
            rm -f "$log.3"
            [[ -f "$log.2" ]] && mv "$log.2" "$log.3"
            [[ -f "$log.1" ]] && mv "$log.1" "$log.2"
            mv "$log" "$log.1"
            touch "$log"
            chmod 640 "$log"
        fi
    fi
}

# Write structured event to events log
write_event() {
    rotate_events_log
    local event_type="$1" message="$2" details="$3"
    local timestamp=$(date -Iseconds)
    
    if command -v jq >/dev/null 2>&1; then
        local jq_args=(--arg timestamp "$timestamp" --arg event_type "$event_type" --arg message "$message")
        local jq_template='{timestamp: $timestamp, event_type: $event_type, message: $message'
        
        if [[ -n "$details" ]]; then
            jq_args+=(--argjson details "$details")
            jq_template+=', details: $details'
        fi
        jq_template+='}'
        
        jq -n "${jq_args[@]}" "$jq_template" >> "$EVENTS_LOG"
    else
        echo "[$timestamp] $event_type: $message" >> "$EVENTS_LOG"
    fi
}

# Check dependencies
check_dependencies() {
    local missing_deps=()
    
    for cmd in ectool cpupower jq inotifywait udevadm; do
        if ! command -v "$cmd" >/dev/null 2>&1; then
            missing_deps+=("$cmd")
        fi
    done
    
    if [[ ${#missing_deps[@]} -gt 0 ]]; then
        log "ERROR" "Missing required dependencies: ${missing_deps[*]}"
        log "INFO" "Install missing dependencies:"
        log "INFO" "  Ubuntu/Debian: sudo apt install coreutils cpupower jq inotify-tools udev"
        log "INFO" "  For ectool: Install chromium-ec package or build from source"
        return 1
    fi
    
    return 0
}

# Initialize IPC directory structure
init_ipc() {
    log "INFO" "Initializing IPC directory structure"
    
    # Create IPC directory if it doesn't exist
    if [[ ! -d "$IPC_DIR" ]]; then
        mkdir -p "$IPC_DIR" || {
            log "ERROR" "Failed to create IPC directory: $IPC_DIR"
            return 1
        }
    fi
    
    # Set proper permissions (0750 rwxr-x--- : root:spinctrl only)
    chmod 750 "$IPC_DIR" || {
        log "ERROR" "Failed to set permissions on IPC directory"
        return 1
    }
    
    # Config lives in /etc and is created by install.sh; the service cannot
    # write it (ProtectSystem=strict). Fall back to built-in defaults if missing.
    if [[ ! -f "$CONFIG_FILE" ]]; then
        log "WARN" "Config $CONFIG_FILE missing; using built-in defaults. Run install.sh to create it."
    fi
    
    # Create FIFO for commands; if a non-FIFO file occupies the path (corruption),
    # remove it first so mkfifo can recreate (req 8.6).
    if [[ -e "$COMMANDS_FIFO" && ! -p "$COMMANDS_FIFO" ]]; then
        log "WARN" "$COMMANDS_FIFO exists but is not a FIFO; recreating"
        rm -f "$COMMANDS_FIFO"
    fi
    if [[ ! -p "$COMMANDS_FIFO" ]]; then
        mkfifo "$COMMANDS_FIFO" || {
            log "ERROR" "Failed to create commands FIFO"
            return 1
        }
        chmod 620 "$COMMANDS_FIFO"
    fi
    
    # Initialize events log (0640 rw-r----- : root:spinctrl only)
    if [[ ! -f "$EVENTS_LOG" ]]; then
        touch "$EVENTS_LOG"
        chmod 640 "$EVENTS_LOG"
    fi
    
    write_event "service_start" "SpinCtrl service started" '{"pid": '$$'}'
    return 0
}

# Load configuration from JSON file
load_config() {
    local effective_config="$CONFIG_FILE"
    if [[ -f "$CONFIG_STATUS_FILE" ]]; then
        effective_config="$CONFIG_STATUS_FILE"
    elif [[ ! -f "$CONFIG_FILE" ]]; then
        log "WARN" "No config (neither $CONFIG_STATUS_FILE nor $CONFIG_FILE); using built-in defaults"
        return 0
    fi
    
    if ! command -v jq >/dev/null 2>&1; then
        log "ERROR" "jq not available for configuration parsing"
        return 1
    fi
    
    log "INFO" "Loading configuration from $effective_config"
    
    # Parse configuration with fallbacks to defaults
    CONFIG[BATTERY_THRESHOLD]=$(jq -r ".battery.threshold // ${CONFIG[BATTERY_THRESHOLD]}" "$effective_config" 2>/dev/null || echo "${CONFIG[BATTERY_THRESHOLD]}")
    CONFIG[FORCE_CHARGE]=$(jq -r ".battery.force_charge // ${CONFIG[FORCE_CHARGE]}" "$effective_config" 2>/dev/null || echo "${CONFIG[FORCE_CHARGE]}")
    CONFIG[CPU_GOVERNOR_AC]=$(jq -r ".cpu.governor_ac // \"${CONFIG[CPU_GOVERNOR_AC]}\"" "$effective_config" 2>/dev/null || echo "${CONFIG[CPU_GOVERNOR_AC]}")
    CONFIG[CPU_GOVERNOR_BATTERY]=$(jq -r ".cpu.governor_battery // \"${CONFIG[CPU_GOVERNOR_BATTERY]}\"" "$effective_config" 2>/dev/null || echo "${CONFIG[CPU_GOVERNOR_BATTERY]}")
    CONFIG[MIN_FREQ_KHZ]=$(jq -r ".cpu.min_freq_khz // empty" "$effective_config" 2>/dev/null || echo "")
    CONFIG[MAX_FREQ_KHZ]=$(jq -r ".cpu.max_freq_khz // empty" "$effective_config" 2>/dev/null || echo "")
    CONFIG[WARN_TEMP]=$(jq -r ".thermal.warn_temp // ${CONFIG[WARN_TEMP]}" "$effective_config" 2>/dev/null || echo "${CONFIG[WARN_TEMP]}")
    CONFIG[HIGH_TEMP]=$(jq -r ".thermal.high_temp // ${CONFIG[HIGH_TEMP]}" "$effective_config" 2>/dev/null || echo "${CONFIG[HIGH_TEMP]}")
    CONFIG[SHUTDOWN_TEMP]=$(jq -r ".thermal.shutdown_temp // ${CONFIG[SHUTDOWN_TEMP]}" "$effective_config" 2>/dev/null || echo "${CONFIG[SHUTDOWN_TEMP]}")
    CONFIG[FAN_OFF_TEMP]=$(jq -r ".thermal.fan_off_temp // ${CONFIG[FAN_OFF_TEMP]}" "$effective_config" 2>/dev/null || echo "${CONFIG[FAN_OFF_TEMP]}")
    CONFIG[FAN_MAX_TEMP]=$(jq -r ".thermal.fan_max_temp // ${CONFIG[FAN_MAX_TEMP]}" "$effective_config" 2>/dev/null || echo "${CONFIG[FAN_MAX_TEMP]}")

    # Propagate associative array into the bare variables consumed by
    # monitor_battery / handle_ac_plugged / configure_thermal / write_status.
    # Without this sync, parsed values sat in CONFIG[...] but the runtime read
    # $BATTERY_THRESHOLD etc. (empty), so loaded config never took effect.
    BATTERY_THRESHOLD="${CONFIG[BATTERY_THRESHOLD]}"
    FORCE_CHARGE="${CONFIG[FORCE_CHARGE]}"
    CPU_GOVERNOR_AC="${CONFIG[CPU_GOVERNOR_AC]}"
    CPU_GOVERNOR_BATTERY="${CONFIG[CPU_GOVERNOR_BATTERY]}"
    WARN_TEMP="${CONFIG[WARN_TEMP]}"
    HIGH_TEMP="${CONFIG[HIGH_TEMP]}"
    SHUTDOWN_TEMP="${CONFIG[SHUTDOWN_TEMP]}"
    FAN_OFF_TEMP="${CONFIG[FAN_OFF_TEMP]}"
    FAN_MAX_TEMP="${CONFIG[FAN_MAX_TEMP]}"
    MIN_FREQ_KHZ="${CONFIG[MIN_FREQ_KHZ]}"
    MAX_FREQ_KHZ="${CONFIG[MAX_FREQ_KHZ]}"

    # Validate configuration
    validate_config || return 1
    
    log "INFO" "Configuration loaded: battery_threshold=$BATTERY_THRESHOLD, cpu_ac=$CPU_GOVERNOR_AC, cpu_battery=$CPU_GOVERNOR_BATTERY, min_freq_khz=$MIN_FREQ_KHZ, max_freq_khz=$MAX_FREQ_KHZ"
    write_event "config_changed" "Configuration reloaded" \
        '{"battery_threshold": '$BATTERY_THRESHOLD', "cpu_governor_ac": "'$CPU_GOVERNOR_AC'", "cpu_governor_battery": "'$CPU_GOVERNOR_BATTERY'"}'
    
    return 0
}

# Validate loaded configuration values
validate_config() {
    local errors=()
    
    # Validate battery threshold
    if [[ ! "$BATTERY_THRESHOLD" =~ ^[0-9]+$ ]] || [[ "$BATTERY_THRESHOLD" -lt 50 ]] || [[ "$BATTERY_THRESHOLD" -gt 100 ]]; then
        errors+=("Battery threshold must be between 50-100, got: $BATTERY_THRESHOLD")
        BATTERY_THRESHOLD="$DEFAULT_BATTERY_THRESHOLD"
    fi
    
    # Validate thermal temperatures
    if [[ ! "$WARN_TEMP" =~ ^[0-9]+$ ]] || [[ "$WARN_TEMP" -lt 40 ]] || [[ "$WARN_TEMP" -gt 100 ]]; then
        errors+=("Warning temperature must be between 40-100°C, got: $WARN_TEMP")
        WARN_TEMP="$DEFAULT_WARN_TEMP"
    fi
    
    if [[ "$WARN_TEMP" -le "$HIGH_TEMP" ]]; then
        errors+=("Warning temperature must be higher than high temperature")
        WARN_TEMP="$DEFAULT_WARN_TEMP"
        HIGH_TEMP="$DEFAULT_HIGH_TEMP"
    fi
    
    # Validate CPU governors
    if [[ -z "$CPU_GOVERNOR_AC" ]]; then
        errors+=("AC CPU governor cannot be empty")
        CPU_GOVERNOR_AC="$DEFAULT_CPU_GOVERNOR_AC"
    fi
    
    if [[ -z "$CPU_GOVERNOR_BATTERY" ]]; then
        errors+=("Battery CPU governor cannot be empty")
        CPU_GOVERNOR_BATTERY="$DEFAULT_CPU_GOVERNOR_BATTERY"
    fi
    
    if [[ ${#errors[@]} -gt 0 ]]; then
        for error in "${errors[@]}"; do
            log "WARN" "Configuration validation error: $error"
        done
        write_event "error" "Configuration validation errors" '{"errors": ["'$(IFS='","'; echo "${errors[*]}")'"]}'
    fi
    
    return 0
}

# Get AC adapter status
get_ac_status() {
    if [[ -f "$AC_ADAPTER_PATH/online" ]]; then
        cat "$AC_ADAPTER_PATH/online"
    else
        echo "0"
    fi
}

# Get battery capacity
get_battery_capacity() {
    if [[ -f "$BATTERY_PATH/capacity" ]]; then
        cat "$BATTERY_PATH/capacity"
    else
        echo "0"
    fi
}

# Get current CPU governor
get_cpu_governor() {
    if command -v cpupower >/dev/null 2>&1; then
        cpupower frequency-info -p 2>/dev/null | grep "current policy" | awk '{print $NF}' || echo "unknown"
    else
        echo "unknown"
    fi
}

# Set charge control with retry logic
set_charge_control() {
    local mode="$1"
    local max_attempts=3
    local attempt=1
    local delay=2
    
    while [[ $attempt -le $max_attempts ]]; do
        log "INFO" "Setting charge control to: $mode (attempt $attempt/$max_attempts)"
        
        if ectool chargecontrol "$mode" 2>/dev/null; then
            write_event "hardware_action" "Charge control set to $mode" '{"mode": "'$mode'"}'
            return 0
        fi
        
        log "WARN" "Failed to set charge control to $mode (attempt $attempt/$max_attempts)"
        
        if [[ $attempt -lt $max_attempts ]]; then
            sleep "$delay"
            delay=$((delay * 2))
        fi
        
        ((attempt++))
    done
    
    log "ERROR" "Failed to set charge control to $mode after $max_attempts attempts"
    write_event "error" "Failed to set charge control after $max_attempts attempts" '{"mode": "'$mode'"}'
    return 1
}

# Set CPU governor with retry logic
set_cpu_governor() {
    local governor="$1"
    local max_attempts=3
    local attempt=1
    local delay=2
    
    while [[ $attempt -le $max_attempts ]]; do
        log "INFO" "Setting CPU governor to: $governor (attempt $attempt/$max_attempts)"
        
        if cpupower frequency-set -g "$governor" >/dev/null 2>&1; then
            # Req 3.3: verify the governor took effect on every core, not just
            # cpu0. cpupower sets all cores by default, but confirm each core's
            # scaling_governor actually matches.
            local total=0 matching=0 gov_file cur
            for gov_file in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do
                [[ -r "$gov_file" ]] || continue
                total=$((total + 1))
                cur=$(cat "$gov_file" 2>/dev/null)
                [[ "$cur" == "$governor" ]] && matching=$((matching + 1))
            done
            if [[ "$total" -eq 0 ]]; then
                write_event "hardware_action" "CPU governor set to $governor" '{"governor": "'$governor'", "cores_verified": -1}'
            elif [[ "$matching" -eq "$total" ]]; then
                write_event "hardware_action" "CPU governor set to $governor" '{"governor": "'$governor'", "cores_verified": '$matching'}'
            else
                log "WARN" "Governor $governor verified on $matching/$total cores"
                write_event "error" "Governor verification partial" '{"governor": "'$governor'", "matched": '$matching', "total": '$total'}'
            fi
            return 0
        fi
        
        log "WARN" "Failed to set CPU governor to $governor (attempt $attempt/$max_attempts)"
        
        if [[ $attempt -lt $max_attempts ]]; then
            sleep "$delay"
            delay=$((delay * 2))
        fi
        
        ((attempt++))
    done
    
    log "ERROR" "Failed to set CPU governor to $governor after $max_attempts attempts"
    write_event "error" "Failed to set CPU governor after $max_attempts attempts" '{"governor": "'$governor'"}'
    return 1
}

# Continuation of spinctrl-service.sh

# Write current system status to JSON file
write_status() {
    local capacity
    local ac_status
    local cpu_governor
    local timestamp
    
    capacity=$(get_battery_capacity)
    ac_status=$(get_ac_status)
    cpu_governor=$(get_cpu_governor)
    timestamp=$(date -Iseconds)
    
    if command -v jq >/dev/null 2>&1; then
        local status_json
        status_json=$(jq -n \
            --arg capacity "$capacity" \
            --arg ac_connected "$ac_status" \
            --arg cpu_governor "$cpu_governor" \
            --arg timestamp "$timestamp" \
            --arg service_pid "$$" \
            --arg threshold "$BATTERY_THRESHOLD" \
            --argjson force_charge "$FORCE_CHARGE" \
            '{
                battery: {
                    capacity: ($capacity | tonumber),
                    charging: (($ac_connected == "1" and $force_charge == true) or ($ac_connected == "1" and ($capacity | tonumber) < ($threshold | tonumber))),
                    threshold_active: ($ac_connected == "1" and ($capacity | tonumber) >= ($threshold | tonumber) and $force_charge == false),
                    ac_connected: ($ac_connected == "1")
                },
                power: {
                    ac_connected: ($ac_connected == "1"),
                    cpu_governor: $cpu_governor
                },
                timestamp: $timestamp,
                service_pid: ($service_pid | tonumber)
            }')
        
        # Write atomically using temp file
        echo "$status_json" > "$STATUS_FILE.tmp" && mv "$STATUS_FILE.tmp" "$STATUS_FILE"
        chmod 640 "$STATUS_FILE"
    else
        log "ERROR" "jq not available for status writing"
        return 1
    fi
}

# Monitor battery capacity and handle threshold
monitor_battery() {
    log "INFO" "Starting battery monitoring (threshold: ${BATTERY_THRESHOLD}%)"
    
    while true; do
        # Check if shutdown was requested
        if [[ "$SHUTDOWN_REQUESTED" == "true" ]]; then
            log "INFO" "Battery monitoring stopping due to shutdown request"
            break
        fi
        
        local capacity
        capacity=$(get_battery_capacity)
        
        # Check if force charge is enabled
        if [[ "$FORCE_CHARGE" == "true" ]]; then
            if [[ "$capacity" -ge 100 ]]; then
                log "INFO" "Force charge complete, battery at ${capacity}%"
                FORCE_CHARGE=false
                write_event "hardware_action" "Force charge completed" '{"capacity": '$capacity'}'
            fi
        else
            # Normal threshold-based charging
            if [[ "$capacity" -ge "$BATTERY_THRESHOLD" ]]; then
                log "INFO" "Battery level ${capacity}% reached threshold ${BATTERY_THRESHOLD}%. Stopping charge."
                if set_charge_control "idle"; then
                    break
                fi
            fi
        fi
        
        # Update status
        write_status
        
        sleep 30
    done
    
    log "INFO" "Battery monitoring stopped"
}

# Handle AC adapter plugged in
handle_ac_plugged() {
    log "INFO" "AC adapter plugged in"
    
    # Stop existing battery monitoring if running
    if [[ -n "$MONITOR_PID" ]] && kill -0 "$MONITOR_PID" 2>/dev/null; then
        log "INFO" "Stopping existing battery monitoring"
        kill "$MONITOR_PID"
        wait "$MONITOR_PID" 2>/dev/null
        MONITOR_PID=""
    fi
    
    local capacity
    capacity=$(get_battery_capacity)
    
    # Set CPU to AC performance mode
    set_cpu_governor "$CPU_GOVERNOR_AC"
    
    # Handle charging logic
    if [[ "$FORCE_CHARGE" == "true" ]]; then
        log "INFO" "Force charge enabled, starting normal charging"
        set_charge_control "normal"
        monitor_battery &
        MONITOR_PID=$!
    elif [[ "$capacity" -ge "$BATTERY_THRESHOLD" ]]; then
        log "INFO" "Battery already at ${capacity}%, setting charge control to idle"
        set_charge_control "idle"
    else
        log "INFO" "Battery at ${capacity}%, starting threshold monitoring"
        set_charge_control "normal"
        monitor_battery &
        MONITOR_PID=$!
    fi
    
    write_status
}

# Handle AC adapter unplugged
handle_ac_unplugged() {
    log "INFO" "AC adapter unplugged"
    
    # Stop battery monitoring if running
    if [[ -n "$MONITOR_PID" ]] && kill -0 "$MONITOR_PID" 2>/dev/null; then
        log "INFO" "Stopping battery monitoring"
        kill "$MONITOR_PID"
        wait "$MONITOR_PID" 2>/dev/null
        MONITOR_PID=""
    fi
    
    # Set CPU to battery mode
    set_cpu_governor "$CPU_GOVERNOR_BATTERY"
    
    # Restore normal charge control
    log "INFO" "Restoring normal charge control"
    set_charge_control "normal"
    
    write_status
}

# Set thermal thresholds using ectool
configure_thermal() {
    log "INFO" "Configuring thermal thresholds"

    local warn_kelvin=$((272 + WARN_TEMP))
    local high_kelvin=$((272 + HIGH_TEMP))
    local shutdown_kelvin=$((272 + SHUTDOWN_TEMP))
    local fan_off_kelvin=$((272 + FAN_OFF_TEMP))
    local fan_max_kelvin=$((272 + FAN_MAX_TEMP))

    # Req 6.3: probe EC thermal sensor availability. If the EC is unreachable,
    # keep the conservative built-in defaults already loaded into the
    # WARN_TEMP/HIGH_TEMP/... bare vars and skip thermalset rather than
    # spamming three doomed ectool calls.
    if ! ectool thermalget 0 >/dev/null 2>&1; then
        log "WARN" "EC thermal sensors unavailable; using built-in defaults, skipping thermalset"
        write_event "error" "EC thermal sensors unavailable; using conservative defaults" \
            '{"warn_temp": '$WARN_TEMP', "high_temp": '$HIGH_TEMP', "shutdown_temp": '$SHUTDOWN_TEMP'}'
        return 1
    fi

    local set_values="$warn_kelvin $high_kelvin $shutdown_kelvin $fan_off_kelvin $fan_max_kelvin"
    local verified_zones=0

    for zone in 0 1 2; do
        if ! ectool thermalset "$zone" "$warn_kelvin" "$high_kelvin" "$shutdown_kelvin" "$fan_off_kelvin" "$fan_max_kelvin" 2>/dev/null; then
            log "WARN" "Failed to configure thermal zone $zone"
            continue
        fi

        # Req 6.4: read back the zone and confirm every set threshold is
        # reflected. Format-tolerant: extract all numbers from thermalget
        # and require each set Kelvin value to appear among them.
        local readback nums all_present v
        readback=$(ectool thermalget "$zone" 2>/dev/null) || {
            log "WARN" "Thermal zone $zone: thermalset ok but thermalget failed; verification inconclusive"
            continue
        }
        nums=$(grep -oE '[0-9]+' <<<"$readback" 2>/dev/null)
        all_present=1
        for v in $set_values; do
            grep -qx "$v" <<<"$nums" 2>/dev/null || all_present=0
        done
        if [[ "$all_present" -eq 1 ]]; then
            log "INFO" "Configured and verified thermal zone $zone"
            verified_zones=$((verified_zones + 1))
        else
            log "WARN" "Thermal zone $zone read-back mismatch (expected: $set_values)"
        fi
    done

    write_event "hardware_action" "Thermal thresholds configured" \
        '{"warn_temp": '$WARN_TEMP', "high_temp": '$HIGH_TEMP', "shutdown_temp": '$SHUTDOWN_TEMP', "zones_verified": '$verified_zones'}'
    return 0
}

# Apply CPU frequency limits (min/max) via cpupower
configure_cpu_frequencies() {
    # No-op if both limits are empty
    if [[ -z "$MIN_FREQ_KHZ" ]] && [[ -z "$MAX_FREQ_KHZ" ]]; then
        return 0
    fi

    local max_attempts=3
    local attempt=1
    local delay=2
    local args=()
    [[ -n "$MIN_FREQ_KHZ" ]] && args+=(-d "$MIN_FREQ_KHZ")
    [[ -n "$MAX_FREQ_KHZ" ]] && args+=(-u "$MAX_FREQ_KHZ")

    while [[ $attempt -le $max_attempts ]]; do
        log "INFO" "Setting CPU frequency limits (attempt $attempt/$max_attempts): min=${MIN_FREQ_KHZ:-none}, max=${MAX_FREQ_KHZ:-none}"

        if cpupower frequency-set "${args[@]}" >/dev/null 2>&1; then
            write_event "hardware_action" "CPU frequency limits applied" \
                '{"min_freq_khz": "'"$MIN_FREQ_KHZ"'", "max_freq_khz": "'"$MAX_FREQ_KHZ"'"}'
            return 0
        fi

        log "WARN" "Failed to set CPU frequency limits (attempt $attempt/$max_attempts)"

        if [[ $attempt -lt $max_attempts ]]; then
            sleep "$delay"
            delay=$((delay * 2))
        fi

        ((attempt++))
    done

    log "ERROR" "Failed to set CPU frequency limits after $max_attempts attempts"
    write_event "error" "Failed to set CPU frequency limits after $max_attempts attempts" \
        '{"min_freq_khz": "'"$MIN_FREQ_KHZ"'", "max_freq_khz": "'"$MAX_FREQ_KHZ"'"}'
    return 1
}

# Check initial system state and apply configuration
check_initial_state() {
    log "INFO" "Checking initial system state"
    
    # Load configuration
    if ! load_config; then
        log "ERROR" "Failed to load configuration, using defaults"
    fi

    # On first boot (no config_status yet), seed it from /etc so the TUI has
    # a config to display. After the first apply_config push, config_status is
    # always written by the service and overrides /etc on subsequent boots.
    if [[ ! -f "$CONFIG_STATUS_FILE" && -f "$CONFIG_FILE" ]]; then
        cp "$CONFIG_FILE" "$CONFIG_STATUS_FILE"
        chmod 640 "$CONFIG_STATUS_FILE"
    fi
    
# Configure thermal thresholds
    configure_thermal

    # Apply CPU frequency limits
    configure_cpu_frequencies

    # Check AC adapter state and apply appropriate settings
    local ac_status
    ac_status=$(get_ac_status)
    
    if [[ "$ac_status" == "1" ]]; then
        log "INFO" "AC adapter is initially plugged in"
        handle_ac_plugged
    else
        log "INFO" "AC adapter is initially unplugged"
        set_cpu_governor "$CPU_GOVERNOR_BATTERY"
        set_charge_control "normal"
        write_status
    fi
}

# Process commands from FIFO
process_commands() {
    log "INFO" "Starting command processor"
    
    while true; do
        if [[ "$SHUTDOWN_REQUESTED" == "true" ]]; then
            log "INFO" "Command processor stopping due to shutdown request"
            break
        fi
        
        # Read from FIFO (this will block until data is available)
        if read -r command < "$COMMANDS_FIFO"; then
            log "INFO" "Received command: $command"
            
            case "$command" in
                "battery_threshold:"*)
                    local threshold="${command#battery_threshold:}"
                    if [[ "$threshold" =~ ^[0-9]+$ ]] && [[ "$threshold" -ge 50 ]] && [[ "$threshold" -le 100 ]]; then
                        local old_threshold="$BATTERY_THRESHOLD"
                        BATTERY_THRESHOLD="$threshold"
                        log "INFO" "Updated battery threshold from $old_threshold% to $threshold%"
                        write_event "command_executed" "Battery threshold changed" \
                            '{"old": '$old_threshold', "new": '$threshold'}'
                        write_status
                    else
                        log "ERROR" "Invalid battery threshold: $threshold (must be 50-100)"
                        write_event "error" "Invalid battery threshold command" '{"value": "'$threshold'"}'
                    fi
                    ;;
                    
                "force_charge")
                    FORCE_CHARGE=true
                    log "INFO" "Force charging enabled"
                    write_event "command_executed" "Force charge enabled" '{}'
                    
                    # If AC is connected, start charging immediately
                    if [[ "$(get_ac_status)" == "1" ]]; then
                        set_charge_control "normal"
                        # Restart battery monitoring if not running
                        if [[ -z "$MONITOR_PID" ]] || ! kill -0 "$MONITOR_PID" 2>/dev/null; then
                            monitor_battery &
                            MONITOR_PID=$!
                        fi
                    fi
                    write_status
                    ;;
                    
                "stop_charge")
                    FORCE_CHARGE=false
                    log "INFO" "Force charging disabled"
                    write_event "command_executed" "Force charge disabled" '{}'
                    
                    # If AC is connected and above threshold, stop charging
                    if [[ "$(get_ac_status)" == "1" ]] && [[ "$(get_battery_capacity)" -ge "$BATTERY_THRESHOLD" ]]; then
                        set_charge_control "idle"
                    fi
                    write_status
                    ;;
                    
                "cpu_governor:"*)
                    local governor="${command#cpu_governor:}"
                    if [[ -n "$governor" ]]; then
                        if set_cpu_governor "$governor"; then
                            log "INFO" "CPU governor set to $governor"
                            write_event "command_executed" "CPU governor changed" '{"governor": "'$governor'"}'
                            write_status
                        fi
                    else
                        log "ERROR" "Empty CPU governor command"
                        write_event "error" "Empty CPU governor command" '{}'
                    fi
                    ;;
                    
                "reload_config")
                    log "INFO" "Reloading configuration"
                    if load_config; then
                        configure_thermal
                        write_event "command_executed" "Configuration reloaded" '{}'
                        write_status
                    else
                        log "ERROR" "Failed to reload configuration"
                        write_event "error" "Failed to reload configuration" '{}'
                    fi
                    ;;
                    
                "shutdown")
                    log "INFO" "Shutdown command received"
                    SHUTDOWN_REQUESTED=true
                    write_event "command_executed" "Shutdown requested" '{}'
                    break
                    ;;

                "apply_config:"*)
                    # Full-config push from a TUI client. Applies at runtime
                    # only; the service does NOT persist to /etc (cannot, under
                    # ProtectSystem=strict). /etc remains the boot-time source.
                    local json="${command#apply_config:}"
                    log "INFO" "Applying full config from client"
                    if printf '%s' "$json" | jq -e . >/dev/null 2>&1; then
                        CONFIG[BATTERY_THRESHOLD]=$(printf '%s' "$json" | jq -r '.battery.threshold')
                        CONFIG[FORCE_CHARGE]=$(printf '%s' "$json" | jq -r '.battery.force_charge')
                        CONFIG[CPU_GOVERNOR_AC]=$(printf '%s' "$json" | jq -r '.cpu.governor_ac')
                        CONFIG[CPU_GOVERNOR_BATTERY]=$(printf '%s' "$json" | jq -r '.cpu.governor_battery')
                        CONFIG[MIN_FREQ_KHZ]=$(printf '%s' "$json" | jq -r '(.cpu.min_freq_khz // empty)')
                        CONFIG[MAX_FREQ_KHZ]=$(printf '%s' "$json" | jq -r '(.cpu.max_freq_khz // empty)')
                        CONFIG[WARN_TEMP]=$(printf '%s' "$json" | jq -r '.thermal.warn_temp')
                        CONFIG[HIGH_TEMP]=$(printf '%s' "$json" | jq -r '.thermal.high_temp')
                        CONFIG[SHUTDOWN_TEMP]=$(printf '%s' "$json" | jq -r '.thermal.shutdown_temp')
                        CONFIG[FAN_OFF_TEMP]=$(printf '%s' "$json" | jq -r '.thermal.fan_off_temp')
                        CONFIG[FAN_MAX_TEMP]=$(printf '%s' "$json" | jq -r '.thermal.fan_max_temp')
                        BATTERY_THRESHOLD="${CONFIG[BATTERY_THRESHOLD]}"
                        FORCE_CHARGE="${CONFIG[FORCE_CHARGE]}"
                        CPU_GOVERNOR_AC="${CONFIG[CPU_GOVERNOR_AC]}"
                        CPU_GOVERNOR_BATTERY="${CONFIG[CPU_GOVERNOR_BATTERY]}"
                        WARN_TEMP="${CONFIG[WARN_TEMP]}"
                        HIGH_TEMP="${CONFIG[HIGH_TEMP]}"
                        SHUTDOWN_TEMP="${CONFIG[SHUTDOWN_TEMP]}"
                        FAN_OFF_TEMP="${CONFIG[FAN_OFF_TEMP]}"
                        FAN_MAX_TEMP="${CONFIG[FAN_MAX_TEMP]}"
                        MIN_FREQ_KHZ="${CONFIG[MIN_FREQ_KHZ]}"
                        MAX_FREQ_KHZ="${CONFIG[MAX_FREQ_KHZ]}"
                        if validate_config; then
                            configure_thermal
                            configure_cpu_frequencies
                            local ac_now
                            ac_now=$(get_ac_status)
                            if [[ "$ac_now" == "1" ]]; then
                                set_cpu_governor "$CPU_GOVERNOR_AC"
                            else
                                set_cpu_governor "$CPU_GOVERNOR_BATTERY"
                            fi
                            write_event "command_executed" "Full config applied" \
                                '{"threshold": '$BATTERY_THRESHOLD', "cpu_ac": "'$CPU_GOVERNOR_AC'"}'
                            # Persist the pushed config to config_status.json so it
                            # survives restarts (overrides /etc on next boot) and
                            # the TUI reads it back.
                            printf '%s' "$json" | jq '.' > "$CONFIG_STATUS_FILE.tmp" 2>/dev/null \
                                && mv "$CONFIG_STATUS_FILE.tmp" "$CONFIG_STATUS_FILE" \
                                && chmod 640 "$CONFIG_STATUS_FILE"
                        else
                            log "WARN" "apply_config: validation failed; defaults restored by validate_config"
                            write_event "error" "apply_config validation failed" '{}'
                        fi
                        write_status
                    else
                        log "ERROR" "apply_config: invalid JSON"
                        write_event "error" "Invalid apply_config JSON" '{}'
                    fi
                    ;;

                *)
                    log "WARN" "Unknown command: $command"
                    write_event "error" "Unknown command received" '{"command": "'$command'"}'
                    ;;
            esac
        fi
    done
    
    log "INFO" "Command processor stopped"
}

# Watch configuration file for changes
watch_config() {
    log "INFO" "Starting configuration file watcher"
    
    while true; do
        if [[ "$SHUTDOWN_REQUESTED" == "true" ]]; then
            log "INFO" "Configuration watcher stopping due to shutdown request"
            break
        fi
        
        # Wait for file modification events
        if inotifywait -e modify,move,create "$CONFIG_FILE" 2>/dev/null; then
            log "INFO" "Configuration file changed, reloading..."
            sleep 0.5  # Debounce rapid changes; keep total reload < 1s (req 1.4)
            
            if load_config; then
                configure_thermal
                configure_cpu_frequencies
                # Update current AC state if needed
                local ac_status
                ac_status=$(get_ac_status)
                if [[ "$ac_status" == "1" ]]; then
                    set_cpu_governor "$CPU_GOVERNOR_AC"
                else
                    set_cpu_governor "$CPU_GOVERNOR_BATTERY"
                fi
                write_status
            else
                log "ERROR" "Failed to reload configuration after file change"
            fi
        fi
    done
    
    log "INFO" "Configuration watcher stopped"
}

# Monitor udev events for power supply changes
monitor_udev_events() {
    log "INFO" "Starting udev event monitoring for power supply changes"
    
    udevadm monitor --property --subsystem-match=power_supply | while read -r line; do
        if [[ "$SHUTDOWN_REQUESTED" == "true" ]]; then
            log "INFO" "Udev monitor stopping due to shutdown request"
            break
        fi
        
        if [[ "$line" == *"POWER_SUPPLY_NAME=AC"* ]] || [[ "$line" == *"DEVNAME=/dev/ADP1"* ]]; then
            sleep 1  # Allow hardware state to settle
            local ac_status
            ac_status=$(get_ac_status)
            
            if [[ "$ac_status" == "1" ]]; then
                handle_ac_plugged
            else
                handle_ac_unplugged
            fi
        fi
    done
}

# Cleanup function for graceful shutdown
cleanup() {
    log "INFO" "Received signal, cleaning up..."
    SHUTDOWN_REQUESTED=true
    
    # Stop background processes
    for pid in "$MONITOR_PID" "$CONFIG_WATCHER_PID" "$COMMAND_PROCESSOR_PID"; do
        if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
            log "INFO" "Stopping background process: $pid"
            kill "$pid"
            wait "$pid" 2>/dev/null
        fi
    done
    
    # Restore hardware to safe defaults
    log "INFO" "Restoring hardware defaults"
    set_charge_control "normal" || log "WARN" "Failed to restore normal charge control"
    
    # Clean up IPC files
    log "INFO" "Cleaning up IPC files"
    rm -f "$STATUS_FILE" "$STATUS_FILE.tmp"
    [[ -p "$COMMANDS_FIFO" ]] && rm -f "$COMMANDS_FIFO"
    
    write_event "service_stop" "SpinCtrl service stopped" '{}'
    
    log "INFO" "Cleanup complete, exiting"
    exit 0
}

# Status update loop (runs in background)
status_update_loop() {
    while true; do
        if [[ "$SHUTDOWN_REQUESTED" == "true" ]]; then
            break
        fi
        
        write_status
        sleep 30
    done
}

# Main function
main() {
    log "INFO" "Starting $SCRIPT_NAME service (PID: $$)"
    
    # Check dependencies
    if ! check_dependencies; then
        exit 1
    fi
    
    if [[ "$DRY_RUN" != "1" ]]; then
        if [[ ! -d "$AC_ADAPTER_PATH" ]]; then
            log "ERROR" "AC adapter path not found: $AC_ADAPTER_PATH"
            exit 1
        fi
        if [[ ! -d "$BATTERY_PATH" ]]; then
            log "ERROR" "Battery path not found: $BATTERY_PATH"
            exit 1
        fi
    fi
    
    # Initialize IPC
    if ! init_ipc; then
        log "ERROR" "Failed to initialize IPC"
        exit 1
    fi
    
    # Set up signal handlers
    trap cleanup SIGTERM SIGINT
    
    # Check initial state and apply configuration
    check_initial_state
    
    # Start background processes
    log "INFO" "Starting background processes"
    
    # Configuration file watcher
    watch_config &
    CONFIG_WATCHER_PID=$!
    
    # Command processor
    process_commands &
    COMMAND_PROCESSOR_PID=$!
    
    # Status update loop
    status_update_loop &
    STATUS_UPDATE_PID=$!
    
    # Main event loop (udev monitoring)
    monitor_udev_events
    
    # If we reach here, something went wrong or shutdown was requested
    cleanup
}

# Entry point
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    DRY_RUN=0
    for arg in "$@"; do
        [[ "$arg" == "--dry-run" ]] && DRY_RUN=1
    done
    if [[ "$DRY_RUN" == "1" ]]; then
        echo "[spinctrl-service] DRY-RUN mode: hardware calls mocked, no changes to hardware."
        ectool() { echo "[dry-run] ectool $*"; case "$1" in thermalget) echo "343 333 353 323 348"; return 0 ;; *) return 0 ;; esac; }
        cpupower() { echo "[dry-run] cpupower $*"; return 0; }
        get_battery_capacity() { echo "75"; }
        get_ac_status() { echo "1"; }
        get_cpu_governor() { echo "performance"; }
        chmod() { command chmod a+rwx "${@:2}" 2>/dev/null || true; }
        check_dependencies() { command -v jq >/dev/null 2>&1 || { echo "ERROR: jq required for dry-run"; return 1; }; return 0; }
        watch_config() { :; }
        monitor_udev_events() { echo "[spinctrl-service] Dry-run: waiting for commands (Ctrl+C to stop)"; while true; do sleep 1; done; }
    fi
    main "$@"
fi