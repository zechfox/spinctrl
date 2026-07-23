use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::sleep;

use crate::hardware::HardwareBackend;

pub struct StatusWriter {
    hardware: Arc<Mutex<Box<dyn HardwareBackend>>>,
    shutdown: Arc<AtomicBool>,
    ipc: Arc<shared::IpcManager>,
}

impl StatusWriter {
    pub fn new(
        hardware: Arc<Mutex<Box<dyn HardwareBackend>>>,
        shutdown: Arc<AtomicBool>,
        ipc: Arc<shared::IpcManager>,
    ) -> Self {
        Self {
            hardware,
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
        let ac_connected = hw.get_ac_status().unwrap_or(false);
        let cpu_governor = hw.get_cpu_governor().unwrap_or_else(|_| "unknown".to_string());
        let thermal_zones = hw.get_thermal_zones().unwrap_or_default();
        drop(hw);

        let charging = ac_connected && battery_capacity < 100;
        let threshold_active = ac_connected && battery_capacity >= 100;

        let status = shared::SystemStatus {
            battery: shared::BatteryStatus {
                capacity: battery_capacity,
                charging,
                threshold_active,
                ac_connected,
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