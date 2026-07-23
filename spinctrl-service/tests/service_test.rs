use spinctrl_service::command_processor::parse_command;
use spinctrl_service::command_processor::ParsedCommand;
use spinctrl_service::hardware::{ChargeMode, HardwareBackend, MockBackend};
use shared::Config;

#[test]
fn test_command_parsing_all_variants() {
    assert_eq!(
        parse_command("force_charge"),
        Some(ParsedCommand::ForceCharge)
    );
    assert_eq!(
        parse_command("stop_charge"),
        Some(ParsedCommand::StopCharge)
    );
    assert_eq!(
        parse_command("reload_config"),
        Some(ParsedCommand::ReloadConfig)
    );
    assert_eq!(parse_command("shutdown"), Some(ParsedCommand::Shutdown));
    assert_eq!(
        parse_command("apply_config:{\"battery\":{\"threshold\":85}}"),
        Some(ParsedCommand::ApplyConfig(
            "{\"battery\":{\"threshold\":85}}".to_string()
        ))
    );
    assert_eq!(parse_command("unknown_cmd"), None);
    assert_eq!(parse_command(""), None);
}

#[test]
fn test_mock_backend_charge_control() {
    let mut mock = MockBackend::new();
    assert!(mock.charge_control_calls.is_empty());

    mock.set_charge_control(ChargeMode::Normal).unwrap();
    assert_eq!(mock.charge_control_calls, vec![ChargeMode::Normal]);

    mock.set_charge_control(ChargeMode::Idle).unwrap();
    assert_eq!(
        mock.charge_control_calls,
        vec![ChargeMode::Normal, ChargeMode::Idle]
    );
}

#[test]
fn test_mock_backend_governor() {
    let mut mock = MockBackend::new();
    let gov = shared::CpuGovernor::Standard(shared::StandardGovernor::Powersave);
    mock.set_cpu_governor(&gov).unwrap();
    assert_eq!(mock.governor_calls, vec!["powersave"]);
    assert_eq!(mock.cpu_governor, "powersave");
}

#[test]
fn test_mock_backend_thermal() {
    let mut mock = MockBackend::new();
    let config = shared::ThermalConfig::default();
    mock.configure_thermal(&config).unwrap();
    assert_eq!(mock.thermal_config_calls.len(), 1);
    assert_eq!(mock.thermal_config_calls[0], config);
}

#[test]
fn test_mock_backend_ac_status() {
    let mut mock = MockBackend::new();
    mock.ac_connected = true;
    assert!(mock.get_ac_status().unwrap());

    mock.ac_connected = false;
    assert!(!mock.get_ac_status().unwrap());
}

#[test]
fn test_mock_backend_battery_capacity() {
    let mut mock = MockBackend::new();
    mock.battery_capacity = 42;
    assert_eq!(mock.get_battery_capacity().unwrap(), 42);
}

#[test]
fn test_apply_config_invalid_json() {
    // Config::from_json rejects invalid JSON
    let result = Config::from_json("not valid json");
    assert!(result.is_err());
}

#[test]
fn test_apply_config_shutdown_temp_out_of_range() {
    let json = r#"{
        "battery": {"threshold": 80, "force_charge": false},
        "cpu": {"governor_ac": "performance", "governor_battery": "powersave"},
        "thermal": {"warn_temp": 70, "high_temp": 55, "shutdown_temp": 200, "fan_off_temp": 50, "fan_max_temp": 75, "profile": "balanced"}
    }"#;
    let result = Config::from_json(json);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Shutdown temperature must be between 50-110"));
}

#[test]
fn test_apply_config_valid_json() {
    let json = r#"{
        "battery": {"threshold": 80, "force_charge": false},
        "cpu": {"governor_ac": "performance", "governor_battery": "powersave"},
        "thermal": {"warn_temp": 70, "high_temp": 55, "shutdown_temp": 80, "fan_off_temp": 50, "fan_max_temp": 75, "profile": "balanced"}
    }"#;
    let config = Config::from_json(json).unwrap();
    assert_eq!(config.battery.threshold, 80);
    assert_eq!(config.thermal.shutdown_temp, 80);
}

#[test]
fn test_mock_backend_frequencies() {
    let mut mock = MockBackend::new();
    mock.configure_cpu_frequencies(Some(1_000_000), Some(2_000_000))
        .unwrap();
    assert_eq!(mock.freq_calls, vec![(Some(1_000_000), Some(2_000_000))]);
}

#[test]
fn test_mock_backend_thermal_zones() {
    let mock = MockBackend::new();
    let zones = mock.get_thermal_zones().unwrap();
    assert!(zones.is_empty());
}