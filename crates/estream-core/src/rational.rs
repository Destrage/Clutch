//! Exact rational numbers for frame rates such as 60000/1001.

use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rational {
    pub num: u32,
    pub den: u32,
}

impl Rational {
    pub const fn new(num: u32, den: u32) -> Self {
        Self { num, den }
    }

    pub const fn fps(fps: u32) -> Self {
        Self { num: fps, den: 1 }
    }

    pub fn is_valid(self) -> bool {
        self.num > 0 && self.den > 0
    }

    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// Exact time (in nanoseconds) at which frame `index` starts, relative to frame 0.
    pub fn frame_time_nanos(self, index: u64) -> u64 {
        ((index as u128 * self.den as u128 * 1_000_000_000) / self.num as u128) as u64
    }

    /// Index of the frame that is active at `nanos` after frame 0.
    pub fn frame_at_nanos(self, nanos: u64) -> u64 {
        ((nanos as u128 * self.num as u128) / (self.den as u128 * 1_000_000_000)) as u64
    }

    /// Nominal duration of one frame (rounded down to the nanosecond).
    pub fn frame_duration(self) -> Duration {
        Duration::from_nanos(self.frame_time_nanos(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ntsc_has_no_drift() {
        let r = Rational::new(60000, 1001);
        // One hour at 59.94 fps: frames 0..=215784 start at or before 3600.0 s.
        let hour = 3_600_000_000_000u64;
        assert_eq!(r.frame_at_nanos(hour), 215_784);
        assert!(r.frame_time_nanos(215_784) <= hour);
        assert!(r.frame_time_nanos(215_785) > hour);
    }

    #[test]
    fn integer_rates() {
        let r = Rational::fps(60);
        assert_eq!(r.frame_time_nanos(60), 1_000_000_000);
        assert_eq!(r.frame_at_nanos(1_000_000_000), 60);
    }
}
