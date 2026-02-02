//! TUI layout and widgets: content area, thinking line, input line.

use ratatui::buffer::Cell;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

use crate::content_layout::{compute_content_rows, ContentAreaWidget};
use crate::content_render::{markdown_to_text, Theme};
use crate::history::HistoryRecord;
use crate::overlay::{theme_label, Overlay, THEMES};
use crate::state::TuiState;

/// Status/thinking needs 3 rows: top border+title, content line, bottom border.
const THINKING_HEIGHT: u16 = 3;
/// Input needs room for wrapped text: top border+title, 2-3 content lines, bottom border.
const INPUT_HEIGHT: u16 = 6;

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

fn history_cache_key(records: &[HistoryRecord], theme: Theme) -> u64 {
    let mut hasher = DefaultHasher::new();
    std::mem::discriminant(&theme).hash(&mut hasher);
    records.len().hash(&mut hasher);
    for r in records {
        std::mem::discriminant(r).hash(&mut hasher);
        match r {
            HistoryRecord::UserMessage { text, .. } => text.hash(&mut hasher),
            HistoryRecord::AssistantContent {
                text, streaming, ..
            } => {
                text.hash(&mut hasher);
                streaming.hash(&mut hasher);
            }
            HistoryRecord::ToolCall {
                tool_name,
                args_preview,
                ..
            } => {
                tool_name.hash(&mut hasher);
                args_preview.hash(&mut hasher);
            }
            HistoryRecord::ToolResult {
                tool_name,
                result_preview,
                is_error,
                ..
            } => {
                tool_name.hash(&mut hasher);
                result_preview.hash(&mut hasher);
                is_error.hash(&mut hasher);
            }
            HistoryRecord::Thinking { message, .. } => message.hash(&mut hasher),
            HistoryRecord::ActionRequired { message, .. } => message.hash(&mut hasher),
            HistoryRecord::Diff { path, hunks, .. } => {
                path.hash(&mut hasher);
                for hunk in hunks {
                    hunk.old_start.hash(&mut hasher);
                    hunk.old_count.hash(&mut hasher);
                    hunk.new_start.hash(&mut hasher);
                    hunk.new_count.hash(&mut hasher);
                    for line in &hunk.lines {
                        std::mem::discriminant(line).hash(&mut hasher);
                        match line {
                            crate::history::DiffLine::Context(s)
                            | crate::history::DiffLine::Add(s)
                            | crate::history::DiffLine::Remove(s) => s.hash(&mut hasher),
                        }
                    }
                }
            }
        }
    }
    hasher.finish()
}

fn truncate_preview(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes.min(s.len());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", s.get(..end).unwrap_or_default())
}

fn record_to_text(record: &HistoryRecord, theme: Theme) -> Text<'static> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let header_style = Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM);
    let error_style = Style::default().fg(Color::Red);

    match record {
        HistoryRecord::UserMessage { text, .. } => {
            lines.push(Line::from(Span::styled("You:".to_string(), header_style)));
            let owned = text.to_string();
            lines.extend(markdown_to_text(&owned, theme).lines.clone());
        }
        HistoryRecord::AssistantContent {
            text, streaming, ..
        } => {
            let label = if *streaming {
                "Assistant (streaming):".to_string()
            } else {
                "Assistant:".to_string()
            };
            lines.push(Line::from(Span::styled(label, header_style)));
            let owned = text.to_string();
            lines.extend(markdown_to_text(&owned, theme).lines.clone());
        }
        HistoryRecord::ToolCall {
            tool_name,
            args_preview,
            ..
        } => {
            lines.push(Line::from(Span::styled(
                format!("─── {} ───", tool_name),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::DIM),
            )));
            let preview = truncate_preview(args_preview, 500);
            lines.push(Line::from(preview));
        }
        HistoryRecord::ToolResult {
            tool_name,
            result_preview,
            is_error,
            ..
        } => {
            let style = if *is_error {
                error_style
            } else {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::DIM)
            };
            lines.push(Line::from(Span::styled(
                format!("{} response:", tool_name),
                style,
            )));
            let preview = truncate_preview(result_preview, 500);
            lines.push(Line::from(preview));
        }
        HistoryRecord::Thinking { message, .. } => {
            let msg = if message.is_empty() {
                "Thinking...".to_string()
            } else {
                message.clone()
            };
            lines.push(Line::from(Span::styled(
                format!(" ⏳ {}", msg),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
            )));
        }
        HistoryRecord::ActionRequired { message, .. } => {
            lines.push(Line::from(Span::styled(
                format!("[Action required] {}", message),
                Style::default().fg(Color::Yellow),
            )));
        }
        HistoryRecord::Diff { path, hunks, .. } => {
            use crate::history::DiffLine;
            lines.push(Line::from(Span::styled(
                format!("diff {}", path),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::DIM),
            )));
            for hunk in hunks {
                for line in &hunk.lines {
                    let (prefix, content, style) = match line {
                        DiffLine::Context(s) => {
                            ("", s.as_str(), Style::default().fg(Color::DarkGray))
                        }
                        DiffLine::Add(s) => ("+", s.as_str(), Style::default().fg(Color::Green)),
                        DiffLine::Remove(s) => ("-", s.as_str(), Style::default().fg(Color::Red)),
                    };
                    lines.push(Line::from(Span::styled(
                        format!("{}{}", prefix, content),
                        style,
                    )));
                }
            }
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(""));
    }
    Text::from(lines)
}

