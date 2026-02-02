//! History domain events: drive inserts/updates to HistoryState.

/// Domain events emitted by the session/output layer and applied to HistoryState.
#[derive(Clone, Debug)]
pub enum HistoryEvent {
    UserMessage {
        text: String,
    },
    AssistantStart,
    StreamChunk {
        text: String,
    },
    StreamEnd,
    ToolRequest {
        id: String,
        tool_name: String,
        args_preview: String,
    },
    ToolResponse {
        id: String,
        tool_name: String,
        result_preview: String,
        is_error: bool,
    },
    ActionRequired {
        id: String,
        message: String,
    },
    ThinkingStart,
    ThinkingUpdate {
        message: String,
    },
    ThinkingEnd,
    DiffShown {
        path: String,
        hunks: Vec<DiffHunk>,
    },
}

#[derive(Clone, Debug)]
pub struct DiffHunk {
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug)]
pub enum DiffLine {
    Context(String),
    Add(String),
    Remove(String),
}

/// Minimal unified-diff parser. Returns (path, hunks) if the text looks like a diff.
pub fn parse_unified_diff(text: &str) -> Option<(String, Vec<DiffHunk>)> {
    let mut path = String::new();
    let mut hunks = Vec::new();
    let mut in_hunk = false;
    let mut current_lines: Vec<DiffLine> = Vec::new();
    let mut old_start = 0u32;
    let mut old_count = 0u32;
    let mut new_start = 0u32;
    let mut new_count = 0u32;

    for line in text.lines() {
        if line.starts_with("--- ") {
            path = line.strip_prefix("--- ").unwrap_or(line).to_string();
            if let Some(stripped) = path.strip_prefix("a/") {
                path = stripped.to_string();
            }
        } else if line.starts_with("+++ ") {
            if path.is_empty() {
                if let Some(p) = line.strip_prefix("+++ ") {
                    path = p
                        .strip_prefix("b/")
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| p.to_string());
                }
            }
        } else if line.starts_with("@@ ") {
            if in_hunk && !current_lines.is_empty() {
                hunks.push(DiffHunk {
                    old_start,
                    old_count,
                    new_start,
                    new_count,
                    lines: std::mem::take(&mut current_lines),
                });
            }
            in_hunk = true;
            let rest = line
                .strip_prefix("@@ ")
                .map(|s| s.trim_end().strip_suffix(" @@").unwrap_or(s.trim_end()));
            if let Some(rest) = rest {
                let parts: Vec<&str> = rest.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let (Some(a), Some(b)) =
                        (parts[0].strip_prefix('-'), parts[1].strip_prefix('+'))
                    {
                        let a: Vec<u32> = a.split(',').filter_map(|s| s.parse().ok()).collect();
                        let b: Vec<u32> = b.split(',').filter_map(|s| s.parse().ok()).collect();
                        old_start = a.first().copied().unwrap_or(0);
                        old_count = a.get(1).copied().unwrap_or(1);
                        new_start = b.first().copied().unwrap_or(0);
                        new_count = b.get(1).copied().unwrap_or(1);
                    }
                }
            }
            current_lines.clear();
        } else if in_hunk {
            if line.starts_with('+') && !line.starts_with("+++") {
                current_lines.push(DiffLine::Add(line.get(1..).unwrap_or_default().to_string()));
            } else if line.starts_with('-') && !line.starts_with("---") {
                current_lines.push(DiffLine::Remove(
                    line.get(1..).unwrap_or_default().to_string(),
                ));
            } else {
                current_lines.push(DiffLine::Context(line.to_string()));
            }
        }
    }
    if in_hunk && !current_lines.is_empty() {
        hunks.push(DiffHunk {
            old_start,
            old_count,
            new_start,
            new_count,
            lines: current_lines,
        });
    }

    if path.is_empty() || hunks.is_empty() {
        None
    } else {
        Some((path, hunks))
    }
}
