//! goose-cli-tui: ratatui-based TUI for goose CLI streaming.
//!
//! Provides a separate binary `goose-tui` with streamed text in a content area
//! and thinking indicator in a fixed status line (no interleaving).

pub mod app;
pub mod sink;
pub mod state;
pub mod terminal;
