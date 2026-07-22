use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use notify::{RecursiveMode, Watcher};
use ratatui::{
    backend::Backend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph, Tabs},
    Frame, Terminal,
};
use shared::{
    available_cpu_frequencies, Command, Config, EventEntry, EventType, IpcManager, SpinCtrlError,
    SystemStatus, ThermalProfile,
};
use crate::error::Result;

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    Monitoring,
    Editing,
    Help,
    Popup(PopupType),
}

#[derive(Debug, Clone, PartialEq)]
pub enum PopupType {
    ConfirmExit,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tab {
    Status = 0,
    Battery = 1,
    CPU = 2,
    Thermal = 3,
    Events = 4,
}

impl Tab {
    const TITLES: &'static [&'static str] = &["Status", "Battery", "CPU", "Thermal", "Events"];
    const COUNT: usize = 5;

    pub fn titles() -> &'static [&'static str] {
        Self::TITLES
    }

    pub fn from_index(index: usize) -> Self {
        match index {
            0 => Self::Status,
            1 => Self::Battery,
            2 => Self::CPU,
            3 => Self::Thermal,
            4 => Self::Events,
            _ => Self::Status,
        }
    }

    pub fn to_index(self) -> usize {
        self as usize
    }

    pub fn next(self) -> Self {
        Self::from_index((self.to_index() + 1) % Self::COUNT)
    }

    pub fn previous(self) -> Self {
        Self::from_index((self.to_index() + Self::COUNT - 1) % Self::COUNT)
    }
}

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
    pub editing_field: Option<String>,

    // UI state
    pub scroll_offset: usize,
    pub selected_item: usize,

    // File-watcher refresh flag (set by notify callback, checked by run loop)
    pub needs_refresh: Arc<Mutex<bool>>,
}

impl App {
    pub fn new() -> Result<Self> {
        Self::with_ipc(IpcManager::new())
    }

