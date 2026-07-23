use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};
use crate::app::state::PopupType;

use super::App;

impl App {
    pub fn draw_popup(&self, f: &mut Frame, popup_type: &PopupType) {
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
            .alignment(ratatui::layout::Alignment::Center);

        f.render_widget(paragraph, popup);
    }
}

/// Center a rectangle of `width × height` inside `area`.
pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}