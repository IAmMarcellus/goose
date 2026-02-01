//! Output sink that writes to TUI state for use with goose-cli session.

use goose_cli::session::OutputSink;
use std::sync::{Arc, Mutex};

use crate::state::TuiState;

/// Sink that forwards output to shared TUI state.
pub struct TuiSink {
    state: Arc<Mutex<TuiState>>,
}

impl TuiSink {
    pub fn new(state: Arc<Mutex<TuiState>>) -> Self {
        Self { state }
    }
}

impl OutputSink for TuiSink {
    fn write_stream_chunk(&self, text: &str) {
        if let Ok(mut s) = self.state.lock() {
            s.append_content(text);
        }
    }

    fn show_thinking(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.show_thinking();
        }
    }

    fn set_thinking_message(&self, msg: &str) {
        if let Ok(mut s) = self.state.lock() {
            s.set_thinking_message(msg);
        }
    }

    fn hide_thinking(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.hide_thinking();
        }
    }

    fn print_stream_start(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.append_content("\nAssistant:\n");
        }
    }

    fn append_content(&self, text: &str) {
        if let Ok(mut s) = self.state.lock() {
            s.append_content(text);
        }
    }
}
