//! Image source: PNG, JPEG, WebP, BMP and animated GIF / WebP.
//! Decoding happens on a worker thread; files are reloaded when they change.

use super::upload::{CpuTexture, Pixels};
use estream_core::Timestamp;
use estream_engine::source::parse;
use estream_engine::{
    PropKind, Property, PropertySchema, SourceContext, SourceError, SourceFrame, SourceStatus,
    SourceType, VideoSource,
};
use estream_gpu::wgpu;
use image::AnimationDecoder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, SystemTime};

pub const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp"];

/// A decoded image: one frame, or an animation with per-frame delays.
#[derive(Clone, Debug, Default)]
pub struct Decoded {
    pub frames: Vec<(Pixels, Duration)>,
}

impl Decoded {
    pub fn total(&self) -> Duration {
        self.frames.iter().map(|(_, d)| *d).sum()
    }

    /// Frame index to show `elapsed` after the animation started (loops).
    pub fn index_at(&self, elapsed: Duration) -> usize {
        let total = self.total();
        if self.frames.len() <= 1 || total.is_zero() {
            return 0;
        }
        let mut t = Duration::from_nanos((elapsed.as_nanos() % total.as_nanos()) as u64);
        for (i, (_, delay)) in self.frames.iter().enumerate() {
            if t < *delay {
                return i;
            }
            t -= *delay;
        }
        self.frames.len() - 1
    }
}

fn to_pixels(img: image::RgbaImage) -> Pixels {
    Pixels {
        width: img.width(),
        height: img.height(),
        rgba: img.into_raw(),
    }
}

fn frames_of<'a>(decoder: impl AnimationDecoder<'a>) -> Result<Decoded, String> {
    let frames = decoder
        .into_frames()
        .collect_frames()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|f| {
            let (num, den) = f.delay().numer_denom_ms();
            // Browsers treat delays under 20 ms as 100 ms; do the same.
            let ms = num.checked_div(den).unwrap_or(100);
            let ms = if ms < 20 { 100 } else { ms };
            (to_pixels(f.into_buffer()), Duration::from_millis(ms as u64))
        })
        .collect::<Vec<_>>();
    if frames.is_empty() {
        return Err("the animation has no frames".into());
    }
    Ok(Decoded { frames })
}

