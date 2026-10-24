//! Game integrations: official local event APIs that report kills, wins and objectives, and
//! replay files that games write to the local disk.

pub mod gsi;
pub mod league;
pub mod replay;
pub mod steam;

use crate::event::EventKind;
use std::time::Instant;

/// An event reported by a game, stamped when it arrived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameEvent {
    pub game: String,
    pub kind: EventKind,
    pub at: Instant,
}