/// Inner content width (area minus left/right border).
fn content_inner_width(area: Rect) -> u16 {
    area.width.saturating_sub(2)
}

/// Inner content height (area minus top/bottom border).
fn content_inner_height(area: Rect) -> u16 {
    area.height.saturating_sub(2)
}

/// Builds flat list of lines from history records (same as before custom layout).
fn build_flat_lines(records: &[HistoryRecord], theme: Theme) -> Vec<Line<'static>> {
    let mut all_lines: Vec<Line<'static>> = Vec::new();
    for record in records {
        let t = record_to_text(record, theme);
        all_lines.extend(t.lines);
        all_lines.push(Line::from(""));
    }
    all_lines
}

/// Returns the content area widget (custom layout with visible rows).
pub fn render_content(state: &Mutex<TuiState>, area: Rect) -> ContentAreaWidget {
    let (records, scroll_offset, scroll_follows_stream, theme) = {
        let s = state.lock().unwrap();
        (
            s.history.records().to_vec(),
            s.scroll_offset,
            s.scroll_follows_stream,
            s.theme,
        )
    };
    let inner_width = content_inner_width(area);
    let viewport_height = content_inner_height(area) as usize;
    let hash = history_cache_key(&records, theme);
    let all_lines = build_flat_lines(&records, theme);
    let rows = compute_content_rows(&all_lines, inner_width, hash);
    let total_rows = rows.len();
    let max_scroll = total_rows.saturating_sub(viewport_height);
    let streaming = records.last().is_some_and(|r| {
        matches!(r, HistoryRecord::AssistantContent { streaming: true, .. })
    });
    let effective_scroll = if viewport_height >= total_rows {
        0
    } else if scroll_follows_stream && streaming {
        max_scroll
    } else {
        scroll_offset.min(max_scroll)
    };
    if effective_scroll != scroll_offset {
        if let Ok(mut s) = state.lock() {
            s.scroll_offset = effective_scroll;
        }
    }
    let visible_len = viewport_height.min(total_rows.saturating_sub(effective_scroll));
    let visible_rows: Vec<Box<[Cell]>> = rows[effective_scroll..][..visible_len].to_vec();
    ContentAreaWidget::new(visible_rows)
}

/// Dots cycle every 400ms when showing "Thinking..." with no custom message.
const THINKING_DOT_MS: u128 = 400;

pub fn render_thinking(state: &Mutex<TuiState>) -> Paragraph<'static> {
    let (thinking_visible, thinking_message, thinking_started_at, session_status) = {
        let s = state.lock().unwrap();
        (
            s.thinking_visible,
            s.thinking_message.clone(),
            s.thinking_started_at,
            s.session_status.clone(),
        )
    };
    let line = if thinking_visible {
        let msg = if thinking_message.is_empty() {
            let dots = thinking_started_at
                .map(|t| {
                    let elapsed = t.elapsed().as_millis();
                    let frame = (elapsed / THINKING_DOT_MS) % 3;
                    match frame {
                        0 => ".",
                        1 => "..",
                        _ => "...",
                    }
                })
                .unwrap_or("...");
            format!("Thinking{}", dots)
        } else {
            thinking_message
        };
        Line::from(format!(" ⏳ {}", msg))
            .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM))
    } else if let Some(ref status) = session_status {
        Line::from(Span::styled(
            status.clone(),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ))
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
        .wrap(Wrap { trim: true })
        .style(Style::default().fg(Color::Green))
}

