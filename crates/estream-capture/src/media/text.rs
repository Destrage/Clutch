//! Text source: shaped with cosmic-text (complex scripts, fallback fonts,
//! color emoji), rasterized on a worker thread with outline, shadow and
//! gradient, and optionally read from a file that is watched for changes.

use super::upload::{CpuTexture, Pixels};
use cosmic_text::{Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache};
use estream_core::{Rgba8, Timestamp};
use estream_engine::source::parse;
use estream_engine::{
    PropKind, Property, PropertySchema, SelectOption, SourceContext, SourceError, SourceFrame,
    SourceStatus, SourceType, VideoSource,
};
use estream_gpu::wgpu;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextSettings {
    pub text: String,
    pub font: String,
    pub size: f64,
    pub color: Rgba8,
    pub gradient: bool,
    pub gradient_color: Rgba8,
    pub outline: bool,
    pub outline_color: Rgba8,
    pub outline_width: f64,
    pub shadow: bool,
    pub shadow_color: Rgba8,
    pub shadow_offset: f64,
    /// "left" | "center" | "right"
    pub align: String,
    /// Wrap width in pixels; 0 disables wrapping.
    pub wrap_width: i64,
    pub from_file: bool,
    pub file: String,
}

impl Default for TextSettings {
    fn default() -> Self {
        Self {
            text: "Hello, Clutch!".into(),
            font: default_font().into(),
            size: 72.0,
            color: Rgba8::WHITE,
            gradient: false,
            gradient_color: Rgba8::new(108, 140, 255, 255),
            outline: false,
            outline_color: Rgba8::BLACK,
            outline_width: 4.0,
            shadow: false,
            shadow_color: Rgba8::new(0, 0, 0, 160),
            shadow_offset: 4.0,
            align: "left".into(),
            wrap_width: 0,
            from_file: false,
            file: String::new(),
        }
    }
}

fn default_font() -> &'static str {
    if cfg!(windows) {
        "Segoe UI"
    } else {
        "sans-serif"
    }
}

struct Fonts {
    system: FontSystem,
    cache: SwashCache,
}

fn fonts() -> &'static Mutex<Fonts> {
    static FONTS: OnceLock<Mutex<Fonts>> = OnceLock::new();
    FONTS.get_or_init(|| {
        Mutex::new(Fonts {
            system: FontSystem::new(),
            cache: SwashCache::new(),
        })
    })
}

/// Installed font family names, sorted.
pub fn families() -> Vec<String> {
    let Ok(fonts) = fonts().lock() else {
        return Vec::new();
    };
    let mut names: Vec<String> = fonts
        .system
        .db()
        .faces()
        .filter_map(|f| f.families.first().map(|(name, _)| name.clone()))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names
}

type Rgba = [f32; 4];

fn rgba(c: Rgba8, alpha: f32) -> Rgba {
    [
        c.r as f32 / 255.0,
        c.g as f32 / 255.0,
        c.b as f32 / 255.0,
        c.a as f32 / 255.0 * alpha,
    ]
}

/// Straight-alpha "source over destination".
fn over(dst: Rgba, src: Rgba) -> Rgba {
    let a = src[3] + dst[3] * (1.0 - src[3]);
    if a <= 0.0 {
        return [0.0; 4];
    }
    let mix = |s: f32, d: f32| (s * src[3] + d * dst[3] * (1.0 - src[3])) / a;
    [
        mix(src[0], dst[0]),
        mix(src[1], dst[1]),
        mix(src[2], dst[2]),
        a,
    ]
}

