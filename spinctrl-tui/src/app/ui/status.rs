use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use crate::app::ui::widgets::zone_temp_color;

use super::App;

impl App {
    pub fn draw_status_tab(&self, f: &mut Frame, area: Rect) {
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
                        .map_or_else(|| "n/a".to_string(), |f| format!("{f} kHz")),
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
            format!(
                "PID {} · updated {}",
                status.service_pid,
                status.timestamp.with_timezone(&chrono::Local).format("%H:%M:%S")
            ),
            Style::default().fg(Color::DarkGray),
        )));

        let paragraph = Paragraph::new(Text::from(lines));
        f.render_widget(paragraph, inner);
    }
}