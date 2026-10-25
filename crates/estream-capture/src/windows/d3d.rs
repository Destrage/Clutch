//! D3D11 ↔ wgpu (D3D12) interop.
//!
//! Capture APIs on Windows hand out `ID3D11Texture2D`s. We create a D3D11 device
//! on the same adapter as wgpu, copy captured frames into textures shared via NT
//! handles, and open those textures in wgpu's D3D12 device — no CPU readback.
//!
//! Ordering is done on the GPU with two fences, so neither side waits on the CPU:
//! capture copies signal a shared fence that wgpu's queue waits on before sampling,
//! and wgpu's queue signals a second fence when it no longer reads a slot, which the
//! capture side checks (without blocking) before writing that slot again.

use crate::CaptureError;
use estream_gpu::{GpuContext, wgpu};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use windows::Win32::Foundation::{CloseHandle, GENERIC_ALL, HMODULE};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Direct3D12::{
    D3D12_FENCE_FLAG_NONE, ID3D12CommandQueue, ID3D12Device, ID3D12Fence, ID3D12Resource,
};
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::core::Interface;

/// Format of shared capture textures in wgpu (sampled as sRGB → linear).
pub const SHARED_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8UnormSrgb;

const INTEL_VENDOR: u32 = 0x8086;
/// Highest value `IDXGIDevice::SetGPUThreadPriority` accepts.
const GPU_THREAD_PRIORITY: i32 = 7;

/// A D3D11 device on the same GPU as wgpu, plus wgpu's D3D12 device.
#[derive(Clone)]
pub struct D3D11Interop {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    d3d12: ID3D12Device,
    gpu: GpuContext,
    /// `None` without D3D11.4 fences (old drivers): copies are then waited for on the CPU.
    fences: Option<Arc<Fences>>,
}

/// The two fences that order capture copies and wgpu reads on the GPU.
struct Fences {
    context: ID3D11DeviceContext4,
    /// Signalled by D3D11 after a capture copy; opened on D3D12 as `copied_12`.
    copied_11: ID3D11Fence,
    copied_12: ID3D12Fence,
    copied: AtomicU64,
    /// Signalled by wgpu's queue once earlier submissions no longer read a slot.
    released: ID3D12Fence,
    release: AtomicU64,
}

// SAFETY: the D3D11 device is created with multithread protection enabled
// (`ID3D11Multithread::SetMultithreadProtected`), which makes the immediate
// context (and its `ID3D11DeviceContext4` view) safe to use from several threads;
// D3D12 devices and fences are free-threaded.
unsafe impl Send for D3D11Interop {}
unsafe impl Sync for D3D11Interop {}

impl std::fmt::Debug for D3D11Interop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("D3D11Interop")
    }
}

