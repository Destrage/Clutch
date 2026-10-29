# Change: Add streaming outputs (RTMP(S), Enhanced RTMP, SRT, RIST, WHIP)

## Why
eStream can record but not go live. Streaming is the core job of the app. It must survive bad
networks without stalling rendering, reconnect by itself, and support the newer protocols (SRT,
WHIP, HEVC/AV1 over Enhanced RTMP).

## What Changes
- **streaming** capability (new):
  - One live stream to a destination chosen by URL scheme:
    - RTMP / RTMPS → FLV, using Enhanced RTMP automatically for HEVC and AV1;
    - SRT and RIST → MPEG-TS;
    - WHIP (WebRTC) for `http(s)://` endpoints.
  - Service presets (Twitch, YouTube, Facebook Live) that prefill the server; custom servers for
    everything else. Protocol-specific constraints are checked before connecting (e.g. WHIP needs
    H.264 + Opus, RTMP needs AAC).
  - Network isolation: a sender thread owns the connection. When the network falls behind, video
    packets are dropped from the send queue (whole GOP tail up to the next keyframe), audio is kept,
    and drops are counted. Rendering and encoding never block on the network.
  - Automatic reconnect with exponential back-off. A forced keyframe and rebased timestamps start
    each new connection.
  - Stats: state (connecting / live / reconnecting / stopped), sent bitrate, network-dropped frames,
    queue delay, reconnect count.
- Stream key stored encrypted for the current user (Windows DPAPI), never written in plain text to
  settings or logs.
- Studio: working **Go Live** button (connecting / live timer / reconnecting states), a Stream
  section in Settings (service, server, key, codec, encoder, bitrate, audio track, reconnect),
  status bar with live bitrate, drops and a congestion indicator.

## Out of scope (later changes)
- Multiple simultaneous destinations (`add-multistream`).
- Dynamic bitrate (lowering encoder bitrate under congestion).
- Sharing one encoder between recording and streaming.
- Twitch Enhanced Broadcasting / multitrack video.

## Impact
- Affected specs: `streaming` (new)
- Affected code: `crates/estream-media` (stream output, network muxer, settings), `apps/estream`
  (Go Live, settings, stats, secret storage)
