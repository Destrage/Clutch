//! Clipping for eStream (see the `add-clips` OpenSpec change).
//!
//! Everything here is plain logic or I/O off the real-time threads: the app feeds in game
//! events, audio levels and hotkey presses, and asks the replay buffer to save the windows
//! the [`Scheduler`] hands back.

pub mod event;
pub mod games;
pub mod hype;
pub mod integrations;
pub mod library;
pub mod markers;
pub mod platform;
pub mod scheduler;

pub use event::{ClipEvent, EventKind};
pub use games::{Detection, Detector, Game, GameList, Integration, UserGame};
pub use hype::{HypeConfig, HypeDetector, Peak, Sensitivity};
pub use library::{Clip, ClipMark, ClipMeta, Library, Trigger};
pub use scheduler::{ClipRequest, Scheduler, Window};
