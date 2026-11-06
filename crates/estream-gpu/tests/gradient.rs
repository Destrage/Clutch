#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Headless pixel tests for gradient paint textures.

use estream_core::{GradientStop, Paint, Rgba8};
use estream_gpu::{GpuContext, GpuOptions, GradientRenderer, readback};

fn gpu() -> GpuContext {
    GpuContext::new_headless(&GpuOptions::default())
        .or_else(|_| {
            GpuContext::new_headless(&GpuOptions {
                force_software: true,
            })
        })
        .expect("a GPU adapter (hardware or software)")
}

struct Pixels {
    width: u32,
    bytes: Vec<u8>,
}

impl Pixels {
    fn at(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.bytes[i],
            self.bytes[i + 1],
            self.bytes[i + 2],
            self.bytes[i + 3],
        ]
    }
}

fn render(gpu: &GpuContext, paint: &Paint, w: u32, h: u32) -> Pixels {
    let renderer = GradientRenderer::new(gpu);
    let out = renderer.render(gpu, paint, w, h);
    assert_eq!((out.texture.width(), out.texture.height()), (w, h));
    Pixels {
        width: w,
        bytes: readback::read_texture(gpu, &out.texture).unwrap(),
    }
}

fn near(a: [u8; 4], b: [u8; 4], tolerance: u8) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tolerance)
}

const RED: Rgba8 = Rgba8::new(255, 0, 0, 255);
const BLUE: Rgba8 = Rgba8::new(0, 0, 255, 255);

#[test]
fn linear_runs_left_to_right_and_blends_in_linear_light() {
    let gpu = gpu();
    let paint = Paint::linear(RED, BLUE);
    let px = render(&gpu, &paint, 256, 4);
    // Every pixel matches the CPU reference at its center, give or take the dither.
    for x in 0..256 {
        let want = paint.color_at((x as f32 + 0.5) / 256.0).to_array();
        assert!(
            near(px.at(x, 1), want, 2),
            "x {x}: {:?} vs {want:?}",
            px.at(x, 1)
        );
    }
    // The middle is 188 in sRGB (half in linear light), give or take the dither.
    let mid = px.at(128, 1);
    assert!(near(mid, [188, 0, 188, 255], 2), "{mid:?}");
}

#[test]
fn angle_180_runs_top_to_bottom() {
    let gpu = gpu();
    let paint = Paint::Linear {
        angle: 180.0,
        stops: vec![GradientStop::new(RED, 0.0), GradientStop::new(BLUE, 1.0)],
    };
    let px = render(&gpu, &paint, 4, 256);
    for y in [0, 64, 128, 200, 255] {
        let want = paint.color_at((y as f32 + 0.5) / 256.0).to_array();
        assert!(
            near(px.at(1, y), want, 2),
            "y {y}: {:?} vs {want:?}",
            px.at(1, y)
        );
    }
}

#[test]
fn radial_is_the_first_stop_at_the_center() {
    let gpu = gpu();
    let paint = Paint::Radial {
        center: [0.5, 0.5],
        radius: 1.0,
        stops: vec![
            GradientStop::new(Rgba8::WHITE, 0.0),
            GradientStop::new(Rgba8::BLACK, 1.0),
        ],
    };
    let px = render(&gpu, &paint, 64, 64);
    assert!(near(px.at(32, 32), [255, 255, 255, 255], 3));
    // The corner pixel's center is just inside the radius: nearly black.
    assert!(px.at(0, 0)[0] < 40, "{:?}", px.at(0, 0));
    let (edge, center) = (px.at(0, 32)[0], px.at(32, 32)[0]);
    assert!(edge < center);
}

#[test]
fn dark_gradient_is_dithered_not_banded() {
    // 1024 pixels across 8 levels: without dither each level would be a 128-pixel band.
    let gpu = gpu();
    let dark = Paint::linear(Rgba8::new(10, 10, 10, 255), Rgba8::new(18, 18, 18, 255));
    let px = render(&gpu, &dark, 1024, 2);
    let row: Vec<u8> = (0..1024).map(|x| px.at(x, 0)[0]).collect();
    let longest_run = row.chunk_by(|a, b| a == b).map(<[u8]>::len).max().unwrap();
    assert!(longest_run < 64, "band of {longest_run} pixels");
    // Still a steady climb on average.
    let avg = |r: &[u8]| r.iter().map(|&v| v as f32).sum::<f32>() / r.len() as f32;
    assert!(avg(&row[..128]) < avg(&row[448..576]));
    assert!(avg(&row[448..576]) < avg(&row[896..]));
}

#[test]
fn solid_is_one_texel() {
    let gpu = gpu();
    let renderer = GradientRenderer::new(&gpu);
    let out = renderer.render(&gpu, &Paint::solid(Rgba8::new(1, 2, 3, 4)), 1920, 1080);
    assert_eq!((out.texture.width(), out.texture.height()), (1, 1));
    let bytes = readback::read_texture(&gpu, &out.texture).unwrap();
    assert_eq!(&bytes[..4], &[1, 2, 3, 4]);
}

#[test]
fn read_pixel_matches_the_whole_readback() {
    let gpu = gpu();
    let renderer = GradientRenderer::new(&gpu);
    let out = renderer.render(&gpu, &Paint::linear(RED, BLUE), 64, 8);
    let all = readback::read_texture(&gpu, &out.texture).unwrap();
    for (x, y) in [(0, 0), (31, 4), (63, 7)] {
        let i = ((y * 64 + x) * 4) as usize;
        assert_eq!(
            readback::read_pixel(&gpu, &out.texture, x, y).unwrap(),
            [all[i], all[i + 1], all[i + 2], all[i + 3]]
        );
    }
    // Outside the texture: the nearest edge pixel.
    let last = (63 * 4) as usize;
    assert_eq!(
        readback::read_pixel(&gpu, &out.texture, 500, 0).unwrap(),
        [all[last], all[last + 1], all[last + 2], all[last + 3]]
    );
}
