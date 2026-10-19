//! A display or window shared through the ScreenCast portal and received over PipeWire.
//!
//! Each session runs on its own thread: the portal calls (which may wait for the user in the
//! system's picker), then a PipeWire loop that copies every frame out of the shared-memory
//! buffer into a [`FrameSlot`]. DMA-BUF is not offered, so the compositor gets SHM buffers.

use super::frames::{Frame, FrameSlot, copy_rows};
use super::tokens;
use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};
use ashpd::desktop::{PersistMode, ResponseError, Session as PortalSession};
use ashpd::enumflags2::BitFlags;
use estream_core::{SourceId, SourceStatus};
use estream_gpu::wgpu;
use pipewire as pw;
use pw::spa;
use pw::spa::param::video::{VideoFormat, VideoInfoRaw};
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Display,
    Window,
}

pub const STOPPED: &str = "Sharing stopped. Use “Choose again” to pick what to share";
const CANCELLED: &str = "Nothing chosen to share. Use “Choose again” to pick";

/// What the session thread and the source share.
pub struct Shared {
    pub frames: FrameSlot,
    status: Mutex<SourceStatus>,
    stop: AtomicBool,
}

impl Shared {
    pub fn status(&self) -> SourceStatus {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn set_status(&self, status: SourceStatus) {
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = status;
    }
}

/// A running screen-share. Dropping it ends the share (without waiting for the thread).
pub struct Session {
    pub shared: Arc<Shared>,
    quit: pw::channel::Sender<()>,
}

impl Session {
    /// Starts sharing; `forget` drops the saved choice first, so the picker shows.
    pub fn start(source: SourceId, target: Target, cursor: bool, forget: bool) -> Self {
        let shared = Arc::new(Shared {
            frames: FrameSlot::default(),
            status: Mutex::new(SourceStatus::Unavailable(
                "Waiting for the screen-share picker…".into(),
            )),
            stop: AtomicBool::new(false),
        });
        let (quit, quit_rx) = pw::channel::channel();
        let thread_shared = shared.clone();
        let spawned = std::thread::Builder::new()
            .name("estream-screencast".into())
            .spawn(move || {
                if forget {
                    tokens::set(source, None);
                }
                run(source, target, cursor, &thread_shared, quit_rx);
            });
        if let Err(e) = spawned {
            shared.set_status(SourceStatus::Error(e.to_string()));
        }
        Self { shared, quit }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        let _ = self.quit.send(());
    }
}

fn run(
    source: SourceId,
    target: Target,
    cursor: bool,
    shared: &Arc<Shared>,
    quit: pw::channel::Receiver<()>,
) {
    let (proxy, session, node, fd) = match pollster::block_on(open(source, target, cursor)) {
        Ok(opened) => opened,
        Err(status) => {
            tracing::info!(?target, ?status, "screen share not started");
            shared.set_status(status);
            return;
        }
    };
    tracing::info!(?target, node, "screen share started");
    if !shared.stop.load(Ordering::Acquire)
        && let Err(e) = receive(node, fd, shared, quit)
    {
        tracing::warn!(error = %e, "screen share stream failed");
        shared.set_status(SourceStatus::Error(e.to_string()));
    }
    let _ = pollster::block_on(session.close());
    drop(proxy);
}

/// Opens a portal session (the picker shows unless the saved token still applies) and the
/// PipeWire remote that carries it.
async fn open(
    source: SourceId,
    target: Target,
    cursor: bool,
) -> Result<(Screencast, PortalSession<Screencast>, u32, OwnedFd), SourceStatus> {
    let failed = |e: ashpd::Error| match e {
        ashpd::Error::Response(ResponseError::Cancelled) => {
            SourceStatus::Unavailable(CANCELLED.into())
        }
        e => SourceStatus::Error(format!("Screen sharing failed: {e}")),
    };
    let proxy = Screencast::new().await.map_err(|e| {
        SourceStatus::Unavailable(format!(
            "Screen capture needs the desktop's screen-sharing portal (xdg-desktop-portal): {e}"
        ))
    })?;
    let session = proxy
        .create_session(Default::default())
        .await
        .map_err(failed)?;
    let modes = proxy.available_cursor_modes().await.unwrap_or_default();
    let cursor_mode = if cursor && modes.contains(CursorMode::Embedded) {
        CursorMode::Embedded
    } else {
        CursorMode::Hidden
    };
    let kind = match target {
        Target::Display => SourceType::Monitor,
        Target::Window => SourceType::Window,
    };
    let token = tokens::get(source);
    proxy
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_cursor_mode(cursor_mode)
                .set_sources(BitFlags::from_flag(kind))
                .set_multiple(false)
                .set_restore_token(token.as_deref())
                .set_persist_mode(PersistMode::ExplicitlyRevoked),
        )
        .await
        .map_err(failed)?;
    let response = proxy
        .start(&session, None, Default::default())
        .await
        .map_err(failed)?
        .response()
        .map_err(failed)?;
    // Tokens are single-use: keep the new one (or forget a spent one).
    tokens::set(source, response.restore_token().map(str::to_owned));
    let node = response
        .streams()
        .first()
        .map(|s| s.pipe_wire_node_id())
        .ok_or_else(|| SourceStatus::Unavailable(CANCELLED.into()))?;
    let fd = proxy
        .open_pipe_wire_remote(&session, Default::default())
        .await
        .map_err(failed)?;
    Ok((proxy, session, node, fd))
}

