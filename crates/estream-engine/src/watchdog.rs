//! Video thread watchdog: the video loop records which stage it is in, and a separate
//! thread logs when it stops making progress (and where), so a hang leaves a trace.

use estream_core::{EngineClock, SourceId, Timestamp};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::time::Duration;
use uuid::Uuid;

/// No progress for this long counts as a stall.
const STALL: Duration = Duration::from_secs(2);
const CHECK_EVERY: Duration = Duration::from_millis(500);

/// Where the video loop is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Stage {
    Sync,
    Publish,
    Idle,
    Wait,
    TickSource,
    Filters,
    Render,
    Submit,
    Poll,
    Deliver,
    Shutdown,
}

impl Stage {
    const ALL: [Stage; 11] = [
        Stage::Sync,
        Stage::Publish,
        Stage::Idle,
        Stage::Wait,
        Stage::TickSource,
        Stage::Filters,
        Stage::Render,
        Stage::Submit,
        Stage::Poll,
        Stage::Deliver,
        Stage::Shutdown,
    ];

    fn from_u8(v: u8) -> Stage {
        Self::ALL.get(v as usize).copied().unwrap_or(Stage::Sync)
    }

    pub fn name(self) -> &'static str {
        match self {
            Stage::Sync => "syncing sources",
            Stage::Publish => "publishing statuses",
            Stage::Idle => "idle",
            Stage::Wait => "waiting for the next frame",
            Stage::TickSource => "ticking a source",
            Stage::Filters => "running filters",
            Stage::Render => "rendering canvases",
            Stage::Submit => "submitting GPU work",
            Stage::Poll => "polling the GPU",
            Stage::Deliver => "delivering frames to outputs",
            Stage::Shutdown => "waiting for the GPU at shutdown",
        }
    }
}

/// Progress markers written by the video thread (atomics only: no locks or allocation).
#[derive(Debug)]
pub struct VideoProgress {
    clock: EngineClock,
    stage: AtomicU8,
    /// Engine time when the current stage was entered.
    since: AtomicU64,
    /// The source being ticked (a `SourceId` split in two), valid in `Stage::TickSource`.
    source: [AtomicU64; 2],
    done: AtomicBool,
}

impl VideoProgress {
    pub fn new(clock: EngineClock) -> Self {
        Self {
            clock,
            stage: AtomicU8::new(Stage::Sync as u8),
            since: AtomicU64::new(clock.now().0),
            source: [AtomicU64::new(0), AtomicU64::new(0)],
            done: AtomicBool::new(false),
        }
    }

    pub fn enter(&self, stage: Stage) {
        self.stage.store(stage as u8, Ordering::Relaxed);
        self.since.store(self.clock.now().0, Ordering::Release);
    }

    pub fn enter_source(&self, id: SourceId) {
        let v = id.0.as_u128();
        self.source[0].store((v >> 64) as u64, Ordering::Relaxed);
        self.source[1].store(v as u64, Ordering::Relaxed);
        self.enter(Stage::TickSource);
    }

    pub fn finish(&self) {
        self.done.store(true, Ordering::Release);
    }

    fn snapshot(&self) -> (Stage, Timestamp, Option<SourceId>) {
        let since = Timestamp(self.since.load(Ordering::Acquire));
        let stage = Stage::from_u8(self.stage.load(Ordering::Relaxed));
        let source = (stage == Stage::TickSource).then(|| {
            let hi = self.source[0].load(Ordering::Relaxed) as u128;
            let lo = self.source[1].load(Ordering::Relaxed) as u128;
            SourceId(Uuid::from_u128(hi << 64 | lo))
        });
        (stage, since, source)
    }
}

/// Marks the video thread finished when dropped, even if it unwinds.
pub struct FinishGuard(pub Arc<VideoProgress>);

impl Drop for FinishGuard {
    fn drop(&mut self) {
        self.0.finish();
    }
}

#[derive(Debug, PartialEq)]
pub enum Report {
    Stalled {
        stage: Stage,
        source: Option<SourceId>,
        stalled: Duration,
    },
    Recovered {
        stalled: Duration,
    },
}

/// Turns progress samples into stall/recovery reports (one of each per stall).
#[derive(Default)]
pub struct Monitor {
    /// Stage start time of the stall being reported.
    stalled_since: Option<Timestamp>,
}

