use std::path::{Path, PathBuf};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::fs::OpenOptionsExt;
use nix::sys::stat::Mode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::error::{SpinCtrlError, Result};

/// Default runtime directory for IPC state (status, fifo, events)
pub const DEFAULT_IPC_DIR: &str = "/var/lib/spinctrl";

/// Path of the persistent runtime config the service writes and the TUI reads.
/// On boot the service reads `/etc/spinctrl/config.json` (read-only factory
/// defaults) then overrides with this file if it exists (the persisted runtime
/// config from prior `apply_config` pushes). The TUI reads this file for
/// display — it never reads `/etc`. The service writes this file on every
/// `apply_config` (persisting pushes across restarts). Lives in `/var/lib`
/// (FHS: variable, service-owned state), writable under `ReadWritePaths`.
pub const DEFAULT_CONFIG_PATH: &str = "/var/lib/spinctrl/config_status.json";

/// IPC file names within the runtime directory
pub const CONFIG_FILE: &str = "config.json";
pub const STATUS_FILE: &str = "status.json";
pub const COMMANDS_FIFO: &str = "commands.fifo";
pub const EVENTS_LOG: &str = "events.log";

/// System status information from the bash service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemStatus {
    pub battery: BatteryStatus,
    pub power: PowerStatus,
    pub thermal: Option<ThermalStatus>,
    pub timestamp: DateTime<Utc>,
    pub service_pid: u32,
}

/// Battery status information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatteryStatus {
    pub capacity: u8,
    pub charging: bool,
    pub threshold_active: bool,
    pub ac_connected: bool,
    /// Battery State-of-Health as a percentage (0-100), clamped.
    /// `None` when unavailable (no `energy_full`/`energy_full_design`
    /// in sysfs and `ectool battery` fallback failed or returned no data).
    #[serde(default)]
    pub health: Option<u8>,
    /// Battery cycle count. `None` when the sysfs `cycle_count` file is
    /// absent, unreadable, or zero (kernel ABI: 0 means "not available").
    #[serde(default)]
    pub cycle_count: Option<u32>,
}

/// Power management status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerStatus {
    pub ac_connected: bool,
    pub cpu_governor: String,
    pub cpu_freq_khz: Option<u32>,
}

/// Thermal status information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThermalStatus {
    pub zones: Vec<ThermalZone>,
}

/// Individual thermal zone data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThermalZone {
    pub id: u8,
    pub temperature: i32,
    pub trip_points: Vec<i32>,
}

/// Command types that can be sent to the service
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum Command {
    ForceCharge,
    StopCharge,
    /// Push the full config JSON to the service at runtime. The service
    /// applies it immediately (thermal/governor/threshold) and persists it
    /// to `config_status.json` so changes survive restarts.
    ApplyConfig(String),
    ReloadConfig,
    Shutdown,
}

/// Event log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEntry {
    pub timestamp: DateTime<Utc>,
    pub event_type: EventType,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

/// Types of events that can be logged
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    ConfigChanged,
    CommandExecuted,
    HardwareAction,
    Error,
    ServiceStart,
    ServiceStop,
}

/// IPC manager for handling file-based communication.
///
/// Configuration is read from a system-wide path (`/etc/spinctrl/config.json`
/// by default) owned root:spinctrl 0640. Runtime state (status, FIFO,
/// events) lives in a separate runtime directory (`/var/lib/spinctrl` by
/// default) owned root:spinctrl 0750. The TUI writes config changes by
/// pushing `Command::ApplyConfig` onto the FIFO, never by writing the
/// config file directly.
pub struct IpcManager {
    config_path: PathBuf,
    runtime_dir: PathBuf,
}

impl IpcManager {
    /// Create a new IPC manager pointing at the production paths:
    /// `/etc/spinctrl/config.json` and `/var/lib/spinctrl/`.
    pub fn new() -> Self {
        Self {
            config_path: PathBuf::from(DEFAULT_CONFIG_PATH),
            runtime_dir: PathBuf::from(DEFAULT_IPC_DIR),
        }
    }

