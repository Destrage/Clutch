#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Headless GPU tests for the compositor and output conversion.

use estream_core::{BlendMode, Crop, ItemTransform, Size};
use estream_gpu::{
    CANVAS_FORMAT, CanvasRenderer, Compositor, DrawItem, GpuContext, GpuOptions, Nv12Target,
    OutputConverter, readback, wgpu,
};
use glam::Vec2;

fn gpu() -> GpuContext {
    GpuContext::new_headless(&GpuOptions::default())
        .or_else(|_| {
            GpuContext::new_headless(&GpuOptions {
                force_software: true,
            })
        })
        .expect("a GPU adapter (hardware or software)")
}

/// Uploads a straight-alpha sRGB image as a source texture.
fn source_texture(gpu: &GpuContext, width: u32, height: u32, rgba: &[u8]) -> wgpu::Texture {
    let tex = gpu.create_render_texture("src", width, height, wgpu::TextureFormat::Rgba8UnormSrgb);
    gpu.queue.write_texture(
        tex.as_image_copy(),
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        tex.size(),
    );
    tex
}

struct Harness {
    gpu: GpuContext,
    compositor: std::sync::Arc<Compositor>,
    converter: std::sync::Arc<OutputConverter>,
}

impl Harness {
    fn new() -> Self {
        let gpu = gpu();
        Self {
            compositor: Compositor::new(&gpu),
            converter: OutputConverter::new(&gpu),
            gpu,
        }
    }

    /// Renders items onto a black canvas and returns sRGB8 pixels.
    fn render(&self, w: u32, h: u32, items: &[DrawItem<'_>]) -> Vec<u8> {
        let canvas = self.render_canvas(w, h, items);
        let preview = estream_gpu::create_preview_texture(&self.gpu, w, h);
        let input = self
            .converter
            .create_input(&self.gpu, &canvas.create_view(&Default::default()));
        let mut enc = self.gpu.device.create_command_encoder(&Default::default());
        self.converter.to_srgb(
            &mut enc,
            &input,
            &estream_gpu::preview_target_view(&preview),
        );
        self.gpu.queue.submit([enc.finish()]);
        readback::read_texture(&self.gpu, &preview).unwrap()
    }

    fn render_canvas(&self, w: u32, h: u32, items: &[DrawItem<'_>]) -> wgpu::Texture {
        let canvas = self
            .gpu
            .create_render_texture("canvas", w, h, CANVAS_FORMAT);
        let mut renderer = CanvasRenderer::new(&self.gpu, self.compositor.clone());
        let mut enc = self.gpu.device.create_command_encoder(&Default::default());
        renderer.render(
            &self.gpu,
            &mut enc,
            &canvas.create_view(&Default::default()),
            Vec2::new(w as f32, h as f32),
            wgpu::Color::BLACK,
            items,
        );
        self.gpu.queue.submit([enc.finish()]);
        canvas
    }

    fn solid(&self, rgba: [u8; 4]) -> wgpu::BindGroup {
        let tex = source_texture(&self.gpu, 1, 1, &rgba);
        self.compositor
            .create_source_binding(&self.gpu, &tex.create_view(&Default::default()))
    }
}

fn item<'a>(
    binding: &'a wgpu::BindGroup,
    t: ItemTransform,
    crop: Crop,
    size: Size,
    opacity: f32,
    blend: BlendMode,
) -> DrawItem<'a> {
    let l = t.layout(crop, size);
    DrawItem {
        binding,
        matrix: l.matrix,
        uv_min: l.uv_min,
        uv_max: l.uv_max,
        opacity,
        blend,
    }
}

fn px(img: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * w + x) * 4) as usize;
    [img[i], img[i + 1], img[i + 2], img[i + 3]]
}

#[test]
fn half_opaque_white_over_black_is_linear_half() {
    let h = Harness::new();
    let white = h.solid([255, 255, 255, 255]);
    let full = ItemTransform {
        scale: Vec2::splat(8.0),
        ..Default::default()
    };
    let img = h.render(
        8,
        8,
        &[item(
            &white,
            full,
            Crop::NONE,
            Size::new(1.0, 1.0),
            0.5,
            BlendMode::Normal,
        )],
    );
    let p = px(&img, 8, 4, 4);
    for c in &p[..3] {
        assert!((*c as i32 - 188).abs() <= 1, "expected ~188, got {p:?}");
    }
}

#[test]
fn missing_area_stays_background_and_fit_bounds_letterbox() {
    let h = Harness::new();
    let red = h.solid([255, 0, 0, 255]);
    // A 16:9 source fit into a 100x100 box, centered: occupies y in [21.875, 78.125).
    let t = ItemTransform {
        bounds_mode: estream_core::BoundsMode::Fit,
        bounds: Vec2::new(100.0, 100.0),
        bounds_alignment: estream_core::Alignment::Center,
        ..Default::default()
    };
    let img = h.render(
        100,
        100,
        &[item(
            &red,
            t,
            Crop::NONE,
            Size::new(1920.0, 1080.0),
            1.0,
            BlendMode::Normal,
        )],
    );
    assert_eq!(px(&img, 100, 50, 10), [0, 0, 0, 255], "letterbox above");
    assert_eq!(px(&img, 100, 50, 50), [255, 0, 0, 255], "content");
    assert_eq!(px(&img, 100, 50, 90), [0, 0, 0, 255], "letterbox below");
    assert_eq!(px(&img, 100, 0, 50), [255, 0, 0, 255], "full width");
}

