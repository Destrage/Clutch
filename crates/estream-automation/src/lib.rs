//! Automation runtime: watches triggers and runs macros (see `estream_project::automation`).
//!
//! [`Runner::tick`] is called from the app's UI tick. It never blocks: `Wait` steps schedule a
//! wake-up time. Steps act through the same app surface as the remote API ([`Host`]), so every
//! change goes through the command bus or the app's own actions.

pub mod focus;
pub mod hotkeys;

use estream_api::{Output, Studio};
use estream_core::{CanvasId, MacroId, SceneId};
use estream_engine::Command;
use estream_project::{Automation, Chord, Macro, OutputKind, Project, Step, Transition, Trigger};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The app as macros see it.
pub trait Host: Studio {
    /// Runs a command palette action by id; false if there's no such action.
    fn run_action(&mut self, id: &str) -> bool;
}

/// How often the foreground window is checked.
const FOCUS_EVERY: Duration = Duration::from_millis(250);
/// At most this many starts of one macro per second (damps trigger loops).
const MAX_STARTS_PER_SECOND: usize = 4;

pub fn output_of(kind: OutputKind) -> Output {
    match kind {
        OutputKind::Stream => Output::Stream,
        OutputKind::Record => Output::Record,
        OutputKind::ReplayBuffer => Output::ReplayBuffer,
        OutputKind::VirtualCamera => Output::VirtualCam,
    }
}

struct Run {
    id: MacroId,
    step: usize,
    wake: Instant,
}

/// Watches triggers and runs macros.
pub struct Runner {
    #[cfg(windows)]
    hotkeys: Option<hotkeys::HotkeyListener>,
    /// Hotkey registration id → macro.
    hotkey_ids: HashMap<u32, MacroId>,
    conflicts: Vec<Chord>,
    /// The automation the triggers were set up for.
    current: Option<Arc<Project>>,
    automation: Automation,
    running: Vec<Run>,
    starts: HashMap<MacroId, VecDeque<Instant>>,
    errors: HashMap<MacroId, String>,
    focus: Option<focus::Foreground>,
    focus_at: Option<Instant>,
    probe: Box<dyn FnMut() -> Option<focus::Foreground>>,
    intervals: HashMap<(MacroId, usize), Instant>,
    programs: Option<HashMap<CanvasId, Option<SceneId>>>,
    outputs: Option<HashMap<OutputKind, bool>>,
}

impl Runner {
    /// `global_hotkeys`: register hotkeys with the system (off in tests).
    pub fn new(global_hotkeys: bool) -> Self {
        Self {
            #[cfg(windows)]
            hotkeys: global_hotkeys
                .then(hotkeys::HotkeyListener::start)
                .flatten(),
            hotkey_ids: HashMap::new(),
            conflicts: Vec::new(),
            current: None,
            automation: Automation::default(),
            running: Vec::new(),
            starts: HashMap::new(),
            errors: HashMap::new(),
            focus: None,
            focus_at: None,
            probe: Box::new(focus::foreground),
            intervals: HashMap::new(),
            programs: None,
            outputs: None,
        }
    }

    /// Replaces the foreground-window probe (tests).
    pub fn set_focus_probe(&mut self, probe: impl FnMut() -> Option<focus::Foreground> + 'static) {
        self.probe = Box::new(probe);
    }

    /// Hotkeys another program already holds.
    pub fn conflicts(&self) -> &[Chord] {
        &self.conflicts
    }

    /// The last error of a macro's steps, if any.
    pub fn last_error(&self, id: MacroId) -> Option<&str> {
        self.errors.get(&id).map(String::as_str)
    }

    pub fn is_running(&self, id: MacroId) -> bool {
        self.running.iter().any(|r| r.id == id)
    }

    /// Runs a macro now (Run now, remote `TriggerHotkeyByName`); disabled macros run too.
    pub fn run(&mut self, host: &mut dyn Host, id: MacroId, now: Instant) {
        self.sync(host, now);
        if self.automation.find(id).is_some() {
            self.start(id, now, true);
            self.advance(host, now);
        }
    }

