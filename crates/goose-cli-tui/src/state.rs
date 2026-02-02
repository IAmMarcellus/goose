//! Shared TUI state for content area, thinking line, input, and overlays.

use crate::content_render::Theme;
use crate::history::HistoryState;
use crate::overlay::Overlay;
use std::sync::mpsc;
use std::time::Instant;

pub struct TuiState {
    pub history: HistoryState,
    pub thinking_message: String,
    pub thinking_visible: bool,
    /// When thinking started; used to animate "Thinking..." dots.
    pub thinking_started_at: Option<Instant>,
    pub scroll_offset: usize,
    /// When true, view follows the bottom while assistant is streaming; cleared on scroll up, restored on End or send.
    pub scroll_follows_stream: bool,
    pub input_buffer: String,
    pub overlay: Overlay,
    pub theme: Theme,
    /// Session status line shown in Status pane when idle (e.g. "provider: X | model: Y").
    pub session_status: Option<String>,
    /// When Approval overlay is shown, TUI sends user's choice here for the session thread.
    pub approval_response_tx: Option<mpsc::Sender<goose::permission::Permission>>,
}

impl Default for TuiState {
    fn default() -> Self {
        Self {
            history: HistoryState::default(),
            thinking_message: String::new(),
            thinking_visible: false,
            thinking_started_at: None,
            scroll_offset: 0,
            scroll_follows_stream: true,
            input_buffer: String::new(),
            overlay: Overlay::None,
            theme: Theme::default(),
            session_status: None,
            approval_response_tx: None,
        }
    }
}

impl TuiState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn show_thinking(&mut self) {
        self.thinking_visible = true;
        self.thinking_started_at = Some(Instant::now());
    }

    pub fn set_thinking_message(&mut self, msg: &str) {
        self.thinking_message = msg.to_string();
    }

    pub fn hide_thinking(&mut self) {
        self.thinking_visible = false;
        self.thinking_started_at = None;
    }
}
