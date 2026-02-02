//! Custom content layout: word-wrap Lines at content width, build cached rows of Buffer cells
//! (alignment-aware), and cache by (history_hash, content_width). Used by the content area widget.

use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;
use ratatui::prelude::Alignment;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Widget};
use std::sync::Mutex;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

static ROWS_CACHE: Mutex<Option<((u64, u16), Vec<Box<[Cell]>>)>> = Mutex::new(None);

/// Returns the width in terminal cells of a line (sum of span content widths).
fn line_width(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum()
}

/// Word-wrap a single line at `max_width`: if the line is longer, split by spaces into
/// multiple lines. Each wrapped line keeps the same alignment and uses a single merged style
/// (first span's style merged with line style) for simplicity.
fn wrap_line(line: &Line<'static>, max_width: usize) -> Vec<Line<'static>> {
    if max_width == 0 {
        return vec![];
    }
    let width = line_width(line);
    if width <= max_width {
        return vec![line.clone()];
    }
    let merged_style = line
        .spans
        .first()
        .map(|s| line.style.patch(s.style))
        .unwrap_or(line.style);
    let text: String = line
        .spans
        .iter()
        .map(|s| s.content.as_ref().to_string())
        .collect();
    let alignment = line.alignment.unwrap_or(Alignment::Left);
    let mut out = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    for word in text.split_whitespace() {
        let word_width = UnicodeWidthStr::width(word);
        let need_space = !current.is_empty();
        let space_width = if need_space { 1 } else { 0 };
        if current_width + space_width + word_width <= max_width {
            if need_space {
                current.push(' ');
                current_width += 1;
            }
            current.push_str(word);
            current_width += word_width;
        } else {
            if !current.is_empty() {
                out.push(
                    Line::from(ratatui::text::Span::styled(current.clone(), merged_style))
                        .alignment(alignment),
                );
            }
            current = word.to_string();
            current_width = word_width;
            if current_width > max_width {
                let mut truncated = String::new();
                let mut w = 0usize;
                for g in word.graphemes(true) {
                    let gw = UnicodeWidthStr::width(g);
                    if w + gw > max_width {
                        break;
                    }
                    truncated.push_str(g);
                    w += gw;
                }
                out.push(
                    Line::from(ratatui::text::Span::styled(truncated, merged_style))
                        .alignment(alignment),
                );
                current.clear();
                current_width = 0;
            }
        }
    }
    if !current.is_empty() {
        out.push(
            Line::from(ratatui::text::Span::styled(current, merged_style)).alignment(alignment),
        );
    }
    if out.is_empty() {
        out.push(Line::from("").alignment(alignment));
    }
    out
}

/// Wrap all lines at `content_width` and flatten into a single list.
fn wrap_lines(lines: &[Line<'static>], content_width: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for line in lines {
        out.extend(wrap_line(line, content_width));
    }
    out
}

/// Build one row of cells from a line at the given width. Respects line alignment:
/// Left: start at 0; Center: start at (width - line_width) / 2; Right: start at width - line_width.
fn build_cached_row(line: &Line<'static>, target_width: usize) -> Box<[Cell]> {
    if target_width == 0 {
        return vec![].into_boxed_slice();
    }
    let mut cells = vec![Cell::default(); target_width];
    let line_width: usize = line
        .spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    let alignment = line.alignment.unwrap_or(Alignment::Left);
    let start_col = match alignment {
        Alignment::Left => 0,
        Alignment::Center => (target_width.saturating_sub(line_width)) / 2,
        Alignment::Right => target_width.saturating_sub(line_width.min(target_width)),
    };
    let mut x = start_col;
    for span in &line.spans {
        let span_style = line.style.patch(span.style);
        for symbol in span.content.as_ref().graphemes(true) {
            if symbol.chars().any(|c| c.is_control()) {
                continue;
            }
            let symbol_width = UnicodeWidthStr::width(symbol) as u16;
            if symbol_width == 0 {
                continue;
            }
            if x >= target_width {
                break;
            }
            let idx = x;
            if idx < target_width {
                cells[idx].set_symbol(symbol).set_style(span_style);
            }
            let w = symbol_width as usize;
            x += w;
            for i in 1..w {
                if x - w + i < target_width {
                    cells[x - w + i].reset();
                }
            }
        }
    }
    cells.into_boxed_slice()
}

/// Compute all content rows from flat lines, with caching. Cache key is (history_cache_key, content_width).
/// Caller must pass the same cache_key that was used for the given lines (e.g. history_cache_key(records, theme)).
pub fn compute_content_rows(
    lines: &[Line<'static>],
    content_width: u16,
    cache_key: u64,
) -> Vec<Box<[Cell]>> {
    let width_usize = content_width as usize;
    if width_usize == 0 {
        return vec![];
    }
    {
        let cache = ROWS_CACHE.lock().unwrap();
        if let Some(((key, w), ref rows)) = *cache {
            if key == cache_key && w == content_width {
                return rows.clone();
            }
        }
    }
    let wrapped = wrap_lines(lines, width_usize);
    let rows: Vec<Box<[Cell]>> = wrapped
        .iter()
        .map(|line| build_cached_row(line, width_usize))
        .collect();
    {
        let mut cache = ROWS_CACHE.lock().unwrap();
        *cache = Some(((cache_key, content_width), rows.clone()));
    }
    rows
}

/// Widget that draws a block and a visible slice of pre-rendered rows into the buffer.
pub struct ContentAreaWidget {
    block: Block<'static>,
    rows: Vec<Box<[Cell]>>,
}

impl ContentAreaWidget {
    pub fn new(rows: Vec<Box<[Cell]>>) -> Self {
        Self {
            block: Block::default().borders(Borders::ALL).title(" Content "),
            rows,
        }
    }
}

impl Widget for ContentAreaWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let inner = self.block.inner(area);
        self.block.render(area, buf);
        for (row_idx, y) in (inner.y..inner.y + inner.height).enumerate() {
            if row_idx >= self.rows.len() {
                break;
            }
            let row = &self.rows[row_idx];
            let row_len = row.len().min(inner.width as usize);
            for (col_idx, cell) in row[..row_len].iter().enumerate() {
                let x = inner.x + col_idx as u16;
                if let Some(c) = buf.cell_mut((x, y)) {
                    c.clone_from(cell);
                }
            }
        }
    }
}
