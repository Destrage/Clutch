//! Hardware encoders fed straight from the GPU on Windows (see `estream_engine::gpu_export`).
//!
//! The engine renders each frame's NV12 planes (its own WGSL conversion) into R8 and RG8
//! textures created here on wgpu's D3D12 device with simultaneous access, so D3D11 can read
//! them without resource-state handoffs. An encoder then copies the planes into a pooled
//! FFmpeg D3D11 NV12 texture with a pass-through shader (D3D11 can't copy into NV12 planes
//! directly) and sends that texture to NVENC/AMF. Nothing goes through CPU memory.
//!
//! Ordering is GPU side only: wgpu's queue signals `ready` after a frame's work; the encoder's
//! D3D11 context waits for it before the copy and signals `released` after; wgpu's queue waits
//! for `released` before rendering into those planes again.

use crate::MediaError;
use crate::ffmpeg::ffi;
use estream_engine::gpu::{GpuContext, Nv12Target, wgpu};
use estream_engine::{ExportPlanes, GpuExport, GpuFrame, PlanesHandle};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use windows::Win32::Foundation::{CloseHandle, GENERIC_ALL, HMODULE};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
    D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, ID3DBlob,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, IDXGIFactory4,
};
use windows::core::{Interface, PCSTR, s};

const VERTEX: &str = "
float4 main(uint id : SV_VertexID) : SV_Position {
    float2 uv = float2((id << 1) & 2, id & 2);
    return float4(uv * float2(2, -2) + float2(-1, 1), 0, 1);
}";
const COPY_Y: &str = "
Texture2D<float> plane : register(t0);
float main(float4 pos : SV_Position) : SV_Target { return plane.Load(int3(pos.xy, 0)); }";
const COPY_UV: &str = "
Texture2D<float2> plane : register(t0);
float2 main(float4 pos : SV_Position) : SV_Target { return plane.Load(int3(pos.xy, 0)); }";

/// The D3D11 device and fences shared by every GPU-fed encoder in the process.
struct Inner {
    device: ID3D11Device,
    device1: ID3D11Device1,
    device3: ID3D11Device3,
    context: ID3D11DeviceContext,
    context4: ID3D11DeviceContext4,
    d3d12: ID3D12Device,
    queue: ID3D12CommandQueue,
    ready_12: ID3D12Fence,
    ready_11: ID3D11Fence,
    /// Last `ready` value; signalled under this lock (see `signal_ready`).
    ready: Mutex<u64>,
    released_11: ID3D11Fence,
    released_12: ID3D12Fence,
    /// Last `released` value; signalled under this lock so values stay in order.
    released: Mutex<u64>,
    vs: ID3D11VertexShader,
    ps_y: ID3D11PixelShader,
    ps_uv: ID3D11PixelShader,
    /// FFmpeg's D3D11VA device context wrapping `device`.
    hw_device: HwRef,
}

// SAFETY: the D3D11 device is multithread-protected; D3D12 devices, queues and fences are
// free-threaded; FFmpeg's device context is only touched under its own lock.
unsafe impl Send for Inner {}
unsafe impl Sync for Inner {}

/// An owned `AVBufferRef`.
struct HwRef(*mut ffi::AVBufferRef);

impl Drop for HwRef {
    fn drop(&mut self) {
        // SAFETY: we own this reference.
        unsafe { ffi::av_buffer_unref(&mut self.0) };
    }
}

/// The [`GpuExport`] handed to the engine.
pub struct D3D11Frames {
    inner: Arc<Inner>,
}

/// One slot of NV12 planes as the encoder sees them.
struct Planes {
    inner: Arc<Inner>,
    y: ID3D11ShaderResourceView,
    uv: ID3D11ShaderResourceView,
    /// `released` value after the last encoder copy of these planes.
    released: AtomicU64,
}

// SAFETY: see `Inner`; views are only used under FFmpeg's device lock.
unsafe impl Send for Planes {}
unsafe impl Sync for Planes {}