    /// Runs the macro with this name (or id).
    pub fn run_by_name(
        &mut self,
        host: &mut dyn Host,
        name: &str,
        now: Instant,
    ) -> Result<(), String> {
        self.sync(host, now);
        let id = self
            .automation
            .macros
            .iter()
            .find(|m| m.name == name || m.id.to_string() == name)
            .map(|m| m.id)
            .ok_or_else(|| format!("no macro named “{name}”"))?;
        self.run(host, id, now);
        Ok(())
    }

    /// Fires the macros bound to `chord` (in-app key presses, tests).
    pub fn press(&mut self, chord: &Chord, now: Instant) {
        let ids: Vec<MacroId> = self
            .automation
            .macros
            .iter()
            .filter(|m| {
                m.triggers
                    .iter()
                    .any(|t| matches!(t, Trigger::Hotkey { chord: c } if c == chord))
            })
            .map(|m| m.id)
            .collect();
        for id in ids {
            self.start(id, now, false);
        }
    }

    pub fn tick(&mut self, host: &mut dyn Host, now: Instant) {
        self.sync(host, now);
        self.collect_triggers(host, now);
        self.advance(host, now);
    }

    /// Picks up automation edits (and collection switches).
    fn sync(&mut self, host: &dyn Host, now: Instant) {
        let Some(engine) = host.engine() else {
            self.current = None;
            return;
        };
        let snapshot = engine.snapshot();
        if self
            .current
            .as_ref()
            .is_some_and(|c| Arc::ptr_eq(c, &snapshot))
        {
            return;
        }
        let changed = snapshot.automation != self.automation;
        self.current = Some(snapshot.clone());
        if !changed {
            return;
        }
        self.automation = snapshot.automation.clone();
        self.running
            .retain(|r| self.automation.find(r.id).is_some());
        self.intervals
            .retain(|(id, _), _| self.automation.find(*id).is_some());
        for m in &self.automation.macros {
            for (i, t) in m.triggers.iter().enumerate() {
                if let Trigger::Interval { seconds } = t {
                    self.intervals
                        .entry((m.id, i))
                        .or_insert(now + Duration::from_secs((*seconds).max(1) as u64));
                }
            }
        }
        self.register_hotkeys();
    }

    fn register_hotkeys(&mut self) {
        self.hotkey_ids.clear();
        let mut list = Vec::new();
        for m in self.automation.macros.iter().filter(|m| m.enabled) {
            for t in &m.triggers {
                if let Trigger::Hotkey { chord } = t
                    && !chord.is_empty()
                {
                    let id = list.len() as u32 + 1;
                    self.hotkey_ids.insert(id, m.id);
                    list.push((id, chord.clone()));
                }
            }
        }
        self.conflicts.clear();
        #[cfg(windows)]
        if let Some(listener) = &self.hotkeys {
            let failed = listener.set(list.clone());
            self.conflicts = list
                .into_iter()
                .filter(|(id, _)| failed.contains(id))
                .map(|(_, c)| c)
                .collect();
        }
        #[cfg(not(windows))]
        let _ = list;
    }

