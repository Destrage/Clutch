use crate::PreviewBridge;
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

/// A GPU device and its queue. Cheap to clone.
#[derive(Clone, Debug)]
pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub info: Arc<wgpu::AdapterInfo>,
    /// On the engine's device when the UI has its own: shares previews with the UI.
    pub bridge: Option<Arc<PreviewBridge>>,
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
        let (device, queue) = request_device(&adapter, "estream").await?;
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            info: Arc::new(info),
            bridge: None,
        })
    }

    /// A device for the UI and one for the engine, so UI presents and window resizes never
    /// wait behind compositor submits. Both are on one adapter and share previews through a
    /// [`PreviewBridge`] (Windows, DX12). Elsewhere, or if that fails, both get one device.
    /// Returns `(ui, engine)`.
    pub fn new_pair(options: &GpuOptions) -> Result<(Self, Self), GpuError> {
        let ui = Self::new_headless(options)?;
        if !cfg!(windows) || ui.info.backend != wgpu::Backend::Dx12 {
            return Ok((ui.clone(), ui));
        }
        let (device, queue) = match pollster::block_on(request_device(&ui.adapter, "engine")) {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!(error = %e, "no separate engine GPU device; the UI shares its own");
                return Ok((ui.clone(), ui));
            }
        };
        let mut engine = Self {
            device,
            queue,
            ..ui.clone()
        };
        match PreviewBridge::new(&engine, &ui) {
            Some(bridge) => {
                engine.bridge = Some(Arc::new(bridge));
                tracing::info!("the engine renders on its own GPU device");
                Ok((ui, engine))
            }
            None => Ok((ui.clone(), ui)),
        }
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

    /// Asks the OS GPU scheduler to favour this process, so a game using the whole GPU can't
    /// starve the compositor: high priority with hardware-accelerated GPU scheduling, realtime
    /// without. Skipped on Intel GPUs. Usually needs administrator rights; failure is harmless.
    pub fn raise_scheduling_priority(&self) {
        #[cfg(windows)]
        raise_scheduling_priority(self);
    }
}

async fn request_device(
    adapter: &wgpu::Adapter,
    label: &str,
) -> Result<(wgpu::Device, wgpu::Queue), GpuError> {
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some(label),
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
        #[cfg(not(windows))]
        let _ = &dev;
    }));
    Ok((device, queue))
}

#[cfg(windows)]
const INTEL_VENDOR: u32 = 0x8086;

#[cfg(windows)]
fn raise_scheduling_priority(gpu: &GpuContext) {
    use windows::Wdk::Graphics::Direct3D::{
        D3DKMT_SCHEDULINGPRIORITYCLASS_HIGH, D3DKMT_SCHEDULINGPRIORITYCLASS_REALTIME,
        D3DKMTSetProcessSchedulingPriorityClass,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;

    if gpu.info.vendor == INTEL_VENDOR {
        return;
    }
    // SAFETY: as_hal reads the underlying raw device pointer when using the Dx12 backend.
    let Some(luid) = (unsafe { gpu.device.as_hal::<wgpu::hal::api::Dx12>() })
        // SAFETY: GetAdapterLuid has no preconditions on a valid device.
        .map(|hal| unsafe { hal.raw_device().GetAdapterLuid() })
    else {
        return;
    };
    let hags = hardware_scheduling_enabled(luid);
    let class = if hags {
        D3DKMT_SCHEDULINGPRIORITYCLASS_HIGH
    } else {
        D3DKMT_SCHEDULINGPRIORITYCLASS_REALTIME
    };
    // SAFETY: the pseudo handle from GetCurrentProcess is always valid for this process.
    let status = unsafe { D3DKMTSetProcessSchedulingPriorityClass(GetCurrentProcess(), class) };
    if status.is_ok() {
        tracing::info!(hags, "GPU scheduling priority raised");
    } else {
        tracing::debug!(
            status = status.0,
            hags,
            "GPU scheduling priority not raised (not admin?)"
        );
    }
}

/// Whether hardware-accelerated GPU scheduling is on for the adapter.
#[cfg(windows)]
fn hardware_scheduling_enabled(luid: windows::Win32::Foundation::LUID) -> bool {
    use windows::Wdk::Graphics::Direct3D::{
        D3DKMT_CLOSEADAPTER, D3DKMT_OPENADAPTERFROMLUID, D3DKMT_QUERYADAPTERINFO,
        D3DKMT_WDDM_2_7_CAPS, D3DKMTCloseAdapter, D3DKMTOpenAdapterFromLuid,
        D3DKMTQueryAdapterInfo, KMTQAITYPE_WDDM_2_7_CAPS,
    };
    /// `HwSchEnabled` in D3DKMT_WDDM_2_7_CAPS (bit 0 is `HwSchSupported`).
    const HW_SCH_ENABLED: u32 = 1 << 1;

    let mut open = D3DKMT_OPENADAPTERFROMLUID {
        AdapterLuid: luid,
        hAdapter: 0,
    };
    // SAFETY: `open` is a valid, writable struct; the handle is closed below.
    if unsafe { D3DKMTOpenAdapterFromLuid(&mut open) }.is_err() {
        return false;
    }
    let mut caps = D3DKMT_WDDM_2_7_CAPS::default();
    let mut query = D3DKMT_QUERYADAPTERINFO {
        hAdapter: open.hAdapter,
        Type: KMTQAITYPE_WDDM_2_7_CAPS,
        pPrivateDriverData: (&raw mut caps).cast(),
        PrivateDriverDataSize: size_of::<D3DKMT_WDDM_2_7_CAPS>() as u32,
    };
    // SAFETY: the query points at a caps struct of the size it declares.
    let ok = unsafe { D3DKMTQueryAdapterInfo(&mut query) }.is_ok();
    let close = D3DKMT_CLOSEADAPTER {
        hAdapter: open.hAdapter,
    };
    // SAFETY: closes the adapter handle opened above.
    let _ = unsafe { D3DKMTCloseAdapter(&close) };
    // SAFETY: every bit pattern of the union is a valid u32.
    ok && unsafe { caps.Anonymous.Value } & HW_SCH_ENABLED != 0
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
