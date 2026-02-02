//! Cell-based history state and domain events.

mod events;
mod state;

pub use events::{parse_unified_diff, DiffHunk, DiffLine, HistoryEvent};
pub use state::{HistoryId, HistoryRecord, HistoryState};
