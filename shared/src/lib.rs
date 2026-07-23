pub mod config;
pub mod error;
pub mod ipc;

pub use config::{
    BatteryConfig, Config, CpuConfig, CpuGovernor, StandardGovernor, ThermalConfig,
    ThermalProfile, available_cpu_frequencies,
};
pub use error::{Result, SpinCtrlError};
pub use ipc::{
    BatteryStatus, Command, EventEntry, EventType, IpcManager, PowerStatus, SystemStatus,
    ThermalStatus, ThermalZone,
    CONFIG_FILE, COMMANDS_FIFO, DEFAULT_CONFIG_PATH, DEFAULT_IPC_DIR, EVENTS_LOG, STATUS_FILE,
};