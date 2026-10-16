//! GPU layer for eStream: device setup, texture pool, the canvas compositor
//! and output format conversion. Everything here stays on the GPU; the only
//! CPU readback is the explicit [`readback`] helper used by tests and tools.

pub mod compositor;
pub mod context;
pub mod convert;
pub mod pool;
pub mod readback;

pub use compositor::{CANVAS_FORMAT, CanvasRenderer, Compositor, DrawItem};
pub use context::{GpuContext, GpuError, GpuOptions};
pub use convert::{
    ConvertInput, Nv12Target, OutputConverter, PREVIEW_FORMAT, PREVIEW_VIEW_FORMAT,
    create_preview_texture, preview_target_view,
};
pub use pool::TexturePool;
pub use wgpu;
