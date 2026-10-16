//! The engine: owns the project on a control thread, applies commands from the
//! bus, publishes snapshots/events, autosaves, and drives the video thread.

use crate::command::{Change, Command, CommandError};
use crate::graph::RenderGraph;
use crate::history::History;
use crate::source::{SourceRegistry, SourceStatus};
use crate::video::{self, CanvasOutput, VideoCore, VideoSink};
use arc_swap::ArcSwap;
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TrySendError};
use estream_core::{CanvasId, EngineClock, SourceId};
use estream_gpu::{GpuContext, GpuError, GpuOptions};
use estream_project::{PersistError, Project, persist};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error(transparent)]
    Command(#[from] CommandError),
    #[error(transparent)]
    Gpu(#[from] GpuError),
    #[error(transparent)]
    Persist(#[from] PersistError),
    #[error("no scene collection path is configured")]
    NoCollectionPath,
    #[error("the engine has stopped")]
    Stopped,
    #[error("failed to start engine thread: {0}")]
    Thread(#[from] std::io::Error),
}

pub struct EngineConfig {
    /// Use an existing GPU device (e.g. shared with the UI). Created headless otherwise.
    pub gpu: Option<GpuContext>,
    pub gpu_options: GpuOptions,
    pub registry: SourceRegistry,
    /// Scene collection file used by `save()` and for the recovery autosave.
    pub collection_path: Option<PathBuf>,
    /// Delay after the last change before the recovery file is written.
    pub autosave_delay: Duration,
    pub history_limit: usize,
    /// Repeated edits to the same property within this window merge into one undo step.
    pub coalesce_window: Duration,
    /// Run the real-time video thread. Disable for pure model tests.
    pub video: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            gpu: None,
            gpu_options: GpuOptions::default(),
            registry: SourceRegistry::with_builtins(),
            collection_path: None,
            autosave_delay: Duration::from_secs(2),
            history_limit: 500,
            coalesce_window: Duration::from_millis(500),
            video: true,
        }
    }
}

/// Why a change happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Command,
    Undo,
    Redo,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Changed {
        revision: u64,
        origin: Origin,
        changes: Arc<[Change]>,
    },
    CommandFailed {
        error: String,
    },
    Saved {
        path: PathBuf,
    },
    RecoverySaved {
        path: PathBuf,
    },
}

/// Event stream for one consumer. Never blocks the engine: if the consumer
/// falls behind, events are dropped and [`Subscription::take_lagged`] reports it.
pub struct Subscription {
    rx: Receiver<Event>,
    lagged: Arc<AtomicBool>,
}

impl Subscription {
    pub fn try_recv(&self) -> Option<Event> {
        self.rx.try_recv().ok()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Option<Event> {
        self.rx.recv_timeout(timeout).ok()
    }

    /// True (once) if events were dropped; re-read the snapshot to resync.
    pub fn take_lagged(&self) -> bool {
        self.lagged.swap(false, Ordering::AcqRel)
    }
}

struct Subscriber {
    tx: Sender<Event>,
    lagged: Arc<AtomicBool>,
}

const SUBSCRIPTION_CAPACITY: usize = 1024;

struct Shared {
    snapshot: ArcSwap<Project>,
    graph: ArcSwap<RenderGraph>,
    revision: AtomicU64,
    outputs: ArcSwap<Vec<Arc<CanvasOutput>>>,
    statuses: ArcSwap<HashMap<SourceId, SourceStatus>>,
    subscribers: Mutex<Vec<Subscriber>>,
    shutdown: AtomicBool,
}

impl Shared {
    fn broadcast(&self, event: Event) {
        let Ok(mut subs) = self.subscribers.lock() else {
            return;
        };
        subs.retain(|s| match s.tx.try_send(event.clone()) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                s.lagged.store(true, Ordering::Release);
                true
            }
            Err(TrySendError::Disconnected(_)) => false,
        });
    }
}

impl VideoSink for Shared {
    fn graph(&self) -> Arc<RenderGraph> {
        self.graph.load_full()
    }

    fn publish(&self, outputs: Vec<Arc<CanvasOutput>>, statuses: HashMap<SourceId, SourceStatus>) {
        self.outputs.store(Arc::new(outputs));
        self.statuses.store(Arc::new(statuses));
    }

    fn shutdown_requested(&self) -> &AtomicBool {
        &self.shutdown
    }
}

type Reply<T> = Sender<Result<T, EngineError>>;

enum Request {
    Execute {
        command: Command,
        reply: Option<Reply<u64>>,
    },
    Undo(Reply<bool>),
    Redo(Reply<bool>),
    Seal,
    Save {
        path: Option<PathBuf>,
        reply: Reply<PathBuf>,
    },
    Shutdown,
}

