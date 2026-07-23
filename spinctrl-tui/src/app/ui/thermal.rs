use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use crate::app::editing::EditField;
use crate::app::ui::widgets::{field_description, thermal_profile_name, zone_temp_color};

use super::App;

impl App {
    pub fn draw_thermal_tab(&self, f: &mut Frame, area: Rect) {
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
                self.field_display(EditField::ThermalProfile, profile_name, "", Color::Cyan),
                Span::raw("  "),
                Span::styled(field_description(EditField::ThermalProfile), Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "Temperature Thresholds (°C):",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(vec![
                Span::raw("  Fan off:      "),
                self.field_display(EditField::ThermalFanOff, &tc.fan_off_temp.to_string(), "", Color::Green),
                Span::raw("  "),
                Span::styled(field_description(EditField::ThermalFanOff), Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(vec![
                Span::raw("  High:         "),
                self.field_display(EditField::ThermalHigh, &tc.high_temp.to_string(), "", Color::Yellow),
                Span::raw("  "),
                Span::styled(field_description(EditField::ThermalHigh), Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(vec![
                Span::raw("  Warning:      "),
                self.field_display(EditField::ThermalWarn, &tc.warn_temp.to_string(), "", Color::Yellow),
                Span::raw("  "),
                Span::styled(field_description(EditField::ThermalWarn), Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(vec![
                Span::raw("  Fan max:      "),
                self.field_display(EditField::ThermalFanMax, &tc.fan_max_temp.to_string(), "", Color::Red),
                Span::raw("  "),
                Span::styled(field_description(EditField::ThermalFanMax), Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(vec![
                Span::raw("  Shutdown:     "),
                self.field_display(EditField::ThermalShutdown, &tc.shutdown_temp.to_string(), "", Color::Red),
                Span::raw("  "),
                Span::styled(field_description(EditField::ThermalShutdown), Style::default().fg(Color::DarkGray)),
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
}