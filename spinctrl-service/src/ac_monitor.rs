use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use shared::Config;
use tokio::sync::{Mutex, RwLock};
use tokio::time::sleep;

use crate::hardware::{ChargeMode, HardwareBackend};

pub struct AcMonitor {
    hardware: Arc<Mutex<Box<dyn HardwareBackend>>>,
    config: Arc<RwLock<Config>>,
    force_charge: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
}

impl AcMonitor {
    pub fn new(
        hardware: Arc<Mutex<Box<dyn HardwareBackend>>>,
        config: Arc<RwLock<Config>>,
        force_charge: Arc<AtomicBool>,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        Self {
            hardware,
            config,
            force_charge,
            shutdown,
        }
    }

    pub async fn run(&self) {
        let mut last_ac: Option<bool> = None;

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
                        self.handle_ac_plugged().await;
                    } else if prev && !ac_connected {
                        log::info!("AC adapter unplugged");
                        self.handle_ac_unplugged().await;
                    }
                }
                last_ac = Some(ac_connected);
            }

            sleep(Duration::from_secs(5)).await;
        }
    }

    async fn handle_ac_plugged(&self) {
        let (governor_ac, threshold) = {
            let cfg = self.config.read().await;
            (cfg.cpu.governor_ac.clone(), cfg.battery.threshold)
        };

        let mut hw = self.hardware.lock().await;

        if let Err(e) = hw.set_cpu_governor(&governor_ac) {
            log::error!("Failed to set AC governor: {e}");
        }

        let force = self.force_charge.load(Ordering::Relaxed);
        let capacity = match hw.get_battery_capacity() {
            Ok(c) => c,
            Err(e) => {
                log::error!("Failed to read battery capacity: {e}");
                return;
            }
        };

        if force || capacity < threshold {
            if let Err(e) = hw.set_charge_control(ChargeMode::Normal) {
                log::error!("Failed to set charge control normal: {e}");
            }
        } else if let Err(e) = hw.set_charge_control(ChargeMode::Idle) {
            log::error!("Failed to set charge control idle: {e}");
        }
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