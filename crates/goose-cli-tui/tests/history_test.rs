//! Tests for history events, apply_event, and parse_unified_diff.

use goose_cli_tui::history::{
    parse_unified_diff, DiffLine, HistoryEvent, HistoryRecord, HistoryState,
};

#[test]
fn apply_user_message_appends_record() {
    let mut state = HistoryState::new();
    state.apply_event(HistoryEvent::UserMessage {
        text: "hello".to_string(),
    });
    let records = state.records();
    assert_eq!(records.len(), 1);
    match &records[0] {
        HistoryRecord::UserMessage { text, .. } => assert_eq!(text, "hello"),
        _ => panic!("expected UserMessage"),
    }
}

#[test]
fn apply_assistant_start_then_stream_chunk_appends_one_cell() {
    let mut state = HistoryState::new();
    state.apply_event(HistoryEvent::AssistantStart);
    state.apply_event(HistoryEvent::StreamChunk {
        text: "hi".to_string(),
    });
    state.apply_event(HistoryEvent::StreamChunk {
        text: " there".to_string(),
    });
    state.apply_event(HistoryEvent::StreamEnd);
    let records = state.records();
    assert_eq!(records.len(), 1);
    match &records[0] {
        HistoryRecord::AssistantContent {
            text, streaming, ..
        } => {
            assert_eq!(text, "hi there");
            assert!(!streaming);
        }
        _ => panic!("expected AssistantContent"),
    }
}

#[test]
fn apply_tool_request_and_response_appends_two_cells() {
    let mut state = HistoryState::new();
    state.apply_event(HistoryEvent::ToolRequest {
        id: "id1".to_string(),
        tool_name: "read_file".to_string(),
        args_preview: "{}".to_string(),
    });
    state.apply_event(HistoryEvent::ToolResponse {
        id: "id1".to_string(),
        tool_name: "read_file".to_string(),
        result_preview: "ok".to_string(),
        is_error: false,
    });
    let records = state.records();
    assert_eq!(records.len(), 2);
    match &records[0] {
        HistoryRecord::ToolCall {
            tool_name,
            args_preview,
            ..
        } => {
            assert_eq!(tool_name, "read_file");
            assert_eq!(args_preview, "{}");
        }
        _ => panic!("expected ToolCall"),
    }
    match &records[1] {
        HistoryRecord::ToolResult {
            tool_name,
            result_preview,
            is_error,
            ..
        } => {
            assert_eq!(tool_name, "read_file");
            assert_eq!(result_preview, "ok");
            assert!(!is_error);
        }
        _ => panic!("expected ToolResult"),
    }
}

#[test]
fn apply_user_message_closes_streaming_assistant() {
    let mut state = HistoryState::new();
    state.apply_event(HistoryEvent::AssistantStart);
    state.apply_event(HistoryEvent::StreamChunk {
        text: "partial".to_string(),
    });
    state.apply_event(HistoryEvent::UserMessage {
        text: "next".to_string(),
    });
    let records = state.records();
    assert_eq!(records.len(), 2);
    match &records[0] {
        HistoryRecord::AssistantContent { text, .. } => assert_eq!(text, "partial"),
        _ => panic!("expected AssistantContent"),
    }
    match &records[1] {
        HistoryRecord::UserMessage { text, .. } => assert_eq!(text, "next"),
        _ => panic!("expected UserMessage"),
    }
}

#[test]
fn apply_diff_shown_appends_diff_record() {
    let mut state = HistoryState::new();
    state.apply_event(HistoryEvent::DiffShown {
        path: "foo.rs".to_string(),
        hunks: vec![],
    });
    let records = state.records();
    assert_eq!(records.len(), 1);
    match &records[0] {
        HistoryRecord::Diff { path, hunks, .. } => {
            assert_eq!(path, "foo.rs");
            assert!(hunks.is_empty());
        }
        _ => panic!("expected Diff"),
    }
}

#[test]
fn parse_unified_diff_returns_path_and_hunks() {
    let diff = "--- a/src/foo.rs\n+++ b/src/foo.rs\n@@ -1,3 +1,4 @@\n context\n-removed\n+added\n";
    let parsed = parse_unified_diff(diff).expect("should parse");
    assert_eq!(parsed.0, "src/foo.rs");
    assert_eq!(parsed.1.len(), 1);
    let hunk = &parsed.1[0];
    assert_eq!(hunk.old_start, 1);
    assert_eq!(hunk.old_count, 3);
    assert_eq!(hunk.new_start, 1);
    assert_eq!(hunk.new_count, 4);
    assert_eq!(hunk.lines.len(), 3);
    match &hunk.lines[0] {
        DiffLine::Context(s) => assert_eq!(s, " context"),
        _ => panic!("expected Context"),
    }
    match &hunk.lines[1] {
        DiffLine::Remove(s) => assert_eq!(s, "removed"),
        _ => panic!("expected Remove"),
    }
    match &hunk.lines[2] {
        DiffLine::Add(s) => assert_eq!(s, "added"),
        _ => panic!("expected Add"),
    }
}

#[test]
fn parse_unified_diff_accepts_hunk_without_trailing_at_at() {
    let diff = "--- a/bar.rs\n+++ b/bar.rs\n@@ -2,1 +2,2\n old\n+new\n";
    let parsed = parse_unified_diff(diff).expect("should parse");
    assert_eq!(parsed.0, "bar.rs");
    assert_eq!(parsed.1.len(), 1);
    let hunk = &parsed.1[0];
    assert_eq!(hunk.old_start, 2);
    assert_eq!(hunk.old_count, 1);
    assert_eq!(hunk.new_start, 2);
    assert_eq!(hunk.new_count, 2);
}