#[test]
fn crop_selects_texture_region() {
    let h = Harness::new();
    // 2x1 texture: left green, right blue. Crop off the left pixel.
    let tex = source_texture(&h.gpu, 2, 1, &[0, 255, 0, 255, 0, 0, 255, 255]);
    let binding = h
        .compositor
        .create_source_binding(&h.gpu, &tex.create_view(&Default::default()));
    let t = ItemTransform {
        scale: Vec2::splat(10.0),
        ..Default::default()
    };
    let crop = Crop {
        left: 1,
        ..Crop::NONE
    };
    let img = h.render(
        10,
        10,
        &[item(
            &binding,
            t,
            crop,
            Size::new(2.0, 1.0),
            1.0,
            BlendMode::Normal,
        )],
    );
    // Only the blue half remains, scaled to 10x10.
    assert_eq!(px(&img, 10, 5, 5), [0, 0, 255, 255]);
    assert_eq!(px(&img, 10, 1, 1), [0, 0, 255, 255]);
}

#[test]
fn blend_modes() {
    let h = Harness::new();
    let grey = h.solid([188, 188, 188, 255]); // linear 0.5
    let full = ItemTransform {
        scale: Vec2::splat(4.0),
        ..Default::default()
    };
    let one = Size::new(1.0, 1.0);
    let render = |mode| {
        let img = h.render(
            4,
            4,
            &[
                item(&grey, full, Crop::NONE, one, 1.0, BlendMode::Normal),
                item(&grey, full, Crop::NONE, one, 1.0, mode),
            ],
        );
        px(&img, 4, 2, 2)[0] as i32
    };
    // Linear results: normal 0.5, additive 1.0, multiply 0.25, screen 0.75.
    assert!((render(BlendMode::Normal) - 188).abs() <= 1);
    assert!((render(BlendMode::Additive) - 255).abs() <= 1);
    assert!((render(BlendMode::Multiply) - 137).abs() <= 1);
    assert!((render(BlendMode::Screen) - 225).abs() <= 1);
}

#[test]
fn nv12_bt709_limited_range() {
    let h = Harness::new();
    let red = h.solid([255, 0, 0, 255]);
    let white = h.solid([255, 255, 255, 255]);
    let one = Size::new(1.0, 1.0);
    // Left half white, right half red, on a 4x2 canvas.
    let left = ItemTransform {
        scale: Vec2::new(2.0, 2.0),
        ..Default::default()
    };
    let right = ItemTransform {
        position: Vec2::new(2.0, 0.0),
        ..left
    };
    let canvas = h.render_canvas(
        4,
        2,
        &[
            item(&white, left, Crop::NONE, one, 1.0, BlendMode::Normal),
            item(&red, right, Crop::NONE, one, 1.0, BlendMode::Normal),
        ],
    );
    let target = Nv12Target::new(&h.gpu, 4, 2);
    let input = h
        .converter
        .create_input(&h.gpu, &canvas.create_view(&Default::default()));
    let mut enc = h.gpu.device.create_command_encoder(&Default::default());
    h.converter.to_nv12(&mut enc, &input, &target);
    h.gpu.queue.submit([enc.finish()]);

    let y = readback::read_texture(&h.gpu, &target.y).unwrap();
    let uv = readback::read_texture(&h.gpu, &target.uv).unwrap();
    let close = |a: u8, b: i32| (a as i32 - b).abs() <= 1;
    assert!(close(y[0], 235), "white Y = {}", y[0]);
    assert!(close(y[3], 63), "red Y = {}", y[3]);
    assert!(
        close(uv[0], 128) && close(uv[1], 128),
        "white CbCr {:?}",
        &uv[..2]
    );
    assert!(
        close(uv[2], 102) && close(uv[3], 240),
        "red CbCr {:?}",
        &uv[2..4]
    );
}

#[test]
fn nv12_to_rgb_matches_reference_colors() {
    use estream_gpu::{YuvConverter, YuvMatrix};
    let gpu = gpu();
    let converter = YuvConverter::new(&gpu);
    let frame = converter.create_frame(&gpu, 4, 2);
    // 4x2 BT.709 limited range: left half white, right half pure red.
    // Y plane (2 rows × 4), then CbCr plane (1 row × 2 pairs).
    let data = [
        235, 235, 63, 63, //
        235, 235, 63, 63, //
        128, 128, 102, 240,
    ];
    frame.upload(&gpu, &data, 4, YuvMatrix::Bt709, false);
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    frame.convert(&converter, &mut enc);
    gpu.queue.submit([enc.finish()]);
    let px = readback::read_texture(&gpu, &frame.output).unwrap();
    let close = |a: u8, b: u8| (a as i32 - b as i32).abs() <= 2;
    assert!(
        px[..3].iter().all(|v| close(*v, 255)),
        "white {:?}",
        &px[..4]
    );
    let red = &px[12..16];
    assert!(
        close(red[0], 255) && close(red[1], 0) && close(red[2], 0),
        "red {red:?}"
    );

    // Full-range BT.601 mid grey.
    let frame = converter.create_frame(&gpu, 2, 2);
    frame.upload(
        &gpu,
        &[128, 128, 128, 128, 128, 128],
        2,
        YuvMatrix::Bt601,
        true,
    );
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    frame.convert(&converter, &mut enc);
    gpu.queue.submit([enc.finish()]);
    let px = readback::read_texture(&gpu, &frame.output).unwrap();
    assert!(
        px[..3].iter().all(|v| close(*v, 128)),
        "grey {:?}",
        &px[..4]
    );
}
