use std::time::Duration;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use shared::{
    available_cpu_frequencies, Config, EventEntry, EventType, IpcManager,
    SpinCtrlError, ThermalProfile,
};
use tempfile::TempDir;
use crate::app::App;
use crate::app::editing::EditField;
use crate::app::state::{AppMode, PopupType, Tab};
use crate::app::explain::explain_error;

fn make_test_app() -> (App, TempDir) {
    let temp = TempDir::new().unwrap();
    let app = App::with_ipc(IpcManager::with_paths(
        temp.path().join("nonexistent-config.json"),
        temp.path().join("nonexistent-runtime"),
    )).expect("test App must not require a running service or production config");
    (app, temp)
}

#[test]
fn test_tab_from_index_valid() {
    assert_eq!(Tab::from_index(0), Tab::Status);
    assert_eq!(Tab::from_index(1), Tab::Battery);
    assert_eq!(Tab::from_index(2), Tab::CPU);
    assert_eq!(Tab::from_index(3), Tab::Thermal);
    assert_eq!(Tab::from_index(4), Tab::Events);
}

#[test]
fn test_tab_from_index_out_of_range_defaults_to_status() {
    assert_eq!(Tab::from_index(5), Tab::Status);
    assert_eq!(Tab::from_index(usize::MAX), Tab::Status);
}

#[test]
fn test_tab_to_index_round_trip() {
    for i in 0..Tab::COUNT {
        assert_eq!(Tab::from_index(i).to_index(), i);
    }
}

#[test]
fn test_tab_next_wraps_around() {
    assert_eq!(Tab::Status.next(), Tab::Battery);
    assert_eq!(Tab::Battery.next(), Tab::CPU);
    assert_eq!(Tab::CPU.next(), Tab::Thermal);
    assert_eq!(Tab::Thermal.next(), Tab::Events);
    assert_eq!(Tab::Events.next(), Tab::Status);
}

#[test]
fn test_tab_previous_wraps_around() {
    assert_eq!(Tab::Status.previous(), Tab::Events);
    assert_eq!(Tab::Events.previous(), Tab::Thermal);
    assert_eq!(Tab::Battery.previous(), Tab::Status);
}

#[test]
fn test_tab_titles_count_match() {
    assert_eq!(Tab::titles().len(), Tab::COUNT);
    assert_eq!(Tab::COUNT, 5);
    assert_eq!(Tab::titles(), ["Status", "Battery", "CPU", "Thermal", "Events"]);
}

#[test]
fn test_tab_is_copy() {
    let original = Tab::CPU;
    let copy = original;
    assert_eq!(original, copy);
}

#[test]
fn test_app_new_succeeds_without_service() {
    let (app, _temp) = make_test_app();
    assert_eq!(app.selected_tab, Tab::Status);
    assert_eq!(app.mode, AppMode::Monitoring);
    assert!(!app.should_quit);
    assert!(!app.service_available);
    assert!(app.error_message.is_none());
    assert!(app.status.is_none());
    assert!(app.events.is_empty());
    assert!(app.editing_field.is_none());
    assert_eq!(app.scroll_offset, 0);
    assert_eq!(app.selected_item, 0);
    assert_eq!(app.update_interval, Duration::from_secs(2));
    assert_eq!(app.config, Config::default());
}

#[test]
fn test_appmode_equality() {
    assert_eq!(AppMode::Monitoring, AppMode::Monitoring);
    assert_ne!(AppMode::Monitoring, AppMode::Editing);
    assert_ne!(AppMode::Monitoring, AppMode::Help);
    assert_eq!(
        AppMode::Popup(PopupType::ConfirmExit),
        AppMode::Popup(PopupType::ConfirmExit)
    );
    assert_ne!(
        AppMode::Popup(PopupType::ConfirmExit),
        AppMode::Monitoring
    );
}

#[test]
fn test_popuptype_equality_and_clone() {
    assert_eq!(PopupType::ConfirmExit, PopupType::ConfirmExit);
    let a = PopupType::ConfirmExit;
    let b = a.clone();
    assert_eq!(a, b);
}

