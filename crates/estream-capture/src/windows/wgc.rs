//! Windows.Graphics.Capture sessions feeding a ring of GPU-shared textures.
//!
//! Frames arrive on a free-threaded WinRT callback. Each one is copied (GPU→GPU)
//! into a free slot of a 4-texture ring shared with wgpu; the video thread picks
//! up the newest slot each tick and has its queue wait (on the GPU) for that copy.
//! The newest slot, the slot being read and slots that wgpu's in-flight work may
//! still sample are never written, so frames never tear; when none is free the
//! frame is dropped instead of waiting. Frames arriving much faster than the video
//! thread renders are released without a copy.

use super::d3d::{D3D11Interop, SharedTexture};
use crate::CaptureError;
use estream_core::Size;
use estream_engine::{SourceContext, SourceFrame};
use estream_gpu::wgpu;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Foundation::{TimeSpan, TypedEventHandler};
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

const RING: usize = 4;

/// WGC buffers. Frames are released right after their copy is queued (not finished), so a
/// third buffer keeps Windows from rendering into the one being copied.
const POOL_BUFFERS: i32 = 3;

/// Copy a new frame once at least this share of the video thread's frame interval passed
/// since the last copy. Half keeps a fresh frame for every render without beating against
/// the game's frame rate.
const COPY_SHARE: f64 = 0.5;

#[derive(Default)]
struct RingState {
    /// Bumped whenever the slot textures are recreated (size change).
    generation: u64,
    slots: Vec<Arc<SharedTexture>>,
    /// Per slot: the copy fence value that finishes its frame.
    copied: [u64; RING],
    /// Per slot: the release fence value after which wgpu no longer reads it.
    released: [u64; RING],
    latest: Option<usize>,
    reading: Option<usize>,
    sequence: u64,
    last_copy: Option<Instant>,
    closed: bool,
    error: Option<String>,
}

/// Frames shared between a capture callback and the video thread.
#[derive(Default)]
pub struct FrameRing {
    state: Mutex<RingState>,
    /// Shortest time between copies, in nanoseconds (0: copy every frame).
    min_copy_interval: AtomicU64,
    /// Frames released without a copy (rate cap or no free slot).
    skipped: AtomicU64,
}

impl FrameRing {
    pub fn is_closed(&self) -> bool {
        self.state.lock().map(|s| s.closed).unwrap_or(true)
    }

    pub fn error(&self) -> Option<String> {
        self.state.lock().ok().and_then(|s| s.error.clone())
    }

    /// Frames released without being copied so far.
    pub fn skipped(&self) -> u64 {
        self.skipped.load(Ordering::Relaxed)
    }

    /// Copies a captured texture into a free slot and publishes it. Never waits for the GPU
    /// (unless the driver lacks fences): drops the frame when it isn't needed yet or every
    /// slot is still in use.
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
                s.copied = [0; RING];
                s.released = [0; RING];
                s.latest = None;
                s.reading = None;
                s.last_copy = None;
            }
            let min = Duration::from_nanos(self.min_copy_interval.load(Ordering::Relaxed));
            if s.last_copy.is_some_and(|t| t.elapsed() < min) {
                self.skipped.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
            let free = (0..RING).find(|&i| {
                Some(i) != s.latest && Some(i) != s.reading && interop.released(s.released[i])
            });
            let Some(free) = free else {
                self.skipped.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            };
            s.last_copy = Some(Instant::now());
            (free, s.slots[free].clone())
        };
        slot.1.copy_from(interop, src, width, height);
        let copied = interop.finish_copies()?;
        let mut s = self
            .state
            .lock()
            .map_err(|_| CaptureError::Capture("ring poisoned".into()))?;
        if s.slots.get(slot.0).is_some_and(|t| Arc::ptr_eq(t, &slot.1)) {
            s.copied[slot.0] = copied;
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
    last_tick: Option<Instant>,
    /// Smoothed time between ticks (the video thread's frame interval), in seconds.
    tick_interval: f64,
}

