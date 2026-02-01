use std::io::{self, IsTerminal, Write};
use std::sync::{Arc, Mutex};
use streamdown_parser::ParseEvent;
use streamdown_parser::Parser;
use streamdown_render::Renderer;

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
        let width = console::Term::stdout()
            .size_checked()
            .map(|(_, w)| w as usize)
            .unwrap_or(80);
        let shared_buffer = Arc::new(Mutex::new(Vec::new()));
        let renderer = Renderer::new(SharedWriter(Arc::clone(&shared_buffer)), width);
        Self {
            parser: Parser::new(),
            line_buffer: String::new(),
            shared_buffer,
            renderer,
            #[cfg(test)]
            test_out: None,
        }
    }

    #[cfg(test)]
    pub fn new_for_test(width: usize) -> (Self, Arc<Mutex<Vec<u8>>>) {
        let out = Arc::new(Mutex::new(Vec::new()));
        let shared_buffer = Arc::new(Mutex::new(Vec::new()));
        let renderer = Renderer::new(SharedWriter(Arc::clone(&shared_buffer)), width);
        let sm = Self {
            parser: Parser::new(),
            line_buffer: String::new(),
            shared_buffer,
            renderer,
            test_out: Some(Arc::clone(&out)),
        };
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
        assert!(s.contains("1.") && s.contains("First"), "expected '1. First': {:?}", s);
        assert!(s.contains("2.") && s.contains("Second"), "expected '2. Second': {:?}", s);
        assert!(s.contains("3.") && s.contains("Third"), "expected '3. Third': {:?}", s);
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
        assert!(s.contains("1.") && s.contains("First"), "expected '1. First': {:?}", s);
        assert!(s.contains("2.") && s.contains("Second"), "expected '2. Second' after nested: {:?}", s);
        assert!(s.contains("3.") && s.contains("Third"), "expected '3. Third': {:?}", s);
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
        assert!(s.contains("1.") && s.contains("SACAgent"), "expected '1. SACAgent': {:?}", s);
        assert!(s.contains("2.") && s.contains("SACTrainingAgent"), "expected '2. SACTrainingAgent': {:?}", s);
        assert!(s.contains("3.") && s.contains("SACTradingEnvironment"), "expected '3. SACTradingEnvironment': {:?}", s);
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
        assert!(s.contains("1.") && s.contains("First"), "expected '1. First': {:?}", s);
        assert!(s.contains("2.") && s.contains("Second"), "expected '2. Second' after empty line: {:?}", s);
    }
}
