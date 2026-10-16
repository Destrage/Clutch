use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GpuError {
    #[error("no suitable GPU adapter found: {0}")]
    NoAdapter(#[from] wgpu::RequestAdapterError),
    #[error("failed to create GPU device: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
    #[error("GPU readback failed: {0}")]
    Readback(String),
}

#[derive(Clone, Debug, Default)]
pub struct GpuOptions {
    /// Use a software adapter (WARP / llvmpipe). Useful for CI without a GPU.
    pub force_software: bool,
}

/// Shared GPU device. Cheap to clone; the engine and UI share one instance.
#[derive(Clone, Debug)]
pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub info: Arc<wgpu::AdapterInfo>,
}

impl GpuContext {
    /// Creates a device without any window surface (headless).
    pub fn new_headless(options: &GpuOptions) -> Result<Self, GpuError> {
        pollster::block_on(Self::new_async(options))
    }

    async fn new_async(options: &GpuOptions) -> Result<Self, GpuError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: options.force_software,
                compatible_surface: None,
                ..Default::default()
            })
            .await?;
        let info = adapter.get_info();
        tracing::info!(
            adapter = %info.name,
            backend = ?info.backend,
            driver = %info.driver_info,
            "GPU adapter selected"
        );
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("estream"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
                memory_hints: wgpu::MemoryHints::Performance,
                ..Default::default()
            })
            .await?;
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            info: Arc::new(info),
        })
    }

    /// Creates a 2D texture usable as a render target and shader input.
    pub fn create_render_texture(
        &self,
        label: &str,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }
}
