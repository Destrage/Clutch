//! Strong noise suppression's worker: one `estream-denoise` thread per filter, fed 10 ms blocks
//! through lock-free rings. The audio thread never waits on it.

use crate::BLOCK;
use estream_denoise::Denoiser;
use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::Thread;
use std::time::Duration;

/// Blocks in flight each way; more than this means the worker is behind and blocks are dropped.
const QUEUE: usize = 8;

#[derive(Clone, Copy)]
struct Hop {
    seq: u64,
    samples: [f32; BLOCK],
}

/// The audio thread's end of a worker.
pub(crate) struct StrongLink {
    input: Producer<Hop>,
    output: Consumer<Hop>,
    worker: Thread,
    stop: Arc<AtomicBool>,
    seq: u64,
}

impl StrongLink {
    pub(crate) fn start() -> std::io::Result<Self> {
        let (input, worker_input) = RingBuffer::new(QUEUE);
        let (worker_output, output) = RingBuffer::new(QUEUE);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let handle = std::thread::Builder::new()
            .name("estream-denoise".into())
            .spawn(move || run(worker_input, worker_output, &worker_stop))?;
        Ok(Self {
            input,
            output,
            worker: handle.thread().clone(),
            stop,
            seq: 0,
        })
    }

    /// Sends this block and fills `out` with the previous block's result, if the worker has it.
    /// Never blocks.
    pub(crate) fn exchange(&mut self, mono: &[f32; BLOCK], out: &mut [f32; BLOCK]) -> bool {
        let seq = self.seq;
        self.seq += 1;
        let _ = self.input.push(Hop {
            seq,
            samples: *mono,
        });
        self.worker.unpark();
        let Some(want) = seq.checked_sub(1) else {
            return false;
        };
        while let Ok(hop) = self.output.peek() {
            if hop.seq >= want {
                break;
            }
            let _ = self.output.pop();
        }
        match self.output.peek() {
            Ok(hop) if hop.seq == want => {
                *out = hop.samples;
                let _ = self.output.pop();
                true
            }
            _ => false,
        }
    }
}

impl Drop for StrongLink {
    fn drop(&mut self) {
        // Detached: the worker sees the flag on its next wake and exits on its own.
        self.stop.store(true, Ordering::Release);
        self.worker.unpark();
    }
}

fn run(mut input: Consumer<Hop>, mut output: Producer<Hop>, stop: &AtomicBool) {
    let mut denoiser = match Denoiser::new() {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(error = %e, "strong noise suppression unavailable; using light");
            return;
        }
    };
    // Blocks that queued while the model loaded are stale: start from the next one.
    while input.pop().is_ok() {}
    let mut out = Hop {
        seq: 0,
        samples: [0.0; BLOCK],
    };
    let mut failed = false;
    while !stop.load(Ordering::Acquire) {
        while let Ok(hop) = input.pop() {
            if let Err(e) = denoiser.process(&hop.samples, &mut out.samples) {
                if !failed {
                    tracing::warn!(error = %e, "strong noise suppression failed a block");
                    failed = true;
                }
                continue;
            }
            out.seq = hop.seq;
            let _ = output.push(out);
        }
        std::thread::park_timeout(Duration::from_millis(100));
    }
}
