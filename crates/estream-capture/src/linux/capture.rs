//! Display and window capture through the ScreenCast portal. The system's picker chooses
//! what is shared; "Choose again…" reopens it, and a saved token skips it at later starts.

use super::frames::Frame;
use super::screencast::{Session, Target};
use crate::media::upload::CpuTexture;
use estream_core::Timestamp;
use estream_engine::source::parse;
use estream_engine::{
    Property, PropertySchema, SourceContext, SourceError, SourceFrame, SourceStatus, SourceType,
    VideoSource,
};
use estream_gpu::wgpu;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct CaptureSettings {
    cursor: bool,
    /// Presses of "Choose again…": a change forgets the saved choice.
    pick: i64,
}

impl Default for CaptureSettings {
    fn default() -> Self {
        Self {
            cursor: true,
            pick: 0,
        }
    }
}

pub struct PortalCapture {
    target: Target,
    settings: CaptureSettings,
    session: Option<Session>,
    /// Start a session at the next tick (`Some(true)`: forgetting the saved choice).
    start: Option<bool>,
    frame: Frame,
    last: u64,
    texture: CpuTexture,
}

impl PortalCapture {
    fn new(target: Target) -> Self {
        Self {
            target,
            settings: CaptureSettings::default(),
            session: None,
            start: Some(false),
            frame: Frame::default(),
            last: 0,
            texture: CpuTexture::default(),
        }
    }
}

impl VideoSource for PortalCapture {
    fn update(&mut self, _: &SourceContext<'_>, settings: &Value) -> Result<(), SourceError> {
        let new: CaptureSettings = parse(settings)?;
        if new.pick != self.settings.pick {
            self.start = Some(true);
        } else if new.cursor != self.settings.cursor && self.start.is_none() {
            // The cursor mode is fixed per session; the saved choice skips the picker.
            self.start = Some(false);
        }
        self.settings = new;
        Ok(())
    }

    fn tick(&mut self, ctx: &SourceContext<'_>, _: &mut wgpu::CommandEncoder, _: Timestamp) {
        if let Some(forget) = self.start.take() {
            // Dropping the old session ends its share (its thread winds down on its own).
            self.session = None;
            self.texture.clear();
            self.session = Some(Session::start(
                ctx.source,
                self.target,
                self.settings.cursor,
                forget,
            ));
        }
        let Some(session) = &self.session else {
            return;
        };
        if session.shared.frames.take(&mut self.last, &mut self.frame) {
            let f = &self.frame;
            self.texture
                .upload_raw(ctx, f.width, f.height, f.format, &f.data, f.width * 4);
        } else if !matches!(session.shared.status(), SourceStatus::Active) {
            self.texture.clear();
        }
    }

    fn frame(&self) -> Option<SourceFrame<'_>> {
        self.texture.frame()
    }

    fn status(&self) -> SourceStatus {
        self.session.as_ref().map_or_else(
            || SourceStatus::Unavailable("Starting…".into()),
            |s| s.shared.status(),
        )
    }
}

fn schema(what: &str) -> PropertySchema {
    PropertySchema::new(vec![
        Property::button("pick", what, "Choose again…").hint(&format!(
            "Your system asks which {} to share; this opens its picker again",
            what.to_lowercase()
        )),
        Property::bool("cursor", "Capture cursor", true),
    ])
}

pub fn source_types() -> Vec<SourceType> {
    let display = SourceType::new(
        "display_capture",
        "Display capture",
        "Capture an entire monitor",
        |_| schema("Display"),
        |ctx, settings| {
            let mut s = PortalCapture::new(Target::Display);
            s.update(ctx, settings)?;
            Ok(Box::new(s))
        },
    );
    let window = SourceType::new(
        "window_capture",
        "Window capture",
        "Capture a single application window",
        |_| schema("Window"),
        |ctx, settings| {
            let mut s = PortalCapture::new(Target::Window);
            s.update(ctx, settings)?;
            Ok(Box::new(s))
        },
    );
    vec![display, window]
}
