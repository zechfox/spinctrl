use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use shared::Config;
use tokio::sync::{Mutex, RwLock};
use tokio::time::sleep;

use crate::hardware::HardwareBackend;

pub struct StatusWriter {
    hardware: Arc<Mutex<Box<dyn HardwareBackend>>>,
    config: Arc<RwLock<Config>>,
    shutdown: Arc<AtomicBool>,
    ipc: Arc<shared::IpcManager>,
}

impl StatusWriter {
    pub fn new(
        hardware: Arc<Mutex<Box<dyn HardwareBackend>>>,
        config: Arc<RwLock<Config>>,
        shutdown: Arc<AtomicBool>,
        ipc: Arc<shared::IpcManager>,
    ) -> Self {
        Self {
            hardware,
            config,
            shutdown,
            ipc,
        }
    }

    pub async fn run(&self) {
        loop {
            if self.shutdown.load(Ordering::Relaxed) {
                break;
            }

            self.write_status().await;

            // Sleep for 30 seconds, but check shutdown every 1s
            for _ in 0..30 {
                if self.shutdown.load(Ordering::Relaxed) {
                    break;
                }
                sleep(Duration::from_secs(1)).await;
            }
        }
    }

    async fn write_status(&self) {
        let hw = self.hardware.lock().await;

        let battery_capacity = hw.get_battery_capacity().unwrap_or(0);
        let battery_health = hw.get_battery_health().unwrap_or_default();
        let ac_connected = hw.get_ac_status().unwrap_or(false);
        let cpu_governor = hw.get_cpu_governor().unwrap_or_else(|_| "unknown".to_string());
        let thermal_zones = hw.get_thermal_zones().unwrap_or_default();
        drop(hw);

        // Must mirror the charge-control predicate in ac_monitor.rs and
        // command_processor.rs: idle (threshold enforced) when AC is on,
        // not force_charge, and capacity has reached the threshold.
        let (threshold, force_charge) = {
            let cfg = self.config.read().await;
            (cfg.battery.threshold, cfg.battery.force_charge)
        };
        let charging = ac_connected && (force_charge || battery_capacity < threshold);
        let threshold_active =
            ac_connected && !force_charge && battery_capacity >= threshold;

        let status = shared::SystemStatus {
            battery: shared::BatteryStatus {
                capacity: battery_capacity,
                charging,
                threshold_active,
                ac_connected,
                health: battery_health.health,
                cycle_count: battery_health.cycle_count,
            },
            power: shared::PowerStatus {
                ac_connected,
                cpu_governor,
                cpu_freq_khz: None,
            },
            thermal: if thermal_zones.is_empty() {
                None
            } else {
                Some(shared::ThermalStatus {
                    zones: thermal_zones,
                })
            },
            timestamp: chrono::Utc::now(),
            service_pid: std::process::id(),
        };

        if let Err(e) = self.ipc.write_status(&status) {
            log::error!("Failed to write status: {e}");
        }
    }
}