impl D3D11Interop {
    /// Creates the interop device. Fails if wgpu is not running on D3D12.
    pub fn new(gpu: &GpuContext) -> Result<Self, CaptureError> {
        // SAFETY: we only read the raw device pointer and clone the COM reference.
        let d3d12 = unsafe { gpu.device.as_hal::<wgpu::hal::api::Dx12>() }
            .map(|hal| hal.raw_device().clone())
            .ok_or_else(|| CaptureError::Interop("wgpu is not using the D3D12 backend".into()))?;

        // SAFETY: plain COM calls with valid arguments; outputs are checked.
        unsafe {
            let luid = d3d12.GetAdapterLuid();
            let factory: IDXGIFactory4 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))?;
            let adapter: IDXGIAdapter1 = factory.EnumAdapterByLuid(luid)?;
            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
            let device: ID3D11Device =
                device.ok_or_else(|| CaptureError::Interop("no D3D11 device".into()))?;
            let context =
                context.ok_or_else(|| CaptureError::Interop("no D3D11 context".into()))?;
            let mt: ID3D11Multithread = device.cast()?;
            let _ = mt.SetMultithreadProtected(true);
            // Keep capture copies flowing while a game saturates the GPU (not on Intel GPUs;
            // a raised value needs administrator rights, so failure is expected and harmless).
            if gpu.info.vendor != INTEL_VENDOR {
                let dxgi: IDXGIDevice = device.cast()?;
                if let Err(e) = dxgi.SetGPUThreadPriority(GPU_THREAD_PRIORITY) {
                    tracing::debug!(error = %e, "capture GPU thread priority not raised");
                }
            }
            let fences = match Fences::new(&device, &context, &d3d12) {
                Ok(f) => Some(Arc::new(f)),
                Err(e) => {
                    tracing::info!(error = %e, "capture fences unavailable; copies are waited for on the CPU");
                    None
                }
            };
            Ok(Self {
                device,
                context,
                d3d12,
                gpu: gpu.clone(),
                fences,
            })
        }
    }

    pub fn gpu(&self) -> &GpuContext {
        &self.gpu
    }

    /// Whether copies are ordered by GPU fences (else waited for on the CPU).
    pub fn has_fences(&self) -> bool {
        self.fences.is_some()
    }

    /// Makes the copies issued so far visible to wgpu. With fences this signals and flushes
    /// without waiting and returns the value wgpu must wait for; without, it blocks until the
    /// copies finished and returns 0.
    pub fn finish_copies(&self) -> Result<u64, CaptureError> {
        let Some(f) = &self.fences else {
            self.flush_and_wait()?;
            return Ok(0);
        };
        let value = f.copied.fetch_add(1, Ordering::Relaxed) + 1;
        // SAFETY: the fence and context belong to this device; the context is multithread-protected.
        unsafe {
            f.context.Signal(&f.copied_11, value)?;
            self.context.Flush();
        }
        Ok(value)
    }

    /// Makes wgpu's queue wait (on the GPU) until the copy that signalled `value` is done.
    /// Call before submitting work that samples the copied texture.
    pub fn wait_for_copy(&self, value: u64) {
        let Some(f) = &self.fences else { return };
        if value == 0 {
            return;
        }
        if let Some(queue) = self.raw_queue() {
            // SAFETY: command queues are free-threaded; the fence is open on this device.
            if let Err(e) = unsafe { queue.Wait(&f.copied_12, value) } {
                tracing::debug!(error = %e, "capture fence wait not queued");
            }
        }
    }

    /// Marks everything wgpu submitted so far as done with the slots it read. Returns the
    /// value to pass to [`D3D11Interop::released`] (0 without fences: nothing to wait for).
    pub fn release_reads(&self) -> u64 {
        let Some(f) = &self.fences else { return 0 };
        let Some(queue) = self.raw_queue() else {
            return 0;
        };
        let value = f.release.fetch_add(1, Ordering::Relaxed) + 1;
        // SAFETY: command queues are free-threaded; the fence was created on this device.
        match unsafe { queue.Signal(&f.released, value) } {
            Ok(()) => value,
            Err(_) => 0,
        }
    }

    /// Whether wgpu's work up to a [`D3D11Interop::release_reads`] value finished. Never blocks.
    pub fn released(&self, value: u64) -> bool {
        match &self.fences {
            // SAFETY: reading a fence's completed value is free-threaded.
            Some(f) => (unsafe { f.released.GetCompletedValue() }) >= value,
            None => true,
        }
    }

    fn raw_queue(&self) -> Option<ID3D12CommandQueue> {
        // SAFETY: we only clone the raw COM queue reference; wgpu keeps using it as before.
        unsafe { self.gpu.queue.as_hal::<wgpu::hal::api::Dx12>() }.map(|q| q.as_raw().clone())
    }

    /// Creates a BGRA texture shared between D3D11 (write) and wgpu (sample).
    pub fn create_shared(&self, width: u32, height: u32) -> Result<SharedTexture, CaptureError> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width.max(1),
            Height: height.max(1),
            MipLevels: 1,
            ArraySize: 1,
            // Typeless so wgpu can view it as sRGB while D3D11 copies UNORM data in.
            Format: DXGI_FORMAT_B8G8R8A8_TYPELESS,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: (D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0 | D3D11_RESOURCE_MISC_SHARED.0)
                as u32,
        };
        // SAFETY: valid descriptor; COM results are checked; the NT handle is
        // closed after D3D12 has opened its own reference.
        unsafe {
            let mut texture = None;
            self.device
                .CreateTexture2D(&desc, None, Some(&mut texture))?;
            let d3d11: ID3D11Texture2D =
                texture.ok_or_else(|| CaptureError::Interop("texture creation failed".into()))?;
            let resource: IDXGIResource1 = d3d11.cast()?;
            let handle = resource.CreateSharedHandle(
                None,
                (DXGI_SHARED_RESOURCE_READ | DXGI_SHARED_RESOURCE_WRITE).0,
                None,
            )?;
            let mut d3d12_resource: Option<ID3D12Resource> = None;
            let opened = self.d3d12.OpenSharedHandle(handle, &mut d3d12_resource);
            let _ = CloseHandle(handle);
            opened?;
            let d3d12_resource = d3d12_resource
                .ok_or_else(|| CaptureError::Interop("OpenSharedHandle returned nothing".into()))?;

            let size = wgpu::Extent3d {
                width: desc.Width,
                height: desc.Height,
                depth_or_array_layers: 1,
            };
            let hal = wgpu::hal::dx12::Device::texture_from_raw(
                d3d12_resource,
                SHARED_FORMAT,
                wgpu::TextureDimension::D2,
                size,
                1,
                1,
            );
            let texture = self
                .gpu
                .device
                .create_texture_from_hal::<wgpu::hal::api::Dx12>(
                    hal,
                    &wgpu::TextureDescriptor {
                        label: Some("shared capture"),
                        size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: SHARED_FORMAT,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                    wgpu::TextureUses::UNINITIALIZED,
                );
            Ok(SharedTexture {
                d3d11,
                texture,
                width: desc.Width,
                height: desc.Height,
            })
        }
    }

    /// Blocks until all D3D11 work submitted so far has finished on the GPU,
    /// so D3D12 can safely read the results.
    pub fn flush_and_wait(&self) -> Result<(), CaptureError> {
        // SAFETY: event queries are created and polled on the same device context.
        unsafe {
            let desc = D3D11_QUERY_DESC {
                Query: D3D11_QUERY_EVENT,
                MiscFlags: 0,
            };
            let mut query = None;
            self.device.CreateQuery(&desc, Some(&mut query))?;
            let query =
                query.ok_or_else(|| CaptureError::Interop("query creation failed".into()))?;
            self.context.End(&query);
            self.context.Flush();
            let start = std::time::Instant::now();
            loop {
                let mut done = windows::core::BOOL(0);
                let _ = self.context.GetData(
                    &query,
                    Some(&mut done as *mut _ as *mut core::ffi::c_void),
                    size_of::<windows::core::BOOL>() as u32,
                    0,
                );
                if done.as_bool() {
                    return Ok(());
                }
                if start.elapsed() > std::time::Duration::from_millis(250) {
                    return Err(CaptureError::Interop("D3D11 GPU work timed out".into()));
                }
                // Sleep rather than spin: only drivers without fences come here, and
                // they shouldn't cost a busy core.
                std::thread::sleep(std::time::Duration::from_micros(250));
            }
        }
    }
}

