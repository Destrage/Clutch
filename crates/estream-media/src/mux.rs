//! Muxing to files, the replay packet ring, and remuxing.

use crate::MediaError;
use crate::ffmpeg::{self, Dictionary, Rational as FfRational, codec};
use crate::settings::Container;
use std::collections::VecDeque;
use std::path::Path;

/// Description of one output stream.
#[derive(Clone)]
pub struct StreamSpec {
    pub parameters: codec::Parameters,
    /// Time base of the packets handed to [`Muxer::write`].
    pub time_base: FfRational,
    pub is_video: bool,
    /// Stream title written to the file (`title`, and `handler_name` for MP4/MOV).
    pub name: Option<String>,
    /// One source's own sound: never the default audio stream, and left out of a saved clip
    /// while that source made no sound.
    pub source_track: bool,
}

impl StreamSpec {
    pub fn new(parameters: codec::Parameters, time_base: FfRational, is_video: bool) -> Self {
        Self {
            parameters,
            time_base,
            is_video,
            name: None,
            source_track: false,
        }
    }

    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
}

/// Whether stream `index` of `specs` is a default stream: the video, and the first audio stream
/// that isn't a source track (players play that one).
fn is_default(specs: &[StreamSpec], index: usize) -> bool {
    let spec = &specs[index];
    spec.is_video
        || (!spec.source_track
            && specs
                .iter()
                .position(|s| !s.is_video && !s.source_track)
                .is_some_and(|first| first == index))
}

/// Writes a stream's name and default flag.
fn describe_stream(
    stream: &mut ffmpeg::format::stream::StreamMut<'_>,
    name: Option<&str>,
    default: bool,
) {
    if let Some(name) = name {
        let mut metadata = Dictionary::new();
        metadata.set("title", name);
        metadata.set("handler_name", name);
        stream.set_metadata(metadata);
    }
    // SAFETY: the stream belongs to an output context that is still being set up (before its
    // header is written), and `disposition` is a plain integer field.
    unsafe {
        let s = stream.as_mut_ptr();
        if default {
            (*s).disposition |= ffmpeg::ffi::AV_DISPOSITION_DEFAULT;
        } else {
            (*s).disposition &= !ffmpeg::ffi::AV_DISPOSITION_DEFAULT;
        }
    }
}

pub struct Muxer {
    output: ffmpeg::format::context::Output,
    specs: Vec<StreamSpec>,
    pub bytes: u64,
}

impl Muxer {
    pub fn create(
        path: &Path,
        container: Container,
        specs: &[StreamSpec],
    ) -> Result<Self, MediaError> {
        crate::init();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| MediaError::Io(e.to_string()))?;
        }
        let mut output = ffmpeg::format::output_as(path, container.format_name())?;
        for (i, spec) in specs.iter().enumerate() {
            let mut stream = output.add_stream(ffmpeg::encoder::find(spec.parameters.id()))?;
            stream.set_parameters(spec.parameters.clone());
            stream.set_time_base(spec.time_base);
            // Let the muxer choose the codec tag for this container.
            unsafe {
                (*stream.parameters().as_ptr().cast_mut()).codec_tag = 0;
            }
            describe_stream(&mut stream, spec.name.as_deref(), is_default(specs, i));
        }
        let mut options = Dictionary::new();
        if container == Container::FragmentedMp4 {
            options.set("movflags", "frag_keyframe+empty_moov+default_base_moof");
        }
        output.write_header_with(options)?;
        Ok(Self {
            output,
            specs: specs.to_vec(),
            bytes: 0,
        })
    }

    /// Writes a packet whose timestamps are in the stream's spec time base.
    pub fn write(&mut self, stream: usize, mut packet: ffmpeg::Packet) -> Result<(), MediaError> {
        let Some(spec) = self.specs.get(stream) else {
            return Ok(());
        };
        let target = self
            .output
            .stream(stream)
            .map(|s| s.time_base())
            .unwrap_or(spec.time_base);
        packet.rescale_ts(spec.time_base, target);
        packet.set_stream(stream);
        packet.set_position(-1);
        self.bytes += packet.size() as u64;
        packet.write_interleaved(&mut self.output)?;
        Ok(())
    }

    pub fn finish(mut self) -> Result<(), MediaError> {
        self.output.write_trailer()?;
        Ok(())
    }
}

