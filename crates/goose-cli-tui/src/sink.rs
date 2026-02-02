//! Output sink that writes to TUI state for use with goose-cli session.
//! Pushes history domain events so the TUI can render from cell-based history.

use goose_cli::session::OutputSink;
use std::sync::{Arc, Mutex};

use crate::history::{parse_unified_diff, HistoryEvent};
use crate::state::TuiState;

/// Sink that forwards output to shared TUI state and pushes history events.
pub struct TuiSink {
    state: Arc<Mutex<TuiState>>,
    approval_rx: std::sync::mpsc::Receiver<goose::permission::Permission>,
}

impl TuiSink {
    pub fn new(
        state: Arc<Mutex<TuiState>>,
        approval_rx: std::sync::mpsc::Receiver<goose::permission::Permission>,
    ) -> Self {
        Self { state, approval_rx }
    }
}

impl OutputSink for TuiSink {
    fn write_stream_chunk(&self, text: &str) {
        if let Ok(mut s) = self.state.lock() {
            s.history.apply_event(HistoryEvent::StreamChunk {
                text: text.to_string(),
            });
        }
    }

    fn show_thinking(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.history.apply_event(HistoryEvent::ThinkingStart);
            s.show_thinking();
        }
    }

    fn set_thinking_message(&self, msg: &str) {
        if let Ok(mut s) = self.state.lock() {
            s.history.apply_event(HistoryEvent::ThinkingUpdate {
                message: msg.to_string(),
            });
            s.set_thinking_message(msg);
        }
    }

    fn hide_thinking(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.history.apply_event(HistoryEvent::ThinkingEnd);
            s.hide_thinking();
        }
    }

    fn print_stream_start(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.history.apply_event(HistoryEvent::AssistantStart);
        }
    }

    fn append_content(&self, text: &str) {
        if let Ok(mut s) = self.state.lock() {
            if text.contains("\n@@ ") && text.contains("--- ") {
                if let Some((path, hunks)) = parse_unified_diff(text) {
                    s.history
                        .apply_event(HistoryEvent::DiffShown { path, hunks });
                    return;
                }
            }
            s.history.append_content(text);
        }
    }

    fn end_stream(&self) {
        if let Ok(mut s) = self.state.lock() {
            s.history.apply_event(HistoryEvent::StreamEnd);
        }
    }

    fn push_tool_request(&self, id: &str, tool_name: &str, args_preview: &str) {
        if let Ok(mut s) = self.state.lock() {
            s.history.apply_event(HistoryEvent::ToolRequest {
                id: id.to_string(),
                tool_name: tool_name.to_string(),
                args_preview: args_preview.to_string(),
            });
        }
    }

    fn push_tool_response(&self, id: &str, tool_name: &str, result_preview: &str, is_error: bool) {
        if let Ok(mut s) = self.state.lock() {
            s.history.apply_event(HistoryEvent::ToolResponse {
                id: id.to_string(),
                tool_name: tool_name.to_string(),
                result_preview: result_preview.to_string(),
                is_error,
            });
        }
    }

    fn set_session_status(&self, status: &str) {
        if let Ok(mut s) = self.state.lock() {
            s.session_status = Some(status.to_string());
        }
    }

    fn request_tool_approval(
        &self,
        id: &str,
        tool_name: &str,
        prompt: Option<&str>,
    ) -> Option<goose::permission::Permission> {
        let message = prompt.unwrap_or("Allow this tool call?").to_string();
        {
            let mut s = self.state.lock().ok()?;
            s.overlay = crate::overlay::Overlay::Approval {
                id: id.to_string(),
                tool_name: tool_name.to_string(),
                message,
            };
        }
        let permission = self
            .approval_rx
            .recv()
            .unwrap_or(goose::permission::Permission::Cancel);
        if let Ok(mut s) = self.state.lock() {
            s.overlay = crate::overlay::Overlay::None;
        }
        Some(permission)
    }
}
