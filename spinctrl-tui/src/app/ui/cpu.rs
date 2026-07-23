use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use crate::app::editing::EditField;
use crate::app::ui::widgets::field_description;
use crate::app::editing::fmt_freq;

use super::App;

impl App {
    pub fn draw_cpu_tab(&self, f: &mut Frame, area: Rect) {
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
                self.field_display(EditField::CpuGovernorAc, cc.governor_ac.as_str(), "", Color::Cyan),
                Span::raw("  "),
                Span::styled(field_description(EditField::CpuGovernorAc), Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(vec![
                Span::raw("  Battery:  "),
                self.field_display(EditField::CpuGovernorBattery, cc.governor_battery.as_str(), "", Color::Yellow),
                Span::raw("  "),
                Span::styled(field_description(EditField::CpuGovernorBattery), Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "Frequency Limits:",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(vec![
                Span::raw("  Min:  "),
                self.field_display(EditField::CpuMinFreqKhz, &min_display, "", Color::White),
                Span::raw("  "),
                Span::styled(field_description(EditField::CpuMinFreqKhz), Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(vec![
                Span::raw("  Max:  "),
                self.field_display(EditField::CpuMaxFreqKhz, &max_display, "", Color::White),
                Span::raw("  "),
                Span::styled(field_description(EditField::CpuMaxFreqKhz), Style::default().fg(Color::DarkGray)),
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
                        .map_or_else(|| "n/a".to_string(), |f| format!("{f} kHz")),
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
}