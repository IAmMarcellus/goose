use std::io::{self, IsTerminal, Write};
use std::sync::{Arc, Mutex};
use streamdown_parser::ParseEvent;
use streamdown_parser::Parser;
use streamdown_render::{RenderStyle, Renderer};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Channel capacity 1 so send_chunk().await yields after each chunk, letting the render task run
/// and produce output incrementally instead of batching many chunks.
const STREAM_MARKDOWN_CHANNEL_CAP: usize = 1;

#[derive(Debug)]
pub enum StreamMarkdownCmd {
    Chunk(String),
    Flush,
}

pub struct StreamMarkdownHandle {
    tx: mpsc::Sender<StreamMarkdownCmd>,
    join: JoinHandle<std::io::Result<()>>,
}

/// Writer that appends to a shared buffer so we can reuse one Renderer (and its list state) across lines.
struct SharedWriter(Arc<Mutex<Vec<u8>>>);
impl Write for SharedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().write_all(buf)?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().unwrap().flush()
    }
}

/// Bat/syntect theme names used by goose for non-streaming; reused for streamdown code blocks.
fn syntax_theme_name(theme: crate::session::output::Theme) -> &'static str {
    match theme {
        crate::session::output::Theme::Light => "GitHub",
        crate::session::output::Theme::Dark => "zenburn",
        crate::session::output::Theme::Ansi => "base16",
    }
}

fn render_style_for_theme(theme: crate::session::output::Theme) -> RenderStyle {
    match theme {
        crate::session::output::Theme::Light => RenderStyle {
            bright: "#0969da".to_string(),
            head: "#1f2328".to_string(),
            symbol: "#8250df".to_string(),
            grey: "#656d76".to_string(),
            dark: "#f6f8fa".to_string(),
            mid: "#eaeef2".to_string(),
            light: "#d0d7de".to_string(),
        },
        crate::session::output::Theme::Dark => RenderStyle::default(),
        crate::session::output::Theme::Ansi => RenderStyle {
            bright: "#6cb6ff".to_string(),
            head: "#56d364".to_string(),
            symbol: "#d2a8ff".to_string(),
            grey: "#8b949e".to_string(),
            dark: "#0d1117".to_string(),
            mid: "#161b22".to_string(),
            light: "#21262d".to_string(),
        },
    }
}

pub fn use_stream_markdown() -> bool {
    if !std::io::stdout().is_terminal() {
        return false;
    }
    match std::env::var("GOOSE_CLI_STREAM_MARKDOWN") {
        Ok(v) => !v.eq_ignore_ascii_case("false") && !v.eq_ignore_ascii_case("0"),
        Err(_) => true,
    }
}

pub struct StreamMarkdown {
    parser: Parser,
    line_buffer: String,
    shared_buffer: Arc<Mutex<Vec<u8>>>,
    renderer: Renderer<SharedWriter>,
    #[cfg(test)]
    test_out: Option<Arc<Mutex<Vec<u8>>>>,
}

impl StreamMarkdown {
    pub fn new() -> Self {
        Self::new_with_theme(crate::session::output::get_theme())
    }

    fn new_with_theme_impl(
        width: usize,
        shared_buffer: Arc<Mutex<Vec<u8>>>,
        theme: crate::session::output::Theme,
        #[cfg(test)] test_out: Option<Arc<Mutex<Vec<u8>>>>,
    ) -> Self {
        let style = render_style_for_theme(theme);
        let syntax_name = syntax_theme_name(theme);
        let mut renderer =
            Renderer::with_style(SharedWriter(Arc::clone(&shared_buffer)), width, style);
        renderer.set_theme(syntax_name);
        Self {
            parser: Parser::new(),
            line_buffer: String::new(),
            shared_buffer,
            renderer,
            #[cfg(test)]
            test_out,
        }
    }

    fn new_with_theme(theme: crate::session::output::Theme) -> Self {
        let width = console::Term::stdout()
            .size_checked()
            .map(|(_, w)| w as usize)
            .unwrap_or(80);
        let shared_buffer = Arc::new(Mutex::new(Vec::new()));
        Self::new_with_theme_impl(
            width,
            shared_buffer,
            theme,
            #[cfg(test)]
            None,
        )
    }

