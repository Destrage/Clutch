//! Windows.Graphics.Capture sessions feeding a ring of GPU-shared textures.
//!
//! Frames arrive on a free-threaded WinRT callback. Each one is copied (GPU→GPU)
//! into a free slot of a 3-texture ring shared with wgpu; the video thread picks
//! up the newest finished slot each tick. The slot the video thread is reading
//! and the newest slot are never written, so frames never tear.

use super::d3d::{D3D11Interop, SharedTexture};
use crate::CaptureError;
use estream_core::Size;
use estream_engine::{SourceContext, SourceFrame};
use estream_gpu::wgpu;
use std::sync::{Arc, Mutex};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows::core::{IInspectable, Interface};

const RING: usize = 3;

#[derive(Default)]
struct RingState {
    /// Bumped whenever the slot textures are recreated (size change).
    generation: u64,
    slots: Vec<Arc<SharedTexture>>,
    latest: Option<usize>,
    reading: Option<usize>,
    sequence: u64,
    closed: bool,
    error: Option<String>,
}

/// Frames shared between a capture callback and the video thread.
#[derive(Default)]
pub struct FrameRing {
    state: Mutex<RingState>,
}

impl FrameRing {
    pub fn is_closed(&self) -> bool {
        self.state.lock().map(|s| s.closed).unwrap_or(true)
    }

    pub fn error(&self) -> Option<String> {
        self.state.lock().ok().and_then(|s| s.error.clone())
    }

    /// Copies a captured texture into a free slot and publishes it.
    fn publish(
        &self,
        interop: &D3D11Interop,
        src: &ID3D11Texture2D,
        width: u32,
        height: u32,
    ) -> Result<(), CaptureError> {
        let slot = {
            let mut s = self
                .state
                .lock()
                .map_err(|_| CaptureError::Capture("ring poisoned".into()))?;
            let size_changed = s
                .slots
                .first()
                .is_none_or(|t| t.width != width || t.height != height);
            if size_changed {
                tracing::debug!(width, height, "wgc: creating shared slots");
                let slots = (0..RING)
                    .map(|_| interop.create_shared(width, height).map(Arc::new))
                    .collect::<Result<Vec<_>, _>>()?;
                s.slots = slots;
                s.generation += 1;
                s.latest = None;
                s.reading = None;
            }
            let free = (0..RING)
                .find(|i| Some(*i) != s.latest && Some(*i) != s.reading)
                .unwrap_or(0);
            (free, s.slots[free].clone())
        };
        slot.1.copy_from(interop, src, width, height);
        interop.flush_and_wait()?;
        let mut s = self
            .state
            .lock()
            .map_err(|_| CaptureError::Capture("ring poisoned".into()))?;
        if s.slots.get(slot.0).is_some_and(|t| Arc::ptr_eq(t, &slot.1)) {
            s.latest = Some(slot.0);
            s.sequence += 1;
        }
        Ok(())
    }
}

/// Video-thread side of a [`FrameRing`]: keeps wgpu bindings for the slots.
#[derive(Default)]
pub struct RingReader {
    generation: u64,
    slots: Vec<Arc<SharedTexture>>,
    bindings: Vec<wgpu::BindGroup>,
    current: Option<usize>,
    pub sequence: u64,
}

impl RingReader {
    /// Picks up the newest frame. Never blocks: if the capture thread holds the
    /// lock, the previous frame is kept.
    pub fn tick(&mut self, ring: &FrameRing, ctx: &SourceContext<'_>) {
        let Ok(mut s) = ring.state.try_lock() else {
            return;
        };
        if s.generation != self.generation {
            self.generation = s.generation;
            self.slots = s.slots.clone();
            self.bindings = self
                .slots
                .iter()
                .map(|t| {
                    ctx.compositor
                        .create_source_binding(ctx.gpu, &t.texture.create_view(&Default::default()))
                })
                .collect();
            self.current = None;
        }
        if s.latest.is_some() {
            s.reading = s.latest;
            self.current = s.latest;
            self.sequence = s.sequence;
        }
    }

