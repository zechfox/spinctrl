use clap::Parser;
use std::fs;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};

use nix::unistd::{chown, Group, Gid, Uid};
use spinctrl_service::error::{ServiceError, ServiceResult};
use spinctrl_service::hardware::{EctoolBackend, HardwareBackend};
use spinctrl_service::service::Service;

#[derive(Parser)]
#[command(name = "spinctrl-service")]
#[command(version = env!("CARGO_PKG_VERSION"), long_version = concat!(env!("CARGO_PKG_VERSION"), " (", env!("SPINCTRL_GIT_INFO"), ")"))]
struct Cli {
    /// Dry-run mode: mock hardware, don't touch real hardware
    #[arg(long)]
    dry_run: bool,
}

const ETC_CONFIG: &str = "/etc/spinctrl/config.json";
const VAR_CONFIG_STATUS: &str = "/var/lib/spinctrl/config_status.json";

#[tokio::main]
async fn main() -> ServiceResult<()> {
    let cli = Cli::parse();

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    if !cli.dry_run {
        check_dependencies()?;
    }

    let ipc = shared::IpcManager::new();
    init_ipc(&ipc)?;

    let config = load_config(&ipc, cli.dry_run)?;

    let hardware: Box<dyn HardwareBackend> = if cli.dry_run {
        log::info!("Dry-run mode: using mock hardware");
        Box::new(spinctrl_service::hardware::MockBackend::new())
    } else {
        Box::new(EctoolBackend)
    };

    let service = Service::new(ipc, config, hardware, cli.dry_run);
    service.run().await?;

    Ok(())
}

fn check_dependencies() -> ServiceResult<()> {
    let mut missing = Vec::new();
    for cmd in &["ectool", "cpupower"] {
        if std::process::Command::new("which")
            .arg(cmd)
            .output()
            .map_or(true, |o| !o.status.success())
        {
            missing.push(*cmd);
        }
    }
    if !missing.is_empty() {
        log::error!("Missing dependencies: {missing:?}");
        return Err(ServiceError::Config(format!(
            "Missing dependencies: {missing:?}"
        )));
    }
    Ok(())
}

fn init_ipc(ipc: &shared::IpcManager) -> ServiceResult<()> {
    let status_path = ipc.get_status_path();
    let runtime_dir = status_path
        .parent()
        .ok_or_else(|| ServiceError::Config("Cannot determine runtime directory".to_string()))?;

    if !runtime_dir.exists() {
        fs::create_dir_all(runtime_dir)?;
    }

    let spinctrl_gid = Group::from_name("spinctrl")
        .map_err(|e| {
            log::warn!("Failed to look up 'spinctrl' group: {e}");
            e
        })
        .ok()
        .flatten()
        .map(|g| g.gid);

    set_file_perms(runtime_dir, 0o2750, spinctrl_gid)?;

    let fifo_path = ipc.get_commands_path();
    if fifo_path.exists() && !is_fifo(&fifo_path) {
        log::warn!("{} exists but is not a FIFO; recreating", fifo_path.display());
        fs::remove_file(&fifo_path)?;
    }
    if !fifo_path.exists() {
        nix::unistd::mkfifo(
            &fifo_path,
            nix::sys::stat::Mode::S_IWUSR | nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IRGRP | nix::sys::stat::Mode::S_IWGRP,
        )
        .map_err(|e| ServiceError::Config(format!("Failed to create FIFO: {e}")))?;
    }
    set_file_perms(&fifo_path, 0o620, spinctrl_gid)?;

    let events_path = ipc.get_events_path();
    if !events_path.exists() {
        fs::File::create(&events_path)?;
    }
    set_file_perms(&events_path, 0o640, spinctrl_gid)?;

    Ok(())
}

fn set_file_perms(path: &Path, mode: u32, gid: Option<Gid>) -> ServiceResult<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    if let Some(gid) = gid {
        chown(path, Some(Uid::from_raw(0)), Some(gid))
            .map_err(|e| ServiceError::Config(format!("Failed to chown {}: {e}", path.display())))?;
    }
    Ok(())
}

fn is_fifo(path: &Path) -> bool {
    fs::metadata(path)
        .map(|m| m.file_type().is_fifo())
        .unwrap_or(false)
}

fn load_config(ipc: &shared::IpcManager, _dry_run: bool) -> ServiceResult<shared::Config> {
    let var_path = PathBuf::from(VAR_CONFIG_STATUS);

    if var_path.exists() {
        let content = fs::read_to_string(&var_path).map_err(ServiceError::from)?;
        match shared::Config::from_json(&content) {
            Ok(config) => return Ok(config),
            Err(e) => log::warn!("config_status.json is invalid: {e}, falling back to /etc"),
        }
    }

    let etc_path = PathBuf::from(ETC_CONFIG);
    if etc_path.exists() {
        if let Ok(content) = fs::read_to_string(&etc_path) {
            if let Ok(config) = shared::Config::from_json(&content) {
                let _ = ipc.write_config(&config);
                return Ok(config);
            }
        }
    }

    log::warn!("No config file found, using defaults");
    let config = shared::Config::default();
    let _ = ipc.write_config(&config);
    Ok(config)
}