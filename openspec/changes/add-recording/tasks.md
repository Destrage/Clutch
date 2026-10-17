# Tasks: add-recording

## 1. FFmpeg setup
- [x] 1.1 Bundle FFmpeg 9 LGPL shared build (`third_party/ffmpeg`, git-ignored) + build config (FFMPEG_DIR, LIBCLANG_PATH)
- [x] 1.2 Copy FFmpeg DLLs next to built binaries; version/encoder probe at startup
- [ ] 1.3 CI: download FFmpeg + LLVM — not done (no CI workflow yet; setup is documented in `.cargo/config.toml`)

## 2. Frame delivery
- [x] 2.1 NV12 readback ring (pooled MAP_READ buffers, async map, pooled CPU frames)
- [x] 2.2 Engine API to attach/detach a video consumer to a canvas; drop counting
- [x] 2.3 Audio track consumer via `TrackSink` with bounded queue

## 3. Encoding and muxing (`estream-media`)
- [x] 3.1 Video encoder wrapper (NVENC/AMF/QSV/software), rate control, presets, keyframe interval
- [x] 3.2 Automatic encoder selection by probing
- [x] 3.3 Audio encoders (AAC/Opus) with re-chunking, one per track
- [x] 3.4 Muxer (MKV, fragmented MP4, MP4, MOV) with interleaving and timestamp rebasing
- [x] 3.5 Recording output (thread, start/stop, stats)
- [x] 3.6 Replay buffer (packet ring, save on demand)
- [x] 3.7 Remux MKV → MP4

## 4. Studio
- [x] 4.1 Record button with timer; Stop; toast with saved path. "Show in folder" is a palette action
      (`output.reveal`) rather than a toast button — toasts have no actions yet.
- [x] 4.2 Replay buffer toggle + Save replay (button, palette, Ctrl+Shift+R)
- [x] 4.3 Settings dialog: output folder, filename, container, encoder, codec, rate control, bitrate/quality, audio, tracks, replay length, auto-remux
- [x] 4.4 Status bar: recording size, bitrate, dropped frames

## 5. Verification
- [x] 5.1 Record test pattern + tone (3 s); probe duration, resolution, codecs, stream count.
      Also verified by hand in the app: 1080p60 NVENC H.264 + AAC, duration checked with ffprobe.
- [x] 5.2 Encoder probe test (lists available encoders, auto picks hardware when present)
- [x] 5.3 Replay buffer test (length within bounds; keyframe start is by construction, not asserted)
- [x] 5.4 Remux test; A/V duration match test

## Known issues
- One frame shows as "lagged" when recording starts (readback buffers are allocated on first use).
- AMF/QSV paths are implemented but only NVENC and the software encoders were exercised on this machine.
