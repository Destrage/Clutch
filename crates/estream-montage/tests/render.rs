//! Golden checks of the montage renderer on a headless GPU, with synthetic Y4M sources.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use estream_gpu::{GpuContext, GpuOptions};
use estream_montage::model::{
    Anchor, Crop, Format, Montage, Segment, Title, TitleStyle, Transition,
};
use estream_montage::render::Renderer;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

fn gpu() -> GpuContext {
    static GPU: OnceLock<GpuContext> = OnceLock::new();
    GPU.get_or_init(|| GpuContext::new_headless(&GpuOptions::default()).unwrap())
        .clone()
}

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let d = std::env::temp_dir().join(format!("estream-render-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    })
    .clone()
}

/// BT.601 limited-range Y, Cb, Cr for sRGB-coded `rgb`.
fn yuv(rgb: [f32; 3]) -> [u8; 3] {
    let [r, g, b] = rgb;
    let y = 16.0 + 65.481 * r + 128.553 * g + 24.966 * b;
    let cb = 128.0 - 37.797 * r - 74.203 * g + 112.0 * b;
    let cr = 128.0 + 112.0 * r - 93.786 * g - 18.214 * b;
    [y.round() as u8, cb.round() as u8, cr.round() as u8]
}

/// A 2 s, 30 fps, 320×180 Y4M clip whose pixel color is `color(x / width)`.
fn clip(name: &str, color: impl Fn(f32) -> [f32; 3]) -> PathBuf {
    let path = dir().join(name);
    if path.exists() {
        return path;
    }
    let (w, h) = (320usize, 180usize);
    let mut y = vec![0u8; w * h];
    let mut u = vec![0u8; w * h / 4];
    let mut v = vec![0u8; w * h / 4];
    for row in 0..h {
        for col in 0..w {
            let [yy, cb, cr] = yuv(color(col as f32 / w as f32));
            y[row * w + col] = yy;
            if row % 2 == 0 && col % 2 == 0 {
                let i = (row / 2) * (w / 2) + col / 2;
                u[i] = cb;
                v[i] = cr;
            }
        }
    }
    let mut f = std::fs::File::create(&path).unwrap();
    writeln!(
        f,
        "YUV4MPEG2 W{w} H{h} F30:1 Ip A1:1 C420jpeg XCOLORRANGE=LIMITED"
    )
    .unwrap();
    for _ in 0..60 {
        f.write_all(b"FRAME\n").unwrap();
        f.write_all(&y).unwrap();
        f.write_all(&u).unwrap();
        f.write_all(&v).unwrap();
    }
    path
}

fn red() -> PathBuf {
    clip("red.y4m", |_| [1.0, 0.0, 0.0])
}

fn blue() -> PathBuf {
    clip("blue.y4m", |_| [0.0, 0.0, 1.0])
}

/// Left half red, right half blue.
fn split() -> PathBuf {
    clip("split.y4m", |x| {
        if x < 0.5 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 0.0, 1.0]
        }
    })
}

fn montage(format: Format, sources: &[(&Path, Crop)], transition: Transition) -> Montage {
    let mut m = Montage::new("test", format);
    for (path, crop) in sources {
        let id = m.new_id();
        let mut s = Segment::new(id, *path, 0.0, 2.0);
        s.crop = *crop;
        m.segments.push(s);
    }
    m.normalize();
    m.transitions.fill(transition);
    m
}

/// The RGBA pixel at (fx, fy) in 0..1 of the frame.
fn pixel(r: &Renderer, rgba: &[u8], fx: f32, fy: f32) -> [u8; 3] {
    let (w, h) = r.size();
    let x = ((fx * w as f32) as u32).min(w - 1);
    let y = ((fy * h as f32) as u32).min(h - 1);
    let i = ((y * w + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2]]
}

fn near(got: [u8; 3], want: [u8; 3], tolerance: u8) -> bool {
    got.iter()
        .zip(want)
        .all(|(g, w)| (*g as i16 - w as i16).unsigned_abs() <= tolerance as u16)
}

fn frame(m: &Montage, t: f64, w: u32, h: u32) -> (Renderer, Vec<u8>) {
    let mut r = Renderer::new(&gpu(), w, h);
    r.render(m, t).unwrap();
    let rgba = r.read_rgba().unwrap();
    (r, rgba)
}

#[test]
fn a_segment_shows_its_source() {
    let m = montage(Format::LANDSCAPE, &[(&red(), Crop::FULL)], Transition::Cut);
    let (r, rgba) = frame(&m, 1.0, 320, 180);
    let c = pixel(&r, &rgba, 0.5, 0.5);
    assert!(near(c, [255, 0, 0], 12), "{c:?}");
}

#[test]
fn crossfade_midpoint_mixes_in_linear_light() {
    let m = montage(
        Format::LANDSCAPE,
        &[(&red(), Crop::FULL), (&blue(), Crop::FULL)],
        Transition::Crossfade { secs: 1.0 },
    );
    // Segments are 2 s with a 1 s crossfade: it runs from 1.0 to 2.0 s.
    let (r, rgba) = frame(&m, 1.5, 320, 180);
    let c = pixel(&r, &rgba, 0.5, 0.5);
    // Half of each in linear light is sRGB ~188.
    assert!(near(c, [188, 0, 188], 14), "{c:?}");
}