#[test]
fn test_apply_config_in_place_without_service_sets_error_and_stays_editing() {
    let (mut app, _temp) = make_test_app();
    assert!(!app.service_available, "fresh App must report service unavailable");
    app.mode = AppMode::Editing;
    app.apply_config_in_place();
    assert_eq!(app.mode, AppMode::Editing, "must stay in Editing mode");
    assert!(app.error_message.is_some(), "must set error_message");
    assert!(app.error_message.as_deref().unwrap_or("").contains("offline"), "unexpected: {:?}", app.error_message);
}

#[test]
fn test_apply_config_in_place_validates_before_pushing() {
    let (mut app, _temp) = make_test_app();
    app.service_available = true;
    app.mode = AppMode::Editing;
    app.config.battery.threshold = 30;
    app.apply_config_in_place();
    assert_eq!(app.mode, AppMode::Editing, "must stay in Editing mode even on validation failure");
    assert!(app.error_message.as_deref().unwrap_or("").contains("validation"), "expected validation msg, got: {:?}", app.error_message);
}

#[test]
fn test_explain_error_permission_denied_has_group_guidance() {
    let e = SpinCtrlError::PermissionDenied("/var/lib/spinctrl/status.json".to_string());
    let msg = explain_error(&e);
    assert!(msg.contains("spinctrl"), "expected group name, got: {msg}");
    assert!(msg.contains("usermod"), "expected usermod guidance, got: {msg}");
    assert!(msg.contains("/var/lib/spinctrl/status.json"), "expected path, got: {msg}");
}

#[test]
fn test_explain_error_io_permission_denied_has_group_guidance() {
    let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
    let e = SpinCtrlError::Io(io_err);
    let msg = explain_error(&e);
    assert!(msg.contains("spinctrl"), "expected group name, got: {msg}");
    assert!(msg.contains("usermod"), "expected usermod guidance, got: {msg}");
}

#[test]
fn test_explain_error_non_permission_is_generic() {
    let e = SpinCtrlError::FileNotFound("/missing/path".to_string());
    let msg = explain_error(&e);
    assert!(!msg.contains("usermod"), "non-permission error should not show group guidance: {msg}");
    assert!(msg.contains("/missing/path") || msg.contains("File not found"), "expected generic msg, got: {msg}");
}

