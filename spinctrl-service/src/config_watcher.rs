use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use notify::{Event, EventKind, RecursiveMode, Watcher};
use tokio::sync::{Mutex, RwLock};
use tokio::time::sleep;

use crate::hardware::HardwareBackend;

pub struct ConfigWatcher {
    config_path: PathBuf,
    config: Arc<RwLock<shared::Config>>,
    hardware: Arc<Mutex<Box<dyn HardwareBackend>>>,
    shutdown: Arc<AtomicBool>,
    ipc: Arc<shared::IpcManager>,
}

impl ConfigWatcher {
    pub fn new(
        config_path: PathBuf,
        config: Arc<RwLock<shared::Config>>,
        hardware: Arc<Mutex<Box<dyn HardwareBackend>>>,
        shutdown: Arc<AtomicBool>,
        ipc: Arc<shared::IpcManager>,
    ) -> Self {
        Self {
            config_path,
            config,
            hardware,
            shutdown,
            ipc,
        }
    }

    pub async fn run(&self) {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();

        let path = self.config_path.clone();
        let shutdown_flag = self.shutdown.clone();
        let mut watcher =
            match notify::recommended_watcher(move |res: std::result::Result<Event, notify::Error>| {
                match res {
                    Ok(event) => {
                        if matches!(
                            event.kind,
                            EventKind::Modify(_) | EventKind::Create(_)
                        ) {
                            let _ = tx.send(());
                        }
                    }
                    Err(e) => {
                        log::error!("Config watcher error: {e}");
                    }
                }
            }) {
                Ok(w) => w,
                Err(e) => {
                    log::error!("Failed to create config watcher: {e}");
                    return;
                }
            };

        if let Err(e) = watcher.watch(&path, RecursiveMode::NonRecursive) {
            log::warn!("Config file {} not watchable ({e}); skipping file watcher", path.display());
            return;
        }
        log::info!("Watching config file: {}", path.display());

        // Debounce: after receiving the first event, wait 500ms before
        // reloading, and coalesce any events that arrive during that window.
        while rx.recv().await == Some(()) {
            if shutdown_flag.load(Ordering::Relaxed) {
                break;
            }
            // Debounce: sleep 500ms, then drain any additional events
            sleep(Duration::from_millis(500)).await;
            while rx.try_recv().is_ok() {}
            if shutdown_flag.load(Ordering::Relaxed) {
                break;
            }
            self.reload().await;
        }
    }

    async fn reload(&self) {
        log::info!("Config file changed, reloading");
        match self.ipc.read_config() {
            Ok(new_config) => {
                let mut hw = self.hardware.lock().await;
                if let Err(e) = hw.configure_thermal(&new_config.thermal) {
                    log::error!("Failed to apply thermal on reload: {e}");
                }
                if let Err(e) = hw.configure_cpu_frequencies(
                    new_config.cpu.min_freq_khz,
                    new_config.cpu.max_freq_khz,
                ) {
                    log::error!("Failed to apply CPU frequencies on reload: {e}");
                }
                let ac = hw.get_ac_status().unwrap_or(false);
                let governor = if ac {
                    &new_config.cpu.governor_ac
                } else {
                    &new_config.cpu.governor_battery
                };
                if let Err(e) = hw.set_cpu_governor(governor) {
                    log::error!("Failed to apply governor on reload: {e}");
                }
                drop(hw);

                {
                    let mut cfg = self.config.write().await;
                    *cfg = new_config;
                }
                let _ = self.ipc.log_event(
                    shared::EventType::ConfigChanged,
                    "Config reloaded from file".to_string(),
                    None,
                );
            }
            Err(e) => {
                log::error!("Failed to reload config: {e}");
            }
        }
    }
}