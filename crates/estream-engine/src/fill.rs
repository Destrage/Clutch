//! Fills behind fitted items: the part of an item's bounding box its source leaves empty.
//!
//! - **Frosted**: each frame the item's source is drawn to *cover* the box into a small texture
//!   (a quarter of the box), then blurred and dimmed by a filter chain at that size. The source
//!   is captured once; the extra GPU work runs on a fraction of the pixels.
//! - **Paint**: a solid color or gradient, drawn once whenever it or the box size changes.
//!
//! GPU state is kept per scene item and only reallocated when the fill or the box changes.

use crate::graph::{FlatItem, RenderGraph};
use estream_core::{BlendMode, Crop, ItemId, ItemTransform, Size, SourceId};
use estream_gpu::{
    CANVAS_FORMAT, CanvasRenderer, Compositor, DrawItem, FilterChain, FilterOp, FilterOutput,
    FilterRenderer, GpuContext, GradientRenderer, PaintTexture, wgpu,
};
use estream_project::ItemFill;
use glam::{Affine2, Vec2};
use std::collections::HashMap;
use std::sync::Arc;

/// A Frosted texture is this fraction of the box, within [`MIN_SIDE`]..=[`MAX_SIDE`] on its
/// long side. Shrinking further in one step skips so many source pixels that moving pictures
/// shimmer through the blur.
const SCALE: f32 = 0.25;
const MIN_SIDE: f32 = 96.0;
const MAX_SIDE: f32 = 640.0;
/// Blur radius (in small-texture pixels) at Blur 1.
const MAX_RADIUS: f32 = 32.0;

/// The size of a Frosted texture for a box of `bounds` canvas pixels.
pub fn frosted_size(bounds: Vec2) -> (u32, u32) {
    let bounds = bounds.max(Vec2::ONE);
    let long = bounds.max_element();
    let scale = (long * SCALE).clamp(MIN_SIDE, MAX_SIDE.max(MIN_SIDE)) / long;
    let size = (bounds * scale).round().max(Vec2::ONE);
    (size.x as u32, size.y as u32)
}

/// The filters that turn the covering picture into frost: blur, then dim (brightness times
/// `1 - dim`, as contrast and brightness of the color correction) and saturation.
pub fn frosted_ops(blur: f32, dim: f32, saturation: f32) -> Vec<FilterOp> {
    let dim = dim.clamp(0.0, 1.0);
    vec![
        FilterOp::Blur(blur.clamp(0.0, 1.0) * MAX_RADIUS),
        FilterOp::ColorCorrection([-0.5 * dim, -dim, saturation.clamp(0.0, 2.0), 0.0, 0.0, 1.0]),
    ]
}

enum Gpu {
    Frosted {
        renderer: CanvasRenderer,
        size: (u32, u32),
        view: wgpu::TextureView,
        input: wgpu::BindGroup,
        chain: FilterChain,
        _texture: wgpu::Texture,
        /// The chain produced a picture this frame.
        ready: bool,
    },
    Paint {
        binding: wgpu::BindGroup,
        _texture: PaintTexture,
    },
}

struct Slot {
    fill: ItemFill,
    /// The texture size it was made for.
    size: (u32, u32),
    source: SourceId,
    transform: ItemTransform,
    crop: Crop,
    gpu: Gpu,
    /// Still in the graph (cleared and set again on every sync).
    used: bool,
}

#[derive(Default)]
pub(crate) struct Fills {
    slots: HashMap<ItemId, Slot>,
}

impl Fills {
    /// Matches the slots to the items with a fill in `graph` (program, Preview and transition
    /// scenes). Allocates only for new or changed fills.
    pub fn sync(
        &mut self,
        gpu: &GpuContext,
        compositor: &Arc<Compositor>,
        gradients: &GradientRenderer,
        renderer: &mut FilterRenderer,
        graph: &RenderGraph,
    ) {
        for slot in self.slots.values_mut() {
            slot.used = false;
        }
        for cg in &graph.canvases {
            let lists = [Some(&cg.items), cg.preview.as_ref()]
                .into_iter()
                .flatten()
                .chain(cg.transition.as_ref().map(|t| &t.from));
            for flat in lists.flatten() {
                self.sync_item(gpu, compositor, gradients, renderer, flat);
            }
        }
        self.slots.retain(|_, s| s.used);
    }

