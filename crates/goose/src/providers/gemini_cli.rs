use anyhow::Result;
use async_stream::try_stream;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;

use super::base::{
    stream_from_single_message, MessageStream, Provider, ProviderMetadata, ProviderUsage, Usage,
};
use super::errors::ProviderError;
use super::utils::{filter_extensions_from_system_prompt, RequestLog};
use crate::config::base::GeminiCliCommand;
use crate::config::search_path::SearchPaths;
use crate::config::Config;
use crate::conversation::message::{Message, MessageContent};
use crate::model::ModelConfig;
use crate::providers::base::ConfigKey;
use crate::subprocess::configure_command_no_window;
use rmcp::model::Role;
use rmcp::model::Tool;

pub const GEMINI_CLI_DEFAULT_MODEL: &str = "gemini-2.5-pro";
pub const GEMINI_CLI_KNOWN_MODELS: &[&str] = &[
    "gemini-2.5-pro",
    "gemini-2.5-flash",
    "gemini-2.5-flash-lite",
];

pub const GEMINI_CLI_DOC_URL: &str = "https://ai.google.dev/gemini-api/docs";

#[derive(Debug, serde::Serialize)]
pub struct GeminiCliProvider {
    command: PathBuf,
    model: ModelConfig,
    #[serde(skip)]
    name: String,
}

impl GeminiCliProvider {
    pub async fn from_env(model: ModelConfig) -> Result<Self> {
        let config = Config::global();
        let command: OsString = config.get_gemini_cli_command().unwrap_or_default().into();
        let resolved_command = SearchPaths::builder().with_npm().resolve(command)?;

        Ok(Self {
            command: resolved_command,
            model,
            name: Self::metadata().name,
        })
    }

    /// Build prompt string from system and messages (shared by execute_command and streaming).
    fn build_prompt(&self, system: &str, messages: &[Message]) -> String {
        let mut full_prompt = String::new();
        let filtered_system = filter_extensions_from_system_prompt(system);
        full_prompt.push_str(&filtered_system);
        full_prompt.push_str("\n\n");
        for message in messages.iter().filter(|m| m.is_agent_visible()) {
            let role_prefix = match message.role {
                Role::User => "Human: ",
                Role::Assistant => "Assistant: ",
            };
            full_prompt.push_str(role_prefix);
            for content in &message.content {
                if let MessageContent::Text(text_content) = content {
                    full_prompt.push_str(&text_content.text);
                    full_prompt.push('\n');
                }
            }
            full_prompt.push('\n');
        }
        full_prompt.push_str("Assistant: ");
        full_prompt
    }

    /// Extract Usage from stats.models.<model>.tokens in stream-json or final JSON.
    fn usage_from_value(value: &Value, model_name: &str) -> Usage {
        let models = value
            .get("stats")
            .and_then(|s| s.get("models"))
            .and_then(|m| m.as_object());
        let Some(models) = models else {
            return Usage::default();
        };
        let tokens = models
            .get(model_name)
            .or_else(|| models.values().next())
            .and_then(|m| m.get("tokens"));
        let Some(t) = tokens else {
            return Usage::default();
        };
        let input_tokens = t.get("prompt").and_then(|v| v.as_u64()).map(|v| v as i32);
        let output_tokens = t
            .get("candidates")
            .and_then(|v| v.as_u64())
            .map(|v| v as i32);
        let total_tokens = t
            .get("total")
            .and_then(|v| v.as_u64())
            .map(|v| v as i32)
            .or_else(|| input_tokens.and_then(|i| output_tokens.map(|o| i + o)));
        Usage::new(input_tokens, output_tokens, total_tokens)
    }