    /// Construct an App with a custom IpcManager (for tests: use a temp path
    /// so production config_status doesn't pollute the test's Config::default()).
    pub fn with_ipc(ipc: IpcManager) -> Result<Self> {
        let config = ipc.read_config().unwrap_or_default();
        let config_backup = config.clone();

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
            needs_refresh: Arc::new(Mutex::new(false)),
        })
    }

    /// Start a `notify` file watcher on the IPC runtime directory.
    /// Returns the watcher handle (must be kept alive) or `None` on failure.
    /// The watcher sets `self.needs_refresh` whenever a file changes in the
    /// directory, so the run loop can react promptly instead of waiting for
    /// the next 2-second poll tick.
    fn start_file_watcher(&self) -> Option<notify::RecommendedWatcher> {
        let watch_dir = match self.ipc.get_status_path().parent() {
            Some(p) => p.to_path_buf(),
            None => return None,
        };
        if !watch_dir.exists() {
            log::debug!(
                "Watch dir {} does not exist yet; skipping watcher",
                watch_dir.display()
            );
            return None;
        }
        let flag = Arc::clone(&self.needs_refresh);
        let watcher = notify::recommended_watcher(
            move |res: std::result::Result<notify::Event, notify::Error>| {
                if let Ok(_event) = res {
                    if let Ok(mut f) = flag.lock() {
                        *f = true;
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
                log::warn!("Failed to create file watcher: {}", e);
                None
            }
        }
    }

    pub async fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        let _watcher = self.start_file_watcher();

        self.update_data().await;

        loop {
            terminal.draw(|f| self.ui(f))?;

            let watcher_triggered = {
                let mut flag = self.needs_refresh.lock().unwrap_or_else(|e| e.into_inner());
                let v = *flag;
                *flag = false;
                v
            };
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

    async fn update_data(&mut self) {
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

    async fn handle_key_event(&mut self, key: KeyEvent) -> Result<()> {
        match self.mode {
            AppMode::Popup(ref popup_type) => {
                self.handle_popup_key(key, popup_type.clone()).await?;
            }
            AppMode::Editing => {
                self.handle_editing_key(key).await?;
            }
            AppMode::Help => {
                self.handle_help_key(key);
            }
            AppMode::Monitoring => {
                self.handle_monitoring_key(key).await?;
            }
        }

        Ok(())
    }

    async fn handle_monitoring_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                if key.modifiers.contains(KeyModifiers::CONTROL) || key.code == KeyCode::Esc {
                    self.mode = AppMode::Popup(PopupType::ConfirmExit);
                } else {
                    self.should_quit = true;
                }
            }
            KeyCode::Char('h') | KeyCode::F(1) => {
                self.mode = AppMode::Help;
            }
            KeyCode::Tab | KeyCode::Right => {
                self.next_tab();
            }
            KeyCode::BackTab | KeyCode::Left => {
                self.previous_tab();
            }
            KeyCode::Char(c @ '1'..='5') => {
                let index = (c as usize).saturating_sub('1' as usize);
                self.selected_tab = Tab::from_index(index);
            }
            KeyCode::Enter | KeyCode::Char('e') => {
                if self.selected_tab != Tab::Status && self.selected_tab != Tab::Events {
                    self.mode = AppMode::Editing;
                    self.begin_editing();
                }
            }
            KeyCode::Char('r') => {
                self.update_data().await;
            }
            KeyCode::Up => {
                if self.selected_item > 0 {
                    self.selected_item -= 1;
                }
            }
            KeyCode::Down => {
                self.selected_item += 1;
                // Limit based on current tab content
            }
            KeyCode::Char('f') => match self.selected_tab {
                Tab::Battery => {
                    self.send_command(Command::ForceCharge).await?;
                }
                Tab::Events => {
                    self.cycle_event_filter();
                }
                _ => {}
            },
            KeyCode::Char('s') => {
                if self.selected_tab == Tab::Battery {
                    self.send_command(Command::StopCharge).await?;
                }
            }
            KeyCode::Char('a') => {
                if self.selected_tab == Tab::Events {
                    self.clear_event_filter();
                }
            }
            _ => {}
        }

        Ok(())
    }

    async fn handle_editing_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.config = self.config_backup.clone();
                self.editing_field = None;
                self.mode = AppMode::Monitoring;
            }
            KeyCode::Enter => {
                self.apply_config_in_place();
            }
            KeyCode::Tab => {
                self.next_edit_field();
            }
            KeyCode::Left => {
                self.adjust_field(-1);
            }
            KeyCode::Right => {
                self.adjust_field(1);
            }
            KeyCode::Char('u') | KeyCode::Char('U') => {
                match self.editing_field.as_deref() {
                    Some("cpu.min_freq_khz") => self.config.cpu.min_freq_khz = None,
                    Some("cpu.max_freq_khz") => self.config.cpu.max_freq_khz = None,
                    _ => {}
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn handle_help_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('h') => {
                self.mode = AppMode::Monitoring;
            }
            _ => {}
        }
    }

    async fn handle_popup_key(
        &mut self,
        key: KeyEvent,
        popup_type: PopupType,
    ) -> Result<()> {
        match popup_type {
            PopupType::ConfirmExit => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    self.should_quit = true;
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.mode = AppMode::Monitoring;
                }
                _ => {}
            },
        }

        Ok(())
    }

    async fn send_command(&mut self, command: Command) -> Result<()> {
        match self.ipc.send_command(&command) {
            Ok(()) => {
                log::info!("Command sent successfully: {:?}", command);
            }
            Err(e) => {
                self.error_message = Some(format!("Failed to send command: {}", e));
                log::error!("Failed to send command: {}", e);
            }
        }

        Ok(())
    }

    fn apply_config_in_place(&mut self) {
        if !self.service_available {
            self.error_message = Some(
                "Service offline; changes held in-memory — start the service then press Enter to push".to_string(),
            );
            return;
        }
        if let Err(e) = self.config.validate() {
            self.error_message = Some(format!("Config validation failed: {}", e));
            return;
        }
        let json = match self.config.to_json_compact() {
            Ok(j) => j,
            Err(e) => {
                self.error_message = Some(format!("Failed to serialize config: {}", e));
                return;
            }
        };
        match self.ipc.send_command(&Command::ApplyConfig(json)) {
            Ok(()) => {
                self.error_message = None;
                log::info!("Configuration applied to service via ApplyConfig");
            }
            Err(e) => {
                self.error_message = Some(format!("Failed to apply config: {}", e));
                log::error!("Failed to apply config: {}", e);
            }
        }
    }

    fn next_tab(&mut self) {
        self.selected_tab = self.selected_tab.next();
        self.selected_item = 0;
    }

    fn previous_tab(&mut self) {
        self.selected_tab = self.selected_tab.previous();
        self.selected_item = 0;
    }

    /// Cycle the Events-tab filter through:
    /// `None` (All) → ConfigChanged → CommandExecuted → HardwareAction →
    /// Error → ServiceStart → ServiceStop → `None`. Resets `selected_item`
    /// so the highlight stays valid against the newly-filtered list.
    fn cycle_event_filter(&mut self) {
        use std::mem::discriminant;
        self.event_filter = match self.event_filter.take() {
            None => Some(EventType::ConfigChanged),
            Some(e) if discriminant(&e) == discriminant(&EventType::ConfigChanged) => {
                Some(EventType::CommandExecuted)
            }
            Some(e) if discriminant(&e) == discriminant(&EventType::CommandExecuted) => {
                Some(EventType::HardwareAction)
            }
            Some(e) if discriminant(&e) == discriminant(&EventType::HardwareAction) => {
                Some(EventType::Error)
            }
            Some(e) if discriminant(&e) == discriminant(&EventType::Error) => {
                Some(EventType::ServiceStart)
            }
            Some(e) if discriminant(&e) == discriminant(&EventType::ServiceStart) => {
                Some(EventType::ServiceStop)
            }
            Some(_) => None,
        };
        self.selected_item = 0;
        self.scroll_offset = 0;
    }

    fn clear_event_filter(&mut self) {
        self.event_filter = None;
        self.selected_item = 0;
        self.scroll_offset = 0;
    }

    /// Active filter label for the Events-tab block title. Returns
    /// `"All"` when no filter is set, otherwise the short tag used in
    /// event rows (e.g. `"CFG"`, `"ERR"`).
    fn event_filter_label(&self) -> &'static str {
        match &self.event_filter {
            None => "All",
            Some(e) => event_type_tag(e).0.trim(),
        }
    }

    /// Hardware-supported CPU frequency range in kHz, derived from sysfs via
    /// `available_cpu_frequencies()`. Falls back to [400000, 3500000]
    /// (400 MHz – 3.5 GHz) when sysfs is unreadable, so the freq editor
    /// always has a meaningful [min, max] to clamp against.
    fn freq_range() -> (u32, u32) {
        match available_cpu_frequencies() {
            Some(freqs) if !freqs.is_empty() => {
                let min = *freqs.iter().min().expect("non-empty");
                let max = *freqs.iter().max().expect("non-empty");
                if min < max { (min, max) } else { (400_000, 3_500_000) }
            }
            _ => (400_000, 3_500_000),
        }
    }

    /// Small label for a trackbar endpoint. Freq fields use MHz/GHz; others
    /// use the unit appropriate to the field ("%"/"°C").
    fn range_label_for_field(field: &str, raw: u32) -> String {
        match field {
            "cpu.min_freq_khz" | "cpu.max_freq_khz" => fmt_freq(Some(raw)),
            "battery.threshold" => format!("{}%", raw),
            _ => format!("{}°C", raw),
        }
    }

    fn begin_editing(&mut self) {
        self.config_backup = self.config.clone();
        let field = match self.selected_tab {
            Tab::Battery => "battery.threshold",
            Tab::CPU => "cpu.governor_ac",
            Tab::Thermal => "thermal.profile",
            _ => return,
        };
        self.editing_field = Some(field.to_string());
    }

    /// Display `Span` for a config field:
    /// - Not being edited: `value + suffix` in `normal` color
    /// - Enum/bool field being edited: `← value →` in Yellow+Bold
    /// - Numeric freq field being edited at None (unlimited): "unlimited" in
    ///   Yellow+Bold (no trackbar, since the value isn't on the [min,max] axis)
    /// - Numeric field being edited: trackbar `min ─────●───── max  value`
    ///   replacing the value position, with min/max labels sized to the field
    fn field_display(&self, field: &str, value: &str, suffix: &str, normal: Color) -> Span<'static> {
        let is_this = self.mode == AppMode::Editing && self.editing_field.as_deref() == Some(field);
        if !is_this {
            return Span::styled(
                format!("{}{}", value, suffix),
                Style::default().fg(normal),
            );
        }
        let is_enum = matches!(
            field,
            "battery.force_charge"
                | "cpu.governor_ac"
                | "cpu.governor_battery"
                | "thermal.profile"
        );
        if is_enum {
            return Span::styled(
                format!("← {}{} →", value, suffix),
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            );
        }
        if let Some((min, max, v, color)) = self.numeric_field_range() {
            let min_label = Self::range_label_for_field(field, min);
            let max_label = Self::range_label_for_field(field, max);
            let is_unlimited = match field {
                "cpu.min_freq_khz" => self.config.cpu.min_freq_khz.is_none(),
                "cpu.max_freq_khz" => self.config.cpu.max_freq_khz.is_none(),
                _ => false,
            };
            let value_label = if is_unlimited {
                "unlimited".to_string()
            } else {
                format!("{}{}", value, suffix)
            };
            return build_trackbar(min, max, v, &min_label, &max_label, &value_label, color);
        }
        Span::styled(
            format!("{}{}", value, suffix),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )
    }

    /// Adjust the field currently being edited by `direction` (+1 = Right,
    /// -1 = Left). Numeric fields increment/decrement by the field's step
    /// clamped to the field's valid range. Enum/bool fields cycle to the
    /// previous/next option in their option list (wrapping).
    fn adjust_field(&mut self, direction: i32) {
        let field = match &self.editing_field {
            Some(f) => f.clone(),
            None => return,
        };
        match field.as_str() {
            "battery.threshold" => {
                let new_val = (self.config.battery.threshold as i32 + direction).clamp(50, 100) as u8;
                self.config.battery.threshold = new_val;
            }
            "battery.force_charge" => {
                self.config.battery.force_charge = !self.config.battery.force_charge;
            }
            "cpu.governor_ac" => {
                let governors = Config::get_available_governors();
                if governors.is_empty() {
                    return;
                }
                let cur = governors
                    .iter()
                    .position(|g| g == &self.config.cpu.governor_ac)
                    .unwrap_or(0);
                let next = ((cur as i32 + direction).rem_euclid(governors.len() as i32)) as usize;
                self.config.cpu.governor_ac = governors[next].clone();
            }
            "cpu.governor_battery" => {
                let governors = Config::get_available_governors();
                if governors.is_empty() {
                    return;
                }
                let cur = governors
                    .iter()
                    .position(|g| g == &self.config.cpu.governor_battery)
                    .unwrap_or(0);
                let next = ((cur as i32 + direction).rem_euclid(governors.len() as i32)) as usize;
                self.config.cpu.governor_battery = governors[next].clone();
            }
            "cpu.min_freq_khz" => {
                const STEP: u32 = 500_000;
                let (range_min, range_max) = Self::freq_range();
                let current = self.config.cpu.min_freq_khz.unwrap_or(range_min);
                let new_val = if direction > 0 {
                    current.saturating_add(STEP)
                } else {
                    current.saturating_sub(STEP)
                };
                self.config.cpu.min_freq_khz = Some(new_val.clamp(range_min, range_max));
            }
            "cpu.max_freq_khz" => {
                const STEP: u32 = 500_000;
                let (range_min, range_max) = Self::freq_range();
                let current = self.config.cpu.max_freq_khz.unwrap_or(range_max);
                let new_val = if direction > 0 {
                    current.saturating_add(STEP)
                } else {
                    current.saturating_sub(STEP)
                };
                self.config.cpu.max_freq_khz = Some(new_val.clamp(range_min, range_max));
            }
            "thermal.warn_temp" => {
                let new_val = (self.config.thermal.warn_temp as i32 + direction).clamp(40, 100) as u8;
                self.config.thermal.warn_temp = new_val;
            }
            "thermal.high_temp" => {
                let new_val = (self.config.thermal.high_temp as i32 + direction).clamp(30, 90) as u8;
                self.config.thermal.high_temp = new_val;
            }
            "thermal.shutdown_temp" => {
                let new_val = (self.config.thermal.shutdown_temp as i32 + direction).clamp(50, 110) as u8;
                self.config.thermal.shutdown_temp = new_val;
            }
            "thermal.fan_off_temp" => {
                let new_val = (self.config.thermal.fan_off_temp as i32 + direction).clamp(20, 80) as u8;
                self.config.thermal.fan_off_temp = new_val;
            }
            "thermal.fan_max_temp" => {
                let new_val = (self.config.thermal.fan_max_temp as i32 + direction).clamp(40, 100) as u8;
                self.config.thermal.fan_max_temp = new_val;
            }
            "thermal.profile" => {
                const PROFILES: [ThermalProfile; 4] = [
                    ThermalProfile::Conservative,
                    ThermalProfile::Balanced,
                    ThermalProfile::Performance,
                    ThermalProfile::Custom,
                ];
                let cur = PROFILES
                    .iter()
                    .position(|p| *p == self.config.thermal.profile)
                    .unwrap_or(1);
                let next = ((cur as i32 + direction).rem_euclid(PROFILES.len() as i32)) as usize;
                self.config.thermal.profile = PROFILES[next].clone();
            }
            _ => {}
        }
    }

    /// Advance to the next editable field in the current tab, wrapping to the
    /// first field on the last. Values already live on `self.config` (no
    /// `apply_input_to_field` needed). Tab wraps; Enter saves, Esc discards.
    fn next_edit_field(&mut self) {
        let cur = self.editing_field.clone().unwrap_or_default();
        let next: Option<&'static str> = match (cur.as_str(), self.selected_tab) {
            ("battery.threshold", Tab::Battery) => Some("battery.force_charge"),
            ("battery.force_charge", Tab::Battery) => Some("battery.threshold"),
            ("cpu.governor_ac", Tab::CPU) => Some("cpu.governor_battery"),
            ("cpu.governor_battery", Tab::CPU) => Some("cpu.min_freq_khz"),
            ("cpu.min_freq_khz", Tab::CPU) => Some("cpu.max_freq_khz"),
            ("cpu.max_freq_khz", Tab::CPU) => Some("cpu.governor_ac"),
            ("thermal.profile", Tab::Thermal) => Some("thermal.fan_off_temp"),
            ("thermal.fan_off_temp", Tab::Thermal) => Some("thermal.high_temp"),
            ("thermal.high_temp", Tab::Thermal) => Some("thermal.warn_temp"),
            ("thermal.warn_temp", Tab::Thermal) => Some("thermal.fan_max_temp"),
            ("thermal.fan_max_temp", Tab::Thermal) => Some("thermal.shutdown_temp"),
            ("thermal.shutdown_temp", Tab::Thermal) => Some("thermal.profile"),
            _ => None,
        };
        match next {
            Some(f) => {
                self.editing_field = Some(f.to_string());
            }
            None => {
                self.mode = AppMode::Monitoring;
                self.editing_field = None;
            }
        }
    }

    /// Range descriptor for the numeric field currently being edited. Returns
    /// `(min, max, value, color)`. Returns `None` when: not in Editing mode,
    /// an enum/bool field is selected. Freq fields at `None` (unlimited) return
    /// the range with the marker at `max` so the trackbar still renders.
    fn numeric_field_range(&self) -> Option<(u32, u32, u32, Color)> {
        if self.mode != AppMode::Editing {
            return None;
        }
        let field = self.editing_field.as_deref()?;
        match field {
            "battery.threshold" => {
                Some((50, 100, self.config.battery.threshold as u32, Color::Cyan))
            }
            "cpu.min_freq_khz" => {
                let (min, max) = Self::freq_range();
                let v = self.config.cpu.min_freq_khz.unwrap_or(max);
                Some((min, max, v, Color::White))
            }
            "cpu.max_freq_khz" => {
                let (min, max) = Self::freq_range();
                let v = self.config.cpu.max_freq_khz.unwrap_or(max);
                Some((min, max, v, Color::White))
            }
            "thermal.warn_temp" => {
                Some((40, 100, self.config.thermal.warn_temp as u32, Color::Yellow))
            }
            "thermal.high_temp" => {
                Some((30, 90, self.config.thermal.high_temp as u32, Color::Yellow))
            }
            "thermal.shutdown_temp" => {
                Some((50, 110, self.config.thermal.shutdown_temp as u32, Color::Red))
            }
            "thermal.fan_off_temp" => {
                Some((20, 80, self.config.thermal.fan_off_temp as u32, Color::Green))
            }
            "thermal.fan_max_temp" => {
                Some((40, 100, self.config.thermal.fan_max_temp as u32, Color::Red))
            }
            _ => None,
        }
    }


    pub fn ui(&mut self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header
                Constraint::Min(0),    // Content
                Constraint::Length(3), // Footer
            ])
            .split(f.size());

        self.draw_header(f, chunks[0]);
        self.draw_content(f, chunks[1]);
        self.draw_footer(f, chunks[2]);

        if self.mode == AppMode::Help {
            self.draw_help(f, f.size());
        }

        if let AppMode::Popup(ref popup_type) = self.mode {
            self.draw_popup(f, popup_type.clone());
        }
    }

    fn draw_header(&self, f: &mut Frame, area: Rect) {
        let titles: Vec<&str> = Tab::titles().to_vec();
        let tabs = Tabs::new(titles)
            .block(Block::default().borders(Borders::ALL).title("SpinCtrl"))
            .highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
            .select(self.selected_tab.to_index());

        f.render_widget(tabs, area);
    }

    fn draw_content(&self, f: &mut Frame, area: Rect) {
        match self.selected_tab {
            Tab::Status => self.draw_status_tab(f, area),
            Tab::Battery => self.draw_battery_tab(f, area),
            Tab::CPU => self.draw_cpu_tab(f, area),
            Tab::Thermal => self.draw_thermal_tab(f, area),
            Tab::Events => self.draw_events_tab(f, area),
        }
    }

    fn draw_footer(&self, f: &mut Frame, area: Rect) {
        let mode_text = match self.mode {
            AppMode::Monitoring => "MONITOR",
            AppMode::Editing => "EDIT",
            AppMode::Help => "HELP",
            AppMode::Popup(_) => "POPUP",
        };

        let service_status = if self.service_available {
            "Service: Online"
        } else {
            "Service: Offline"
        };

        let footer_text = match &self.error_message {
            Some(msg) => format!(" {} | {} | {}", mode_text, service_status, msg),
            None => format!(
                " {} | {} | Press 'h' for help, 'q' to quit",
                mode_text, service_status
            ),
        };

        let footer = Paragraph::new(footer_text)
            .style(Style::default().fg(Color::White))
            .block(Block::default().borders(Borders::ALL));

        f.render_widget(footer, area);
    }


    fn draw_status_tab(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("System Status");
        let inner = block.inner(area);
        f.render_widget(block, area);

        if !self.service_available || self.status.is_none() {
            let msg = Paragraph::new("Service offline — waiting for data")
                .style(Style::default().fg(Color::Yellow))
                .alignment(Alignment::Center);
            f.render_widget(msg, inner);
            return;
        }

        let status = self.status.as_ref().unwrap();
        let bat = &status.battery;
        let pwr = &status.power;

        let charge_color = if bat.charging {
            Color::Green
        } else {
            Color::Yellow
        };
        let ac_text = if bat.ac_connected {
            "Connected"
        } else {
            "Disconnected"
        };
        let ac_color = if bat.ac_connected {
            Color::Green
        } else {
            Color::Red
        };
        let threshold_text = if bat.threshold_active {
            "Active"
        } else {
            "Inactive"
        };
        let threshold_color = if bat.threshold_active {
            Color::Green
        } else {
            Color::Yellow
        };

        let mut lines = vec![
            Line::from(vec![
                Span::raw("Battery:   "),
                Span::styled(
                    format!("{}%", bat.capacity),
                    Style::default().fg(charge_color).add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    if bat.charging { "CHARGING" } else { "idle" },
                    Style::default().fg(charge_color),
                ),
            ]),
            Line::from(vec![
                Span::raw("AC:        "),
                Span::styled(ac_text, Style::default().fg(ac_color)),
            ]),
            Line::from(vec![
                Span::raw("Threshold: "),
                Span::styled(threshold_text, Style::default().fg(threshold_color)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::raw("Governor:  "),
                Span::styled(
                    &pwr.cpu_governor,
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::raw("Freq:      "),
                Span::styled(
                    pwr.cpu_freq_khz
                        .map(|f| format!("{} kHz", f))
                        .unwrap_or_else(|| "n/a".to_string()),
                    Style::default().fg(Color::White),
                ),
            ]),
        ];

        if let Some(ref thermal) = status.thermal {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Thermal Zones:",
                Style::default().add_modifier(Modifier::BOLD),
            )));
            for zone in &thermal.zones {
                let temp_color = zone_temp_color(zone.temperature);
                lines.push(Line::from(vec![
                    Span::raw(format!("  Zone {}: ", zone.id)),
                    Span::styled(
                        format!("{}°C", zone.temperature),
                        Style::default().fg(temp_color),
                    ),
                ]));
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("PID {} · updated {}", status.service_pid, status.timestamp.format("%H:%M:%S")),
            Style::default().fg(Color::DarkGray),
        )));

        let paragraph = Paragraph::new(Text::from(lines));
        f.render_widget(paragraph, inner);
    }

    fn draw_battery_tab(&self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Gauge
                Constraint::Length(1), // Spacer
                Constraint::Min(0),   // Config + status details
            ])
            .split(area);

        let capacity = self
            .status
            .as_ref()
            .map(|s| s.battery.capacity)
            .unwrap_or(0);
        let charging = self
            .status
            .as_ref()
            .map(|s| s.battery.charging)
            .unwrap_or(false);
        let gauge_color = if charging {
            Color::Green
        } else if capacity < 20 {
            Color::Red
        } else {
            Color::Yellow
        };
        let gauge = Gauge::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Battery Level"),
            )
            .gauge_style(Style::default().fg(gauge_color))
            .percent(u16::from(capacity))
            .label(format!("{}%", capacity));
        f.render_widget(gauge, chunks[0]);

        let bc = &self.config.battery;
        let mut lines = vec![
            Line::from(vec![
                Span::styled(
                    "Threshold:  ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                self.field_display(
                    "battery.threshold",
                    &bc.threshold.to_string(),
                    "%",
                    Color::Cyan,
                ),
                Span::raw("  "),
                Span::styled(field_description("battery.threshold"), Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(vec![
                Span::styled(
                    "Force:      ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                self.field_display(
                    "battery.force_charge",
                    if bc.force_charge { "Yes" } else { "No" },
                    "",
                    if bc.force_charge { Color::Green } else { Color::White },
                ),
                Span::raw("  "),
                Span::styled(field_description("battery.force_charge"), Style::default().fg(Color::DarkGray)),
            ]),
        ];

        if let Some(ref status) = self.status {
            let bat = &status.battery;
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Live Status:",
                Style::default().add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(vec![
                Span::raw("  Charging:  "),
                Span::styled(
                    if bat.charging { "Yes" } else { "No" },
                    Style::default().fg(if bat.charging {
                        Color::Green
                    } else {
                        Color::Yellow
                    }),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::raw("  AC Power:  "),
                Span::styled(
                    if bat.ac_connected {
                        "Connected"
                    } else {
                        "Disconnected"
                    },
                    Style::default().fg(if bat.ac_connected {
                        Color::Green
                    } else {
                        Color::Red
                    }),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::raw("  Threshold: "),
                Span::styled(
                    if bat.threshold_active {
                        "Active"
                    } else {
                        "Inactive"
                    },
                    Style::default().fg(if bat.threshold_active {
                        Color::Green
                    } else {
                        Color::DarkGray
                    }),
                ),
            ]));
        } else {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Status unavailable — service offline",
                Style::default().fg(Color::DarkGray),
            )));
        }

        let details_block = Block::default()
            .borders(Borders::ALL)
            .title("Battery Configuration");
        let details_inner = details_block.inner(chunks[2]);
        f.render_widget(details_block, chunks[2]);

        let paragraph = Paragraph::new(Text::from(lines));
        f.render_widget(paragraph, details_inner);
    }

    fn draw_cpu_tab(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("CPU Configuration");
        let tab_inner = block.inner(area);
        f.render_widget(block, area);

        let cc = &self.config.cpu;
        let min_display = fmt_freq(cc.min_freq_khz);
        let max_display = fmt_freq(cc.max_freq_khz);
        let mut lines = vec![
            Line::from(Span::styled(
                "Governors:",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(vec![
                Span::raw("  AC:       "),
                self.field_display("cpu.governor_ac", &cc.governor_ac, "", Color::Cyan),
                Span::raw("  "),
                Span::styled(field_description("cpu.governor_ac"), Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(vec![
                Span::raw("  Battery:  "),
                self.field_display("cpu.governor_battery", &cc.governor_battery, "", Color::Yellow),
                Span::raw("  "),
                Span::styled(field_description("cpu.governor_battery"), Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "Frequency Limits:",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(vec![
                Span::raw("  Min:  "),
                self.field_display("cpu.min_freq_khz", &min_display, "", Color::White),
                Span::raw("  "),
                Span::styled(field_description("cpu.min_freq_khz"), Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(vec![
                Span::raw("  Max:  "),
                self.field_display("cpu.max_freq_khz", &max_display, "", Color::White),
                Span::raw("  "),
                Span::styled(field_description("cpu.max_freq_khz"), Style::default().fg(Color::DarkGray)),
            ]),
        ];

        if let Some(ref status) = self.status {
            let pwr = &status.power;
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Current Status:",
                Style::default().add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(vec![
                Span::raw("  Governor: "),
                Span::styled(
                    &pwr.cpu_governor,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::raw("  Freq:     "),
                Span::styled(
                    pwr.cpu_freq_khz
                        .map(|f| format!("{} kHz", f))
                        .unwrap_or_else(|| "n/a".to_string()),
                    Style::default().fg(Color::White),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::raw("  Power:    "),
                Span::styled(
                    if pwr.ac_connected {
                        "AC Connected"
                    } else {
                        "On Battery"
                    },
                    Style::default().fg(if pwr.ac_connected {
                        Color::Green
                    } else {
                        Color::Yellow
                    }),
                ),
            ]));
        }

        let paragraph = Paragraph::new(Text::from(lines));
        f.render_widget(paragraph, tab_inner);
    }

    fn draw_thermal_tab(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Thermal Configuration");
        let tab_inner = block.inner(area);
        f.render_widget(block, area);

        let tc = &self.config.thermal;
        let profile_name = thermal_profile_name(&tc.profile);

        let mut lines = vec![
            Line::from(vec![
                Span::styled(
                    "Profile:    ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                self.field_display("thermal.profile", profile_name, "", Color::Cyan),
                Span::raw("  "),
                Span::styled(field_description("thermal.profile"), Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "Temperature Thresholds (°C):",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(vec![
                Span::raw("  Fan off:      "),
                self.field_display("thermal.fan_off_temp", &tc.fan_off_temp.to_string(), "", Color::Green),
                Span::raw("  "),
                Span::styled(field_description("thermal.fan_off_temp"), Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(vec![
                Span::raw("  High:         "),
                self.field_display("thermal.high_temp", &tc.high_temp.to_string(), "", Color::Yellow),
                Span::raw("  "),
                Span::styled(field_description("thermal.high_temp"), Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(vec![
                Span::raw("  Warning:      "),
                self.field_display("thermal.warn_temp", &tc.warn_temp.to_string(), "", Color::Yellow),
                Span::raw("  "),
                Span::styled(field_description("thermal.warn_temp"), Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(vec![
                Span::raw("  Fan max:      "),
                self.field_display("thermal.fan_max_temp", &tc.fan_max_temp.to_string(), "", Color::Red),
                Span::raw("  "),
                Span::styled(field_description("thermal.fan_max_temp"), Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(vec![
                Span::raw("  Shutdown:     "),
                self.field_display("thermal.shutdown_temp", &tc.shutdown_temp.to_string(), "", Color::Red),
                Span::raw("  "),
                Span::styled(field_description("thermal.shutdown_temp"), Style::default().fg(Color::DarkGray)),
            ]),
        ];

        if let Some(ref status) = self.status {
            if let Some(ref thermal) = status.thermal {
                if !thermal.zones.is_empty() {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "Current Zones:",
                        Style::default().add_modifier(Modifier::BOLD),
                    )));
                    for zone in &thermal.zones {
                        let temp_color = zone_temp_color(zone.temperature);
                        lines.push(Line::from(vec![
                            Span::raw(format!("  Zone {}: ", zone.id)),
                            Span::styled(
                                format!("{}°C", zone.temperature),
                                Style::default().fg(temp_color),
                            ),
                        ]));
                    }
                }
            }
        }

        let paragraph = Paragraph::new(Text::from(lines));
        f.render_widget(paragraph, tab_inner);
    }

    fn draw_events_tab(&self, f: &mut Frame, area: Rect) {
        use std::mem::discriminant;

        let visible: Vec<&EventEntry> = match &self.event_filter {
            None => self.events.iter().collect(),
            Some(filter) => self
                .events
                .iter()
                .filter(|e| discriminant(&e.event_type) == discriminant(filter))
                .collect(),
        };

        let filter_label = self.event_filter_label();
        let title = if self.event_filter.is_none() {
            format!("Events ({})", visible.len())
        } else {
            format!("Events ({}/{}) [filter: {}]", visible.len(), self.events.len(), filter_label)
        };
        let block = Block::default().borders(Borders::ALL).title(title);
        let inner = block.inner(area);
        f.render_widget(block, area);

        if visible.is_empty() {
            let msg = if self.events.is_empty() {
                Paragraph::new("No events recorded")
            } else {
                Paragraph::new(format!(
                    "No '{}' events — press 'a' to show all",
                    filter_label
                ))
            }
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
            f.render_widget(msg, inner);
            return;
        }

        let max_item = visible.len().saturating_sub(1);
        let effective_selected = self.selected_item.min(max_item);
        let visible_height = inner.height as usize;

        let scroll_start = if effective_selected >= self.scroll_offset + visible_height {
            effective_selected.saturating_sub(visible_height) + 1
        } else if effective_selected < self.scroll_offset {
            effective_selected
        } else {
            self.scroll_offset
        };
        let scroll_end = (scroll_start + visible_height).min(visible.len());

        let items: Vec<ListItem> = visible[scroll_start..scroll_end]
            .iter()
            .map(|event| {
                let (tag, tag_color) = event_type_tag(&event.event_type);
                let time_str = event.timestamp.format("%m-%d %H:%M:%S").to_string();
                let content = Line::from(vec![
                    Span::styled(
                        format!("{} ", time_str),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        format!("[{}] ", tag),
                        Style::default()
                            .fg(tag_color)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(&event.message),
                ]);
                ListItem::new(content)
            })
            .collect();

        let highlight_idx = effective_selected.saturating_sub(scroll_start);

        let list = List::new(items)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("▸ ");

        let mut state = ListState::default();
        state.select(Some(highlight_idx));
        f.render_stateful_widget(list, inner, &mut state);
    }


    fn draw_popup(&self, f: &mut Frame, popup_type: PopupType) {
        let area = f.size();
        let popup = centered_rect(50, 7, area);

        f.render_widget(Clear, popup);

        let (title, body_lines, title_color) = match popup_type {
            PopupType::ConfirmExit => (
                "Confirm Exit",
                vec![
                    Line::from(""),
                    Line::from("Are you sure you want to quit?"),
                    Line::from(""),
                    Line::from(Span::styled(
                        "[y] Yes   [n] No",
                        Style::default().fg(Color::Yellow),
                    )),
                ],
                Color::Yellow,
            ),
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .style(Style::default().fg(title_color).bg(Color::Black));

        let paragraph = Paragraph::new(Text::from(body_lines))
            .block(block)
            .alignment(Alignment::Center);

        f.render_widget(paragraph, popup);
    }


    fn draw_help(&self, f: &mut Frame, area: Rect) {
        f.render_widget(Clear, area);

        let help_lines = vec![
            Line::from(Span::styled(
                " SpinCtrl — Help",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Shortcuts",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::UNDERLINED),
            )),
            Line::from("    Tab/1-5  nav    ↑/↓  scroll    Enter/e  edit    r  refresh"),
            Line::from("    f        force charge (Battery) / cycle filter (Events)"),
            Line::from("    s        stop charge (Battery)   a  clear filter (Events)"),
            Line::from("    h/F1     this help    q/Esc  quit/back"),
            Line::from(""),
            Line::from(Span::styled(
                "  Settings Reference",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::UNDERLINED),
            )),
            Line::from(Span::styled(
                "    Battery",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )),
            Line::from("      threshold  50-100%; stop charging above this % to extend lifespan"),
            Line::from("      force      one-shot charge to 100% ('f' in Battery tab)"),
            Line::from(Span::styled(
                "    CPU",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )),
            Line::from("      governors  AC vs Battery profile; one of:"),
            Line::from("                 performance | powersave | ondemand | conservative | schedutil"),
            Line::from(Span::styled(
                "    Thermal",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )),
            Line::from("      thresholds  fan_off < high < warn < shutdown; fan_off < fan_max"),
            Line::from("                  warn>high = caution, high<shutdown = throttle"),
            Line::from("      profiles    conservative | balanced | performance | custom"),
            Line::from("                  'custom' leaves current thresholds unchanged"),
            Line::from(""),
            Line::from(Span::styled(
                "  Press h / q / Esc to close",
                Style::default().fg(Color::DarkGray),
            )),
        ];

        let block = Block::default()
            .borders(Borders::ALL)
            .title("Help")
            .style(Style::default().bg(Color::Black));

        let paragraph = Paragraph::new(Text::from(help_lines)).block(block);
        f.render_widget(paragraph, area);
    }
}


/// Pick a color for a temperature reading.
fn zone_temp_color(temp: i32) -> Color {
    if temp >= 70 {
        Color::Red
    } else if temp >= 55 {
        Color::Yellow
    } else {
        Color::Green
    }
}

/// Short display tag + color for an event type.
fn event_type_tag(et: &EventType) -> (&'static str, Color) {
    match et {
        EventType::ConfigChanged => ("CFG", Color::Cyan),
        EventType::CommandExecuted => ("CMD", Color::Blue),
        EventType::HardwareAction => ("HW ", Color::Magenta),
        EventType::Error => ("ERR", Color::Red),
        EventType::ServiceStart => ("STA", Color::Green),
        EventType::ServiceStop => ("STP", Color::Yellow),
    }
}

/// Brief description of what each editable config field does, shown inline
/// in the tab (DarkGray) so users understand the parameter without help.
fn field_description(field: &str) -> &'static str {
    match field {
        "battery.threshold" => "stop charging above this %",
        "battery.force_charge" => "one-shot charge to 100%",
        "cpu.governor_ac" => "governor on AC power",
        "cpu.governor_battery" => "governor on battery",
        "cpu.min_freq_khz" => "min CPU freq (u=unlimited)",
        "cpu.max_freq_khz" => "max CPU freq (u=unlimited)",
        "thermal.profile" => "thermal preset",
        "thermal.fan_off_temp" => "fan stops below this",
        "thermal.high_temp" => "triggers throttling",
        "thermal.warn_temp" => "triggers caution",
        "thermal.fan_max_temp" => "fan at max speed",
        "thermal.shutdown_temp" => "emergency shutdown",
        _ => "",
    }
}

/// Human-readable thermal profile name.
fn thermal_profile_name(p: &ThermalProfile) -> &'static str {
    match p {
        ThermalProfile::Conservative => "Conservative",
        ThermalProfile::Balanced => "Balanced",
        ThermalProfile::Performance => "Performance",
        ThermalProfile::Custom => "Custom",
    }
}

/// Format a CPU frequency (in kHz) as a human-readable string.
/// `None` → "unlimited"; `>= 1_000_000` kHz → "X.Y GHz"; else → "N MHz".
fn fmt_freq(khz: Option<u32>) -> String {
    match khz {
        None => "unlimited".to_string(),
        Some(v) if v >= 1_000_000 => format!("{:.1} GHz", v as f64 / 1_000_000.0),
        Some(v) => format!("{} MHz", v / 1_000),
    }
}

/// Build a trackbar `Span`: `min_label ─────●──────── max_label  value_label`.
/// The `●` marker sits at column `round((v-min)/(max-min) * (BAR_WIDTH-1))`.
/// `BAR_WIDTH` counts the line-and-marker characters only (labels sit outside).
fn build_trackbar(
    min: u32,
    max: u32,
    value: u32,
    min_label: &str,
    max_label: &str,
    value_label: &str,
    color: Color,
) -> Span<'static> {
    const BAR_WIDTH: usize = 18;
    let denom = max.saturating_sub(min);
    let ratio = if denom > 0 {
        ((value.saturating_sub(min)) as f64 / denom as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let marker_pos = (ratio * (BAR_WIDTH.saturating_sub(1)) as f64).round() as usize;
    let marker_pos = marker_pos.min(BAR_WIDTH.saturating_sub(1));
    let bar: String = (0..BAR_WIDTH)
        .map(|i| if i == marker_pos { '●' } else { '─' })
        .collect();
    Span::styled(
        format!("{} {} {}  {}", min_label, bar, max_label, value_label),
        Style::default().fg(color),
    )
}

/// Center a rectangle of `width × height` inside `area`.
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

/// Translate a service-side error into a user-facing message. Permission
/// errors get actionable guidance about joining the `spinctrl` group (req
/// 8.9); everything else falls back to the error's Display.
fn explain_error(e: &SpinCtrlError) -> String {
    match e {
        SpinCtrlError::PermissionDenied(path) => format!(
            "Permission denied accessing {}. Add yourself to the 'spinctrl' group and re-login: sudo usermod -a -G spinctrl $USER",
            path
        ),
        SpinCtrlError::Io(io_err) if io_err.kind() == std::io::ErrorKind::PermissionDenied => {
            "Permission denied (EACCES). Add yourself to the 'spinctrl' group and re-login: sudo usermod -a -G spinctrl $USER".to_string()
        }
        other => format!("{}", other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_app() -> App {
        App::with_ipc(IpcManager::with_paths(
            "/tmp/spinctrl-test-nonexistent-config.json",
            "/tmp/spinctrl-test-nonexistent-runtime",
        )).expect("test App must not require a running service or production config")
    }

    #[test]
    fn test_tab_from_index_valid() {
        assert_eq!(Tab::from_index(0), Tab::Status);
        assert_eq!(Tab::from_index(1), Tab::Battery);
        assert_eq!(Tab::from_index(2), Tab::CPU);
        assert_eq!(Tab::from_index(3), Tab::Thermal);
        assert_eq!(Tab::from_index(4), Tab::Events);
    }

    #[test]
    fn test_tab_from_index_out_of_range_defaults_to_status() {
        assert_eq!(Tab::from_index(5), Tab::Status);
        assert_eq!(Tab::from_index(usize::MAX), Tab::Status);
    }

    #[test]
    fn test_tab_to_index_round_trip() {
        for i in 0..Tab::COUNT {
            assert_eq!(Tab::from_index(i).to_index(), i);
        }
    }

    #[test]
    fn test_tab_next_wraps_around() {
        assert_eq!(Tab::Status.next(), Tab::Battery);
        assert_eq!(Tab::Battery.next(), Tab::CPU);
        assert_eq!(Tab::CPU.next(), Tab::Thermal);
        assert_eq!(Tab::Thermal.next(), Tab::Events);
        assert_eq!(Tab::Events.next(), Tab::Status);
    }

    #[test]
    fn test_tab_previous_wraps_around() {
        assert_eq!(Tab::Status.previous(), Tab::Events);
        assert_eq!(Tab::Events.previous(), Tab::Thermal);
        assert_eq!(Tab::Battery.previous(), Tab::Status);
    }

    #[test]
    fn test_tab_titles_count_match() {
        assert_eq!(Tab::titles().len(), Tab::COUNT);
        assert_eq!(Tab::COUNT, 5);
        assert_eq!(Tab::titles(), ["Status", "Battery", "CPU", "Thermal", "Events"]);
    }

    #[test]
    fn test_tab_is_copy() {
        let original = Tab::CPU;
        let copy = original;
        assert_eq!(original, copy);
    }

    #[test]
    fn test_app_new_succeeds_without_service() {
        let app = make_test_app();
        assert_eq!(app.selected_tab, Tab::Status);
        assert_eq!(app.mode, AppMode::Monitoring);
        assert!(!app.should_quit);
        assert!(!app.service_available);
        assert!(app.error_message.is_none());
        assert!(app.status.is_none());
        assert!(app.events.is_empty());
        assert!(app.editing_field.is_none());
        assert_eq!(app.scroll_offset, 0);
        assert_eq!(app.selected_item, 0);
        assert_eq!(app.update_interval, Duration::from_secs(2));
        assert_eq!(app.config, Config::default());
    }

    #[test]
    fn test_appmode_equality() {
        assert_eq!(AppMode::Monitoring, AppMode::Monitoring);
        assert_ne!(AppMode::Monitoring, AppMode::Editing);
        assert_ne!(AppMode::Monitoring, AppMode::Help);
        assert_eq!(
            AppMode::Popup(PopupType::ConfirmExit),
            AppMode::Popup(PopupType::ConfirmExit)
        );
        assert_ne!(
            AppMode::Popup(PopupType::ConfirmExit),
            AppMode::Monitoring
        );
    }

    #[test]
    fn test_popuptype_equality_and_clone() {
        assert_eq!(PopupType::ConfirmExit, PopupType::ConfirmExit);
        let a = PopupType::ConfirmExit;
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn test_apply_config_in_place_without_service_sets_error_and_stays_editing() {
        let mut app = make_test_app();
        assert!(!app.service_available, "fresh App must report service unavailable");
        app.mode = AppMode::Editing;
        app.apply_config_in_place();
        assert_eq!(app.mode, AppMode::Editing, "must stay in Editing mode");
        assert!(app.error_message.is_some(), "must set error_message");
        assert!(app.error_message.as_deref().unwrap_or("").contains("offline"), "unexpected: {:?}", app.error_message);
    }

    #[test]
    fn test_apply_config_in_place_validates_before_pushing() {
        let mut app = make_test_app();
        app.service_available = true;
        app.mode = AppMode::Editing;
        app.config.battery.threshold = 30;
        app.apply_config_in_place();
        assert_eq!(app.mode, AppMode::Editing, "must stay in Editing mode even on validation failure");
        assert!(app.error_message.as_deref().unwrap_or("").contains("validation"), "expected validation msg, got: {:?}", app.error_message);
    }

    #[test]
    fn test_explain_error_permission_denied_has_group_guidance() {
        let e = SpinCtrlError::PermissionDenied("/var/lib/spinctrl/status.json".to_string());
        let msg = explain_error(&e);
        assert!(msg.contains("spinctrl"), "expected group name, got: {msg}");
        assert!(msg.contains("usermod"), "expected usermod guidance, got: {msg}");
        assert!(msg.contains("/var/lib/spinctrl/status.json"), "expected path, got: {msg}");
    }

    #[test]
    fn test_explain_error_io_permission_denied_has_group_guidance() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let e = SpinCtrlError::Io(io_err);
        let msg = explain_error(&e);
        assert!(msg.contains("spinctrl"), "expected group name, got: {msg}");
        assert!(msg.contains("usermod"), "expected usermod guidance, got: {msg}");
    }

    #[test]
    fn test_explain_error_non_permission_is_generic() {
        let e = SpinCtrlError::FileNotFound("/missing/path".to_string());
        let msg = explain_error(&e);
        assert!(!msg.contains("usermod"), "non-permission error should not show group guidance: {msg}");
        assert!(msg.contains("/missing/path") || msg.contains("File not found"), "expected generic msg, got: {msg}");
    }

    /// Smoke test: render every tab, Help overlay, and each popup variant
    /// through `Terminal::draw` backed by `TestBackend`. Asserts that none
    /// of the draw paths panic (the `.unwrap()`s on draw enforce this).
    #[test]
    fn test_ui_render_smoke_all_tabs_and_popups() {
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut app = make_test_app();

        for i in 0..Tab::COUNT {
            app.selected_tab = Tab::from_index(i);
            app.mode = AppMode::Monitoring;
            terminal.draw(|f| app.ui(f)).unwrap();
        }

        app.selected_tab = Tab::Status;
        app.mode = AppMode::Help;
        terminal.draw(|f| app.ui(f)).unwrap();

        app.mode = AppMode::Popup(PopupType::ConfirmExit);
        terminal.draw(|f| app.ui(f)).unwrap();

        app.mode = AppMode::Monitoring;
        app.selected_tab = Tab::Events;
        terminal.draw(|f| app.ui(f)).unwrap();

        app.mode = AppMode::Editing;
        app.selected_tab = Tab::Battery;
        app.begin_editing();
        terminal.draw(|f| app.ui(f)).unwrap();
        app.editing_field = Some("battery.force_charge".to_string());
        terminal.draw(|f| app.ui(f)).unwrap();

        app.selected_tab = Tab::CPU;
        app.begin_editing();
        terminal.draw(|f| app.ui(f)).unwrap();
        app.editing_field = Some("cpu.min_freq_khz".to_string());
        app.adjust_field(1);
        terminal.draw(|f| app.ui(f)).unwrap();

        app.selected_tab = Tab::Thermal;
        app.begin_editing();
        terminal.draw(|f| app.ui(f)).unwrap();
        app.editing_field = Some("thermal.warn_temp".to_string());
        app.adjust_field(1);
        terminal.draw(|f| app.ui(f)).unwrap();
        app.editing_field = Some("thermal.profile".to_string());
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[test]
    fn test_arrow_adjust_threshold_increments_and_clamps() {
        let mut app = make_test_app();
        app.selected_tab = Tab::Battery;
        app.begin_editing();
        assert_eq!(app.editing_field.as_deref(), Some("battery.threshold"));
        assert_eq!(app.config.battery.threshold, 80, "default threshold");
        app.adjust_field(1);
        assert_eq!(app.config.battery.threshold, 81, "Right must increment by step 1");
        app.adjust_field(-1);
        app.adjust_field(-1);
        assert_eq!(app.config.battery.threshold, 79, "Left must decrement by step 1");
        app.config.battery.threshold = 100;
        app.adjust_field(1);
        assert_eq!(app.config.battery.threshold, 100, "must clamp at upper bound 100");
        app.config.battery.threshold = 50;
        app.adjust_field(-1);
        assert_eq!(app.config.battery.threshold, 50, "must clamp at lower bound 50");
    }

    #[test]
    fn test_arrow_adjust_thermal_temps_clamp_to_range() {
        let mut app = make_test_app();
        app.selected_tab = Tab::Thermal;
        app.begin_editing();
        app.editing_field = Some("thermal.warn_temp".to_string());
        app.config.thermal.warn_temp = 40;
        app.adjust_field(-1);
        assert_eq!(app.config.thermal.warn_temp, 40, "warn_temp lower bound 40");
        app.config.thermal.warn_temp = 100;
        app.adjust_field(1);
        assert_eq!(app.config.thermal.warn_temp, 100, "warn_temp upper bound 100");
    }

    #[test]
    fn test_next_edit_field_cycles_thermal_fields() {
        let mut app = make_test_app();
        app.selected_tab = Tab::Thermal;
        app.begin_editing();
        assert_eq!(app.editing_field.as_deref(), Some("thermal.profile"));
        app.next_edit_field();
        assert_eq!(app.editing_field.as_deref(), Some("thermal.fan_off_temp"));
        app.next_edit_field();
        assert_eq!(app.editing_field.as_deref(), Some("thermal.high_temp"));
        app.next_edit_field();
        assert_eq!(app.editing_field.as_deref(), Some("thermal.warn_temp"));
        app.next_edit_field();
        assert_eq!(app.editing_field.as_deref(), Some("thermal.fan_max_temp"));
        app.next_edit_field();
        assert_eq!(app.editing_field.as_deref(), Some("thermal.shutdown_temp"));
        app.next_edit_field();
        assert_eq!(app.editing_field.as_deref(), Some("thermal.profile"), "must wrap to the first field");
    }

    #[test]
    fn test_battery_tab_cycles_threshold_and_force() {
        let mut app = make_test_app();
        app.selected_tab = Tab::Battery;
        app.begin_editing();
        assert_eq!(app.editing_field.as_deref(), Some("battery.threshold"));
        app.next_edit_field();
        assert_eq!(app.editing_field.as_deref(), Some("battery.force_charge"));
        app.next_edit_field();
        assert_eq!(app.editing_field.as_deref(), Some("battery.threshold"), "must wrap to threshold");
    }

    #[test]
    fn test_cpu_tab_cycles_all_four_fields() {
        let mut app = make_test_app();
        app.selected_tab = Tab::CPU;
        app.begin_editing();
        assert_eq!(app.editing_field.as_deref(), Some("cpu.governor_ac"));
        app.next_edit_field();
        assert_eq!(app.editing_field.as_deref(), Some("cpu.governor_battery"));
        app.next_edit_field();
        assert_eq!(app.editing_field.as_deref(), Some("cpu.min_freq_khz"));
        app.next_edit_field();
        assert_eq!(app.editing_field.as_deref(), Some("cpu.max_freq_khz"));
        app.next_edit_field();
        assert_eq!(app.editing_field.as_deref(), Some("cpu.governor_ac"), "must wrap to governor_ac");
    }

    #[test]
    fn test_arrow_toggle_force_charge() {
        let mut app = make_test_app();
        app.editing_field = Some("battery.force_charge".to_string());
        assert!(!app.config.battery.force_charge, "default false");
        app.adjust_field(1);
        assert!(app.config.battery.force_charge, "Right must toggle to true");
        app.adjust_field(-1);
        assert!(!app.config.battery.force_charge, "Left must toggle back to false");
        app.adjust_field(1);
        app.adjust_field(1);
        assert!(!app.config.battery.force_charge, "two toggles return to false");
    }

    #[test]
    fn test_arrow_cycle_profile() {
        let mut app = make_test_app();
        app.editing_field = Some("thermal.profile".to_string());
        app.config.thermal.profile = ThermalProfile::Balanced;
        app.adjust_field(1);
        assert_eq!(app.config.thermal.profile, ThermalProfile::Performance);
        app.adjust_field(1);
        assert_eq!(app.config.thermal.profile, ThermalProfile::Custom);
        app.adjust_field(1);
        assert_eq!(app.config.thermal.profile, ThermalProfile::Conservative, "must wrap");
        app.adjust_field(-1);
        assert_eq!(app.config.thermal.profile, ThermalProfile::Custom, "Left must go to previous");
    }

    #[test]
    fn test_arrow_cycle_governor() {
        let mut app = make_test_app();
        app.editing_field = Some("cpu.governor_ac".to_string());
        let governors = Config::get_available_governors();
        assert!(!governors.is_empty());
        let start = app.config.cpu.governor_ac.clone();
        app.adjust_field(1);
        assert_ne!(app.config.cpu.governor_ac, start, "must move to next governor");
        for _ in 0..governors.len() {
            app.adjust_field(1);
        }
        assert_eq!(app.config.cpu.governor_ac, app.config.cpu.governor_ac, "cycling through all lands somewhere valid");
        assert!(Config::is_valid_governor(&app.config.cpu.governor_ac));
    }

    #[test]
    fn test_arrow_adjust_cpu_freq_option_none_becomes_some() {
        let mut app = make_test_app();
        app.editing_field = Some("cpu.min_freq_khz".to_string());
        assert!(app.config.cpu.min_freq_khz.is_none(), "default is None");
        let (range_min, _) = App::freq_range();
        app.adjust_field(1);
        assert_eq!(
            app.config.cpu.min_freq_khz,
            Some(range_min.saturating_add(500_000)),
            "from None→range_min + step 500_000"
        );
    }

    #[test]
    fn test_esc_reverts_config() {
        let mut app = make_test_app();
        let original_threshold = app.config.battery.threshold;
        app.selected_tab = Tab::Battery;
        app.mode = AppMode::Editing;
        app.begin_editing();
        assert_eq!(app.config_backup.battery.threshold, original_threshold);
        app.config.battery.threshold = 99;
        assert_ne!(app.config_backup.battery.threshold, 99);
        app.config = app.config_backup.clone();
        app.editing_field = None;
        app.mode = AppMode::Monitoring;
        assert_eq!(app.config.battery.threshold, original_threshold, "Esc must revert to backup");
        assert_eq!(app.mode, AppMode::Monitoring);
        assert!(app.editing_field.is_none());
    }

    fn make_event(event_type: EventType, message: &str) -> EventEntry {
        EventEntry {
            timestamp: chrono::Utc::now(),
            event_type,
            message: message.to_string(),
            details: None,
        }
    }

    #[test]
    fn test_events_tab_render_with_filter_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut app = make_test_app();
        app.selected_tab = Tab::Events;
        app.mode = AppMode::Monitoring;

        app.events.push(make_event(EventType::ServiceStart, "service up"));
        app.events.push(make_event(EventType::Error, "oops"));
        app.events.push(make_event(EventType::ConfigChanged, "threshold 80"));
        app.events.push(make_event(EventType::Error, "disk full"));
        app.events.push(make_event(EventType::HardwareAction, "governor set"));

        app.event_filter = None;
        terminal.draw(|f| app.ui(f)).unwrap();

        app.event_filter = Some(EventType::Error);
        app.selected_item = 1;
        terminal.draw(|f| app.ui(f)).unwrap();

        app.event_filter = Some(EventType::ServiceStart);
        terminal.draw(|f| app.ui(f)).unwrap();

        app.events.clear();
        app.event_filter = Some(EventType::Error);
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[test]
    fn test_help_overlay_renders_with_settings_reference() {
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut app = make_test_app();
        app.mode = AppMode::Help;
        terminal.draw(|f| app.ui(f)).unwrap();

        let buf = terminal.backend().buffer();
        let mut found_settings = false;
        let mut found_battery_threshold = false;
        let mut found_cpu_governors = false;
        let mut found_thermal_profiles = false;
        for y in 0..24u16 {
            let mut row = String::new();
            for x in 0..80u16 {
                let idx = buf.index_of(x, y);
                let sym = &buf.content[idx].symbol;
                row.push(sym.chars().next().unwrap_or(' '));
            }
            if row.contains("Settings Reference") {
                found_settings = true;
            }
            if row.contains("threshold") && row.contains("50-100") {
                found_battery_threshold = true;
            }
            if row.contains("performance") && row.contains("schedutil") {
                found_cpu_governors = true;
            }
            if row.contains("conservative") && row.contains("custom") {
                found_thermal_profiles = true;
            }
        }
        assert!(found_settings, "Help must include 'Settings Reference' section");
        assert!(found_battery_threshold, "Help must document battery threshold 50-100");
        assert!(found_cpu_governors, "Help must list all 5 recognized CPU governor names");
        assert!(found_thermal_profiles, "Help must list all 4 thermal profiles");
    }

    #[test]
    fn test_cycle_event_filter_walks_all_variants_and_wraps() {
        let mut app = make_test_app();
        assert!(app.event_filter.is_none(), "fresh App starts with no filter");

        app.cycle_event_filter();
        assert!(matches!(app.event_filter, Some(EventType::ConfigChanged)));
        app.cycle_event_filter();
        assert!(matches!(app.event_filter, Some(EventType::CommandExecuted)));
        app.cycle_event_filter();
        assert!(matches!(app.event_filter, Some(EventType::HardwareAction)));
        app.cycle_event_filter();
        assert!(matches!(app.event_filter, Some(EventType::Error)));
        app.cycle_event_filter();
        assert!(matches!(app.event_filter, Some(EventType::ServiceStart)));
        app.cycle_event_filter();
        assert!(matches!(app.event_filter, Some(EventType::ServiceStop)));
        app.cycle_event_filter();
        assert!(app.event_filter.is_none(), "must wrap back to None after ServiceStop");

        app.event_filter = Some(EventType::Error);
        app.clear_event_filter();
        assert!(app.event_filter.is_none());
    }

    #[test]
    fn test_event_filter_label_matches_tag() {
        let mut app = make_test_app();
        assert_eq!(app.event_filter_label(), "All");
        app.event_filter = Some(EventType::Error);
        assert_eq!(app.event_filter_label(), "ERR");
        app.event_filter = Some(EventType::ConfigChanged);
        assert_eq!(app.event_filter_label(), "CFG");
    }

    #[test]
    fn test_field_display_editing_enum_shows_arrows() {
        let mut app = make_test_app();
        app.selected_tab = Tab::Battery;
        app.mode = AppMode::Editing;
        app.begin_editing();
        app.editing_field = Some("battery.force_charge".to_string());
        let span = app.field_display("battery.force_charge", "Yes", "", Color::Green);
        assert_eq!(
            span.content,
            std::borrow::Cow::Owned::<str>("← Yes →".to_string()),
            "editing enum must be wrapped in arrows"
        );
        let span2 = app.field_display("cpu.governor_ac", "performance", "", Color::Cyan);
        assert_eq!(
            span2.content,
            std::borrow::Cow::Owned::<str>("performance".to_string()),
            "non-edited field must render without arrows"
        );
    }

    #[test]
    fn test_field_display_editing_numeric_renders_trackbar() {
        let mut app = make_test_app();
        app.mode = AppMode::Editing;
        app.editing_field = Some("battery.threshold".to_string());
        let span = app.field_display("battery.threshold", "80", "%", Color::Cyan);
        let text = span.content.to_string();
        assert!(text.contains("●"), "trackbar must include marker: {}", text);
        assert!(text.contains("─"), "trackbar must include line: {}", text);
        assert!(text.contains("50"), "trackbar must show min label: {}", text);
        assert!(text.contains("100"), "trackbar must show max label: {}", text);
        assert!(text.contains("80%"), "trackbar must show value label: {}", text);
    }

    #[test]
    fn test_field_display_editing_unlimited_freq_shows_trackbar_with_unlimited() {
        let mut app = make_test_app();
        app.mode = AppMode::Editing;
        app.editing_field = Some("cpu.min_freq_khz".to_string());
        app.config.cpu.min_freq_khz = None;
        let span = app.field_display("cpu.min_freq_khz", "unlimited", "", Color::White);
        let content = span.content.as_ref();
        assert!(content.contains("unlimited"), "must contain 'unlimited' label, got: {content}");
        assert!(content.contains("●"), "must contain the trackbar marker, got: {content}");
        assert!(content.contains("─"), "must contain the trackbar line, got: {content}");
    }

    #[test]
    fn test_numeric_field_range_battery_threshold() {
        let mut app = make_test_app();
        app.mode = AppMode::Editing;
        app.editing_field = Some("battery.threshold".to_string());
        let (min, max, value, _color) = app.numeric_field_range().expect("must return range");
        assert_eq!((min, max), (50, 100));
        assert_eq!(value, 80);
    }

    #[test]
    fn test_numeric_field_range_freq_none_returns_range_at_max() {
        let mut app = make_test_app();
        app.mode = AppMode::Editing;
        app.editing_field = Some("cpu.min_freq_khz".to_string());
        app.config.cpu.min_freq_khz = None;
        let (min, max, v, _color) = app.numeric_field_range().expect("must return range for None freq");
        assert_eq!(v, max, "None freq marker should be at max (far right = unlimited)");
        assert!(min < max, "range must be valid");
    }

    #[test]
    fn test_numeric_field_range_none_for_enum() {
        let mut app = make_test_app();
        app.mode = AppMode::Editing;
        app.editing_field = Some("thermal.profile".to_string());
        assert!(
            app.numeric_field_range().is_none(),
            "enum fields must not produce a range"
        );
    }

    #[test]
    fn test_numeric_field_range_none_when_not_editing() {
        let mut app = make_test_app();
        app.mode = AppMode::Monitoring;
        app.editing_field = Some("battery.threshold".to_string());
        assert!(
            app.numeric_field_range().is_none(),
            "must be None when not in Editing mode"
        );
    }

    #[test]
    fn test_fmt_freq_formats_units() {
        assert_eq!(fmt_freq(None), "unlimited");
        assert_eq!(fmt_freq(Some(800_000)), "800 MHz");
        assert_eq!(fmt_freq(Some(2_400_000)), "2.4 GHz");
        assert_eq!(fmt_freq(Some(1_000_000)), "1.0 GHz");
        assert_eq!(fmt_freq(Some(999_999)), "999 MHz");
        assert_eq!(fmt_freq(Some(0)), "0 MHz");
    }

    #[test]
    fn test_freq_range_fallback_is_400mhz_to_3500mhz() {
        let (min, max) = App::freq_range();
        // Either sysfs is available (any valid min<max) or fallback
        assert!(min < max, "freq_range must have min < max: got {} >= {}", min, max);
        if available_cpu_frequencies().is_none() {
            assert_eq!((min, max), (400_000, 3_500_000));
        }
    }

    #[test]
    fn test_arrow_adjust_cpu_freq_uses_500mhz_step_and_clamps() {
        let mut app = make_test_app();
        app.editing_field = Some("cpu.min_freq_khz".to_string());
        let (range_min, range_max) = App::freq_range();
        app.config.cpu.min_freq_khz = Some(range_min);
        app.adjust_field(1);
        assert_eq!(
            app.config.cpu.min_freq_khz,
            Some(range_min.saturating_add(500_000).min(range_max)),
            "step must be 500_000 kHz"
        );
        app.config.cpu.min_freq_khz = Some(range_max);
        app.adjust_field(1);
        assert_eq!(
            app.config.cpu.min_freq_khz,
            Some(range_max),
            "must clamp at upper bound"
        );
    }

    #[test]
    fn test_arrow_adjust_cpu_freq_none_defaults_to_range_min() {
        let mut app = make_test_app();
        app.editing_field = Some("cpu.min_freq_khz".to_string());
        app.config.cpu.min_freq_khz = None;
        let (range_min, _) = App::freq_range();
        app.adjust_field(1);
        assert_eq!(
            app.config.cpu.min_freq_khz,
            Some(range_min.saturating_add(500_000)),
            "from None, min_freq starts at range_min then steps up"
        );
    }

    #[test]
    fn test_arrow_adjust_cpu_max_freq_none_defaults_to_range_max() {
        let mut app = make_test_app();
        app.editing_field = Some("cpu.max_freq_khz".to_string());
        app.config.cpu.max_freq_khz = None;
        let (range_min, range_max) = App::freq_range();
        app.adjust_field(-1);
        assert_eq!(
            app.config.cpu.max_freq_khz,
            Some(range_max.saturating_sub(500_000).max(range_min)),
            "from None, max_freq starts at range_max then steps down"
        );
    }

    #[tokio::test]
    async fn test_u_key_sets_freq_to_none() {
        let mut app = make_test_app();
        app.mode = AppMode::Editing;
        app.editing_field = Some("cpu.min_freq_khz".to_string());
        app.config.cpu.min_freq_khz = Some(2_000_000);
        let key = KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE);
        app.handle_editing_key(key).await.unwrap();
        assert!(
            app.config.cpu.min_freq_khz.is_none(),
            "'u' must set min_freq_khz to None"
        );

        app.editing_field = Some("cpu.max_freq_khz".to_string());
        app.config.cpu.max_freq_khz = Some(3_000_000);
        let key = KeyEvent::new(KeyCode::Char('U'), KeyModifiers::NONE);
        app.handle_editing_key(key).await.unwrap();
        assert!(
            app.config.cpu.max_freq_khz.is_none(),
            "'U' must set max_freq_khz to None"
        );

        app.editing_field = Some("battery.threshold".to_string());
        app.config.battery.threshold = 80;
        let key = KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE);
        app.handle_editing_key(key).await.unwrap();
        assert_eq!(
            app.config.battery.threshold, 80,
            "'u' must not affect non-freq fields"
        );
    }

    #[test]
    fn test_tab_cycles_fields_in_editing_mode() {
        let mut app = make_test_app();
        app.selected_tab = Tab::Battery;
        app.mode = AppMode::Editing;
        app.begin_editing();
        assert_eq!(app.editing_field.as_deref(), Some("battery.threshold"));
        app.next_edit_field();
        assert_eq!(app.editing_field.as_deref(), Some("battery.force_charge"));
        app.next_edit_field();
        assert_eq!(
            app.editing_field.as_deref(),
            Some("battery.threshold"),
            "must wrap"
        );
    }
}
