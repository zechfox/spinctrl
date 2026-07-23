use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use super::App;

impl App {
    pub fn draw_help(&self, f: &mut Frame, area: Rect) {
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