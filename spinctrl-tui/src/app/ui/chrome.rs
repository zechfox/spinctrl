use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph, Tabs},
    Frame,
};
use crate::app::state::Tab;

use super::App;

impl App {
    pub fn draw_header(&self, f: &mut Frame, area: Rect) {
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

    pub fn draw_footer(&self, f: &mut Frame, area: Rect) {
        let mode_text = match self.mode {
            crate::app::state::AppMode::Monitoring => "MONITOR",
            crate::app::state::AppMode::Editing => "EDIT",
            crate::app::state::AppMode::Help => "HELP",
            crate::app::state::AppMode::Popup(_) => "POPUP",
        };

        let service_status = if self.service_available {
            "Service: Online"
        } else {
            "Service: Offline"
        };

        let footer_text = self.error_message.as_ref().map_or_else(
            || format!(" {mode_text} | {service_status} | Press 'h' for help, 'q' to quit"),
            |msg| format!(" {mode_text} | {service_status} | {msg}"),
        );

        let footer = Paragraph::new(footer_text)
            .style(Style::default().fg(Color::White))
            .block(Block::default().borders(Borders::ALL));

        f.render_widget(footer, area);
    }
}