/// The process's GPU frame export for `gpu`, created once. `None` (logged) when the GPU or
/// driver can't share planes with D3D11.
pub fn export(gpu: &GpuContext) -> Option<Arc<D3D11Frames>> {
    type Cache = Mutex<Vec<(wgpu::Device, Option<Arc<D3D11Frames>>)>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().ok()?;
    if let Some((_, found)) = cache.iter().find(|(d, _)| *d == gpu.device) {
        return found.clone();
    }
    crate::init();
    let made = match Inner::new(gpu) {
        Ok(inner) => Some(Arc::new(D3D11Frames {
            inner: Arc::new(inner),
        })),
        Err(e) => {
            tracing::info!(error = %e, "GPU frames for encoders unavailable; using CPU frames");
            None
        }
    };
    cache.push((gpu.device.clone(), made.clone()));
    made
}

fn err(e: impl std::fmt::Display) -> MediaError {
    MediaError::Engine(format!("GPU frames: {e}"))
}

impl Inner {
    fn new(gpu: &GpuContext) -> Result<Self, MediaError> {
        // SAFETY: we only clone raw COM references out of wgpu's device and queue.
        let d3d12 = unsafe { gpu.device.as_hal::<wgpu::hal::api::Dx12>() }
            .map(|d| d.raw_device().clone())
            .ok_or_else(|| err("wgpu is not on D3D12"))?;
        let queue = unsafe { gpu.queue.as_hal::<wgpu::hal::api::Dx12>() }
            .map(|q| q.as_raw().clone())
            .ok_or_else(|| err("wgpu is not on D3D12"))?;
        // SAFETY: plain COM calls with valid arguments; every result is checked.
        unsafe {
            let luid = d3d12.GetAdapterLuid();
            let factory: IDXGIFactory4 =
                CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).map_err(err)?;
            let adapter: windows::Win32::Graphics::Dxgi::IDXGIAdapter1 =
                factory.EnumAdapterByLuid(luid).map_err(err)?;
            let (mut device, mut context) = (None, None);
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(err)?;
            let device: ID3D11Device = device.ok_or_else(|| err("no D3D11 device"))?;
            let context: ID3D11DeviceContext = context.ok_or_else(|| err("no D3D11 context"))?;
            let mt: ID3D11Multithread = device.cast().map_err(err)?;
            let _ = mt.SetMultithreadProtected(true);
            let device1: ID3D11Device1 = device.cast().map_err(err)?;
            let device3: ID3D11Device3 = device.cast().map_err(err)?;
            let device5: ID3D11Device5 = device.cast().map_err(err)?;
            let context4: ID3D11DeviceContext4 = context.cast().map_err(err)?;

            // ready: wgpu (D3D12) signals, D3D11 waits.
            let ready_12: ID3D12Fence =
                d3d12.CreateFence(0, D3D12_FENCE_FLAG_SHARED).map_err(err)?;
            let handle = d3d12
                .CreateSharedHandle(&ready_12, None, GENERIC_ALL.0, None)
                .map_err(err)?;
            let mut ready_11: Option<ID3D11Fence> = None;
            let opened = device5.OpenSharedFence(handle, &mut ready_11);
            let _ = CloseHandle(handle);
            opened.map_err(err)?;
            let ready_11 = ready_11.ok_or_else(|| err("ready fence did not open"))?;

            // released: D3D11 signals, wgpu (D3D12) waits.
            let mut released_11: Option<ID3D11Fence> = None;
            device5
                .CreateFence(0, D3D11_FENCE_FLAG_SHARED, &mut released_11)
                .map_err(err)?;
            let released_11 = released_11.ok_or_else(|| err("CreateFence returned nothing"))?;
            let handle = released_11
                .CreateSharedHandle(None, GENERIC_ALL.0, None)
                .map_err(err)?;
            let mut released_12: Option<ID3D12Fence> = None;
            let opened = d3d12.OpenSharedHandle(handle, &mut released_12);
            let _ = CloseHandle(handle);
            opened.map_err(err)?;
            let released_12 = released_12.ok_or_else(|| err("released fence did not open"))?;

            let vs_code = compile(VERTEX, s!("vs_5_0"))?;
            let mut vs = None;
            device
                .CreateVertexShader(&vs_code, None, Some(&mut vs))
                .map_err(err)?;
            let pixel = |src: &str| -> Result<ID3D11PixelShader, MediaError> {
                let code = compile(src, s!("ps_5_0"))?;
                let mut ps = None;
                device
                    .CreatePixelShader(&code, None, Some(&mut ps))
                    .map_err(err)?;
                ps.ok_or_else(|| err("pixel shader not created"))
            };
            let ps_y = pixel(COPY_Y)?;
            let ps_uv = pixel(COPY_UV)?;

            let hw_device = hw_device(&device, &context)?;
            Ok(Self {
                device,
                device1,
                device3,
                context,
                context4,
                d3d12,
                queue,
                ready_12,
                ready_11,
                ready: Mutex::new(0),
                released_11,
                released_12,
                released: Mutex::new(0),
                vs: vs.ok_or_else(|| err("vertex shader not created"))?,
                ps_y,
                ps_uv,
                hw_device,
            })
        }
    }

    /// Creates one plane on wgpu's D3D12 device (shared, simultaneous access) and opens it
    /// in D3D11 and wgpu.
    fn plane(
        &self,
        gpu: &GpuContext,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Result<(wgpu::Texture, ID3D11ShaderResourceView), MediaError> {
        let dxgi = match format {
            wgpu::TextureFormat::R8Unorm => DXGI_FORMAT_R8_UNORM,
            _ => DXGI_FORMAT_R8G8_UNORM,
        };
        let desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            Alignment: 0,
            Width: width as u64,
            Height: height,
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: dxgi,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
            Flags: D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET
                | D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS,
        };
        let heap = D3D12_HEAP_PROPERTIES {
            Type: D3D12_HEAP_TYPE_DEFAULT,
            ..Default::default()
        };
        // SAFETY: valid descriptors; the NT handle is closed once D3D11 opened its reference.
        unsafe {
            let mut resource: Option<ID3D12Resource> = None;
            self.d3d12
                .CreateCommittedResource(
                    &heap,
                    D3D12_HEAP_FLAG_SHARED,
                    &desc,
                    D3D12_RESOURCE_STATE_COMMON,
                    None,
                    &mut resource,
                )
                .map_err(err)?;
            let resource = resource.ok_or_else(|| err("plane not created"))?;
            let handle = self
                .d3d12
                .CreateSharedHandle(&resource, None, GENERIC_ALL.0, None)
                .map_err(err)?;
            let opened: windows::core::Result<ID3D11Texture2D> =
                self.device1.OpenSharedResource1(handle);
            let _ = CloseHandle(handle);
            let texture = opened.map_err(err)?;
            let mut srv = None;
            self.device
                .CreateShaderResourceView(&texture, None, Some(&mut srv))
                .map_err(err)?;

            let size = wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            };
            let hal = wgpu::hal::dx12::Device::texture_from_raw(
                resource,
                format,
                wgpu::TextureDimension::D2,
                size,
                1,
                1,
            );
            let texture = gpu.device.create_texture_from_hal::<wgpu::hal::api::Dx12>(
                hal,
                &wgpu::TextureDescriptor {
                    label: Some("nv12 plane (shared)"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::TextureUses::UNINITIALIZED,
            );
            Ok((texture, srv.ok_or_else(|| err("plane view not created"))?))
        }
    }

    /// FFmpeg's lock around the shared D3D11 immediate context.
    fn lock(&self) -> DeviceLock<'_> {
        // SAFETY: `hw_device` is an initialised D3D11VA device context.
        let hw = unsafe { &*device_hwctx(self.hw_device.0) };
        if let Some(lock) = hw.lock {
            // SAFETY: FFmpeg's own lock callback and context.
            unsafe { lock(hw.lock_ctx) };
        }
        DeviceLock { hw }
    }
}

