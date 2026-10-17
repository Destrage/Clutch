#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Headless pixel tests for video filters.

use estream_gpu::{
    Compositor, CubeLut, FilterChain, FilterOp, FilterOutput, FilterRenderer, GpuContext,
    GpuOptions, readback, wgpu,
};
use std::sync::Arc;

fn gpu() -> GpuContext {
    GpuContext::new_headless(&GpuOptions::default())
        .or_else(|_| {
            GpuContext::new_headless(&GpuOptions {
                force_software: true,
            })
        })
        .expect("a GPU adapter (hardware or software)")
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

struct Image {
    width: u32,
    px: Vec<[f32; 4]>,
}

impl Image {
    fn at(&self, x: u32, y: u32) -> [f32; 4] {
        self.px[(y * self.width + x) as usize]
    }
}

struct Bench {
    gpu: GpuContext,
    compositor: Arc<Compositor>,
    renderer: FilterRenderer,
}

impl Bench {
    fn new() -> Self {
        let gpu = gpu();
        let compositor = Compositor::new(&gpu);
        let renderer = FilterRenderer::new(&gpu, compositor.clone());
        Self {
            gpu,
            compositor,
            renderer,
        }
    }

    /// An sRGB source texture from a per-pixel color function, and its binding.
    fn source(
        &self,
        w: u32,
        h: u32,
        color: impl Fn(u32, u32) -> [u8; 4],
    ) -> (wgpu::Texture, wgpu::BindGroup) {
        let texture = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut data = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                data.extend(color(x, y));
            }
        }
        self.gpu.queue.write_texture(
            texture.as_image_copy(),
            &data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&Default::default());
        let binding = self.compositor.create_source_binding(&self.gpu, &view);
        (texture, binding)
    }

    /// Runs `chain` over the source; `None` if the chain passed everything through.
    fn run(
        &self,
        chain: &mut FilterChain,
        binding: &wgpu::BindGroup,
        w: u32,
        h: u32,
    ) -> Option<Image> {
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        let out = chain.run(
            &self.renderer,
            &self.gpu,
            &mut encoder,
            FilterOutput {
                binding,
                width: w,
                height: h,
                texture: None,
            },
            0.0,
        )?;
        let (width, texture) = (out.width, out.texture.unwrap().clone());
        self.gpu.queue.submit([encoder.finish()]);
        let bytes = readback::read_texture(&self.gpu, &texture).unwrap();
        let px = bytes
            .as_chunks::<8>()
            .0
            .iter()
            .map(|p| {
                let c = |i: usize| f16(u16::from_le_bytes([p[i], p[i + 1]]));
                [c(0), c(2), c(4), c(6)]
            })
            .collect();
        Some(Image { width, px })
    }

    fn chain(&mut self, ops: Vec<FilterOp>) -> FilterChain {
        let mut chain = FilterChain::new();
        chain.set_ops(&mut self.renderer, &self.gpu, ops);
        chain
    }
}

/// Left half green, right half red.
fn green_red(x: u32, _: u32) -> [u8; 4] {
    if x < 4 {
        [0, 255, 0, 255]
    } else {
        [255, 0, 0, 255]
    }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.03
}

#[test]
fn chroma_key_removes_green_and_keeps_red() {
    let mut b = Bench::new();
    let (_t, src) = b.source(8, 8, green_red);
    let mut chain = b.chain(vec![
        FilterOp::from_params("chroma_key", |_| None, None).unwrap(),
    ]);
    let img = b.run(&mut chain, &src, 8, 8).unwrap();
    assert!(img.at(1, 4)[3] < 0.02, "green keyed: {:?}", img.at(1, 4));
    let red = img.at(6, 4);
    assert!(
        close(red[3], 1.0) && close(red[0], 1.0),
        "red kept: {red:?}"
    );
}

#[test]
fn color_correction_desaturates_and_fades() {
    let mut b = Bench::new();
    let (_t, src) = b.source(8, 8, green_red);
    let mut chain = b.chain(vec![FilterOp::ColorCorrection([
        0.0, 0.0, 0.0, 0.0, 0.0, 0.5,
    ])]);
    let px = b.run(&mut chain, &src, 8, 8).unwrap().at(6, 4);
    assert!(close(px[0], px[1]) && close(px[1], px[2]), "grey: {px:?}");
    assert!(close(px[3], 0.5), "opacity: {px:?}");
}

#[test]
fn luma_key_cuts_dark_pixels() {
    let mut b = Bench::new();
    let (_t, src) = b.source(8, 8, |x, _| {
        if x < 4 {
            [0, 0, 0, 255]
        } else {
            [255, 255, 255, 255]
        }
    });
    let mut chain = b.chain(vec![FilterOp::LumaKey([0.5, 1.0, 0.05])]);
    let img = b.run(&mut chain, &src, 8, 8).unwrap();
    assert!(img.at(1, 4)[3] < 0.02);
    assert!(close(img.at(6, 4)[3], 1.0));
}

