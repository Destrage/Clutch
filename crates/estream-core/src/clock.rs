//! The engine's single monotonic timebase.

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// Nanoseconds since the engine clock's epoch (engine start).
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct Timestamp(pub u64);

impl Timestamp {
    pub const ZERO: Self = Self(0);

    pub fn as_nanos(self) -> u64 {
        self.0
    }

    pub fn as_secs_f64(self) -> f64 {
        self.0 as f64 / 1e9
    }

    pub fn saturating_sub(self, other: Self) -> Duration {
        Duration::from_nanos(self.0.saturating_sub(other.0))
    }
}

impl std::ops::Add<Duration> for Timestamp {
    type Output = Timestamp;
    fn add(self, rhs: Duration) -> Timestamp {
        Timestamp(self.0 + rhs.as_nanos() as u64)
    }
}

/// Monotonic clock shared by every media producer and consumer.
///
/// Backed by `std::time::Instant` (QueryPerformanceCounter on Windows,
/// `mach_absolute_time` on macOS, `CLOCK_MONOTONIC` on Linux).
#[derive(Clone, Copy, Debug)]
pub struct EngineClock {
    epoch: Instant,
}

impl EngineClock {
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
        }
    }

    pub fn now(&self) -> Timestamp {
        self.timestamp_of(Instant::now())
    }

    pub fn timestamp_of(&self, instant: Instant) -> Timestamp {
        Timestamp(instant.saturating_duration_since(self.epoch).as_nanos() as u64)
    }

    pub fn instant_of(&self, ts: Timestamp) -> Instant {
        self.epoch + Duration::from_nanos(ts.0)
    }
}

impl Default for EngineClock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_is_monotonic() {
        let clock = EngineClock::new();
        let mut last = clock.now();
        for _ in 0..10_000 {
            let now = clock.now();
            assert!(now >= last);
            last = now;
        }
    }

    #[test]
    fn instant_round_trip() {
        let clock = EngineClock::new();
        let ts = Timestamp(1_234_567_890);
        assert_eq!(clock.timestamp_of(clock.instant_of(ts)), ts);
    }
}
