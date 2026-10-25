//! The Clutch engine: command bus, undo/redo, render graph, frame clock,
//! runtime sources and the real-time video thread.
//!
//! All project mutations go through [`Engine::execute`] / [`Engine::submit`]
//! as [`Command`]s. Consumers read immutable snapshots ([`Engine::snapshot`])
//! and events ([`Engine::subscribe`]); finished frames are exposed per canvas
//! via [`Engine::canvas_output`].

pub mod audio;
pub mod command;
pub mod diagnostics;
pub mod engine;
pub mod feed;
mod filters;
/// Drift-free frame scheduling (re-exported from stream-core).
pub mod frame_clock {
    pub use estream_core::frame_clock::*;
}
pub mod gpu_export;
pub mod graph;
pub mod history;
pub mod properties;
pub mod source;
pub mod video;
mod watchdog;

pub use command::{CanvasPatch, Change, Command, CommandError, ItemPatch};
pub use engine::{Engine, EngineConfig, EngineError, Event, Origin, Subscription, VideoTap};
pub use feed::VideoFrame;
pub use gpu_export::{ExportPlanes, GpuExport, GpuFrame, PlanesHandle};
pub use properties::{PropKind, Property, PropertySchema, SelectOption};
pub use source::{
    NOT_ON_THIS_SYSTEM, SourceContext, SourceError, SourceFrame, SourceRegistry, SourceStatus,
    SourceType, VideoSource,
};
pub use video::{CanvasFrame, CanvasOutput, SourceInfo, UiPreview, VideoCore};
pub use watchdog::{FrameTimings, Stage, StageTiming};

pub use estream_audio as audio_engine;
pub use estream_core as core;
pub use estream_gpu as gpu;
pub use estream_project as project;
