//! Per-application audio through PipeWire: one capture stream per playback stream of the
//! chosen app (browsers and games often open several), mixed on the audio thread. Streams
//! come and go with the app's sound; the source re-attaches whenever it plays again.

use estream_audio::convert::InputConverter;
use estream_audio::ring::{StereoConsumer, StereoProducer, stereo_ring};
use estream_audio::{AudioSource, Block, SAMPLE_RATE};
use estream_core::{SourceStatus, Timestamp};
use estream_engine::{Property, PropertySchema, SelectOption, SourceType};
use pipewire as pw;
use pw::spa;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

/// Whether a playback stream's app (its executable or name) is `wanted`. Case and a
/// Windows `.exe` suffix are ignored, so projects carry over between systems.
pub fn app_matches(wanted: &str, app: &str) -> bool {
    let bare = |s: &str| {
        let s = s.trim().to_lowercase();
        s.strip_suffix(".exe").map(str::to_owned).unwrap_or(s)
    };
    !wanted.trim().is_empty() && bare(wanted) == bare(app)
}

enum Msg {
    Add(u32, StereoConsumer),
    Remove(u32),
}

/// Keep at most this much buffered per stream; trim to the jitter headroom beyond it.
const MAX_BUFFERED: usize = SAMPLE_RATE as usize * 60 / 1000;
const KEEP_BUFFERED: usize = SAMPLE_RATE as usize * 20 / 1000;

struct Session {
    rx: Receiver<Msg>,
    quit: pw::channel::Sender<()>,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.quit.send(());
    }
}

pub struct AppAudioSource {
    app: String,
    session: Option<Session>,
    inputs: Vec<(u32, StereoConsumer)>,
    scratch: Block,
    status: Arc<Mutex<SourceStatus>>,
}

impl AppAudioSource {
    fn new() -> Self {
        Self {
            app: String::new(),
            session: None,
            inputs: Vec::with_capacity(8),
            scratch: Block::SILENT,
            status: Arc::new(Mutex::new(SourceStatus::Unavailable(
                "Choose an application".into(),
            ))),
        }
    }
}

impl AudioSource for AppAudioSource {
    fn update(&mut self, settings: &Value) -> Result<(), String> {
        let app = settings
            .get("app")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if app == self.app && (self.session.is_some() || app.is_empty()) {
            return Ok(());
        }
        self.session = None;
        self.inputs.clear();
        self.app = app.clone();
        let set = |s| *self.status.lock().unwrap_or_else(|e| e.into_inner()) = s;
        if app.is_empty() {
            set(SourceStatus::Unavailable("Choose an application".into()));
            return Ok(());
        }
        set(waiting(&app));
        let (tx, rx) = channel();
        let (quit, quit_rx) = pw::channel::channel();
        let status = self.status.clone();
        std::thread::Builder::new()
            .name("estream-app-audio".into())
            .spawn(move || {
                if let Err(e) = run(&app, &tx, &status, quit_rx) {
                    tracing::warn!(error = %e, %app, "app audio capture failed");
                    *status.lock().unwrap_or_else(|e| e.into_inner()) =
                        SourceStatus::Error(format!("PipeWire: {e}"));
                }
            })
            .map_err(|e| e.to_string())?;
        self.session = Some(Session { rx, quit });
        Ok(())
    }

    fn read(&mut self, block: &mut Block, _: Timestamp) -> usize {
        if let Some(session) = &self.session {
            while let Ok(msg) = session.rx.try_recv() {
                match msg {
                    Msg::Add(id, consumer) => self.inputs.push((id, consumer)),
                    Msg::Remove(id) => self.inputs.retain(|(i, _)| *i != id),
                }
            }
        }
        block.clear();
        let mut frames = 0;
        for (_, consumer) in &mut self.inputs {
            let available = consumer.available();
            if available > MAX_BUFFERED {
                consumer.skip(available - KEEP_BUFFERED);
            }
            self.scratch.clear();
            frames = frames.max(consumer.read_block(&mut self.scratch));
            block.add(&self.scratch);
        }
        frames
    }

    fn status(&self) -> SourceStatus {
        // Never block the audio thread on the status lock.
        self.status
            .try_lock()
            .map(|s| s.clone())
            .unwrap_or(SourceStatus::Active)
    }
}

fn waiting(app: &str) -> SourceStatus {
    SourceStatus::Unavailable(format!("Waiting for {app} to play sound"))
}

/// A capture stream on one of the app's playback streams.
struct Capture {
    _stream: pw::stream::StreamRc,
    _listener: pw::stream::StreamListener<(InputConverter, StereoProducer, Vec<f32>)>,
}

