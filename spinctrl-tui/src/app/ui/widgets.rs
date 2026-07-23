use ratatui::style::Color;
use shared::{EventType, ThermalProfile};
use crate::app::editing::EditField;

/// Pick a color for a temperature reading.
pub const fn zone_temp_color(temp: i32) -> Color {
    if temp >= 70 {
        Color::Red
    } else if temp >= 55 {
        Color::Yellow
    } else {
        Color::Green
    }
}

/// Short display tag + color for an event type.
pub struct EventTypeTag {
    pub tag: &'static str,
    pub color: Color,
}

/// Short display tag + color for an event type.
pub const fn event_type_tag(et: &EventType) -> EventTypeTag {
    match et {
        EventType::ConfigChanged => EventTypeTag { tag: "CFG", color: Color::Cyan },
        EventType::CommandExecuted => EventTypeTag { tag: "CMD", color: Color::Blue },
        EventType::HardwareAction => EventTypeTag { tag: "HW", color: Color::Magenta },
        EventType::Error => EventTypeTag { tag: "ERR", color: Color::Red },
        EventType::ServiceStart => EventTypeTag { tag: "STA", color: Color::Green },
        EventType::ServiceStop => EventTypeTag { tag: "STP", color: Color::Yellow },
    }
}

/// Brief description of what each editable config field does, shown inline
/// in the tab (`DarkGray`) so users understand the parameter without help.
pub const fn field_description(field: EditField) -> &'static str {
    match field {
        EditField::BatteryThreshold => "stop charging above this %",
        EditField::BatteryForceCharge => "one-shot charge to 100%",
        EditField::CpuGovernorAc => "governor on AC power",
        EditField::CpuGovernorBattery => "governor on battery",
        EditField::CpuMinFreqKhz => "min CPU freq (u=unlimited)",
        EditField::CpuMaxFreqKhz => "max CPU freq (u=unlimited)",
        EditField::ThermalProfile => "thermal preset",
        EditField::ThermalFanOff => "fan stops below this",
        EditField::ThermalHigh => "triggers throttling",
        EditField::ThermalWarn => "triggers caution",
        EditField::ThermalFanMax => "fan at max speed",
        EditField::ThermalShutdown => "emergency shutdown",
    }
}

/// Human-readable thermal profile name.
pub const fn thermal_profile_name(p: &ThermalProfile) -> &'static str {
    match p {
        ThermalProfile::Conservative => "Conservative",
        ThermalProfile::Balanced => "Balanced",
        ThermalProfile::Performance => "Performance",
        ThermalProfile::Custom => "Custom",
    }
}