    #[cfg(test)]
    pub fn new_for_test(width: usize) -> (Self, Arc<Mutex<Vec<u8>>>) {
        let out = Arc::new(Mutex::new(Vec::new()));
        let shared_buffer = Arc::new(Mutex::new(Vec::new()));
        let theme = crate::session::output::get_theme();
        let sm = Self::new_with_theme_impl(width, shared_buffer, theme, Some(Arc::clone(&out)));
        (sm, out)
    }

    #[cfg(test)]
    pub fn new_for_test_with_theme(
        width: usize,
        theme: crate::session::output::Theme,
    ) -> (Self, Arc<Mutex<Vec<u8>>>) {
        let out = Arc::new(Mutex::new(Vec::new()));
        let shared_buffer = Arc::new(Mutex::new(Vec::new()));
        let sm = Self::new_with_theme_impl(width, shared_buffer, theme, Some(Arc::clone(&out)));
        (sm, out)
    }

    pub fn push_chunk(&mut self, text: &str) -> io::Result<()> {
        self.line_buffer.push_str(text);
        while let Some((line, rest)) = self.line_buffer.split_once('\n') {
            let line = line.to_string();
            let rest = rest.to_string();
            self.line_buffer = rest;
            self.render_line(&line)?;
        }
        Ok(())
    }

    pub fn flush(&mut self) -> io::Result<()> {
        if !self.line_buffer.is_empty() {
            let line = std::mem::take(&mut self.line_buffer);
            self.render_line(&line)?;
        }
        self.flush_out()
    }

    fn flush_out(&mut self) -> io::Result<()> {
        #[cfg(test)]
        if let Some(ref out) = self.test_out {
            return out.lock().unwrap().flush();
        }
        io::stdout().flush()
    }

    fn write_out(&mut self, buf: &[u8]) -> io::Result<()> {
        #[cfg(test)]
        if let Some(ref out) = self.test_out {
            out.lock().unwrap().write_all(buf)?;
            return Ok(());
        }
        io::stdout().write_all(buf)
    }

    fn render_line(&mut self, line: &str) -> io::Result<()> {
        self.shared_buffer.lock().unwrap().clear();
        for event in self.parser.parse_line(line) {
            // Skip ListEnd so the renderer's list state is not reset between items.
            // Otherwise nested content (bullets, indented text) causes ListEnd, and the next
            // "1. ..." is rendered as "1." again instead of "2.", "3.", etc.
            if matches!(&event, ParseEvent::ListEnd) {
                continue;
            }
            self.renderer.render_event(&event)?;
        }
        let buf = self.shared_buffer.lock().unwrap().clone();
        self.write_out(&buf)?;
        self.flush_out()
    }
}

impl Default for StreamMarkdown {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) async fn run_stream_markdown_task(
    mut sm: StreamMarkdown,
    mut rx: mpsc::Receiver<StreamMarkdownCmd>,
) -> std::io::Result<()> {
    while let Some(cmd) = rx.recv().await {
        match cmd {
            StreamMarkdownCmd::Chunk(s) => {
                if let Err(e) = sm.push_chunk(&s) {
                    tracing::warn!("stream markdown push_chunk: {}", e);
                }
            }
            StreamMarkdownCmd::Flush => {
                if let Err(e) = sm.flush() {
                    tracing::warn!("stream markdown flush: {}", e);
                }
            }
        }
    }
    let _ = sm.flush();
    Ok(())
}

pub fn start_stream_markdown() -> StreamMarkdownHandle {
    let (tx, rx) = mpsc::channel(STREAM_MARKDOWN_CHANNEL_CAP);
    let sm = StreamMarkdown::new();
    let join = tokio::spawn(async move { run_stream_markdown_task(sm, rx).await });
    StreamMarkdownHandle { tx, join }
}

impl StreamMarkdownHandle {
    pub async fn send_chunk(&self, text: &str) {
        let _ = self.tx.send(StreamMarkdownCmd::Chunk(text.to_string())).await;
    }

    pub async fn flush_and_drop(self) -> std::io::Result<()> {
        let _ = self.tx.send(StreamMarkdownCmd::Flush).await;
        drop(self.tx);
        self.join.await.map_err(|e| {
            std::io::Error::other(format!("stream markdown task join: {}", e))
        })?
    }
}

pub async fn flush_and_drop_handle(handle: &mut Option<StreamMarkdownHandle>) {
    if let Some(h) = handle.take() {
        let _ = h.flush_and_drop().await;
    }
}