    fn collect_triggers(&mut self, host: &dyn Host, now: Instant) {
        let mut fired: Vec<MacroId> = Vec::new();

        #[cfg(windows)]
        if let Some(listener) = &self.hotkeys {
            while let Ok(id) = listener.presses.try_recv() {
                if let Some(m) = self.hotkey_ids.get(&id) {
                    fired.push(*m);
                }
            }
        }

        let wants_focus = self.automation.macros.iter().any(|m| {
            m.enabled
                && m.triggers
                    .iter()
                    .any(|t| matches!(t, Trigger::WindowFocus { .. }))
        });
        if wants_focus
            && self
                .focus_at
                .is_none_or(|t| now.duration_since(t) >= FOCUS_EVERY)
        {
            self.focus_at = Some(now);
            let current = (self.probe)();
            if current != self.focus {
                let before = self.focus.take();
                for m in &self.automation.macros {
                    let hit = m.triggers.iter().any(|t| match t {
                        Trigger::WindowFocus { pattern } => {
                            current.as_ref().is_some_and(|f| f.matches(pattern))
                                && !before.as_ref().is_some_and(|f| f.matches(pattern))
                        }
                        _ => false,
                    });
                    if hit {
                        fired.push(m.id);
                    }
                }
                self.focus = current;
            }
        }

        for ((id, _), due) in self.intervals.iter_mut() {
            if now >= *due {
                fired.push(*id);
                let period = self
                    .automation
                    .find(*id)
                    .and_then(|m| {
                        m.triggers.iter().find_map(|t| match t {
                            Trigger::Interval { seconds } => Some(*seconds),
                            _ => None,
                        })
                    })
                    .unwrap_or(60)
                    .max(1);
                *due = now + Duration::from_secs(period as u64);
            }
        }

        if let Some(project) = &self.current {
            let programs: HashMap<CanvasId, Option<SceneId>> = project
                .canvases
                .iter()
                .map(|c| (c.id, c.active_scene))
                .collect();
            if let Some(before) = &self.programs {
                for (canvas, scene) in &programs {
                    if before.get(canvas) != Some(scene)
                        && let Some(scene) = scene
                    {
                        for m in &self.automation.macros {
                            if m.triggers.iter().any(
                                |t| matches!(t, Trigger::SceneActivated { scene: s } if s == scene),
                            ) {
                                fired.push(m.id);
                            }
                        }
                    }
                }
            }
            self.programs = Some(programs);
        }

        let outputs: HashMap<OutputKind, bool> = OutputKind::ALL
            .iter()
            .map(|k| (*k, host.output_status(output_of(*k)).active))
            .collect();
        if let Some(before) = &self.outputs {
            for (kind, active) in &outputs {
                if before.get(kind) != Some(active) {
                    for m in &self.automation.macros {
                        if m.triggers.iter().any(|t| {
                            matches!(t, Trigger::Output { output, started } if output == kind && started == active)
                        }) {
                            fired.push(m.id);
                        }
                    }
                }
            }
        }
        self.outputs = Some(outputs);

        for id in fired {
            self.start(id, now, false);
        }
    }

    /// Starts a macro unless it's disabled (for triggers), already running or looping.
    fn start(&mut self, id: MacroId, now: Instant, manual: bool) {
        let Some(m) = self.automation.find(id) else {
            return;
        };
        if (!m.enabled && !manual) || self.is_running(id) {
            return;
        }
        let starts = self.starts.entry(id).or_default();
        while starts
            .front()
            .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(1))
        {
            starts.pop_front();
        }
        if starts.len() >= MAX_STARTS_PER_SECOND {
            tracing::warn!(name = %m.name, "macro triggered too often; skipped");
            return;
        }
        starts.push_back(now);
        tracing::info!(name = %m.name, "macro started");
        self.errors.remove(&id);
        self.running.push(Run {
            id,
            step: 0,
            wake: now,
        });
    }

    /// Runs steps of every running macro that's due, up to its next wait.
    fn advance(&mut self, host: &mut dyn Host, now: Instant) {
        let mut i = 0;
        while i < self.running.len() {
            if self.running[i].wake > now {
                i += 1;
                continue;
            }
            let Some(m) = self.automation.find(self.running[i].id).cloned() else {
                self.running.remove(i);
                continue;
            };
            let mut waiting = false;
            while let Some(step) = m.steps.get(self.running[i].step) {
                self.running[i].step += 1;
                match step {
                    Step::Wait { ms: 0 } => {}
                    Step::Wait { ms } => {
                        self.running[i].wake = now + Duration::from_millis(*ms as u64);
                        waiting = true;
                        break;
                    }
                    _ => {
                        if let Err(e) = execute(host, step) {
                            tracing::warn!(name = %m.name, error = %e, "macro step failed");
                            self.errors.insert(m.id, e);
                        }
                    }
                }
            }
            if waiting {
                i += 1;
            } else {
                self.running.remove(i);
            }
        }
    }
}

