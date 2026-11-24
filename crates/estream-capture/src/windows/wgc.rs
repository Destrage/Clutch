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
use estream_gpu::HdrToSdr;
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
use windows::Win32::Graphics::Direct3D11::{D3D11_TEXTURE2D_DESC, ID3D11Texture2D};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R16G16B16A16_FLOAT;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows::core::{IInspectable, Interface};

/// Keeps the process's multithreaded apartment alive for its lifetime (once), so threads
/// without COM can use WinRT through it and cached WinRT factories never outlive it.
fn keep_mta() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // SAFETY: plain COM call; the cookie is never released, on purpose.
        if let Err(e) = unsafe { windows::Win32::System::Com::CoIncrementMTAUsage() } {
            tracing::warn!(error = %e, "couldn't keep the COM MTA alive");
        }
    });
}

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
    /// The captured monitor's HDR levels while it shows HDR (frames are then scRGB).
    hdr: Option<super::hdr::HdrDisplay>,
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
            let hdr = is_hdr_texture(src);
            let size_changed = s
                .slots
                .first()
                .is_none_or(|t| t.width != width || t.height != height || t.hdr != hdr);
            if size_changed {
                tracing::debug!(width, height, hdr, "wgc: creating shared slots");
                let slots = (0..RING)
                    .map(|_| interop.create_shared_as(width, height, hdr).map(Arc::new))
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

/// Whether a captured texture is 16-bit float (scRGB from an HDR monitor).
fn is_hdr_texture(texture: &ID3D11Texture2D) -> bool {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: reading the descriptor of a live texture.
    unsafe { texture.GetDesc(&mut desc) };
    desc.Format == DXGI_FORMAT_R16G16B16A16_FLOAT
}

impl FrameRing {
    fn set_hdr(&self, hdr: Option<super::hdr::HdrDisplay>) {
        if let Ok(mut s) = self.state.lock() {
            s.hdr = hdr;
        }
    }
}

/// HDR slots are mapped to SDR into one texture, which the compositor samples instead.
struct Mapped {
    // Kept alive with its view.
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// The compositor's binding of `texture`.
    binding: wgpu::BindGroup,
    params: wgpu::Buffer,
    /// Per slot: the mapping pass's binding.
    inputs: Vec<wgpu::BindGroup>,
    shown: Option<estream_gpu::HdrParams>,
}

/// Video-thread side of a [`FrameRing`]: keeps wgpu bindings for the slots.
#[derive(Default)]
pub struct RingReader {
    generation: u64,
    slots: Vec<Arc<SharedTexture>>,
    bindings: Vec<wgpu::BindGroup>,
    mapped: Option<Mapped>,
    current: Option<usize>,
    pub sequence: u64,
    last_tick: Option<Instant>,
    /// Smoothed time between ticks (the video thread's frame interval), in seconds.
    tick_interval: f64,
}

impl RingReader {
    /// Picks up the newest frame and orders this frame's GPU work after its copy; HDR frames
    /// are mapped to SDR into `encoder`. Never blocks: if the capture thread holds the lock,
    /// the previous frame is kept.
    pub fn tick(
        &mut self,
        ring: &FrameRing,
        ctx: &SourceContext<'_>,
        interop: &D3D11Interop,
        encoder: &mut wgpu::CommandEncoder,
        mapper: &HdrToSdr,
    ) {
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
            let hdr = self.slots.first().is_some_and(|t| t.hdr);
            self.bindings = if hdr {
                Vec::new()
            } else {
                self.slots
                    .iter()
                    .map(|t| {
                        ctx.compositor.create_source_binding(
                            ctx.gpu,
                            &t.texture.create_view(&Default::default()),
                        )
                    })
                    .collect()
            };
            self.mapped = self
                .slots
                .first()
                .filter(|_| hdr)
                .map(|first| self.create_mapped(ctx, mapper, first.width, first.height));
            self.current = None;
        }
        let Some(latest) = s.latest else { return };
        let fresh = self.current != Some(latest);
        if fresh {
            // Work submitted so far is the last to read the old slot.
            if let Some(old) = self.current {
                s.released[old] = interop.release_reads();
            }
            interop.wait_for_copy(s.copied[latest]);
        }
        s.reading = Some(latest);
        self.current = Some(latest);
        self.sequence = s.sequence;
        let levels = s.hdr.map(|d| d.params()).unwrap_or_else(|| {
            estream_gpu::HdrParams::new(estream_gpu::hdr::DEFAULT_SDR_WHITE_NITS, 0.0)
        });
        drop(s);
        if let Some(m) = &mut self.mapped {
            let changed = m.shown != Some(levels);
            if changed {
                ctx.gpu.queue.write_buffer(&m.params, 0, levels.as_bytes());
                m.shown = Some(levels);
            }
            if (fresh || changed)
                && let Some(input) = m.inputs.get(latest)
            {
                mapper.encode(encoder, input, &m.view);
            }
        }
    }

    fn create_mapped(
        &self,
        ctx: &SourceContext<'_>,
        mapper: &HdrToSdr,
        width: u32,
        height: u32,
    ) -> Mapped {
        let texture = mapper.create_target(ctx.gpu, width, height);
        let view = texture.create_view(&Default::default());
        let binding = ctx.compositor.create_source_binding(ctx.gpu, &view);
        let params = mapper.create_params(
            ctx.gpu,
            &estream_gpu::HdrParams::new(estream_gpu::hdr::DEFAULT_SDR_WHITE_NITS, 0.0),
        );
        let inputs = self
            .slots
            .iter()
            .map(|t| {
                mapper.bind(
                    ctx.gpu,
                    &t.texture.create_view(&Default::default()),
                    &params,
                )
            })
            .collect();
        Mapped {
            _texture: texture,
            view,
            binding,
            params,
            inputs,
            shown: None,
        }
    }

    /// The video thread's frame interval as seen by this source, if known yet.
    pub fn frame_interval(&self) -> Option<Duration> {
        (self.tick_interval > 0.0).then(|| Duration::from_secs_f64(self.tick_interval))
    }

    pub fn frame(&self) -> Option<SourceFrame<'_>> {
        let i = self.current?;
        let slot = self.slots.get(i)?;
        let binding = match &self.mapped {
            Some(m) => &m.binding,
            None => self.bindings.get(i)?,
        };
        Some(SourceFrame {
            binding,
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

// SAFETY: monitor and window handles are plain values, valid on any thread.
unsafe impl Send for Target {}

impl Target {
    /// The HDR levels of the monitor showing the target, when it is in HDR.
    fn hdr(self) -> Option<super::hdr::HdrDisplay> {
        #[cfg(test)]
        if tests::FORCE_HDR.load(Ordering::Relaxed) {
            // An SDR monitor captured as scRGB: SDR white is 80 nits there.
            return Some(super::hdr::HdrDisplay {
                sdr_white_nits: 80.0,
                peak_nits: 80.0,
            });
        }
        let monitor = match self {
            Target::Monitor(m) => m,
            Target::Window(w) => super::hdr::monitor_of_window(w),
        };
        super::hdr::monitor_hdr(monitor)
    }
}

/// How often a capture checks whether its monitor's HDR state changed.
const HDR_CHECK: Duration = Duration::from_secs(1);

/// The frame pool's size and format, as last set.
struct PoolState {
    size: windows::Graphics::SizeInt32,
    hdr: Option<super::hdr::HdrDisplay>,
    checked: Instant,
}

fn pixel_format(hdr: bool) -> DirectXPixelFormat {
    if hdr {
        DirectXPixelFormat::R16G16B16A16Float
    } else {
        DirectXPixelFormat::B8G8R8A8UIntNormalized
    }
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
    /// The WinRT call fails on a thread without COM (Clutch started after "Restore unsaved
    /// changes?" called it there), which hid display, window and game capture for that
    /// session. Keeping the process's MTA alive lets an uninitialized thread make it, and
    /// leaves the caller's own apartment as it was (a helper thread that initialized and
    /// exited tore the MTA down under the cached WinRT factories, which then crashed).
    pub fn is_supported() -> bool {
        keep_mta();
        GraphicsCaptureSession::IsSupported().unwrap_or(false)
    }

    /// Starts capturing `target`. With `map_hdr`, an HDR monitor is captured in 16-bit float
    /// (mapped to SDR on the GPU); otherwise Windows' own 8-bit picture is used.
    pub fn start(
        interop: &D3D11Interop,
        target: Target,
        cursor: bool,
        map_hdr: bool,
    ) -> Result<Self, CaptureError> {
        // SAFETY: COM/WinRT initialization and factory calls with valid handles.
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
            let interop_factory =
                windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
            let item: GraphicsCaptureItem = match target {
                Target::Monitor(m) => interop_factory.CreateForMonitor(m)?,
                Target::Window(w) => interop_factory.CreateForWindow(w)?,
            };
            let dxgi: IDXGIDevice = interop.device.cast()?;
            let device: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)?.cast()?;
            let size = item.Size()?;
            let hdr = if map_hdr { target.hdr() } else { None };
            if let Some(d) = hdr {
                tracing::info!(
                    sdr_white_nits = d.sdr_white_nits,
                    peak_nits = d.peak_nits,
                    "capturing an HDR monitor: mapping it to SDR"
                );
            }
            let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                &device,
                pixel_format(hdr.is_some()),
                POOL_BUFFERS,
                size,
            )?;
            let ring = Arc::new(FrameRing::default());
            ring.set_hdr(hdr);

            let pool_state = Arc::new(Mutex::new(PoolState {
                size,
                hdr,
                checked: Instant::now(),
            }));
            {
                let ring = ring.clone();
                let interop = interop.clone();
                let device = AgileDevice(device.clone());
                pool.FrameArrived(
                    &TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(
                        move |pool, _| {
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
                            let Ok(mut state) = pool_state.lock() else {
                                return Ok(());
                            };
                            // HDR switched on or off, or a window moved to another monitor:
                            // checked once a second, here on the capture thread.
                            let mut recreate = false;
                            if map_hdr && state.checked.elapsed() >= HDR_CHECK {
                                state.checked = Instant::now();
                                let now = target.hdr();
                                if now.is_some() != state.hdr.is_some() {
                                    tracing::info!(hdr = now.is_some(), "capture: HDR changed");
                                    recreate = true;
                                }
                                if now != state.hdr {
                                    state.hdr = now;
                                    ring.set_hdr(now);
                                }
                            }
                            // Follow window/monitor size changes.
                            if state.size.Width != content.Width
                                || state.size.Height != content.Height
                            {
                                state.size = content;
                                recreate = true;
                            }
                            if recreate {
                                let _ = pool.Recreate(
                                    device.get(),
                                    pixel_format(state.hdr.is_some()),
                                    POOL_BUFFERS,
                                    state.size,
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
            session.StartCapture()?;
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

    /// Makes every capture started by a test treat its monitor as HDR.
    pub(super) static FORCE_HDR: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    /// Windows delivers 16-bit float frames when asked, and the ring keeps them as HDR slots
    /// (an SDR monitor works too: it is then scRGB with white at 1.0). No window opens.
    #[test]
    fn hdr_capture_delivers_float_frames() {
        let Some((interop, _)) = setup() else {
            eprintln!("skipping: no D3D12 GPU");
            return;
        };
        if !WgcCapture::is_supported() {
            eprintln!("skipping: no Windows.Graphics.Capture");
            return;
        }
        let Some(primary) = super::super::enumerate::monitors()
            .into_iter()
            .find(|m| m.primary)
        else {
            eprintln!("skipping: no monitor");
            return;
        };
        FORCE_HDR.store(true, Ordering::Relaxed);
        let capture = WgcCapture::start(&interop, Target::Monitor(primary.handle), false, true);
        FORCE_HDR.store(false, Ordering::Relaxed);
        let capture = capture.unwrap();
        let start = Instant::now();
        while sequence(&capture.ring) == 0 && start.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(20));
        }
        {
            let s = capture.ring.state.lock().unwrap();
            assert!(s.sequence > 0, "no frame arrived: {:?}", s.error);
            assert!(s.slots.iter().all(|t| t.hdr), "frames weren't 16-bit float");
            assert_eq!(s.hdr.map(|d| d.sdr_white_nits), Some(80.0));
        }

        // The video thread's side: the newest frame is mapped into the texture the compositor
        // samples, and nothing in it is brighter than SDR white.
        let gpu = interop.gpu().clone();
        let compositor = estream_gpu::Compositor::new(&gpu);
        let gradients = estream_gpu::GradientRenderer::new(&gpu);
        let ctx = SourceContext {
            gpu: &gpu,
            compositor: &compositor,
            gradients: &gradients,
            source: estream_core::SourceId::new(),
        };
        let mapper = HdrToSdr::new(&gpu);
        let mut reader = RingReader::default();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        reader.tick(&capture.ring, &ctx, &interop, &mut encoder, &mapper);
        gpu.queue.submit([encoder.finish()]);
        assert!(reader.frame().is_some());
        let mapped = reader.mapped.as_ref().expect("HDR frames are mapped");
        let bytes = estream_gpu::readback::read_texture(&gpu, &mapped._texture).unwrap();
        let half = |b: &[u8]| {
            let bits = u16::from_le_bytes([b[0], b[1]]);
            let exp = ((bits >> 10) & 0x1f) as i32;
            let frac = (bits & 0x3ff) as f32;
            if exp == 0 {
                frac * 2f32.powi(-24)
            } else {
                (1.0 + frac / 1024.0) * 2f32.powi(exp - 15)
            }
        };
        let mut lit = 0usize;
        for px in bytes.as_chunks::<8>().0 {
            for c in 0..3 {
                let v = half(&px[c * 2..c * 2 + 2]);
                assert!((0.0..=1.001).contains(&v), "channel value {v}");
                if v > 0.01 {
                    lit += 1;
                }
            }
        }
        assert!(lit > 0, "the mapped picture is black");
    }

    #[test]
    fn slots_follow_the_captured_format() {
        let Some((interop, sdr)) = setup() else {
            eprintln!("skipping: no D3D12 GPU");
            return;
        };
        let hdr = interop.create_shared_as(64, 32, true).unwrap();
        assert!(is_hdr_texture(&hdr.d3d11) && !is_hdr_texture(&sdr.d3d11));
        let ring = FrameRing::default();
        ring.publish(&interop, &sdr.d3d11, 64, 32).unwrap();
        let first = ring.state.lock().unwrap().generation;
        ring.publish(&interop, &hdr.d3d11, 64, 32).unwrap();
        let s = ring.state.lock().unwrap();
        assert!(s.generation > first, "HDR frames get new slots");
        assert!(s.slots.iter().all(|t| t.hdr));
        assert_eq!(s.sequence, 2);
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
