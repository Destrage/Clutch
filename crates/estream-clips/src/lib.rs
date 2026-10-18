//! Clipping for eStream (see the `add-clips` OpenSpec change).
//!
//! Everything here is plain logic or I/O off the real-time threads: the app feeds in game
//! events, audio levels and hotkey presses, and asks the replay buffer to save the windows
//! the [`Scheduler`] hands back.

pub mod event;
pub mod scheduler;

pub use event::{ClipEvent, EventKind};
pub use scheduler::{ClipRequest, Scheduler, Window};
