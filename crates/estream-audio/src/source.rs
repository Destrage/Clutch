use crate::block::Block;
use estream_core::{SourceStatus, Timestamp};
use serde_json::Value;

/// A producer of audio. Lives on the audio thread: `read` must not block or allocate.
pub trait AudioSource: Send {
    /// Applies new settings (may hand work to a helper thread).
    fn update(&mut self, settings: &Value) -> Result<(), String>;

    /// Fills `block` with the next 10 ms. Returns how many frames were real
    /// audio; the rest of the block is left silent.
    fn read(&mut self, block: &mut Block, now: Timestamp) -> usize;

    fn status(&self) -> SourceStatus {
        SourceStatus::Active
    }
}
