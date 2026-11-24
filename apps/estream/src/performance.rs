//! Performance profiles: the settings that decide what recording, the replay buffer and
//! streaming cost a game, as named sets, and a recommendation for this PC.

use crate::settings::Settings;
use estream_media::{Preset, RateControl, VideoCodec, encode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Gaming,
    Balanced,
    Quality,
}

impl Profile {
    pub const ALL: [Profile; 3] = [Profile::Gaming, Profile::Balanced, Profile::Quality];

    pub fn label(self) -> &'static str {
        match self {
            Profile::Gaming => "Gaming",
            Profile::Balanced => "Balanced",
            Profile::Quality => "Quality",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }

    /// The next profile that costs the game less.
    pub fn lighter(self) -> Option<Profile> {
        match self {
            Profile::Gaming => None,
            Profile::Balanced => Some(Profile::Gaming),
            Profile::Quality => Some(Profile::Balanced),
        }
    }
}

/// What Clutch could find out about this PC.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hardware {
    /// The GPU's name, as the renderer reports it.
    pub gpu: String,
    pub cores: usize,
    pub memory_bytes: Option<u64>,
    /// The hardware H.264 encoder that opens, e.g. "NVIDIA NVENC H.264".
    pub hw_h264: Option<String>,
    pub hw_hevc: bool,
    pub hw_av1: bool,
}

impl Hardware {
    /// Probes the encoders (opening each once; cached afterwards) and the machine.
    pub fn probe(gpu: &str) -> Self {
        let hardware = |codec| encode::available(codec).into_iter().find(|e| e.hardware);
        Self {
            gpu: gpu.to_owned(),
            cores: std::thread::available_parallelism().map_or(1, |n| n.get()),
            memory_bytes: estream_health::system::total_memory(),
            hw_h264: hardware(VideoCodec::H264).map(|e| e.label.to_owned()),
            hw_hevc: hardware(VideoCodec::Hevc).is_some(),
            hw_av1: hardware(VideoCodec::Av1).is_some(),
        }
    }

    fn memory_gb(&self) -> Option<f64> {
        self.memory_bytes
            .map(|b| b as f64 / (1024.0 * 1024.0 * 1024.0))
    }

    /// One line for the Performance page, e.g. "NVIDIA … · NVENC (H.264, HEVC, AV1) · 16
    /// cores · 32 GB".
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.gpu.is_empty() {
            parts.push(self.gpu.clone());
        }
        match &self.hw_h264 {
            Some(name) => {
                let family = name.trim_end_matches(" H.264");
                let mut codecs = vec!["H.264"];
                if self.hw_hevc {
                    codecs.push("HEVC");
                }
                if self.hw_av1 {
                    codecs.push("AV1");
                }
                parts.push(format!("{family} ({})", codecs.join(", ")));
            }
            None => parts.push("no hardware encoder".to_owned()),
        }
        parts.push(format!("{} cores", self.cores));
        if let Some(gb) = self.memory_gb() {
            parts.push(format!("{} GB memory", gb.round() as u64));
        }
        parts.join(" · ")
    }

    /// Why the recommendation is what it is, when the PC holds it back.
    pub fn warning(&self) -> Option<String> {
        if self.hw_h264.is_none() {
            return Some(
                "No hardware encoder was found, so encoding runs on the CPU and games will lose \
                 frames while you record or stream. Updating your graphics driver may help."
                    .to_owned(),
            );
        }
        if self.memory_gb().is_some_and(|gb| gb < 8.0) {
            return Some("This PC has little memory, so the clip buffer is kept short.".to_owned());
        }
        None
    }
}

/// The profile Optimize picks. Quality is never picked automatically: it trades encoder
/// headroom for picture quality, and the point of optimizing is to leave the game alone.
pub fn recommend(hw: &Hardware) -> Profile {
    let weak = hw.hw_h264.is_none() || hw.cores < 4 || hw.memory_gb().is_some_and(|gb| gb < 8.0);
    if weak {
        Profile::Gaming
    } else {
        Profile::Balanced
    }
}

/// Upload kept free for the game, voice chat and swings in the connection (kb/s).
const UPLOAD_RESERVE_KBPS: f64 = 1_500.0;
/// Share of the measured upload the streams may use.
const UPLOAD_SHARE: f64 = 0.75;
/// No stream bitrate is set below this.
pub const FLOOR_KBPS: u32 = 1_500;

