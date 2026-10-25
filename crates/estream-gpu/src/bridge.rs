//! Previews rendered on the engine's GPU device and shown on the UI's (Windows, DX12).
//!
//! The UI and the engine each get their own wgpu device on the same adapter, so a UI present
//! or a window resize never waits behind compositor submits (wgpu locks per device). wgpu-hal
//! opens both on the adapter's one `ID3D12Device`, with a queue each, so a texture and a
//! fence made once work on both queues:
//! - each preview is a committed resource with simultaneous access, wrapped as a wgpu texture
//!   on both devices;
//! - the engine signals `frame` after the submit that draws previews; the UI queues a GPU wait
//!   for that value, copies the preview into a texture of its own, then signals `copied`;
//! - before redrawing a preview the engine checks, without waiting, that the UI's last copy
//!   of it has finished, and otherwise leaves that preview alone for a frame.
//!
//! Neither CPU thread ever waits for the other's GPU work.

use crate::GpuContext;

/// Shares previews between the engine's device and the UI's. Only exists on Windows with DX12;
/// elsewhere the engine and the UI share one device.
pub struct PreviewBridge {
    #[cfg(windows)]
    imp: imp::Bridge,
    #[cfg(not(windows))]
    never: std::convert::Infallible,
}

impl std::fmt::Debug for PreviewBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PreviewBridge")
    }
}

impl PreviewBridge {
    /// Connects `engine` (where previews are drawn) with `ui` (where they are shown). `None`
    /// when they aren't two DX12 devices on one adapter.
    pub fn new(engine: &GpuContext, ui: &GpuContext) -> Option<Self> {
        #[cfg(windows)]
        {
            imp::Bridge::new(engine, ui).map(|imp| Self { imp })
        }
        #[cfg(not(windows))]
        {
            let _ = (engine, ui);
            None
        }
    }

    /// A preview texture on the engine's device and the same texture on the UI's.
    pub fn create_preview(
        &self,
        width: u32,
        height: u32,
    ) -> Option<(wgpu::Texture, wgpu::Texture)> {
        #[cfg(windows)]
        {
            self.imp.create_preview(width, height)
        }
        #[cfg(not(windows))]
        match self.never {}
    }

    /// Call on the engine's thread right after the submit that drew previews. Returns the
    /// value the UI waits for before reading them.
    pub fn frame_submitted(&self) -> u64 {
        #[cfg(windows)]
        {
            self.imp.frame_submitted()
        }
        #[cfg(not(windows))]
        match self.never {}
    }

    /// Whether the UI's copy `copy` (from [`Self::copy_to_ui`]) has finished on the GPU.
    pub fn copied(&self, copy: u64) -> bool {
        #[cfg(windows)]
        {
            self.imp.copied(copy)
        }
        #[cfg(not(windows))]
        match self.never {}
    }

    /// On the UI's thread: copies `preview` (the UI's side of a bridge texture) into `target`
    /// once the engine's frame `frame` is done on the GPU. Returns the copy's value.
    pub fn copy_to_ui(
        &self,
        ui: &GpuContext,
        preview: &wgpu::Texture,
        target: &wgpu::Texture,
        frame: u64,
    ) -> u64 {
        #[cfg(windows)]
        {
            self.imp.copy_to_ui(ui, preview, target, frame)
        }
        #[cfg(not(windows))]
        match self.never {}
    }
}