#[test]
fn crop_pad_changes_size_and_places_the_source() {
    let mut b = Bench::new();
    let (_t, src) = b.source(8, 8, green_red);
    // Pad 10 px on the left, crop 2 px from the bottom.
    let mut chain = b.chain(vec![FilterOp::CropPad([-10.0, 0.0, 0.0, 2.0])]);
    let img = b.run(&mut chain, &src, 8, 8).unwrap();
    assert_eq!(img.width, 18);
    assert_eq!(img.px.len(), 18 * 6);
    assert!(img.at(5, 3)[3] < 0.02, "padding is transparent");
    let first = img.at(10, 3);
    assert!(
        close(first[1], 1.0) && close(first[3], 1.0),
        "source starts at x=10: {first:?}"
    );
}

#[test]
fn blur_spreads_symmetrically() {
    let mut b = Bench::new();
    let (_t, src) = b.source(
        33,
        4,
        |x, _| if x == 16 { [255; 4] } else { [0, 0, 0, 255] },
    );
    let mut chain = b.chain(vec![FilterOp::Blur(4.0)]);
    let img = b.run(&mut chain, &src, 33, 4).unwrap();
    let (l, c, r) = (img.at(15, 2)[0], img.at(16, 2)[0], img.at(17, 2)[0]);
    assert!(c < 0.9 && c > l, "center softened: {c}");
    assert!(l > 0.01 && close(l, r), "symmetric: {l} {r}");
    assert!(img.at(2, 2)[0] < 0.001, "far pixels stay black");
}

#[test]
fn lut_identity_and_inverse() {
    let mut b = Bench::new();
    let (_t, src) = b.source(8, 8, green_red);
    let lut = |f: &dyn Fn(u8) -> u8| CubeLut {
        size: 2,
        rgba: (0..8u32)
            .flat_map(|i| {
                let c = |bit: u32| if i & bit != 0 { 255 } else { 0 };
                [f(c(1)), f(c(2)), f(c(4)), 255]
            })
            .collect(),
    };
    let mut chain = b.chain(vec![FilterOp::Lut(1.0)]);
    assert!(
        b.run(&mut chain, &src, 8, 8).is_none(),
        "no LUT yet: pass-through"
    );
    chain.set_lut(&b.renderer, &b.gpu, 0, Some(&lut(&|v| v)));
    let same = b.run(&mut chain, &src, 8, 8).unwrap().at(6, 4);
    assert!(
        close(same[0], 1.0) && close(same[1], 0.0),
        "identity: {same:?}"
    );
    chain.set_lut(&b.renderer, &b.gpu, 0, Some(&lut(&|v| 255 - v)));
    let inv = b.run(&mut chain, &src, 8, 8).unwrap().at(6, 4);
    assert!(
        close(inv[0], 0.0) && close(inv[1], 1.0) && close(inv[2], 1.0),
        "inverted: {inv:?}"
    );
}

#[test]
fn image_mask_uses_alpha() {
    let mut b = Bench::new();
    let (_t, src) = b.source(8, 8, green_red);
    let mut chain = b.chain(vec![FilterOp::ImageMask(false)]);
    let mask: Vec<u8> = (0..2)
        .flat_map(|x| [255, 255, 255, if x == 0 { 0 } else { 255 }])
        .collect();
    chain.set_mask(&b.renderer, &b.gpu, 0, Some((2, 1, &mask)));
    let img = b.run(&mut chain, &src, 8, 8).unwrap();
    assert!(img.at(0, 4)[3] < 0.05, "masked: {:?}", img.at(0, 4));
    assert!(close(img.at(7, 4)[3], 1.0));
}

#[test]
fn custom_filter_runs_and_chains() {
    let mut b = Bench::new();
    let (_t, src) = b.source(8, 8, green_red);
    let swap = "fn effect(uv: vec2<f32>) -> vec4<f32> {\n    let c = source(uv);\n    return vec4<f32>(c.g, c.r, c.b, c.a);\n}";
    let mut chain = b.chain(vec![
        FilterOp::Custom {
            code: swap.into(),
            params: [0.0; 4],
        },
        FilterOp::ColorCorrection([0.0, 0.0, 1.0, 0.0, 0.0, 1.0]),
    ]);
    let img = b.run(&mut chain, &src, 8, 8).unwrap();
    let px = img.at(1, 4);
    assert!(
        close(px[0], 1.0) && close(px[1], 0.0),
        "green became red: {px:?}"
    );
}

#[test]
fn invalid_custom_filter_passes_through() {
    let mut b = Bench::new();
    let (_t, src) = b.source(8, 8, green_red);
    let mut chain = b.chain(vec![FilterOp::Custom {
        code: "fn effect(uv: vec2<f32>) -> vec4<f32> { return nope; }".into(),
        params: [0.0; 4],
    }]);
    assert!(b.run(&mut chain, &src, 8, 8).is_none());
}
