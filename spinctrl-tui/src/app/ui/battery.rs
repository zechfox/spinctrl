use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Gauge, Paragraph},
    Frame,
};
use crate::app::editing::EditField;
use crate::app::ui::widgets::field_description;

use super::App;

impl App {
    pub fn draw_battery_tab(&self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Gauge
                Constraint::Length(1), // Spacer
                Constraint::Min(0),   // Config + status details
            ])
            .split(area);

        let capacity = self.status.as_ref().map_or(0, |s| s.battery.capacity);
        let charging = self.status.as_ref().is_some_and(|s| s.battery.charging);
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
            .label(format!("{capacity}%"));
        f.render_widget(gauge, chunks[0]);

        let bc = &self.config.battery;
        let mut lines = vec![
            Line::from({
                let threshold_color = if bc.force_charge {
                    Color::DarkGray
                } else {
                    Color::Cyan
                };
                let threshold_note = if bc.force_charge {
                    " (inactive — force charge on)"
                } else {
                    ""
                };
                vec![
                    Span::styled(
                        "Threshold:  ",
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    self.field_display(
                        EditField::BatteryThreshold,
                        &bc.threshold.to_string(),
                        "%",
                        threshold_color,
                    ),
                    Span::raw("  "),
                    Span::styled(
                        format!("{}{}", field_description(EditField::BatteryThreshold), threshold_note),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]
            }),
            Line::from(vec![
                Span::styled(
                    "Force:      ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                self.field_display(
                    EditField::BatteryForceCharge,
                    if bc.force_charge { "Yes" } else { "No" },
                    "",
                    if bc.force_charge { Color::Green } else { Color::White },
                ),
                Span::raw("  "),
                Span::styled(field_description(EditField::BatteryForceCharge), Style::default().fg(Color::DarkGray)),
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
}