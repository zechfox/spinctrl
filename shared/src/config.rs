use serde::{Deserialize, Serialize};
use crate::error::{SpinCtrlError, Result};

/// Main configuration structure for SpinCtrl
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub battery: BatteryConfig,
    pub cpu: CpuConfig,
    pub thermal: ThermalConfig,
    #[serde(default = "default_version")]
    pub version: u32,
}

const fn default_version() -> u32 { 1 }

/// Battery management configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BatteryConfig {
    /// Charge threshold percentage (50-100)
    pub threshold: u8,
    /// Force charging override
    #[serde(default)]
    pub force_charge: bool,
}

/// CPU performance configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CpuConfig {
    /// CPU governor when AC adapter is connected
    pub governor_ac: CpuGovernor,
    /// CPU governor when running on battery
    pub governor_battery: CpuGovernor,
    /// Minimum CPU frequency in kHz (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_freq_khz: Option<u32>,
    /// Maximum CPU frequency in kHz (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_freq_khz: Option<u32>,
}

/// Thermal management configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThermalConfig {
    /// Warning temperature threshold in Celsius
    pub warn_temp: u8,
    /// High temperature threshold in Celsius
    pub high_temp: u8,
    /// Shutdown temperature threshold in Celsius
    pub shutdown_temp: u8,
    /// Fan off temperature in Celsius
    pub fan_off_temp: u8,
    /// Fan maximum speed temperature in Celsius
    pub fan_max_temp: u8,
    /// Thermal profile preset
    #[serde(default = "default_thermal_profile")]
    pub profile: ThermalProfile,
}

/// Predefined thermal profiles
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ThermalProfile {
    Conservative,
    Balanced,
    Performance,
    Custom,
}

/// The five standard Linux cpufreq governors. Serializes as the lowercase
/// snake_case name (`"performance"`, `"powersave"`, …) for wire-format
/// compatibility with the existing `config.json`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum StandardGovernor {
    Performance,
    Powersave,
    Ondemand,
    Conservative,
    Schedutil,
}

impl StandardGovernor {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Performance => "performance",
            Self::Powersave => "powersave",
            Self::Ondemand => "ondemand",
            Self::Conservative => "conservative",
            Self::Schedutil => "schedutil",
        }
    }
}

/// A CPU governor value. Known governors deserialize to the [`StandardGovernor`]
/// set; anything else falls back to [`CpuGovernor::Custom`] so unknown sysfs
/// governors (e.g. `userspace`, `interactive`) round-trip without data loss.
///
/// Serializes back to a plain string, so the JSON shape is unchanged:
/// `"performance"` ↔ `CpuGovernor::Standard(StandardGovernor::Performance)`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(untagged)]
pub enum CpuGovernor {
    Standard(StandardGovernor),
    Custom(String),
}

impl CpuGovernor {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Standard(s) => s.as_str(),
            Self::Custom(s) => s,
        }
    }
}

impl std::fmt::Display for CpuGovernor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for CpuGovernor {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s {
            "performance" => Self::Standard(StandardGovernor::Performance),
            "powersave" => Self::Standard(StandardGovernor::Powersave),
            "ondemand" => Self::Standard(StandardGovernor::Ondemand),
            "conservative" => Self::Standard(StandardGovernor::Conservative),
            "schedutil" => Self::Standard(StandardGovernor::Schedutil),
            other => Self::Custom(other.to_string()),
        })
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            battery: BatteryConfig::default(),
            cpu: CpuConfig::default(),
            thermal: ThermalConfig::default(),
            version: 1,
        }
    }
}

impl BatteryConfig {
    fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if !(50..=100).contains(&self.threshold) {
            errors.push("Battery threshold must be between 50-100%".to_string());
        }
        errors
    }
}

impl CpuConfig {
    fn validate(&self) -> Vec<String> {
        self.validate_with_available(available_cpu_frequencies().as_deref())
    }

