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
        for spec in specs {
            let mut stream = output.add_stream(ffmpeg::encoder::find(spec.parameters.id()))?;
            stream.set_parameters(spec.parameters.clone());
            stream.set_time_base(spec.time_base);
            // Let the muxer choose the codec tag for this container.
            unsafe {
                (*stream.parameters().as_ptr().cast_mut()).codec_tag = 0;
            }
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

/// Keeps the most recent `seconds` of packets, always starting at a video keyframe.
pub struct PacketRing {
    pub specs: Vec<StreamSpec>,
    seconds: f64,
    packets: VecDeque<(usize, ffmpeg::Packet, f64)>,
    pub bytes: u64,
}

impl PacketRing {
    pub fn new(specs: Vec<StreamSpec>, seconds: f64) -> Self {
        Self {
            specs,
            seconds,
            packets: VecDeque::new(),
            bytes: 0,
        }
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
        let mut muxer = Muxer::create(path, container, &self.specs)?;
        let mut end = t0;
        let mut video_done = false;
        for (stream, packet, t) in self.packets.iter().skip(start) {
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
            muxer.write(*stream, p)?;
        }
        muxer.finish()?;
        Ok((t0, end))
    }
}

/// Copies all streams of `input` into `output` (e.g. MKV → MP4) without re-encoding.
pub fn remux(input: &Path, output: &Path, container: Container) -> Result<(), MediaError> {
    crate::init();
    let mut ictx = ffmpeg::format::input(input)?;
    let mut octx = ffmpeg::format::output_as(output, container.format_name())?;
    let mut map = Vec::new();
    for stream in ictx.streams() {
        let medium = stream.parameters().medium();
        if !matches!(
            medium,
            ffmpeg::media::Type::Video | ffmpeg::media::Type::Audio
        ) {
            map.push(None);
            continue;
        }
        let mut out = octx.add_stream(ffmpeg::encoder::find(codec::Id::None))?;
        out.set_parameters(stream.parameters());
        unsafe {
            (*out.parameters().as_ptr().cast_mut()).codec_tag = 0;
        }
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