    /// Create a new IPC manager with explicit config path and runtime dir.
    pub fn with_paths<P1, P2>(config_path: P1, runtime_dir: P2) -> Self
    where
        P1: AsRef<Path>,
        P2: AsRef<Path>,
    {
        Self {
            config_path: config_path.as_ref().to_path_buf(),
            runtime_dir: runtime_dir.as_ref().to_path_buf(),
        }
    }

    /// Create a new IPC manager rooted at a single directory. Both the
    /// config file and runtime state resolve under `dir`. Intended for
    /// tests where isolation matters more than the production split.
    pub fn with_dir<P: AsRef<Path>>(dir: P) -> Self {
        let dir_buf = dir.as_ref().to_path_buf();
        Self {
            config_path: dir_buf.join(CONFIG_FILE),
            runtime_dir: dir_buf,
        }
    }

    /// Initialize the runtime directory structure with proper permissions.
    /// Intended for test isolation; production paths are owned by
    /// `install.sh` and the systemd unit.
    pub fn initialize(&self) -> Result<()> {
        self.create_directory_structure()?;
        self.set_permissions()?;
        self.create_default_config()?;
        Ok(())
    }

    fn create_directory_structure(&self) -> Result<()> {
        if !self.runtime_dir.exists() {
            fs::create_dir_all(&self.runtime_dir).map_err(|e| {
                SpinCtrlError::DirectoryCreation(format!("{}: {}", self.runtime_dir.display(), e))
            })?;
        }
        Ok(())
    }

