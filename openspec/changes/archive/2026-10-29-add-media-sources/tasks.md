# Tasks: add-media-sources

## 1. Player and source (`estream-media`)
- [x] 1.1 Player: playlist (files and folders), demux/decode thread, NV12/RGBA video, 48 kHz stereo
      audio, playback clock; libvpx decoders for VP8/VP9 so WebM alpha survives
- [x] 1.2 Controls: pause/resume, restart, next/previous, seek; position, duration, current item
- [x] 1.3 Registry shared by a source's video and audio sides (players stop without blocking)
- [x] 1.4 `media` source type (video + audio sides, settings schema)

## 2. Engine
- [x] 2.1 `_source_id` in settings handed to source factories (never stored)
- [x] 2.2 Stinger transition kind; helper media source while it runs; outgoing scene until the
      switch point, then the new scene, with the stinger drawn on top in the same pass

## 3. Studio
- [x] 3.1 Register media sources; Media section with controls (play/pause, restart, previous,
      next, seek bar, time) in the inspector
- [x] 3.2 Stinger option in the transition picker (video file, switch point, duration)

## 4. Verification
- [x] 4.1 Player tests with generated media (video+audio in real time, playlist with image, loop
      and end, pause, missing file, alpha, shared player)
- [x] 4.2 Engine tests (alpha media on canvas, shared player through the mixer, stinger switch point)
- [x] 4.3 In-app check: clip playing in the scene, media controls, pause

## Also fixed
- The app panicked during process exit in debug builds (Slint's thread-local GPU queue dropped
  after wgpu's thread-locals). The process now keeps one reference to the GPU objects.

## Known issues / follow-ups
- Once, the app exited at startup right after the known `Surface::configure` race was logged; not
  reproduced in 9 further launches.
- Stingers start decoding when the transition starts, so the first ~50–100 ms can be missing.
  Preloading the stinger would fix this.
- Software decoding only; "play only when visible" and restart-on-show need scene visibility
  events.
- Pausing a media source pauses its audio by withholding samples, which re-primes the audio lead
  on resume (about 100 ms of silence).