    /// Pure validation core. `available` is the hardware-supported kHz list
    /// (from `available_cpu_frequencies()`); pass `None` to skip the
    /// available-range check (sysfs unreadable, or tests). This split keeps
    /// the range logic unit-testable without real hardware/sysfs.
    fn validate_with_available(&self, available: Option<&[u32]>) -> Vec<String> {
        let mut errors = Vec::new();
        if self.governor_ac.as_str().is_empty() {
            errors.push("AC governor cannot be empty".to_string());
        }
        if self.governor_battery.as_str().is_empty() {
            errors.push("Battery governor cannot be empty".to_string());
        }
        if let (Some(min), Some(max)) = (self.min_freq_khz, self.max_freq_khz) {
            if min >= max {
                errors.push("Minimum frequency must be less than maximum frequency".to_string());
            }
        }
        if let Some(freqs) = available {
            let min_avail = freqs.iter().copied().min();
            let max_avail = freqs.iter().copied().max();
            if let (Some(min), Some(min_avail)) = (self.min_freq_khz, min_avail) {
                if min < min_avail {
                    errors.push(format!(
                        "Minimum frequency {} kHz is below the lowest available {} kHz",
                        min, min_avail
                    ));
                }
            }
            if let (Some(max), Some(max_avail)) = (self.max_freq_khz, max_avail) {
                if max > max_avail {
                    errors.push(format!(
                        "Maximum frequency {} kHz exceeds the highest available {} kHz",
                        max, max_avail
                    ));
                }
            }
        }
        errors
    }
}

/// Read the hardware-supported CPU frequencies (kHz) from sysfs
/// (`scaling_available_frequencies` on cpu0). Returns `None` if the path is
/// absent or unreadable (non-Linux, or the TUI user lacks read access), in
/// which case `CpuConfig::validate` falls back to the min<max check only.
pub fn available_cpu_frequencies() -> Option<Vec<u32>> {
    const CPU_FREQ_SYSFS: &str = "/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_frequencies";
    let content = std::fs::read_to_string(CPU_FREQ_SYSFS).ok()?;
    let freqs: Vec<u32> = content
        .split_whitespace()
        .filter_map(|s| s.parse::<u32>().ok())
        .collect();
    if freqs.is_empty() { None } else { Some(freqs) }
}

impl ThermalConfig {
    fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        
        // Range checks
        if !(40..=100).contains(&self.warn_temp) {
            errors.push("Warning temperature must be between 40-100°C".to_string());
        }
        if !(50..=110).contains(&self.shutdown_temp) {
            errors.push("Shutdown temperature must be between 50-110°C".to_string());
        }
        
        // Ordering checks
        if self.warn_temp <= self.high_temp {
            errors.push("Warning temperature must be higher than high temperature".to_string());
        }
        if self.high_temp >= self.shutdown_temp {
            errors.push("High temperature must be lower than shutdown temperature".to_string());
        }
        if self.fan_off_temp >= self.fan_max_temp {
            errors.push("Fan off temperature must be lower than fan max temperature".to_string());
        }
        
        errors
    }
}

impl Default for BatteryConfig {
    fn default() -> Self {
        Self { threshold: 80, force_charge: false }
    }
}

impl Default for CpuConfig {
    fn default() -> Self {
        Self {
            governor_ac: CpuGovernor::Standard(StandardGovernor::Performance),
            governor_battery: CpuGovernor::Standard(StandardGovernor::Powersave),
            min_freq_khz: None,
            max_freq_khz: None,
        }
    }
}

impl Default for ThermalConfig {
    fn default() -> Self {
        Self {
            warn_temp: 70,
            high_temp: 55,
            shutdown_temp: 80,
            fan_off_temp: 50,
            fan_max_temp: 75,
            profile: ThermalProfile::Balanced,
        }
    }
}

fn default_thermal_profile() -> ThermalProfile {
    ThermalProfile::Balanced
}

impl Config {
    /// Create a new configuration with default values
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Load configuration from JSON string
    pub fn from_json(json: &str) -> Result<Self> {
        let config: Config = serde_json::from_str(json)?;
        config.validate()?;
        Ok(config)
    }
    
    /// Convert configuration to pretty-printed JSON string
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Convert configuration to compact single-line JSON. Required for the
    /// `apply_config` FIFO wire format, which is line-oriented and cannot
    /// carry embedded newlines.
    pub fn to_json_compact(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }
    
    /// Validate all configuration values
    pub fn validate(&self) -> Result<()> {
        let mut errors = Vec::new();
        
        errors.extend(self.battery.validate());
        errors.extend(self.cpu.validate());
        errors.extend(self.thermal.validate());
        
        if errors.is_empty() {
            Ok(())
        } else {
            Err(SpinCtrlError::ConfigValidation(errors))
        }
    }
    
