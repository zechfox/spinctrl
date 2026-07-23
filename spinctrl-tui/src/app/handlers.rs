use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use shared::Command;
use crate::app::state::{AppMode, PopupType, Tab};
use crate::app::editing::EditField;
use crate::error::Result;

use super::App;

impl App {
    pub async fn handle_key_event(&mut self, key: KeyEvent) -> Result<()> {
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

    pub async fn handle_monitoring_key(&mut self, key: KeyEvent) -> Result<()> {
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
            KeyCode::Char('a') if self.selected_tab == Tab::Events => {
                    self.clear_event_filter();
                }
            _ => {}
        }

        Ok(())
    }

    pub async fn handle_editing_key(&mut self, key: KeyEvent) -> Result<()> {
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
            KeyCode::Char('u' | 'U') => {
                match self.editing_field {
                    Some(EditField::CpuMinFreqKhz) => self.config.cpu.min_freq_khz = None,
                    Some(EditField::CpuMaxFreqKhz) => self.config.cpu.max_freq_khz = None,
                    _ => {}
                }
            }
            _ => {}
        }

        Ok(())
    }

    pub fn handle_help_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'h') => {
                self.mode = AppMode::Monitoring;
            }
            _ => {}
        }
    }

    pub async fn handle_popup_key(
        &mut self,
        key: KeyEvent,
        popup_type: PopupType,
    ) -> Result<()> {
        match popup_type {
            PopupType::ConfirmExit => match key.code {
                KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
                    self.should_quit = true;
                }
                KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                    self.mode = AppMode::Monitoring;
                }
                _ => {}
            },
        }

        Ok(())
    }

    pub async fn send_command(&mut self, command: Command) -> Result<()> {
        match self.ipc.send_command(&command) {
            Ok(()) => {
                log::info!("Command sent successfully: {command:?}");
            }
            Err(e) => {
                self.error_message = Some(format!("Failed to send command: {e}"));
                log::error!("Failed to send command: {e}");
            }
        }

        Ok(())
    }

    pub fn apply_config_in_place(&mut self) {
        if !self.service_available {
            self.error_message = Some(
                "Service offline; changes held in-memory — start the service then press Enter to push".to_string(),
            );
            return;
        }
        if let Err(e) = self.config.validate() {
            self.error_message = Some(format!("Config validation failed: {e}"));
            return;
        }
        let json = match self.config.to_json_compact() {
            Ok(j) => j,
            Err(e) => {
                self.error_message = Some(format!("Failed to serialize config: {e}"));
                return;
            }
        };
        match self.ipc.send_command(&Command::ApplyConfig(json)) {
            Ok(()) => {
                self.error_message = None;
                self.config_backup = self.config.clone();
                log::info!("Configuration applied to service via ApplyConfig");
            }
            Err(e) => {
                self.error_message = Some(format!("Failed to apply config: {e}"));
                log::error!("Failed to apply config: {e}");
            }
        }
    }

    pub fn next_tab(&mut self) {
        self.selected_tab = self.selected_tab.next();
        self.selected_item = 0;
    }

    pub fn previous_tab(&mut self) {
        self.selected_tab = self.selected_tab.previous();
        self.selected_item = 0;
    }
}