impl RingReader {
    /// Picks up the newest frame and orders this frame's GPU work after its copy. Never
    /// blocks: if the capture thread holds the lock, the previous frame is kept.
    pub fn tick(&mut self, ring: &FrameRing, ctx: &SourceContext<'_>, interop: &D3D11Interop) {
        let now = Instant::now();
        if let Some(last) = self.last_tick.replace(now) {
            let dt = (now - last).as_secs_f64().min(0.25);
            self.tick_interval = if self.tick_interval == 0.0 {
                dt
            } else {
                self.tick_interval * 0.9 + dt * 0.1
            };
            ring.min_copy_interval.store(
                (self.tick_interval * COPY_SHARE * 1e9) as u64,
                Ordering::Relaxed,
            );
        }
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
        let Some(latest) = s.latest else { return };
        if self.current != Some(latest) {
            // Work submitted so far is the last to read the old slot.
            if let Some(old) = self.current {
                s.released[old] = interop.release_reads();
            }
            interop.wait_for_copy(s.copied[latest]);
        }
        s.reading = Some(latest);
        self.current = Some(latest);
        self.sequence = s.sequence;
    }

    /// The video thread's frame interval as seen by this source, if known yet.
    pub fn frame_interval(&self) -> Option<Duration> {
        (self.tick_interval > 0.0).then(|| Duration::from_secs_f64(self.tick_interval))
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
    pub interop: D3D11Interop,
    /// The update interval last asked of Windows.
    min_update: Option<Duration>,
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
                POOL_BUFFERS,
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
                                    POOL_BUFFERS,
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
                interop: interop.clone(),
                min_update: None,
            })
        }
    }

    pub fn set_cursor(&self, cursor: bool) {
        let _ = self.session.SetIsCursorCaptureEnabled(cursor);
    }

    /// Asks Windows to deliver frames no more often than `interval` (Windows 11 24H2 and
    /// later; ignored elsewhere, where the ring's own cap applies).
    pub fn set_min_update_interval(&mut self, interval: Duration) {
        // Only when it moved by more than 10%: the estimate jitters frame to frame.
        if self.min_update.is_some_and(|old| {
            (old.as_secs_f64() - interval.as_secs_f64()).abs() <= old.as_secs_f64() * 0.1
        }) {
            return;
        }
        self.min_update = Some(interval);
        let span = TimeSpan {
            Duration: (interval.as_nanos() / 100) as i64,
        };
        if let Err(e) = self.session.SetMinUpdateInterval(span) {
            tracing::debug!(error = %e, "capture update interval not supported");
        }
    }
}

impl Drop for WgcCapture {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use estream_gpu::{GpuContext, GpuOptions};

    fn setup() -> Option<(D3D11Interop, SharedTexture)> {
        let gpu = GpuContext::new_headless(&GpuOptions::default()).ok()?;
        let interop = D3D11Interop::new(&gpu).ok()?;
        let src = interop.create_shared(64, 32).ok()?;
        Some((interop, src))
    }

    fn sequence(ring: &FrameRing) -> u64 {
        ring.state.lock().unwrap().sequence
    }

    #[test]
    fn frames_faster_than_needed_are_not_copied() {
        let Some((interop, src)) = setup() else {
            eprintln!("skipping: no D3D12 GPU");
            return;
        };
        let ring = FrameRing::default();
        ring.min_copy_interval
            .store(Duration::from_secs(10).as_nanos() as u64, Ordering::Relaxed);
        for _ in 0..3 {
            ring.publish(&interop, &src.d3d11, 64, 32).unwrap();
        }
        assert_eq!(sequence(&ring), 1);
        assert_eq!(ring.skipped(), 2);

        ring.min_copy_interval.store(0, Ordering::Relaxed);
        ring.publish(&interop, &src.d3d11, 64, 32).unwrap();
        assert_eq!(sequence(&ring), 2);
    }

    #[test]
    fn slots_still_read_by_the_gpu_are_never_overwritten() {
        let Some((interop, src)) = setup() else {
            eprintln!("skipping: no D3D12 GPU");
            return;
        };
        if !interop.has_fences() {
            eprintln!("skipping: no D3D11.4 fences");
            return;
        }
        let ring = FrameRing::default();
        ring.publish(&interop, &src.d3d11, 64, 32).unwrap();
        let latest = {
            let mut s = ring.state.lock().unwrap();
            // Every other slot is still read by work that never finishes.
            let latest = s.latest.unwrap();
            for i in (0..RING).filter(|&i| i != latest) {
                s.released[i] = u64::MAX;
            }
            latest
        };
        ring.publish(&interop, &src.d3d11, 64, 32).unwrap();
        let s = ring.state.lock().unwrap();
        assert_eq!(s.latest, Some(latest), "a busy slot was written");
        assert_eq!(s.sequence, 1);
        assert_eq!(ring.skipped(), 1);
    }
}
