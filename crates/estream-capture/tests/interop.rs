#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use estream_capture::windows::d3d::D3D11Interop;
use estream_gpu::{GpuContext, GpuOptions, readback, wgpu};

/// D3D11 writes a color into a shared texture; wgpu (D3D12) copies it out.
#[test]
fn d3d11_texture_is_visible_to_wgpu() {
    let gpu = GpuContext::new_headless(&GpuOptions::default()).unwrap();
    let Ok(interop) = D3D11Interop::new(&gpu) else {
        eprintln!("skipping: wgpu is not on D3D12");
        return;
    };
    let shared = interop.create_shared(64, 32).unwrap();
    shared.clear(&interop, [1.0, 0.5, 0.0, 1.0]).unwrap();
    interop.flush_and_wait().unwrap();

    // Copy through a regular wgpu texture (shared ones are sample-only).
    let compositor = estream_gpu::Compositor::new(&gpu);
    let binding =
        compositor.create_source_binding(&gpu, &shared.texture.create_view(&Default::default()));
    let canvas = gpu.create_render_texture("c", 64, 32, estream_gpu::CANVAS_FORMAT);
    let mut renderer = estream_gpu::CanvasRenderer::new(&gpu, compositor.clone());
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    renderer.render(
        &gpu,
        &mut enc,
        &canvas.create_view(&Default::default()),
        glam_vec(64.0, 32.0),
        wgpu::Color::BLACK,
        &[estream_gpu::DrawItem {
            binding: &binding,
            matrix: glam_affine(64.0, 32.0),
            uv_min: [0.0, 0.0].into(),
            uv_max: [1.0, 1.0].into(),
            opacity: 1.0,
            blend: estream_core::BlendMode::Normal,
        }],
    );
    let preview = estream_gpu::create_preview_texture(&gpu, 64, 32);
    let converter = estream_gpu::OutputConverter::new(&gpu);
    let input = converter.create_input(&gpu, &canvas.create_view(&Default::default()));
    converter.to_srgb(
        &mut enc,
        &input,
        &estream_gpu::preview_target_view(&preview),
    );
    gpu.queue.submit([enc.finish()]);
    let pixels = readback::read_texture(&gpu, &preview).unwrap();
    // D3D11 wrote UNORM (1.0, 0.5, 0.0); sampled as sRGB and re-encoded → same bytes.
    let p = &pixels[(16 * 64 + 32) * 4..][..4];
    assert!(
        (p[0] as i32 - 255).abs() <= 1 && (p[1] as i32 - 128).abs() <= 1 && p[2] <= 1,
        "{p:?}"
    );
}

fn glam_vec(x: f32, y: f32) -> estream_gpu::glam::Vec2 {
    estream_gpu::glam::Vec2::new(x, y)
}

fn glam_affine(w: f32, h: f32) -> estream_gpu::glam::Affine2 {
    estream_gpu::glam::Affine2::from_scale(glam_vec(w, h))
}