/// Cursor position for the input field (after " > " + buffer). Accounts for line wrapping.
/// Caller must only use when overlay is not active.
pub fn input_cursor_position(input_area: Rect, buffer_len: usize) -> (u16, u16) {
    const PROMPT_LEN: usize = 3; // " > "
    let inner_width = input_area.width.saturating_sub(2) as usize;
    if inner_width == 0 {
        return (input_area.x + 1, input_area.y + 1);
    }
    let cursor_index = PROMPT_LEN + buffer_len;
    let row = cursor_index / inner_width;
    let col = cursor_index % inner_width;
    let inner_height = input_area.height.saturating_sub(2) as usize;
    let row = row.min(inner_height.saturating_sub(1));
    let x = input_area.x + 1 + (col as u16);
    let y = input_area.y + 1 + (row as u16);
    (x, y)
}

const OVERLAY_HEIGHT: u16 = 8;

pub fn overlay_area(area: Rect) -> Rect {
    let overlay_h = area.height.min(OVERLAY_HEIGHT);
    let y = area.y + area.height.saturating_sub(overlay_h);
    Rect {
        x: area.x,
        y,
        width: area.width,
        height: overlay_h,
    }
}

pub fn content_area_with_overlay(area: Rect, overlay_active: bool) -> Rect {
    if overlay_active {
        Rect {
            height: area.height.saturating_sub(OVERLAY_HEIGHT),
            ..area
        }
    } else {
        area
    }
}

pub fn render_overlay(state: &Mutex<TuiState>, _area: Rect) -> Option<Paragraph<'static>> {
    let overlay = state.lock().unwrap().overlay.clone();
    match overlay {
        Overlay::None => None,
        Overlay::ThemePicker { selected } => {
            let mut lines = vec![Line::from(Span::styled(
                " Theme (↑/↓ select, Enter apply, Esc close) ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))];
            for (i, &theme) in THEMES.iter().enumerate() {
                let marker = if i == selected { "▸ " } else { "  " };
                let style = if i == selected {
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                lines.push(Line::from(Span::styled(
                    format!("{}{}", marker, theme_label(theme)),
                    style,
                )));
            }
            Some(
                Paragraph::new(Text::from(lines))
                    .block(Block::default().borders(Borders::ALL).title(" Theme "))
                    .style(Style::default().fg(Color::White)),
            )
        }
        Overlay::Settings => {
            let (model_line, theme_line, approval_line) = {
                let s = state.lock().unwrap();
                let model = goose::config::Config::global()
                    .get_goose_model()
                    .unwrap_or_else(|_| "not set".to_string());
                let theme_name = theme_label(s.theme);
                let mode = goose::config::Config::global()
                    .get_goose_mode()
                    .unwrap_or(goose::config::GooseMode::Auto);
                let approval_label = match mode {
                    goose::config::GooseMode::Auto => "auto (no approval)",
                    goose::config::GooseMode::Approve => "approve (ask before)",
                    goose::config::GooseMode::SmartApprove => "smart_approve",
                    goose::config::GooseMode::Chat => "chat (no tools)",
                };
                (
                    format!(" Model:   {}", model),
                    format!(" Theme:   {} (Ctrl+T to change)", theme_name),
                    format!(" Approval: {} ", approval_label),
                )
            };
            let lines = vec![
                Line::from(Span::styled(
                    " Settings (read from config) ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(model_line),
                Line::from(theme_line),
                Line::from(approval_line),
                Line::from(Span::styled(
                    " [Esc] Close ",
                    Style::default().fg(Color::DarkGray),
                )),
            ];
            Some(
                Paragraph::new(Text::from(lines))
                    .block(Block::default().borders(Borders::ALL).title(" Settings "))
                    .style(Style::default()),
            )
        }
        Overlay::Approval {
            tool_name, message, ..
        } => {
            let lines = vec![
                Line::from(Span::styled(
                    format!(" Approve tool: {}? ", tool_name),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(message),
                Line::from(Span::styled(
                    " [Y] Approve  [N] Deny  [Esc] Cancel ",
                    Style::default().fg(Color::DarkGray),
                )),
            ];
            Some(
                Paragraph::new(Text::from(lines))
                    .block(Block::default().borders(Borders::ALL).title(" Approval "))
                    .style(Style::default()),
            )
        }
        Overlay::FilePicker => Some(
            Paragraph::new(Line::from("File picker (placeholder)"))
                .block(Block::default().borders(Borders::ALL).title(" File "))
                .style(Style::default()),
        ),
    }
}
