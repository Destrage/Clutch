//! Turns clip requests into windows of time to save from the replay buffer.
//!
//! A request asks for `pre` before and `post` after a moment. Requests whose windows overlap
//! or touch merge into one clip (a kill and then a double kill 4 s later give one clip), as
//! long as the merged clip stays within the maximum length; past that a new clip starts.
//! A window is handed out once its end is buffered, plus a short settle time.

use crate::event::ClipEvent;
use estream_core::Timestamp;
use std::time::Duration;

/// How long after a window's end it is saved, so the last packets are in the buffer.
pub const SETTLE: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, PartialEq)]
pub struct ClipRequest {
    pub event: ClipEvent,
    pub pre: Duration,
    pub post: Duration,
}

/// A span of engine-clock time to save as one clip.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub start: Timestamp,
    pub end: Timestamp,
    /// Every event in the clip, in time order.
    pub events: Vec<ClipEvent>,
}

impl Window {
    pub fn length(&self) -> Duration {
        Duration::from_nanos(self.end.0.saturating_sub(self.start.0))
    }
}

#[derive(Debug)]
pub struct Scheduler {
    pending: Vec<Window>,
    max: Duration,
}

impl Scheduler {
    /// `max`: the longest clip a merge may produce.
    pub fn new(max: Duration) -> Self {
        Self {
            pending: Vec::new(),
            max,
        }
    }

    pub fn set_max(&mut self, max: Duration) {
        self.max = max;
    }

    /// Windows waiting for their end to be buffered.
    pub fn pending(&self) -> &[Window] {
        &self.pending
    }

    pub fn request(&mut self, request: ClipRequest) {
        let at = request.event.at;
        let start = Timestamp(at.0.saturating_sub(request.pre.as_nanos() as u64));
        let end = Timestamp(at.0.saturating_add(request.post.as_nanos() as u64));
        let max = self.max.as_nanos() as u64;
        let merge = self.pending.iter_mut().find(|w| {
            let (s, e) = (w.start.min(start), w.end.max(end));
            start <= w.end && end >= w.start && e.0 - s.0 <= max
        });
        match merge {
            Some(w) => {
                w.start = w.start.min(start);
                w.end = w.end.max(end);
                let i = w.events.partition_point(|e| e.at <= at);
                w.events.insert(i, request.event);
            }
            None => self.pending.push(Window {
                start,
                end,
                events: vec![request.event],
            }),
        }
    }

    /// Removes and returns the windows whose end (plus [`SETTLE`]) is at or before `now`.
    pub fn due(&mut self, now: Timestamp) -> Vec<Window> {
        let settle = SETTLE.as_nanos() as u64;
        let (due, waiting) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|w| w.end.0.saturating_add(settle) <= now.0);
        self.pending = waiting;
        due
    }

    /// Drops every pending window (e.g. when the replay buffer stops).
    pub fn clear(&mut self) {
        self.pending.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EventKind;

    const S: u64 = 1_000_000_000;

    fn req(kind: EventKind, at_s: f64, pre: u64, post: u64) -> ClipRequest {
        ClipRequest {
            event: ClipEvent {
                kind,
                at: Timestamp((at_s * S as f64) as u64),
                game: Some("Counter-Strike 2".into()),
            },
            pre: Duration::from_secs(pre),
            post: Duration::from_secs(post),
        }
    }

    fn secs(t: Timestamp) -> f64 {
        t.0 as f64 / S as f64
    }

    #[test]
    fn waits_for_the_post_roll() {
        let mut s = Scheduler::new(Duration::from_secs(90));
        s.request(req(EventKind::Kill, 100.0, 10, 3));
        assert!(s.due(Timestamp(103 * S)).is_empty(), "end not buffered yet");
        let due = s.due(Timestamp(103 * S + S / 2));
        assert_eq!(due.len(), 1);
        assert_eq!((secs(due[0].start), secs(due[0].end)), (90.0, 103.0));
        assert!(s.pending().is_empty());
    }

    #[test]
    fn overlapping_requests_merge() {
        let mut s = Scheduler::new(Duration::from_secs(90));
        s.request(req(EventKind::Kill, 100.0, 10, 3));
        s.request(req(EventKind::MultiKill { n: 2 }, 104.0, 10, 3));
        let due = s.due(Timestamp(200 * S));
        assert_eq!(due.len(), 1);
        let w = &due[0];
        assert_eq!((secs(w.start), secs(w.end)), (90.0, 107.0));
        assert_eq!(w.length(), Duration::from_secs(17));
        let kinds: Vec<_> = w.events.iter().map(|e| e.kind.clone()).collect();
        assert_eq!(kinds, [EventKind::Kill, EventKind::MultiKill { n: 2 }]);
    }

    #[test]
    fn touching_windows_merge_but_gaps_do_not() {
        let mut s = Scheduler::new(Duration::from_secs(90));
        s.request(req(EventKind::Kill, 100.0, 10, 3)); // 90..103
        s.request(req(EventKind::Kill, 113.0, 10, 3)); // 103..116: touches
        s.request(req(EventKind::Kill, 140.0, 10, 3)); // 130..143: gap
        let due = s.due(Timestamp(200 * S));
        assert_eq!(due.len(), 2);
        assert_eq!((secs(due[0].start), secs(due[0].end)), (90.0, 116.0));
        assert_eq!((secs(due[1].start), secs(due[1].end)), (130.0, 143.0));
    }

    #[test]
    fn merges_stop_at_the_maximum_length() {
        let mut s = Scheduler::new(Duration::from_secs(30));
        s.request(req(EventKind::Kill, 100.0, 10, 3)); // 90..103
        s.request(req(EventKind::Kill, 110.0, 10, 3)); // 100..113 → 90..113 (23 s)
        s.request(req(EventKind::Kill, 120.0, 10, 3)); // 110..123 → would be 33 s: new clip
        let due = s.due(Timestamp(200 * S));
        assert_eq!(due.len(), 2);
        assert_eq!(due[0].length(), Duration::from_secs(23));
        assert_eq!(due[0].events.len(), 2);
        assert_eq!((secs(due[1].start), secs(due[1].end)), (110.0, 123.0));
    }

    #[test]
    fn late_events_are_kept_in_time_order() {
        let mut s = Scheduler::new(Duration::from_secs(90));
        s.request(req(EventKind::Kill, 104.0, 10, 3));
        s.request(req(EventKind::Hype, 101.0, 10, 3)); // detected later, happened earlier
        let due = s.due(Timestamp(200 * S));
        assert_eq!(due[0].events[0].kind, EventKind::Hype);
        assert_eq!(due[0].events[1].kind, EventKind::Kill);
    }

    #[test]
    fn early_requests_do_not_underflow() {
        let mut s = Scheduler::new(Duration::from_secs(90));
        s.request(req(EventKind::Manual, 2.0, 30, 0));
        let due = s.due(Timestamp(3 * S));
        assert_eq!(due[0].start, Timestamp::ZERO);
    }
}
