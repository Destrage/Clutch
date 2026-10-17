//! Recording settings (stored in app settings, like an OBS profile).

use estream_core::CanvasId;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Container {
    /// Matroska: survives crashes (default).
    #[default]
    Mkv,
    /// Fragmented MP4: crash-safe and widely playable.
    FragmentedMp4,
    Mp4,
    Mov,
}

impl Container {
    pub const ALL: [Container; 4] = [
        Container::Mkv,
        Container::FragmentedMp4,
        Container::Mp4,
        Container::Mov,
    ];

    pub fn extension(self) -> &'static str {
        match self {
            Container::Mkv => "mkv",
            Container::FragmentedMp4 | Container::Mp4 => "mp4",
            Container::Mov => "mov",
        }
    }

    pub fn format_name(self) -> &'static str {
        match self {
            Container::Mkv => "matroska",
            Container::FragmentedMp4 | Container::Mp4 => "mp4",
            Container::Mov => "mov",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Container::Mkv => "MKV (crash-safe)",
            Container::FragmentedMp4 => "Fragmented MP4 (crash-safe)",
            Container::Mp4 => "MP4",
            Container::Mov => "MOV",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoCodec {
    #[default]
    H264,
    Hevc,
    Av1,
}

impl VideoCodec {
    pub const ALL: [VideoCodec; 3] = [VideoCodec::H264, VideoCodec::Hevc, VideoCodec::Av1];

    pub fn label(self) -> &'static str {
        match self {
            VideoCodec::H264 => "H.264",
            VideoCodec::Hevc => "HEVC (H.265)",
            VideoCodec::Av1 => "AV1",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateControl {
    Cbr,
    Vbr,
    /// Constant quality (CQP/CQ/CRF depending on the encoder).
    #[default]
    ConstantQuality,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    Performance,
    #[default]
    Balanced,
    Quality,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioCodec {
    #[default]
    Aac,
    Opus,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordingSettings {
    pub folder: PathBuf,
    /// `strftime`-style pattern for file names (without extension).
    pub filename: String,
    pub container: Container,
    pub codec: VideoCodec,
    /// `"auto"` or an FFmpeg encoder name such as `h264_nvenc`.
    pub encoder: String,
    pub rate_control: RateControl,
    pub bitrate_kbps: u32,
    /// Quality level for constant-quality mode (lower = better; ~18–28).
    pub quality: u32,
    pub keyframe_seconds: u32,
    pub preset: Preset,
    pub audio_codec: AudioCodec,
    pub audio_bitrate_kbps: u32,
    /// Bitmask of audio tracks to record (bit 0 = track 1).
    pub tracks: u8,
    pub replay_seconds: u32,
    /// Remux MKV recordings to MP4 after they stop.
    pub auto_remux: bool,
    /// Canvas to record (`None` = the first canvas). Also used for the replay buffer.
    pub canvas: Option<CanvasId>,
    /// Record every canvas at once (one file each).
    pub all_canvases: bool,
}

impl Default for RecordingSettings {
    fn default() -> Self {
        let folder = dirs_video().join("eStream");
        Self {
            folder,
            filename: "eStream %Y-%m-%d %H-%M-%S".into(),
            container: Container::Mkv,
            codec: VideoCodec::H264,
            encoder: "auto".into(),
            rate_control: RateControl::ConstantQuality,
            bitrate_kbps: 12_000,
            quality: 22,
            keyframe_seconds: 2,
            preset: Preset::Balanced,
            audio_codec: AudioCodec::Aac,
            audio_bitrate_kbps: 192,
            tracks: 0b1,
            replay_seconds: 30,
            auto_remux: false,
            canvas: None,
            all_canvases: false,
        }
    }
}

fn dirs_video() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|h| PathBuf::from(h).join("Videos"))
        .unwrap_or_else(|| PathBuf::from("."))
}

impl RecordingSettings {
    /// A new, unused file path for a recording started now.
    pub fn next_path(&self, suffix: &str, container: Container) -> PathBuf {
        let stem = chrono::Local::now().format(&self.filename).to_string();
        let stem = format!("{stem}{suffix}");
        let mut path = self
            .folder
            .join(format!("{stem}.{}", container.extension()));
        let mut n = 2;
        while path.exists() {
            path = self
                .folder
                .join(format!("{stem} ({n}).{}", container.extension()));
            n += 1;
        }
        path
    }
}

/// Streaming service presets. They only prefill the server; it stays editable because
/// ingest hosts change over time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Service {
    Twitch,
    YouTube,
    Facebook,
    #[default]
    Custom,
}

impl Service {
    pub const ALL: [Service; 4] = [
        Service::Twitch,
        Service::YouTube,
        Service::Facebook,
        Service::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Service::Twitch => "Twitch",
            Service::YouTube => "YouTube",
            Service::Facebook => "Facebook Live",
            Service::Custom => "Custom (RTMP, SRT, RIST, WHIP)",
        }
    }

    /// Server prefilled when the service is picked (empty for custom).
    pub fn default_server(self) -> &'static str {
        match self {
            Service::Twitch => "rtmp://live.twitch.tv/app",
            Service::YouTube => "rtmps://a.rtmps.youtube.com/live2",
            Service::Facebook => "rtmps://live-api-s.facebook.com:443/rtmp/",
            Service::Custom => "",
        }
    }

    /// Highest video bitrate the service accepts for H.264, if it publishes one.
    pub fn max_video_kbps(self) -> Option<u32> {
        match self {
            Service::Twitch => Some(6_000),
            _ => None,
        }
    }
}