struct DeviceLock<'a> {
    hw: &'a ffi::AVD3D11VADeviceContext,
}

impl Drop for DeviceLock<'_> {
    fn drop(&mut self) {
        if let Some(unlock) = self.hw.unlock {
            // SAFETY: pairs with the lock taken in `Inner::lock`.
            unsafe { unlock(self.hw.lock_ctx) };
        }
    }
}

/// # Safety
/// `device` must be an initialised `AV_HWDEVICE_TYPE_D3D11VA` device context.
unsafe fn device_hwctx(device: *mut ffi::AVBufferRef) -> *mut ffi::AVD3D11VADeviceContext {
    // SAFETY: per the caller; `data` of a device context buffer is an `AVHWDeviceContext`.
    unsafe {
        let ctx = (*device).data as *mut ffi::AVHWDeviceContext;
        (*ctx).hwctx as *mut ffi::AVD3D11VADeviceContext
    }
}

/// Wraps our D3D11 device in an FFmpeg D3D11VA device context (FFmpeg takes a reference to
/// the device and context and adds its default lock).
fn hw_device(device: &ID3D11Device, context: &ID3D11DeviceContext) -> Result<HwRef, MediaError> {
    // SAFETY: FFmpeg allocation and init calls; the COM references we hand over are AddRef'd
    // clones that FFmpeg releases when the context is freed.
    unsafe {
        let buf = ffi::av_hwdevice_ctx_alloc(ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA);
        if buf.is_null() {
            return Err(err("av_hwdevice_ctx_alloc failed"));
        }
        let hw = HwRef(buf);
        let ctx = device_hwctx(buf);
        (*ctx).device = device.clone().into_raw() as *mut _;
        (*ctx).device_context = context.clone().into_raw() as *mut _;
        let ret = ffi::av_hwdevice_ctx_init(buf);
        if ret < 0 {
            return Err(err(format!("av_hwdevice_ctx_init: {}", ffmpeg_error(ret))));
        }
        Ok(hw)
    }
}

