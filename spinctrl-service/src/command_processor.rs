use std::fs::OpenOptions;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use shared::{Config, EventType, IpcManager};
use tokio::sync::{mpsc, Mutex, RwLock};

use crate::error::{ServiceError, ServiceResult};
use crate::hardware::{ChargeMode, HardwareBackend};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedCommand {
    ForceCharge,
    StopCharge,
    ApplyConfig(String),
    ReloadConfig,
    Shutdown,
}

/// Parse a command string from the FIFO wire format.
#[must_use]
pub fn parse_command(line: &str) -> Option<ParsedCommand> {
    match line {
        "force_charge" => Some(ParsedCommand::ForceCharge),
        "stop_charge" => Some(ParsedCommand::StopCharge),
        "reload_config" => Some(ParsedCommand::ReloadConfig),
        "shutdown" => Some(ParsedCommand::Shutdown),
        s if s.starts_with("apply_config:") => {
            let json = &s["apply_config:".len()..];
            Some(ParsedCommand::ApplyConfig(json.to_string()))
        }
        _ => None,
    }
}

pub fn start_fifo_reader(
    fifo_path: PathBuf,
    cmd_tx: mpsc::UnboundedSender<String>,
    shutdown: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        loop {
            if shutdown.load(Ordering::Relaxed) {
                break;
            }

            let file = match OpenOptions::new()
                .read(true)
                .write(true)
                .open(&fifo_path)
            {
                Ok(f) => f,
                Err(e) => {
                    log::error!("Failed to open FIFO: {e}");
                    std::thread::sleep(Duration::from_millis(500));
                    continue;
                }
            };

            let mut reader = BufReader::new(file);
            let mut line = String::new();
            loop {
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) => {
                        break;
                    }
                    Ok(_) => {
                        let trimmed = line.trim().to_string();
                        if !trimmed.is_empty() {
                            let _ = cmd_tx.send(trimmed);
                        }
                    }
                    Err(e) => {
                        log::error!("FIFO read error: {e}");
                        break;
                    }
                }
            }
        }
    });
}

pub struct CommandDispatcher {
    hardware: Arc<Mutex<Box<dyn HardwareBackend>>>,
    config: Arc<RwLock<Config>>,
    force_charge: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    ipc: Arc<IpcManager>,
}

impl CommandDispatcher {
    pub fn new(
        hardware: Arc<Mutex<Box<dyn HardwareBackend>>>,
        config: Arc<RwLock<Config>>,
        force_charge: Arc<AtomicBool>,
        shutdown: Arc<AtomicBool>,
        ipc: Arc<IpcManager>,
    ) -> Self {
        Self {
            hardware,
            config,
            force_charge,
            shutdown,
            ipc,
        }
    }

    pub async fn run(&self, mut rx: mpsc::UnboundedReceiver<String>) {
        while let Some(line) = rx.recv().await {
            if self.shutdown.load(Ordering::Relaxed) {
                break;
            }
            log::info!("Received command: {line}");
            let Some(cmd) = parse_command(&line) else {
                log::warn!("Unknown command: {line}");
                let _ = self.ipc.log_event(
                    EventType::Error,
                    "Unknown command received".to_string(),
                    Some(serde_json::json!({"command": line})),
                );
                continue;
            };
            if let Err(e) = self.dispatch(cmd).await {
                log::error!("Command dispatch error: {e}");
            }
        }
    }

    async fn dispatch(&self, cmd: ParsedCommand) -> ServiceResult<()> {
        match cmd {
            ParsedCommand::ForceCharge => self.handle_force_charge().await,
            ParsedCommand::StopCharge => self.handle_stop_charge().await,
            ParsedCommand::ApplyConfig(json) => self.handle_apply_config(&json).await,
            ParsedCommand::ReloadConfig => self.handle_reload_config().await,
            ParsedCommand::Shutdown => {
                log::info!("Shutdown command received");
                self.shutdown.store(true, Ordering::Relaxed);
                let _ = self.ipc.log_event(
                    EventType::CommandExecuted,
                    "Shutdown requested".to_string(),
                    None,
                );
                Ok(())
            }
        }
    }

    async fn handle_force_charge(&self) -> ServiceResult<()> {
        self.force_charge.store(true, Ordering::Relaxed);
        log::info!("Force charge enabled");
        let _ = self.ipc.log_event(
            EventType::CommandExecuted,
            "Force charge enabled".to_string(),
            None,
        );
        let mut hw = self.hardware.lock().await;
        if hw.get_ac_status().unwrap_or(false) {
            hw.set_charge_control(ChargeMode::Normal)?;
        }
        drop(hw);
        Ok(())
    }

    async fn handle_stop_charge(&self) -> ServiceResult<()> {
        self.force_charge.store(false, Ordering::Relaxed);
        log::info!("Force charge disabled");
        let _ = self.ipc.log_event(
            EventType::CommandExecuted,
            "Force charge disabled".to_string(),
            None,
        );
        let mut hw = self.hardware.lock().await;
        if hw.get_ac_status().unwrap_or(false) {
            let threshold = {
                let cfg = self.config.read().await;
                cfg.battery.threshold
            };
            let capacity = hw.get_battery_capacity().unwrap_or(0);
            if capacity >= threshold {
                hw.set_charge_control(ChargeMode::Idle)?;
            }
        }
        drop(hw);
        Ok(())
    }

    async fn handle_apply_config(&self, json: &str) -> ServiceResult<()> {
        let new_config =
            Config::from_json(json).map_err(|e| ServiceError::Config(e.to_string()))?;
        new_config
            .validate()
            .map_err(|e| ServiceError::Config(e.to_string()))?;

        {
            let mut hw = self.hardware.lock().await;
            hw.configure_thermal(&new_config.thermal)?;
            hw.configure_cpu_frequencies(
                new_config.cpu.min_freq_khz,
                new_config.cpu.max_freq_khz,
            )?;
            let ac = hw.get_ac_status().unwrap_or(false);
            let governor = if ac {
                &new_config.cpu.governor_ac
            } else {
                &new_config.cpu.governor_battery
            };
            hw.set_cpu_governor(governor)?;
        }

        {
            let mut cfg = self.config.write().await;
            *cfg = new_config.clone();
        }

        self.ipc
            .write_config(&new_config)
            .map_err(ServiceError::Shared)?;

        let _ = self.ipc.log_event(
            EventType::CommandExecuted,
            "Full config applied".to_string(),
            Some(serde_json::json!({
                "threshold": new_config.battery.threshold,
                "cpu_ac": new_config.cpu.governor_ac.as_str(),
            })),
        );
        log::info!("Config applied and persisted");
        Ok(())
    }

    async fn handle_reload_config(&self) -> ServiceResult<()> {
        log::info!("Reloading configuration");
        let new_config = self
            .ipc
            .read_config()
            .map_err(ServiceError::Shared)?;

        {
            let mut hw = self.hardware.lock().await;
            hw.configure_thermal(&new_config.thermal)?;
            hw.configure_cpu_frequencies(
                new_config.cpu.min_freq_khz,
                new_config.cpu.max_freq_khz,
            )?;
            let ac = hw.get_ac_status().unwrap_or(false);
            let governor = if ac {
                &new_config.cpu.governor_ac
            } else {
                &new_config.cpu.governor_battery
            };
            hw.set_cpu_governor(governor)?;
        }

        {
            let mut cfg = self.config.write().await;
            *cfg = new_config;
        }

        let _ = self.ipc.log_event(
            EventType::ConfigChanged,
            "Configuration reloaded".to_string(),
            None,
        );
        Ok(())
    }
}