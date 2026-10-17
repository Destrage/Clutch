#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Headless pixel tests for scene transitions: red (outgoing) → blue (incoming).

use estream_gpu::{
    BuiltinKind, CANVAS_FORMAT, EXAMPLE_CUSTOM_TRANSITION, GpuContext, GpuOptions,
    TransitionParams, TransitionRenderer, TransitionShader, readback, wgpu,
};

const W: u32 = 64;
const H: u32 = 32;

fn gpu() -> GpuContext {
    GpuContext::new_headless(&GpuOptions::default())
        .or_else(|_| {
            GpuContext::new_headless(&GpuOptions {
                force_software: true,
            })
        })
        .expect("a GPU adapter (hardware or software)")
}

fn clear(encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, color: wgpu::Color) {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(color),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
}

fn f16(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = ((bits >> 10) & 0x1f) as i32;
    let frac = (bits & 0x3ff) as f32;
    sign * match exp {
        0 => frac / 1024.0 * 2f32.powi(-14),
        31 => f32::INFINITY,
        e => (1.0 + frac / 1024.0) * 2f32.powi(e - 15),
    }
}

/// Renders one transition frame and returns linear RGBA per pixel.
struct Bench {
    gpu: GpuContext,
    renderer: TransitionRenderer,
    target: wgpu::Texture,
}

impl Bench {
    fn new() -> Self {
        let gpu = gpu();
        let renderer = TransitionRenderer::new(&gpu);
        let target = gpu.create_render_texture("target", W, H, CANVAS_FORMAT);
        Self {
            gpu,
            renderer,
            target,
        }
    }

    fn render(&mut self, shader: TransitionShader<'_>, params: TransitionParams) -> Vec<[f32; 4]> {
        if let TransitionShader::Custom(code) = &shader {
            assert!(self.renderer.prepare_custom(&self.gpu, code));
        }
        let targets = self.renderer.create_targets(&self.gpu, W, H);
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        clear(&mut encoder, &targets.from_view, wgpu::Color::RED);
        clear(&mut encoder, &targets.to_view, wgpu::Color::BLUE);
        let view = self.target.create_view(&Default::default());
        self.renderer
            .render(&self.gpu, &mut encoder, &targets, &view, &shader, &params);
        self.gpu.queue.submit([encoder.finish()]);
        let bytes = readback::read_texture(&self.gpu, &self.target).unwrap();
        bytes
            .as_chunks::<8>()
            .0
            .iter()
            .map(|p| {
                let c = |i: usize| f16(u16::from_le_bytes([p[i], p[i + 1]]));
                [c(0), c(2), c(4), c(6)]
            })
            .collect()
    }
}

fn at(img: &[[f32; 4]], x: u32, y: u32) -> [f32; 4] {
    img[(y * W + x) as usize]
}

fn close(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.02)
}

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

fn params(progress: f32) -> TransitionParams {
    TransitionParams {
        progress,
        ..Default::default()
    }
}

#[test]
fn fade_is_a_linear_mix() {
    let mut b = Bench::new();
    let shader = TransitionShader::Builtin(BuiltinKind::Fade);
    assert!(close(at(&b.render(shader.clone(), params(0.0)), 5, 5), RED));
    let mid = at(&b.render(shader.clone(), params(0.5)), 5, 5);
    assert!(close(mid, [0.5, 0.0, 0.5, 1.0]), "{mid:?}");
    assert!(close(at(&b.render(shader, params(1.0)), 5, 5), BLUE));
}

#[test]
fn fade_to_color_passes_through_the_color() {
    let mut b = Bench::new();
    let shader = TransitionShader::Builtin(BuiltinKind::FadeToColor);
    let black = [0.0, 0.0, 0.0, 1.0];
    let mid = at(&b.render(shader.clone(), params(0.5)), 5, 5);
    assert!(close(mid, black), "{mid:?}");
    let quarter = at(&b.render(shader, params(0.25)), 5, 5);
    assert!(close(quarter, [0.5, 0.0, 0.0, 1.0]), "{quarter:?}");
}

