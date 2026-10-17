# Tasks: add-audio-engine

## 1. Foundation
- [x] 1.1 Move `SourceStatus`, `FrameClock`/`wait_until` to `estream-core`
- [x] 1.2 `estream-audio` crate: block type, SPSC rings (`rtrb`), sinc resampler wrapper (`rubato`), dB helpers
- [x] 1.3 `AudioSource` trait; `SourceType` optional audio factory (video factory optional too)
- [x] 1.4 `AudioConfig` on `SourceDef` (volume, mute, balance, mono, sync offset, monitoring, tracks, global, filters) + `UpdateSourceAudio` / `SetProjectAudio` commands

## 2. Mixer
- [x] 2.1 Audio thread with 100 Hz frame clock (catch-up after stalls); source sync with the graph
- [x] 2.2 Per-source chain: delay, filters, mono/balance, smoothed volume/mute, post-fader meters, routing
- [x] 2.3 Six track mixes + monitor mix + master meter; `TrackSink` interface for recording/streaming
- [x] 2.4 Underrun/overrun handling with bounded latency (pad with silence; trim beyond 60 ms)

## 3. Sources and devices
- [x] 3.1 Test tone source
- [x] 3.2 Input devices (cpal) with enumeration, default device, resampling, reconnect loop (unplug not physically tested yet)
- [x] 3.3 Desktop audio (output loopback) — verified with live metering
- [x] 3.4 Per-application audio (WASAPI process loopback, process tree) with app picker and re-attach — opt-in test captures a single process
- [ ] 3.5 Monitoring output device — implemented (cpal output, linear resampling), not yet verified by ear; no UI to pick a non-default monitoring device yet

## 4. Filters
- [x] 4.1 Gain, noise gate, compressor, limiter
- [x] 4.2 Noise suppression (RNNoise via `nnnoiseless`)

## 5. Studio
- [x] 5.1 Mixer panel: master + source strips with stereo peak/RMS meters, peak hold, fader, dB readout, mute
- [x] 5.2 Inspector audio section: volume, mute, balance, mono, all-scenes, sync offset, monitoring, tracks 1–6
- [x] 5.3 Filter chain editor (add/remove/move up-down/toggle, generic parameters)
- [x] 5.4 New collections get global Desktop Audio + Microphone; audio sources in the Add Source dialog

## 6. Verification
- [x] 6.1 Mixer tests with synthetic signals (volume dB, mute, routing, monitor-only, removal, balance, delay, filters)
- [x] 6.2 Filter tests (limiter ceiling, gate close/re-open, compressor ratio, RNNoise attenuation)
- [x] 6.3 Block-rate test (100 ± 2 blocks per second over 2 s)
- [ ] 6.4 Manual checks: desktop audio verified; microphone and device unplug/replug still to check
