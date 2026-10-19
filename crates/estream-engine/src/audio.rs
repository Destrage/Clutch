//! The real-time audio thread: runs the mixer on a 100 Hz frame clock.

use crate::graph::RenderGraph;
use crossbeam_channel::Receiver;
use estream_audio::devices::MonitorOutput;
use estream_audio::mixer::MeterSet;
use estream_audio::{AudioCore, TrackSink};
use estream_core::frame_clock::{FrameClock, wait_until};
use estream_core::{EngineClock, Rational, SourceId, SourceStatus, Timestamp};
use estream_project::Monitoring;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Callbacks from the audio thread to the engine's shared state.
pub(crate) trait AudioHost: Send + Sync + 'static {
    fn graph(&self) -> Arc<RenderGraph>;
    fn publish_audio(&self, meters: MeterSet, statuses: HashMap<SourceId, SourceStatus>);
    fn shutdown_requested(&self) -> &AtomicBool;
}

/// At most this many missed blocks are caught up after a stall (keeps tracks continuous).
const MAX_CATCH_UP: u64 = 10;
/// One mixer block (100 Hz) in nanoseconds.
const BLOCK_NANOS: u64 = 10_000_000;

pub(crate) fn run(
    mut core: AudioCore,
    clock: EngineClock,
    host: Arc<dyn AudioHost>,
    new_sinks: Receiver<Box<dyn TrackSink>>,
) {
    let mut frames = FrameClock::new(Rational::fps(100), clock.now());
    let mut revision = u64::MAX;
    let mut monitor_key: Option<(String, bool)> = None;
    let mut last_statuses = HashMap::new();
    let mut statuses_checked = Instant::now();
    while !host.shutdown_requested().load(Ordering::Acquire) {
        while let Ok(sink) = new_sinks.try_recv() {
            core.add_sink(sink);
        }
        let graph = host.graph();
        let changed = graph.revision != revision;
        if changed {
            revision = graph.revision;
            core.sync(&graph.audio);
            // Open the monitoring device only while something is monitored.
            let wants = graph
                .audio
                .nodes
                .iter()
                .any(|n| n.config.monitoring != Monitoring::Off);
            let key = (graph.audio.monitor_device.clone(), wants);
            if monitor_key.as_ref() != Some(&key) {
                core.set_monitor_sink(
                    wants.then(|| Box::new(MonitorOutput::start(&key.0)) as Box<_>),
                );
                monitor_key = Some(key);
            }
        }
        if changed || statuses_checked.elapsed() >= Duration::from_millis(250) {
            statuses_checked = Instant::now();
            let statuses = core.statuses();
            if changed || statuses != last_statuses {
                host.publish_audio(core.meters(), statuses.clone());
                last_statuses = statuses;
            }
        }
        wait_until(&clock, frames.next_deadline());
        let tick = frames.advance(clock.now());
        // Each caught-up block keeps its own slot on the clock; outputs fill longer gaps.
        let catch_up = tick.skipped.min(MAX_CATCH_UP);
        for back in (0..=catch_up).rev() {
            core.process(Timestamp(
                tick.timestamp.0.saturating_sub(back * BLOCK_NANOS),
            ));
        }
    }
}
