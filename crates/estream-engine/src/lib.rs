//! The eStream engine: command bus, undo/redo, render graph, frame clock,
//! runtime sources and the real-time video thread.
//!
//! All project mutations go through [`Engine::execute`] / [`Engine::submit`]
//! as [`Command`]s. Consumers read immutable snapshots ([`Engine::snapshot`])
//! and events ([`Engine::subscribe`]); finished frames are exposed per canvas
//! via [`Engine::canvas_output`].

pub mod command;
pub mod diagnostics;
pub mod engine;
pub mod frame_clock;
pub mod graph;
pub mod history;
pub mod source;
pub mod video;

pub use command::{CanvasPatch, Change, Command, CommandError, ItemPatch};
pub use engine::{Engine, EngineConfig, EngineError, Event, Origin, Subscription};
pub use source::{SourceRegistry, SourceStatus, VideoSource};
pub use video::{CanvasFrame, CanvasOutput, SourceInfo, VideoCore};

pub use estream_core as core;
pub use estream_gpu as gpu;
pub use estream_project as project;
