use std::fs;
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

use shared::ThermalConfig;
use shared::ThermalZone;

use crate::error::{ServiceError, ServiceResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeMode {
    Normal,
    Idle,
}

impl ChargeMode {
    #[must_use]
    pub const fn as_ectool_arg(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Idle => "idle",
        }
    }
}

pub trait HardwareBackend: Send + 'static {
    /// Set the battery charge control mode.
    ///
    /// # Errors
    /// Returns `Hardware` error if the operation fails.
    fn set_charge_control(&mut self, mode: ChargeMode) -> ServiceResult<()>;
    /// Set the CPU frequency governor.
    ///
    /// # Errors
    /// Returns `Hardware` error if the operation fails.
    fn set_cpu_governor(&mut self, governor: &shared::CpuGovernor) -> ServiceResult<()>;
    /// Configure EC thermal thresholds.
    ///
    /// # Errors
    /// Returns `Hardware` error if the EC is unreachable or the operation fails.
    fn configure_thermal(&mut self, config: &ThermalConfig) -> ServiceResult<()>;
    /// Configure CPU frequency limits (min/max).
    ///
    /// # Errors
    /// Returns `Hardware` error if the operation fails.
    fn configure_cpu_frequencies(
        &mut self,
        min: Option<u32>,
        max: Option<u32>,
    ) -> ServiceResult<()>;
    /// Check if AC adapter is connected.
    ///
    /// # Errors
    /// Returns `Hardware` error if the sysfs path is unreadable.
    fn get_ac_status(&self) -> ServiceResult<bool>;
    /// Get battery charge capacity as percentage.
    ///
    /// # Errors
    /// Returns `Hardware` error if the sysfs path is unreadable.
    fn get_battery_capacity(&self) -> ServiceResult<u8>;
    /// Get the current CPU governor.
    ///
    /// # Errors
    /// Returns `Hardware` error if cpupower is unavailable.
    fn get_cpu_governor(&self) -> ServiceResult<String>;
    /// Get thermal zone readings.
    ///
    /// # Errors
    /// Returns `Hardware` error if the sysfs paths are unreadable.
    fn get_thermal_zones(&self) -> ServiceResult<Vec<ThermalZone>>;
}

// ---------------------------------------------------------------------------
// EctoolBackend – shells out to ectool / cpupower / sysfs
// ---------------------------------------------------------------------------

pub struct EctoolBackend;

impl EctoolBackend {
    fn retry_with_backoff<F, T>(action: F, description: &str) -> ServiceResult<T>
    where
        F: Fn() -> ServiceResult<T>,
    {
        let mut delay = Duration::from_secs(2);
        for attempt in 1..=3 {
            match action() {
                Ok(v) => return Ok(v),
                Err(e) => {
                    log::warn!(
                        "{description} failed (attempt {attempt}/3): {e}"
                    );
                    if attempt < 3 {
                        sleep(delay);
                        delay *= 2;
                    }
                }
            }
        }
        Err(ServiceError::Hardware(format!(
            "{description} failed after 3 attempts"
        )))
    }
}

impl HardwareBackend for EctoolBackend {
    fn set_charge_control(&mut self, mode: ChargeMode) -> ServiceResult<()> {
        let arg = mode.as_ectool_arg();
        Self::retry_with_backoff(
            || {
                let status = Command::new("ectool")
                    .args(["chargecontrol", arg])
                    .status()
                    .map_err(|e| ServiceError::Hardware(format!("ectool spawn: {e}")))?;
                if status.success() {
                    Ok(())
                } else {
                    Err(ServiceError::Hardware(format!(
                        "ectool chargecontrol exited with {status}"
                    )))
                }
            },
            &format!("ectool chargecontrol {arg}"),
        )
    }