/// Runs one (non-wait) step.
pub fn execute(host: &mut dyn Host, step: &Step) -> Result<(), String> {
    let engine_cmd = |host: &dyn Host, command: Command| -> Result<(), String> {
        let engine = host.engine().ok_or("no scene collection is open")?;
        engine
            .execute(command)
            .map(|_| ())
            .map_err(|e| e.to_string())
    };
    match step {
        Step::SwitchScene { scene, cut } => {
            let engine = host.engine().ok_or("no scene collection is open")?;
            let canvas = engine
                .snapshot()
                .scene(*scene)
                .map(|s| s.canvas)
                .ok_or("the scene no longer exists")?;
            engine
                .transition_to(canvas, *scene, cut.then_some(Transition::CUT))
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
        Step::SourceVisible { source, mode } => {
            let p = host
                .engine()
                .ok_or("no scene collection is open")?
                .snapshot();
            if p.source(*source).is_none() {
                return Err("the source no longer exists".into());
            }
            let mut commands = Vec::new();
            for canvas in &p.canvases {
                let Some(scene) = canvas.active_scene.and_then(|s| p.scene(s)) else {
                    continue;
                };
                scene.walk(&mut |item| {
                    if item.source_id() == Some(*source) {
                        commands.push(Command::UpdateItem {
                            scene: scene.id,
                            item: item.id,
                            patch: estream_engine::ItemPatch {
                                visible: Some(mode.apply(item.visible)),
                                ..Default::default()
                            },
                        });
                    }
                });
            }
            if commands.is_empty() {
                return Err("the source isn't in the program scene".into());
            }
            engine_cmd(host, Command::Batch { commands })
        }
        Step::SourceMute { source, mode } => {
            let p = host
                .engine()
                .ok_or("no scene collection is open")?
                .snapshot();
            let s = p.source(*source).ok_or("the source no longer exists")?;
            let mut audio = s.audio.clone();
            audio.muted = mode.apply(audio.muted);
            engine_cmd(
                host,
                Command::UpdateSourceAudio {
                    source: s.id,
                    audio,
                },
            )
        }
        Step::Output { output, mode } => {
            let o = output_of(*output);
            let current = host.output_status(o).active;
            let want = mode.apply(current);
            if want != current {
                host.set_output_active(o, want)?;
            }
            Ok(())
        }
        Step::SaveReplay => host.save_replay().map(|_| ()),
        Step::StudioTransition => {
            if !host.studio_mode() {
                return Err("studio mode is off".into());
            }
            host.trigger_studio_transition()
        }
        Step::Action { id } => {
            if host.run_action(id) {
                Ok(())
            } else {
                Err(format!("there's no action “{id}”"))
            }
        }
        Step::Wait { .. } => Ok(()),
    }
}

/// A one-line description of a macro's triggers (for lists).
pub fn trigger_summary(m: &Macro, project: &Project) -> String {
    let parts: Vec<String> = m
        .triggers
        .iter()
        .map(|t| match t {
            Trigger::Hotkey { chord } if chord.is_empty() => "Hotkey (not set)".into(),
            Trigger::Hotkey { chord } => chord.to_string(),
            Trigger::WindowFocus { pattern } => format!("When “{pattern}” is focused"),
            Trigger::Interval { seconds } if seconds % 60 == 0 => {
                format!("Every {} min", seconds / 60)
            }
            Trigger::Interval { seconds } => format!("Every {seconds} s"),
            Trigger::SceneActivated { scene } => format!(
                "When “{}” goes live",
                project.scene(*scene).map_or("?", |s| s.name.as_str())
            ),
            Trigger::Output { output, started } => format!(
                "When {} {}",
                output.label().to_lowercase(),
                if *started { "starts" } else { "stops" }
            ),
        })
        .collect();
    if parts.is_empty() {
        "Run manually".into()
    } else {
        parts.join(" · ")
    }
}
