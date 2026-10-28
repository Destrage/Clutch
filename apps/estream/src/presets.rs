//! Canvas presets for the first-run wizard and hardware encoder hints.

use estream_core::Rational;
use estream_project::{FilterDef, Project, SceneItem, SourceDef};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasPreset {
    pub label: &'static str,
    pub detail: &'static str,
    pub width: u32,
    pub height: u32,
    pub fps: Rational,
}

pub const PRESETS: &[CanvasPreset] = &[
    CanvasPreset {
        label: "1080p · 60 fps",
        detail: "1920 × 1080 — gaming, most streams",
        width: 1920,
        height: 1080,
        fps: Rational::fps(60),
    },
    CanvasPreset {
        label: "1080p · 30 fps",
        detail: "1920 × 1080 — talk shows, lower bandwidth",
        width: 1920,
        height: 1080,
        fps: Rational::fps(30),
    },
    CanvasPreset {
        label: "1440p · 60 fps",
        detail: "2560 × 1440 — high quality recording",
        width: 2560,
        height: 1440,
        fps: Rational::fps(60),
    },
    CanvasPreset {
        label: "720p · 60 fps",
        detail: "1280 × 720 — slower connections",
        width: 1280,
        height: 720,
        fps: Rational::fps(60),
    },
    CanvasPreset {
        label: "Vertical 1080 × 1920 · 30 fps",
        detail: "Shorts, Reels, TikTok",
        width: 1080,
        height: 1920,
        fps: Rational::fps(30),
    },
    CanvasPreset {
        label: "4K · 30 fps",
        detail: "3840 × 2160 — recording",
        width: 3840,
        height: 2160,
        fps: Rational::fps(30),
    },
];

/// What the user wants to do; decides the canvas and the first scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Gaming,
    Camera,
    Recording,
    Vertical,
}

impl Purpose {
    pub const ALL: [Purpose; 4] = [
        Purpose::Gaming,
        Purpose::Camera,
        Purpose::Recording,
        Purpose::Vertical,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Purpose::Gaming => "Streaming games",
            Purpose::Camera => "Talking to camera",
            Purpose::Recording => "Recording videos & tutorials",
            Purpose::Vertical => "Vertical video",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Purpose::Gaming => "Your game, webcam in the corner",
            Purpose::Camera => "Just chatting, podcasts, reactions",
            Purpose::Recording => "Your screen, webcam in the corner",
            Purpose::Vertical => "TikTok, YouTube Shorts, Reels",
        }
    }

    /// The canvas preset this purpose starts with.
    pub fn preset(self) -> CanvasPreset {
        let pick = |w, h, fps| {
            PRESETS
                .iter()
                .copied()
                .find(|p| p.width == w && p.height == h && p.fps == Rational::fps(fps))
                .unwrap_or(PRESETS[0])
        };
        match self {
            Purpose::Gaming | Purpose::Recording => pick(1920, 1080, 60),
            Purpose::Camera => pick(1920, 1080, 30),
            Purpose::Vertical => pick(1080, 1920, 30),
        }
    }

    /// Purposes that are usually streamed live (chat, alerts and break scenes start on).
    fn live(self) -> bool {
        matches!(self, Purpose::Gaming | Purpose::Camera)
    }
}

/// Parts of the first setup the user can leave out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Webcam,
    Game,
    Microphone,
    DesktopAudio,
    Chat,
    Alerts,
    BreakScenes,
}

impl Part {
    pub const ALL: [Part; 7] = [
        Part::Webcam,
        Part::Game,
        Part::Microphone,
        Part::DesktopAudio,
        Part::Chat,
        Part::Alerts,
        Part::BreakScenes,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Part::Webcam => "webcam",
            Part::Game => "game",
            Part::Microphone => "mic",
            Part::DesktopAudio => "desktop",
            Part::Chat => "chat",
            Part::Alerts => "alerts",
            Part::BreakScenes => "breaks",
        }
    }

    pub fn from_key(key: &str) -> Option<Part> {
        Self::ALL.into_iter().find(|p| p.key() == key)
    }

    fn default_for(self, purpose: Purpose) -> bool {
        match self {
            Part::Chat | Part::Alerts | Part::BreakScenes => purpose.live(),
            _ => true,
        }
    }
}

/// What the user ticked or unticked in the wizard; the rest follows the purpose.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Choices([Option<bool>; Part::ALL.len()]);