    fn set_cpu_governor(&mut self, governor: &shared::CpuGovernor) -> ServiceResult<()> {
        let gov_str = governor.as_str().to_string();
        Self::retry_with_backoff(
            || {
                let status = Command::new("cpupower")
                    .args(["frequency-set", "-g", &gov_str])
                    .status()
                    .map_err(|e| ServiceError::Hardware(format!("cpupower spawn: {e}")))?;
                if !status.success() {
                    return Err(ServiceError::Hardware(format!(
                        "cpupower frequency-set exited with {status}"
                    )));
                }
                // Verify on all cores
                let mut total = 0u32;
                let mut matching = 0u32;
                if let Ok(entries) = fs::read_dir("/sys/devices/system/cpu") {
                    for entry in entries.flatten() {
                        let gov_path = entry.path().join("cpufreq/scaling_governor");
                        if gov_path.exists() {
                            total += 1;
                            if let Ok(cur) = fs::read_to_string(&gov_path) {
                                if cur.trim() == gov_str {
                                    matching += 1;
                                }
                            }
                        }
                    }
                }
                if total > 0 && matching < total {
                    log::warn!(
                        "Governor {gov_str} verified on {matching}/{total} cores"
                    );
                }
                log::info!(
                    "CPU governor set to {gov_str} ({matching}/{total} cores verified)"
                );
                Ok(())
            },
            &format!("cpupower frequency-set -g {gov_str}"),
        )
    }

    fn configure_thermal(&mut self, config: &ThermalConfig) -> ServiceResult<()> {
        // Probe EC thermal sensor availability
        let probe = Command::new("ectool")
            .args(["thermalget", "0"])
            .output()
            .map_err(|e| ServiceError::Hardware(format!("ectool spawn: {e}")))?;
        if !probe.status.success() {
            log::warn!("EC thermal sensors unavailable; skipping thermalset");
            return Err(ServiceError::Hardware(
                "EC thermal sensors unavailable".to_string(),
            ));
        }

        let warn_k = i32::from(config.warn_temp) + 272;
        let high_k = i32::from(config.high_temp) + 272;
        let shutdown_k = i32::from(config.shutdown_temp) + 272;
        let fan_off_k = i32::from(config.fan_off_temp) + 272;
        let fan_max_k = i32::from(config.fan_max_temp) + 272;

        let mut verified_zones = 0u32;
        for zone in 0..=2 {
            let status = Command::new("ectool")
                .args([
                    "thermalset",
                    &zone.to_string(),
                    &warn_k.to_string(),
                    &high_k.to_string(),
                    &shutdown_k.to_string(),
                    &fan_off_k.to_string(),
                    &fan_max_k.to_string(),
                ])
                .status()
                .map_err(|e| ServiceError::Hardware(format!("ectool thermalset spawn: {e}")))?;
            if !status.success() {
                log::warn!("Failed to configure thermal zone {zone}");
                continue;
            }

            // Read back and verify all 5 values
            let readback = Command::new("ectool")
                .args(["thermalget", &zone.to_string()])
                .output();
            if let Ok(output) = readback {
                let text = String::from_utf8_lossy(&output.stdout);
                let nums: Vec<i32> = text
                    .split(|c: char| !c.is_ascii_digit() && c != '-')
                    .filter_map(|s| s.parse::<i32>().ok())
                    .collect();
                let expected = [warn_k, high_k, shutdown_k, fan_off_k, fan_max_k];
                let all_present = expected.iter().all(|v| nums.contains(v));
                if all_present {
                    log::info!("Configured and verified thermal zone {zone}");
                    verified_zones += 1;
                } else {
                    log::warn!("Thermal zone {zone} read-back mismatch (expected: {expected:?})");
                }
            }
        }

        log::info!("Thermal thresholds configured ({verified_zones} zones verified)");
        Ok(())
    }

    fn configure_cpu_frequencies(
        &mut self,
        min: Option<u32>,
        max: Option<u32>,
    ) -> ServiceResult<()> {
        if min.is_none() && max.is_none() {
            return Ok(());
        }
        Self::retry_with_backoff(
            || {
                let mut args: Vec<String> = vec!["frequency-set".to_string()];
                if let Some(m) = min {
                    args.push("-d".to_string());
                    args.push(m.to_string());
                }
                if let Some(m) = max {
                    args.push("-u".to_string());
                    args.push(m.to_string());
                }
                let status = Command::new("cpupower")
                    .args(&args)
                    .status()
                    .map_err(|e| ServiceError::Hardware(format!("cpupower spawn: {e}")))?;
                if status.success() {
                    Ok(())
                } else {
                    Err(ServiceError::Hardware(format!(
                        "cpupower frequency-set exited with {status}"
                    )))
                }
            },
            "cpupower frequency-set",
        )
    }

    fn get_ac_status(&self) -> ServiceResult<bool> {
        let content =
            fs::read_to_string("/sys/class/power_supply/AC/online").map_err(|e| {
                ServiceError::Hardware(format!("Failed to read AC status: {e}"))
            })?;
        Ok(content.trim() == "1")
    }

