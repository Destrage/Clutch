# Design: Recording

## Frame delivery (GPU → encoder)
1. When at least one encoder is attached to a canvas, the video thread also converts the canvas to
   NV12 (`OutputConverter::to_nv12`) and copies the planes into one of N pooled `MAP_READ` buffers
   in the same submission.
2. `map_async` is requested; completion is polled each tick (never waited on). When a buffer is
   mapped, its rows are copied into a pooled `Vec<u8>` and sent to the encoder thread through a
   bounded channel together with the frame's engine timestamp. The GPU buffer is unmapped and reused.
3. If the encoder falls behind (channel full, no free buffer), the frame is dropped and counted as
   an *encoder lag* drop; rendering is never stalled.

Zero-copy NVENC input (D3D11 textures as FFmpeg hw frames) is a later optimization; the readback
of 1080p60 NV12 is ~190 MB/s, well within PCIe bandwidth.

## Encoding and muxing
- One output = one encoder thread that owns the FFmpeg muxer, the video encoder and one audio
  encoder per enabled track. Audio blocks (480 frames) are queued by the `TrackSink` and
  re-chunked into encoder frames (1024 for AAC, 960 for Opus).
- Time bases: video `1/fps` (frame index from timestamp), audio `1/48000` (sample count).
  The output's time zero is the first video frame's timestamp; earlier audio is discarded.
- Packets are written with `av_interleaved_write_frame`.
- Encoder selection order for "auto": NVENC → AMF → QSV → software. Each is probed by actually
  opening it at the requested size; the first that opens wins.

## Replay buffer
A separate output whose "muxer" is an in-memory packet ring trimmed by duration (always starting
at a keyframe). Saving writes the buffered packets (with rebased timestamps) to a new file using
the same container code. It keeps running after a save.

## Containers
MKV by default (playable after a crash). Fragmented MP4 (`movflags=frag_keyframe+empty_moov+default_base_moof`)
for crash-safe MP4. Plain MP4/MOV write the index at the end. Optional remux MKV→MP4 after stop
(stream copy, no re-encode).

## Settings
Stored in the app settings (like OBS profiles), not in scene collections: folder, filename
pattern, container, encoder, codec, rate control, bitrate / quality, keyframe interval, preset,
audio codec + bitrate, tracks, replay seconds, auto-remux.

## Risks
- FFmpeg API/ABI: dynamically linked LGPL build; the version is pinned by the bundled DLLs.
- Consumer NVIDIA GPUs limit concurrent NVENC sessions (8 on current drivers); recording +
  replay buffer + streaming use 3.