/// Grows a coverage mask by `radius` pixels (round brush).
fn dilate(mask: &[f32], w: usize, h: usize, radius: f32) -> Vec<f32> {
    let r = radius.ceil() as i32;
    let offsets: Vec<(i32, i32, f32)> = (-r..=r)
        .flat_map(|dy| (-r..=r).map(move |dx| (dx, dy)))
        .filter_map(|(dx, dy)| {
            let d = ((dx * dx + dy * dy) as f32).sqrt();
            // Soft edge for anti-aliasing.
            let weight = (radius + 0.5 - d).clamp(0.0, 1.0);
            (weight > 0.0).then_some((dx, dy, weight))
        })
        .collect();
    let mut out = vec![0.0; w * h];
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let mut v: f32 = 0.0;
            for &(dx, dy, weight) in &offsets {
                let (sx, sy) = (x + dx, y + dy);
                if sx >= 0 && sy >= 0 && (sx as usize) < w && (sy as usize) < h {
                    v = v.max(mask[sy as usize * w + sx as usize] * weight);
                    if v >= 1.0 {
                        break;
                    }
                }
            }
            out[y as usize * w + x as usize] = v;
        }
    }
    out
}

/// Renders text to straight-alpha RGBA pixels.
pub fn render(settings: &TextSettings, text: &str) -> Pixels {
    let Ok(mut guard) = fonts().lock() else {
        return Pixels::default();
    };
    let Fonts { system, cache } = &mut *guard;
    let size = settings.size.clamp(4.0, 1000.0) as f32;
    let mut buffer = Buffer::new(system, Metrics::new(size, size * 1.25));
    let wrap = (settings.wrap_width > 0).then_some(settings.wrap_width as f32);
    buffer.set_size(wrap, None);
    let align = match settings.align.as_str() {
        "center" => Align::Center,
        "right" => Align::Right,
        _ => Align::Left,
    };
    let attrs = Attrs::new().family(Family::Name(&settings.font));
    buffer.set_text(text, &attrs, Shaping::Advanced, Some(align));
    buffer.shape_until_scroll(system, false);

    let (mut text_w, mut text_h) = (0.0f32, 0.0f32);
    for run in buffer.layout_runs() {
        text_w = text_w.max(run.line_w);
        text_h = text_h.max(run.line_top + run.line_height);
    }
    if let Some(w) = wrap {
        text_w = text_w.max(w);
    }
    if text_w <= 0.0 || text_h <= 0.0 {
        return Pixels::default();
    }
    let outline = if settings.outline {
        settings.outline_width.clamp(0.0, 64.0) as f32
    } else {
        0.0
    };
    let shadow = if settings.shadow {
        settings.shadow_offset.clamp(0.0, 128.0) as f32
    } else {
        0.0
    };
    let pad = (outline.ceil() + shadow.ceil()) as i32 + 2;
    let w = (text_w.ceil() as i32 + pad * 2).min(8192) as usize;
    let h = (text_h.ceil() as i32 + pad * 2).min(8192) as usize;

    // Glyph layer: white for regular glyphs, real colors for emoji.
    let mut glyphs = vec![[0.0f32; 4]; w * h];
    buffer.draw(
        system,
        cache,
        Color::rgba(255, 255, 255, 255),
        |x, y, gw, gh, color| {
            let src = [
                color.r() as f32 / 255.0,
                color.g() as f32 / 255.0,
                color.b() as f32 / 255.0,
                color.a() as f32 / 255.0,
            ];
            for py in y + pad..y + pad + gh as i32 {
                for px in x + pad..x + pad + gw as i32 {
                    if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                        let i = py as usize * w + px as usize;
                        glyphs[i] = over(glyphs[i], src);
                    }
                }
            }
        },
    );

    let mask: Vec<f32> = glyphs.iter().map(|p| p[3]).collect();
    let outline_mask = (outline > 0.0).then(|| dilate(&mask, w, h, outline));
    let shadow_source = outline_mask.as_ref().unwrap_or(&mask);
    let shift = shadow.round() as usize;

    let mut out = Pixels::new(w as u32, h as u32);
    for y in 0..h {
        let t = y as f32 / h.max(1) as f32;
        let fill = if settings.gradient {
            let (a, b) = (
                rgba(settings.color, 1.0),
                rgba(settings.gradient_color, 1.0),
            );
            [0, 1, 2, 3].map(|i| a[i] + (b[i] - a[i]) * t)
        } else {
            rgba(settings.color, 1.0)
        };
        for x in 0..w {
            let i = y * w + x;
            let mut px = [0.0f32; 4];
            if settings.shadow && x >= shift && y >= shift {
                let m = shadow_source[(y - shift) * w + (x - shift)];
                px = over(px, rgba(settings.shadow_color, m));
            }
            if let Some(o) = &outline_mask {
                px = over(px, rgba(settings.outline_color, o[i]));
            }
            let g = glyphs[i];
            if g[3] > 0.0 {
                let mono = g[0] > 0.99 && g[1] > 0.99 && g[2] > 0.99;
                let src = if mono {
                    [fill[0], fill[1], fill[2], fill[3] * g[3]]
                } else {
                    g
                };
                px = over(px, src);
            }
            let o = &mut out.rgba[i * 4..i * 4 + 4];
            for c in 0..4 {
                o[c] = (px[c].clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
    }
    out
}

/// Worker: renders on settings changes and when the watched file changes.
fn spawn_renderer() -> (Sender<TextSettings>, Receiver<Result<Pixels, String>>) {
    let (settings_tx, settings_rx) = channel::<TextSettings>();
    let (out_tx, out_rx) = channel();
    let _ = std::thread::Builder::new()
        .name("estream-text".into())
        .spawn(move || {
            let mut settings: Option<TextSettings> = None;
            let mut file_stamp: Option<SystemTime> = None;
            loop {
                let mut dirty = false;
                match settings_rx.recv_timeout(Duration::from_millis(300)) {
                    Ok(new) => {
                        // Coalesce bursts (e.g. typing) into one render.
                        let mut latest = new;
                        while let Ok(newer) = settings_rx.try_recv() {
                            latest = newer;
                        }
                        settings = Some(latest);
                        file_stamp = None;
                        dirty = true;
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                }
                let Some(s) = &settings else { continue };
                let text = if s.from_file {
                    let stamp = std::fs::metadata(&s.file).and_then(|m| m.modified()).ok();
                    if stamp != file_stamp {
                        file_stamp = stamp;
                        dirty = true;
                    }
                    if !dirty {
                        continue;
                    }
                    match std::fs::read_to_string(&s.file) {
                        Ok(t) => Ok(t.trim_end_matches(['\r', '\n']).to_owned()),
                        Err(e) => Err(format!("Can't read {}: {e}", s.file)),
                    }
                } else {
                    if !dirty {
                        continue;
                    }
                    Ok(s.text.clone())
                };
                let result = text.map(|t| render(s, &t));
                if out_tx.send(result).is_err() {
                    return;
                }
            }
        });
    (settings_tx, out_rx)
}

pub struct TextSource {
    settings_tx: Sender<TextSettings>,
    results: Receiver<Result<Pixels, String>>,
    texture: CpuTexture,
    status: SourceStatus,
}

impl VideoSource for TextSource {
    fn update(&mut self, _: &SourceContext<'_>, settings: &Value) -> Result<(), SourceError> {
        let s: TextSettings = parse(settings)?;
        let _ = self.settings_tx.send(s);
        Ok(())
    }

    fn tick(&mut self, ctx: &SourceContext<'_>, _: &mut wgpu::CommandEncoder, _: Timestamp) {
        let mut latest = None;
        while let Ok(r) = self.results.try_recv() {
            latest = Some(r);
        }
        match latest {
            Some(Ok(pixels)) => {
                self.texture.upload(ctx, &pixels);
                self.status = SourceStatus::Active;
            }
            Some(Err(e)) => self.status = SourceStatus::Unavailable(e),
            None => {}
        }
    }

    fn frame(&self) -> Option<SourceFrame<'_>> {
        self.texture.frame()
    }

    fn status(&self) -> SourceStatus {
        self.status.clone()
    }
}

fn color_prop(key: &str, label: &str, c: Rgba8) -> Property {
    Property::new(
        key,
        label,
        PropKind::Color,
        serde_json::to_value(c).unwrap_or_default(),
    )
}

pub fn source_type() -> SourceType {
    SourceType::new(
        "text",
        "Text",
        "Styled text with outline and shadow",
        |_| {
            let fonts = families()
                .into_iter()
                .map(|f| SelectOption::new(f.clone(), f))
                .collect::<Vec<_>>();
            schema(fonts)
        },
        |ctx, settings| {
            let (settings_tx, results) = spawn_renderer();
            let mut s = TextSource {
                settings_tx,
                results,
                texture: CpuTexture::default(),
                status: SourceStatus::Unavailable("Rendering…".into()),
            };
            s.update(ctx, settings)?;
            Ok(Box::new(s))
        },
    )
    // Validation must not load the system font database on the control thread.
    .with_validation(|_| schema(Vec::new()))
}

fn schema(fonts: Vec<SelectOption>) -> PropertySchema {
    let d = TextSettings::default();
    PropertySchema::new(vec![
        Property::text("text", "Text", true, &d.text),
        Property::select("font", "Font", fonts, d.font.as_str()),
        Property::float("size", "Size", 4.0, 1000.0, 1.0, d.size),
        color_prop("color", "Color", d.color),
        Property::bool("gradient", "Gradient", false),
        color_prop("gradient_color", "Gradient end color", d.gradient_color),
        Property::bool("outline", "Outline", false),
        color_prop("outline_color", "Outline color", d.outline_color),
        Property::float(
            "outline_width",
            "Outline width",
            0.0,
            64.0,
            1.0,
            d.outline_width,
        ),
        Property::bool("shadow", "Drop shadow", false),
        color_prop("shadow_color", "Shadow color", d.shadow_color),
        Property::float(
            "shadow_offset",
            "Shadow distance",
            0.0,
            128.0,
            1.0,
            d.shadow_offset,
        ),
        Property::select(
            "align",
            "Alignment",
            vec![
                SelectOption::new("left", "Left"),
                SelectOption::new("center", "Center"),
                SelectOption::new("right", "Right"),
            ],
            "left",
        ),
        Property::int("wrap_width", "Wrap width (0 = off)", 0, 8192, 0),
        Property::bool("from_file", "Read text from file", false),
        Property::new(
            "file",
            "Text file",
            PropKind::Path {
                filters: vec!["txt".into()],
            },
            "",
        )
        .hint("Updates within a second of the file changing"),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_text_with_outline_and_shadow() {
        let s = TextSettings {
            size: 40.0,
            ..Default::default()
        };
        let plain = render(&s, "Hello");
        assert!(
            plain.width > 60 && plain.height > 30,
            "{}x{}",
            plain.width,
            plain.height
        );
        assert!(
            plain.rgba.chunks(4).any(|p| p == [255, 255, 255, 255]),
            "solid white glyph pixels"
        );

        let styled = TextSettings {
            outline: true,
            shadow: true,
            ..s.clone()
        };
        let px = render(&styled, "Hello");
        assert!(px.width > plain.width, "outline/shadow add padding");
        assert!(
            px.rgba.chunks(4).any(|p| p[3] == 255 && p[0] < 30),
            "dark outline pixels"
        );
    }

    #[test]
    fn complex_scripts_and_emoji_render() {
        let s = TextSettings {
            size: 40.0,
            ..Default::default()
        };
        let latin = render(&s, "Hi");
        let mixed = render(&s, "Hi مرحبا 你好 😀");
        assert!(
            mixed.width > latin.width * 3,
            "fallback fonts shaped the other scripts"
        );
        if cfg!(windows) {
            // Segoe UI Emoji is a color font: some pixels are neither white nor transparent.
            assert!(
                mixed
                    .rgba
                    .chunks(4)
                    .any(|p| p[3] > 200 && (p[0] as i32 - p[2] as i32).abs() > 60),
                "color emoji pixels"
            );
        }
    }

    #[test]
    fn empty_text_renders_nothing() {
        assert_eq!(render(&TextSettings::default(), "").width, 0);
    }
}
