# Design: Streaming outputs

## Threads
```
video/audio threads ──frames──▶ encoder thread ──packets──▶ SendQueue ──▶ sender thread ──▶ network
                                  ▲ force_keyframe (atomic) ◀─────────────────────┘
```
- The encoder thread is the same one recordings use (`Session` / `State`). Its target is a
  new `Target::Network(SendQueue)`. Pushing a packet never blocks.
- The **sender thread** owns the FFmpeg output context and does every blocking network call:
  connect, header, write and trailer. It pops packets from the queue and writes them interleaved.

## Send queue and congestion
`SendQueue` = `Mutex<VecDeque<Queued>>` + `Condvar`. Each entry carries the stream index, the
packet, whether it is video/keyframe, and its time in seconds.
- If the buffered video span (newest − oldest video time) exceeds `drop_threshold` (default
  700 ms), the queue drops video packets from the oldest end up to the next keyframe. This keeps
  decoders valid (no P-frames without their reference). Audio
  packets are never dropped.
- If no keyframe is queued, all queued video is dropped and the queue waits for the next keyframe
  before accepting video again.
- Every drop adds to `OutputStats::network_dropped`. The queue delay (span) is exposed for the
  congestion indicator.

## Connecting and reconnecting
1. Format by scheme:
   - `rtmp`, `rtmps` → `flv` (FFmpeg 9 writes Enhanced RTMP FourCC headers for HEVC and AV1);
   - `srt`, `rist` → `mpegts`;
   - `http`, `https` → `whip`.
2. The output context is allocated with an **interrupt callback** tied to the stop flag, so a
   hanging connect or write returns quickly on Stop. `rw_timeout` (10 s) catches dead peers.
   Muxers with `AVFMT_NOFILE` (WHIP) open their own connection in `write_header`.
3. On a connect or write error: state → Reconnecting(n). The queue is cleared and set to wait for
   a keyframe. The sender waits `delay · 2^(n-1)` (capped at 60 s) and then reconnects. On success
   it sets `force_keyframe`. The encoder marks the next frame as an I-frame
   (`AV_PICTURE_TYPE_I`), and timestamps of the new connection are rebased to start at 0.
4. After `max_retries` failed attempts the stream stops with an error shown to the user.

## URLs and secrets
- RTMP: `server + "/" + key` (a trailing `/` on the server is handled).
- SRT: the key, if set, becomes the `streamid` query parameter.
- WHIP: the key, if set, is sent as a Bearer token (`authorization` option).
- RIST: the URL is used as entered.
- URLs are **redacted** in logs and errors (key and query string replaced by `***`).
- The key is stored in app settings as DPAPI-encrypted base64 (`CryptProtectData`, current user
  scope). Other platforms keep it in memory only until a keychain backend lands (not needed yet;
  Windows-first).

## Encoder settings for streaming
- CBR is forced. Keyframe interval is 2 s (all services require ≤ 2–4 s). No B-frames (the
  existing default).
- RTMP needs AAC audio. WHIP needs H.264 video and Opus audio.
- One audio track (selectable, default track 1).
- Service presets carry a recommended maximum bitrate. Settings warn when the chosen bitrate is
  above it. Presets prefill the server, but the field stays editable because ingest hosts change.

## Testing
- Unit:
  - URL building and redaction;
  - scheme → format mapping;
  - congestion dropping (keyframe-aligned, audio kept);
  - back-off schedule.
- Integration, using the bundled `ffmpeg.exe` as a local server:
  - RTMP: `ffmpeg -listen 1 -i rtmp://127.0.0.1:<port>/live/<key> -c copy out.flv`; probe codecs
    and duration.
  - SRT: `ffmpeg -i srt://127.0.0.1:<port>?mode=listener -c copy out.ts`.
  - Reconnect: start streaming with no server (state is Reconnecting), then start the server; the
    stream connects and the received file starts with a keyframe.
- WHIP needs a WebRTC server. It is not covered by automated tests; this is listed as an open item.

## Risks
- Ingest hostnames change. Presets only prefill an editable field.
- A blocking TLS handshake inside FFmpeg. Mitigated by the interrupt callback and `rw_timeout`.
