use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, RwLock};

use crate::ac_monitor::AcMonitor;
use crate::command_processor::{start_fifo_reader, CommandDispatcher};
use crate::config_watcher::ConfigWatcher;
use crate::error::ServiceResult;
use crate::hardware::HardwareBackend;
use crate::status_writer::StatusWriter;

pub struct Service {
    pub ipc: Arc<shared::IpcManager>,
    pub config: Arc<RwLock<shared::Config>>,
    pub force_charge: Arc<AtomicBool>,
    pub hardware: Arc<Mutex<Box<dyn HardwareBackend>>>,
    pub shutdown: Arc<AtomicBool>,
    pub dry_run: bool,
}

impl Service {
    #[must_use]
    pub fn new(
        ipc: shared::IpcManager,
        config: shared::Config,
        hardware: Box<dyn HardwareBackend>,
        dry_run: bool,
    ) -> Self {
        Self {
            ipc: Arc::new(ipc),
            config: Arc::new(RwLock::new(config)),
            force_charge: Arc::new(AtomicBool::new(false)),
            hardware: Arc::new(Mutex::new(hardware)),
            shutdown: Arc::new(AtomicBool::new(false)),
            dry_run,
        }
    }

    /// Run the service: start all tasks and wait for shutdown signal.
    ///
    /// # Errors
    /// Returns `ServiceError` if IPC initialization fails.
    pub async fn run(&self) -> ServiceResult<()> {
        // Log service start
        let _ = self.ipc.log_event(
            shared::EventType::ServiceStart,
            "SpinCtrl service started".to_string(),
            Some(serde_json::json!({"pid": std::process::id()})),
        );

        // Apply initial hardware state
        self.apply_initial_state().await;

        // Start tasks
        let ac_monitor = AcMonitor::new(
            self.hardware.clone(),
            self.config.clone(),
            self.force_charge.clone(),
            self.shutdown.clone(),
        );

        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        let fifo_path = self.ipc.get_commands_path();
        start_fifo_reader(fifo_path, cmd_tx, self.shutdown.clone());

        let dispatcher = CommandDispatcher::new(
            self.hardware.clone(),
            self.config.clone(),
            self.force_charge.clone(),
            self.shutdown.clone(),
            self.ipc.clone(),
        );

        let config_watcher = ConfigWatcher::new(
            PathBuf::from("/etc/spinctrl/config.json"),
            self.config.clone(),
            self.hardware.clone(),
            self.shutdown.clone(),
            self.ipc.clone(),
        );

        let status_writer = StatusWriter::new(
            self.hardware.clone(),
            self.shutdown.clone(),
            self.ipc.clone(),
        );

        let ac_handle = tokio::spawn(async move { ac_monitor.run().await });
        let cmd_handle = tokio::spawn(async move { dispatcher.run(cmd_rx).await });
        let watcher_handle = tokio::spawn(async move { config_watcher.run().await });
        let status_handle = tokio::spawn(async move { status_writer.run().await });

        // Wait for shutdown signal
        self.wait_for_shutdown().await;

        self.shutdown.store(true, Ordering::Relaxed);

        let _ = tokio::time::timeout(Duration::from_secs(3), async {
            let _ = tokio::join!(ac_handle, cmd_handle, watcher_handle, status_handle);
        })
        .await;

        self.cleanup().await;

        Ok(())
    }

    async fn apply_initial_state(&self) {
        let config = self.config.read().await;
        let mut hw = self.hardware.lock().await;

        if let Err(e) = hw.configure_thermal(&config.thermal) {
            log::error!("Failed to apply initial thermal config: {e}");
        }

        if let Err(e) = hw.configure_cpu_frequencies(config.cpu.min_freq_khz, config.cpu.max_freq_khz)
        {
            log::error!("Failed to apply initial CPU frequencies: {e}");
        }

        let ac = hw.get_ac_status().unwrap_or(false);
        let governor = if ac {
            &config.cpu.governor_ac
        } else {
            &config.cpu.governor_battery
        };
        if let Err(e) = hw.set_cpu_governor(governor) {
            log::error!("Failed to set initial governor: {e}");
        }

        if ac {
            let capacity = hw.get_battery_capacity().unwrap_or(0);
            if capacity >= config.battery.threshold {
                log::info!("Initial: battery at threshold, setting charge idle");
                let _ = hw.set_charge_control(crate::hardware::ChargeMode::Idle);
            } else {
                let _ = hw.set_charge_control(crate::hardware::ChargeMode::Normal);
            }
        }
    }

    async fn wait_for_shutdown(&self) {
        let kind = tokio::signal::unix::SignalKind::terminate();
        let mut sigterm = match tokio::signal::unix::signal(kind) {
            Ok(signal) => signal,
            Err(e) => {
                log::warn!("Failed to register SIGTERM handler: {e}");
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };

        tokio::select! {
            _ = sigterm.recv() => {
                log::info!("Received SIGTERM");
            }
            _ = tokio::signal::ctrl_c() => {
                log::info!("Received SIGINT");
            }
        }
    }

    async fn cleanup(&self) {
        log::info!("Cleaning up");

        // Restore charge control to normal
        let mut hw = self.hardware.lock().await;
        if let Err(e) = hw.set_charge_control(crate::hardware::ChargeMode::Normal) {
            log::error!("Failed to restore charge control during cleanup: {e}");
        }
        drop(hw);

        // Cleanup IPC files
        if let Err(e) = self.ipc.cleanup() {
            log::error!("Failed to cleanup IPC: {e}");
        }

        let _ = self.ipc.log_event(
            shared::EventType::ServiceStop,
            "SpinCtrl service stopped".to_string(),
            None,
        );
    }
}