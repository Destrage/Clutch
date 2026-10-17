//! Streaming to several destinations at once. Destinations with identical encoder settings
//! share one encoder session; each destination has its own connection, send queue, stats,
//! reconnect logic and stop switch, so one failing never disturbs the others.

use crate::MediaError;
use crate::ffmpeg;
use crate::output::{Kind, OutputStats, Session};
use crate::settings::{RecordingSettings, StreamSettings, redact};
use crate::stream::{DestinationStats, NetSpec, SendQueue, StreamState};
use estream_engine::Engine;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// A place to stream to.
#[derive(Clone, Debug, PartialEq)]
pub struct Destination {
    pub name: String,
    pub settings: StreamSettings,
}

/// A destination within a running [`MultiStream`].
pub struct LiveDestination {
    pub stats: Arc<DestinationStats>,
    own_stop: Arc<AtomicBool>,
    /// `None` if the destination never started (invalid settings or encoder failure).
    queue: Option<Arc<SendQueue<ffmpeg::Packet>>>,
    group: Option<usize>,
    started: Instant,
}

impl LiveDestination {
    pub fn name(&self) -> &str {
        &self.stats.name
    }

    pub fn state(&self) -> StreamState {
        if self.queue.is_none() || self.own_stop.load(Ordering::Acquire) {
            StreamState::Stopped
        } else {
            self.stats.network.state()
        }
    }

    /// Streaming, connecting or reconnecting.
    pub fn is_active(&self) -> bool {
        self.state() != StreamState::Stopped
    }

    pub fn error(&self) -> Option<String> {
        self.stats.error()
    }

    /// Time since this destination was (re)started.
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }
}

/// One encoder session and the encoder settings that identify it.
struct Group {
    key: RecordingSettings,
    session: Option<Session>,
}

/// A live stream to one or more destinations.
pub struct MultiStream {
    groups: Vec<Group>,
    destinations: Vec<LiveDestination>,
    started: Instant,
}

impl MultiStream {
    /// Starts every destination. Invalid ones are kept with their error so the user can see
    /// what to fix; starting fails only if none of them could start.
    pub fn start(engine: &Engine, destinations: &[Destination]) -> Result<Self, MediaError> {
        let mut multi = Self {
            groups: Vec::new(),
            destinations: Vec::new(),
            started: Instant::now(),
        };
        for d in destinations {
            let live = multi.launch(engine, d);
            multi.destinations.push(live);
        }
        if !multi.destinations.iter().any(LiveDestination::is_active) {
            let problems: Vec<String> = multi
                .destinations
                .iter()
                .filter_map(|d| d.error().map(|e| format!("{}: {e}", d.name())))
                .collect();
            return Err(MediaError::Engine(if problems.is_empty() {
                "No stream destinations are enabled".into()
            } else {
                problems.join("; ")
            }));
        }
        Ok(multi)
    }

    fn launch(&mut self, engine: &Engine, d: &Destination) -> LiveDestination {
        let stats = Arc::new(DestinationStats {
            name: d.name.clone(),
            destination: redact(&d.settings.url()),
            ..Default::default()
        });
        let mut live = LiveDestination {
            stats: stats.clone(),
            own_stop: Arc::new(AtomicBool::new(false)),
            queue: None,
            group: None,
            started: Instant::now(),
        };
        if let Err(problem) = d.settings.validate() {
            stats.set_error(problem);
            stats.network.set_state(StreamState::Stopped);
            return live;
        }
        let spec = NetSpec::new(d.settings.clone(), stats.clone());
        live.own_stop = spec.own_stop.clone();
        let queue = spec.queue.clone();
        let key = d.settings.encoder_settings();
        let joined = self
            .groups
            .iter()
            .position(|g| g.key == key && g.session.is_some());
        let result = match joined {
            Some(g) => match &self.groups[g].session {
                Some(session) => session.add_destination(spec).map(|_| g),
                None => Err(MediaError::Engine("encoder session ended".into())),
            },
            None => Session::start(engine, &key, Kind::Network(vec![spec])).map(|session| {
                self.groups.push(Group {
                    key,
                    session: Some(session),
                });
                self.groups.len() - 1
            }),
        };
        match result {
            Ok(g) => {
                tracing::info!(
                    name = %d.name,
                    destination = %stats.destination,
                    shared = joined.is_some(),
                    "stream destination started"
                );
                live.queue = Some(queue);
                live.group = Some(g);
            }
            Err(e) => {
                stats.set_error(e.to_string());
                stats.network.set_state(StreamState::Stopped);
            }
        }
        live
    }

    /// Adds a destination while live; it joins a matching encoder or gets its own.
    pub fn add_destination(&mut self, engine: &Engine, d: &Destination) -> usize {
        let live = self.launch(engine, d);
        self.destinations.push(live);
        self.destinations.len() - 1
    }

    /// Starts destination `index` again (e.g. after it was stopped or failed), possibly with
    /// new settings. Other destinations are not touched.
    pub fn restart_destination(&mut self, engine: &Engine, index: usize, d: &Destination) {
        if index >= self.destinations.len() {
            return;
        }
        self.stop_destination(engine, index);
        let live = self.launch(engine, d);
        self.destinations[index] = live;
    }

    /// Ends one destination; the others continue.
    pub fn stop_destination(&mut self, engine: &Engine, index: usize) {
        if let Some(d) = self.destinations.get(index) {
            d.own_stop.store(true, Ordering::Release);
            if let Some(q) = &d.queue {
                q.close();
            }
            tracing::info!(name = %d.name(), "stream destination stopped");
        }
        self.poll(engine);
    }

    /// Updates encoder counters and stops encoders no destination uses any more.
    pub fn poll(&mut self, engine: &Engine) {
        for (g, group) in self.groups.iter_mut().enumerate() {
            let Some(session) = &group.session else {
                continue;
            };
            session.update_dropped(engine);
            let in_use = self
                .destinations
                .iter()
                .any(|d| d.group == Some(g) && d.is_active());
            if !in_use && let Some(session) = group.session.take() {
                session.stop_in_background(engine);
            }
        }
    }

    pub fn destinations(&self) -> &[LiveDestination] {
        &self.destinations
    }

    /// Encoder statistics (frames, encoder lag, encoder name) feeding destination `index`.
    pub fn encoder_stats(&self, index: usize) -> Option<&Arc<OutputStats>> {
        let g = self.destinations.get(index)?.group?;
        self.groups.get(g)?.session.as_ref().map(|s| &s.stats)
    }

    /// Number of video encoders currently running.
    pub fn encoder_count(&self) -> usize {
        self.groups.iter().filter(|g| g.session.is_some()).count()
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// True while any destination is streaming, connecting or reconnecting.
    pub fn is_active(&self) -> bool {
        self.destinations.iter().any(LiveDestination::is_active)
    }

    /// Stops everything and waits for connections to close.
    pub fn stop(mut self, engine: &Engine) -> Result<(), MediaError> {
        let sessions: Vec<Session> = self
            .groups
            .iter_mut()
            .filter_map(|g| g.session.take())
            .collect();
        let mut result = Ok(());
        // Detach all first so every connection winds down in parallel.
        let mut detached = Vec::new();
        for mut s in sessions {
            s.detach(engine);
            detached.push(s);
        }
        for s in detached {
            if let Err(e) = s.join() {
                result = Err(e);
            }
        }
        result
    }

    /// Stops everything without blocking the caller.
    pub fn stop_in_background(mut self, engine: &Engine) {
        for g in &mut self.groups {
            if let Some(session) = g.session.take() {
                session.stop_in_background(engine);
            }
        }
    }
}