    pub fn frame(&self) -> Option<SourceFrame<'_>> {
        let i = self.current?;
        let slot = self.slots.get(i)?;
        Some(SourceFrame {
            binding: self.bindings.get(i)?,
            size: Size::new(slot.width as f32, slot.height as f32),
        })
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// WinRT Direct3D devices are agile (usable from any thread).
struct AgileDevice(IDirect3DDevice);

// SAFETY: IDirect3DDevice from CreateDirect3D11DeviceFromDXGIDevice is agile.
unsafe impl Send for AgileDevice {}

impl AgileDevice {
    // A method (not field access) so closures capture the whole Send wrapper.
    fn get(&self) -> &IDirect3DDevice {
        &self.0
    }
}

/// What to capture.
#[derive(Clone, Copy, Debug)]
pub enum Target {
    Monitor(HMONITOR),
    Window(HWND),
}

/// A running capture session. Stops when dropped.
pub struct WgcCapture {
    session: GraphicsCaptureSession,
    pool: Direct3D11CaptureFramePool,
    pub ring: Arc<FrameRing>,
}

// SAFETY: WinRT capture objects are agile (free-threaded).
unsafe impl Send for WgcCapture {}

impl WgcCapture {
    pub fn is_supported() -> bool {
        GraphicsCaptureSession::IsSupported().unwrap_or(false)
    }

    pub fn start(
        interop: &D3D11Interop,
        target: Target,
        cursor: bool,
    ) -> Result<Self, CaptureError> {
        // SAFETY: COM/WinRT initialization and factory calls with valid handles.
        unsafe {
            tracing::debug!("wgc: start");
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
            let interop_factory =
                windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
            let item: GraphicsCaptureItem = match target {
                Target::Monitor(m) => interop_factory.CreateForMonitor(m)?,
                Target::Window(w) => interop_factory.CreateForWindow(w)?,
            };
            let dxgi: IDXGIDevice = interop.device.cast()?;
            let device: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)?.cast()?;
            tracing::debug!("wgc: item created");
            let size = item.Size()?;
            let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                &device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
                size,
            )?;
            let ring = Arc::new(FrameRing::default());

            let pool_size = Arc::new(Mutex::new(size));
            {
                let ring = ring.clone();
                let interop = interop.clone();
                let device = AgileDevice(device.clone());
                pool.FrameArrived(
                    &TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(
                        move |pool, _| {
                            tracing::trace!("wgc: frame arrived");
                            let Ok(pool) = pool.ok() else { return Ok(()) };
                            let frame = pool.TryGetNextFrame()?;
                            let content = frame.ContentSize()?;
                            let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
                            let texture: ID3D11Texture2D = access.GetInterface()?;
                            let (w, h) =
                                (content.Width.max(1) as u32, content.Height.max(1) as u32);
                            if let Err(e) = ring.publish(&interop, &texture, w, h)
                                && let Ok(mut s) = ring.state.lock()
                            {
                                s.error = Some(e.to_string());
                            }
                            drop(frame);
                            // Follow window/monitor size changes.
                            if let Ok(mut current) = pool_size.lock()
                                && (current.Width != content.Width
                                    || current.Height != content.Height)
                            {
                                *current = content;
                                let _ = pool.Recreate(
                                    device.get(),
                                    DirectXPixelFormat::B8G8R8A8UIntNormalized,
                                    2,
                                    content,
                                );
                            }
                            Ok(())
                        },
                    ),
                )?;
            }
            {
                let ring = ring.clone();
                item.Closed(
                    &TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
                        if let Ok(mut s) = ring.state.lock() {
                            s.closed = true;
                        }
                        Ok(())
                    }),
                )?;
            }

            let session = pool.CreateCaptureSession(&item)?;
            let _ = session.SetIsCursorCaptureEnabled(cursor);
            // Hide the yellow capture border where the OS allows it (Windows 11).
            let _ = session.SetIsBorderRequired(false);
            tracing::debug!("wgc: starting session");
            session.StartCapture()?;
            tracing::debug!("wgc: session started");
            Ok(Self {
                session,
                pool,
                ring,
            })
        }
    }

    pub fn set_cursor(&self, cursor: bool) {
        let _ = self.session.SetIsCursorCaptureEnabled(cursor);
    }
}

impl Drop for WgcCapture {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}
