//! Tests for Gemini CLI provider streaming (--output-format stream-json).
//!
//! Uses a script that echoes NDJSON lines to avoid requiring the real gemini CLI in CI.

#![cfg(unix)]

use anyhow::Result;
use futures::StreamExt;
use goose::conversation::message::Message;
use goose::model::ModelConfig;
use goose::providers::base::Provider;
use goose::providers::gemini_cli::{GeminiCliProvider, GEMINI_CLI_DEFAULT_MODEL};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;

fn write_echo_ndjson_script(dir: &TempDir) -> Result<std::path::PathBuf> {
    let script_path = dir.path().join("echo_ndjson");
    let mut f = std::fs::File::create(&script_path)?;
    writeln!(f, "#!/bin/sh")?;
    const LINES: &[&str] = &[
        "{\"text\":\"Hello\"}",
        "{\"text\":\" \"}",
        "{\"text\":\"world\"}",
        "{\"stats\":{\"models\":{\"gemini-2.5-pro\":{\"tokens\":{\"prompt\":1,\"candidates\":2,\"total\":3}}}}}",
    ];
    for line in LINES {
        writeln!(f, "echo '{}'", line)?;
    }
    writeln!(f, "exec /bin/true")?;
    f.sync_all()?;
    drop(f);
    let mut perms = std::fs::metadata(&script_path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&script_path, perms)?;
    Ok(script_path)
}

#[tokio::test]
#[ignore = "requires script in PATH to exit 0; run with --ignored when testing locally"]
async fn test_gemini_cli_streaming_yields_multiple_events() -> Result<()> {
    let dir = TempDir::new()?;
    let _script_path = write_echo_ndjson_script(&dir)?;
    let script_name = "echo_ndjson";
    let dir_path = dir.path().to_string_lossy().to_string();
    let existing_path = std::env::var_os("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", dir_path, existing_path.to_string_lossy());

    let _guard = env_lock::lock_env([
        ("GEMINI_CLI_COMMAND", Some(script_name)),
        ("PATH", Some(new_path.as_str())),
    ]);

    let model = ModelConfig::new(GEMINI_CLI_DEFAULT_MODEL)?;
    let provider = GeminiCliProvider::from_env(model).await?;

    assert!(provider.supports_streaming());

    let messages = vec![Message::user().with_text("Say hello")];
    let mut stream = provider
        .stream("test-session", "You are helpful.", &messages, &[])
        .await?;

    let mut message_count = 0;
    let mut text_parts: Vec<String> = Vec::new();
    let mut last_usage = None;

    while let Some(result) = stream.next().await {
        let (message, usage) = result?;
        if let Some(msg) = message {
            message_count += 1;
            let t = msg.as_concat_text();
            if !t.is_empty() {
                text_parts.push(t);
            }
        }
        if usage.is_some() {
            last_usage = usage;
        }
    }

    assert!(
        message_count >= 2,
        "expected at least 2 message events, got {}",
        message_count
    );
    let full_text = text_parts.join("");
    assert!(
        full_text.contains("Hello"),
        "streamed text should contain 'Hello', got: {:?}",
        full_text
    );
    assert!(
        full_text.contains("world"),
        "streamed text should contain 'world', got: {:?}",
        full_text
    );
    assert!(
        last_usage.is_some(),
        "expected a final usage event with stats"
    );

    Ok(())
}

#[tokio::test]
#[ignore = "requires script in PATH to exit 0; run with --ignored when testing locally"]
async fn test_gemini_cli_streaming_skips_malformed_lines() -> Result<()> {
    let dir = TempDir::new()?;
    let script_path = dir.path().join("echo_mixed");
    let mut f = std::fs::File::create(&script_path)?;
    writeln!(f, "#!/bin/sh")?;
    writeln!(f, "echo 'not json'")?;
    writeln!(f, "echo '{{\"text\":\"ok\"}}'")?;
    writeln!(f, "echo '{{\"stats\":{{}}}}'")?;
    writeln!(f, "exec /bin/true")?;
    f.sync_all()?;
    drop(f);
    let mut perms = std::fs::metadata(&script_path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&script_path, perms)?;

    let dir_path = dir.path().to_string_lossy().to_string();
    let existing_path = std::env::var_os("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", dir_path, existing_path.to_string_lossy());

    let _guard = env_lock::lock_env([
        ("GEMINI_CLI_COMMAND", Some("echo_mixed")),
        ("PATH", Some(new_path.as_str())),
    ]);

    let model = ModelConfig::new(GEMINI_CLI_DEFAULT_MODEL)?;
    let provider = GeminiCliProvider::from_env(model).await?;

    let messages = vec![Message::user().with_text("Hi")];
    let mut stream = provider.stream("test-session", "", &messages, &[]).await?;

    let mut got_message = false;
    while let Some(result) = stream.next().await {
        let (message, _) = result?;
        if let Some(msg) = message {
            if msg.as_concat_text() == "ok" {
                got_message = true;
            }
        }
    }

    assert!(
        got_message,
        "should receive the valid JSON message despite malformed line"
    );

    Ok(())
}

#[tokio::test]
async fn test_gemini_cli_streaming_session_name_uses_single_message() -> Result<()> {
    let dir = TempDir::new()?;
    let _script_path = write_echo_ndjson_script(&dir)?;
    let dir_path = dir.path().to_string_lossy().to_string();
    let existing_path = std::env::var_os("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", dir_path, existing_path.to_string_lossy());

    let _guard = env_lock::lock_env([
        ("GEMINI_CLI_COMMAND", Some("echo_ndjson")),
        ("PATH", Some(new_path.as_str())),
    ]);

    let model = ModelConfig::new(GEMINI_CLI_DEFAULT_MODEL)?;
    let provider = GeminiCliProvider::from_env(model).await?;

    let messages = vec![Message::user().with_text("First user message here")];
    let system = "Reply with only a description in four words or less";
    let mut stream = provider
        .stream("test-session", system, &messages, &[])
        .await?;

    let mut count = 0;
    while let Some(result) = stream.next().await {
        let _ = result?;
        count += 1;
    }

    assert_eq!(
        count, 1,
        "session-name shortcut should yield exactly one event"
    );

    Ok(())
}
