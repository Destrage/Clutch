//! Lock-free SPSC rings of interleaved stereo frames at 48 kHz.

use crate::BLOCK;
use crate::block::Block;
use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Writer side (device callback).
pub struct StereoProducer {
    inner: Producer<f32>,
    /// Frames dropped because the ring was full.
    pub overflows: Arc<AtomicU64>,
}

/// Reader side (audio thread).
pub struct StereoConsumer {
    inner: Consumer<f32>,
    pub overflows: Arc<AtomicU64>,
}

pub fn stereo_ring(capacity_frames: usize) -> (StereoProducer, StereoConsumer) {
    let (p, c) = RingBuffer::new(capacity_frames * 2);
    let overflows = Arc::new(AtomicU64::new(0));
    (
        StereoProducer {
            inner: p,
            overflows: overflows.clone(),
        },
        StereoConsumer {
            inner: c,
            overflows,
        },
    )
}

impl StereoProducer {
    /// Pushes interleaved stereo samples. Frames that don't fit are dropped.
    pub fn push(&mut self, interleaved: &[f32]) {
        let frames = interleaved.len() / 2;
        let free = self.inner.slots() / 2;
        let n = frames.min(free);
        if n < frames {
            self.overflows
                .fetch_add((frames - n) as u64, Ordering::Relaxed);
        }
        if let Ok(chunk) = self.inner.write_chunk_uninit(n * 2) {
            chunk.fill_from_iter(interleaved[..n * 2].iter().copied());
        }
    }

    /// Pushes one planar block.
    pub fn push_block(&mut self, block: &Block) {
        let n = BLOCK.min(self.inner.slots() / 2);
        if n < BLOCK {
            self.overflows
                .fetch_add((BLOCK - n) as u64, Ordering::Relaxed);
        }
        if let Ok(chunk) = self.inner.write_chunk_uninit(n * 2) {
            chunk.fill_from_iter((0..n).flat_map(|i| [block.l[i], block.r[i]]));
        }
    }
}

impl StereoConsumer {
    pub fn available(&self) -> usize {
        self.inner.slots() / 2
    }

    /// Drops the oldest `frames` frames.
    pub fn skip(&mut self, frames: usize) {
        let n = (frames * 2).min(self.inner.slots());
        if let Ok(chunk) = self.inner.read_chunk(n) {
            chunk.commit_all();
        }
    }

    /// Fills `block` from the ring; returns frames read (the rest is left as-is).
    pub fn read_block(&mut self, block: &mut Block) -> usize {
        let n = BLOCK.min(self.available());
        if let Ok(chunk) = self.inner.read_chunk(n * 2) {
            let (a, b) = chunk.as_slices();
            for (i, s) in a.iter().chain(b).enumerate() {
                if i % 2 == 0 {
                    block.l[i / 2] = *s;
                } else {
                    block.r[i / 2] = *s;
                }
            }
            chunk.commit_all();
        }
        n
    }

    /// Reads up to `out.len() / 2` frames into an interleaved buffer.
    pub fn read_interleaved(&mut self, out: &mut [f32]) -> usize {
        let n = (out.len() / 2).min(self.available());
        if let Ok(chunk) = self.inner.read_chunk(n * 2) {
            let (a, b) = chunk.as_slices();
            out[..a.len()].copy_from_slice(a);
            out[a.len()..a.len() + b.len()].copy_from_slice(b);
            chunk.commit_all();
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_read_blocks() {
        let (mut p, mut c) = stereo_ring(2000);
        let samples: Vec<f32> = (0..BLOCK * 2).map(|i| i as f32).collect();
        p.push(&samples);
        let mut block = Block::SILENT;
        assert_eq!(c.read_block(&mut block), BLOCK);
        assert_eq!((block.l[1], block.r[1]), (2.0, 3.0));
        assert_eq!(c.read_block(&mut block), 0);
    }

    #[test]
    fn overflow_is_counted() {
        let (mut p, c) = stereo_ring(100);
        p.push(&vec![0.0; 300]);
        assert_eq!(c.overflows.load(Ordering::Relaxed), 50);
        assert_eq!(c.available(), 100);
    }
}
