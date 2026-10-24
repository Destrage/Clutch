# Tasks: add-stream-alerts

## 1. Typed events
- [x] 1.1 `StreamEvent` with kind, platform, name, amount, months, tier, message; readers fill it (Twitch, Kick, YouTube)
- [x] 1.2 Chat event text unchanged (built per platform as before); tests per platform payload for the typed fields

## 2. Event sounds
- [x] 2.1 Per-kind sound files and a volume in settings; chime as fallback; bell mutes all
- [x] 2.2 Short-sound player (`estream-media::sounds`: decode once with FFmpeg, cache, mix, play on the monitoring device)
- [x] 2.3 Settings UI: a sound picker and play button per kind

## 3. Overlay server
- [ ] 3.1 Loopback HTTP/WebSocket server with a per-install token; starts when an overlay source exists
- [ ] 3.2 Asset route by id for user files; config and alert messages over the WebSocket
- [ ] 3.3 Tests: token required, unknown asset ids rejected, events reach a connected client

## 4. Alerts source
- [ ] 4.1 `alerts` source type built on the browser source (canvas size, transparent, audio on)
- [ ] 4.2 Bundled alerts page: queue, per-kind animation, sound, text template, layout, duration
- [ ] 4.3 Inspector: per-kind settings, minimum amount, Test button
- [ ] 4.4 Live text: template placeholders, optional parts, text box, enter/leave timing, shrink to fit
- [ ] 4.5 Alert packs: `pack.json` / per-kind `alert.json`, several sounds (in turn or random), big variants, per-source overrides
- [ ] 4.6 Built-in pack from the project owner's animations and sounds
- [ ] 4.7 Advanced: per-kind `alert.html` + CSS

## 5. Verification
- [ ] 5.1 `cargo test --workspace`, `cargo clippy --workspace --all-targets`
- [ ] 5.2 Test alerts for every kind on a 1080p canvas: transparent, sound on the source's fader, one at a time
- [ ] 5.3 Live check with a real Twitch follow and a Kick follow