    fn get_battery_capacity(&self) -> ServiceResult<u8> {
        let content = fs::read_to_string("/sys/class/power_supply/BAT0/capacity")
            .map_err(|e| ServiceError::Hardware(format!("Failed to read battery capacity: {e}")))?;
        content
            .trim()
            .parse::<u8>()
            .map_err(|e| ServiceError::Hardware(format!("Invalid battery capacity: {e}")))
    }

    fn get_cpu_governor(&self) -> ServiceResult<String> {
        let output = Command::new("cpupower")
            .args(["frequency-info", "-p"])
            .output()
            .map_err(|e| ServiceError::Hardware(format!("cpupower spawn: {e}")))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Parse "current policy" line → last token
        for line in stdout.lines() {
            if line.contains("current policy") {
                if let Some(last) = line.split_whitespace().next_back() {
                    return Ok(last.to_string());
                }
            }
        }
        Ok("unknown".to_string())
    }

    fn get_thermal_zones(&self) -> ServiceResult<Vec<ThermalZone>> {
        let mut zones = Vec::new();
        let base = std::path::Path::new("/sys/class/thermal");
        if !base.exists() {
            return Ok(zones);
        }
        let Ok(entries) = fs::read_dir(base) else {
            return Ok(zones);
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.starts_with("thermal_zone") {
                continue;
            }
            let id: u8 = match name
                .strip_prefix("thermal_zone")
                .and_then(|s| s.parse().ok())
            {
                Some(id) => id,
                None => continue,
            };
            let temp_path = path.join("temp");
            let temperature = fs::read_to_string(&temp_path)
                .map_or(0, |content| {
                    content
                        .trim()
                        .parse::<i32>()
                        .map_or(0, |v| v / 1000)
                });
            let trip_points = Vec::new();
            zones.push(ThermalZone {
                id,
                temperature,
                trip_points,
            });
        }
        Ok(zones)
    }
}

// ---------------------------------------------------------------------------
// MockBackend – configurable canned values for tests
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct MockBackend {
    pub ac_connected: bool,
    pub battery_capacity: u8,
    pub cpu_governor: String,
    pub thermal_zones: Vec<ThermalZone>,
    pub charge_control_calls: Vec<ChargeMode>,
    pub governor_calls: Vec<String>,
    pub thermal_config_calls: Vec<ThermalConfig>,
    pub freq_calls: Vec<(Option<u32>, Option<u32>)>,
}

impl Default for MockBackend {
    fn default() -> Self {
        Self {
            ac_connected: true,
            battery_capacity: 75,
            cpu_governor: "performance".to_string(),
            thermal_zones: Vec::new(),
            charge_control_calls: Vec::new(),
            governor_calls: Vec::new(),
            thermal_config_calls: Vec::new(),
            freq_calls: Vec::new(),
        }
    }
}

impl MockBackend {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl HardwareBackend for MockBackend {
    fn set_charge_control(&mut self, mode: ChargeMode) -> ServiceResult<()> {
        self.charge_control_calls.push(mode);
        Ok(())
    }

    fn set_cpu_governor(&mut self, governor: &shared::CpuGovernor) -> ServiceResult<()> {
        self.governor_calls.push(governor.as_str().to_string());
        self.cpu_governor = governor.as_str().to_string();
        Ok(())
    }

    fn configure_thermal(&mut self, config: &ThermalConfig) -> ServiceResult<()> {
        self.thermal_config_calls.push(config.clone());
        Ok(())
    }

    fn configure_cpu_frequencies(
        &mut self,
        min: Option<u32>,
        max: Option<u32>,
    ) -> ServiceResult<()> {
        self.freq_calls.push((min, max));
        Ok(())
    }

    fn get_ac_status(&self) -> ServiceResult<bool> {
        Ok(self.ac_connected)
    }

    fn get_battery_capacity(&self) -> ServiceResult<u8> {
        Ok(self.battery_capacity)
    }

    fn get_cpu_governor(&self) -> ServiceResult<String> {
        Ok(self.cpu_governor.clone())
    }

    fn get_thermal_zones(&self) -> ServiceResult<Vec<ThermalZone>> {
        Ok(self.thermal_zones.clone())
    }
}