use std::mem::discriminant;
use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};
use shared::{EventEntry, EventType};
use crate::app::ui::widgets::event_type_tag;

use super::App;

impl App {
    /// Cycle the Events-tab filter through:
    /// `None` (All) → `ConfigChanged` → `CommandExecuted` → `HardwareAction` →
    /// `Error` → `ServiceStart` → `ServiceStop` → `None`. Resets `selected_item`
    /// so the highlight stays valid against the newly-filtered list.
    pub fn cycle_event_filter(&mut self) {
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

    pub fn clear_event_filter(&mut self) {
        self.event_filter = None;
        self.selected_item = 0;
        self.scroll_offset = 0;
    }

    pub fn event_filter_label(&self) -> &'static str {
        self.event_filter.as_ref().map_or("All", |e| event_type_tag(e).tag)
    }

    pub fn draw_events_tab(&self, f: &mut Frame, area: Rect) {
        let visible: Vec<&EventEntry> = self.event_filter.as_ref().map_or_else(
            || self.events.iter().collect(),
            |filter| {
                self.events
                    .iter()
                    .filter(|e| discriminant(&e.event_type) == discriminant(filter))
                    .collect()
            },
        );

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
                    "No '{filter_label}' events — press 'a' to show all"
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
                let tag = event_type_tag(&event.event_type);
                let time_str = event
                    .timestamp
                    .with_timezone(&chrono::Local)
                    .format("%m-%d %H:%M:%S")
                    .to_string();
                let content = Line::from(vec![
                    Span::styled(
                        format!("{time_str} "),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        format!("[{}] ", tag.tag),
                        Style::default()
                            .fg(tag.color)
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
}