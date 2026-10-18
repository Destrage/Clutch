//! The macro runner against a headless engine, with a simulated clock.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use estream_api::{MediaAction, MediaStatus, Output, OutputStatus, Studio};
use estream_automation::focus::Foreground;
use estream_automation::{Host, Runner};
use estream_core::{Rational, SceneId, SourceId};
use estream_engine::{Command, Engine, EngineConfig};
use estream_project::{
    Automation, Chord, GameEventKind, Macro, OutputKind, Project, Scene, SceneItem, SourceDef,
    Step, Toggle, Trigger,
};
use serde_json::json;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

struct FakeHost {
    engine: Engine,
    active: [bool; 4],
    actions: Vec<String>,
    clips: Vec<(Duration, Duration)>,
}

fn index(o: Output) -> usize {
    match o {
        Output::Stream => 0,
        Output::Record => 1,
        Output::ReplayBuffer => 2,
        Output::VirtualCam => 3,
    }
}

impl Studio for FakeHost {
    fn engine(&self) -> Option<&Engine> {
        Some(&self.engine)
    }
    fn collection_name(&self) -> String {
        "Test".into()
    }
    fn record_directory(&self) -> PathBuf {
        PathBuf::new()
    }
    fn studio_mode(&self) -> bool {
        false
    }
    fn set_studio_mode(&mut self, _: bool) {}
    fn preview_scene(&self) -> Option<SceneId> {
        None
    }
    fn set_preview_scene(&mut self, _: SceneId) -> Result<(), String> {
        Ok(())
    }
    fn trigger_studio_transition(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn output_status(&self, output: Output) -> OutputStatus {
        OutputStatus {
            active: self.active[index(output)],
            ..Default::default()
        }
    }
    fn set_output_active(
        &mut self,
        output: Output,
        active: bool,
    ) -> Result<Option<PathBuf>, String> {
        self.active[index(output)] = active;
        Ok(None)
    }
    fn save_replay(&mut self) -> Result<(), String> {
        Err("off".into())
    }
    fn last_replay(&self) -> Option<PathBuf> {
        None
    }
    fn last_recording(&self) -> Option<PathBuf> {
        None
    }
    fn media_status(&self, _: SourceId) -> Option<MediaStatus> {
        None
    }
    fn media_action(&mut self, _: SourceId, _: MediaAction) -> Result<(), String> {
        Ok(())
    }
    fn media_seek(&mut self, _: SourceId, _: Duration) -> Result<(), String> {
        Ok(())
    }
    fn press_input_button(&mut self, _: SourceId, _: &str) -> Result<(), String> {
        Ok(())
    }
}

impl Host for FakeHost {
    fn run_action(&mut self, id: &str) -> bool {
        self.actions.push(id.to_owned());
        id != "nope"
    }
    fn save_clip(&mut self, before: Duration, after: Duration) -> Result<(), String> {
        self.clips.push((before, after));
        Ok(())
    }
}

struct Setup {
    host: FakeHost,
    runner: Runner,
    a: SceneId,
    b: SceneId,
    boxed: SourceId,
    mic: SourceId,
    t0: Instant,
}

/// Scenes A (on program, with "Box") and B; inputs Box and Mic.
fn setup() -> Setup {
    let mut project = Project::new_default(320, 180, Rational::fps(30));
    let canvas = project.canvases[0].id;
    let boxed = SourceDef::new("Box", "color", json!({ "width": 32, "height": 32 }));
    let mic = SourceDef::new("Mic", "audio_test_tone", json!({}));
    project.scenes[0].name = "A".into();
    project.scenes[0].items.push(SceneItem::source(boxed.id));
    let b = Scene::new("B", canvas);
    let (a, b_id) = (project.scenes[0].id, b.id);
    project.scenes.push(b);
    let (boxed_id, mic_id) = (boxed.id, mic.id);
    project.sources.push(boxed);
    project.sources.push(mic);
    let engine = Engine::start(
        project,
        EngineConfig {
            video: false,
            audio: false,
            ..Default::default()
        },
    )
    .unwrap();
    Setup {
        host: FakeHost {
            engine,
            active: [false; 4],
            actions: Vec::new(),
            clips: Vec::new(),
        },
        runner: Runner::new(false),
        a,
        b: b_id,
        boxed: boxed_id,
        mic: mic_id,
        t0: Instant::now(),
    }
}

impl Setup {
    fn set(&self, macros: Vec<Macro>) {
        self.host
            .engine
            .execute(Command::SetAutomation {
                automation: Automation { macros },
            })
            .unwrap();
    }

    fn tick(&mut self, ms: u64) {
        let now = self.t0 + Duration::from_millis(ms);
        self.runner.tick(&mut self.host, now);
    }

    fn program(&self) -> SceneId {
        self.host.engine.snapshot().canvases[0]
            .active_scene
            .unwrap()
    }

