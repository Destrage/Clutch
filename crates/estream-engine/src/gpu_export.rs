//! Frames handed to hardware encoders on the GPU, without a CPU readback.
//!
//! The engine stays platform-neutral: an output crate implements [`GpuExport`] for its
//! platform (D3D11 on Windows) and passes it in [`crate::EngineConfig::gpu_export`]. The
//! engine renders NV12 into the planes the export creates and hands consumers that asked for
//! GPU frames a [`GpuFrame`] instead of CPU bytes.

use estream_gpu::{GpuContext, Nv12Target};
use std::any::Any;
use std::sync::Arc;

/// Platform-specific planes a consumer reads (downcast to the export's own type).
pub type PlanesHandle = Arc<dyn Any + Send + Sync>;

/// One slot of NV12 planes: the wgpu targets the engine renders into, and the handle the
/// consumer receives. A slot is free again once every frame holding its handle is dropped.
pub struct ExportPlanes {
    pub target: Nv12Target,
    pub handle: PlanesHandle,
}

/// Hooks a platform output layer provides so frames can stay on the GPU.
pub trait GpuExport: Send + Sync {
    /// Creates a slot of `width × height` NV12 planes (even sizes), or `None` if this GPU
    /// can't share them.
    fn create_planes(&self, gpu: &GpuContext, width: u32, height: u32) -> Option<ExportPlanes>;

    /// Queues on wgpu's queue (GPU side, no CPU wait) a wait for the consumer's last read of
    /// `handle`, before the engine renders into those planes again.
    fn wait_released(&self, gpu: &GpuContext, handle: &PlanesHandle);

    /// Signals after a frame's work was submitted. Consumers wait (on the GPU) for the
    /// returned value before reading planes delivered with it.
    fn signal_ready(&self, gpu: &GpuContext) -> u64;
}

/// A frame left on the GPU: the planes it was rendered into and the fence value that
/// finishes it.
#[derive(Clone)]
pub struct GpuFrame {
    pub planes: PlanesHandle,
    pub ready: u64,
}

impl std::fmt::Debug for GpuFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GpuFrame(ready {})", self.ready)
    }
}
