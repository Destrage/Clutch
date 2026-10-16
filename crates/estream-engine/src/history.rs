//! Undo/redo history with coalescing of rapid repeated edits.

use crate::command::{Applied, CoalesceKey, Command, CommandError};
use estream_project::Project;
use std::time::{Duration, Instant};

#[derive(Debug)]
struct Entry {
    /// Reverts the step.
    undo: Command,
    key: Option<CoalesceKey>,
    at: Instant,
}

#[derive(Debug)]
pub struct History {
    undo: Vec<Entry>,
    /// Commands that re-apply undone steps.
    redo: Vec<Command>,
    limit: usize,
    window: Duration,
    sealed: bool,
}

impl Default for History {
    fn default() -> Self {
        Self::new(500, Duration::from_millis(500))
    }
}

impl History {
    pub fn new(limit: usize, window: Duration) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            limit,
            window,
            sealed: false,
        }
    }

    /// Records a forward edit given its inverse. Clears the redo stack.
    pub fn record(&mut self, key: Option<CoalesceKey>, inverse: Command, now: Instant) {
        self.redo.clear();
        if !self.sealed
            && let (Some(key), Some(top)) = (&key, self.undo.last_mut())
            && top.key.as_ref() == Some(key)
            && now.saturating_duration_since(top.at) <= self.window
        {
            // Keep the oldest inverse: undo returns to the state before the whole gesture.
            top.at = now;
            return;
        }
        self.sealed = false;
        self.undo.push(Entry {
            undo: inverse,
            key,
            at: now,
        });
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
    }

    /// Ends the current coalescing run (e.g. when a drag gesture finishes).
    pub fn seal(&mut self) {
        self.sealed = true;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self, project: &mut Project) -> Result<Option<Applied>, CommandError> {
        let Some(entry) = self.undo.pop() else {
            return Ok(None);
        };
        match entry.undo.clone().apply(project) {
            Ok(applied) => {
                self.redo.push(applied.inverse.clone());
                self.sealed = true;
                Ok(Some(applied))
            }
            Err(e) => {
                self.undo.push(entry);
                Err(e)
            }
        }
    }

    pub fn redo(&mut self, project: &mut Project) -> Result<Option<Applied>, CommandError> {
        let Some(command) = self.redo.pop() else {
            return Ok(None);
        };
        match command.clone().apply(project) {
            Ok(applied) => {
                self.undo.push(Entry {
                    undo: applied.inverse.clone(),
                    key: None,
                    at: Instant::now(),
                });
                self.sealed = true;
                Ok(Some(applied))
            }
            Err(e) => {
                self.redo.push(command);
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::ItemPatch;
    use estream_core::{ItemTransform, Rational};
    use estream_project::{SceneItem, SourceDef};
    use glam::Vec2;

    struct Doc {
        project: Project,
        history: History,
    }

    impl Doc {
        fn exec(&mut self, cmd: Command, now: Instant) {
            let key = cmd.coalesce_key();
            let applied = cmd.apply(&mut self.project).unwrap();
            self.history.record(key, applied.inverse, now);
        }
    }

    fn setup() -> (Doc, estream_core::SceneId, estream_core::ItemId) {
        let mut doc = Doc {
            project: Project::new_default(1280, 720, Rational::fps(30)),
            history: History::default(),
        };
        let scene = doc.project.scenes[0].id;
        let src = SourceDef::new("S", "color", serde_json::Value::Null);
        let item = SceneItem::source(src.id);
        let item_id = item.id;
        let t = Instant::now();
        doc.exec(
            Command::AddSource {
                source: src,
                index: 0,
            },
            t,
        );
        doc.exec(
            Command::AddItem {
                scene,
                parent: None,
                index: 0,
                item,
            },
            t,
        );
        (doc, scene, item_id)
    }

    fn move_to(scene: estream_core::SceneId, item: estream_core::ItemId, x: f32) -> Command {
        Command::UpdateItem {
            scene,
            item,
            patch: ItemPatch {
                transform: Some(ItemTransform {
                    position: Vec2::new(x, 0.0),
                    ..Default::default()
                }),
                ..Default::default()
            },
        }
    }

    fn x_of(doc: &Doc, scene: estream_core::SceneId, item: estream_core::ItemId) -> f32 {
        doc.project
            .scene(scene)
            .unwrap()
            .find_item(item)
            .unwrap()
            .transform
            .position
            .x
    }

    #[test]
    fn drag_coalesces_into_one_undo_step() {
        let (mut doc, scene, item) = setup();
        let t0 = Instant::now();
        for i in 1..=50 {
            doc.exec(
                move_to(scene, item, i as f32 * 10.0),
                t0 + Duration::from_millis(i * 16),
            );
        }
        assert_eq!(x_of(&doc, scene, item), 500.0);
        doc.history.undo(&mut doc.project).unwrap();
        assert_eq!(
            x_of(&doc, scene, item),
            0.0,
            "one undo reverts the whole drag"
        );
        doc.history.redo(&mut doc.project).unwrap();
        assert_eq!(x_of(&doc, scene, item), 500.0);
    }

    #[test]
    fn edits_after_pause_or_seal_are_separate_steps() {
        let (mut doc, scene, item) = setup();
        let t0 = Instant::now();
        doc.exec(move_to(scene, item, 10.0), t0);
        doc.exec(move_to(scene, item, 20.0), t0 + Duration::from_secs(2));
        doc.history.seal();
        doc.exec(move_to(scene, item, 30.0), t0 + Duration::from_millis(2100));
        doc.history.undo(&mut doc.project).unwrap();
        assert_eq!(x_of(&doc, scene, item), 20.0);
        doc.history.undo(&mut doc.project).unwrap();
        assert_eq!(x_of(&doc, scene, item), 10.0);
    }

    #[test]
    fn new_edit_clears_redo() {
        let (mut doc, scene, item) = setup();
        let t0 = Instant::now();
        doc.exec(move_to(scene, item, 10.0), t0);
        doc.history.undo(&mut doc.project).unwrap();
        assert!(doc.history.can_redo());
        doc.exec(
            Command::RenameScene {
                scene,
                name: "X".into(),
            },
            t0 + Duration::from_secs(1),
        );
        assert!(!doc.history.can_redo());
    }

    /// Random command sequences: undoing everything returns to the original project,
    /// and redoing everything returns to the final project.
    #[test]
    fn random_sequences_undo_to_original() {
        for seed in 0..40u64 {
            let mut rng = fastrand::Rng::with_seed(seed);
            let mut doc = Doc {
                project: Project::new_default(1920, 1080, Rational::fps(60)),
                history: History::new(10_000, Duration::ZERO),
            };
            let original = doc.project.clone();
            let mut applied_count = 0;
            for _ in 0..60 {
                if let Some(cmd) = random_command(&doc.project, &mut rng) {
                    let key = cmd.coalesce_key();
                    if let Ok(a) = cmd.apply(&mut doc.project) {
                        doc.history.record(key, a.inverse, Instant::now());
                        doc.history.seal();
                        applied_count += 1;
                    }
                }
            }
            let final_state = doc.project.clone();
            while doc.history.can_undo() {
                doc.history.undo(&mut doc.project).unwrap();
            }
            assert_eq!(
                doc.project, original,
                "seed {seed}, {applied_count} commands"
            );
            while doc.history.can_redo() {
                doc.history.redo(&mut doc.project).unwrap();
            }
            assert_eq!(doc.project, final_state, "seed {seed} redo");
        }
    }

    fn random_command(p: &Project, rng: &mut fastrand::Rng) -> Option<Command> {
        let canvas = p.canvases.first()?.id;
        let scene = if p.scenes.is_empty() {
            None
        } else {
            Some(&p.scenes[rng.usize(..p.scenes.len())])
        };
        let source = if p.sources.is_empty() {
            None
        } else {
            Some(p.sources[rng.usize(..p.sources.len())].id)
        };
        let mut items = Vec::new();
        if let Some(s) = scene {
            s.walk(&mut |i| items.push((i.id, i.is_group())));
        }
        let pick_item =
            |rng: &mut fastrand::Rng| (!items.is_empty()).then(|| items[rng.usize(..items.len())]);
        Some(match rng.u8(..11) {
            0 => Command::AddSource {
                source: SourceDef::new("S", "color", serde_json::json!({ "n": rng.u32(..) })),
                index: rng.usize(..5),
            },
            1 => Command::RemoveSource { source: source? },
            2 => Command::AddScene {
                scene: estream_project::Scene::new("N", canvas),
                index: rng.usize(..5),
            },
            3 => Command::RemoveScene { scene: scene?.id },
            4 => Command::SetActiveScene {
                canvas,
                scene: Some(scene?.id),
            },
            5 | 6 => {
                let item = if rng.bool() {
                    SceneItem::source(source?)
                } else {
                    SceneItem::group("G", vec![SceneItem::source(source?)])
                };
                let parent = pick_item(rng).filter(|(_, g)| *g).map(|(id, _)| id);
                Command::AddItem {
                    scene: scene?.id,
                    parent,
                    index: rng.usize(..4),
                    item,
                }
            }
            7 => Command::RemoveItem {
                scene: scene?.id,
                item: pick_item(rng)?.0,
            },
            8 => {
                let parent = pick_item(rng).filter(|(_, g)| *g).map(|(id, _)| id);
                Command::MoveItem {
                    scene: scene?.id,
                    item: pick_item(rng)?.0,
                    parent,
                    index: rng.usize(..4),
                }
            }
            9 => Command::UpdateItem {
                scene: scene?.id,
                item: pick_item(rng)?.0,
                patch: ItemPatch {
                    visible: Some(rng.bool()),
                    opacity: Some(rng.f32()),
                    ..Default::default()
                },
            },
            _ => Command::SetSourceSettings {
                source: source?,
                settings: serde_json::json!({ "v": rng.u8(..) }),
            },
        })
    }
}
