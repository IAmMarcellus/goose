//! TUI layout and widgets: content area, thinking line, input line.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use std::sync::Mutex;

use crate::state::TuiState;

const THINKING_HEIGHT: u16 = 1;
const INPUT_HEIGHT: u16 = 1;

pub fn layout_chunks(area: Rect) -> (Rect, Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(THINKING_HEIGHT),
            Constraint::Length(INPUT_HEIGHT),
        ])
        .split(area);
    (chunks[0], chunks[1], chunks[2])
}

pub fn render_content(state: &Mutex<TuiState>, area: Rect) -> Paragraph<'static> {
    let (content, scroll_offset) = {
        let s = state.lock().unwrap();
        (s.content.clone(), s.scroll_offset)
    };
    let line_count = content.lines().count();
    let viewport_height = area.height as usize;
    let scroll_y = if line_count > viewport_height {
        (line_count - viewport_height).min(scroll_offset as usize)
    } else {
        0
    };

    Paragraph::new(content)
        .block(Block::default().borders(Borders::ALL).title(" Content "))
        .wrap(Wrap { trim: true })
        .scroll((scroll_y as u16, 0))
        .style(Style::default().fg(Color::White))
}

pub fn render_thinking(state: &Mutex<TuiState>) -> Paragraph<'static> {
    let (thinking_visible, thinking_message) = {
        let s = state.lock().unwrap();
        (s.thinking_visible, s.thinking_message.clone())
    };
    let line = if thinking_visible {
        let msg = if thinking_message.is_empty() {
            "Thinking..."
        } else {
            thinking_message.as_str()
        };
        Line::from(format!(" ⏳ {}", msg))
            .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM))
    } else {
        Line::from("")
    };

    Paragraph::new(line)
        .block(Block::default().borders(Borders::ALL).title(" Status "))
        .style(Style::default())
}

pub fn render_input(state: &Mutex<TuiState>) -> Paragraph<'static> {
    let input_buffer = state.lock().unwrap().input_buffer.clone();
    let line = Line::from(format!(" > {}", input_buffer));
    Paragraph::new(line)
        .block(Block::default().borders(Borders::ALL).title(" Input "))
        .style(Style::default().fg(Color::Green))
}