/// Decodes an image file (all frames for animated GIF/WebP).
pub fn decode(path: &Path) -> Result<Decoded, String> {
    use std::io::BufReader;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let open = || {
        std::fs::File::open(path)
            .map(BufReader::new)
            .map_err(|e| e.to_string())
    };
    match ext.as_str() {
        "gif" => {
            frames_of(image::codecs::gif::GifDecoder::new(open()?).map_err(|e| e.to_string())?)
        }
        "webp" => {
            let decoder =
                image::codecs::webp::WebPDecoder::new(open()?).map_err(|e| e.to_string())?;
            if decoder.has_animation() {
                frames_of(decoder)
            } else {
                let img = image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
                Ok(Decoded {
                    frames: vec![(to_pixels(img.to_rgba8()), Duration::ZERO)],
                })
            }
        }
        _ => {
            let img = image::ImageReader::new(open()?)
                .with_guessed_format()
                .map_err(|e| e.to_string())?
                .decode()
                .map_err(|e| e.to_string())?;
            Ok(Decoded {
                frames: vec![(to_pixels(img.to_rgba8()), Duration::ZERO)],
            })
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageSettings {
    pub path: String,
}

/// Loads `path` on a worker thread, then watches it for changes.
fn spawn_loader(path: PathBuf, stop: Arc<AtomicBool>) -> Receiver<Result<Decoded, String>> {
    let (tx, rx) = channel();
    let _ = std::thread::Builder::new()
        .name("estream-image".into())
        .spawn(move || {
            let mut last: Option<SystemTime> = None;
            while !stop.load(Ordering::Acquire) {
                let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                if modified.is_none() && last.is_none() {
                    let _ = tx.send(Err(format!("File not found: {}", path.display())));
                    last = Some(SystemTime::UNIX_EPOCH);
                } else if modified.is_some() && modified != last {
                    last = modified;
                    if tx.send(decode(&path)).is_err() {
                        return;
                    }
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        });
    rx
}

/// Results channel and stop flag of a running loader thread.
type Loader = (Receiver<Result<Decoded, String>>, Arc<AtomicBool>);

pub struct ImageSource {
    settings: ImageSettings,
    loader: Option<Loader>,
    decoded: Decoded,
    shown: Option<usize>,
    started: Option<Timestamp>,
    texture: CpuTexture,
    status: SourceStatus,
}

impl ImageSource {
    fn new() -> Self {
        Self {
            settings: ImageSettings::default(),
            loader: None,
            decoded: Decoded::default(),
            shown: None,
            started: None,
            texture: CpuTexture::default(),
            status: SourceStatus::Unavailable("Choose an image file".into()),
        }
    }

    fn stop_loader(&mut self) {
        if let Some((_, stop)) = self.loader.take() {
            stop.store(true, Ordering::Release);
        }
    }
}

impl Drop for ImageSource {
    fn drop(&mut self) {
        self.stop_loader();
    }
}

impl VideoSource for ImageSource {
    fn update(&mut self, _: &SourceContext<'_>, settings: &Value) -> Result<(), SourceError> {
        let new: ImageSettings = parse(settings)?;
        if new != self.settings || self.loader.is_none() {
            self.stop_loader();
            self.settings = new;
            if self.settings.path.is_empty() {
                self.status = SourceStatus::Unavailable("Choose an image file".into());
            } else {
                let stop = Arc::new(AtomicBool::new(false));
                let rx = spawn_loader(PathBuf::from(&self.settings.path), stop.clone());
                self.loader = Some((rx, stop));
                self.status = SourceStatus::Unavailable("Loading…".into());
            }
        }
        Ok(())
    }

    fn tick(&mut self, ctx: &SourceContext<'_>, _: &mut wgpu::CommandEncoder, now: Timestamp) {
        if let Some((rx, _)) = &self.loader {
            match rx.try_recv() {
                Ok(Ok(decoded)) => {
                    self.decoded = decoded;
                    self.shown = None;
                    self.started = Some(now);
                    self.status = SourceStatus::Active;
                }
                Ok(Err(e)) => {
                    self.decoded = Decoded::default();
                    self.texture.clear();
                    self.status = SourceStatus::Unavailable(e);
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => {}
            }
        }
        if self.decoded.frames.is_empty() {
            return;
        }
        let elapsed = now.saturating_sub(self.started.unwrap_or(now));
        let index = self.decoded.index_at(elapsed);
        if self.shown != Some(index) {
            self.texture.upload(ctx, &self.decoded.frames[index].0);
            self.shown = Some(index);
        }
    }

    fn frame(&self) -> Option<SourceFrame<'_>> {
        self.texture.frame()
    }

    fn status(&self) -> SourceStatus {
        self.status.clone()
    }
}

pub fn source_type() -> SourceType {
    SourceType::new(
        "image",
        "Image",
        "PNG, JPEG, WebP and animated GIF",
        |_| {
            PropertySchema::new(vec![
                Property::new(
                    "path",
                    "Image file",
                    PropKind::Path {
                        filters: EXTENSIONS.iter().map(|s| s.to_string()).collect(),
                    },
                    "",
                )
                .hint("Reloads automatically when the file changes"),
            ])
        },
        |ctx, settings| {
            let mut s = ImageSource::new();
            s.update(ctx, settings)?;
            Ok(Box::new(s))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("estream-img-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn decodes_png_and_animated_gif() {
        let png = temp("a.png");
        image::RgbaImage::from_pixel(3, 2, image::Rgba([10, 20, 30, 128]))
            .save(&png)
            .unwrap();
        let d = decode(&png).unwrap();
        assert_eq!(
            (d.frames.len(), d.frames[0].0.width, d.frames[0].0.height),
            (1, 3, 2)
        );
        assert_eq!(&d.frames[0].0.rgba[..4], &[10, 20, 30, 128]);

        let gif = temp("a.gif");
        {
            let file = std::fs::File::create(&gif).unwrap();
            let mut enc = image::codecs::gif::GifEncoder::new(file);
            for c in [255u8, 0] {
                let frame = image::Frame::from_parts(
                    image::RgbaImage::from_pixel(2, 2, image::Rgba([c, 0, 0, 255])),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(50, 1),
                );
                enc.encode_frame(frame).unwrap();
            }
        }
        let d = decode(&gif).unwrap();
        assert_eq!(d.frames.len(), 2);
        assert_eq!(d.total(), Duration::from_millis(100));
        assert_eq!(d.index_at(Duration::from_millis(10)), 0);
        assert_eq!(d.index_at(Duration::from_millis(60)), 1);
        assert_eq!(d.index_at(Duration::from_millis(110)), 0, "loops");
    }

    #[test]
    fn missing_file_is_an_error() {
        assert!(decode(Path::new("does/not/exist.png")).is_err());
    }
}