#[test]
fn dip_to_black_is_black_at_its_midpoint() {
    let m = montage(
        Format::LANDSCAPE,
        &[(&red(), Crop::FULL), (&blue(), Crop::FULL)],
        Transition::DipToBlack { secs: 1.0 },
    );
    let (r, rgba) = frame(&m, 1.5, 320, 180);
    let c = pixel(&r, &rgba, 0.5, 0.5);
    assert!(near(c, [0, 0, 0], 12), "{c:?}");
}

#[test]
fn whip_shows_both_sides_midway() {
    let m = montage(
        Format::LANDSCAPE,
        &[(&red(), Crop::FULL), (&blue(), Crop::FULL)],
        Transition::Whip { secs: 1.0 },
    );
    let (r, rgba) = frame(&m, 1.5, 320, 180);
    let left = pixel(&r, &rgba, 0.1, 0.5);
    let right = pixel(&r, &rgba, 0.9, 0.5);
    assert!(
        left[0] > 150 && right[2] > 150,
        "left {left:?} right {right:?}"
    );
}

#[test]
fn vertical_crop_fills_the_frame_from_the_middle() {
    let crop = Crop::fill(16.0 / 9.0, 9.0 / 16.0, 0.0);
    let m = montage(Format::VERTICAL, &[(&split(), crop)], Transition::Cut);
    let (r, rgba) = frame(&m, 0.5, 180, 320);
    assert!(
        near(pixel(&r, &rgba, 0.25, 0.05), [255, 0, 0], 12),
        "left of the middle is red"
    );
    assert!(
        near(pixel(&r, &rgba, 0.75, 0.95), [0, 0, 255], 12),
        "right of the middle is blue"
    );
    // Without the crop, the picture is letterboxed: black above and below.
    let full = montage(Format::VERTICAL, &[(&split(), Crop::FULL)], Transition::Cut);
    let (r, rgba) = frame(&full, 0.5, 180, 320);
    assert!(near(pixel(&r, &rgba, 0.5, 0.05), [0, 0, 0], 8));
    assert!(near(pixel(&r, &rgba, 0.25, 0.5), [255, 0, 0], 12));
}

#[test]
fn titles_draw_over_the_video_and_the_end_fades_out() {
    let mut m = montage(Format::LANDSCAPE, &[(&blue(), Crop::FULL)], Transition::Cut);
    let id = m.new_id();
    m.titles.push(Title {
        id,
        text: "HIGHLIGHTS".into(),
        style: TitleStyle::Bold,
        start: 0.0,
        dur: 2.0,
        anchor: Anchor::Center,
    });
    let (r, rgba) = frame(&m, 1.0, 640, 360);
    let (w, h) = r.size();
    // Somewhere along the middle row there is white text.
    let white = (0..w).any(|x| {
        let i = (((h / 2) * w + x) * 4) as usize;
        rgba[i] > 230 && rgba[i + 1] > 230 && rgba[i + 2] > 230
    });
    assert!(white, "no title pixels on the middle row");
    // The corners are still the video.
    assert!(near(pixel(&r, &rgba, 0.02, 0.02), [0, 0, 255], 12));

    // The end fade is linear in light: halfway is sRGB ~188, the last instant is black.
    m.fade_out = 0.5;
    let (r, rgba) = frame(&m, m.duration() - 0.25, 640, 360);
    let half = pixel(&r, &rgba, 0.02, 0.02);
    assert!(near(half, [0, 0, 188], 14), "halfway {half:?}");
    let (r, rgba) = frame(&m, m.duration() - 0.001, 640, 360);
    let end = pixel(&r, &rgba, 0.02, 0.02);
    assert!(near(end, [0, 0, 0], 10), "faded to black: {end:?}");
}

#[test]
fn nv12_readback_has_the_right_size() {
    let m = montage(Format::LANDSCAPE, &[(&red(), Crop::FULL)], Transition::Cut);
    let mut r = Renderer::new(&gpu(), 320, 180);
    r.render(&m, 0.5).unwrap();
    let nv12 = r.read_nv12().unwrap();
    assert_eq!(nv12.len(), 320 * 180 * 3 / 2);
    // Red in BT.709 limited range: Y ≈ 63.
    assert!(
        (nv12[320 * 90 + 160] as i16 - 63).abs() <= 6,
        "Y {}",
        nv12[320 * 90 + 160]
    );
}

#[test]
fn missing_sources_are_an_error() {
    let m = montage(
        Format::LANDSCAPE,
        &[(Path::new("C:/no/such/clip.mp4"), Crop::FULL)],
        Transition::Cut,
    );
    let mut r = Renderer::new(&gpu(), 320, 180);
    assert!(r.render(&m, 0.5).is_err());
}
