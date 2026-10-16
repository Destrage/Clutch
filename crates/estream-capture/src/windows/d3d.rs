//! D3D11 ↔ wgpu (D3D12) interop.
//!
//! Capture APIs on Windows hand out `ID3D11Texture2D`s. We create a D3D11 device
//! on the same adapter as wgpu, copy captured frames into textures shared via NT
//! handles, and open those textures in wgpu's D3D12 device — no CPU readback.

use crate::CaptureError;
use estream_gpu::{GpuContext, wgpu};
use windows::Win32::Foundation::{CloseHandle, HMODULE};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Direct3D12::{ID3D12Device, ID3D12Resource};
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::core::Interface;

/// Format of shared capture textures in wgpu (sampled as sRGB → linear).
pub const SHARED_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8UnormSrgb;

/// A D3D11 device on the same GPU as wgpu, plus wgpu's D3D12 device.
#[derive(Clone)]
pub struct D3D11Interop {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    d3d12: ID3D12Device,
    gpu: GpuContext,
}

// SAFETY: the D3D11 device is created with multithread protection enabled
// (`ID3D11Multithread::SetMultithreadProtected`), which makes the immediate
// context safe to use from several threads; D3D12 devices are free-threaded.
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
            Ok(Self {
                device,
                context,
                d3d12,
                gpu: gpu.clone(),
            })
        }
    }

    pub fn gpu(&self) -> &GpuContext {
        &self.gpu
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
                std::thread::yield_now();
            }
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
