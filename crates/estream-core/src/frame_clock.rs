//! Drift-free fixed-rate frame scheduling.

use crate::{EngineClock, Rational, Timestamp};
use std::time::Duration;

/// One scheduled frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameTick {
    pub index: u64,
    /// Presentation timestamp of this frame (its scheduled deadline).
    pub timestamp: Timestamp,
    /// Frames skipped right before this one because the engine fell behind.
    pub skipped: u64,
}

/// Schedules frames at `start + index * den/num` seconds. Deadlines never drift:
/// a late frame does not push later frames back; missed frames are skipped and counted.
#[derive(Clone, Debug)]
pub struct FrameClock {
    rate: Rational,
    start: Timestamp,
    next_index: u64,
    lagged: u64,
    rendered: u64,
}

impl FrameClock {
    pub fn new(rate: Rational, start: Timestamp) -> Self {
        Self {
            rate,
            start,
            next_index: 0,
            lagged: 0,
            rendered: 0,
        }
    }

    pub fn rate(&self) -> Rational {
        self.rate
    }

    pub fn next_deadline(&self) -> Timestamp {
        self.deadline_of(self.next_index)
    }

    fn deadline_of(&self, index: u64) -> Timestamp {
        Timestamp(self.start.0 + self.rate.frame_time_nanos(index))
    }

    /// Claims the frame due at `now`. Call once `now >= next_deadline()`.
    pub fn advance(&mut self, now: Timestamp) -> FrameTick {
        let mut index = self.next_index;
        let current = self.rate.frame_at_nanos(now.0.saturating_sub(self.start.0));
        let skipped = current.saturating_sub(index);
        if skipped > 0 {
            index = current;
            self.lagged += skipped;
        }
        self.next_index = index + 1;
        self.rendered += 1;
        FrameTick {
            index,
            timestamp: self.deadline_of(index),
            skipped,
        }
    }

    pub fn lagged(&self) -> u64 {
        self.lagged
    }

    pub fn rendered(&self) -> u64 {
        self.rendered
    }
}

/// Waits until `deadline` with sub-millisecond precision: a coarse sleep
/// (high-resolution waitable timer on Windows) followed by a short spin.
pub fn wait_until(clock: &EngineClock, deadline: Timestamp) {
    const SPIN: Duration = Duration::from_micros(1500);
    loop {
        let now = clock.now();
        if now >= deadline {
            return;
        }
        let remaining = deadline.saturating_sub(now);
        if remaining > SPIN {
            std::thread::sleep(remaining - SPIN);
        } else if remaining > Duration::from_micros(200) {
            std::thread::yield_now();
        } else {
            std::hint::spin_loop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_time_frames_are_consecutive() {
        let rate = Rational::fps(60);
        let mut clock = FrameClock::new(rate, Timestamp(1000));
        for i in 0..120 {
            let deadline = clock.next_deadline();
            let tick = clock.advance(deadline);
            assert_eq!(tick.index, i);
            assert_eq!(tick.skipped, 0);
            assert_eq!(tick.timestamp, deadline);
        }
        assert_eq!(clock.lagged(), 0);
    }

    #[test]
    fn overload_skips_without_drift() {
        let rate = Rational::fps(60);
        let mut clock = FrameClock::new(rate, Timestamp(0));
        clock.advance(Timestamp(0));
        // Frame 1 was due at 16.67ms but we only got to it at 55ms (frame 3's slot).
        let tick = clock.advance(Timestamp(55_000_000));
        assert_eq!(tick.index, 3);
        assert_eq!(tick.skipped, 2);
        assert_eq!(clock.lagged(), 2);
        // The schedule stays aligned to the original timeline.
        assert_eq!(clock.next_deadline(), Timestamp(rate.frame_time_nanos(4)));
    }
}