impl Fences {
    fn new(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        d3d12: &ID3D12Device,
    ) -> Result<Self, CaptureError> {
        // SAFETY: COM calls with valid arguments; the NT handle is closed after D3D12 opened it.
        unsafe {
            let device5: ID3D11Device5 = device.cast()?;
            let context: ID3D11DeviceContext4 = context.cast()?;
            let mut copied_11: Option<ID3D11Fence> = None;
            device5.CreateFence(0, D3D11_FENCE_FLAG_SHARED, &mut copied_11)?;
            let copied_11 = copied_11
                .ok_or_else(|| CaptureError::Interop("CreateFence returned nothing".into()))?;
            let handle = copied_11.CreateSharedHandle(None, GENERIC_ALL.0, None)?;
            let mut copied_12: Option<ID3D12Fence> = None;
            let opened = d3d12.OpenSharedHandle(handle, &mut copied_12);
            let _ = CloseHandle(handle);
            opened?;
            let copied_12 = copied_12
                .ok_or_else(|| CaptureError::Interop("shared fence did not open".into()))?;
            let released: ID3D12Fence = d3d12.CreateFence(0, D3D12_FENCE_FLAG_NONE)?;
            Ok(Self {
                context,
                copied_11,
                copied_12,
                copied: AtomicU64::new(0),
                released,
                release: AtomicU64::new(0),
            })
        }
    }
}

/// A texture written by D3D11 and sampled by wgpu.
pub struct SharedTexture {
    pub d3d11: ID3D11Texture2D,
    pub texture: wgpu::Texture,
    pub width: u32,
    pub height: u32,
}

// SAFETY: COM texture references are free-threaded; all D3D11 access goes
// through the multithread-protected device context.
unsafe impl Send for SharedTexture {}
unsafe impl Sync for SharedTexture {}

impl SharedTexture {
    /// Copies `width × height` pixels from the top-left of `src` (a BGRA texture).
    pub fn copy_from(
        &self,
        interop: &D3D11Interop,
        src: &ID3D11Texture2D,
        width: u32,
        height: u32,
    ) {
        let region = D3D11_BOX {
            left: 0,
            top: 0,
            front: 0,
            right: width.min(self.width),
            bottom: height.min(self.height),
            back: 1,
        };
        // SAFETY: both textures belong to `interop.device`; the box is clamped to the destination.
        unsafe {
            interop
                .context
                .CopySubresourceRegion(&self.d3d11, 0, 0, 0, 0, src, 0, Some(&region));
        }
    }

    /// Fills the texture with a color via D3D11 (used by tests and placeholders).
    pub fn clear(&self, interop: &D3D11Interop, rgba: [f32; 4]) -> Result<(), CaptureError> {
        let desc = D3D11_RENDER_TARGET_VIEW_DESC {
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            ViewDimension: D3D11_RTV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_RENDER_TARGET_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_RTV { MipSlice: 0 },
            },
        };
        // SAFETY: valid RTV descriptor for this texture.
        unsafe {
            let mut rtv = None;
            interop
                .device
                .CreateRenderTargetView(&self.d3d11, Some(&desc), Some(&mut rtv))?;
            let rtv = rtv.ok_or_else(|| CaptureError::Interop("RTV creation failed".into()))?;
            interop.context.ClearRenderTargetView(&rtv, &rgba);
        }
        Ok(())
    }
}