impl Choices {
    pub fn includes(&self, part: Part, purpose: Purpose) -> bool {
        self.0[part as usize].unwrap_or_else(|| part.default_for(purpose))
    }

    pub fn toggle(&mut self, part: Part, purpose: Purpose) {
        self.0[part as usize] = Some(!self.includes(part, purpose));
    }
}

/// One line of the wizard's "We'll set up" list.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    /// The part this line ticks on and off; `None` when it is always set up.
    pub part: Option<Part>,
    pub on: bool,
}

impl Line {
    pub fn fixed(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            part: None,
            on: true,
        }
    }
}

/// What the first run found on this PC.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Detected {
    /// Primary monitor: (capture id, description).
    pub monitor: Option<(String, String)>,
    /// First webcam: (device id, name).
    pub camera: Option<(String, String)>,
    /// Default microphone name.
    pub mic: Option<String>,
    /// Encoder that will be used, in words.
    pub encoder: String,
}

/// Lines describing what the first setup will contain.
pub fn summary(purpose: Purpose, d: &Detected, choices: &Choices) -> Vec<Line> {
    let has = |part| choices.includes(part, purpose);
    let opt = |part, text: String| Line {
        text,
        part: Some(part),
        on: has(part),
    };
    let mut lines = Vec::new();
    let screen = d
        .monitor
        .as_ref()
        .map_or("your screen".to_owned(), |(_, label)| label.clone());
    if matches!(purpose, Purpose::Camera | Purpose::Vertical) {
        match &d.camera {
            Some((_, name)) => {
                lines.push(opt(Part::Webcam, format!("Webcam: {name}")));
                if !has(Part::Webcam) {
                    lines.push(Line::fixed(format!("Screen instead: {screen}")));
                }
            }
            None => lines.push(Line::fixed(format!(
                "No webcam found, so your screen is used: {screen}"
            ))),
        }
    } else {
        lines.push(Line::fixed(format!("Screen: {screen}")));
        if purpose == Purpose::Gaming && cfg!(windows) {
            lines.push(opt(
                Part::Game,
                "Your game and its sound, whenever one is fullscreen".into(),
            ));
        }
        if let Some((_, name)) = &d.camera {
            lines.push(opt(Part::Webcam, format!("Webcam in the corner: {name}")));
        }
    }
    lines.push(opt(
        Part::Microphone,
        match &d.mic {
            Some(name) => format!("Microphone with noise suppression: {name}"),
            None => "Microphone with noise suppression (none found yet)".into(),
        },
    ));
    lines.push(opt(Part::DesktopAudio, "Desktop audio".into()));
    lines.push(opt(
        Part::Chat,
        "Chat on screen from Twitch, Kick and YouTube".into(),
    ));
    lines.push(opt(
        Part::Alerts,
        "Alerts for follows, subs, raids and more".into(),
    ));
    lines.push(opt(
        Part::BreakScenes,
        "“Starting soon” and “Be right back” scenes".into(),
    ));
    lines.push(Line::fixed(format!("Encoder: {}", d.encoder)));
    lines
}

