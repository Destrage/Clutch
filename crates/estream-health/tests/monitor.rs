//! The health monitor with synthetic one-second samples.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use estream_health::{
    CanvasCounters, DestCounters, HISTORY, Monitor, OutputCounters, Sample, Severity, SystemSample,
};
use std::time::{Duration, Instant};

/// A studio whose counters advance by the given per-second amounts.
struct Sim {
    t0: Instant,
    sec: u64,
    rendered: u64,
    lagged: u64,
    frames: u64,
    dropped: u64,
    bytes: u64,
    net_bytes: u64,
    net_dropped: u64,
    system: SystemSample,
    queue_ms: u64,
    reconnecting: bool,
    recording: bool,
}

impl Sim {
    fn new() -> Self {
        Self {
            t0: Instant::now(),
            sec: 0,
            rendered: 0,
            lagged: 0,
            frames: 0,
            dropped: 0,
            bytes: 0,
            net_bytes: 0,
            net_dropped: 0,
            system: SystemSample {
                cpu_total: 20.0,
                cpu_process: 5.0,
                memory_bytes: 300 << 20,
                gpu: Some(30.0),
                disk_free: Some(500_000_000_000),
            },
            queue_ms: 20,
            reconnecting: false,
            recording: true,
        }
    }

    /// One second: `lag` frames skipped of 60, `drop` encoder drops of 60.
    fn step(&mut self, m: &mut Monitor, lag: u64, drop: u64, net_drop: u64) -> Vec<String> {
        self.sec += 1;
        self.rendered += 60 - lag;
        self.lagged += lag;
        self.frames += 60 - drop;
        self.dropped += drop;
        self.bytes += 750_000; // 6 Mb/s
        self.net_bytes += 750_000;
        self.net_dropped += net_drop;
        let sample = Sample {
            at: self.t0 + Duration::from_secs(self.sec),
            canvases: vec![CanvasCounters {
                name: "Main".into(),
                target_fps: 60.0,
                rendered: self.rendered,
                lagged: self.lagged,
                encoder_dropped: 0,
                render_ms: 4.0,
            }],
            outputs: vec![OutputCounters {
                key: "record".into(),
                name: "Recording".into(),
                encoder: "x264".into(),
                frames: self.frames,
                dropped: self.dropped,
                bytes: self.bytes,
                recording: self.recording,
            }],
            destinations: vec![DestCounters {
                key: "stream:0".into(),
                name: "Twitch".into(),
                state: if self.reconnecting {
                    "Reconnecting"
                } else {
                    "Live"
                }
                .into(),
                reconnecting: self.reconnecting,
                bytes: self.net_bytes,
                dropped: self.net_dropped,
                queue_ms: self.queue_ms,
                reconnects: 0,
                fps: 60.0,
            }],
            system: self.system.clone(),
        };
        m.push(sample).into_iter().map(|a| a.id).collect()
    }
}

#[test]
fn rates_and_history() {
    let mut m = Monitor::new();
    let mut s = Sim::new();
    for _ in 0..3 {
        s.step(&mut m, 0, 0, 0);
    }
    let p = m.latest();
    assert!((p.fps - 60.0).abs() < 0.01, "{p:?}");
    assert_eq!(p.skipped_pct, 0.0);
    assert!((p.upload_kbps - 6000.0).abs() < 1.0, "{p:?}");
    assert!((m.outputs()[0].kbps - 6000.0).abs() < 1.0);
    assert!((m.outputs()[0].fps - 60.0).abs() < 0.01);
    assert_eq!(m.health(), Severity::Ok);
    for _ in 0..400 {
        s.step(&mut m, 0, 0, 0);
    }
    assert_eq!(m.history().len(), HISTORY);
}

#[test]
fn counter_resets_start_a_new_baseline() {
    let mut m = Monitor::new();
    let mut s = Sim::new();
    s.step(&mut m, 0, 0, 0);
    s.step(&mut m, 0, 0, 0);
    // The output restarted: its counters go back to zero.
    s.frames = 0;
    s.dropped = 0;
    s.bytes = 0;
    s.step(&mut m, 0, 0, 0);
    assert!(m.outputs()[0].kbps <= 6000.0 + 1.0, "no huge spike");
    assert_eq!(m.health(), Severity::Ok);
}

