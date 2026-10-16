//! Shared foundation types for eStream: IDs, time, geometry and colors.

pub mod clock;
pub mod color;
pub mod geometry;
pub mod id;
pub mod rational;

pub use clock::{EngineClock, Timestamp};
pub use color::Rgba8;
pub use geometry::{Alignment, BlendMode, BoundsMode, Crop, ItemTransform, Size};
pub use id::{CanvasId, ItemId, SceneId, SourceId};
pub use rational::Rational;
