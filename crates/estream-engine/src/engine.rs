//! The engine: owns the project on a control thread, applies commands from the
//! bus, publishes snapshots/events, autosaves, and drives the video thread.

use crate::audio::{self, AudioHost};
use crate::command::{Change, Command, CommandError};
use crate::feed::{VideoConsumer, VideoFrame};
use crate::graph::{ActiveTransition, RenderGraph, ViewState};
use crate::history::History;
use crate::source::{SourceRegistry, SourceStatus};
use crate::video::{self, CanvasOutput, SourceInfo, VideoCore, VideoSink};
use arc_swap::ArcSwap;
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TrySendError};
use estream_audio::mixer::MeterSet;
use estream_audio::{AudioCore, SourceFactory, TrackSink};
use estream_core::{CanvasId, EngineClock, SceneId, SourceId};
use estream_gpu::{GpuContext, GpuError, GpuOptions};
use estream_project::{PersistError, Project, Transition, TransitionKind, persist};
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
    /// Run the real-time audio thread.
    pub audio: bool,
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
            audio: true,
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
    /// Studio-mode Preview renders, per canvas.
    previews: ArcSwap<Vec<Arc<CanvasOutput>>>,
    /// Merged video + audio source information.
    statuses: ArcSwap<HashMap<SourceId, SourceInfo>>,
    video_infos: ArcSwap<HashMap<SourceId, SourceInfo>>,
    audio_statuses: ArcSwap<HashMap<SourceId, SourceStatus>>,
    audio_meters: ArcSwap<MeterSet>,
    consumers: ArcSwap<Vec<VideoConsumer>>,
    next_consumer: AtomicU64,
    subscribers: Mutex<Vec<Subscriber>>,
    shutdown: AtomicBool,
    can_undo: AtomicBool,
    can_redo: AtomicBool,
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

impl Shared {
    /// Video information, plus status for audio-only sources (no video part).
    fn merge_statuses(&self) {
        let mut merged: HashMap<SourceId, SourceInfo> = (**self.video_infos.load()).clone();
        for (id, status) in self.audio_statuses.load().iter() {
            merged.entry(*id).or_insert_with(|| SourceInfo {
                status: status.clone(),
                size: None,
            });
        }
        self.statuses.store(Arc::new(merged));
    }
}

impl AudioHost for Shared {
    fn graph(&self) -> Arc<RenderGraph> {
        self.graph.load_full()
    }

    fn publish_audio(&self, meters: MeterSet, statuses: HashMap<SourceId, SourceStatus>) {
        self.audio_meters.store(Arc::new(meters));
        self.audio_statuses.store(Arc::new(statuses));
        self.merge_statuses();
    }

    fn shutdown_requested(&self) -> &AtomicBool {
        &self.shutdown
    }
}

impl VideoSink for Shared {
    fn consumers(&self) -> Arc<Vec<VideoConsumer>> {
        self.consumers.load_full()
    }

    fn graph(&self) -> Arc<RenderGraph> {
        self.graph.load_full()
    }

    fn publish(&self, outputs: Vec<Arc<CanvasOutput>>, statuses: HashMap<SourceId, SourceInfo>) {
        self.outputs.store(Arc::new(outputs));
        self.video_infos.store(Arc::new(statuses));
        self.merge_statuses();
    }

    fn publish_previews(&self, previews: Vec<Arc<CanvasOutput>>) {
        self.previews.store(Arc::new(previews));
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
    TransitionTo {
        canvas: CanvasId,
        scene: SceneId,
        transition: Option<Transition>,
        reply: Reply<u64>,
    },
    SetPreview {
        canvas: CanvasId,
        scene: Option<SceneId>,
    },
    Save {
        path: Option<PathBuf>,
        reply: Reply<PathBuf>,
    },
    Shutdown,
}

/// Handle to a running engine. Dropping it shuts the engine down cleanly
/// (saving the collection if a path is configured).
/// Attaches and detaches video consumers without borrowing the [`Engine`].
#[derive(Clone)]
pub struct VideoTap {
    shared: Arc<Shared>,
}

impl VideoTap {
    /// Delivers NV12 frames of `canvas` to `tx` until [`VideoTap::remove`] is called.
    pub fn add(&self, canvas: CanvasId, tx: crossbeam_channel::Sender<VideoFrame>) -> u64 {
        let id = self.shared.next_consumer.fetch_add(1, Ordering::Relaxed);
        self.shared.consumers.rcu(|list| {
            let mut list = (**list).clone();
            list.push(VideoConsumer {
                id,
                canvas,
                tx: tx.clone(),
            });
            list
        });
        id
    }