/// Handle to a running engine. Dropping it shuts the engine down cleanly
/// (saving the collection if a path is configured).
pub struct Engine {
    requests: Sender<Request>,
    shared: Arc<Shared>,
    gpu: Option<GpuContext>,
    clock: EngineClock,
    control: Option<JoinHandle<()>>,
    video: Option<JoinHandle<()>>,
}

impl Engine {
    pub fn start(project: Project, config: EngineConfig) -> Result<Self, EngineError> {
        let clock = EngineClock::new();
        let shared = Arc::new(Shared {
            graph: ArcSwap::from_pointee(RenderGraph::build(&project, 0)),
            snapshot: ArcSwap::from_pointee(project.clone()),
            revision: AtomicU64::new(0),
            outputs: ArcSwap::from_pointee(Vec::new()),
            statuses: ArcSwap::from_pointee(HashMap::new()),
            subscribers: Mutex::new(Vec::new()),
            shutdown: AtomicBool::new(false),
        });

        let gpu = if config.video {
            Some(match config.gpu {
                Some(gpu) => gpu,
                None => GpuContext::new_headless(&config.gpu_options)?,
            })
        } else {
            None
        };

        let video = match &gpu {
            Some(gpu) => {
                let mut core = VideoCore::new(gpu.clone(), config.registry.clone());
                // Create canvases and sources up front so outputs exist when `start` returns.
                core.sync(shared.graph.load_full(), clock.now());
                shared.publish(core.outputs(), core.statuses());
                let sink: Arc<dyn VideoSink> = shared.clone();
                Some(
                    std::thread::Builder::new()
                        .name("estream-video".into())
                        .spawn(move || video::run(core, clock, sink))?,
                )
            }
            None => None,
        };

        let (requests, rx) = crossbeam_channel::bounded(1024);
        let mut control = Control {
            project,
            history: History::new(config.history_limit, config.coalesce_window),
            shared: shared.clone(),
            path: config.collection_path,
            autosave_delay: config.autosave_delay,
            dirty_since_save: false,
            autosave_at: None,
        };
        let control = std::thread::Builder::new()
            .name("estream-engine".into())
            .spawn(move || control.run(rx))?;

        tracing::info!("engine started");
        Ok(Self {
            requests,
            shared,
            gpu,
            clock,
            control: Some(control),
            video,
        })
    }

    fn request<T>(&self, make: impl FnOnce(Reply<T>) -> Request) -> Result<T, EngineError> {
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.requests
            .send(make(tx))
            .map_err(|_| EngineError::Stopped)?;
        rx.recv().map_err(|_| EngineError::Stopped)?
    }

    /// Applies a command and waits for the result. Returns the new revision.
    pub fn execute(&self, command: Command) -> Result<u64, EngineError> {
        self.request(|reply| Request::Execute {
            command,
            reply: Some(reply),
        })
    }

    /// Queues a command without waiting. Failures are reported as [`Event::CommandFailed`].
    pub fn submit(&self, command: Command) -> Result<(), EngineError> {
        self.requests
            .send(Request::Execute {
                command,
                reply: None,
            })
            .map_err(|_| EngineError::Stopped)
    }

    /// Undoes the last step. Returns false if there was nothing to undo.
    pub fn undo(&self) -> Result<bool, EngineError> {
        self.request(Request::Undo)
    }

    pub fn redo(&self) -> Result<bool, EngineError> {
        self.request(Request::Redo)
    }

    /// Ends the current undo coalescing run (call when a drag gesture ends).
    pub fn seal_history(&self) {
        let _ = self.requests.send(Request::Seal);
    }

    /// Saves the collection to the configured path and removes the recovery file.
    pub fn save(&self) -> Result<PathBuf, EngineError> {
        self.request(|reply| Request::Save { path: None, reply })
    }

    /// Saves to `path` and makes it the collection path.
    pub fn save_as(&self, path: PathBuf) -> Result<PathBuf, EngineError> {
        self.request(|reply| Request::Save {
            path: Some(path),
            reply,
        })
    }

    /// The latest immutable project snapshot.
    pub fn snapshot(&self) -> Arc<Project> {
        self.shared.snapshot.load_full()
    }

    pub fn revision(&self) -> u64 {
        self.shared.revision.load(Ordering::Acquire)
    }

    pub fn subscribe(&self) -> Subscription {
        let (tx, rx) = crossbeam_channel::bounded(SUBSCRIPTION_CAPACITY);
        let lagged = Arc::new(AtomicBool::new(false));
        if let Ok(mut subs) = self.shared.subscribers.lock() {
            subs.push(Subscriber {
                tx,
                lagged: lagged.clone(),
            });
        }
        Subscription { rx, lagged }
    }

    pub fn canvas_outputs(&self) -> Arc<Vec<Arc<CanvasOutput>>> {
        self.shared.outputs.load_full()
    }

