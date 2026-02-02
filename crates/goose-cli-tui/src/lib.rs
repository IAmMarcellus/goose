//! goose-cli-tui: ratatui-based TUI for goose CLI streaming.
//!
//! Cell-based history (user, assistant, tool call/result, thinking, diff), content area with
//! scroll, thinking line, and input. Overlays: theme picker (Ctrl+T), settings (Ctrl+S),
//! tool approval (Y/N/Esc when confirmation is shown).

pub mod app;
pub mod content_layout;
pub mod content_render;
pub mod history;
pub mod overlay;
pub mod sink;
pub mod state;
pub mod terminal;