/// What a stream destination needs, for [`budget`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreamNeed {
    pub service: estream_media::Service,
    pub enabled: bool,
    pub current_kbps: u32,
    pub audio_kbps: u32,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}

/// What set a destination's bitrate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    /// Its share of the measured upload.
    Upload,
    /// The platform's ceiling for the canvas.
    Platform,
    /// A custom server keeps its bitrate when the upload has room for it.
    Kept,
}

/// A destination's bitrate from the upload.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BitrateFit {
    pub kbps: u32,
    pub limit: Limit,
    /// The share is below what the canvas needs for a clear picture on the platform.
    pub short: bool,
}

/// Fits each enabled destination's video bitrate into the measured upload: the lower of 75%
/// of it and all but 1.5 Mb/s, split evenly, less each destination's audio; never above the
/// platform's ceiling or below [`FLOOR_KBPS`]. A custom server is only ever lowered.
/// Disabled destinations get `None` (they keep their bitrate).
pub fn budget(upload_kbps: f64, needs: &[StreamNeed]) -> Vec<Option<BitrateFit>> {
    let usable = (upload_kbps * UPLOAD_SHARE).min(upload_kbps - UPLOAD_RESERVE_KBPS);
    let enabled = needs.iter().filter(|n| n.enabled).count().max(1);
    let share = usable / enabled as f64;
    needs
        .iter()
        .map(|n| {
            if !n.enabled {
                return None;
            }
            let room = (share - n.audio_kbps as f64).max(0.0) as u32;
            let (ceiling, at_ceiling) = match n.service.ceiling_kbps(n.width, n.height, n.fps) {
                Some(c) => (c, Limit::Platform),
                None => (n.current_kbps, Limit::Kept),
            };
            let (kbps, limit) = if room >= ceiling {
                (ceiling, at_ceiling)
            } else {
                (room.max(FLOOR_KBPS), Limit::Upload)
            };
            let short = room < n.service.minimum_kbps(n.width, n.height, n.fps);
            Some(BitrateFit { kbps, limit, short })
        })
        .collect()
}

/// A video bitrate that fits what a struggling stream actually got through: 85% of the rate
/// sent, less audio, rounded down to 100 kb/s. `None` when that isn't below the current one.
pub fn fit_to_sent(sent_kbps: f64, current_kbps: u32, audio_kbps: u32) -> Option<u32> {
    let kbps = ((sent_kbps.max(0.0) * 0.85) as u32).saturating_sub(audio_kbps) / 100 * 100;
    let kbps = kbps.max(FLOOR_KBPS);
    (kbps < current_kbps).then_some(kbps)
}

/// The settings a profile covers, as they are now or as a profile would set them.
#[derive(Clone, Debug, PartialEq)]
pub struct Covered {
    pub rec_codec: VideoCodec,
    pub rec_encoder: String,
    pub rec_rate: RateControl,
    pub rec_quality: u32,
    pub rec_preset: Preset,
    pub clip_buffer: u32,
    /// Each destination's encoder, preset and bitrate.
    pub streams: Vec<(String, Preset, u32)>,
}

impl Covered {
    pub fn read(s: &Settings) -> Self {
        let r = &s.recording;
        Self {
            rec_codec: r.codec,
            rec_encoder: r.encoder.clone(),
            rec_rate: r.rate_control,
            rec_quality: r.quality,
            rec_preset: r.preset,
            clip_buffer: s.clips.buffer_seconds,
            streams: s
                .destinations
                .iter()
                .map(|d| {
                    (
                        d.stream.encoder.clone(),
                        d.stream.preset,
                        d.stream.bitrate_kbps,
                    )
                })
                .collect(),
        }
    }

    pub fn write(&self, s: &mut Settings) {
        let r = &mut s.recording;
        r.codec = self.rec_codec;
        r.encoder = self.rec_encoder.clone();
        r.rate_control = self.rec_rate;
        r.quality = self.rec_quality;
        r.preset = self.rec_preset;
        s.clips.buffer_seconds = self.clip_buffer;
        for (d, (encoder, preset, kbps)) in s.destinations.iter_mut().zip(&self.streams) {
            d.stream.encoder = encoder.clone();
            d.stream.preset = *preset;
            d.stream.bitrate_kbps = *kbps;
        }
    }