/// The first scene collection for a purpose, without the parts the user left out.
pub fn build_project(
    purpose: Purpose,
    preset: CanvasPreset,
    d: &Detected,
    choices: &Choices,
) -> Project {
    use estream_core::{Alignment, BoundsMode, ItemTransform, Rgba8};
    use estream_project::Scene;
    use glam::Vec2;
    let has = |part| choices.includes(part, purpose);
    let mut project = Project::new_default(preset.width, preset.height, preset.fps);
    let size = Vec2::new(preset.width as f32, preset.height as f32);
    let fit = |mode: BoundsMode| ItemTransform {
        bounds_mode: mode,
        bounds: size,
        bounds_alignment: Alignment::Center,
        ..Default::default()
    };
    let corner = || {
        let b = size * 0.28;
        ItemTransform {
            position: size - b - Vec2::new(size.x * 0.025, size.y * 0.04),
            bounds_mode: BoundsMode::Fit,
            bounds: b,
            bounds_alignment: Alignment::BottomRight,
            ..Default::default()
        }
    };
    let mut items = Vec::new();
    let mut add = |project: &mut Project,
                   name: &str,
                   kind: &str,
                   settings: serde_json::Value,
                   t: ItemTransform| {
        let source = SourceDef::new(name, kind, settings);
        let mut item = SceneItem::source(source.id);
        item.transform = t;
        items.push(item);
        project.sources.push(source);
    };
    let screen =
        |project: &mut Project,
         add: &mut dyn FnMut(&mut Project, &str, &str, serde_json::Value, ItemTransform)| {
            let monitor = d
                .monitor
                .as_ref()
                .map_or(String::new(), |(id, _)| id.clone());
            add(
                project,
                "Screen",
                "display_capture",
                serde_json::json!({ "monitor": monitor, "cursor": true }),
                fit(BoundsMode::Fit),
            );
        };
    let camera = |t| {
        d.camera
            .as_ref()
            .filter(|_| has(Part::Webcam))
            .map(|(id, _)| ("Webcam", "camera", serde_json::json!({ "device": id }), t))
    };
    match purpose {
        Purpose::Gaming => {
            project.scenes[0].name = "Gameplay".into();
            screen(&mut project, &mut add);
            // Elsewhere the screen capture shows the game.
            if cfg!(windows) && has(Part::Game) {
                add(
                    &mut project,
                    "Game",
                    "game_capture",
                    // Desktop audio carries the game.
                    serde_json::json!({ "mode": "auto", "cursor": false, "audio": false }),
                    fit(BoundsMode::Fit),
                );
            }
            if let Some((n, k, s, t)) = camera(corner()) {
                add(&mut project, n, k, s, t);
            }
        }
        Purpose::Recording => {
            project.scenes[0].name = "Recording".into();
            screen(&mut project, &mut add);
            if let Some((n, k, s, t)) = camera(corner()) {
                add(&mut project, n, k, s, t);
            }
        }
        Purpose::Camera | Purpose::Vertical => {
            project.scenes[0].name = if purpose == Purpose::Camera {
                "Just chatting".into()
            } else {
                "Vertical".into()
            };
            // Vertical video fills the frame with the camera (cropping the sides).
            let mode = if purpose == Purpose::Vertical {
                BoundsMode::Fill
            } else {
                BoundsMode::Fit
            };
            match camera(fit(mode)) {
                Some((n, k, s, t)) => add(&mut project, n, k, s, t),
                None => screen(&mut project, &mut add),
            }
        }
    }
    project.scenes[0].items = items;

    if has(Part::BreakScenes) {
        let canvas = project.canvases[0].id;
        let background = SourceDef::new(
            "Background",
            "color",
            serde_json::json!({
                "width": preset.width,
                "height": preset.height,
                "color": Rgba8::new(20, 24, 33, 255),
            }),
        );
        // Large text, centred: it is scaled to fit a box in the middle.
        let b = Vec2::new(size.x * 0.6, size.y * 0.18);
        let middle = ItemTransform {
            position: (size - b) * 0.5,
            bounds_mode: BoundsMode::Fit,
            bounds: b,
            bounds_alignment: Alignment::Center,
            ..Default::default()
        };
        for title in ["Starting soon", "Be right back"] {
            let text = SourceDef::new(
                title,
                "text",
                serde_json::json!({ "text": title, "size": 160.0, "align": "center", "shadow": true }),
            );
            let mut backdrop = SceneItem::source(background.id);
            backdrop.transform = fit(BoundsMode::Stretch);
            let mut words = SceneItem::source(text.id);
            words.transform = middle;
            let mut scene = Scene::new(title, canvas);
            scene.items = vec![backdrop, words];
            project.scenes.push(scene);
            project.sources.push(text);
        }
        project.sources.push(background);
    }

    // Overlays on every scene, the break scenes too, over everything else.
    let landscape = size.x >= size.y;
    let mut overlays = Vec::new();
    if has(Part::Chat) {
        // The page keeps its 1080p size and is scaled to the canvas.
        let (page, scale) = if landscape {
            (Vec2::new(500.0, 700.0), size.y / 1080.0)
        } else {
            (Vec2::new(900.0, 640.0), size.x / 1080.0)
        };
        let b = page * scale;
        let position = if landscape {
            Vec2::new(size.x * 0.025, size.y - b.y - size.y * 0.04)
        } else {
            Vec2::new((size.x - b.x) * 0.5, size.y * 0.42)
        };
        let source = SourceDef::new(
            "Chat",
            crate::overlay::chat::KIND,
            serde_json::json!({ "width": page.x as u32, "height": page.y as u32 }),
        );
        overlays.push((
            source.id,
            ItemTransform {
                position,
                bounds_mode: BoundsMode::Fit,
                bounds: b,
                bounds_alignment: Alignment::BottomLeft,
                ..Default::default()
            },
        ));
        project.sources.push(source);
    }
    if has(Part::Alerts) {
        let (w, h) = if landscape {
            (1920, 1080)
        } else {
            (1080, 1920)
        };
        let source = SourceDef::new(
            "Alerts",
            crate::overlay::alerts::KIND,
            serde_json::json!({ "width": w, "height": h }),
        );
        overlays.push((source.id, fit(BoundsMode::Fit)));
        project.sources.push(source);
    }
    for scene in &mut project.scenes {
        for (source, transform) in &overlays {
            let mut item = SceneItem::source(*source);
            item.transform = *transform;
            scene.items.push(item);
        }
    }

    for (name, kind, filters) in [
        ("Desktop Audio", "audio_output", vec![]),
        (
            "Microphone",
            "audio_input",
            vec![FilterDef {
                id: "noise-suppression".into(),
                kind: "noise_suppression".into(),
                enabled: true,
                settings: serde_json::json!({}),
            }],
        ),
    ] {
        let part = if kind == "audio_input" {
            Part::Microphone
        } else {
            Part::DesktopAudio
        };
        if !has(part) {
            continue;
        }
        let mut source = SourceDef::new(name, kind, serde_json::json!({ "device": "" }));
        source.audio.global = true;
        source.audio.filters = filters;
        project.sources.push(source);
    }
    project
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_valid_canvases() {
        for p in PRESETS {
            assert!(p.width % 2 == 0 && p.height % 2 == 0, "{}", p.label);
            assert!(p.fps.is_valid());
        }
    }

    fn detected(camera: bool) -> Detected {
        Detected {
            monitor: Some(("\\\\.\\DISPLAY1".into(), "Display 1 (2560 × 1440)".into())),
            camera: camera.then(|| ("cam-1".into(), "LifeCam".into())),
            mic: Some("G435".into()),
            encoder: "NVIDIA NVENC".into(),
        }
    }

    fn kinds(p: &Project) -> Vec<String> {
        scene_kinds(p, 0)
    }

    fn scene_kinds(p: &Project, scene: usize) -> Vec<String> {
        p.scenes[scene]
            .items
            .iter()
            .filter_map(|i| i.source_id())
            .filter_map(|id| p.source(id))
            .map(|s| s.kind.clone())
            .collect()
    }

    fn build(purpose: Purpose, d: &Detected, choices: &Choices) -> Project {
        build_project(purpose, purpose.preset(), d, choices)
    }

    /// Every item of a scene stays inside the canvas.
    fn inside(p: &Project, scene: usize) {
        let c = &p.canvases[0];
        for item in &p.scenes[scene].items {
            let t = item.transform;
            assert!(t.position.x >= 0.0 && t.position.y >= 0.0, "{t:?}");
            assert!(
                t.position.x + t.bounds.x <= c.width as f32 + 0.5
                    && t.position.y + t.bounds.y <= c.height as f32 + 0.5,
                "{t:?}"
            );
        }
    }

    #[test]
    fn purposes_build_ready_scenes() {
        let d = detected(true);
        let all = Choices::default();
        let g = build(Purpose::Gaming, &d, &all);
        let overlays = ["chat_overlay", "alerts"];
        if cfg!(windows) {
            assert_eq!(
                kinds(&g),
                [
                    &["display_capture", "game_capture", "camera"][..],
                    &overlays
                ]
                .concat()
            );
        } else {
            assert_eq!(
                kinds(&g),
                [&["display_capture", "camera"][..], &overlays].concat()
            );
        }
        assert_eq!(g.canvases[0].fps, Rational::fps(60));
        let mic = g.sources.iter().find(|s| s.kind == "audio_input").unwrap();
        assert!(mic.audio.global);
        assert_eq!(mic.audio.filters[0].kind, "noise_suppression");
        assert!(g.sources.iter().any(|s| s.kind == "audio_output"));
        inside(&g, 0);

        let c = build(Purpose::Camera, &d, &all);
        assert_eq!(kinds(&c), ["camera", "chat_overlay", "alerts"]);
        let no_cam = build(Purpose::Camera, &detected(false), &all);
        assert_eq!(
            kinds(&no_cam)[0],
            "display_capture",
            "falls back to the screen"
        );

        let v = build(Purpose::Vertical, &d, &all);
        assert_eq!((v.canvases[0].width, v.canvases[0].height), (1080, 1920));
        assert_eq!(kinds(&v), ["camera"], "vertical starts without overlays");
        assert_eq!(v.scenes.len(), 1);

        let r = build(Purpose::Recording, &d, &all);
        assert_eq!(kinds(&r), ["display_capture", "camera"]);
    }

    #[test]
    fn break_scenes_carry_the_overlays() {
        let g = build(Purpose::Gaming, &detected(true), &Choices::default());
        let names: Vec<&str> = g.scenes.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Gameplay", "Starting soon", "Be right back"]);
        for i in 1..3 {
            assert_eq!(
                scene_kinds(&g, i),
                ["color", "text", "chat_overlay", "alerts"]
            );
            inside(&g, i);
        }
        // One chat and one alerts source, shown in every scene.
        assert_eq!(
            g.sources
                .iter()
                .filter(|s| s.kind == "chat_overlay")
                .count(),
            1
        );
        assert_eq!(g.sources.iter().filter(|s| s.kind == "alerts").count(), 1);
        assert!(g.scenes.iter().all(|s| s.canvas == g.canvases[0].id));

        // Ticked on a vertical canvas, they still fit.
        let mut choices = Choices::default();
        for part in [Part::Chat, Part::Alerts, Part::BreakScenes] {
            choices.toggle(part, Purpose::Vertical);
        }
        let v = build(Purpose::Vertical, &detected(true), &choices);
        assert_eq!(v.scenes.len(), 3);
        for i in 0..3 {
            inside(&v, i);
        }
    }

    #[test]
    fn left_out_parts_are_not_added() {
        let d = detected(true);
        let mut choices = Choices::default();
        for part in [
            Part::Webcam,
            Part::Microphone,
            Part::Chat,
            Part::BreakScenes,
        ] {
            choices.toggle(part, Purpose::Gaming);
        }
        let g = build(Purpose::Gaming, &d, &choices);
        assert!(!g.sources.iter().any(|s| s.kind == "camera"));
        assert!(!g.sources.iter().any(|s| s.kind == "audio_input"));
        assert!(g.sources.iter().any(|s| s.kind == "audio_output"));
        assert!(!g.sources.iter().any(|s| s.kind == "chat_overlay"));
        assert!(g.sources.iter().any(|s| s.kind == "alerts"));
        assert_eq!(g.scenes.len(), 1);

        // Without the webcam, a camera purpose shows the screen.
        let c = build(Purpose::Camera, &d, &choices);
        assert_eq!(kinds(&c)[0], "display_capture");
    }

    #[test]
    fn choices_follow_the_purpose_until_changed() {
        let mut choices = Choices::default();
        assert!(choices.includes(Part::Chat, Purpose::Gaming));
        assert!(!choices.includes(Part::Chat, Purpose::Recording));
        choices.toggle(Part::Webcam, Purpose::Gaming);
        choices.toggle(Part::Chat, Purpose::Recording);
        for purpose in Purpose::ALL {
            assert!(
                !choices.includes(Part::Webcam, purpose),
                "kept across purposes"
            );
            assert!(choices.includes(Part::Chat, purpose));
        }
        for part in Part::ALL {
            assert_eq!(Part::from_key(part.key()), Some(part));
        }
    }

    #[test]
    fn summaries_mention_what_was_found() {
        let text = |lines: Vec<Line>| {
            lines
                .into_iter()
                .map(|l| l.text)
                .collect::<Vec<_>>()
                .join("\n")
        };
        let all = Choices::default();
        let lines = text(summary(Purpose::Gaming, &detected(true), &all));
        assert!(
            lines.contains("Display 1") && lines.contains("LifeCam") && lines.contains("NVENC")
        );
        let lines = text(summary(Purpose::Camera, &detected(false), &all));
        assert!(lines.contains("No webcam found"), "{lines}");

        let mut choices = Choices::default();
        choices.toggle(Part::Webcam, Purpose::Camera);
        let lines = summary(Purpose::Camera, &detected(true), &choices);
        let webcam = lines.iter().find(|l| l.part == Some(Part::Webcam)).unwrap();
        assert!(!webcam.on);
        assert!(lines.iter().any(|l| l.text.starts_with("Screen instead")));
        let encoder = lines.last().unwrap();
        assert_eq!((encoder.part, encoder.on), (None, true));
    }
}