    fn muted(&self) -> bool {
        self.host
            .engine
            .snapshot()
            .source(self.mic)
            .unwrap()
            .audio
            .muted
    }
}

fn mac(name: &str, triggers: Vec<Trigger>, steps: Vec<Step>) -> Macro {
    Macro {
        triggers,
        steps,
        ..Macro::new(name)
    }
}

#[test]
fn hotkey_runs_steps_through_the_command_bus() {
    let mut s = setup();
    let chord = Chord::parse("Ctrl+F9").unwrap();
    s.set(vec![mac(
        "BRB",
        vec![Trigger::Hotkey {
            chord: chord.clone(),
        }],
        vec![
            Step::SwitchScene {
                scene: s.b,
                cut: true,
            },
            Step::SourceMute {
                source: s.mic,
                mode: Toggle::On,
            },
            Step::SourceVisible {
                source: s.boxed,
                mode: Toggle::Off,
            },
            Step::Action {
                id: "studio.toggle".into(),
            },
        ],
    )]);
    s.tick(0);
    // Box is in scene A; hide it while A is still on program.
    s.runner.press(&chord, s.t0);
    s.tick(1);
    assert_eq!(s.program(), s.b);
    assert!(s.muted());
    assert_eq!(s.host.actions, vec!["studio.toggle"]);
    // The box lives in A, which was program until the switch: its visibility step ran after
    // the switch, so it reports an error and the macro carried on.
    let id = s.host.engine.snapshot().automation.macros[0].id;
    assert!(s.runner.last_error(id).is_some());
    // Every change is undoable.
    assert!(s.host.engine.undo().unwrap());
    assert!(!s.muted());
}

#[test]
fn source_visibility_in_the_program_scene() {
    let mut s = setup();
    s.set(vec![mac(
        "Hide box",
        vec![],
        vec![Step::SourceVisible {
            source: s.boxed,
            mode: Toggle::Toggle,
        }],
    )]);
    let id = s.host.engine.snapshot().automation.macros[0].id;
    s.runner.run(&mut s.host, id, s.t0);
    let visible = |s: &Setup| s.host.engine.snapshot().scene(s.a).unwrap().items[0].visible;
    assert!(!visible(&s));
    s.runner.run(&mut s.host, id, s.t0 + Duration::from_secs(1));
    assert!(visible(&s));
}

#[test]
fn waits_are_scheduled_not_slept() {
    let mut s = setup();
    s.set(vec![mac(
        "Intro",
        vec![],
        vec![
            Step::SwitchScene {
                scene: s.b,
                cut: true,
            },
            Step::Wait { ms: 500 },
            Step::SwitchScene {
                scene: s.a,
                cut: true,
            },
        ],
    )]);
    let id = s.host.engine.snapshot().automation.macros[0].id;
    let started = Instant::now();
    s.runner.run(&mut s.host, id, s.t0);
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "run() doesn't block"
    );
    assert_eq!(s.program(), s.b);
    assert!(s.runner.is_running(id));
    s.tick(400);
    assert_eq!(s.program(), s.b);
    s.tick(600);
    assert_eq!(s.program(), s.a);
    assert!(!s.runner.is_running(id));
}

#[test]
fn intervals_repeat() {
    let mut s = setup();
    s.set(vec![mac(
        "Flip",
        vec![Trigger::Interval { seconds: 2 }],
        vec![Step::SourceMute {
            source: s.mic,
            mode: Toggle::Toggle,
        }],
    )]);
    s.tick(0);
    s.tick(1900);
    assert!(!s.muted());
    s.tick(2100);
    assert!(s.muted());
    s.tick(4200);
    assert!(!s.muted());
}

#[test]
fn scene_and_output_events() {
    let mut s = setup();
    s.set(vec![
        mac(
            "On B",
            vec![Trigger::SceneActivated { scene: s.b }],
            vec![Step::SourceMute {
                source: s.mic,
                mode: Toggle::On,
            }],
        ),
        mac(
            "Live",
            vec![Trigger::Output {
                output: OutputKind::Stream,
                started: true,
            }],
            vec![Step::Output {
                output: OutputKind::ReplayBuffer,
                mode: Toggle::On,
            }],
        ),
    ]);
    s.tick(0);
    assert!(!s.muted(), "nothing fires on the first look");
    let canvas = s.host.engine.snapshot().canvases[0].id;
    s.host
        .engine
        .transition_to(canvas, s.b, Some(estream_project::Transition::CUT))
        .unwrap();
    s.tick(100);
    assert!(s.muted());
    s.host.engine.undo().unwrap(); // unmute; B stays on program
    s.tick(200);
    assert!(!s.muted(), "fires once per change, not while B stays live");

    s.host.active[0] = true; // stream started
    s.tick(300);
    assert!(s.host.active[2], "replay buffer started with the stream");
}

