//! Shared foundation types for Clutch: IDs, time, geometry and colors.

pub mod clock;
pub mod color;
pub mod frame_clock;
pub mod geometry;
pub mod id;
pub mod rational;
pub mod status;

pub use clock::{EngineClock, Timestamp};
pub use color::Rgba8;
pub use geometry::{Alignment, BlendMode, BoundsMode, Crop, ItemTransform, Size};
pub use id::{CanvasId, ItemId, MacroId, SceneId, SourceId};
pub use rational::Rational;
pub use status::SourceStatus;