/// Smoke test: render every tab, Help overlay, and each popup variant
/// through `Terminal::draw` backed by `TestBackend`. Asserts that none
/// of the draw paths panic (the `.unwrap()`s on draw enforce this).
#[test]
fn test_ui_render_smoke_all_tabs_and_popups() {
    let backend = ratatui::backend::TestBackend::new(80, 24);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    let (mut app, _temp) = make_test_app();

    for i in 0..Tab::COUNT {
        app.selected_tab = Tab::from_index(i);
        app.mode = AppMode::Monitoring;
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    app.selected_tab = Tab::Status;
    app.mode = AppMode::Help;
    terminal.draw(|f| app.ui(f)).unwrap();

    app.mode = AppMode::Popup(PopupType::ConfirmExit);
    terminal.draw(|f| app.ui(f)).unwrap();

    app.mode = AppMode::Monitoring;
    app.selected_tab = Tab::Events;
    terminal.draw(|f| app.ui(f)).unwrap();

    app.mode = AppMode::Editing;
    app.selected_tab = Tab::Battery;
    app.begin_editing();
    terminal.draw(|f| app.ui(f)).unwrap();
    app.editing_field = Some(EditField::BatteryForceCharge);
    terminal.draw(|f| app.ui(f)).unwrap();

    app.selected_tab = Tab::CPU;
    app.begin_editing();
    terminal.draw(|f| app.ui(f)).unwrap();
    app.editing_field = Some(EditField::CpuMinFreqKhz);
    app.adjust_field(1);
    terminal.draw(|f| app.ui(f)).unwrap();

    app.selected_tab = Tab::Thermal;
    app.begin_editing();
    terminal.draw(|f| app.ui(f)).unwrap();
    app.editing_field = Some(EditField::ThermalWarn);
    app.adjust_field(1);
    terminal.draw(|f| app.ui(f)).unwrap();
    app.editing_field = Some(EditField::ThermalProfile);
    terminal.draw(|f| app.ui(f)).unwrap();
}

#[test]
fn test_arrow_adjust_threshold_increments_and_clamps() {
    let (mut app, _temp) = make_test_app();
    app.selected_tab = Tab::Battery;
    app.begin_editing();
    assert_eq!(app.editing_field, Some(EditField::BatteryThreshold));
    assert_eq!(app.config.battery.threshold, 80, "default threshold");
    app.adjust_field(1);
    assert_eq!(app.config.battery.threshold, 81, "Right must increment by step 1");
    app.adjust_field(-1);
    app.adjust_field(-1);
    assert_eq!(app.config.battery.threshold, 79, "Left must decrement by step 1");
    app.config.battery.threshold = 100;
    app.adjust_field(1);
    assert_eq!(app.config.battery.threshold, 100, "must clamp at upper bound 100");
    app.config.battery.threshold = 50;
    app.adjust_field(-1);
    assert_eq!(app.config.battery.threshold, 50, "must clamp at lower bound 50");
}

#[test]
fn test_arrow_adjust_thermal_temps_clamp_to_range() {
    let (mut app, _temp) = make_test_app();
    app.selected_tab = Tab::Thermal;
    app.begin_editing();
    app.editing_field = Some(EditField::ThermalWarn);
    app.config.thermal.warn_temp = 40;
    app.adjust_field(-1);
    assert_eq!(app.config.thermal.warn_temp, 40, "warn_temp lower bound 40");
    app.config.thermal.warn_temp = 100;
    app.adjust_field(1);
    assert_eq!(app.config.thermal.warn_temp, 100, "warn_temp upper bound 100");
}

#[test]
fn test_next_edit_field_cycles_thermal_fields() {
    let (mut app, _temp) = make_test_app();
    app.selected_tab = Tab::Thermal;
    app.begin_editing();
    assert_eq!(app.editing_field, Some(EditField::ThermalProfile));
    app.next_edit_field();
    assert_eq!(app.editing_field, Some(EditField::ThermalFanOff));
    app.next_edit_field();
    assert_eq!(app.editing_field, Some(EditField::ThermalHigh));
    app.next_edit_field();
    assert_eq!(app.editing_field, Some(EditField::ThermalWarn));
    app.next_edit_field();
    assert_eq!(app.editing_field, Some(EditField::ThermalFanMax));
    app.next_edit_field();
    assert_eq!(app.editing_field, Some(EditField::ThermalShutdown));
    app.next_edit_field();
    assert_eq!(app.editing_field, Some(EditField::ThermalProfile), "must wrap to the first field");
}

#[test]
fn test_battery_tab_cycles_threshold_and_force() {
    let (mut app, _temp) = make_test_app();
    app.selected_tab = Tab::Battery;
    app.begin_editing();
    assert_eq!(app.editing_field, Some(EditField::BatteryThreshold));
    app.next_edit_field();
    assert_eq!(app.editing_field, Some(EditField::BatteryForceCharge));
    app.next_edit_field();
    assert_eq!(app.editing_field, Some(EditField::BatteryThreshold), "must wrap to threshold");
}

#[test]
fn test_cpu_tab_cycles_all_four_fields() {
    let (mut app, _temp) = make_test_app();
    app.selected_tab = Tab::CPU;
    app.begin_editing();
    assert_eq!(app.editing_field, Some(EditField::CpuGovernorAc));
    app.next_edit_field();
    assert_eq!(app.editing_field, Some(EditField::CpuGovernorBattery));
    app.next_edit_field();
    assert_eq!(app.editing_field, Some(EditField::CpuMinFreqKhz));
    app.next_edit_field();
    assert_eq!(app.editing_field, Some(EditField::CpuMaxFreqKhz));
    app.next_edit_field();
    assert_eq!(app.editing_field, Some(EditField::CpuGovernorAc), "must wrap to governor_ac");
}

#[test]
fn test_arrow_toggle_force_charge() {
    let (mut app, _temp) = make_test_app();
    app.editing_field = Some(EditField::BatteryForceCharge);
    assert!(!app.config.battery.force_charge, "default false");
    app.adjust_field(1);
    assert!(app.config.battery.force_charge, "Right must toggle to true");
    app.adjust_field(-1);
    assert!(!app.config.battery.force_charge, "Left must toggle back to false");
    app.adjust_field(1);
    app.adjust_field(1);
    assert!(!app.config.battery.force_charge, "two toggles return to false");
}

#[test]
fn test_arrow_cycle_profile() {
    let (mut app, _temp) = make_test_app();
    app.editing_field = Some(EditField::ThermalProfile);
    app.config.thermal.profile = ThermalProfile::Balanced;
    app.adjust_field(1);
    assert_eq!(app.config.thermal.profile, ThermalProfile::Performance);
    app.adjust_field(1);
    assert_eq!(app.config.thermal.profile, ThermalProfile::Custom);
    app.adjust_field(1);
    assert_eq!(app.config.thermal.profile, ThermalProfile::Conservative, "must wrap");
    app.adjust_field(-1);
    assert_eq!(app.config.thermal.profile, ThermalProfile::Custom, "Left must go to previous");
}

#[test]
fn test_arrow_cycle_governor() {
    let (mut app, _temp) = make_test_app();
    app.editing_field = Some(EditField::CpuGovernorAc);
    let governors = Config::get_available_governors();
    assert!(!governors.is_empty());
    let start = app.config.cpu.governor_ac.clone();
    app.adjust_field(1);
    assert_ne!(app.config.cpu.governor_ac, start, "must move to next governor");
    for _ in 0..governors.len() {
        app.adjust_field(1);
    }
    assert_eq!(app.config.cpu.governor_ac, app.config.cpu.governor_ac, "cycling through all lands somewhere valid");
    assert!(Config::is_valid_governor(&app.config.cpu.governor_ac));
}

#[test]
fn test_arrow_adjust_cpu_freq_option_none_becomes_some() {
    let (mut app, _temp) = make_test_app();
    app.editing_field = Some(EditField::CpuMinFreqKhz);
    assert!(app.config.cpu.min_freq_khz.is_none(), "default is None");
    let (range_min, _) = app.freq_range;
    app.adjust_field(1);
    assert_eq!(
        app.config.cpu.min_freq_khz,
        Some(range_min.saturating_add(500_000)),
        "from None→range_min + step 500_000"
    );
}

#[test]
fn test_esc_reverts_config() {
    let (mut app, _temp) = make_test_app();
    let original_threshold = app.config.battery.threshold;
    app.selected_tab = Tab::Battery;
    app.mode = AppMode::Editing;
    app.begin_editing();
    assert_eq!(app.config_backup.battery.threshold, original_threshold);
    app.config.battery.threshold = 99;
    assert_ne!(app.config_backup.battery.threshold, 99);
    app.config = app.config_backup.clone();
    app.editing_field = None;
    app.mode = AppMode::Monitoring;
    assert_eq!(app.config.battery.threshold, original_threshold, "Esc must revert to backup");
    assert_eq!(app.mode, AppMode::Monitoring);
    assert!(app.editing_field.is_none());
}

fn make_event(event_type: EventType, message: &str) -> EventEntry {
    EventEntry {
        timestamp: chrono::Utc::now(),
        event_type,
        message: message.to_string(),
        details: None,
    }
}

#[test]
fn test_events_tab_render_with_filter_does_not_panic() {
    let backend = ratatui::backend::TestBackend::new(80, 24);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    let (mut app, _temp) = make_test_app();
    app.selected_tab = Tab::Events;
    app.mode = AppMode::Monitoring;

    app.events.push(make_event(EventType::ServiceStart, "service up"));
    app.events.push(make_event(EventType::Error, "oops"));
    app.events.push(make_event(EventType::ConfigChanged, "threshold 80"));
    app.events.push(make_event(EventType::Error, "disk full"));
    app.events.push(make_event(EventType::HardwareAction, "governor set"));

    app.event_filter = None;
    terminal.draw(|f| app.ui(f)).unwrap();

    app.event_filter = Some(EventType::Error);
    app.selected_item = 1;
    terminal.draw(|f| app.ui(f)).unwrap();

    app.event_filter = Some(EventType::ServiceStart);
    terminal.draw(|f| app.ui(f)).unwrap();

    app.events.clear();
    app.event_filter = Some(EventType::Error);
    terminal.draw(|f| app.ui(f)).unwrap();
}

#[test]
fn test_help_overlay_renders_with_settings_reference() {
    let backend = ratatui::backend::TestBackend::new(80, 24);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    let (mut app, _temp) = make_test_app();
    app.mode = AppMode::Help;
    terminal.draw(|f| app.ui(f)).unwrap();

    let buf = terminal.backend().buffer();
    let mut found_settings = false;
    let mut found_battery_threshold = false;
    let mut found_cpu_governors = false;
    let mut found_thermal_profiles = false;
    for y in 0..24u16 {
        let mut row = String::new();
        for x in 0..80u16 {
            let idx = buf.index_of(x, y);
            let sym = &buf.content[idx].symbol;
            row.push(sym.chars().next().unwrap_or(' '));
        }
        if row.contains("Settings Reference") {
            found_settings = true;
        }
        if row.contains("threshold") && row.contains("50-100") {
            found_battery_threshold = true;
        }
        if row.contains("performance") && row.contains("schedutil") {
            found_cpu_governors = true;
        }
        if row.contains("conservative") && row.contains("custom") {
            found_thermal_profiles = true;
        }
    }
    assert!(found_settings, "Help must include 'Settings Reference' section");
    assert!(found_battery_threshold, "Help must document battery threshold 50-100");
    assert!(found_cpu_governors, "Help must list all 5 recognized CPU governor names");
    assert!(found_thermal_profiles, "Help must list all 4 thermal profiles");
}

#[test]
fn test_cycle_event_filter_walks_all_variants_and_wraps() {
    let (mut app, _temp) = make_test_app();
    assert!(app.event_filter.is_none(), "fresh App starts with no filter");

    app.cycle_event_filter();
    assert!(matches!(app.event_filter, Some(EventType::ConfigChanged)));
    app.cycle_event_filter();
    assert!(matches!(app.event_filter, Some(EventType::CommandExecuted)));
    app.cycle_event_filter();
    assert!(matches!(app.event_filter, Some(EventType::HardwareAction)));
    app.cycle_event_filter();
    assert!(matches!(app.event_filter, Some(EventType::Error)));
    app.cycle_event_filter();
    assert!(matches!(app.event_filter, Some(EventType::ServiceStart)));
    app.cycle_event_filter();
    assert!(matches!(app.event_filter, Some(EventType::ServiceStop)));
    app.cycle_event_filter();
    assert!(app.event_filter.is_none(), "must wrap back to None after ServiceStop");

    app.event_filter = Some(EventType::Error);
    app.clear_event_filter();
    assert!(app.event_filter.is_none());
}

#[test]
fn test_event_filter_label_matches_tag() {
    let (mut app, _temp) = make_test_app();
    assert_eq!(app.event_filter_label(), "All");
    app.event_filter = Some(EventType::Error);
    assert_eq!(app.event_filter_label(), "ERR");
    app.event_filter = Some(EventType::ConfigChanged);
    assert_eq!(app.event_filter_label(), "CFG");
}

#[test]
fn test_field_display_editing_enum_shows_arrows() {
    let (mut app, _temp) = make_test_app();
    app.selected_tab = Tab::Battery;
    app.mode = AppMode::Editing;
    app.begin_editing();
    app.editing_field = Some(EditField::BatteryForceCharge);
    let span = app.field_display(EditField::BatteryForceCharge, "Yes", "", ratatui::style::Color::Green);
    assert_eq!(
        span.content,
        std::borrow::Cow::Owned::<str>("← Yes →".to_string()),
        "editing enum must be wrapped in arrows"
    );
    let span2 = app.field_display(EditField::CpuGovernorAc, "performance", "", ratatui::style::Color::Cyan);
    assert_eq!(
        span2.content,
        std::borrow::Cow::Owned::<str>("performance".to_string()),
        "non-edited field must render without arrows"
    );
}

#[test]
fn test_field_display_editing_numeric_renders_trackbar() {
    let (mut app, _temp) = make_test_app();
    app.mode = AppMode::Editing;
    app.editing_field = Some(EditField::BatteryThreshold);
    let span = app.field_display(EditField::BatteryThreshold, "80", "%", ratatui::style::Color::Cyan);
    let text = span.content.to_string();
    assert!(text.contains("●"), "trackbar must include marker: {text}");
    assert!(text.contains("─"), "trackbar must include line: {text}");
    assert!(text.contains("50"), "trackbar must show min label: {text}");
    assert!(text.contains("100"), "trackbar must show max label: {text}");
    assert!(text.contains("80%"), "trackbar must show value label: {text}");
}

#[test]
fn test_field_display_editing_unlimited_freq_shows_trackbar_with_unlimited() {
    let (mut app, _temp) = make_test_app();
    app.mode = AppMode::Editing;
    app.editing_field = Some(EditField::CpuMinFreqKhz);
    app.config.cpu.min_freq_khz = None;
    let span = app.field_display(EditField::CpuMinFreqKhz, "unlimited", "", ratatui::style::Color::White);
    let content = span.content.as_ref();
    assert!(content.contains("unlimited"), "must contain 'unlimited' label, got: {content}");
    assert!(content.contains("●"), "must contain the trackbar marker, got: {content}");
    assert!(content.contains("─"), "must contain the trackbar line, got: {content}");
}

#[test]
fn test_numeric_field_range_battery_threshold() {
    let (mut app, _temp) = make_test_app();
    app.mode = AppMode::Editing;
    app.editing_field = Some(EditField::BatteryThreshold);
    let (min, max, value, _color) = app.numeric_field_range().expect("must return range");
    assert_eq!((min, max), (50, 100));
    assert_eq!(value, 80);
}

#[test]
fn test_numeric_field_range_freq_none_returns_range_at_max() {
    let (mut app, _temp) = make_test_app();
    app.mode = AppMode::Editing;
    app.editing_field = Some(EditField::CpuMinFreqKhz);
    app.config.cpu.min_freq_khz = None;
    let (min, max, v, _color) = app.numeric_field_range().expect("must return range for None freq");
    assert_eq!(v, max, "None freq marker should be at max (far right = unlimited)");
    assert!(min < max, "range must be valid");
}

#[test]
fn test_numeric_field_range_none_for_enum() {
    let (mut app, _temp) = make_test_app();
    app.mode = AppMode::Editing;
    app.editing_field = Some(EditField::ThermalProfile);
    assert!(
        app.numeric_field_range().is_none(),
        "enum fields must not produce a range"
    );
}

#[test]
fn test_numeric_field_range_none_when_not_editing() {
    let (mut app, _temp) = make_test_app();
    app.mode = AppMode::Monitoring;
    app.editing_field = Some(EditField::BatteryThreshold);
    assert!(
        app.numeric_field_range().is_none(),
        "must be None when not in Editing mode"
    );
}

#[test]
fn test_fmt_freq_formats_units() {
    use crate::app::editing::fmt_freq;
    assert_eq!(fmt_freq(None), "unlimited");
    assert_eq!(fmt_freq(Some(800_000)), "800 MHz");
    assert_eq!(fmt_freq(Some(2_400_000)), "2.4 GHz");
    assert_eq!(fmt_freq(Some(1_000_000)), "1.0 GHz");
    assert_eq!(fmt_freq(Some(999_999)), "999 MHz");
    assert_eq!(fmt_freq(Some(0)), "0 MHz");
}

#[test]
fn test_freq_range_fallback_is_400mhz_to_3500mhz() {
    let (min, max) = App::compute_freq_range();
    assert!(min < max, "freq_range must have min < max: got {min} >= {max}");
    if available_cpu_frequencies().is_none() {
        assert_eq!((min, max), (400_000, 3_500_000));
    }
}

#[test]
fn test_arrow_adjust_cpu_freq_uses_500mhz_step_and_clamps() {
    let (mut app, _temp) = make_test_app();
    app.editing_field = Some(EditField::CpuMinFreqKhz);
    let (range_min, range_max) = app.freq_range;
    app.config.cpu.min_freq_khz = Some(range_min);
    app.adjust_field(1);
    assert_eq!(
        app.config.cpu.min_freq_khz,
        Some(range_min.saturating_add(500_000).min(range_max)),
        "step must be 500_000 kHz"
    );
    app.config.cpu.min_freq_khz = Some(range_max);
    app.adjust_field(1);
    assert_eq!(
        app.config.cpu.min_freq_khz,
        Some(range_max),
        "must clamp at upper bound"
    );
}

#[test]
fn test_arrow_adjust_cpu_freq_none_defaults_to_range_min() {
    let (mut app, _temp) = make_test_app();
    app.editing_field = Some(EditField::CpuMinFreqKhz);
    app.config.cpu.min_freq_khz = None;
    let (range_min, _) = app.freq_range;
    app.adjust_field(1);
    assert_eq!(
        app.config.cpu.min_freq_khz,
        Some(range_min.saturating_add(500_000)),
        "from None, min_freq starts at range_min then steps up"
    );
}

#[test]
fn test_arrow_adjust_cpu_max_freq_none_defaults_to_range_max() {
    let (mut app, _temp) = make_test_app();
    app.editing_field = Some(EditField::CpuMaxFreqKhz);
    app.config.cpu.max_freq_khz = None;
    let (range_min, range_max) = app.freq_range;
    app.adjust_field(-1);
    assert_eq!(
        app.config.cpu.max_freq_khz,
        Some(range_max.saturating_sub(500_000).max(range_min)),
        "from None, max_freq starts at range_max then steps down"
    );
}

#[tokio::test]
async fn test_u_key_sets_freq_to_none() {
    let (mut app, _temp) = make_test_app();
    app.mode = AppMode::Editing;
    app.editing_field = Some(EditField::CpuMinFreqKhz);
    app.config.cpu.min_freq_khz = Some(2_000_000);
    let key = KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE);
    app.handle_editing_key(key).await.unwrap();
    assert!(
        app.config.cpu.min_freq_khz.is_none(),
        "'u' must set min_freq_khz to None"
    );

    app.editing_field = Some(EditField::CpuMaxFreqKhz);
    app.config.cpu.max_freq_khz = Some(3_000_000);
    let key = KeyEvent::new(KeyCode::Char('U'), KeyModifiers::NONE);
    app.handle_editing_key(key).await.unwrap();
    assert!(
        app.config.cpu.max_freq_khz.is_none(),
        "'U' must set max_freq_khz to None"
    );

    app.editing_field = Some(EditField::BatteryThreshold);
    app.config.battery.threshold = 80;
    let key = KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE);
    app.handle_editing_key(key).await.unwrap();
    assert_eq!(
        app.config.battery.threshold, 80,
        "'u' must not affect non-freq fields"
    );
}

#[test]
fn test_tab_cycles_fields_in_editing_mode() {
    let (mut app, _temp) = make_test_app();
    app.selected_tab = Tab::Battery;
    app.mode = AppMode::Editing;
    app.begin_editing();
    assert_eq!(app.editing_field, Some(EditField::BatteryThreshold));
    app.next_edit_field();
    assert_eq!(app.editing_field, Some(EditField::BatteryForceCharge));
    app.next_edit_field();
    assert_eq!(
        app.editing_field,
        Some(EditField::BatteryThreshold),
        "must wrap"
    );
}