#[cfg(test)]
mod tests {
    // Manual validation (from plan): (1) TTY: run a session that streams (e.g. simple question),
    // confirm markdown (headers, bold, code blocks) appears incrementally. (2) Non-TTY: pipe
    // output (e.g. `goose ... | cat`) and confirm raw text, no ANSI from streamdown.
    use super::*;

    #[test]
    fn use_stream_markdown_false_when_env_false() {
        let old = std::env::var("GOOSE_CLI_STREAM_MARKDOWN").ok();
        std::env::set_var("GOOSE_CLI_STREAM_MARKDOWN", "false");
        assert!(!use_stream_markdown());
        if let Some(v) = old {
            std::env::set_var("GOOSE_CLI_STREAM_MARKDOWN", v);
        } else {
            std::env::remove_var("GOOSE_CLI_STREAM_MARKDOWN");
        }
    }

    #[test]
    fn use_stream_markdown_false_when_env_zero() {
        let old = std::env::var("GOOSE_CLI_STREAM_MARKDOWN").ok();
        std::env::set_var("GOOSE_CLI_STREAM_MARKDOWN", "0");
        assert!(!use_stream_markdown());
        if let Some(v) = old {
            std::env::set_var("GOOSE_CLI_STREAM_MARKDOWN", v);
        } else {
            std::env::remove_var("GOOSE_CLI_STREAM_MARKDOWN");
        }
    }

    #[test]
    fn stream_markdown_renders_line_to_buffer() {
        let (mut sm, out) = StreamMarkdown::new_for_test(80);
        sm.push_chunk("# Hi\n").unwrap();
        sm.flush().unwrap();
        let buf = out.lock().unwrap();
        let s = String::from_utf8_lossy(&buf);
        assert!(
            s.contains("Hi"),
            "rendered output should contain 'Hi': {:?}",
            s
        );
    }

    #[test]
    fn stream_markdown_numbered_list_shows_1_2_3() {
        let (mut sm, out) = StreamMarkdown::new_for_test(80);
        sm.push_chunk("1. First\n2. Second\n3. Third\n").unwrap();
        sm.flush().unwrap();
        let buf = out.lock().unwrap();
        let s = String::from_utf8_lossy(&buf);
        assert!(
            s.contains("1.") && s.contains("First"),
            "expected '1. First': {:?}",
            s
        );
        assert!(
            s.contains("2.") && s.contains("Second"),
            "expected '2. Second': {:?}",
            s
        );
        assert!(
            s.contains("3.") && s.contains("Third"),
            "expected '3. Third': {:?}",
            s
        );
    }

    #[test]
    fn stream_markdown_numbered_list_after_nested_content_shows_2_and_3() {
        let (mut sm, out) = StreamMarkdown::new_for_test(80);
        sm.push_chunk("1. First\n").unwrap();
        sm.push_chunk("    sub text\n").unwrap();
        sm.push_chunk("2. Second\n").unwrap();
        sm.push_chunk("3. Third\n").unwrap();
        sm.flush().unwrap();
        let buf = out.lock().unwrap();
        let s = String::from_utf8_lossy(&buf);
        assert!(
            s.contains("1.") && s.contains("First"),
            "expected '1. First': {:?}",
            s
        );
        assert!(
            s.contains("2.") && s.contains("Second"),
            "expected '2. Second' after nested: {:?}",
            s
        );
        assert!(
            s.contains("3.") && s.contains("Third"),
            "expected '3. Third': {:?}",
            s
        );
    }

    /// Same as user scenario: model outputs "1." for every top-level item, with nested bullets in between.
    #[test]
    fn stream_markdown_numbered_list_with_nested_bullets_all_ones_in_source() {
        let (mut sm, out) = StreamMarkdown::new_for_test(80);
        sm.push_chunk("1. SACAgent\n").unwrap();
        sm.push_chunk("    This is the core.\n").unwrap();
        sm.push_chunk("    - Actor-Critic\n").unwrap();
        sm.push_chunk("    - Automatic Entropy\n").unwrap();
        sm.push_chunk("1. SACTrainingAgent\n").unwrap();
        sm.push_chunk("1. SACTradingEnvironment\n").unwrap();
        sm.flush().unwrap();
        let buf = out.lock().unwrap();
        let s = String::from_utf8_lossy(&buf);
        assert!(
            s.contains("1.") && s.contains("SACAgent"),
            "expected '1. SACAgent': {:?}",
            s
        );
        assert!(
            s.contains("2.") && s.contains("SACTrainingAgent"),
            "expected '2. SACTrainingAgent': {:?}",
            s
        );
        assert!(
            s.contains("3.") && s.contains("SACTradingEnvironment"),
            "expected '3. SACTradingEnvironment': {:?}",
            s
        );
    }