    /// Apply a thermal profile to the configuration
    pub fn apply_thermal_profile(&mut self, profile: ThermalProfile) {
        match profile {
            ThermalProfile::Conservative => {
                self.thermal.warn_temp = 65;
                self.thermal.high_temp = 50;
                self.thermal.shutdown_temp = 75;
                self.thermal.fan_off_temp = 45;
                self.thermal.fan_max_temp = 70;
            }
            ThermalProfile::Balanced => {
                self.thermal.warn_temp = 70;
                self.thermal.high_temp = 55;
                self.thermal.shutdown_temp = 80;
                self.thermal.fan_off_temp = 50;
                self.thermal.fan_max_temp = 75;
            }
            ThermalProfile::Performance => {
                self.thermal.warn_temp = 80;
                self.thermal.high_temp = 65;
                self.thermal.shutdown_temp = 90;
                self.thermal.fan_off_temp = 60;
                self.thermal.fan_max_temp = 85;
            }
            ThermalProfile::Custom => {
                // Don't change existing values for custom profile
            }
        }
        self.thermal.profile = profile;
    }
    
    /// Get available CPU governors. Reads
    /// `/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors`
    /// (space-separated); falls back to the built-in list if the sysfs path is
    /// unreadable (non-Linux, no cpufreq support, or the TUI user lacks read
    /// access) so validation never falsely rejects a real governor.
    pub fn get_available_governors() -> Vec<CpuGovernor> {
        const SYSFS_GOVERNORS: &str = "/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors";
        if let Ok(content) = std::fs::read_to_string(SYSFS_GOVERNORS) {
            let governors: Vec<CpuGovernor> = content
                .split_whitespace()
                .map(|s| s.parse().unwrap_or(CpuGovernor::Custom(s.to_string())))
                .collect();
            if !governors.is_empty() {
                return governors;
            }
        }
        vec![
            CpuGovernor::Standard(StandardGovernor::Performance),
            CpuGovernor::Standard(StandardGovernor::Powersave),
            CpuGovernor::Standard(StandardGovernor::Ondemand),
            CpuGovernor::Standard(StandardGovernor::Conservative),
            CpuGovernor::Standard(StandardGovernor::Schedutil),
        ]
    }
    
    /// Check if a governor is valid (present in the available list)
    pub fn is_valid_governor(governor: &CpuGovernor) -> bool {
        Self::get_available_governors().contains(governor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.battery.threshold, 80);
        assert_eq!(config.cpu.governor_ac, CpuGovernor::Standard(StandardGovernor::Performance));
        assert_eq!(config.cpu.governor_battery, CpuGovernor::Standard(StandardGovernor::Powersave));
        assert_eq!(config.thermal.warn_temp, 70);
        assert!(config.validate().is_ok());
    }
    
    #[test]
    fn test_config_validation() {
        let mut config = Config::default();
        
        // Test invalid battery threshold
        config.battery.threshold = 30;
        assert!(config.validate().is_err());
        
        // Reset and test invalid thermal configuration
        config = Config::default();
        config.thermal.warn_temp = 50;
        config.thermal.high_temp = 60;
        assert!(config.validate().is_err());
    }
    
    #[test]
    fn test_json_serialization() {
        let config = Config::default();
        let json = config.to_json().unwrap();
        let deserialized = Config::from_json(&json).unwrap();
        assert_eq!(config, deserialized);
    }
    
    #[test]
    fn test_thermal_profiles() {
        let mut config = Config::default();
        
        config.apply_thermal_profile(ThermalProfile::Conservative);
        assert_eq!(config.thermal.warn_temp, 65);
        assert_eq!(config.thermal.profile, ThermalProfile::Conservative);
        
        config.apply_thermal_profile(ThermalProfile::Performance);
        assert_eq!(config.thermal.warn_temp, 80);
        assert_eq!(config.thermal.profile, ThermalProfile::Performance);
    }
    
    #[test]
    fn test_frequency_validation() {
        let mut config = Config::default();
        config.cpu.min_freq_khz = Some(2000000);
        config.cpu.max_freq_khz = Some(1000000); // Invalid: min > max
        
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_from_json_invalid_json() {
        assert!(Config::from_json("{ not valid json").is_err());
    }

    #[test]
    fn test_from_json_rejects_invalid_threshold() {
        let json = r#"{
            "battery": {"threshold": 30, "force_charge": false},
            "cpu": {"governor_ac": "performance", "governor_battery": "powersave"},
            "thermal": {"warn_temp": 70, "high_temp": 55, "shutdown_temp": 80, "fan_off_temp": 50, "fan_max_temp": 75, "profile": "balanced"}
        }"#;
        assert!(Config::from_json(json).is_err());
    }

