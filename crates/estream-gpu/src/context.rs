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
        // Prefer the platform's native API (DX12 on Windows: needed for D3D11 capture
        // interop, and avoids third-party Vulkan layers; Vulkan on Linux, then GL for old
        // drivers), then fall back to any backend. `WGPU_BACKEND` in the environment
        // overrides this.
        let preferred = if wgpu::Backends::from_env().is_some() {
            vec![None]
        } else if cfg!(windows) {
            vec![Some(wgpu::Backends::DX12), None]
        } else if cfg!(target_os = "linux") {
            vec![Some(wgpu::Backends::VULKAN), Some(wgpu::Backends::GL), None]
        } else {
            vec![None]
        };
        let mut result = None;
        for backends in preferred {
            let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
            if let Some(backends) = backends {
                desc.backends = backends;
            }
            let instance = wgpu::Instance::new(desc);
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    force_fallback_adapter: options.force_software,
                    compatible_surface: None,
                    ..Default::default()
                })
                .await;
            let ok = adapter.is_ok();
            result = Some((instance, adapter));
            if ok {
                break;
            }
        }
        let Some((instance, adapter)) = result else {
            unreachable!("at least one backend set is tried")
        };
        let adapter = adapter?;
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
        // A live show must not abort on a GPU validation error: log it instead.
        let dev = device.clone();
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
            tracing::error!(error = %e, "GPU validation error");
            #[cfg(windows)]
            log_device_removed_reason(&dev);
        }));
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

#[cfg(windows)]
fn log_device_removed_reason(device: &wgpu::Device) {
    // SAFETY: as_hal reads the underlying raw device pointer when using the Dx12 backend.
    if let Some(hal) = unsafe { device.as_hal::<wgpu::hal::api::Dx12>() } {
        let raw = hal.raw_device();
        // SAFETY: GetDeviceRemovedReason queries the DXGI device status on the D3D12 device.
        let reason = unsafe { raw.GetDeviceRemovedReason() };
        if let Err(err) = reason {
            let code = err.code().0 as u32;
            let name = match code {
                0x887A0006 => "DXGI_ERROR_DEVICE_HUNG (GPU stopped responding to commands)",
                0x887A0005 => {
                    "DXGI_ERROR_DEVICE_REMOVED (GPU physically removed or driver crashed)"
                }
                0x887A0007 => {
                    "DXGI_ERROR_DEVICE_RESET (GPU driver reset due to timeout or invalid command)"
                }
                0x887A002B => {
                    "DXGI_ERROR_DRIVER_INTERNAL_ERROR (GPU driver encountered an internal error)"
                }
                0x887A0001 => "DXGI_ERROR_INVALID_CALL (application provided invalid parameters)",
                _ => "Unknown DXGI device removed error",
            };
            tracing::error!(
                code = format!("{code:#010x}"),
                name,
                "GPU device was lost or removed"
            );
        }
    }
}