/// Spans closer than this (seconds) are merged into one.
const HEARD_MERGE: f64 = 1.0;

/// Keeps the most recent `seconds` of packets, always starting at a video keyframe.
pub struct PacketRing {
    pub specs: Vec<StreamSpec>,
    seconds: f64,
    packets: VecDeque<(usize, ffmpeg::Packet, f64)>,
    /// Per stream: when a source track's source was heard (seconds on the ring's clock).
    heard: Vec<VecDeque<(f64, f64)>>,
    pub bytes: u64,
}

impl PacketRing {
    pub fn new(specs: Vec<StreamSpec>, seconds: f64) -> Self {
        Self {
            heard: specs.iter().map(|_| VecDeque::new()).collect(),
            specs,
            seconds,
            packets: VecDeque::new(),
            bytes: 0,
        }
    }

    /// Notes that stream `stream`'s source made sound from `from` to `to` (seconds).
    pub fn mark_heard(&mut self, stream: usize, from: f64, to: f64) {
        let Some(spans) = self.heard.get_mut(stream) else {
            return;
        };
        match spans.back_mut() {
            Some(last) if from <= last.1 + HEARD_MERGE => last.1 = last.1.max(to),
            _ => spans.push_back((from, to)),
        }
    }

    /// Whether stream `stream` belongs in a file covering `from..=to`: always, unless it is a
    /// source track whose source made no sound in that time.
    fn keeps(&self, stream: usize, from: f64, to: f64) -> bool {
        !self.specs[stream].source_track
            || self.heard[stream]
                .iter()
                .any(|&(a, b)| a <= to && b >= from)
    }

    fn time_of(&self, stream: usize, packet: &ffmpeg::Packet) -> f64 {
        let tb = self
            .specs
            .get(stream)
            .map_or(FfRational(1, 1), |s| s.time_base);
        packet.pts().or(packet.dts()).unwrap_or(0) as f64 * tb.0 as f64 / tb.1.max(1) as f64
    }

    pub fn push(&mut self, stream: usize, packet: ffmpeg::Packet) {
        let t = self.time_of(stream, &packet);
        self.bytes += packet.size() as u64;
        self.packets.push_back((stream, packet, t));
        // Drop whole GOPs from the front while the rest still covers the window.
        loop {
            let newest = self.packets.back().map_or(0.0, |p| p.2);
            let next_key = self
                .packets
                .iter()
                .enumerate()
                .skip(1)
                .find(|(_, (s, p, _))| self.specs[*s].is_video && p.is_key())
                .map(|(i, (_, _, t))| (i, *t));
            match next_key {
                Some((i, t)) if newest - t >= self.seconds => {
                    for (_, p, _) in self.packets.drain(..i) {
                        self.bytes -= p.size() as u64;
                    }
                    for spans in &mut self.heard {
                        while spans.front().is_some_and(|s| s.1 < t) {
                            spans.pop_front();
                        }
                    }
                }
                _ => break,
            }
        }
    }

    /// Duration currently buffered, in seconds.
    pub fn duration(&self) -> f64 {
        match (self.packets.front(), self.packets.back()) {
            (Some(a), Some(b)) => b.2 - a.2,
            _ => 0.0,
        }
    }

    /// Writes all buffered packets (from the first video keyframe) to a new file.
    pub fn save(&self, path: &Path, container: Container) -> Result<(), MediaError> {
        self.save_range(f64::NEG_INFINITY, f64::INFINITY, path, container)
            .map(|_| ())
    }