#[test]
fn slide_left_pushes_the_old_scene_out() {
    let mut b = Bench::new();
    let shader = TransitionShader::Builtin(BuiltinKind::Slide);
    // Halfway (ease(0.5) = 0.5): left half still red, right half already blue.
    let img = b.render(shader, params(0.5));
    assert!(close(at(&img, 8, 16), RED), "{:?}", at(&img, 8, 16));
    assert!(close(at(&img, 56, 16), BLUE), "{:?}", at(&img, 56, 16));
}

#[test]
fn swipe_down_covers_from_the_top() {
    let mut b = Bench::new();
    let shader = TransitionShader::Builtin(BuiltinKind::Swipe);
    let img = b.render(
        shader,
        TransitionParams {
            progress: 0.5,
            direction: [0.0, 1.0],
            ..Default::default()
        },
    );
    assert!(close(at(&img, 32, 4), BLUE), "top {:?}", at(&img, 32, 4));
    assert!(
        close(at(&img, 32, 28), RED),
        "bottom {:?}",
        at(&img, 32, 28)
    );
}

#[test]
fn wipe_edge_moves_with_progress() {
    let mut b = Bench::new();
    let shader = TransitionShader::Builtin(BuiltinKind::Wipe);
    let right = |p: f32| TransitionParams {
        progress: p,
        direction: [1.0, 0.0],
        softness: 0.05,
        ..Default::default()
    };
    // Wiping right: the left side is revealed first.
    let early = b.render(shader.clone(), right(0.25));
    assert!(close(at(&early, 2, 16), BLUE), "{:?}", at(&early, 2, 16));
    assert!(close(at(&early, 60, 16), RED), "{:?}", at(&early, 60, 16));
    let late = b.render(shader.clone(), right(0.9));
    assert!(close(at(&late, 50, 16), BLUE), "{:?}", at(&late, 50, 16));
    assert!(close(at(&b.render(shader, right(1.0)), 63, 16), BLUE));
}

#[test]
fn custom_shader_runs() {
    let mut b = Bench::new();
    let start = b.render(
        TransitionShader::Custom(EXAMPLE_CUSTOM_TRANSITION),
        params(0.0),
    );
    assert!(close(at(&start, 32, 16), RED));
    // The circle reveal shows blue in the center before the corners.
    let mid = b.render(
        TransitionShader::Custom(EXAMPLE_CUSTOM_TRANSITION),
        params(0.3),
    );
    assert!(close(at(&mid, 32, 16), BLUE), "{:?}", at(&mid, 32, 16));
    assert!(close(at(&mid, 0, 0), RED), "{:?}", at(&mid, 0, 0));
}

#[test]
fn invalid_custom_shader_falls_back_to_fade() {
    let mut b = Bench::new();
    let broken = "fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> { return oops; }";
    assert!(!b.renderer.prepare_custom(&b.gpu, broken));
    let targets_mid = {
        let targets = b.renderer.create_targets(&b.gpu, W, H);
        let mut encoder = b.gpu.device.create_command_encoder(&Default::default());
        clear(&mut encoder, &targets.from_view, wgpu::Color::RED);
        clear(&mut encoder, &targets.to_view, wgpu::Color::BLUE);
        let view = b.target.create_view(&Default::default());
        b.renderer.render(
            &b.gpu,
            &mut encoder,
            &targets,
            &view,
            &TransitionShader::Custom(broken),
            &params(0.5),
        );
        b.gpu.queue.submit([encoder.finish()]);
        readback::read_texture(&b.gpu, &b.target).unwrap()
    };
    let px = &targets_mid[(5 * W as usize + 5) * 8..][..8];
    let r = f16(u16::from_le_bytes([px[0], px[1]]));
    assert!((r - 0.5).abs() < 0.02, "fade fallback, got r={r}");
}
