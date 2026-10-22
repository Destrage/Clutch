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

/// Lines describing what the first scene will contain.
pub fn summary(purpose: Purpose, d: &Detected) -> Vec<String> {
    let mut lines = Vec::new();
    let screen = d
        .monitor
        .as_ref()
        .map_or("your screen".to_owned(), |(_, label)| label.clone());
    let camera_first = matches!(purpose, Purpose::Camera | Purpose::Vertical);
    match (&d.camera, camera_first) {
        (Some((_, name)), true) => lines.push(format!("Webcam: {name}")),
        (None, true) => lines.push(format!("No webcam found, so your screen is used: {screen}")),
        _ => {}
    }
    if !camera_first {
        lines.push(format!("Screen: {screen}"));
        if purpose == Purpose::Gaming && cfg!(windows) {
            lines.push("Your game and its sound, whenever one is fullscreen".into());
        }
        if let Some((_, name)) = &d.camera {
            lines.push(format!("Webcam in the corner: {name}"));
        }
    }
    lines.push(match &d.mic {
        Some(name) => format!("Microphone with noise suppression: {name}"),
        None => "Microphone with noise suppression (none found yet)".into(),
    });
    lines.push("Desktop audio".into());
    lines.push(format!("Encoder: {}", d.encoder));
    lines
}

/// The first scene collection for a purpose.
pub fn build_project(purpose: Purpose, preset: CanvasPreset, d: &Detected) -> Project {
    use estream_core::{Alignment, BoundsMode, ItemTransform};
    use glam::Vec2;
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
            .map(|(id, _)| ("Webcam", "camera", serde_json::json!({ "device": id }), t))
    };
    match purpose {
        Purpose::Gaming => {
            project.scenes[0].name = "Gameplay".into();
            screen(&mut project, &mut add);
            // Elsewhere the screen capture shows the game.
            if cfg!(windows) {
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
        p.scenes[0]
            .items
            .iter()
            .filter_map(|i| i.source_id())
            .filter_map(|id| p.source(id))
            .map(|s| s.kind.clone())
            .collect()
    }

    #[test]
    fn purposes_build_ready_scenes() {
        let d = detected(true);
        let g = build_project(Purpose::Gaming, Purpose::Gaming.preset(), &d);
        if cfg!(windows) {
            assert_eq!(kinds(&g), ["display_capture", "game_capture", "camera"]);
        } else {
            assert_eq!(kinds(&g), ["display_capture", "camera"]);
        }
        assert_eq!(g.canvases[0].fps, Rational::fps(60));
        let mic = g.sources.iter().find(|s| s.kind == "audio_input").unwrap();
        assert!(mic.audio.global);
        assert_eq!(mic.audio.filters[0].kind, "noise_suppression");
        assert!(g.sources.iter().any(|s| s.kind == "audio_output"));

        let c = build_project(Purpose::Camera, Purpose::Camera.preset(), &d);
        assert_eq!(kinds(&c), ["camera"]);
        let no_cam = build_project(Purpose::Camera, Purpose::Camera.preset(), &detected(false));
        assert_eq!(
            kinds(&no_cam),
            ["display_capture"],
            "falls back to the screen"
        );

        let v = build_project(Purpose::Vertical, Purpose::Vertical.preset(), &d);
        assert_eq!((v.canvases[0].width, v.canvases[0].height), (1080, 1920));

        // The corner webcam stays inside the canvas.
        let cam = g.scenes[0].items.last().unwrap();
        let t = cam.transform;
        assert!(t.position.x + t.bounds.x <= 1920.0 && t.position.y + t.bounds.y <= 1080.0);
    }

    #[test]
    fn summaries_mention_what_was_found() {
        let lines = summary(Purpose::Gaming, &detected(true)).join("\n");
        assert!(
            lines.contains("Display 1") && lines.contains("LifeCam") && lines.contains("NVENC")
        );
        let lines = summary(Purpose::Camera, &detected(false)).join("\n");
        assert!(lines.contains("No webcam found"), "{lines}");
    }
}