    fn sync_item(
        &mut self,
        gpu: &GpuContext,
        compositor: &Arc<Compositor>,
        gradients: &GradientRenderer,
        renderer: &mut FilterRenderer,
        flat: &FlatItem,
    ) {
        let bounds = flat.transform.bounds;
        let size = match &flat.fill {
            ItemFill::None => return,
            _ if bounds.min_element() < 1.0 => return,
            ItemFill::Frosted { .. } => frosted_size(bounds),
            ItemFill::Paint { paint } => {
                estream_gpu::paint_size(paint, bounds.x.round() as u32, bounds.y.round() as u32)
            }
        };
        if let Some(slot) = self.slots.get_mut(&flat.item) {
            slot.used = true;
            slot.source = flat.source;
            slot.transform = flat.transform;
            slot.crop = flat.crop;
            if slot.fill == flat.fill && slot.size == size {
                return;
            }
            // Same kind and size: only the frost settings changed.
            if let (
                ItemFill::Frosted { .. },
                ItemFill::Frosted {
                    blur,
                    dim,
                    saturation,
                },
                Gpu::Frosted { chain, size: s, .. },
            ) = (&slot.fill, &flat.fill, &mut slot.gpu)
                && *s == size
            {
                chain.set_ops(renderer, gpu, frosted_ops(*blur, *dim, *saturation));
                slot.fill = flat.fill.clone();
                return;
            }
        }
        let gpu_state = match &flat.fill {
            ItemFill::None => return,
            ItemFill::Frosted {
                blur,
                dim,
                saturation,
            } => {
                let texture =
                    gpu.create_render_texture("frosted fill", size.0, size.1, CANVAS_FORMAT);
                let view = texture.create_view(&Default::default());
                let input = compositor.create_source_binding(gpu, &view);
                let mut chain = FilterChain::new();
                chain.set_ops(renderer, gpu, frosted_ops(*blur, *dim, *saturation));
                Gpu::Frosted {
                    renderer: CanvasRenderer::new(gpu, compositor.clone()),
                    size,
                    view,
                    input,
                    chain,
                    _texture: texture,
                    ready: false,
                }
            }
            ItemFill::Paint { paint } => {
                let texture = gradients.render(gpu, paint, size.0, size.1);
                let binding = compositor.create_source_binding(gpu, &texture.view);
                Gpu::Paint {
                    binding,
                    _texture: texture,
                }
            }
        };
        self.slots.insert(
            flat.item,
            Slot {
                fill: flat.fill.clone(),
                size,
                source: flat.source,
                transform: flat.transform,
                crop: flat.crop,
                gpu: gpu_state,
                used: true,
            },
        );
    }

    /// Records this frame's Frosted passes. `frame` gives a source's current picture.
    pub fn run<'a>(
        &mut self,
        gpu: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        filters: &FilterRenderer,
        frame: impl Fn(SourceId) -> Option<(&'a wgpu::BindGroup, Size)>,
    ) {
        for slot in self.slots.values_mut() {
            let Gpu::Frosted {
                renderer,
                size,
                view,
                input,
                chain,
                ready,
                ..
            } = &mut slot.gpu
            else {
                continue;
            };
            *ready = false;
            let Some((binding, source_size)) = frame(slot.source) else {
                continue;
            };
            let (uv_min, uv_max) = slot.transform.cover_uv(slot.crop, source_size);
            if uv_max.x <= uv_min.x || uv_max.y <= uv_min.y {
                continue;
            }
            let target = Vec2::new(size.0 as f32, size.1 as f32);
            renderer.render(
                gpu,
                encoder,
                view,
                target,
                wgpu::Color::TRANSPARENT,
                &[DrawItem {
                    binding,
                    matrix: Affine2::from_scale(target),
                    uv_min,
                    uv_max,
                    opacity: 1.0,
                    blend: BlendMode::Normal,
                }],
            );
            let source = FilterOutput {
                binding: input,
                width: size.0,
                height: size.1,
                texture: None,
            };
            *ready = chain.run(filters, gpu, encoder, source, 0.0).is_some();
        }
    }

    /// What to draw under `flat`: its fill stretched over the bounding box.
    pub fn item(&self, flat: &FlatItem) -> Option<DrawItem<'_>> {
        if flat.fill.is_none() {
            return None;
        }
        let slot = self.slots.get(&flat.item)?;
        let binding = match &slot.gpu {
            Gpu::Frosted { chain, ready, .. } => {
                if !*ready {
                    return None;
                }
                chain.output()?.binding
            }
            Gpu::Paint { binding, .. } => binding,
        };
        Some(DrawItem {
            binding,
            matrix: flat.parent * flat.transform.bounds_matrix(),
            uv_min: Vec2::ZERO,
            uv_max: Vec2::ONE,
            opacity: flat.opacity,
            blend: flat.blend,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frosted_texture_is_a_quarter_within_limits() {
        assert_eq!(frosted_size(Vec2::new(1080.0, 1920.0)), (270, 480));
        assert_eq!(frosted_size(Vec2::new(2160.0, 3840.0)), (360, 640));
        assert_eq!(frosted_size(Vec2::new(100.0, 200.0)), (48, 96));
    }

    #[test]
    fn dim_multiplies_brightness() {
        // Color correction: s' = (s - 0.5)(1 + contrast) + 0.5 + brightness.
        let [FilterOp::Blur(r), FilterOp::ColorCorrection(c)] = frosted_ops(0.5, 0.4, 1.0)[..]
        else {
            panic!("blur then color correction")
        };
        assert_eq!(r, 16.0);
        for s in [0.0f32, 0.3, 1.0] {
            let out = (s - 0.5) * (1.0 + c[1]) + 0.5 + c[0];
            assert!((out - s * 0.6).abs() < 1e-6);
        }
    }
}
