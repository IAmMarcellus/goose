//! History state: vector of records with stable IDs, apply_event for domain events.

use std::num::NonZeroU64;

use super::{DiffHunk, HistoryEvent};

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub struct HistoryId(NonZeroU64);

impl HistoryId {
    fn next() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        Self(NonZeroU64::new(n).unwrap_or_else(|| NonZeroU64::new(1).unwrap()))
    }
}

#[derive(Clone, Debug)]
pub enum HistoryRecord {
    UserMessage {
        id: HistoryId,
        text: String,
    },
    AssistantContent {
        id: HistoryId,
        text: String,
        streaming: bool,
    },
    ToolCall {
        id: HistoryId,
        tool_name: String,
        args_preview: String,
    },
    ToolResult {
        id: HistoryId,
        tool_name: String,
        result_preview: String,
        is_error: bool,
    },
    Thinking {
        id: HistoryId,
        message: String,
    },
    ActionRequired {
        id: HistoryId,
        message: String,
    },
    Diff {
        id: HistoryId,
        path: String,
        hunks: Vec<DiffHunk>,
    },
}

impl HistoryRecord {
    pub fn id(&self) -> HistoryId {
        match self {
            HistoryRecord::UserMessage { id, .. } => *id,
            HistoryRecord::AssistantContent { id, .. } => *id,
            HistoryRecord::ToolCall { id, .. } => *id,
            HistoryRecord::ToolResult { id, .. } => *id,
            HistoryRecord::Thinking { id, .. } => *id,
            HistoryRecord::ActionRequired { id, .. } => *id,
            HistoryRecord::Diff { id, .. } => *id,
        }
    }
}

#[derive(Default)]
pub struct HistoryState {
    records: Vec<HistoryRecord>,
}

impl HistoryState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn records(&self) -> &[HistoryRecord] {
        &self.records
    }

    fn close_streaming_assistant(&mut self) {
        if let Some(HistoryRecord::AssistantContent { streaming, .. }) = self.records.last_mut() {
            *streaming = false;
        }
    }

    /// When the session sends full message markdown (e.g. after streaming or for non-streamed message),
    /// finalize the last streaming assistant cell or push a new assistant content cell.
    pub fn append_content(&mut self, text: &str) {
        if let Some(HistoryRecord::AssistantContent {
            text: ref mut t,
            streaming,
            ..
        }) = self.records.last_mut()
        {
            if *streaming {
                t.push_str(text);
                *streaming = false;
                return;
            }
        }
        self.records.push(HistoryRecord::AssistantContent {
            id: HistoryId::next(),
            text: text.to_string(),
            streaming: false,
        });
    }

    pub fn apply_event(&mut self, event: HistoryEvent) {
        match event {
            HistoryEvent::UserMessage { text } => {
                self.close_streaming_assistant();
                self.records.push(HistoryRecord::UserMessage {
                    id: HistoryId::next(),
                    text,
                });
            }
            HistoryEvent::AssistantStart => {
                self.records.push(HistoryRecord::AssistantContent {
                    id: HistoryId::next(),
                    text: String::new(),
                    streaming: true,
                });
            }
            HistoryEvent::StreamChunk { text } => {
                if let Some(HistoryRecord::AssistantContent {
                    text: ref mut t,
                    streaming,
                    ..
                }) = self.records.last_mut()
                {
                    if *streaming {
                        t.push_str(&text);
                    }
                } else {
                    self.records.push(HistoryRecord::AssistantContent {
                        id: HistoryId::next(),
                        text,
                        streaming: true,
                    });
                }
            }
            HistoryEvent::StreamEnd => {
                self.close_streaming_assistant();
            }
            HistoryEvent::ToolRequest {
                id: _,
                tool_name,
                args_preview,
            } => {
                self.close_streaming_assistant();
                self.records.push(HistoryRecord::ToolCall {
                    id: HistoryId::next(),
                    tool_name,
                    args_preview,
                });
            }
            HistoryEvent::ToolResponse {
                id: _,
                tool_name,
                result_preview,
                is_error,
            } => {
                self.close_streaming_assistant();
                self.records.push(HistoryRecord::ToolResult {
                    id: HistoryId::next(),
                    tool_name,
                    result_preview,
                    is_error,
                });
            }
            HistoryEvent::ActionRequired { id: _, message } => {
                self.records.push(HistoryRecord::ActionRequired {
                    id: HistoryId::next(),
                    message,
                });
            }
            HistoryEvent::ThinkingStart => {
                self.close_streaming_assistant();
                self.records.push(HistoryRecord::Thinking {
                    id: HistoryId::next(),
                    message: String::new(),
                });
            }
            HistoryEvent::ThinkingUpdate { message } => {
                if let Some(HistoryRecord::Thinking {
                    message: ref mut m, ..
                }) = self.records.last_mut()
                {
                    *m = message;
                } else {
                    self.records.push(HistoryRecord::Thinking {
                        id: HistoryId::next(),
                        message,
                    });
                }
            }
            HistoryEvent::ThinkingEnd => {
                if let Some(HistoryRecord::Thinking { .. }) = self.records.last_mut() {
                    // Keep the record; thinking ended (could collapse or leave as-is)
                }
            }
            HistoryEvent::DiffShown { path, hunks } => {
                self.close_streaming_assistant();
                self.records.push(HistoryRecord::Diff {
                    id: HistoryId::next(),
                    path,
                    hunks,
                });
            }
        }
    }
}