    /// `profile` applied over `self`. `bitrates` holds each destination's recommended bitrate
    /// (`None` keeps the current one, e.g. for a custom server).
    pub fn with_profile(&self, profile: Profile, hw: &Hardware, bitrates: &[Option<u32>]) -> Self {
        let (quality, preset, buffer) = match profile {
            Profile::Gaming => (23, Preset::Performance, 60),
            Profile::Balanced => (20, Preset::Balanced, 120),
            Profile::Quality => (18, Preset::Quality, 120),
        };
        // HEVC records smaller files at the same quality, when the GPU encodes it.
        let codec = if profile == Profile::Quality && hw.hw_hevc {
            VideoCodec::Hevc
        } else {
            VideoCodec::H264
        };
        let small_memory = hw.memory_gb().is_some_and(|gb| gb < 8.0);
        Self {
            rec_codec: codec,
            rec_encoder: "auto".to_owned(),
            rec_rate: RateControl::ConstantQuality,
            rec_quality: quality,
            rec_preset: preset,
            clip_buffer: if small_memory { 60 } else { buffer },
            streams: self
                .streams
                .iter()
                .enumerate()
                .map(|(i, (_, _, kbps))| {
                    let kbps = bitrates.get(i).copied().flatten().unwrap_or(*kbps);
                    ("auto".to_owned(), preset, kbps)
                })
                .collect(),
        }
    }

    /// The profile these settings match (bitrates aside), or `None` for custom settings.
    pub fn profile(&self, hw: &Hardware) -> Option<Profile> {
        Profile::ALL.into_iter().find(|p| {
            let t = self.with_profile(*p, hw, &[]);
            // Before the probe finishes, Quality's codec isn't known; HEVC counts too.
            let codec = self.rec_codec == t.rec_codec
                || (*p == Profile::Quality && self.rec_codec == VideoCodec::Hevc);
            // Little memory shortens every profile's buffer to 60 s.
            let buffer = self.clip_buffer == t.clip_buffer || self.clip_buffer == 60;
            let streams = self
                .streams
                .iter()
                .zip(&t.streams)
                .all(|(a, b)| a.0 == b.0 && a.1 == b.1);
            codec
                && buffer
                && streams
                && self.rec_encoder == t.rec_encoder
                && self.rec_rate == t.rec_rate
                && self.rec_quality == t.rec_quality
                && self.rec_preset == t.rec_preset
        })
    }

    /// What changes from `self` to `to`, one line each ("Recording quality: 22 → 20").
    /// `notes` adds a reason to each destination's bitrate line ("limited by your upload").
    pub fn changes(&self, to: &Covered, names: &[String], notes: &[Option<&str>]) -> Vec<String> {
        let mut out = Vec::new();
        let mut line = |what: &str, a: String, b: String| {
            if a != b {
                out.push(format!("{what}: {a} → {b}"));
            }
        };
        line(
            "Recording codec",
            self.rec_codec.label().into(),
            to.rec_codec.label().into(),
        );
        line(
            "Recording encoder",
            encoder_label(&self.rec_encoder),
            encoder_label(&to.rec_encoder),
        );
        line(
            "Recording rate control",
            rate_label(self.rec_rate),
            rate_label(to.rec_rate),
        );
        line(
            "Recording quality",
            self.rec_quality.to_string(),
            to.rec_quality.to_string(),
        );
        line(
            "Recording preset",
            preset_label(self.rec_preset),
            preset_label(to.rec_preset),
        );
        line(
            "Clip buffer",
            format!("{} s", self.clip_buffer),
            format!("{} s", to.clip_buffer),
        );
        for (i, (a, b)) in self.streams.iter().zip(&to.streams).enumerate() {
            let name = names.get(i).map_or("Stream", String::as_str);
            line(
                &format!("{name} encoder"),
                encoder_label(&a.0),
                encoder_label(&b.0),
            );
            line(
                &format!("{name} preset"),
                preset_label(a.1),
                preset_label(b.1),
            );
            if a.2 != b.2 {
                line(
                    &format!("{name} bitrate"),
                    format!("{} kbps", a.2),
                    match notes.get(i).copied().flatten() {
                        Some(note) => format!("{} kbps ({note})", b.2),
                        None => format!("{} kbps", b.2),
                    },
                );
            }
        }
        out
    }
}

