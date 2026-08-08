use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use shared::Config;
use tokio::sync::{Mutex, RwLock};
use tokio::time::sleep;

use crate::hardware::{ChargeMode, HardwareBackend};

/// Decide the EC charge-control mode for the given battery state. Idle
/// (threshold enforcement) applies when the battery has reached the configured
/// threshold and force-charge is not active.
const fn desired_charge_mode(force_charge: bool, threshold: u8, capacity: u8) -> ChargeMode {
    if force_charge || capacity < threshold {
        ChargeMode::Normal
    } else {
        ChargeMode::Idle
    }
}

pub struct AcMonitor {
    hardware: Arc<Mutex<Box<dyn HardwareBackend>>>,
    config: Arc<RwLock<Config>>,
    shutdown: Arc<AtomicBool>,
}

impl AcMonitor {
    pub fn new(
        hardware: Arc<Mutex<Box<dyn HardwareBackend>>>,
        config: Arc<RwLock<Config>>,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        Self {
            hardware,
            config,
            shutdown,
        }
    }

    pub async fn run(&self) {
        let mut last_ac: Option<bool> = None;
        let mut last_charge_mode: Option<ChargeMode> = None;

        loop {
            if self.shutdown.load(Ordering::Relaxed) {
                log::info!("AC monitor stopping due to shutdown");
                break;
            }

            let ac_connected = {
                let hw = self.hardware.lock().await;
                match hw.get_ac_status() {
                    Ok(v) => v,
                    Err(e) => {
                        log::error!("Failed to read AC status: {e}");
                        sleep(Duration::from_secs(5)).await;
                        continue;
                    }
                }
            };

            if last_ac != Some(ac_connected) {
                if let Some(prev) = last_ac {
                    if !prev && ac_connected {
                        log::info!("AC adapter plugged in");
                        let mode = self.handle_ac_plugged().await;
                        last_charge_mode = Some(mode);
                    } else if prev && !ac_connected {
                        log::info!("AC adapter unplugged");
                        self.handle_ac_unplugged().await;
                        last_charge_mode = None;
                    }
                }
                last_ac = Some(ac_connected);
            }

            if ac_connected {
                // The battery can cross the threshold while AC stays connected
                // (no AC transition fires in that case), so re-evaluate the
                // charge mode on every poll. Only touch the hardware when the
                // desired mode actually changes.
                let (threshold, force_charge) = {
                    let cfg = self.config.read().await;
                    (cfg.battery.threshold, cfg.battery.force_charge)
                };
                let capacity = {
                    let hw = self.hardware.lock().await;
                    match hw.get_battery_capacity() {
                        Ok(c) => Some(c),
                        Err(e) => {
                            log::error!("Failed to read battery capacity: {e}");
                            None
                        }
                    }
                };
                if let Some(capacity) = capacity {
                    let mode = desired_charge_mode(force_charge, threshold, capacity);
                    if last_charge_mode != Some(mode) {
                        let mut hw = self.hardware.lock().await;
                        match hw.set_charge_control(mode) {
                            Ok(()) => {
                                last_charge_mode = Some(mode);
                                log::info!("Charge control set to {mode:?}");
                            }
                            Err(e) => log::error!("Failed to set charge control {mode:?}: {e}"),
                        }
                    }
                }
            }

            sleep(Duration::from_secs(5)).await;
        }
    }

    async fn handle_ac_plugged(&self) -> ChargeMode {
        let (governor_ac, threshold, force) = {
            let cfg = self.config.read().await;
            (cfg.cpu.governor_ac.clone(), cfg.battery.threshold, cfg.battery.force_charge)
        };

        let mut hw = self.hardware.lock().await;

        if let Err(e) = hw.set_cpu_governor(&governor_ac) {
            log::error!("Failed to set AC governor: {e}");
        }

        let capacity = match hw.get_battery_capacity() {
            Ok(c) => c,
            Err(e) => {
                log::error!("Failed to read battery capacity: {e}");
                return ChargeMode::Normal;
            }
        };

        let mode = desired_charge_mode(force, threshold, capacity);
        if let Err(e) = hw.set_charge_control(mode) {
            log::error!("Failed to set charge control {mode:?}: {e}");
        }
        drop(hw);
        mode
    }

    async fn handle_ac_unplugged(&self) {
        let governor_battery = {
            let cfg = self.config.read().await;
            cfg.cpu.governor_battery.clone()
        };

        let mut hw = self.hardware.lock().await;

        if let Err(e) = hw.set_cpu_governor(&governor_battery) {
            log::error!("Failed to set battery governor: {e}");
        }

        if let Err(e) = hw.set_charge_control(ChargeMode::Normal) {
            log::error!("Failed to restore normal charge control: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{desired_charge_mode, ChargeMode};

    #[test]
    fn test_desired_charge_mode_threshold() {
        assert_eq!(desired_charge_mode(false, 80, 79), ChargeMode::Normal);
        assert_eq!(desired_charge_mode(false, 80, 80), ChargeMode::Idle);
        assert_eq!(desired_charge_mode(false, 80, 100), ChargeMode::Idle);
        assert_eq!(desired_charge_mode(true, 80, 100), ChargeMode::Normal);
        assert_eq!(desired_charge_mode(true, 80, 20), ChargeMode::Normal);
    }
}
