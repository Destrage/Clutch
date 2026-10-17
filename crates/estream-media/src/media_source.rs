//! The `media` source type: video, audio and image files, alone or as a playlist.
//! Its video and audio sides share one [`Player`] (found by the engine-provided `_source_id`).

use crate::player::{self, AUDIO_LEAD, PixelLayout, PlayState, Player, PlayerSettings};
use estream_audio::{AudioSource, BLOCK, Block};
use estream_core::{Size, SourceStatus, Timestamp};
use estream_engine::gpu::{Nv12Frame, YuvConverter, YuvMatrix, wgpu};
use estream_engine::{
    PropKind, Property, PropertySchema, SourceContext, SourceError, SourceFrame, SourceRegistry,
    SourceType, VideoSource,
};
use serde_json::Value;
use std::sync::Arc;

/// Registers the media source type.
pub fn register(registry: &mut SourceRegistry) {
    registry.register(source_type());
}

fn extensions() -> Vec<String> {
    player::VIDEO_EXTENSIONS
        .iter()
        .chain(player::AUDIO_EXTENSIONS)
        .chain(player::IMAGE_EXTENSIONS)
        .map(|e| (*e).to_owned())
        .collect()
}

pub fn schema() -> PropertySchema {
    PropertySchema::new(vec![
        Property::new(
            "file",
            "File",
            PropKind::Path {
                filters: extensions(),
            },
            "",
        ),
        Property::text("playlist", "More files", true, "")
            .hint("Files or folders, one per line; they play after the file above"),
        Property::bool("loop", "Loop", true),
        Property::bool("shuffle", "Shuffle", false),
        Property::float("image_seconds", "Show images for", 0.5, 3600.0, 0.5, 5.0)
            .hint("Seconds per still image (slideshows)"),
    ])
}

pub fn source_type() -> SourceType {
    SourceType::new(
        "media",
        "Media",
        "Video, audio and image files: one file, a playlist or a slideshow",
        |_| schema(),
        |ctx, settings| Ok(Box::new(MediaVideo::new(ctx, settings))),
    )
    .with_audio(|settings| Ok(Box::new(MediaAudio::new(settings))))
}

fn player_for(settings: &Value) -> Arc<Player> {
    let config = PlayerSettings::from_json(settings);
    match settings.get("_source_id").and_then(|v| v.as_str()) {
        Some(id) => player::acquire(id, config),
        // Without an id (e.g. a standalone instance) the player isn't shared.
        None => Player::new(config),
    }
}

fn status_of(player: &Player) -> SourceStatus {
    let s = player.status();
    match (&s.state, s.error) {
        (PlayState::Idle, Some(e)) => SourceStatus::Error(e),
        (PlayState::Idle, None) if s.count == 0 => {
            SourceStatus::Unavailable("Choose a file".into())
        }
        _ => SourceStatus::Active,
    }
}

// ---------------------------------------------------------------- video side

struct RgbaTexture {
    texture: wgpu::Texture,
    binding: wgpu::BindGroup,
}

pub struct MediaVideo {
    player: Arc<Player>,
    converter: Arc<YuvConverter>,
    nv12: Option<(Nv12Frame, wgpu::BindGroup)>,
    rgba: Option<RgbaTexture>,
    /// Which texture holds the current picture.
    current: Option<PixelLayout>,
    size: Size,
    sequence: u64,
}

impl MediaVideo {
    fn new(ctx: &SourceContext<'_>, settings: &Value) -> Self {
        Self {
            player: player_for(settings),
            converter: YuvConverter::new(ctx.gpu),
            nv12: None,
            rgba: None,
            current: None,
            size: Size::default(),
            sequence: 0,
        }
    }
}

impl VideoSource for MediaVideo {
    fn update(&mut self, _: &SourceContext<'_>, settings: &Value) -> Result<(), SourceError> {
        self.player.update(PlayerSettings::from_json(settings));
        Ok(())
    }

    fn tick(&mut self, ctx: &SourceContext<'_>, encoder: &mut wgpu::CommandEncoder, _: Timestamp) {
        let sequence = self.player.sequence();
        if sequence == self.sequence {
            return;
        }
        // Never wait for the decoder: try again next frame if it holds the picture.
        let Some(guard) = self.player.try_picture() else {
            return;
        };
        let Some(picture) = guard.as_ref() else {
            self.current = None;
            self.sequence = sequence;
            return;
        };
        self.sequence = picture.sequence;
        let (w, h) = (picture.width, picture.height);
        match picture.layout {
            PixelLayout::Nv12 => {
                if self
                    .nv12
                    .as_ref()
                    .is_none_or(|(f, _)| f.width != w || f.height != h)
                {
                    let frame = self.converter.create_frame(ctx.gpu, w, h);
                    let binding = ctx.compositor.create_source_binding(
                        ctx.gpu,
                        &frame.output.create_view(&Default::default()),
                    );
                    self.nv12 = Some((frame, binding));
                }
                if let Some((frame, _)) = &self.nv12 {
                    frame.upload(
                        ctx.gpu,
                        &picture.data,
                        w,
                        YuvMatrix::for_height(h),
                        picture.full_range,
                    );
                    frame.convert(&self.converter, encoder);
                }
            }
            PixelLayout::Rgba => {
                if self
                    .rgba
                    .as_ref()
                    .is_none_or(|t| t.texture.width() != w || t.texture.height() != h)
                {
                    let texture = ctx.gpu.device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("media rgba"),
                        size: wgpu::Extent3d {
                            width: w,
                            height: h,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    });
                    let binding = ctx
                        .compositor
                        .create_source_binding(ctx.gpu, &texture.create_view(&Default::default()));
                    self.rgba = Some(RgbaTexture { texture, binding });
                }
                if let Some(t) = &self.rgba {
                    ctx.gpu.queue.write_texture(
                        t.texture.as_image_copy(),
                        &picture.data,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(w * 4),
                            rows_per_image: Some(h),
                        },
                        t.texture.size(),
                    );
                }
            }
        }
        self.current = Some(picture.layout);
        self.size = Size::new(w as f32, h as f32);
    }

    fn frame(&self) -> Option<SourceFrame<'_>> {
        let binding = match self.current? {
            PixelLayout::Nv12 => &self.nv12.as_ref()?.1,
            PixelLayout::Rgba => &self.rgba.as_ref()?.binding,
        };
        Some(SourceFrame {
            binding,
            size: self.size,
        })
    }

    fn status(&self) -> SourceStatus {
        status_of(&self.player)
    }
}

// ---------------------------------------------------------------- audio side

pub struct MediaAudio {
    player: Arc<Player>,
    /// Waiting for `AUDIO_LEAD` of audio before playing (keeps audio on the video's clock).
    priming: bool,
}

impl MediaAudio {
    fn new(settings: &Value) -> Self {
        Self {
            player: player_for(settings),
            priming: true,
        }
    }
}

impl AudioSource for MediaAudio {
    fn update(&mut self, settings: &Value) -> Result<(), String> {
        self.player.update(PlayerSettings::from_json(settings));
        Ok(())
    }

    fn read(&mut self, block: &mut Block, _: Timestamp) -> usize {
        let lead = (AUDIO_LEAD.as_secs_f64() * 48_000.0) as usize;
        if self.priming {
            if self.player.buffered_audio() < lead {
                return 0;
            }
            self.priming = false;
        }
        let n = self.player.read_audio(&mut block.l, &mut block.r);
        if n < BLOCK {
            self.priming = true; // underrun or pause: build up the lead again
        }
        n
    }

    fn status(&self) -> SourceStatus {
        status_of(&self.player)
    }
}
