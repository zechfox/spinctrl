pub mod state;
pub mod handlers;
pub mod editing;
pub mod events_tab;
pub mod explain;
pub mod ui;

use std::sync::Arc;
use std::time::{Duration, Instant};
use crossterm::event::{self, Event};
use notify::{RecursiveMode, Watcher};
use ratatui::{
    backend::Backend,
    Terminal,
};
use shared::{
    Config, CpuGovernor, EventEntry, EventType, IpcManager, SystemStatus,
};
use crate::error::Result;
use crate::app::explain::explain_error;
use crate::app::editing::EditField;
use crate::app::state::{AppMode, Tab};

pub struct App {
    pub should_quit: bool,
    pub mode: AppMode,
    pub selected_tab: Tab,
    pub config: Config,
    /// Snapshot of `config` taken when entering Editing mode; restored on Esc
    /// so the user can abandon all in-session edits in one keystroke.
    pub config_backup: Config,
    pub status: Option<SystemStatus>,
    pub events: Vec<EventEntry>,
    /// Active Events-tab filter. `None` shows all events; `Some(t)` shows
    /// only entries whose `event_type` matches `t`. Cycled with `f` on the
    /// Events tab; cleared with `a` (or by cycling past the last variant).
    pub event_filter: Option<EventType>,
    pub ipc: IpcManager,
    pub last_update: Instant,
    pub update_interval: Duration,
    pub service_available: bool,
    pub error_message: Option<String>,

    // Input state for editing
    pub editing_field: Option<EditField>,

    // UI state
    pub scroll_offset: usize,
    pub selected_item: usize,

    // File-watcher refresh flag (set by notify callback, checked by run loop)
    pub needs_refresh: Arc<std::sync::atomic::AtomicBool>,

    // Cached at construction time (H6)
    pub available_governors: Vec<CpuGovernor>,
    pub freq_range: (u32, u32),
}

impl App {
    pub fn new() -> Result<Self> {
        Self::with_ipc(IpcManager::new())
    }

    /// Construct an App with a custom `IpcManager` (for tests: use a temp path
    /// so production `config_status` doesn't pollute the test's `Config::default()`).
    pub fn with_ipc(ipc: IpcManager) -> Result<Self> {
        let config = ipc.read_config().unwrap_or_default();
        let config_backup = config.clone();
        let available_governors = Config::get_available_governors();
        let freq_range = Self::compute_freq_range();

        Ok(Self {
            should_quit: false,
            mode: AppMode::Monitoring,
            selected_tab: Tab::Status,
            config,
            config_backup,
            status: None,
            events: Vec::new(),
            event_filter: None,
            ipc,
            last_update: Instant::now(),
            update_interval: Duration::from_secs(2),
            service_available: false,
            error_message: None,
            editing_field: None,
            scroll_offset: 0,
            selected_item: 0,
            needs_refresh: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            available_governors,
            freq_range,
        })
    }

    /// Start a `notify` file watcher on the IPC runtime directory.
    /// Returns the watcher handle (must be kept alive) or `None` on failure.
    /// Only `config_status.json`, `status.json`, and `events.log` changes
    /// trigger a refresh — FIFO writes are excluded to prevent stale-config
    /// overwrite of optimistic updates from just-sent `f`/`s` commands.
    fn start_file_watcher(&self) -> Option<notify::RecommendedWatcher> {
        let watch_dir = self.ipc.get_status_path().parent()?.to_path_buf();
        if !watch_dir.exists() {
            log::debug!(
                "Watch dir {} does not exist yet; skipping watcher",
                watch_dir.display()
            );
            return None;
        }
        let flag = Arc::clone(&self.needs_refresh);
        let config_path = self.ipc.get_config_path();
        let status_path = self.ipc.get_status_path();
        let events_path = self.ipc.get_events_path();
        let watcher = notify::recommended_watcher(
            move |res: std::result::Result<notify::Event, notify::Error>| {
                if let Ok(event) = res {
                    // Only refresh for data-file changes, not FIFO writes.
                    // Empty paths = unknown source — refresh as a fallback.
                    let should_refresh = event.paths.is_empty()
                        || event.paths.iter().any(|p| {
                            p == &config_path || p == &status_path || p == &events_path
                        });
                    if should_refresh {
                        flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            },
        );
        match watcher {
            Ok(mut w) => {
                if let Err(e) = w.watch(&watch_dir, RecursiveMode::NonRecursive) {
                    log::warn!("Failed to watch {}: {}", watch_dir.display(), e);
                    return None;
                }
                log::info!("Watching {} for file changes", watch_dir.display());
                Some(w)
            }
            Err(e) => {
                log::warn!("Failed to create file watcher: {e}");
                None
            }
        }
    }

    pub async fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        let _watcher = self.start_file_watcher();

        self.update_data().await;

        loop {
            terminal.draw(|f| self.ui(f))?;

            let watcher_triggered = self.needs_refresh.swap(false, std::sync::atomic::Ordering::Relaxed);
            if watcher_triggered {
                self.update_data().await;
                self.last_update = Instant::now();
            }

            if crossterm::event::poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    self.handle_key_event(key).await?;
                }
            }

            if self.last_update.elapsed() >= self.update_interval {
                self.update_data().await;
                self.last_update = Instant::now();
            }

            if self.should_quit {
                break;
            }
        }

        Ok(())
    }

    pub async fn update_data(&mut self) {
        match self.ipc.read_status() {
            Ok(Some(status)) => {
                self.status = Some(status);
                self.service_available = true;
                self.error_message = None;
            }
            Ok(None) => {
                self.service_available = false;
                if self.error_message.is_none() {
                    self.error_message = Some("Service not running".to_string());
                }
            }
            Err(e) => {
                self.service_available = false;
                self.error_message =
                    Some(format!("Failed to read status: {}", explain_error(&e)));
            }
        }

        if let Ok(events) = self.ipc.read_recent_events(100) {
            self.events = events;
        }

        if let Ok(config) = self.ipc.read_config() {
            if self.mode != AppMode::Editing {
                self.config = config;
            }
        }
    }
}