fn ffmpeg_error(code: i32) -> String {
    crate::ffmpeg::Error::from(code).to_string()
}

fn compile(source: &str, target: PCSTR) -> Result<Vec<u8>, MediaError> {
    // SAFETY: the source buffer outlives the call; blobs are checked before use.
    unsafe {
        let mut code: Option<ID3DBlob> = None;
        let mut errors: Option<ID3DBlob> = None;
        let result = D3DCompile(
            source.as_ptr().cast(),
            source.len(),
            PCSTR::null(),
            None,
            None,
            s!("main"),
            target,
            0,
            0,
            &mut code,
            Some(&mut errors),
        );
        if let Err(e) = result {
            let detail = errors
                .map(|b| {
                    let bytes = std::slice::from_raw_parts(
                        b.GetBufferPointer() as *const u8,
                        b.GetBufferSize(),
                    );
                    String::from_utf8_lossy(bytes).into_owned()
                })
                .unwrap_or_default();
            return Err(err(format!("shader: {e} {detail}")));
        }
        let code = code.ok_or_else(|| err("shader compiled to nothing"))?;
        Ok(
            std::slice::from_raw_parts(code.GetBufferPointer() as *const u8, code.GetBufferSize())
                .to_vec(),
        )
    }
}

impl GpuExport for D3D11Frames {
    fn create_planes(&self, gpu: &GpuContext, width: u32, height: u32) -> Option<ExportPlanes> {
        let made = (|| {
            let (y, y_srv) = self
                .inner
                .plane(gpu, width, height, wgpu::TextureFormat::R8Unorm)?;
            let (uv, uv_srv) =
                self.inner
                    .plane(gpu, width / 2, height / 2, wgpu::TextureFormat::Rg8Unorm)?;
            Ok::<_, MediaError>((y, uv, y_srv, uv_srv))
        })();
        match made {
            Ok((y, uv, y_srv, uv_srv)) => Some(ExportPlanes {
                target: Nv12Target::from_planes(y, uv),
                handle: Arc::new(Planes {
                    inner: self.inner.clone(),
                    y: y_srv,
                    uv: uv_srv,
                    released: AtomicU64::new(0),
                }),
            }),
            Err(e) => {
                tracing::warn!(error = %e, "GPU frame planes not created");
                None
            }
        }
    }