#[test]
fn encoder_overload_raises_after_a_delay_and_clears_after_calm() {
    let mut m = Monitor::new();
    let mut s = Sim::new();
    s.step(&mut m, 0, 0, 0);
    let mut raised = Vec::new();
    for _ in 0..4 {
        raised.extend(s.step(&mut m, 0, 6, 0)); // 10% dropped
    }
    assert!(raised.is_empty(), "not before 5 s");
    for _ in 0..3 {
        raised.extend(s.step(&mut m, 0, 6, 0));
    }
    assert_eq!(raised, vec!["encoder:record"]);
    let alert = &m.alerts()[0];
    assert_eq!(alert.severity, Severity::Problem);
    assert!(
        alert.advice.contains("hardware encoder"),
        "{}",
        alert.advice
    );
    assert_eq!(m.health(), Severity::Problem);
    // Staying overloaded doesn't announce it again.
    assert!(s.step(&mut m, 0, 6, 0).is_empty());

    // Calm: the window empties, then the calm period passes.
    for _ in 0..25 {
        s.step(&mut m, 0, 0, 0);
    }
    assert_eq!(m.health(), Severity::Ok, "{:?}", m.alerts());
}

#[test]
fn render_lag_severity_follows_the_share_of_skipped_frames() {
    let mut m = Monitor::new();
    let mut s = Sim::new();
    s.step(&mut m, 0, 0, 0);
    for _ in 0..8 {
        s.step(&mut m, 3, 0, 0); // 5%
    }
    assert_eq!(m.alerts()[0].id, "render-lag:Main");
    assert_eq!(m.alerts()[0].severity, Severity::Warning);
    for _ in 0..12 {
        s.step(&mut m, 12, 0, 0); // 20%
    }
    assert_eq!(m.alerts()[0].severity, Severity::Problem);
    assert!(m.latest().skipped_pct > 19.0);
}

#[test]
fn network_and_reconnects() {
    let mut m = Monitor::new();
    let mut s = Sim::new();
    s.step(&mut m, 0, 0, 0);
    s.queue_ms = 3500;
    let mut raised = Vec::new();
    for _ in 0..5 {
        raised.extend(s.step(&mut m, 0, 0, 0));
    }
    assert!(
        raised.contains(&"network:stream:0".to_owned()),
        "{raised:?}"
    );
    s.queue_ms = 20;
    s.reconnecting = true;
    for _ in 0..4 {
        raised.extend(s.step(&mut m, 0, 0, 0));
    }
    assert!(
        raised.contains(&"reconnecting:stream:0".to_owned()),
        "{raised:?}"
    );
    let reconnect = m
        .alerts()
        .into_iter()
        .find(|a| a.id == "reconnecting:stream:0")
        .unwrap();
    assert!(reconnect.title.contains("Twitch"));
}

#[test]
fn system_load_and_disk_space() {
    let mut m = Monitor::new();
    let mut s = Sim::new();
    s.system.cpu_total = 97.0;
    for _ in 0..10 {
        s.step(&mut m, 0, 0, 0);
    }
    assert!(
        m.alerts().iter().all(|a| a.id != "cpu"),
        "15 s of high CPU first"
    );
    for _ in 0..8 {
        s.step(&mut m, 0, 0, 0);
    }
    assert!(m.alerts().iter().any(|a| a.id == "cpu"));

    // 750 kB/s of recording with 300 MB free: about 6.7 minutes left.
    s.system.disk_free = Some(300_000_000);
    s.step(&mut m, 0, 0, 0);
    let disk = m.alerts().into_iter().find(|a| a.id == "disk").unwrap();
    assert_eq!(disk.severity, Severity::Problem, "under 1 GB");
    assert!(disk.detail.contains("min left"), "{}", disk.detail);

    // Not recording: free space doesn't matter.
    let mut m = Monitor::new();
    let mut s = Sim::new();
    s.recording = false;
    s.system.disk_free = Some(300_000_000);
    s.step(&mut m, 0, 0, 0);
    s.step(&mut m, 0, 0, 0);
    assert!(m.alerts().iter().all(|a| a.id != "disk"));
}