    pub fn remove(&self, id: u64) {
        self.shared.consumers.rcu(|list| {
            list.iter()
                .filter(|c| c.id != id)
                .cloned()
                .collect::<Vec<_>>()
        });
    }
}

pub struct Engine {
    requests: Sender<Request>,
    shared: Arc<Shared>,
    gpu: Option<GpuContext>,
    registry: SourceRegistry,
    clock: EngineClock,
    control: Option<JoinHandle<()>>,
    video: Option<JoinHandle<()>>,
    audio: Option<JoinHandle<()>>,
    track_sinks: Option<crossbeam_channel::Sender<Box<dyn TrackSink>>>,
}

impl Engine {
    pub fn start(project: Project, config: EngineConfig) -> Result<Self, EngineError> {
        let clock = EngineClock::new();
        let shared = Arc::new(Shared {
            graph: ArcSwap::from_pointee(RenderGraph::build(&project, 0)),
            snapshot: ArcSwap::from_pointee(project.clone()),
            revision: AtomicU64::new(0),
            outputs: ArcSwap::from_pointee(Vec::new()),
            previews: ArcSwap::from_pointee(Vec::new()),
            statuses: ArcSwap::from_pointee(HashMap::new()),
            video_infos: ArcSwap::from_pointee(HashMap::new()),
            audio_statuses: ArcSwap::from_pointee(HashMap::new()),
            audio_meters: ArcSwap::from_pointee(MeterSet::default()),
            consumers: ArcSwap::from_pointee(Vec::new()),
            next_consumer: AtomicU64::new(1),
            subscribers: Mutex::new(Vec::new()),
            shutdown: AtomicBool::new(false),
            can_undo: AtomicBool::new(false),
            can_redo: AtomicBool::new(false),
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

        let (track_sinks, audio) = if config.audio {
            let registry = config.registry.clone();
            let factory: SourceFactory =
                Arc::new(move |kind, settings| registry.create_audio(kind, settings));
            let mut core = AudioCore::new(factory);
            core.sync(&shared.graph.load().audio);
            shared.publish_audio(core.meters(), core.statuses());
            let (tx, rx) = crossbeam_channel::unbounded();
            let host: Arc<dyn AudioHost> = shared.clone();
            let handle = std::thread::Builder::new()
                .name("estream-audio".into())
                .spawn(move || audio::run(core, clock, host, rx))?;
            (Some(tx), Some(handle))
        } else {
            (None, None)
        };

        let (requests, rx) = crossbeam_channel::bounded(1024);
        let mut control = Control {
            registry: config.registry.clone(),
            project,
            history: History::new(config.history_limit, config.coalesce_window),
            shared: shared.clone(),
            path: config.collection_path,
            autosave_delay: config.autosave_delay,
            dirty_since_save: false,
            autosave_at: None,
            view: ViewState::default(),
            clock,
            generation: 0,
            starting: None,
        };
        let control = std::thread::Builder::new()
            .name("estream-engine".into())
            .spawn(move || control.run(rx))?;

        tracing::info!("engine started");
        Ok(Self {
            requests,
            shared,
            gpu,
            registry: config.registry,
            clock,
            control: Some(control),
            video,
            audio,
            track_sinks,
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

    /// Makes `scene` the program scene of `canvas` with a transition (the project's
    /// transition if `None`). Returns the new revision.
    pub fn transition_to(
        &self,
        canvas: CanvasId,
        scene: SceneId,
        transition: Option<Transition>,
    ) -> Result<u64, EngineError> {
        self.request(|reply| Request::TransitionTo {
            canvas,
            scene,
            transition,
            reply,
        })
    }

    /// Shows `scene` in the studio-mode Preview of `canvas`; `None` turns the Preview off.
    pub fn set_preview_scene(&self, canvas: CanvasId, scene: Option<SceneId>) {
        let _ = self.requests.send(Request::SetPreview { canvas, scene });
    }

    /// True while a program transition is running on `canvas`.
    pub fn is_transitioning(&self, canvas: CanvasId) -> bool {
        let now = self.clock.now();
        self.shared.graph.load().canvases.iter().any(|c| {
            c.id == canvas
                && c.transition
                    .as_ref()
                    .is_some_and(|t| t.progress(now).is_some())
        })
    }

    /// The studio-mode Preview render of `canvas`, while studio mode is on.
    pub fn preview_output(&self, canvas: CanvasId) -> Option<Arc<CanvasOutput>> {
        self.shared
            .previews
            .load()
            .iter()
            .find(|o| o.id == canvas)
            .cloned()
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

    /// Whether there is a step to undo (updated before change events are sent).
    pub fn can_undo(&self) -> bool {
        self.shared.can_undo.load(Ordering::Acquire)
    }

    pub fn can_redo(&self) -> bool {
        self.shared.can_redo.load(Ordering::Acquire)
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
        self.shared
            .statuses
            .load()
            .get(&source)
            .map(|i| i.status.clone())
    }

    /// Size of the source's latest frame, if it has produced one.
    pub fn source_size(&self, source: SourceId) -> Option<estream_core::Size> {
        self.shared
            .statuses
            .load()
            .get(&source)
            .and_then(|i| i.size)
    }

    /// Status and size of every source.
    /// Delivers NV12 frames of `canvas` to `tx` (bounded; full = frame dropped). Returns an id.
    pub fn add_video_consumer(
        &self,
        canvas: CanvasId,
        tx: crossbeam_channel::Sender<VideoFrame>,
    ) -> u64 {
        self.video_tap().add(canvas, tx)
    }

    pub fn remove_video_consumer(&self, id: u64) {
        self.video_tap().remove(id);
    }

    /// A handle other threads can keep to attach and detach video consumers later (e.g. an
    /// output that only wants frames while someone is watching).
    pub fn video_tap(&self) -> VideoTap {
        VideoTap {
            shared: self.shared.clone(),
        }
    }

    /// Level meters for the sources being mixed and the master mix.
    pub fn audio_meters(&self) -> Arc<MeterSet> {
        self.shared.audio_meters.load_full()
    }

    /// Receives every mixed audio block (all six tracks). Used by outputs.
    pub fn add_track_sink(&self, sink: Box<dyn TrackSink>) {
        if let Some(tx) = &self.track_sinks {
            let _ = tx.send(sink);
        }
    }

    pub fn source_infos(&self) -> Arc<HashMap<SourceId, SourceInfo>> {
        self.shared.statuses.load_full()
    }

    /// The source types this engine can create (for schemas and pickers).
    pub fn registry(&self) -> &SourceRegistry {
        &self.registry
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
        if let Some(handle) = self.audio.take() {
            let _ = handle.join();
        }
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
    registry: SourceRegistry,
    project: Project,
    history: History,
    shared: Arc<Shared>,
    path: Option<PathBuf>,
    autosave_delay: Duration,
    dirty_since_save: bool,
    autosave_at: Option<Instant>,
    /// Studio Preview scenes and running transitions (not saved).
    view: ViewState,
    clock: EngineClock,
    /// Render graph generation (bumped on project and view changes).
    generation: u64,
    /// Canvas whose transition `transition_to` is starting right now.
    starting: Option<CanvasId>,
}

impl Control {
    /// When the earliest running transition ends, as an `Instant`.
    fn transition_deadline(&self) -> Option<Instant> {
        let now = self.clock.now();
        self.view
            .transitions
            .values()
            .map(|t| Instant::now() + Duration::from_nanos(t.end().0.saturating_sub(now.0)))
            .min()
    }

    fn run(&mut self, rx: Receiver<Request>) {
        loop {
            let deadline = match (self.autosave_at, self.transition_deadline()) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            let request = match deadline {
                Some(at) => match rx.recv_deadline(at) {
                    Ok(r) => r,
                    Err(RecvTimeoutError::Timeout) => {
                        self.expire_transitions();
                        if self.autosave_at.is_some_and(|a| a <= Instant::now()) {
                            self.autosave();
                        }
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
                Request::TransitionTo {
                    canvas,
                    scene,
                    transition,
                    reply,
                } => {
                    let _ = reply.send(self.transition_to(canvas, scene, transition));
                }
                Request::SetPreview { canvas, scene } => {
                    let changed = match scene {
                        Some(scene) => self.view.preview.insert(canvas, scene) != Some(scene),
                        None => self.view.preview.remove(&canvas).is_some(),
                    };
                    if changed {
                        self.publish_graph();
                    }
                }
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
        self.validate(&command)?;
        let key = command.coalesce_key();
        let applied = command.apply(&mut self.project)?;
        self.history.record(key, applied.inverse, Instant::now());
        Ok(self.publish(applied.changes, Origin::Command))
    }

    /// Validates source settings against their type's schema.
    fn validate(&self, command: &Command) -> Result<(), CommandError> {
        let check = |kind: &str, settings: &serde_json::Value| {
            self.registry
                .validate(kind, settings)
                .map_err(CommandError::Invalid)
        };
        match command {
            Command::AddSource { source, .. } => check(&source.kind, &source.settings),
            Command::SetSourceSettings { source, settings } => match self.project.source(*source) {
                Some(def) => check(&def.kind, settings),
                None => Ok(()),
            },
            Command::Batch { commands } => commands.iter().try_for_each(|c| self.validate(c)),
            Command::SetSourceFilters { filters, .. } => filters.iter().try_for_each(|f| {
                let info = estream_gpu::filter::kind(&f.kind)
                    .ok_or_else(|| CommandError::Invalid(format!("unknown filter '{}'", f.kind)))?;
                match f.settings.get("code").and_then(|c| c.as_str()) {
                    Some(code) if info.code => {
                        estream_gpu::validate_custom_filter(code).map_err(CommandError::Invalid)
                    }
                    _ => Ok(()),
                }
            }),
            Command::SetTransition { transition } => match &transition.kind {
                TransitionKind::Shader { code } => {
                    estream_gpu::validate_custom_transition(code).map_err(CommandError::Invalid)
                }
                _ => Ok(()),
            },
            _ => Ok(()),
        }
    }

    /// Switches the program scene, recording a transition the video thread renders.
    fn transition_to(
        &mut self,
        canvas: CanvasId,
        scene: SceneId,
        transition: Option<Transition>,
    ) -> Result<u64, EngineError> {
        let from = self
            .project
            .canvas(canvas)
            .ok_or(CommandError::NoCanvas(canvas))?
            .active_scene;
        if from == Some(scene) {
            return Ok(self.shared.revision.load(Ordering::Acquire));
        }
        let transition = transition.unwrap_or_else(|| self.project.transition.clone());
        let previous = self.view.transitions.remove(&canvas);
        if !transition.is_instant() {
            self.view.transitions.insert(
                canvas,
                ActiveTransition {
                    from,
                    transition,
                    start: self.clock.now(),
                },
            );
        }
        self.starting = Some(canvas);
        let result = self.execute(Command::SetActiveScene {
            canvas,
            scene: Some(scene),
        });
        self.starting = None;
        if result.is_err() {
            // Nothing changed: restore the previous view state.
            self.view.transitions.remove(&canvas);
            if let Some(previous) = previous {
                self.view.transitions.insert(canvas, previous);
            }
        }
        result
    }

    /// Drops finished transitions so the graph (and audio mix) no longer include them.
    fn expire_transitions(&mut self) {
        let now = self.clock.now();
        let before = self.view.transitions.len();
        self.view.transitions.retain(|_, t| t.end() > now);
        if self.view.transitions.len() != before {
            self.publish_graph();
        }
    }

    /// Rebuilds and publishes the render graph (project + view state).
    fn publish_graph(&mut self) {
        self.generation += 1;
        self.shared.graph.store(Arc::new(RenderGraph::build_with(
            &self.project,
            self.generation,
            &self.view,
            self.clock.now(),
        )));
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
        self.shared
            .can_undo
            .store(self.history.can_undo(), Ordering::Release);
        self.shared
            .can_redo
            .store(self.history.can_redo(), Ordering::Release);
        let revision = self.shared.revision.fetch_add(1, Ordering::AcqRel) + 1;
        self.shared.snapshot.store(Arc::new(self.project.clone()));
        // A program change that isn't the transition just started (plain command, undo)
        // is a cut: drop any transition running on that canvas.
        for change in &changes {
            if let Change::ActiveSceneChanged { canvas, .. } = change
                && self.starting != Some(*canvas)
            {
                self.view.transitions.remove(canvas);
            }
        }
        // Previews of scenes that no longer exist are dropped.
        let project = &self.project;
        self.view
            .preview
            .retain(|_, scene| project.scene(*scene).is_some());
        self.publish_graph();
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
