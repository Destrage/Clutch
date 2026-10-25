//! Clutch audio engine: 48 kHz stereo `f32` mixing in 10 ms blocks, with
//! per-source filters, meters, monitoring and six output tracks.
//!
//! Real-time rules: [`AudioCore::process`] never allocates or blocks; device
//! callbacks talk to it only through lock-free SPSC rings.

pub mod block;
pub mod convert;
pub mod db;
pub mod devices;
pub mod filters;
pub mod meter;
pub mod mixer;
#[cfg(target_os = "linux")]
pub mod pipewire;
pub mod ring;
pub mod source;
mod strong;
pub mod tone;
pub mod voice;

pub use block::Block;
pub use meter::{Meter, MeterReading};
pub use mixer::{AudioCore, AudioGraph, AudioNode, SourceFactory, TrackSink};
pub use source::AudioSource;

/// Internal sample rate.
pub const SAMPLE_RATE: u32 = 48_000;
/// Frames per block (10 ms at 48 kHz).
pub const BLOCK: usize = 480;
/// Number of output tracks.
pub const TRACKS: usize = 6;
