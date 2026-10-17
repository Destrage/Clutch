//! Lock-free level meters written by the audio thread, read by the UI.

use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Debug, Default)]
pub struct Meter {
    peak: [AtomicU32; 2],
    rms: [AtomicU32; 2],
}

/// Linear levels (0 = silence, 1 = full scale).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeterReading {
    pub peak: [f32; 2],
    pub rms: [f32; 2],
}

impl Meter {
    pub fn store(&self, peak: [f32; 2], rms: [f32; 2]) {
        for i in 0..2 {
            self.peak[i].store(peak[i].to_bits(), Ordering::Relaxed);
            self.rms[i].store(rms[i].to_bits(), Ordering::Relaxed);
        }
    }

    pub fn read(&self) -> MeterReading {
        let get = |a: &AtomicU32| f32::from_bits(a.load(Ordering::Relaxed));
        MeterReading {
            peak: [get(&self.peak[0]), get(&self.peak[1])],
            rms: [get(&self.rms[0]), get(&self.rms[1])],
        }
    }
}
