pub mod chrome;
pub mod status;
pub mod battery;
pub mod cpu;
pub mod thermal;
pub mod popup;
pub mod help;
pub mod widgets;

use ratatui::{
    layout::{Constraint, Direction, Layout},
    Frame,
};
use crate::app::state::AppMode;

use super::App;

impl App {
    pub fn ui(&self, f: &mut Frame) {
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
            self.draw_popup(f, popup_type);
        }
    }

    fn draw_content(&self, f: &mut Frame, area: ratatui::layout::Rect) {
        match self.selected_tab {
            crate::app::state::Tab::Status => self.draw_status_tab(f, area),
            crate::app::state::Tab::Battery => self.draw_battery_tab(f, area),
            crate::app::state::Tab::CPU => self.draw_cpu_tab(f, area),
            crate::app::state::Tab::Thermal => self.draw_thermal_tab(f, area),
            crate::app::state::Tab::Events => self.draw_events_tab(f, area),
        }
    }
}