#[cfg(windows)]
mod imp {
    use crate::{GpuContext, PREVIEW_FORMAT, PREVIEW_VIEW_FORMAT};
    use std::sync::atomic::{AtomicU64, Ordering};
    use windows::Win32::Graphics::Direct3D12::*;
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_TYPELESS, DXGI_SAMPLE_DESC};

    pub(super) struct Bridge {
        device: ID3D12Device,
        engine: wgpu::Device,
        ui: wgpu::Device,
        engine_queue: ID3D12CommandQueue,
        ui_queue: ID3D12CommandQueue,
        frame: ID3D12Fence,
        copied: ID3D12Fence,
        frame_value: AtomicU64,
        copied_value: AtomicU64,
    }

    // SAFETY: D3D12 devices, queues and fences are free-threaded.
    unsafe impl Send for Bridge {}
    // SAFETY: as above; the counters are atomics.
    unsafe impl Sync for Bridge {}

    fn raw_device(gpu: &GpuContext) -> Option<ID3D12Device> {
        // SAFETY: as_hal only reads the DX12 device behind a wgpu device.
        unsafe { gpu.device.as_hal::<wgpu::hal::api::Dx12>() }.map(|d| d.raw_device().clone())
    }

    fn raw_queue(gpu: &GpuContext) -> Option<ID3D12CommandQueue> {
        // SAFETY: as_hal only reads the DX12 queue behind a wgpu queue.
        unsafe { gpu.queue.as_hal::<wgpu::hal::api::Dx12>() }.map(|q| q.as_raw().clone())
    }

    impl Bridge {
        pub(super) fn new(engine: &GpuContext, ui: &GpuContext) -> Option<Self> {
            let device = raw_device(engine)?;
            if raw_device(ui)? != device {
                tracing::warn!("the UI and engine GPU devices differ; previews share one device");
                return None;
            }
            let fence = || {
                // SAFETY: plain fence creation on a valid device.
                unsafe { device.CreateFence::<ID3D12Fence>(0, D3D12_FENCE_FLAG_NONE) }
                    .map_err(|e| tracing::warn!(error = %e, "preview fence not created"))
                    .ok()
            };
            Some(Self {
                frame: fence()?,
                copied: fence()?,
                engine_queue: raw_queue(engine)?,
                ui_queue: raw_queue(ui)?,
                engine: engine.device.clone(),
                ui: ui.device.clone(),
                device,
                frame_value: AtomicU64::new(0),
                copied_value: AtomicU64::new(0),
            })
        }

        pub(super) fn create_preview(
            &self,
            width: u32,
            height: u32,
        ) -> Option<(wgpu::Texture, wgpu::Texture)> {
            let (width, height) = (width.max(1), height.max(1));
            let heap = D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_DEFAULT,
                ..Default::default()
            };
            let desc = D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                Alignment: 0,
                Width: u64::from(width),
                Height: height,
                DepthOrArraySize: 1,
                MipLevels: 1,
                // Typeless, as wgpu makes textures with an sRGB view format.
                Format: DXGI_FORMAT_R8G8B8A8_TYPELESS,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
                // Simultaneous access: each queue uses it from the common state, so the two
                // devices' state tracking never has to agree.
                Flags: D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET
                    | D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS,
            };
            let mut resource: Option<ID3D12Resource> = None;
            // SAFETY: valid heap and resource descriptions; the result is checked.
            let made = unsafe {
                self.device.CreateCommittedResource(
                    &heap,
                    D3D12_HEAP_FLAG_NONE,
                    &desc,
                    D3D12_RESOURCE_STATE_COMMON,
                    None,
                    &mut resource,
                )
            };
            let resource = match (made, resource) {
                (Ok(()), Some(r)) => r,
                (Err(e), _) => {
                    tracing::warn!(error = %e, "shared preview texture not created");
                    return None;
                }
                (Ok(()), None) => return None,
            };
            let size = wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            };
            let wrap = |device: &wgpu::Device,
                        label: &str,
                        usage: wgpu::TextureUsages,
                        view_formats: &[wgpu::TextureFormat]| {
                // SAFETY: the resource is a 2D texture of this size and format, made on the
                // device behind both wgpu devices; each wrapper holds its own reference.
                unsafe {
                    let hal = wgpu::hal::dx12::Device::texture_from_raw(
                        resource.clone(),
                        PREVIEW_FORMAT,
                        wgpu::TextureDimension::D2,
                        size,
                        1,
                        1,
                    );
                    device.create_texture_from_hal::<wgpu::hal::api::Dx12>(
                        hal,
                        &wgpu::TextureDescriptor {
                            label: Some(label),
                            size,
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: PREVIEW_FORMAT,
                            usage,
                            view_formats,
                        },
                        wgpu::TextureUses::UNINITIALIZED,
                    )
                }
            };
            let engine = wrap(
                &self.engine,
                "preview",
                wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                &[PREVIEW_VIEW_FORMAT],
            );
            let ui = wrap(
                &self.ui,
                "preview (UI side)",
                wgpu::TextureUsages::COPY_SRC,
                &[],
            );
            Some((engine, ui))
        }

        pub(super) fn frame_submitted(&self) -> u64 {
            let value = self.frame_value.fetch_add(1, Ordering::AcqRel) + 1;
            // SAFETY: queues a signal of a fence on this device's queue.
            if let Err(e) = unsafe { self.engine_queue.Signal(&self.frame, value) } {
                tracing::warn!(error = %e, "preview frame signal failed");
            }
            value
        }

        pub(super) fn copied(&self, copy: u64) -> bool {
            // SAFETY: reads the fence's completed value.
            copy == 0 || unsafe { self.copied.GetCompletedValue() } >= copy
        }

        pub(super) fn copy_to_ui(
            &self,
            ui: &GpuContext,
            preview: &wgpu::Texture,
            target: &wgpu::Texture,
            frame: u64,
        ) -> u64 {
            // The wait is queued before the copy below, so the copy reads the finished frame.
            // SAFETY: queues a GPU wait on a fence of this device.
            if let Err(e) = unsafe { self.ui_queue.Wait(&self.frame, frame) } {
                tracing::warn!(error = %e, "preview wait failed");
            }
            let mut encoder = ui
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("preview to UI"),
                });
            encoder.copy_texture_to_texture(
                preview.as_image_copy(),
                target.as_image_copy(),
                preview.size(),
            );
            ui.queue.submit([encoder.finish()]);
            let value = self.copied_value.fetch_add(1, Ordering::AcqRel) + 1;
            // SAFETY: queues a signal of a fence on this device's queue, after the copy.
            if let Err(e) = unsafe { self.ui_queue.Signal(&self.copied, value) } {
                tracing::warn!(error = %e, "preview copy signal failed");
            }
            value
        }
    }
}
