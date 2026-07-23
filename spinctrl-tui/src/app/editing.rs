use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use shared::ThermalProfile;
use crate::app::state::{AppMode, Tab};

use super::App;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditField {
    BatteryThreshold,
    BatteryForceCharge,
    CpuGovernorAc,
    CpuGovernorBattery,
    CpuMinFreqKhz,
    CpuMaxFreqKhz,
    ThermalProfile,
    ThermalFanOff,
    ThermalHigh,
    ThermalWarn,
    ThermalFanMax,
    ThermalShutdown,
}

impl App {
    pub fn begin_editing(&mut self) {
        self.config_backup = self.config.clone();
        let field = match self.selected_tab {
            Tab::Battery => {
                if self.config.battery.force_charge {
                    EditField::BatteryForceCharge
                } else {
                    EditField::BatteryThreshold
                }
            }
            Tab::CPU => EditField::CpuGovernorAc,
            Tab::Thermal => EditField::ThermalProfile,
            _ => return,
        };
        self.editing_field = Some(field);
    }

    pub fn field_display(&self, field: EditField, value: &str, suffix: &str, normal: Color) -> Span<'static> {
        let is_this = self.mode == AppMode::Editing && self.editing_field == Some(field);
        if !is_this {
            return Span::styled(
                format!("{value}{suffix}"),
                Style::default().fg(normal),
            );
        }
        let is_enum = matches!(
            field,
            EditField::BatteryForceCharge
                | EditField::CpuGovernorAc
                | EditField::CpuGovernorBattery
                | EditField::ThermalProfile
        );
        if is_enum {
            return Span::styled(
                format!("← {value}{suffix} →"),
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            );
        }
        if let Some((min, max, v, color)) = self.numeric_field_range() {
            let min_label = Self::range_label_for_field(field, min);
            let max_label = Self::range_label_for_field(field, max);
            let is_unlimited = match field {
                EditField::CpuMinFreqKhz => self.config.cpu.min_freq_khz.is_none(),
                EditField::CpuMaxFreqKhz => self.config.cpu.max_freq_khz.is_none(),
                _ => false,
            };
            let value_label = if is_unlimited {
                "unlimited".to_string()
            } else {
                format!("{value}{suffix}")
            };
            return build_trackbar(min, max, v, &min_label, &max_label, &value_label, color);
        }
        Span::styled(
            format!("{value}{suffix}"),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )
    }

    pub fn adjust_field(&mut self, direction: i32) {
        let Some(field) = self.editing_field else { return };
        match field {
            EditField::BatteryThreshold => {
                if self.config.battery.force_charge {
                    return; // Threshold is disabled while force charge is active
                }
                let new_val = (i32::from(self.config.battery.threshold) + direction).clamp(50, 100);
                self.config.battery.threshold = new_val as u8;
            }
            EditField::BatteryForceCharge => {
                self.config.battery.force_charge = !self.config.battery.force_charge;
            }
            EditField::CpuGovernorAc => {
                Self::cycle_governor(&self.available_governors, &mut self.config.cpu.governor_ac, direction);
            }
            EditField::CpuGovernorBattery => {
                Self::cycle_governor(&self.available_governors, &mut self.config.cpu.governor_battery, direction);
            }
            EditField::CpuMinFreqKhz => {
                const STEP: u32 = 500_000;
                let (range_min, range_max) = self.freq_range;
                let current = self.config.cpu.min_freq_khz.unwrap_or(range_min);
                let new_val = if direction > 0 {
                    current.saturating_add(STEP)
                } else {
                    current.saturating_sub(STEP)
                };
                self.config.cpu.min_freq_khz = Some(new_val.clamp(range_min, range_max));
            }
            EditField::CpuMaxFreqKhz => {
                const STEP: u32 = 500_000;
                let (range_min, range_max) = self.freq_range;
                let current = self.config.cpu.max_freq_khz.unwrap_or(range_max);
                let new_val = if direction > 0 {
                    current.saturating_add(STEP)
                } else {
                    current.saturating_sub(STEP)
                };
                self.config.cpu.max_freq_khz = Some(new_val.clamp(range_min, range_max));
            }
            EditField::ThermalWarn => {
                let new_val = (i32::from(self.config.thermal.warn_temp) + direction).clamp(40, 100);
                self.config.thermal.warn_temp = new_val as u8;
            }
            EditField::ThermalHigh => {
                let new_val = (i32::from(self.config.thermal.high_temp) + direction).clamp(30, 90);
                self.config.thermal.high_temp = new_val as u8;
            }
            EditField::ThermalShutdown => {
                let new_val = (i32::from(self.config.thermal.shutdown_temp) + direction).clamp(50, 110);
                self.config.thermal.shutdown_temp = new_val as u8;
            }
            EditField::ThermalFanOff => {
                let new_val = (i32::from(self.config.thermal.fan_off_temp) + direction).clamp(20, 80);
                self.config.thermal.fan_off_temp = new_val as u8;
            }
            EditField::ThermalFanMax => {
                let new_val = (i32::from(self.config.thermal.fan_max_temp) + direction).clamp(40, 100);
                self.config.thermal.fan_max_temp = new_val as u8;
            }
            EditField::ThermalProfile => {
                const PROFILES: [ThermalProfile; 4] = [
                    ThermalProfile::Conservative,
                    ThermalProfile::Balanced,
                    ThermalProfile::Performance,
                    ThermalProfile::Custom,
                ];
                let cur = PROFILES
                    .iter()
                    .position(|p| *p == self.config.thermal.profile)
                    .unwrap_or(1);
                #[expect(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                let next = (cur as i32 + direction).rem_euclid(PROFILES.len() as i32) as usize;
                self.config.thermal.profile = PROFILES[next].clone();
            }
        }
    }

    fn cycle_governor(
        available: &[shared::CpuGovernor],
        current: &mut shared::CpuGovernor,
        direction: i32,
    ) {
        if available.is_empty() {
            return;
        }
        let cur = available.iter().position(|g| g == current).unwrap_or(0);
        #[expect(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let next = (cur as i32 + direction).rem_euclid(available.len() as i32) as usize;
        *current = available[next].clone();
    }

    pub fn next_edit_field(&mut self) {
        let cur = self.editing_field.unwrap_or(EditField::BatteryThreshold);
        let next: Option<EditField> = match (cur, self.selected_tab) {
            (EditField::BatteryThreshold, Tab::Battery) => Some(EditField::BatteryForceCharge),
            (EditField::BatteryForceCharge, Tab::Battery) => {
                if self.config.battery.force_charge {
                    None  // Exit edit mode — threshold is disabled, nothing left to edit
                } else {
                    Some(EditField::BatteryThreshold)
                }
            }
            (EditField::CpuGovernorAc, Tab::CPU) => Some(EditField::CpuGovernorBattery),
            (EditField::CpuGovernorBattery, Tab::CPU) => Some(EditField::CpuMinFreqKhz),
            (EditField::CpuMinFreqKhz, Tab::CPU) => Some(EditField::CpuMaxFreqKhz),
            (EditField::CpuMaxFreqKhz, Tab::CPU) => Some(EditField::CpuGovernorAc),
            (EditField::ThermalProfile, Tab::Thermal) => Some(EditField::ThermalFanOff),
            (EditField::ThermalFanOff, Tab::Thermal) => Some(EditField::ThermalHigh),
            (EditField::ThermalHigh, Tab::Thermal) => Some(EditField::ThermalWarn),
            (EditField::ThermalWarn, Tab::Thermal) => Some(EditField::ThermalFanMax),
            (EditField::ThermalFanMax, Tab::Thermal) => Some(EditField::ThermalShutdown),
            (EditField::ThermalShutdown, Tab::Thermal) => Some(EditField::ThermalProfile),
            _ => None,
        };
        if let Some(f) = next {
            self.editing_field = Some(f);
        } else {
            self.mode = AppMode::Monitoring;
            self.editing_field = None;
        }
    }

    pub fn numeric_field_range(&self) -> Option<(u32, u32, u32, Color)> {
        if self.mode != AppMode::Editing {
            return None;
        }
        let field = self.editing_field?;
        match field {
            EditField::BatteryThreshold => {
                Some((50, 100, u32::from(self.config.battery.threshold), Color::Cyan))
            }
            EditField::CpuMinFreqKhz => {
                let (min, max) = self.freq_range;
                let v = self.config.cpu.min_freq_khz.unwrap_or(max);
                Some((min, max, v, Color::White))
            }
            EditField::CpuMaxFreqKhz => {
                let (min, max) = self.freq_range;
                let v = self.config.cpu.max_freq_khz.unwrap_or(max);
                Some((min, max, v, Color::White))
            }
            EditField::ThermalWarn => {
                Some((40, 100, u32::from(self.config.thermal.warn_temp), Color::Yellow))
            }
            EditField::ThermalHigh => {
                Some((30, 90, u32::from(self.config.thermal.high_temp), Color::Yellow))
            }
            EditField::ThermalShutdown => {
                Some((50, 110, u32::from(self.config.thermal.shutdown_temp), Color::Red))
            }
            EditField::ThermalFanOff => {
                Some((20, 80, u32::from(self.config.thermal.fan_off_temp), Color::Green))
            }
            EditField::ThermalFanMax => {
                Some((40, 100, u32::from(self.config.thermal.fan_max_temp), Color::Red))
            }
            _ => None,
        }
    }

    pub fn compute_freq_range() -> (u32, u32) {
        match shared::available_cpu_frequencies() {
            Some(freqs) if !freqs.is_empty() => {
                let min = *freqs.iter().min().expect("non-empty");
                let max = *freqs.iter().max().expect("non-empty");
                if min < max { (min, max) } else { (400_000, 3_500_000) }
            }
            _ => (400_000, 3_500_000),
        }
    }

    pub fn range_label_for_field(field: EditField, raw: u32) -> String {
        match field {
            EditField::CpuMinFreqKhz | EditField::CpuMaxFreqKhz => fmt_freq(Some(raw)),
            EditField::BatteryThreshold => format!("{raw}%"),
            _ => format!("{raw}°C"),
        }
    }
}

fn build_trackbar(
    min: u32,
    max: u32,
    value: u32,
    min_label: &str,
    max_label: &str,
    value_label: &str,
    color: Color,
) -> Span<'static> {
    const BAR_WIDTH: usize = 18;
    let denom = max.saturating_sub(min);
    let ratio = if denom > 0 {
        (f64::from(value.saturating_sub(min)) / f64::from(denom)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
    let marker_pos = (ratio * (BAR_WIDTH.saturating_sub(1)) as f64).round() as usize;
    let marker_pos = marker_pos.min(BAR_WIDTH.saturating_sub(1));
    let bar: String = (0..BAR_WIDTH)
        .map(|i| if i == marker_pos { '●' } else { '─' })
        .collect();
    Span::styled(
        format!("{min_label} {bar} {max_label}  {value_label}"),
        Style::default().fg(color),
    )
}

pub fn fmt_freq(khz: Option<u32>) -> String {
    match khz {
        None => "unlimited".to_string(),
        Some(v) if v >= 1_000_000 => format!("{:.1} GHz", f64::from(v) / 1_000_000.0),
        Some(v) => format!("{} MHz", v / 1_000),
    }
}