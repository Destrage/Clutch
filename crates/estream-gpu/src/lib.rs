//! GPU layer for Clutch: device setup, texture pool, the canvas compositor
//! and output format conversion. Everything here stays on the GPU; the only
//! CPU readback is the explicit [`readback`] helper used by tests and tools.

pub mod compositor;
pub mod context;
pub mod convert;
pub mod filter;
pub mod pool;
pub mod readback;
pub mod transition;
mod user_shader;
pub mod yuv;

pub use compositor::{CANVAS_FORMAT, CanvasRenderer, Compositor, DrawItem};
pub use context::{GpuContext, GpuError, GpuOptions};
pub use convert::{
    ConvertInput, Nv12Target, OutputConverter, PREVIEW_FORMAT, PREVIEW_VIEW_FORMAT,
    create_preview_texture, preview_target_view,
};
pub use filter::{
    CubeLut, EXAMPLE_CUSTOM_FILTER, FilterChain, FilterOp, FilterOutput, FilterRenderer,
    parse_cube, validate_custom_filter,
};
pub use glam;
pub use pool::TexturePool;
pub use transition::{
    BuiltinKind, EXAMPLE_CUSTOM_TRANSITION, TransitionParams, TransitionRenderer, TransitionShader,
    TransitionTargets, validate_custom_transition,
};
pub use wgpu;
pub use yuv::{Nv12Frame, YUV_OUTPUT_FORMAT, YuvConverter, YuvMatrix};