    fn wait_released(&self, _: &GpuContext, handle: &PlanesHandle) {
        let Some(planes) = handle.downcast_ref::<Planes>() else {
            return;
        };
        let value = planes.released.load(Ordering::Acquire);
        if value > 0 {
            // SAFETY: command queues and fences are free-threaded.
            let _ = unsafe { self.inner.queue.Wait(&self.inner.released_12, value) };
        }
    }

    fn signal_ready(&self, _: &GpuContext) -> u64 {
        // Several engines' video threads may share this export: taking the value and queuing
        // its signal under one lock keeps the fence's values increasing in queue order (a
        // later, lower signal would move it back and strand a waiter).
        let Ok(mut ready) = self.inner.ready.lock() else {
            return 0;
        };
        // SAFETY: command queues and fences are free-threaded.
        match unsafe { self.inner.queue.Signal(&self.inner.ready_12, *ready + 1) } {
            Ok(()) => {
                *ready += 1;
                *ready
            }
            Err(_) => 0,
        }
    }
}

/// An encoder's side: an FFmpeg D3D11 frames pool of NV12 textures, filled from GPU frames.
pub struct GpuInput {
    inner: Arc<Inner>,
    frames: HwRef,
    /// Plane render targets per pooled texture (keyed by its pointer).
    targets: HashMap<usize, [ID3D11RenderTargetView; 2]>,
    width: u32,
    height: u32,
}

// SAFETY: see `Inner`; an encoder uses its input from one thread at a time.
unsafe impl Send for GpuInput {}

impl GpuInput {
    /// Sets up an NV12 frames pool for frames like `frame` (`width × height`).
    pub fn new(frame: &GpuFrame, width: u32, height: u32) -> Result<Self, MediaError> {
        let planes = frame
            .planes
            .downcast_ref::<Planes>()
            .ok_or_else(|| err("these frames came from another export"))?;
        let inner = planes.inner.clone();
        // SAFETY: FFmpeg frames-context setup on our initialised device context.
        let frames = unsafe {
            let buf = ffi::av_hwframe_ctx_alloc(inner.hw_device.0);
            if buf.is_null() {
                return Err(err("av_hwframe_ctx_alloc failed"));
            }
            let frames = HwRef(buf);
            let ctx = (*buf).data as *mut ffi::AVHWFramesContext;
            (*ctx).format = ffi::AVPixelFormat::AV_PIX_FMT_D3D11;
            (*ctx).sw_format = ffi::AVPixelFormat::AV_PIX_FMT_NV12;
            (*ctx).width = width as i32;
            (*ctx).height = height as i32;
            // 0: textures are made one by one as the pool needs them, then reused.
            (*ctx).initial_pool_size = 0;
            let d3d = (*ctx).hwctx as *mut ffi::AVD3D11VAFramesContext;
            (*d3d).BindFlags = D3D11_BIND_RENDER_TARGET.0 as u32;
            let ret = ffi::av_hwframe_ctx_init(buf);
            if ret < 0 {
                return Err(err(format!("av_hwframe_ctx_init: {}", ffmpeg_error(ret))));
            }
            frames
        };
        Ok(Self {
            inner,
            frames,
            targets: HashMap::new(),
            width,
            height,
        })
    }

    /// The frames context to set on the encoder (a new reference the caller owns).
    pub fn frames_ref(&self) -> *mut ffi::AVBufferRef {
        // SAFETY: `frames` is a valid buffer reference.
        unsafe { ffi::av_buffer_ref(self.frames.0) }
    }

