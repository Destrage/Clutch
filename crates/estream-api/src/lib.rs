//! Remote control for Clutch: an obs-websocket v5 compatible server.
//!
//! The server does network I/O on its own thread. The app calls [`Api::tick`] from its UI
//! tick: that answers queued requests (through the [`Studio`] trait and the engine's command
//! bus) and broadcasts events for whatever changed since the last tick.

pub mod events;
pub mod handlers;
pub mod protocol;
pub mod server;
pub mod studio;
pub mod views;

pub use server::{RequestError, Server, ServerError};
pub use studio::{MediaAction, MediaState, MediaStatus, Output, OutputStatus, Studio};

use protocol::intent;
use std::time::{Duration, Instant};

/// How often app state and volume meters are checked.
const POLL_EVERY: Duration = Duration::from_millis(50);

pub struct Api {
    server: Server,
    ctx: handlers::Ctx,
    translator: events::Translator,
    poller: events::StatePoller,
    polled_at: Instant,
    /// (time, rendered frames) of the last fps sample.
    fps_sample: Option<(Instant, u64)>,
}

impl Api {
    /// Starts the server on `port` (0 = any free port). An empty password disables auth.
    pub fn start(port: u16, password: Option<String>) -> Result<Self, ServerError> {
        Ok(Self {
            server: Server::start(port, password)?,
            ctx: handlers::Ctx::default(),
            translator: events::Translator::default(),
            poller: events::StatePoller::default(),
            polled_at: Instant::now(),
            fps_sample: None,
        })
    }

    pub fn port(&self) -> u16 {
        self.server.port()
    }

    pub fn client_count(&self) -> usize {
        self.server.client_count()
    }

    /// Answers queued requests and broadcasts events. Call often (every UI tick).
    pub fn tick(&mut self, studio: &mut dyn Studio) {
        let pending: Vec<_> = self.server.pending().collect();
        for request in pending {
            let reply = if request.request_type == "BroadcastCustomEvent" {
                match request.data.get("eventData") {
                    Some(data) if data.is_object() => {
                        self.server
                            .broadcast("CustomEvent", intent::GENERAL, Some(data.clone()));
                        Ok(serde_json::Value::Null)
                    }
                    _ => Err(RequestError::new(
                        protocol::status::MISSING_REQUEST_FIELD,
                        "missing `eventData` object",
                    )),
                }
            } else {
                handlers::handle(
                    &mut self.ctx,
                    studio,
                    &request.request_type,
                    &request.data,
                    &request.session,
                )
            };
            // Events caused by the request go out before its response.
            self.project_events(studio);
            request.respond(reply);
        }
        self.project_events(studio);
        if self.polled_at.elapsed() >= POLL_EVERY {
            self.polled_at = Instant::now();
            for e in self.poller.poll(studio) {
                self.server.broadcast(e.kind, e.intent, e.data);
            }
            if let Some(engine) = studio.engine() {
                if self.server.wants(intent::INPUT_VOLUME_METERS) {
                    let e = events::volume_meters(engine);
                    self.server.broadcast(e.kind, e.intent, e.data);
                }
                self.sample_fps(engine);
            }
        }
    }

    fn project_events(&mut self, studio: &dyn Studio) {
        let Some(engine) = studio.engine() else {
            self.translator.reset();
            return;
        };
        let snapshot = engine.snapshot();
        let transforms = self.server.wants(intent::SCENE_ITEM_TRANSFORM_CHANGED);
        for e in self
            .translator
            .diff(&snapshot, engine, &mut self.ctx.ids, transforms)
        {
            self.server.broadcast(e.kind, e.intent, e.data);
        }
    }

    fn sample_fps(&mut self, engine: &estream_engine::Engine) {
        use std::sync::atomic::Ordering::Relaxed;
        let snapshot = engine.snapshot();
        let Some(canvas) = snapshot.canvases.first() else {
            return;
        };
        self.server.set_frame_time(canvas.fps.frame_duration());
        let Some(output) = engine.canvas_output(canvas.id) else {
            return;
        };
        let rendered = output.stats.rendered.load(Relaxed);
        let now = Instant::now();
        match self.fps_sample {
            Some((at, frames)) if now.duration_since(at) >= Duration::from_secs(1) => {
                self.ctx.active_fps =
                    rendered.saturating_sub(frames) as f64 / now.duration_since(at).as_secs_f64();
                self.fps_sample = Some((now, rendered));
            }
            None => self.fps_sample = Some((now, rendered)),
            Some(_) => {}
        }
    }

    /// A different scene collection was opened: start event diffs afresh.
    pub fn collection_changed(&mut self) {
        self.translator.reset();
    }

    /// Clutch is exiting: clients get `ExitStarted`, then the sessions close.
    pub fn shutdown(self) {
        self.server.shutdown();
    }

    /// Settings changed: closes every session (clients reconnect with the new settings).
    pub fn invalidate(self) {
        self.server.invalidate();
    }
}
