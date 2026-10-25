#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The preview bridge between the engine's device and the UI's (Windows, DX12 GPUs only).

use estream_gpu::{
    GpuContext, GpuOptions, create_preview_texture, preview_target_view, readback, wgpu,
};

/// Clears `texture` (through its sRGB view) to `color` on the engine's queue.
fn draw(engine: &GpuContext, texture: &wgpu::Texture, color: wgpu::Color) {
    let view = preview_target_view(texture);
    let mut encoder = engine.device.create_command_encoder(&Default::default());
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(color),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    });
    engine.queue.submit([encoder.finish()]);
}

#[test]
fn previews_drawn_on_the_engine_device_reach_the_ui_device() {
    let (ui, engine) = GpuContext::new_pair(&GpuOptions::default()).expect("a GPU adapter");
    let Some(bridge) = engine.bridge.clone() else {
        eprintln!("no separate engine device here (not DX12); nothing to test");
        return;
    };
    assert!(
        ui.bridge.is_none(),
        "only the engine's context carries the bridge"
    );
    let (shared, shared_ui) = bridge.create_preview(64, 32).expect("a shared preview");
    let target = create_preview_texture(&ui, 64, 32);

    // Two rounds, so the second draw has to wait for the first copy to be released.
    for (color, expect) in [
        (wgpu::Color::RED, [255, 0, 0, 255]),
        (wgpu::Color::BLUE, [0, 0, 255, 255]),
    ] {
        draw(&engine, &shared, color);
        let frame = bridge.frame_submitted();
        let copy = bridge.copy_to_ui(&ui, &shared_ui, &target, frame);
        let pixels = readback::read_texture(&ui, &target).unwrap();
        assert_eq!(&pixels[..4], &expect, "the UI sees the engine's frame");
        assert!(
            bridge.copied(copy),
            "the copy has finished once its result was read"
        );
    }
}