/// Watches the graph and captures every playback stream of `app` until told to quit.
fn run(
    app: &str,
    tx: &Sender<Msg>,
    status: &Arc<Mutex<SourceStatus>>,
    quit: pw::channel::Receiver<()>,
) -> Result<(), pw::Error> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let registry = core.get_registry_rc()?;
    let _quit = quit.attach(mainloop.loop_(), {
        let mainloop = mainloop.clone();
        move |()| mainloop.quit()
    });
    let captures: Rc<RefCell<HashMap<u32, Capture>>> = Rc::default();
    let format = format_param()?;
    let set_status = {
        let (status, app) = (status.clone(), app.to_owned());
        move |count: usize| {
            *status.lock().unwrap_or_else(|e| e.into_inner()) = if count > 0 {
                SourceStatus::Active
            } else {
                waiting(&app)
            };
        }
    };
    let _registry_listener = registry
        .add_listener_local()
        .global({
            let (captures, core, tx, app, set_status) = (
                captures.clone(),
                core.clone(),
                tx.clone(),
                app.to_owned(),
                set_status.clone(),
            );
            move |global| {
                if global.type_ != pw::types::ObjectType::Node {
                    return;
                }
                let Some(props) = global.props else { return };
                if props.get("media.class") != Some("Stream/Output/Audio")
                    || !estream_audio::pipewire::app_of(props)
                        .is_some_and(|a| app_matches(&app, &a))
                {
                    return;
                }
                let Some(serial) = props.get(*pw::keys::OBJECT_SERIAL) else {
                    return;
                };
                match capture(&core, serial, &format) {
                    Ok((capture, consumer)) => {
                        tracing::info!(%app, node = global.id, "app audio stream attached");
                        let _ = tx.send(Msg::Add(global.id, consumer));
                        let mut captures = captures.borrow_mut();
                        captures.insert(global.id, capture);
                        set_status(captures.len());
                    }
                    Err(e) => tracing::warn!(error = %e, %app, "couldn't capture app stream"),
                }
            }
        })
        .global_remove({
            let (captures, tx) = (captures.clone(), tx.clone());
            move |id| {
                let mut list = captures.borrow_mut();
                if list.remove(&id).is_some() {
                    let _ = tx.send(Msg::Remove(id));
                    set_status(list.len());
                }
            }
        })
        .register();
    mainloop.run();
    captures.borrow_mut().clear();
    Ok(())
}

/// A capture stream linked to the playback node `serial`: 48 kHz stereo f32 (PipeWire
/// converts), pushed into a ring the audio thread reads.
fn capture(
    core: &pw::core::CoreRc,
    serial: &str,
    format: &[u8],
) -> Result<(Capture, StereoConsumer), pw::Error> {
    let stream = pw::stream::StreamRc::new(
        core.clone(),
        "clutch-app-audio",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Production",
            *pw::keys::TARGET_OBJECT => serial,
            // When the app's stream ends, stop instead of falling back to a microphone.
            *pw::keys::NODE_DONT_RECONNECT => "true",
        },
    )?;
    // 500 ms of headroom; the source trims to keep latency low.
    let (producer, consumer) = stereo_ring(SAMPLE_RATE as usize / 2);
    let listener = stream
        .add_local_listener_with_user_data((
            InputConverter::new(2, SAMPLE_RATE),
            producer,
            Vec::new(),
        ))
        .process(|stream, (converter, producer, samples)| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let Some(data) = buffer.datas_mut().first_mut() else {
                return;
            };
            let (offset, size) = (data.chunk().offset() as usize, data.chunk().size() as usize);
            let Some(bytes) = data.data().and_then(|d| d.get(offset..offset + size)) else {
                return;
            };
            samples.clear();
            samples.extend(
                bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b)),
            );
            converter.push(samples, producer);
        })
        .register()?;
    let mut params = [spa::pod::Pod::from_bytes(format).ok_or(pw::Error::CreationFailed)?];
    stream.connect(
        spa::utils::Direction::Input,
        None,
        pw::stream::StreamFlags::AUTOCONNECT
            | pw::stream::StreamFlags::MAP_BUFFERS
            | pw::stream::StreamFlags::RT_PROCESS,
        &mut params,
    )?;
    Ok((
        Capture {
            _stream: stream,
            _listener: listener,
        },
        consumer,
    ))
}

/// 48 kHz interleaved stereo f32.
fn format_param() -> Result<Vec<u8>, pw::Error> {
    let mut info = spa::param::audio::AudioInfoRaw::new();
    info.set_format(spa::param::audio::AudioFormat::F32LE);
    info.set_rate(SAMPLE_RATE);
    info.set_channels(2);
    let mut position = [0; spa::sys::SPA_AUDIO_MAX_CHANNELS as usize];
    position[0] = spa::sys::SPA_AUDIO_CHANNEL_FL;
    position[1] = spa::sys::SPA_AUDIO_CHANNEL_FR;
    info.set_position(position);
    let object = spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: info.into(),
    };
    spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(object),
    )
    .map(|(cursor, _)| cursor.into_inner())
    .map_err(|_| pw::Error::CreationFailed)
}

pub fn source_type() -> SourceType {
    SourceType::audio(
        "app_audio",
        "Application audio",
        "Capture one app's audio (games, browsers, music players)",
        |settings| {
            let mut apps: Vec<String> = estream_audio::pipewire::snapshot()
                .map(|g| g.apps.into_iter().map(|a| a.app).collect())
                .unwrap_or_default();
            apps.sort_by_key(|a| a.to_lowercase());
            apps.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
            let current = settings
                .get("app")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !current.is_empty() && !apps.iter().any(|a| app_matches(current, a)) {
                apps.insert(0, current.to_owned());
            }
            let options = apps
                .into_iter()
                .map(|a| SelectOption::new(a.clone(), a))
                .collect();
            PropertySchema::new(vec![
                Property::select("app", "Application", options, "").hint(
                    "Apps appear here while they play sound; re-attaches whenever the app plays again",
                ),
            ])
        },
        |settings| {
            let mut s = AppAudioSource::new();
            s.update(settings)?;
            Ok(Box::new(s))
        },
    )
    .with_validation(|_| {
        PropertySchema::new(vec![Property::select("app", "Application", vec![], "")])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apps_match_across_systems() {
        assert!(app_matches("brave.exe", "brave"));
        assert!(app_matches("Firefox", "firefox"));
        assert!(!app_matches("", "firefox"));
        assert!(!app_matches("spotify", "firefox"));
    }
}
