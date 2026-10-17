# Tasks: add-streaming-outputs

## 1. Media (`estream-media`)
- [x] 1.1 `StreamSettings` + service presets + URL building / redaction / protocol detection
- [x] 1.2 Forced keyframe support in `VideoEncoder` (IDR on NVENC/QSV/AMF via `forced-idr`/`forced_idr`;
      Kvazaar ignores it, so streams fall back to the next regular keyframe)
- [x] 1.3 `SendQueue` with keyframe-aligned congestion dropping
- [x] 1.4 Network output: interrupt-aware connect, sender thread, reconnect with back-off
- [x] 1.5 `Stream` output (start/stop/stats/state) on the shared encoder session; non-blocking stop
- [x] 1.6 Validation of protocol constraints (RTMP needs a key, WHIP→H.264; audio codec chosen per transport)

## 2. Studio
- [x] 2.1 Stream key encrypted at rest (DPAPI)
- [x] 2.2 Go Live button: connecting / live timer / reconnecting
- [x] 2.3 Settings: Stream section (service, server, key, codec, encoder, bitrate, audio, reconnect)
- [x] 2.4 Status bar: live bitrate, drops, congestion indicator

## 3. Verification
- [x] 3.1 Unit tests: URLs, redaction, congestion dropping, back-off, key never serialized, DPAPI round trip
- [x] 3.2 RTMP end-to-end against a local FFmpeg server (H.264; HEVC and AV1 via Enhanced RTMP)
- [x] 3.3 SRT end-to-end against a local FFmpeg server
- [x] 3.4 Reconnect test (server appears later; received file starts at a keyframe)
- [x] 3.5 Stop while the server hangs returns in ~1.2 s
- [x] 3.6 In-app check: Go Live to a local RTMP server, 1080p60 NVENC H.264 + AAC, 9 s received intact

## Known issues / not covered
- WHIP is implemented but not tested (needs a WebRTC/WHIP server). RIST is untested.
- Encoder probing runs on the UI thread the first time (~0.5 s pause on first Go Live / Settings).
- libkvazaar needs frame sizes that are multiples of 8; libsvtav1 did not open in the probe on this
  machine (AV1 software fallback unavailable here).
- Service presets prefill ingest URLs from public docs/user reports; they stay editable.