    #[test]
    fn stream_markdown_numbered_list_empty_line_between_items_still_increments() {
        let (mut sm, out) = StreamMarkdown::new_for_test(80);
        sm.push_chunk("1. First\n").unwrap();
        sm.push_chunk("\n").unwrap();
        sm.push_chunk("1. Second\n").unwrap();
        sm.flush().unwrap();
        let buf = out.lock().unwrap();
        let s = String::from_utf8_lossy(&buf);
        assert!(
            s.contains("1.") && s.contains("First"),
            "expected '1. First': {:?}",
            s
        );
        assert!(
            s.contains("2.") && s.contains("Second"),
            "expected '2. Second' after empty line: {:?}",
            s
        );
    }

    #[test]
    fn stream_markdown_theme_light_produces_styled_output() {
        let (mut sm, out) =
            StreamMarkdown::new_for_test_with_theme(80, crate::session::output::Theme::Light);
        sm.push_chunk("# Title\n").unwrap();
        sm.flush().unwrap();
        let buf = out.lock().unwrap();
        let s = String::from_utf8_lossy(&buf);
        assert!(!s.is_empty());
        assert!(s.contains("Title"));
        assert!(s.contains("\x1b["), "Light theme should emit ANSI: {:?}", s);
    }

    #[test]
    fn stream_markdown_theme_dark_produces_styled_output() {
        let (mut sm, out) =
            StreamMarkdown::new_for_test_with_theme(80, crate::session::output::Theme::Dark);
        sm.push_chunk("# Title\n").unwrap();
        sm.flush().unwrap();
        let buf = out.lock().unwrap();
        let s = String::from_utf8_lossy(&buf);
        assert!(!s.is_empty());
        assert!(s.contains("Title"));
        assert!(s.contains("\x1b["), "Dark theme should emit ANSI: {:?}", s);
    }

    #[test]
    fn stream_markdown_theme_ansi_produces_styled_output() {
        let (mut sm, out) =
            StreamMarkdown::new_for_test_with_theme(80, crate::session::output::Theme::Ansi);
        sm.push_chunk("# Title\n").unwrap();
        sm.flush().unwrap();
        let buf = out.lock().unwrap();
        let s = String::from_utf8_lossy(&buf);
        assert!(!s.is_empty());
        assert!(s.contains("Title"));
        assert!(s.contains("\x1b["), "Ansi theme should emit ANSI: {:?}", s);
    }

    #[tokio::test]
    async fn handle_send_chunk_and_flush_renders_to_buffer() {
        let (sm, out) = StreamMarkdown::new_for_test(80);
        let (tx, rx) = mpsc::channel(STREAM_MARKDOWN_CHANNEL_CAP);
        let join = tokio::spawn(run_stream_markdown_task(sm, rx));
        let handle = StreamMarkdownHandle { tx, join };
        handle.send_chunk("# Hi\n").await;
        handle.send_chunk("**bold**\n").await;
        handle.flush_and_drop().await.unwrap();
        let buf = out.lock().unwrap();
        let s = String::from_utf8_lossy(&buf);
        assert!(s.contains("Hi"), "expected 'Hi' in output: {:?}", s);
        assert!(s.contains("bold"), "expected 'bold' in output: {:?}", s);
    }

    #[tokio::test]
    async fn handle_multiple_chunks_then_flush_ordering() {
        let (sm, out) = StreamMarkdown::new_for_test(80);
        let (tx, rx) = mpsc::channel(STREAM_MARKDOWN_CHANNEL_CAP);
        let join = tokio::spawn(run_stream_markdown_task(sm, rx));
        let handle = StreamMarkdownHandle { tx, join };
        handle.send_chunk("1. First\n").await;
        handle.send_chunk("2. Second\n").await;
        handle.send_chunk("3. Third\n").await;
        handle.flush_and_drop().await.unwrap();
        let buf = out.lock().unwrap();
        let s = String::from_utf8_lossy(&buf);
        assert!(s.contains("1.") && s.contains("First"), "expected '1. First': {:?}", s);
        assert!(s.contains("2.") && s.contains("Second"), "expected '2. Second': {:?}", s);
        assert!(s.contains("3.") && s.contains("Third"), "expected '3. Third': {:?}", s);
    }
}
