#!/bin/bash
# SpinCtrl bash service test cases. Sourced by run.sh AFTER the service
# script is sourced (functions in scope) and AFTER log/write_event are
# overridden to no-ops so tests are quiet and don't touch /var/lib.

test_validate_config_valid() {
    BATTERY_THRESHOLD=80; WARN_TEMP=70; HIGH_TEMP=55; SHUTDOWN_TEMP=80
    FAN_OFF_TEMP=50; FAN_MAX_TEMP=75
    CPU_GOVERNOR_AC="performance"; CPU_GOVERNOR_BATTERY="powersave"
    validate_config
    assert_eq "$BATTERY_THRESHOLD" "80" "valid config: threshold unchanged"
    assert_eq "$WARN_TEMP" "70" "valid config: warn unchanged"
    assert_eq "$CPU_GOVERNOR_AC" "performance" "valid config: gov_ac unchanged"
}

test_validate_config_invalid_threshold() {
    BATTERY_THRESHOLD=30
    validate_config
    assert_eq "$BATTERY_THRESHOLD" "$DEFAULT_BATTERY_THRESHOLD" "threshold<50 resets to default"
    BATTERY_THRESHOLD=150
    validate_config
    assert_eq "$BATTERY_THRESHOLD" "$DEFAULT_BATTERY_THRESHOLD" "threshold>100 resets to default"
}

test_validate_config_empty_governor() {
    CPU_GOVERNOR_AC=""
    validate_config
    assert_eq "$CPU_GOVERNOR_AC" "$DEFAULT_CPU_GOVERNOR_AC" "empty gov_ac resets"
    CPU_GOVERNOR_BATTERY=""
    validate_config
    assert_eq "$CPU_GOVERNOR_BATTERY" "$DEFAULT_CPU_GOVERNOR_BATTERY" "empty gov_battery resets"
}

test_validate_config_warn_le_high() {
    WARN_TEMP=50; HIGH_TEMP=60
    validate_config
    assert_eq "$WARN_TEMP" "$DEFAULT_WARN_TEMP" "warn<=high: warn resets"
    assert_eq "$HIGH_TEMP" "$DEFAULT_HIGH_TEMP" "warn<=high: high resets"
}

test_load_config_parses_json() {
    if ! command -v jq >/dev/null 2>&1; then
        echo "  SKIP  (jq not available in this environment; load_config requires it)"
        return
    fi
    local tmp_dir
    tmp_dir=$(mktemp -d)
    cat > "$tmp_dir/config.json" << 'JSON'
{
  "battery": {"threshold": 85, "force_charge": false},
  "cpu": {"governor_ac": "ondemand", "governor_battery": "powersave"},
  "thermal": {"warn_temp": 65, "high_temp": 50, "shutdown_temp": 75, "fan_off_temp": 45, "fan_max_temp": 70, "profile": "conservative"},
  "version": 1
}
JSON
    CONFIG_FILE="$tmp_dir/config.json"
    load_config || true
    assert_eq "${CONFIG[BATTERY_THRESHOLD]}" "85" "load: CONFIG threshold==85"
    assert_eq "$BATTERY_THRESHOLD" "85" "load: bare BATTERY_THRESHOLD==85 (sync works)"
    assert_eq "${CONFIG[CPU_GOVERNOR_AC]}" "ondemand" "load: CONFIG gov_ac==ondemand"
    assert_eq "$CPU_GOVERNOR_AC" "ondemand" "load: bare gov_ac==ondemand"
    assert_eq "${CONFIG[WARN_TEMP]}" "65" "load: CONFIG warn==65"
    assert_eq "$WARN_TEMP" "65" "load: bare warn==65"
    rm -rf "$tmp_dir"
}

test_thermal_readback_all_present() {
    local readback="warn: 343K high: 333K shutdown: 353K fan_off: 323K fan_max: 348K"
    if check_thermal_readback "$readback" 343 333 353 323 348; then
        echo "  PASS  thermal readback: all 5 values detected"
    else
        echo "  FAIL  thermal readback: all-present check failed"
        _TEST_FAILED=1
    fi
}

test_thermal_readback_missing_one() {
    local readback="warn: 343K high: 333K shutdown: 353K fan_off: 323K fan_max: 340K"
    if check_thermal_readback "$readback" 343 333 353 323 348; then
        echo "  FAIL  thermal readback: should have detected missing 348"
        _TEST_FAILED=1
    else
        echo "  PASS  thermal readback: correctly detected missing value"
    fi
}
