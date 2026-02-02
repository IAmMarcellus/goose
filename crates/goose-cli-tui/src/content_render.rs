//! Convert markdown content to ratatui Text (Line/Span) with styles.
//! Used by the TUI content area for headings, bold, italic, inline code, and fenced code blocks.
//! Respects HTML alignment: <p align="center"> and <div align="right"> set per-line alignment.

use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
use ratatui::prelude::Alignment;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle as SyntectFontStyle, ThemeSet};
use syntect::parsing::SyntaxSet;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    Dark,
    Light,
    Ansi,
}

impl Theme {
    pub fn syntect_theme_name(self) -> &'static str {
        match self {
            Theme::Dark => "zenburn",
            Theme::Light => "GitHub",
            Theme::Ansi => "base16",
        }
    }
}

fn style_for_heading(_level: u8) -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

fn code_inline_style() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::DIM)
}

fn link_style() -> Style {
    Style::default()
        .fg(Color::LightBlue)
        .add_modifier(Modifier::UNDERLINED)
}

fn syntect_style_to_ratatui(s: syntect::highlighting::Style) -> Style {
    let fg = s.foreground;
    let ratatui_fg = Color::Rgb(fg.r, fg.g, fg.b);
    let mut style = Style::default().fg(ratatui_fg);
    if s.font_style.contains(SyntectFontStyle::BOLD) {
        style = style.add_modifier(Modifier::BOLD);
    }
    if s.font_style.contains(SyntectFontStyle::ITALIC) {
        style = style.add_modifier(Modifier::ITALIC);
    }
    style
}

pub fn markdown_to_text(content: &str, theme: Theme) -> Text<'static> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current_line: Vec<Span<'static>> = Vec::new();
    let mut style_stack: Vec<Style> = vec![Style::default().fg(Color::White)];
    let mut in_code_block = false;
    let mut code_block_lang: Option<String> = None;
    let mut code_block_buffer = String::new();
    let mut alignment_stack: Vec<Alignment> = vec![Alignment::Left];

    fn flush_line(
        lines: &mut Vec<Line<'static>>,
        current: &mut Vec<Span<'static>>,
        alignment: Alignment,
    ) {
        if !current.is_empty() {
            lines.push(Line::from(current.clone()).alignment(alignment));
            current.clear();
        }
    }

    let current_alignment = |stack: &[Alignment]| stack.last().copied().unwrap_or(Alignment::Left);

    fn parse_html_align(html: &str) -> Option<Alignment> {
        let html_lower = html.to_lowercase();
        if html_lower.contains("align=\"center\"") || html_lower.contains("align='center'") {
            return Some(Alignment::Center);
        }
        if html_lower.contains("align=\"right\"") || html_lower.contains("align='right'") {
            return Some(Alignment::Right);
        }
        None
    }

    fn current_style(stack: &[Style]) -> Style {
        stack.iter().fold(Style::default(), |acc, s| {
            let mut out = acc;
            if let Some(c) = s.fg {
                out = out.fg(c);
            }
            out = out.add_modifier(s.add_modifier);
            out
        })
    }

    for event in Parser::new(content) {
        match &event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    flush_line(
                        &mut lines,
                        &mut current_line,
                        current_alignment(&alignment_stack),
                    );
                }
                Tag::Heading { level, .. } => {
                    flush_line(
                        &mut lines,
                        &mut current_line,
                        current_alignment(&alignment_stack),
                    );
                    let style = style_for_heading(*level as u8);
                    style_stack.push(style);
                }
                Tag::CodeBlock(CodeBlockKind::Fenced(ref info)) => {
                    flush_line(
                        &mut lines,
                        &mut current_line,
                        current_alignment(&alignment_stack),
                    );
                    in_code_block = true;
                    let lang = info
                        .split(|c: char| c.is_whitespace())
                        .next()
                        .unwrap_or("")
                        .trim();
                    code_block_lang = if lang.is_empty() {
                        None
                    } else {
                        Some(lang.to_string())
                    };
                    code_block_buffer.clear();
                }
                Tag::CodeBlock(CodeBlockKind::Indented) => {
                    flush_line(
                        &mut lines,
                        &mut current_line,
                        current_alignment(&alignment_stack),
                    );
                    in_code_block = true;
                    code_block_lang = None;
                    code_block_buffer.clear();
                }
                Tag::Strong => {
                    style_stack.push(Style::default().add_modifier(Modifier::BOLD));
                }
                Tag::Emphasis => {
                    style_stack.push(Style::default().add_modifier(Modifier::ITALIC));
                }
                Tag::Link { .. } => {
                    style_stack.push(link_style());
                }
                Tag::List(_) | Tag::Item => {
                    flush_line(
                        &mut lines,
                        &mut current_line,
                        current_alignment(&alignment_stack),
                    );
                }
                Tag::HtmlBlock => {}
                _ => {}
            },
            Event::End(tag_end) => match tag_end {
                TagEnd::CodeBlock => {
                    flush_code_block(
                        &mut lines,
                        &code_block_buffer,
                        code_block_lang.as_deref(),
                        theme,
                    );
                    in_code_block = false;
                    code_block_lang = None;
                    code_block_buffer.clear();
                }
                TagEnd::Heading(_) | TagEnd::Strong | TagEnd::Emphasis | TagEnd::Link => {
                    if style_stack.len() > 1 {
                        style_stack.pop();
                    }
                }
                TagEnd::Paragraph => {
                    if alignment_stack.len() > 1 {
                        alignment_stack.pop();
                    }
                }
                TagEnd::HtmlBlock => {
                    if alignment_stack.len() > 1 {
                        alignment_stack.pop();
                    }
                }
                _ => {}
            },
            Event::Html(html) => {
                let html_str = html.as_ref();
                if let Some(align) = parse_html_align(html_str) {
                    alignment_stack.push(align);
                }
            }
            Event::InlineHtml(html) => {
                let html_str = html.as_ref();
                if let Some(align) = parse_html_align(html_str) {
                    alignment_stack.push(align);
                }
            }
            Event::Text(s) => {
                let t = s.to_string();
                if in_code_block {
                    code_block_buffer.push_str(&t);
                } else {
                    let style = current_style(&style_stack);
                    current_line.push(Span::styled(t, style));
                }
            }
            Event::Code(s) => {
                let t = s.to_string();
                current_line.push(Span::styled(t, code_inline_style()));
            }
            Event::SoftBreak => {
                current_line.push(Span::raw(" "));
            }
            Event::HardBreak => {
                flush_line(
                    &mut lines,
                    &mut current_line,
                    current_alignment(&alignment_stack),
                );
            }
            Event::Rule => {
                flush_line(
                    &mut lines,
                    &mut current_line,
                    current_alignment(&alignment_stack),
                );
                current_line.push(Span::styled(
                    "─".repeat(40),
                    Style::default().add_modifier(Modifier::DIM),
                ));
                flush_line(
                    &mut lines,
                    &mut current_line,
                    current_alignment(&alignment_stack),
                );
            }
            _ => {}
        }
    }

    if in_code_block && !code_block_buffer.is_empty() {
        flush_code_block(
            &mut lines,
            &code_block_buffer,
            code_block_lang.as_deref(),
            theme,
        );
    } else {
        flush_line(
            &mut lines,
            &mut current_line,
            current_alignment(&alignment_stack),
        );
    }

    Text::from(lines)
}