impl Monitor {
    pub fn observe(&mut self, progress: &VideoProgress, now: Timestamp) -> Option<Report> {
        let (stage, since, source) = progress.snapshot();
        let stalled = Duration::from_nanos(now.0.saturating_sub(since.0));
        match self.stalled_since {
            Some(at) if at != since => {
                self.stalled_since = None;
                let stalled = Duration::from_nanos(since.0.saturating_sub(at.0));
                Some(Report::Recovered { stalled })
            }
            Some(_) => None,
            None if stalled >= STALL => {
                self.stalled_since = Some(since);
                Some(Report::Stalled {
                    stage,
                    source,
                    stalled,
                })
            }
            None => None,
        }
    }
}

/// Starts the watchdog thread. It is never joined, so a hung video thread can't hang it
/// too; it ends once the video thread finishes. `name_of` names a source for the log.
pub fn spawn(
    progress: Arc<VideoProgress>,
    name_of: impl Fn(SourceId) -> Option<String> + Send + 'static,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("estream-video-watchdog".into())
        .spawn(move || {
            let mut monitor = Monitor::default();
            while !progress.done.load(Ordering::Acquire) {
                std::thread::sleep(CHECK_EVERY);
                match monitor.observe(&progress, progress.clock.now()) {
                    Some(Report::Stalled {
                        stage,
                        source,
                        stalled,
                    }) => {
                        let source = source.map(|id| name_of(id).unwrap_or_else(|| id.to_string()));
                        tracing::warn!(
                            stage = stage.name(),
                            source = source.as_deref().unwrap_or("-"),
                            stalled_ms = stalled.as_millis() as u64,
                            "video thread stalled"
                        );
                    }
                    Some(Report::Recovered { stalled }) => {
                        tracing::info!(
                            stalled_ms = stalled.as_millis() as u64,
                            "video thread recovered"
                        );
                    }
                    None => {}
                }
            }
        })
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(progress: &VideoProgress, offset: Duration) -> Timestamp {
        Timestamp(progress.since.load(Ordering::Acquire) + offset.as_nanos() as u64)
    }

    #[test]
    fn reports_a_stall_once_with_its_stage_and_source() {
        let progress = VideoProgress::new(EngineClock::new());
        let id = SourceId::new();
        progress.enter_source(id);
        let mut monitor = Monitor::default();
        assert_eq!(
            monitor.observe(&progress, at(&progress, Duration::from_millis(500))),
            None
        );
        let report = monitor.observe(&progress, at(&progress, Duration::from_secs(3)));
        assert_eq!(
            report,
            Some(Report::Stalled {
                stage: Stage::TickSource,
                source: Some(id),
                stalled: Duration::from_secs(3),
            })
        );
        assert_eq!(
            monitor.observe(&progress, at(&progress, Duration::from_secs(5))),
            None
        );
    }

    #[test]
    fn reports_recovery_when_the_stage_moves_on() {
        let progress = VideoProgress::new(EngineClock::new());
        progress.enter(Stage::Submit);
        let start = progress.since.load(Ordering::Acquire);
        let mut monitor = Monitor::default();
        let stalled = monitor.observe(&progress, at(&progress, Duration::from_secs(2)));
        assert!(matches!(
            stalled,
            Some(Report::Stalled {
                stage: Stage::Submit,
                source: None,
                ..
            })
        ));
        // The thread moves on 4 s after the stall began.
        progress.stage.store(Stage::Poll as u8, Ordering::Relaxed);
        progress.since.store(
            start + Duration::from_secs(4).as_nanos() as u64,
            Ordering::Release,
        );
        let now = at(&progress, Duration::from_millis(10));
        assert_eq!(
            monitor.observe(&progress, now),
            Some(Report::Recovered {
                stalled: Duration::from_secs(4)
            })
        );
        assert_eq!(monitor.observe(&progress, now), None);
    }

    #[test]
    fn short_frames_are_not_stalls() {
        let progress = VideoProgress::new(EngineClock::new());
        progress.enter(Stage::Wait);
        let mut monitor = Monitor::default();
        assert_eq!(
            monitor.observe(&progress, at(&progress, Duration::from_millis(16))),
            None
        );
    }
}