#[test]
fn window_focus_fires_on_the_edge() {
    let mut s = setup();
    let windows = Rc::new(RefCell::new(None::<Foreground>));
    {
        let windows = windows.clone();
        s.runner.set_focus_probe(move || windows.borrow().clone());
    }
    s.set(vec![mac(
        "Game",
        vec![Trigger::WindowFocus {
            pattern: "valorant".into(),
        }],
        vec![Step::SourceMute {
            source: s.mic,
            mode: Toggle::Toggle,
        }],
    )]);
    s.tick(0);
    *windows.borrow_mut() = Some(Foreground {
        program: "VALORANT.exe".into(),
        title: "VALORANT".into(),
    });
    s.tick(300);
    assert!(s.muted());
    s.tick(600);
    s.tick(900);
    assert!(s.muted(), "staying focused doesn't refire");
    *windows.borrow_mut() = Some(Foreground {
        program: "chrome.exe".into(),
        title: "News".into(),
    });
    s.tick(1200);
    *windows.borrow_mut() = Some(Foreground {
        program: "VALORANT.exe".into(),
        title: "VALORANT".into(),
    });
    s.tick(1500);
    assert!(!s.muted(), "coming back fires again");
}

#[test]
fn no_reentry_no_loops_and_disabled_macros() {
    let mut s = setup();
    let chord = Chord::parse("F8").unwrap();
    let mut slow = mac(
        "Slow",
        vec![Trigger::Hotkey {
            chord: chord.clone(),
        }],
        vec![Step::Action { id: "a".into() }, Step::Wait { ms: 10_000 }],
    );
    let fast = mac(
        "Fast",
        vec![Trigger::Hotkey {
            chord: chord.clone(),
        }],
        vec![Step::Action { id: "b".into() }],
    );
    let mut off = mac(
        "Off",
        vec![Trigger::Hotkey {
            chord: chord.clone(),
        }],
        vec![Step::Action { id: "c".into() }],
    );
    off.enabled = false;
    slow.enabled = true;
    s.set(vec![slow, fast, off]);
    s.tick(0);
    for i in 0..10 {
        s.runner.press(&chord, s.t0 + Duration::from_millis(i * 10));
        s.tick(i * 10 + 5);
    }
    let count = |host: &FakeHost, id: &str| host.actions.iter().filter(|a| *a == id).count();
    assert_eq!(count(&s.host, "a"), 1, "a running macro isn't restarted");
    assert_eq!(count(&s.host, "b"), 4, "at most 4 starts a second");
    assert_eq!(count(&s.host, "c"), 0, "disabled macros don't trigger");
    s.runner
        .run_by_name(&mut s.host, "Off", s.t0 + Duration::from_secs(5))
        .unwrap();
    assert_eq!(count(&s.host, "c"), 1, "but can be run by hand");
    assert!(
        s.runner
            .run_by_name(&mut s.host, "Missing", s.t0 + Duration::from_secs(5))
            .is_err()
    );
}

#[test]
fn game_events_fire_matching_macros() {
    let mut s = setup();
    s.set(vec![
        mac(
            "Any kill",
            vec![Trigger::GameEvent {
                game: None,
                event: GameEventKind::Kill,
            }],
            vec![Step::Action { id: "kill".into() }],
        ),
        mac(
            "CS2 multi-kill",
            vec![Trigger::GameEvent {
                game: Some("Counter-Strike 2".into()),
                event: GameEventKind::MultiKill,
            }],
            vec![Step::Action { id: "multi".into() }],
        ),
    ]);
    s.tick(0);
    s.runner.game_event("Dota 2", GameEventKind::Kill, s.t0);
    s.tick(1);
    assert_eq!(s.host.actions, ["kill"]);
    // Another game's multi-kill doesn't match the CS2-only macro.
    s.runner
        .game_event("Dota 2", GameEventKind::MultiKill, s.t0);
    s.tick(2);
    assert_eq!(s.host.actions, ["kill"]);
    // An ace counts as a multi-kill; game names match case-insensitively.
    s.runner.game_event(
        "counter-strike 2",
        GameEventKind::Ace,
        s.t0 + Duration::from_secs(2),
    );
    s.tick(2001);
    assert_eq!(s.host.actions, ["kill", "multi"]);
}

#[test]
fn save_clip_step_asks_the_host_for_a_clip() {
    let mut s = setup();
    s.set(vec![mac(
        "Clip it",
        vec![],
        vec![Step::SaveClip {
            before: 20,
            after: 5,
        }],
    )]);
    s.tick(0);
    let id = s.host.engine.snapshot().automation.macros[0].id;
    s.runner.run(&mut s.host, id, s.t0);
    assert_eq!(
        s.host.clips,
        [(Duration::from_secs(20), Duration::from_secs(5))]
    );
    let mut with_trigger = s.host.engine.snapshot().automation.macros[0].clone();
    with_trigger.triggers.push(Trigger::GameEvent {
        game: Some("Valorant".into()),
        event: GameEventKind::Hype,
    });
    let summary = estream_automation::trigger_summary(&with_trigger, &s.host.engine.snapshot());
    assert_eq!(summary, "On hype in Valorant");
}