fn flush_code_block(lines: &mut Vec<Line<'static>>, code: &str, lang: Option<&str>, theme: Theme) {
    if code.is_empty() {
        lines.push(Line::from(""));
        return;
    }

    static SYNTAX_SET: std::sync::OnceLock<SyntaxSet> = std::sync::OnceLock::new();
    static THEME_SET: std::sync::OnceLock<ThemeSet> = std::sync::OnceLock::new();

    let ps = SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines);
    let ts = THEME_SET.get_or_init(ThemeSet::load_defaults);

    let theme_name = theme.syntect_theme_name();
    let syn_theme = ts
        .themes
        .get(theme_name)
        .or_else(|| ts.themes.keys().next().and_then(|k| ts.themes.get(k)));

    let Some(syn_theme) = syn_theme else {
        for line in code.lines() {
            lines.push(Line::from(Span::styled(
                line.to_string(),
                code_inline_style(),
            )));
        }
        return;
    };

    let syntax = lang
        .and_then(|l| ps.find_syntax_by_token(l))
        .or_else(|| lang.and_then(|l| ps.find_syntax_by_extension(l)))
        .or_else(|| Some(ps.find_syntax_plain_text()));

    let Some(syntax) = syntax else {
        for line in code.lines() {
            lines.push(Line::from(Span::styled(
                line.to_string(),
                code_inline_style(),
            )));
        }
        return;
    };

    let mut highlighter = HighlightLines::new(syntax, syn_theme);

    for line in code.lines() {
        let line_with_newline = format!("{}\n", line);
        match highlighter.highlight_line(&line_with_newline, ps) {
            Ok(ranges) => {
                let spans: Vec<Span<'static>> = ranges
                    .into_iter()
                    .map(|(style, s)| Span::styled(s.to_string(), syntect_style_to_ratatui(style)))
                    .collect();
                if spans.is_empty() {
                    lines.push(Line::from(""));
                } else {
                    lines.push(Line::from(spans));
                }
            }
            Err(_) => {
                lines.push(Line::from(Span::styled(
                    line.to_string(),
                    code_inline_style(),
                )));
            }
        }
    }
}