struct Negotiated {
    width: u32,
    height: u32,
    format: Option<(wgpu::TextureFormat, bool)>,
    frame: Frame,
}

/// The texture format for a PipeWire format, and whether its 4th byte is padding.
fn texture_format(format: VideoFormat) -> Option<(wgpu::TextureFormat, bool)> {
    use wgpu::TextureFormat::{Bgra8UnormSrgb, Rgba8UnormSrgb};
    match format {
        VideoFormat::BGRA => Some((Bgra8UnormSrgb, false)),
        VideoFormat::BGRx => Some((Bgra8UnormSrgb, true)),
        VideoFormat::RGBA => Some((Rgba8UnormSrgb, false)),
        VideoFormat::RGBx => Some((Rgba8UnormSrgb, true)),
        _ => None,
    }
}

/// Runs the PipeWire loop for `node` until the share ends or the session is dropped.
fn receive(
    node: u32,
    fd: OwnedFd,
    shared: &Arc<Shared>,
    quit: pw::channel::Receiver<()>,
) -> Result<(), pw::Error> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextBox::new(mainloop.loop_(), None)?;
    let core = context.connect_fd(fd, None)?;
    let _quit = quit.attach(mainloop.loop_(), {
        let mainloop = mainloop.clone();
        move |()| mainloop.quit()
    });
    let stream = pw::stream::StreamBox::new(
        &core,
        "clutch-screen-capture",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )?;
    let _listener = stream
        .add_local_listener_with_user_data(Negotiated {
            width: 0,
            height: 0,
            format: None,
            frame: Frame::default(),
        })
        .state_changed({
            let (shared, mainloop) = (shared.clone(), mainloop.clone());
            move |_, _, _, new| match new {
                pw::stream::StreamState::Streaming => shared.set_status(SourceStatus::Active),
                pw::stream::StreamState::Error(e) => {
                    shared.set_status(SourceStatus::Error(e));
                    mainloop.quit();
                }
                pw::stream::StreamState::Unconnected => {
                    shared.set_status(SourceStatus::Unavailable(STOPPED.into()));
                    mainloop.quit();
                }
                _ => {}
            }
        })
        .param_changed(|_, n, id, param| {
            let Some(param) = param else { return };
            if id != spa::param::ParamType::Format.as_raw() {
                return;
            }
            let mut info = VideoInfoRaw::default();
            if info.parse(param).is_ok() {
                n.width = info.size().width;
                n.height = info.size().height;
                n.format = texture_format(info.format());
                tracing::debug!(format = ?info.format(), n.width, n.height, "screen share format");
            }
        })
        .process({
            let shared = shared.clone();
            move |stream, n| {
                let Some(mut buffer) = stream.dequeue_buffer() else {
                    return;
                };
                let Some((format, opaque)) = n.format else {
                    return;
                };
                let Some(data) = buffer.datas_mut().first_mut() else {
                    return;
                };
                let chunk = data.chunk();
                let (offset, size, stride) = (
                    chunk.offset() as usize,
                    chunk.size() as usize,
                    chunk.stride(),
                );
                if size == 0 || chunk.flags().contains(spa::buffer::ChunkFlags::CORRUPTED) {
                    return; // cursor-only update, or a damaged frame
                }
                let stride = if stride > 0 {
                    stride as usize
                } else {
                    n.width as usize * 4
                };
                let Some(src) = data.data().and_then(|m| m.get(offset..)) else {
                    return;
                };
                if copy_rows(src, n.width, n.height, stride, opaque, &mut n.frame.data) {
                    n.frame.width = n.width;
                    n.frame.height = n.height;
                    n.frame.format = format;
                    shared.frames.publish(&mut n.frame);
                }
            }
        })
        .register()?;

    let format = format_param()?;
    let mut params = [spa::pod::Pod::from_bytes(&format).ok_or(pw::Error::CreationFailed)?];
    stream.connect(
        spa::utils::Direction::Input,
        Some(node),
        pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
        &mut params,
    )?;
    if !shared.stop.load(Ordering::Acquire) {
        mainloop.run();
    }
    let _ = stream.disconnect();
    Ok(())
}

/// The formats we accept: 4-byte RGB in any size and rate (no modifiers, so SHM buffers).
fn format_param() -> Result<Vec<u8>, pw::Error> {
    use spa::param::format::{FormatProperties, MediaSubtype, MediaType};
    let object = spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        spa::pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        spa::pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        spa::pod::property!(
            FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRx,
            VideoFormat::BGRx,
            VideoFormat::BGRA,
            VideoFormat::RGBx,
            VideoFormat::RGBA
        ),
        spa::pod::property!(
            FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            spa::utils::Rectangle {
                width: 1920,
                height: 1080
            },
            spa::utils::Rectangle {
                width: 1,
                height: 1
            },
            spa::utils::Rectangle {
                width: 16384,
                height: 16384
            }
        ),
        spa::pod::property!(
            FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            spa::utils::Fraction { num: 60, denom: 1 },
            spa::utils::Fraction { num: 0, denom: 1 },
            spa::utils::Fraction { num: 360, denom: 1 }
        ),
    );
    spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(object),
    )
    .map(|(cursor, _)| cursor.into_inner())
    .map_err(|_| pw::Error::CreationFailed)
}