fn encoder_label(name: &str) -> String {
    if name == "auto" {
        "Automatic".to_owned()
    } else {
        name.to_owned()
    }
}

fn rate_label(r: RateControl) -> String {
    match r {
        RateControl::Cbr => "Constant bitrate",
        RateControl::Vbr => "Variable bitrate",
        RateControl::ConstantQuality => "Constant quality",
    }
    .to_owned()
}

fn preset_label(p: Preset) -> String {
    match p {
        Preset::Performance => "Performance",
        Preset::Balanced => "Balanced",
        Preset::Quality => "Quality",
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pc(hw_encoder: bool, cores: usize, gb: u64) -> Hardware {
        Hardware {
            gpu: "Test GPU".into(),
            cores,
            memory_bytes: Some(gb * 1024 * 1024 * 1024),
            hw_h264: hw_encoder.then(|| "NVIDIA NVENC H.264".to_owned()),
            hw_hevc: hw_encoder,
            hw_av1: hw_encoder,
        }
    }

    fn covered(streams: usize) -> Covered {
        Covered {
            rec_codec: VideoCodec::H264,
            rec_encoder: "h264_nvenc".into(),
            rec_rate: RateControl::Cbr,
            rec_quality: 22,
            rec_preset: Preset::Balanced,
            clip_buffer: 120,
            streams: vec![("auto".into(), Preset::Balanced, 6000); streams],
        }
    }

    #[test]
    fn recommendation_follows_the_hardware() {
        assert_eq!(recommend(&pc(true, 16, 32)), Profile::Balanced);
        assert_eq!(recommend(&pc(false, 16, 32)), Profile::Gaming);
        assert_eq!(recommend(&pc(true, 2, 32)), Profile::Gaming);
        assert_eq!(recommend(&pc(true, 8, 4)), Profile::Gaming);
        assert!(pc(false, 8, 16).warning().is_some());
        assert!(pc(true, 8, 16).warning().is_none());
    }

    #[test]
    fn profiles_round_trip() {
        let hw = pc(true, 16, 32);
        let now = covered(2);
        assert_eq!(now.profile(&hw), None);
        for p in Profile::ALL {
            let target = now.with_profile(p, &hw, &[Some(6000), None]);
            assert_eq!(target.profile(&hw), Some(p), "{p:?}");
        }
    }

    #[test]
    fn quality_records_hevc_only_with_a_hardware_encoder() {
        let now = covered(0);
        let with = now.with_profile(Profile::Quality, &pc(true, 16, 32), &[]);
        let without = now.with_profile(Profile::Quality, &pc(false, 16, 32), &[]);
        assert_eq!(with.rec_codec, VideoCodec::Hevc);
        assert_eq!(without.rec_codec, VideoCodec::H264);
    }

    #[test]
    fn gaming_is_lighter_than_balanced() {
        let hw = pc(true, 16, 32);
        let g = covered(1).with_profile(Profile::Gaming, &hw, &[]);
        let b = covered(1).with_profile(Profile::Balanced, &hw, &[]);
        assert_eq!(g.rec_preset, Preset::Performance);
        assert!(g.clip_buffer < b.clip_buffer);
        assert!(g.rec_quality > b.rec_quality);
        assert_eq!(Profile::Quality.lighter(), Some(Profile::Balanced));
    }

    #[test]
    fn bitrates_follow_the_platform_or_stay() {
        let hw = pc(true, 16, 32);
        let t = covered(2).with_profile(Profile::Balanced, &hw, &[Some(8000), None]);
        assert_eq!(t.streams[0].2, 8000);
        assert_eq!(t.streams[1].2, 6000);
    }

    #[test]
    fn changes_list_only_differences() {
        let hw = pc(true, 16, 32);
        let now = covered(1);
        let to = now.with_profile(Profile::Balanced, &hw, &[]);
        let lines = now.changes(&to, &["Twitch".into()], &[]);
        assert!(
            lines.contains(&"Recording quality: 22 → 20".to_owned()),
            "{lines:?}"
        );
        assert!(
            lines.iter().all(|l| !l.starts_with("Recording preset")),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("Recording rate control"))
        );
        assert!(to.changes(&to, &[], &[]).is_empty());

        // Bitrate lines carry their reason, and only when the bitrate changes.
        let faster = now.with_profile(Profile::Balanced, &hw, &[Some(8_000)]);
        let lines = faster.changes(&to, &["Twitch".into()], &[Some("limited by your upload")]);
        assert!(
            lines.contains(
                &"Twitch bitrate: 8000 kbps → 6000 kbps (limited by your upload)".to_owned()
            ),
            "{lines:?}"
        );
        assert!(
            to.changes(&to, &["Twitch".into()], &[Some("platform maximum")])
                .is_empty()
        );
    }

    fn need(service: estream_media::Service, w: u32, h: u32, fps: f64) -> StreamNeed {
        StreamNeed {
            service,
            enabled: true,
            current_kbps: 6_000,
            audio_kbps: 160,
            width: w,
            height: h,
            fps,
        }
    }

    #[test]
    fn fast_upload_gets_the_platform_ceiling() {
        use estream_media::Service;
        let fits = budget(100_000.0, &[need(Service::YouTube, 1920, 1080, 60.0)]);
        let fit = fits[0].unwrap();
        assert_eq!(
            (fit.kbps, fit.limit, fit.short),
            (17_000, Limit::Platform, false)
        );
    }

    #[test]
    fn slow_upload_limits_and_warns() {
        use estream_media::Service;
        // 5 Mb/s: min(3750, 3500) - 160 audio.
        let fit = budget(5_000.0, &[need(Service::Twitch, 1920, 1080, 60.0)])[0].unwrap();
        assert_eq!((fit.kbps, fit.limit), (3_340, Limit::Upload));
        assert!(fit.short, "1080p60 needs about 4.5 Mb/s");
        // Very slow: the floor, still marked as short.
        let fit = budget(1_000.0, &[need(Service::Twitch, 1280, 720, 30.0)])[0].unwrap();
        assert_eq!(fit.kbps, FLOOR_KBPS);
        assert!(fit.short);
    }

    #[test]
    fn destinations_share_the_upload() {
        use estream_media::Service;
        let needs = [
            need(Service::Twitch, 1920, 1080, 60.0),
            need(Service::Kick, 1920, 1080, 60.0),
            StreamNeed {
                enabled: false,
                ..need(Service::YouTube, 1920, 1080, 60.0)
            },
        ];
        // 20 Mb/s: 15 Mb/s usable, 7.5 each, less 160 audio.
        let fits = budget(20_000.0, &needs);
        assert_eq!(fits[0].unwrap().kbps, 6_000, "Twitch's limit");
        assert_eq!(fits[0].unwrap().limit, Limit::Platform);
        assert_eq!(fits[1].unwrap().kbps, 7_340);
        assert_eq!(fits[1].unwrap().limit, Limit::Upload);
        assert_eq!(fits[2], None, "disabled destinations keep theirs");
    }

    #[test]
    fn congested_streams_get_what_got_through() {
        // 4 Mb/s got through: 3400 - 160 audio, rounded down.
        assert_eq!(fit_to_sent(4_000.0, 6_000, 160), Some(3_200));
        assert_eq!(fit_to_sent(0.0, 6_000, 160), Some(FLOOR_KBPS));
        assert_eq!(fit_to_sent(9_000.0, 6_000, 160), None, "never raised");
        assert_eq!(fit_to_sent(1_000.0, FLOOR_KBPS, 160), None);
    }

    #[test]
    fn custom_servers_are_only_lowered() {
        use estream_media::Service;
        let custom = need(Service::Custom, 1920, 1080, 60.0);
        let fit = budget(100_000.0, &[custom])[0].unwrap();
        assert_eq!((fit.kbps, fit.limit), (6_000, Limit::Kept));
        let fit = budget(6_000.0, &[custom])[0].unwrap();
        assert_eq!((fit.kbps, fit.limit), (4_340, Limit::Upload));
    }

    #[test]
    fn little_memory_keeps_the_buffer_short() {
        let t = covered(0).with_profile(Profile::Balanced, &pc(true, 8, 4), &[]);
        assert_eq!(t.clip_buffer, 60);
    }
}