    /// Execute gemini CLI command with simple text prompt
    async fn execute_command(
        &self,
        system: &str,
        messages: &[Message],
        _tools: &[Tool],
    ) -> Result<Vec<String>, ProviderError> {
        let full_prompt = self.build_prompt(system, messages);

        if std::env::var("GOOSE_GEMINI_CLI_DEBUG").is_ok() {
            println!("=== GEMINI CLI PROVIDER DEBUG ===");
            println!("Command: {:?}", self.command);
            println!("Full prompt: {}", full_prompt);
            println!("================================");
        }

        let mut cmd = Command::new(&self.command);
        configure_command_no_window(&mut cmd);

        if let Ok(path) = SearchPaths::builder().with_npm().path() {
            cmd.env("PATH", path);
        }

        // Only pass model parameter if it's in the known models list
        if GEMINI_CLI_KNOWN_MODELS.contains(&self.model.model_name.as_str()) {
            cmd.arg("-m").arg(&self.model.model_name);
        }

        if cfg!(windows) {
            let sanitized_prompt = full_prompt.replace("\r\n", "\\n").replace('\n', "\\n");

            cmd.arg("-p").arg(&sanitized_prompt).arg("--yolo");
        } else {
            cmd.arg("-p").arg(&full_prompt).arg("--yolo");
        }

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            ProviderError::RequestFailed(format!(
                "Failed to spawn Gemini CLI command '{:?}': {}. \
                Make sure the Gemini CLI is installed and available in the configured search paths.",
                self.command, e
            ))
        })?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ProviderError::RequestFailed("Failed to capture stdout".to_string()))?;

        let mut reader = BufReader::new(stdout);
        let mut lines = Vec::new();
        let mut line = String::new();

        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => break, // EOF
                Ok(_) => {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() && !trimmed.starts_with("Loaded cached credentials") {
                        lines.push(trimmed.to_string());
                    }
                }
                Err(e) => {
                    return Err(ProviderError::RequestFailed(format!(
                        "Failed to read output: {}",
                        e
                    )));
                }
            }
        }

        let exit_status = child.wait().await.map_err(|e| {
            ProviderError::RequestFailed(format!("Failed to wait for command: {}", e))
        })?;

        if !exit_status.success() {
            return Err(ProviderError::RequestFailed(format!(
                "Command failed with exit code: {:?}",
                exit_status.code()
            )));
        }

        tracing::debug!(
            "Gemini CLI executed successfully, got {} lines",
            lines.len()
        );

        Ok(lines)
    }

    /// Parse simple text response
    fn parse_response(&self, lines: &[String]) -> Result<(Message, Usage), ProviderError> {
        // Join all lines into a single response
        let response_text = lines.join("\n");

        if response_text.trim().is_empty() {
            return Err(ProviderError::RequestFailed(
                "Empty response from gemini command".to_string(),
            ));
        }

        let message = Message::new(
            Role::Assistant,
            chrono::Utc::now().timestamp(),
            vec![MessageContent::text(response_text)],
        );

        let usage = Usage::default(); // No usage info available for gemini CLI

        Ok((message, usage))
    }

    /// Generate a simple session description without calling subprocess
    fn generate_simple_session_description(
        &self,
        messages: &[Message],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        // Extract the first user message text
        let description = messages
            .iter()
            .find(|m| m.role == Role::User)
            .and_then(|m| {
                m.content.iter().find_map(|c| match c {
                    MessageContent::Text(text_content) => Some(&text_content.text),
                    _ => None,
                })
            })
            .map(|text| {
                // Take first few words, limit to 4 words
                text.split_whitespace()
                    .take(4)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_else(|| "Simple task".to_string());

        if std::env::var("GOOSE_GEMINI_CLI_DEBUG").is_ok() {
            println!("=== GEMINI CLI PROVIDER DEBUG ===");
            println!("Generated simple session description: {}", description);
            println!("Skipped subprocess call for session description");
            println!("================================");
        }

        let message = Message::new(
            Role::Assistant,
            chrono::Utc::now().timestamp(),
            vec![MessageContent::text(description.clone())],
        );

        let usage = Usage::default();

        Ok((
            message,
            ProviderUsage::new(self.model.model_name.clone(), usage),
        ))
    }
}

#[async_trait]
impl Provider for GeminiCliProvider {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::new(
            "gemini-cli",
            "Gemini CLI",
            "Execute Gemini models via gemini CLI tool",
            GEMINI_CLI_DEFAULT_MODEL,
            GEMINI_CLI_KNOWN_MODELS.to_vec(),
            GEMINI_CLI_DOC_URL,
            vec![ConfigKey::from_value_type::<GeminiCliCommand>(true, false)],
        )
    }

    fn get_name(&self) -> &str {
        &self.name
    }

    fn get_model_config(&self) -> ModelConfig {
        // Return the model config with appropriate context limit for Gemini models
        self.model.clone()
    }

    #[tracing::instrument(
        skip(self, _model_config, system, messages, tools),
        fields(model_config, input, output, input_tokens, output_tokens, total_tokens)
    )]
    async fn complete_with_model(
        &self,
        _session_id: Option<&str>, // CLI has no external session-id flag to propagate.
        _model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        // Check if this is a session description request (short system prompt asking for 4 words or less)
        if system.contains("four words or less") || system.contains("4 words or less") {
            return self.generate_simple_session_description(messages);
        }

        // Create a dummy payload for debug tracing
        let payload = json!({
            "command": self.command,
            "model": self.model.model_name,
            "system": system,
            "messages": messages.len()
        });

        let mut log = RequestLog::start(&self.model, &payload).map_err(|e| {
            ProviderError::RequestFailed(format!("Failed to start request log: {}", e))
        })?;

        let lines = self.execute_command(system, messages, tools).await?;

        let (message, usage) = self.parse_response(&lines)?;

        let response = json!({
            "lines": lines.len(),
            "usage": usage
        });

        log.write(&response, Some(&usage)).map_err(|e| {
            ProviderError::RequestFailed(format!("Failed to write request log: {}", e))
        })?;

        Ok((
            message,
            ProviderUsage::new(self.model.model_name.clone(), usage),
        ))
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    async fn stream(
        &self,
        _session_id: &str,
        system: &str,
        messages: &[Message],
        _tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        if system.contains("four words or less") || system.contains("4 words or less") {
            let (message, usage) = self.generate_simple_session_description(messages)?;
            return Ok(stream_from_single_message(message, usage));
        }

        let full_prompt = self.build_prompt(system, messages);
        let command = self.command.clone();
        let model_name = self.model.model_name.clone();

        let mut cmd = Command::new(&command);
        configure_command_no_window(&mut cmd);
        if let Ok(path) = SearchPaths::builder().with_npm().path() {
            cmd.env("PATH", path);
        }
        if GEMINI_CLI_KNOWN_MODELS.contains(&model_name.as_str()) {
            cmd.arg("-m").arg(&model_name);
        }
        if cfg!(windows) {
            let sanitized_prompt = full_prompt.replace("\r\n", "\\n").replace('\n', "\\n");
            cmd.arg("-p")
                .arg(&sanitized_prompt)
                .arg("--yolo")
                .arg("--output-format")
                .arg("stream-json");
        } else {
            cmd.arg("-p")
                .arg(&full_prompt)
                .arg("--yolo")
                .arg("--output-format")
                .arg("stream-json");
        }
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            ProviderError::RequestFailed(format!(
                "Failed to spawn Gemini CLI command '{:?}': {}. \
                Make sure the Gemini CLI is installed and available in the configured search paths.",
                command, e
            ))
        })?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ProviderError::RequestFailed("Failed to capture stdout".to_string()))?;

        let stderr_handle = child.stderr.take();
        let mut child_handle = child;

        Ok(Box::pin(try_stream! {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            let mut yielded_any_message = false;

            if let Some(stderr) = stderr_handle {
                tokio::spawn(async move {
                    let mut reader = BufReader::new(stderr);
                    let mut buf = String::new();
                    if reader.read_to_string(&mut buf).await.is_ok() && !buf.trim().is_empty() {
                        tracing::debug!("Gemini CLI stderr: {}", buf.trim());
                    }
                });
            }

            loop {
                line.clear();
                let n = reader.read_line(&mut line).await.map_err(|e| {
                    ProviderError::RequestFailed(format!("Failed to read stream output: {}", e))
                })?;
                if n == 0 {
                    break;
                }
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with("Loaded cached credentials") {
                    continue;
                }
                let value: Value = match serde_json::from_str(trimmed) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::trace!("Gemini CLI stream: skip malformed JSON line: {}", e);
                        continue;
                    }
                };
                if let Some(err) = value.get("error") {
                    let msg = err
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("Unknown Gemini CLI error");
                    Err(ProviderError::RequestFailed(msg.to_string()))?;
                }
                if let Some(text) = value
                    .get("text")
                    .and_then(|t| t.as_str())
                    .or_else(|| value.get("response").and_then(|r| r.as_str()))
                {
                    if !text.is_empty() {
                        let message = Message::new(
                            Role::Assistant,
                            chrono::Utc::now().timestamp(),
                            vec![MessageContent::text(text.to_string())],
                        );
                        yielded_any_message = true;
                        yield (Some(message), None);
                    }
                }
                if value.get("stats").is_some() {
                    let usage = GeminiCliProvider::usage_from_value(&value, &model_name);
                    yield (None, Some(ProviderUsage::new(model_name.clone(), usage)));
                }
            }

            let exit_status = child_handle.wait().await.map_err(|e| {
                ProviderError::RequestFailed(format!("Failed to wait for Gemini CLI: {}", e))
            })?;
            if !exit_status.success() {
                Err(ProviderError::RequestFailed(format!(
                    "Gemini CLI exited with code: {:?}",
                    exit_status.code()
                )))?;
            }
            if !yielded_any_message {
                let empty = Message::new(
                    Role::Assistant,
                    chrono::Utc::now().timestamp(),
                    vec![MessageContent::text(String::new())],
                );
                yield (Some(empty), None);
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_usage_from_value_parses_stats() {
        let value = json!({
            "stats": {
                "models": {
                    "gemini-2.5-pro": {
                        "tokens": {
                            "prompt": 10,
                            "candidates": 20,
                            "total": 30
                        }
                    }
                }
            }
        });
        let usage = GeminiCliProvider::usage_from_value(&value, "gemini-2.5-pro");
        assert_eq!(usage.input_tokens, Some(10));
        assert_eq!(usage.output_tokens, Some(20));
        assert_eq!(usage.total_tokens, Some(30));
    }

    #[test]
    fn test_usage_from_value_missing_stats_returns_default() {
        let value = json!({"response": "hello"});
        let usage = GeminiCliProvider::usage_from_value(&value, "gemini-2.5-pro");
        assert_eq!(usage.input_tokens, None);
        assert_eq!(usage.output_tokens, None);
        assert_eq!(usage.total_tokens, None);
    }
}