    #[test]
    fn test_from_json_missing_version_defaults_to_one() {
        let json = r#"{
            "battery": {"threshold": 80, "force_charge": false},
            "cpu": {"governor_ac": "performance", "governor_battery": "powersave"},
            "thermal": {"warn_temp": 70, "high_temp": 55, "shutdown_temp": 80, "fan_off_temp": 50, "fan_max_temp": 75, "profile": "balanced"}
        }"#;
        let config = Config::from_json(json).expect("valid config should parse");
        assert_eq!(config.version, 1);
    }

    #[test]
    fn test_from_json_round_trip_preserves_all_fields() {
        let original = Config::default();
        let json = original.to_json().unwrap();
        let restored = Config::from_json(&json).unwrap();
        assert_eq!(original, restored);
    }

    #[test]
    fn test_governor_validation_empty_ac() {
        let mut config = Config::default();
        config.cpu.governor_ac = CpuGovernor::Custom(String::new());
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_governor_validation_empty_battery() {
        let mut config = Config::default();
        config.cpu.governor_battery = CpuGovernor::Custom(String::new());
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_is_valid_governor_accepts_returned_rejects_fake() {
        let available = Config::get_available_governors();
        assert!(!available.is_empty());
        let any_valid = available.iter().any(|g| Config::is_valid_governor(g));
        assert!(any_valid, "a returned governor should be valid: {:?}", available);
        assert!(!Config::is_valid_governor(&CpuGovernor::Custom("definitely_not_a_real_governor_xyz".to_string())));
        assert!(!Config::is_valid_governor(&CpuGovernor::Custom(String::new())));
    }

    #[test]
    fn test_get_available_governors_non_empty() {
        let governors = Config::get_available_governors();
        assert!(!governors.is_empty(), "must return a non-empty list (sysfs or fallback)");
    }

    #[test]
    fn test_apply_thermal_profile_balanced_matches_default() {
        let mut config = Config::default();
        config.thermal.warn_temp = 1;
        config.thermal.high_temp = 2;
        config.thermal.shutdown_temp = 3;
        config.thermal.fan_off_temp = 4;
        config.thermal.fan_max_temp = 5;
        config.apply_thermal_profile(ThermalProfile::Balanced);
        let default = Config::default();
        assert_eq!(config.thermal.warn_temp, default.thermal.warn_temp);
        assert_eq!(config.thermal.high_temp, default.thermal.high_temp);
        assert_eq!(config.thermal.shutdown_temp, default.thermal.shutdown_temp);
        assert_eq!(config.thermal.fan_off_temp, default.thermal.fan_off_temp);
        assert_eq!(config.thermal.fan_max_temp, default.thermal.fan_max_temp);
        assert_eq!(config.thermal.profile, ThermalProfile::Balanced);
    }

    #[test]
    fn test_apply_thermal_profile_custom_preserves_values() {
        let mut config = Config::default();
        config.thermal.warn_temp = 95;
        config.thermal.high_temp = 60;
        config.apply_thermal_profile(ThermalProfile::Custom);
        assert_eq!(config.thermal.warn_temp, 95);
        assert_eq!(config.thermal.high_temp, 60);
        assert_eq!(config.thermal.profile, ThermalProfile::Custom);
    }

    #[test]
    fn test_battery_threshold_boundaries() {
        let mut config = Config::default();
        config.battery.threshold = 50;
        assert!(config.validate().is_ok());
        config.battery.threshold = 100;
        assert!(config.validate().is_ok());
        config.battery.threshold = 49;
        assert!(config.validate().is_err());
        config.battery.threshold = 101;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_thermal_high_ge_shutdown_invalid() {
        let mut config = Config::default();
        config.thermal.high_temp = config.thermal.shutdown_temp;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_fan_off_ge_fan_max_invalid() {
        let mut config = Config::default();
        config.thermal.fan_off_temp = config.thermal.fan_max_temp;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_to_json_is_pretty_multiline() {
        let json = Config::default().to_json().unwrap();
        assert!(json.contains('\n'), "pretty JSON should span multiple lines");
    }

    #[test]
    fn test_new_equals_default() {
        assert_eq!(Config::new(), Config::default());
    }

    #[test]
    fn test_cpu_freq_below_available_invalid() {
        let mut config = Config::default();
        config.cpu.min_freq_khz = Some(500_000);
        config.cpu.max_freq_khz = Some(2_000_000);
        let errors = config.cpu.validate_with_available(Some(&[1_000_000, 2_000_000, 3_000_000]));
        assert!(
            errors.iter().any(|e| e.contains("below the lowest available")),
            "expected below-range error, got: {errors:?}"
        );
    }

    #[test]
    fn test_cpu_freq_above_available_invalid() {
        let mut config = Config::default();
        config.cpu.min_freq_khz = Some(1_000_000);
        config.cpu.max_freq_khz = Some(5_000_000);
        let errors = config.cpu.validate_with_available(Some(&[1_000_000, 2_000_000, 3_000_000]));
        assert!(
            errors.iter().any(|e| e.contains("exceeds the highest available")),
            "expected above-range error, got: {errors:?}"
        );
    }

    #[test]
    fn test_cpu_freq_within_available_valid() {
        let mut config = Config::default();
        config.cpu.min_freq_khz = Some(1_000_000);
        config.cpu.max_freq_khz = Some(3_000_000);
        let errors = config.cpu.validate_with_available(Some(&[1_000_000, 2_000_000, 3_000_000]));
        assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
    }

    #[test]
    fn test_cpu_freq_range_check_skipped_when_available_none() {
        let mut config = Config::default();
        config.cpu.min_freq_khz = Some(1);
        config.cpu.max_freq_khz = Some(2);
        let errors = config.cpu.validate_with_available(None);
        assert!(errors.is_empty(), "available=None skips range check, got: {errors:?}");
    }

    #[test]
    fn test_available_cpu_frequencies_is_callable_or_none() {
        let _ = available_cpu_frequencies();
    }

    #[test]
    fn test_cpu_governor_serde_roundtrip_standard() {
        let json = serde_json::to_string(&CpuGovernor::Standard(StandardGovernor::Performance)).unwrap();
        assert_eq!(json, "\"performance\"");
        let back: CpuGovernor = serde_json::from_str(&json).unwrap();
        assert_eq!(back, CpuGovernor::Standard(StandardGovernor::Performance));
    }

    #[test]
    fn test_cpu_governor_serde_roundtrip_custom() {
        let json = serde_json::to_string(&CpuGovernor::Custom("userspace".to_string())).unwrap();
        assert_eq!(json, "\"userspace\"");
        let back: CpuGovernor = serde_json::from_str(&json).unwrap();
        assert_eq!(back, CpuGovernor::Custom("userspace".to_string()));
    }

    #[test]
    fn test_cpu_governor_from_str_all_standard() {
        assert_eq!("performance".parse::<CpuGovernor>().unwrap(), CpuGovernor::Standard(StandardGovernor::Performance));
        assert_eq!("powersave".parse::<CpuGovernor>().unwrap(), CpuGovernor::Standard(StandardGovernor::Powersave));
        assert_eq!("ondemand".parse::<CpuGovernor>().unwrap(), CpuGovernor::Standard(StandardGovernor::Ondemand));
        assert_eq!("conservative".parse::<CpuGovernor>().unwrap(), CpuGovernor::Standard(StandardGovernor::Conservative));
        assert_eq!("schedutil".parse::<CpuGovernor>().unwrap(), CpuGovernor::Standard(StandardGovernor::Schedutil));
    }

    #[test]
    fn test_cpu_governor_from_str_unknown_becomes_custom() {
        let g: CpuGovernor = "userspace".parse().unwrap();
        assert_eq!(g, CpuGovernor::Custom("userspace".to_string()));
    }

    #[test]
    fn test_cpu_governor_display_matches_as_str() {
        assert_eq!(CpuGovernor::Standard(StandardGovernor::Performance).to_string(), "performance");
        assert_eq!(CpuGovernor::Custom("interactive".to_string()).to_string(), "interactive");
    }

    #[test]
    fn test_config_json_backward_compatible_with_string_governor() {
        let json = r#"{
            "battery": {"threshold": 80, "force_charge": false},
            "cpu": {"governor_ac": "performance", "governor_battery": "powersave"},
            "thermal": {"warn_temp": 70, "high_temp": 55, "shutdown_temp": 80, "fan_off_temp": 50, "fan_max_temp": 75, "profile": "balanced"}
        }"#;
        let config = Config::from_json(json).expect("legacy string-governor JSON must parse");
        assert_eq!(config.cpu.governor_ac, CpuGovernor::Standard(StandardGovernor::Performance));
        assert_eq!(config.cpu.governor_battery, CpuGovernor::Standard(StandardGovernor::Powersave));
    }
}