    /// Writes the packets between `from` and `to` (seconds on the ring's clock) to a new file,
    /// starting at the last video keyframe at or before `from` (or the first one buffered).
    /// Returns the span actually written, on the ring's clock.
    pub fn save_range(
        &self,
        from: f64,
        to: f64,
        path: &Path,
        container: Container,
    ) -> Result<(f64, f64), MediaError> {
        let keys: Vec<usize> = (0..self.packets.len())
            .filter(|&i| {
                let (s, p, _) = &self.packets[i];
                self.specs[*s].is_video && p.is_key()
            })
            .collect();
        let start = keys
            .iter()
            .take_while(|&&i| self.packets[i].2 <= from)
            .last()
            .or(keys.first())
            .copied()
            .ok_or_else(|| MediaError::Io("the replay buffer has no video yet".into()))?;
        let t0 = self.packets[start].2;
        if to < t0 {
            return Err(MediaError::Io("that moment is no longer buffered".into()));
        }
        // Ring stream → file stream, leaving out source tracks that were silent throughout.
        let mut map = vec![None; self.specs.len()];
        let mut specs = Vec::new();
        for (i, spec) in self.specs.iter().enumerate() {
            if self.keeps(i, t0, to) {
                map[i] = Some(specs.len());
                specs.push(spec.clone());
            }
        }
        let mut muxer = Muxer::create(path, container, &specs)?;
        let mut end = t0;
        let mut video_done = false;
        for (stream, packet, t) in self.packets.iter().skip(start) {
            let Some(out) = map[*stream] else {
                continue;
            };
            let video = self.specs[*stream].is_video;
            if video && *t > to {
                // Packets are in decode order: stop video at the first one past the end, so
                // no later frame refers to one that was left out.
                video_done = true;
            }
            if *t < t0 || *t > to || (video && video_done) {
                continue; // audio from before the keyframe, or past the end
            }
            if video {
                end = end.max(*t);
            }
            let tb = self.specs[*stream].time_base;
            let offset = (t0 * tb.1 as f64 / tb.0.max(1) as f64).round() as i64;
            let mut p = packet.clone();
            p.set_pts(p.pts().map(|v| v - offset));
            p.set_dts(p.dts().map(|v| v - offset));
            muxer.write(out, p)?;
        }
        muxer.finish()?;
        Ok((t0, end))
    }
}

/// Copies all streams of `input` into `output` (e.g. MKV → MP4) without re-encoding.
pub fn remux(input: &Path, output: &Path, container: Container) -> Result<(), MediaError> {
    remux_without(input, output, container, &[])
}

/// A stream's title (`title`, or MP4's `handler_name` when it isn't a generic handler name).
pub fn stream_name(stream: &ffmpeg::format::stream::Stream<'_>) -> Option<String> {
    let metadata = stream.metadata();
    metadata
        .get("title")
        .or_else(|| {
            metadata
                .get("handler_name")
                .filter(|h| !matches!(*h, "SoundHandler" | "VideoHandler" | ""))
        })
        .map(str::to_owned)
}

/// Whether `stream` is marked as a default stream.
pub fn stream_is_default(stream: &ffmpeg::format::stream::Stream<'_>) -> bool {
    stream
        .disposition()
        .contains(ffmpeg::format::stream::Disposition::DEFAULT)
}

/// Like [`remux`], leaving out the input streams whose indexes are in `leave_out`. Stream
/// names and default flags are kept.
pub fn remux_without(
    input: &Path,
    output: &Path,
    container: Container,
    leave_out: &[usize],
) -> Result<(), MediaError> {
    crate::init();
    let mut ictx = ffmpeg::format::input(input)?;
    let mut octx = ffmpeg::format::output_as(output, container.format_name())?;
    let mut map = Vec::new();
    for stream in ictx.streams() {
        let medium = stream.parameters().medium();
        if !matches!(
            medium,
            ffmpeg::media::Type::Video | ffmpeg::media::Type::Audio
        ) || leave_out.contains(&stream.index())
        {
            map.push(None);
            continue;
        }
        let mut out = octx.add_stream(ffmpeg::encoder::find(codec::Id::None))?;
        out.set_parameters(stream.parameters());
        unsafe {
            (*out.parameters().as_ptr().cast_mut()).codec_tag = 0;
        }
        describe_stream(
            &mut out,
            stream_name(&stream).as_deref(),
            stream_is_default(&stream),
        );
        map.push(Some((out.index(), stream.time_base())));
    }
    octx.write_header()?;
    for (stream, mut packet) in ictx.packets() {
        let Some(Some((index, tb))) = map.get(stream.index()).cloned() else {
            continue;
        };
        let target = octx.stream(index).map(|s| s.time_base()).unwrap_or(tb);
        packet.rescale_ts(tb, target);
        packet.set_stream(index);
        packet.set_position(-1);
        packet.write_interleaved(&mut octx)?;
    }
    octx.write_trailer()?;
    Ok(())
}