/// Transport, derived from the destination URL scheme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    /// RTMP or RTMPS (FLV; Enhanced RTMP for HEVC/AV1).
    Rtmp,
    Srt,
    Rist,
    /// WebRTC-HTTP ingestion.
    Whip,
}

impl Protocol {
    pub fn of(url: &str) -> Option<Self> {
        let scheme = url.split_once("://")?.0.to_ascii_lowercase();
        Some(match scheme.as_str() {
            "rtmp" | "rtmps" => Protocol::Rtmp,
            "srt" => Protocol::Srt,
            "rist" => Protocol::Rist,
            "http" | "https" => Protocol::Whip,
            _ => return None,
        })
    }

    /// FFmpeg muxer for this transport.
    pub fn format_name(self) -> &'static str {
        match self {
            Protocol::Rtmp => "flv",
            Protocol::Srt | Protocol::Rist => "mpegts",
            Protocol::Whip => "whip",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Protocol::Rtmp => "RTMP",
            Protocol::Srt => "SRT",
            Protocol::Rist => "RIST",
            Protocol::Whip => "WHIP",
        }
    }

    /// Audio codec the transport needs.
    pub fn audio_codec(self) -> AudioCodec {
        match self {
            Protocol::Whip => AudioCodec::Opus,
            _ => AudioCodec::Aac,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamSettings {
    pub service: Service,
    pub server: String,
    /// Stream key. Never serialized: the app keeps it in encrypted storage.
    #[serde(skip)]
    pub key: String,
    pub codec: VideoCodec,
    /// `"auto"` or an FFmpeg encoder name.
    pub encoder: String,
    pub bitrate_kbps: u32,
    pub preset: Preset,
    pub keyframe_seconds: u32,
    pub audio_bitrate_kbps: u32,
    /// Audio track to send (0 = track 1).
    pub audio_track: u8,
    pub reconnect: bool,
    pub retry_delay_seconds: u32,
    pub max_retries: u32,
    /// Queued video beyond this is dropped (at keyframe boundaries).
    pub drop_threshold_ms: u32,
    /// Canvas to stream (`None` = the first canvas).
    pub canvas: Option<CanvasId>,
}

impl Default for StreamSettings {
    fn default() -> Self {
        Self {
            service: Service::Custom,
            server: String::new(),
            key: String::new(),
            codec: VideoCodec::H264,
            encoder: "auto".into(),
            bitrate_kbps: 6_000,
            preset: Preset::Balanced,
            keyframe_seconds: 2,
            audio_bitrate_kbps: 160,
            audio_track: 0,
            reconnect: true,
            retry_delay_seconds: 2,
            max_retries: 25,
            drop_threshold_ms: 700,
            canvas: None,
        }
    }
}

impl StreamSettings {
    pub fn protocol(&self) -> Option<Protocol> {
        Protocol::of(self.server.trim())
    }

    /// Checks the settings before connecting; the message is shown to the user.
    pub fn validate(&self) -> Result<Protocol, String> {
        let server = self.server.trim();
        if server.is_empty() {
            return Err("Set a stream server in Settings → Stream".into());
        }
        let protocol = self.protocol().ok_or_else(|| {
            "The server must start with rtmp://, rtmps://, srt://, rist:// or https:// (WHIP)"
                .to_string()
        })?;
        if protocol == Protocol::Rtmp && self.key.trim().is_empty() {
            return Err("Enter your stream key in Settings → Stream".into());
        }
        if protocol == Protocol::Whip && self.codec != VideoCodec::H264 {
            return Err("WHIP (WebRTC) needs the H.264 video codec".into());
        }
        Ok(protocol)
    }

    /// Full destination URL, including the key where the protocol carries it in the URL.
    pub fn url(&self) -> String {
        let server = self.server.trim();
        let key = self.key.trim();
        match self.protocol() {
            Some(Protocol::Rtmp) if !key.is_empty() => {
                format!("{}/{}", server.trim_end_matches('/'), key)
            }
            Some(Protocol::Srt) if !key.is_empty() => {
                let sep = if server.contains('?') { '&' } else { '?' };
                format!("{server}{sep}streamid={}", percent_encode(key))
            }
            _ => server.to_owned(),
        }
    }

    /// Encoder settings for streaming: CBR, the transport's audio codec, one track.
    pub fn encoder_settings(&self) -> RecordingSettings {
        let protocol = self.protocol().unwrap_or(Protocol::Rtmp);
        RecordingSettings {
            codec: self.codec,
            encoder: self.encoder.clone(),
            rate_control: RateControl::Cbr,
            bitrate_kbps: self.bitrate_kbps.max(100),
            keyframe_seconds: self.keyframe_seconds.clamp(1, 4),
            preset: self.preset,
            audio_codec: protocol.audio_codec(),
            audio_bitrate_kbps: self.audio_bitrate_kbps,
            tracks: 1 << self.audio_track.min(5),
            canvas: self.canvas,
            ..Default::default()
        }
    }

    /// Delay before reconnect attempt `n` (1-based): exponential, capped at 60 s.
    pub fn retry_delay(&self, attempt: u32) -> std::time::Duration {
        let base = self.retry_delay_seconds.max(1) as u64;
        let secs = base.saturating_mul(1u64 << attempt.saturating_sub(1).min(16));
        std::time::Duration::from_secs(secs.min(60))
    }
}

/// `scheme://host[:port]/***`: hides keys, tokens, passwords and query strings in logs.
pub fn redact(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return "***".into();
    };
    let (authority, tail) = match rest.find(['/', '?']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    if tail.trim_matches('/').is_empty() {
        format!("{scheme}://{host}")
    } else {
        format!("{scheme}://{host}/***")
    }
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn stream(server: &str, key: &str) -> StreamSettings {
        StreamSettings {
            server: server.into(),
            key: key.into(),
            ..Default::default()
        }
    }

    #[test]
    fn protocol_from_scheme() {
        assert_eq!(Protocol::of("rtmps://x/app"), Some(Protocol::Rtmp));
        assert_eq!(Protocol::of("SRT://x:9000"), Some(Protocol::Srt));
        assert_eq!(Protocol::of("rist://x:5000"), Some(Protocol::Rist));
        assert_eq!(Protocol::of("https://x/whip"), Some(Protocol::Whip));
        assert_eq!(Protocol::of("ftp://x"), None);
        assert_eq!(Protocol::of("live.twitch.tv"), None);
    }

    #[test]
    fn urls_carry_the_key() {
        assert_eq!(stream("rtmp://h/app/", "k1").url(), "rtmp://h/app/k1");
        assert_eq!(stream("rtmp://h/app", "k1").url(), "rtmp://h/app/k1");
        assert_eq!(
            stream("srt://h:9000", "#!::r=live/x").url(),
            "srt://h:9000?streamid=%23%21%3A%3Ar%3Dlive%2Fx"
        );
        assert_eq!(
            stream("srt://h:9000?latency=2000", "id").url(),
            "srt://h:9000?latency=2000&streamid=id"
        );
        // WHIP sends the key as a bearer token, not in the URL.
        assert_eq!(stream("https://h/whip", "tok").url(), "https://h/whip");
    }

    #[test]
    fn redaction_hides_secrets() {
        assert_eq!(
            redact("rtmp://live.twitch.tv/app/live_123_abc"),
            "rtmp://live.twitch.tv/***"
        );
        assert_eq!(redact("srt://h:9000?streamid=secret"), "srt://h:9000/***");
        assert_eq!(redact("rtmp://user:pw@h:1935/app/k"), "rtmp://h:1935/***");
        assert_eq!(redact("https://h"), "https://h");
        assert_eq!(redact("garbage"), "***");
    }

    #[test]
    fn validation() {
        assert!(stream("", "k").validate().is_err());
        assert!(stream("rtmp://h/app", "").validate().is_err());
        assert_eq!(stream("rtmp://h/app", "k").validate(), Ok(Protocol::Rtmp));
        assert_eq!(stream("srt://h:1", "").validate(), Ok(Protocol::Srt));
        let mut whip = stream("https://h/whip", "");
        whip.codec = VideoCodec::Av1;
        assert!(whip.validate().is_err());
    }

    #[test]
    fn backoff_doubles_and_caps() {
        let s = StreamSettings::default();
        let secs: Vec<u64> = (1..=7).map(|n| s.retry_delay(n).as_secs()).collect();
        assert_eq!(secs, [2, 4, 8, 16, 32, 60, 60]);
    }

    #[test]
    fn key_is_never_serialized() {
        let s = stream("rtmp://h/app", "secret-key");
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("secret-key"));
    }
}
