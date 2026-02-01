//! Shared TUI state for content area, thinking line, and input.

#[derive(Default)]
pub struct TuiState {
    pub content: String,
    pub thinking_message: String,
    pub thinking_visible: bool,
    pub scroll_offset: u16,
    pub input_buffer: String,
}

impl TuiState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn append_content(&mut self, text: &str) {
        self.content.push_str(text);
    }

    pub fn show_thinking(&mut self) {
        self.thinking_visible = true;
    }

    pub fn set_thinking_message(&mut self, msg: &str) {
        self.thinking_message = msg.to_string();
    }

    pub fn hide_thinking(&mut self) {
        self.thinking_visible = false;
    }

    pub fn content_lines(&self) -> Vec<&str> {
        self.content.lines().collect()
    }
}
