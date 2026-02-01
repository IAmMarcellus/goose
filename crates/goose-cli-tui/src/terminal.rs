//! Terminal init and restore for TUI (alternate screen, raw mode).

use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use std::io;

/// Enter alternate screen and enable raw mode for TUI.
pub fn init() -> io::Result<()> {
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(io::stdout(), EnterAlternateScreen)?;
    Ok(())
}

/// Leave alternate screen and disable raw mode.
pub fn restore() -> io::Result<()> {
    let _ = crossterm::terminal::disable_raw_mode();
    let _ = crossterm::execute!(io::stdout(), LeaveAlternateScreen);
    Ok(())
}