    fn set_permissions(&self) -> Result<()> {
        let metadata = fs::metadata(&self.runtime_dir)?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o750);
        fs::set_permissions(&self.runtime_dir, permissions)?;
        Ok(())
    }

    fn create_default_config(&self) -> Result<()> {
        let config_path = self.get_config_path();
        if !config_path.exists() {
            let default_config = Config::default();
            self.write_config(&default_config)?;
        }
        Ok(())
    }

    pub fn get_config_path(&self) -> PathBuf { self.config_path.clone() }
    pub fn get_status_path(&self) -> PathBuf { self.runtime_dir.join(STATUS_FILE) }
    pub fn get_commands_path(&self) -> PathBuf { self.runtime_dir.join(COMMANDS_FIFO) }
    pub fn get_events_path(&self) -> PathBuf { self.runtime_dir.join(EVENTS_LOG) }

    /// Generic JSON write method
    fn write_json<T: Serialize>(&self, path: &Path, data: &T) -> Result<()> {
        let json = serde_json::to_string_pretty(data)?;
        self.write_json_atomic(path, &json)
    }
    
    /// Generic JSON read method
    fn read_json<T: for<'de> Deserialize<'de>>(&self, path: &Path) -> Result<Option<T>> {
        // Use metadata rather than Path::exists so an EACCES on the path is
        // surfaced as PermissionDenied instead of being silently masked as
        // "missing" (exists() returns false on EACCES), which would hide
        // group-membership problems from the TUI (req 8.9).
        match fs::metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                return Err(SpinCtrlError::PermissionDenied(path.display().to_string()));
            }
            Err(_) => return Ok(None),
            Ok(_) => {}
        }
        let json = self.read_file(path)?;
        Ok(Some(serde_json::from_str(&json)?))
    }
    
    /// Write configuration atomically
    pub fn write_config(&self, config: &Config) -> Result<()> {
        config.validate()?;
        self.write_json(&self.get_config_path(), config)
    }
    
    /// Read configuration from file
    pub fn read_config(&self) -> Result<Config> {
        self.read_json(&self.get_config_path())?
            .map_or_else(|| Ok(Config::default()), Ok)
    }
    
    /// Write system status atomically
    pub fn write_status(&self, status: &SystemStatus) -> Result<()> {
        self.write_json(&self.get_status_path(), status)
    }
    
    /// Read system status from file
    pub fn read_status(&self) -> Result<Option<SystemStatus>> {
        self.read_json(&self.get_status_path())
    }
    
    /// Send a command to the bash service via FIFO. Opens the FIFO with
    /// O_NONBLOCK so that, when the service is not running (no reader
    /// attached), the open fails immediately with ENXIO instead of blocking
    /// the caller indefinitely (which would hang the TUI on the `f`/`s`
    /// keys when the service is down).
    pub fn send_command(&self, command: &Command) -> Result<()> {
        let command_str = self.command_to_string(command);
        let fifo_path = self.get_commands_path();

        // Create FIFO if it doesn't exist
        if !fifo_path.exists() {
            nix::unistd::mkfifo(&fifo_path, Mode::S_IWUSR | Mode::S_IRUSR | Mode::S_IRGRP)
                .map_err(|e| SpinCtrlError::ServiceComm(format!("Failed to create FIFO: {}", e)))?;
        }

        // O_NONBLOCK: opening a FIFO for write-only with no reader attached
        // fails with ENXIO immediately rather than blocking.
        let mut file = OpenOptions::new()
            .write(true)
            .custom_flags(nix::fcntl::OFlag::O_NONBLOCK.bits() as i32)
            .open(&fifo_path)
            .map_err(|e| {
                if e.raw_os_error() == Some(nix::errno::Errno::ENXIO as i32) {
                    SpinCtrlError::ServiceComm("Service not running (no FIFO reader)".to_string())
                } else {
                    SpinCtrlError::ServiceComm(format!("Failed to open FIFO: {}", e))
                }
            })?;

        writeln!(file, "{}", command_str)
            .map_err(|e| SpinCtrlError::ServiceComm(format!("Failed to write command: {}", e)))?;

        Ok(())
    }
    
    /// Append an event to the events log
    pub fn log_event(&self, event_type: EventType, message: String, details: Option<serde_json::Value>) -> Result<()> {
        let entry = EventEntry {
            timestamp: Utc::now(),
            event_type,
            message,
            details,
        };
        
        let json = serde_json::to_string(&entry)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.get_events_path())?;
        
        writeln!(file, "{}", json)?;
        Ok(())
    }
    
    /// Read recent events from the log
    pub fn read_recent_events(&self, limit: usize) -> Result<Vec<EventEntry>> {
        let path = self.get_events_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        
        let content = self.read_file(&path)?;
        let mut events: Vec<EventEntry> = content
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        
        // Return the most recent events
        events.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        events.truncate(limit);
        Ok(events)
    }
    
    /// Write JSON content atomically using a temporary file
    fn write_json_atomic(&self, path: &Path, content: &str) -> Result<()> {
        let temp_path = path.with_extension("tmp");
        
        // Write to temporary file
        {
            let mut file = File::create(&temp_path)?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
        }
        
        // Set proper permissions (640 - rw-r-----)
        let metadata = fs::metadata(&temp_path)?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o640);
        fs::set_permissions(&temp_path, permissions)?;
        
        // Atomically move temporary file to final location
        fs::rename(&temp_path, path)?;
        
        Ok(())
    }
    
    /// Read entire file content
    fn read_file(&self, path: &Path) -> Result<String> {
        let mut file = File::open(path)
            .map_err(|e| SpinCtrlError::FileNotFound(format!("{}: {}", path.display(), e)))?;
        
        let mut content = String::new();
        file.read_to_string(&mut content)?;
        Ok(content)
    }
    
    /// Convert command to string format for FIFO communication
    fn command_to_string(&self, command: &Command) -> String {
        match command {
            Command::ForceCharge => "force_charge".to_string(),
            Command::StopCharge => "stop_charge".to_string(),
            Command::ApplyConfig(json) => format!("apply_config:{}", json),
            Command::ReloadConfig => "reload_config".to_string(),
            Command::Shutdown => "shutdown".to_string(),
        }
    }
    
    /// Check if the IPC directory is properly set up
    pub fn is_initialized(&self) -> bool {
        self.runtime_dir.exists() && self.get_config_path().exists()
    }
    
    /// Remove transient IPC files (status, temp files, FIFO) for a clean
    /// shutdown. The events log is preserved as an audit trail.
    pub fn cleanup(&self) -> Result<()> {
        for entry in fs::read_dir(&self.runtime_dir)? {
            let entry = entry?;
            let path = entry.path();
            if let Some(extension) = path.extension() {
                if extension == "tmp" {
                    let _ = fs::remove_file(path);
                }
            }
        }

        let status_path = self.get_status_path();
        if status_path.exists() {
            let _ = fs::remove_file(status_path);
        }

        let fifo_path = self.get_commands_path();
        if fifo_path.exists() {
            let _ = fs::remove_file(fifo_path);
        }

        Ok(())
    }
}

