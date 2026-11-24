#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The HDR → SDR pass on the GPU matches its CPU reference.

use estream_gpu::hdr::map_pixel;
use estream_gpu::{GpuContext, GpuOptions, HdrParams, HdrToSdr, readback, wgpu};

fn gpu() -> GpuContext {
    GpuContext::new_headless(&GpuOptions::default())
        .or_else(|_| {
            GpuContext::new_headless(&GpuOptions {
                force_software: true,
            })
        })
        .expect("a GPU adapter (hardware or software)")
}

/// IEEE half to f32 (normal numbers and zero are all the test produces).
fn half(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = ((bits >> 10) & 0x1f) as i32;
    let frac = (bits & 0x3ff) as f32;
    match exp {
        0 => sign * frac * 2f32.powi(-24),
        _ => sign * (1.0 + frac / 1024.0) * 2f32.powi(exp - 15),
    }
}

#[test]
fn gpu_matches_the_cpu_reference() {
    let gpu = gpu();
    // scRGB inputs: black, SDR grey, SDR white (200 nits), highlights, beyond the peak,
    // a saturated highlight and an out-of-gamut color.
    let inputs: [[f32; 4]; 8] = [
        [0.0, 0.0, 0.0, 1.0],
        [1.25, 1.25, 1.25, 1.0],
        [2.5, 2.5, 2.5, 1.0],
        [5.0, 5.0, 5.0, 1.0],
        [10.0, 10.0, 10.0, 1.0],
        [40.0, 40.0, 40.0, 1.0],
        [8.0, 4.0, 2.0, 1.0],
        [-0.3, 1.5, 0.2, 1.0],
    ];
    let width = inputs.len() as u32;
    let input = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scrgb"),
        size: wgpu::Extent3d {
            width,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let bytes: Vec<u8> = inputs
        .iter()
        .flatten()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    gpu.queue.write_texture(
        input.as_image_copy(),
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 16),
            rows_per_image: Some(1),
        },
        input.size(),
    );

    let params = HdrParams::new(200.0, 1000.0);
    let mapper = HdrToSdr::new(&gpu);
    let buffer = mapper.create_params(&gpu, &params);
    let bind = mapper.bind(&gpu, &input.create_view(&Default::default()), &buffer);
    let target = mapper.create_target(&gpu, width, 1);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    mapper.encode(
        &mut encoder,
        &bind,
        &target.create_view(&Default::default()),
    );
    gpu.queue.submit([encoder.finish()]);

    let out = readback::read_texture(&gpu, &target).unwrap();
    for (i, input) in inputs.iter().enumerate() {
        let texel = &out[i * 8..i * 8 + 8];
        let got: Vec<f32> = (0..4)
            .map(|c| half(u16::from_le_bytes([texel[c * 2], texel[c * 2 + 1]])))
            .collect();
        let want = map_pixel([input[0], input[1], input[2]], &params);
        for c in 0..3 {
            // Half floats keep about three decimal digits.
            assert!(
                (got[c] - want[c]).abs() <= 2e-3 + want[c] * 1e-3,
                "texel {i} channel {c}: got {got:?}, want {want:?}"
            );
            assert!(got[c] <= 1.0 + 1e-3, "texel {i} brighter than white");
        }
        assert_eq!(got[3], 1.0);
    }
}