    /// Copies `frame`'s planes into a pooled encoder texture, ordered on the GPU, and
    /// returns that texture as an FFmpeg frame ready to send.
    pub fn upload(&mut self, frame: &GpuFrame) -> Result<crate::ffmpeg::frame::Video, MediaError> {
        let planes = frame
            .planes
            .downcast_ref::<Planes>()
            .ok_or_else(|| err("these frames came from another export"))?;
        let mut out = crate::ffmpeg::frame::Video::empty();
        // SAFETY: FFmpeg hands out a pooled D3D11 texture (data[0]) and its array index (data[1]).
        let (texture, index) = unsafe {
            let ret = ffi::av_hwframe_get_buffer(self.frames.0, out.as_mut_ptr(), 0);
            if ret < 0 {
                return Err(err(format!("av_hwframe_get_buffer: {}", ffmpeg_error(ret))));
            }
            let raw = (*out.as_ptr()).data[0] as *mut core::ffi::c_void;
            let index = (*out.as_ptr()).data[1] as usize as u32;
            let texture = ID3D11Texture2D::from_raw_borrowed(&raw)
                .ok_or_else(|| err("pool frame has no texture"))?
                .clone();
            (texture, index)
        };
        let key = texture.as_raw() as usize ^ ((index as usize) << 48);
        if !self.targets.contains_key(&key) {
            let targets = [
                self.plane_target(&texture, index, 0, DXGI_FORMAT_R8_UNORM)?,
                self.plane_target(&texture, index, 1, DXGI_FORMAT_R8G8_UNORM)?,
            ];
            self.targets.insert(key, targets);
        }
        let [y_target, uv_target] = &self.targets[&key];
        let inner = &self.inner;
        {
            let _lock = inner.lock();
            // SAFETY: all objects belong to `inner.device`; state is set fully before each draw
            // and resources are unbound afterwards. Calls happen under FFmpeg's device lock.
            unsafe {
                let ctx = &inner.context;
                inner
                    .context4
                    .Wait(&inner.ready_11, frame.ready)
                    .map_err(err)?;
                ctx.IASetInputLayout(None);
                ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
                ctx.VSSetShader(&inner.vs, None);
                let pass = |target: &ID3D11RenderTargetView,
                            ps: &ID3D11PixelShader,
                            source: &ID3D11ShaderResourceView,
                            w: u32,
                            h: u32| {
                    ctx.OMSetRenderTargets(Some(&[Some(target.clone())]), None);
                    ctx.RSSetViewports(Some(&[D3D11_VIEWPORT {
                        TopLeftX: 0.0,
                        TopLeftY: 0.0,
                        Width: w as f32,
                        Height: h as f32,
                        MinDepth: 0.0,
                        MaxDepth: 1.0,
                    }]));
                    ctx.PSSetShader(ps, None);
                    ctx.PSSetShaderResources(0, Some(&[Some(source.clone())]));
                    ctx.Draw(3, 0);
                };
                pass(y_target, &inner.ps_y, &planes.y, self.width, self.height);
                pass(
                    uv_target,
                    &inner.ps_uv,
                    &planes.uv,
                    self.width / 2,
                    self.height / 2,
                );
                ctx.PSSetShaderResources(0, Some(&[None]));
                ctx.OMSetRenderTargets(None, None);
                let mut released = inner.released.lock().map_err(|_| err("lock poisoned"))?;
                *released += 1;
                inner
                    .context4
                    .Signal(&inner.released_11, *released)
                    .map_err(err)?;
                planes.released.fetch_max(*released, Ordering::AcqRel);
                ctx.Flush();
            }
        }
        Ok(out)
    }

    fn plane_target(
        &self,
        texture: &ID3D11Texture2D,
        index: u32,
        plane: u32,
        format: DXGI_FORMAT,
    ) -> Result<ID3D11RenderTargetView, MediaError> {
        let desc = D3D11_RENDER_TARGET_VIEW_DESC1 {
            Format: format,
            ViewDimension: D3D11_RTV_DIMENSION_TEXTURE2DARRAY,
            Anonymous: D3D11_RENDER_TARGET_VIEW_DESC1_0 {
                Texture2DArray: D3D11_TEX2D_ARRAY_RTV1 {
                    MipSlice: 0,
                    FirstArraySlice: index,
                    ArraySize: 1,
                    PlaneSlice: plane,
                },
            },
        };
        let mut view: Option<ID3D11RenderTargetView1> = None;
        // SAFETY: valid descriptor for an NV12 texture plane created by FFmpeg on this device.
        unsafe {
            self.inner
                .device3
                .CreateRenderTargetView1(texture, Some(&desc), Some(&mut view))
                .map_err(err)?;
        }
        view.ok_or_else(|| err("plane target not created"))?
            .cast()
            .map_err(err)
    }
}