impl Default for IpcManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    
    fn create_test_ipc_manager() -> (IpcManager, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let ipc = IpcManager::with_dir(temp_dir.path());
        (ipc, temp_dir)
    }
    
    #[test]
    fn test_ipc_initialization() {
        let (ipc, _temp_dir) = create_test_ipc_manager();
        assert!(ipc.initialize().is_ok());
        assert!(ipc.is_initialized());
    }
    
    #[test]
    fn test_config_operations() {
        let (ipc, _temp_dir) = create_test_ipc_manager();
        ipc.initialize().unwrap();
        
        // Test writing and reading config
        let mut config = Config::default();
        config.battery.threshold = 85;
        
        assert!(ipc.write_config(&config).is_ok());
        let read_config = ipc.read_config().unwrap();
        assert_eq!(read_config.battery.threshold, 85);
    }
    
    #[test]
    fn test_status_operations() {
        let (ipc, _temp_dir) = create_test_ipc_manager();
        ipc.initialize().unwrap();
        
        let status = SystemStatus {
            battery: BatteryStatus {
                capacity: 75,
                charging: true,
                threshold_active: false,
                ac_connected: true,
                health: Some(92),
                cycle_count: Some(109),
            },
            power: PowerStatus {
                ac_connected: true,
                cpu_governor: "performance".to_string(),
                cpu_freq_khz: Some(2400000),
            },
            thermal: None,
            timestamp: Utc::now(),
            service_pid: 1234,
        };
        
        assert!(ipc.write_status(&status).is_ok());
        let read_status = ipc.read_status().unwrap();
        assert!(read_status.is_some());
        let read_battery = &read_status.unwrap().battery;
        assert_eq!(read_battery.capacity, 75);
        assert_eq!(read_battery.health, Some(92));
        assert_eq!(read_battery.cycle_count, Some(109));
    }
    
    #[test]
    fn test_event_logging() {
        let (ipc, _temp_dir) = create_test_ipc_manager();
        ipc.initialize().unwrap();
        
        // Log some events
        assert!(ipc.log_event(
            EventType::ConfigChanged,
            "Battery threshold changed".to_string(),
            Some(serde_json::json!({"old": 80, "new": 85}))
        ).is_ok());
        
        assert!(ipc.log_event(
            EventType::HardwareAction,
            "CPU governor changed".to_string(),
            None
        ).is_ok());
        
        // Read events back
        let events = ipc.read_recent_events(10).unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0].event_type, EventType::HardwareAction));
        assert!(matches!(events[1].event_type, EventType::ConfigChanged));
    }
    
    #[test]
    fn test_command_serialization() {
        let ipc = IpcManager::new();
        
        assert_eq!(
            ipc.command_to_string(&Command::ForceCharge),
            "force_charge"
        );
        
        assert_eq!(
            ipc.command_to_string(&Command::StopCharge),
            "stop_charge"
        );
        
        assert_eq!(
            ipc.command_to_string(&Command::ApplyConfig(r#"{"battery":{"threshold":85}}"#.into())),
            r#"apply_config:{"battery":{"threshold":85}}"#
        );
    }

    #[test]
    fn test_with_dir_colocates_config_and_runtime() {
        let temp = TempDir::new().unwrap();
        let ipc = IpcManager::with_dir(temp.path());
        assert_eq!(ipc.get_config_path(), temp.path().join(CONFIG_FILE));
        assert_eq!(ipc.get_status_path(), temp.path().join(STATUS_FILE));
        assert_eq!(ipc.get_commands_path(), temp.path().join(COMMANDS_FIFO));
        assert_eq!(ipc.get_events_path(), temp.path().join(EVENTS_LOG));
    }

    #[test]
    fn test_with_paths_separates_config_and_runtime() {
        let config_temp = TempDir::new().unwrap();
        let runtime_temp = TempDir::new().unwrap();
        let ipc = IpcManager::with_paths(
            config_temp.path().join(CONFIG_FILE),
            runtime_temp.path(),
        );
        assert_eq!(ipc.get_config_path(), config_temp.path().join(CONFIG_FILE));
        assert_eq!(ipc.get_status_path(), runtime_temp.path().join(STATUS_FILE));
        assert_eq!(ipc.get_commands_path(), runtime_temp.path().join(COMMANDS_FIFO));
        assert_eq!(ipc.get_events_path(), runtime_temp.path().join(EVENTS_LOG));
    }

    #[test]
    fn test_new_uses_production_paths() {
        let ipc = IpcManager::new();
        assert_eq!(ipc.get_config_path(), PathBuf::from(DEFAULT_CONFIG_PATH));
        assert_eq!(ipc.get_status_path(), PathBuf::from(DEFAULT_IPC_DIR).join(STATUS_FILE));
        assert_eq!(ipc.get_commands_path(), PathBuf::from(DEFAULT_IPC_DIR).join(COMMANDS_FIFO));
        assert_eq!(ipc.get_events_path(), PathBuf::from(DEFAULT_IPC_DIR).join(EVENTS_LOG));
    }

    #[test]
    fn test_apply_config_round_trip_via_separated_paths() {
        let config_temp = TempDir::new().unwrap();
        let runtime_temp = TempDir::new().unwrap();
        let ipc = IpcManager::with_paths(
            config_temp.path().join(CONFIG_FILE),
            runtime_temp.path(),
        );
        ipc.initialize().unwrap();

        let mut config = Config::default();
        config.battery.threshold = 85;
        let json = config.to_json_compact().unwrap();
        assert!(!json.contains('\n'), "compact JSON must be single-line for the FIFO wire format");
        let wire = ipc.command_to_string(&Command::ApplyConfig(json.clone()));
        assert!(wire.starts_with("apply_config:"));
        assert!(!wire.contains('\n'), "apply_config wire must be a single line");
        assert!(wire.contains("\"threshold\":85"));
    }

    #[test]
    fn test_is_initialized_false_before_init() {
        let temp = TempDir::new().unwrap();
        let ipc = IpcManager::with_dir(temp.path());
        assert!(!ipc.is_initialized());
    }

    #[test]
    fn test_read_status_returns_none_without_init() {
        let temp = TempDir::new().unwrap();
        let ipc = IpcManager::with_dir(temp.path());
        assert!(ipc.read_status().unwrap().is_none());
    }

    #[test]
    fn test_read_recent_events_empty_without_log() {
        let temp = TempDir::new().unwrap();
        let ipc = IpcManager::with_dir(temp.path());
        let events = ipc.read_recent_events(10).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn test_command_to_string_all_variants() {
        let ipc = IpcManager::new();
        assert_eq!(ipc.command_to_string(&Command::ForceCharge), "force_charge");
        assert_eq!(ipc.command_to_string(&Command::StopCharge), "stop_charge");
        assert_eq!(
            ipc.command_to_string(&Command::ApplyConfig(r#"{"battery":{"threshold":85}}"#.into())),
            r#"apply_config:{"battery":{"threshold":85}}"#
        );
        assert_eq!(ipc.command_to_string(&Command::ReloadConfig), "reload_config");
        assert_eq!(ipc.command_to_string(&Command::Shutdown), "shutdown");
    }

    #[test]
    fn test_cleanup_ok_after_init() {
        let temp = TempDir::new().unwrap();
        let ipc = IpcManager::with_dir(temp.path());
        ipc.initialize().unwrap();
        assert!(ipc.cleanup().is_ok());
    }

    #[test]
    fn test_default_matches_new() {
        let new = IpcManager::new();
        let default = IpcManager::default();
        assert_eq!(new.get_config_path(), default.get_config_path());
    }

    #[test]
    fn test_send_command_no_reader_returns_error() {
        // No service reading the FIFO; the O_NONBLOCK open must fail with
        // ENXIO immediately instead of blocking. Regression guard: if someone
        // reverts to a blocking open, this test will hang.
        let temp = TempDir::new().unwrap();
        let ipc = IpcManager::with_dir(temp.path());
        match ipc.send_command(&Command::ForceCharge) {
            Err(e) => assert!(
                e.to_string().contains("no FIFO reader") || e.to_string().contains("Service not running"),
                "expected no-reader error, got: {}", e
            ),
            Ok(_) => panic!("send_command must not succeed with no FIFO reader attached"),
        }
    }
}