    pub fn canvas_output(&self, canvas: CanvasId) -> Option<Arc<CanvasOutput>> {
        self.shared
            .outputs
            .load()
            .iter()
            .find(|o| o.id == canvas)
            .cloned()
    }

    pub fn source_status(&self, source: SourceId) -> Option<SourceStatus> {
        self.shared.statuses.load().get(&source).cloned()
    }

    pub fn gpu(&self) -> Option<&GpuContext> {
        self.gpu.as_ref()
    }

    pub fn clock(&self) -> EngineClock {
        self.clock
    }

    /// Stops the engine, saving if a collection path is configured.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        let _ = self.requests.send(Request::Shutdown);
        if let Some(handle) = self.control.take() {
            let _ = handle.join();
        }
        self.shared.shutdown.store(true, Ordering::Release);
        if let Some(handle) = self.video.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop();
        tracing::info!("engine stopped");
    }
}

/// State owned by the control thread.
struct Control {
    project: Project,
    history: History,
    shared: Arc<Shared>,
    path: Option<PathBuf>,
    autosave_delay: Duration,
    dirty_since_save: bool,
    autosave_at: Option<Instant>,
}

impl Control {
    fn run(&mut self, rx: Receiver<Request>) {
        loop {
            let request = match self.autosave_at {
                Some(at) => match rx.recv_deadline(at) {
                    Ok(r) => r,
                    Err(RecvTimeoutError::Timeout) => {
                        self.autosave();
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                },
                None => match rx.recv() {
                    Ok(r) => r,
                    Err(_) => break,
                },
            };
            match request {
                Request::Execute { command, reply } => {
                    let result = self.execute(command);
                    match reply {
                        Some(reply) => {
                            let _ = reply.send(result);
                        }
                        None => {
                            if let Err(e) = result {
                                tracing::warn!(error = %e, "command failed");
                                self.shared.broadcast(Event::CommandFailed {
                                    error: e.to_string(),
                                });
                            }
                        }
                    }
                }
                Request::Undo(reply) => {
                    let result = self.history.undo(&mut self.project);
                    let _ = reply.send(self.finish_history(result, Origin::Undo));
                }
                Request::Redo(reply) => {
                    let result = self.history.redo(&mut self.project);
                    let _ = reply.send(self.finish_history(result, Origin::Redo));
                }
                Request::Seal => self.history.seal(),
                Request::Save { path, reply } => {
                    let _ = reply.send(self.save(path));
                }
                Request::Shutdown => break,
            }
        }
        // Clean shutdown: persist the collection so no recovery prompt appears next time.
        if self.path.is_some()
            && self.dirty_since_save
            && let Err(e) = self.save(None)
        {
            tracing::error!(error = %e, "failed to save on shutdown");
        }
    }

    fn execute(&mut self, command: Command) -> Result<u64, EngineError> {
        let key = command.coalesce_key();
        let applied = command.apply(&mut self.project)?;
        self.history.record(key, applied.inverse, Instant::now());
        Ok(self.publish(applied.changes, Origin::Command))
    }

    fn finish_history(
        &mut self,
        result: Result<Option<crate::command::Applied>, CommandError>,
        origin: Origin,
    ) -> Result<bool, EngineError> {
        match result? {
            Some(applied) => {
                self.publish(applied.changes, origin);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn publish(&mut self, changes: Vec<Change>, origin: Origin) -> u64 {
        let revision = self.shared.revision.fetch_add(1, Ordering::AcqRel) + 1;
        self.shared.snapshot.store(Arc::new(self.project.clone()));
        self.shared
            .graph
            .store(Arc::new(RenderGraph::build(&self.project, revision)));
        self.shared.broadcast(Event::Changed {
            revision,
            origin,
            changes: changes.into(),
        });
        self.dirty_since_save = true;
        if self.path.is_some() {
            self.autosave_at = Some(Instant::now() + self.autosave_delay);
        }
        revision
    }

    fn autosave(&mut self) {
        self.autosave_at = None;
        let Some(path) = &self.path else { return };
        match persist::save_recovery(path, &self.project) {
            Ok(()) => {
                tracing::debug!("recovery file written");
                self.shared.broadcast(Event::RecoverySaved {
                    path: persist::recovery_path(path),
                });
            }
            Err(e) => tracing::error!(error = %e, "autosave failed"),
        }
    }

    fn save(&mut self, path: Option<PathBuf>) -> Result<PathBuf, EngineError> {
        if let Some(path) = path {
            self.path = Some(path);
        }
        let path = self.path.clone().ok_or(EngineError::NoCollectionPath)?;
        persist::save(&path, &self.project)?;
        persist::discard_recovery(&path)?;
        self.dirty_since_save = false;
        self.autosave_at = None;
        tracing::info!(path = %path.display(), "scene collection saved");
        self.shared.broadcast(Event::Saved { path: path.clone() });
        Ok(path)
    }
}
