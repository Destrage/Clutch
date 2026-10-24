# Tasks: add-stream-alerts

## 1. Typed events
- [ ] 1.1 `StreamEvent` with kind, platform, name, amount, months, tier, message; readers fill it (Twitch, Kick, YouTube)
- [ ] 1.2 Chat event text built from the typed event (unchanged look); tests per platform payload

## 2. Event sounds
- [ ] 2.1 Per-kind sound files and a volume in settings; chime as fallback; bell mutes all
- [ ] 2.2 Short-sound player (decode once, cache, play asynchronously)
- [ ] 2.3 Settings UI: a sound picker and play button per kind

## 3. Overlay server
- [ ] 3.1 Loopback HTTP/WebSocket server with a per-install token; starts when an overlay source exists
- [ ] 3.2 Asset route by id for user files; config and alert messages over the WebSocket
- [ ] 3.3 Tests: token required, unknown asset ids rejected, events reach a connected client

## 4. Alerts source
- [ ] 4.1 `alerts` source type built on the browser source (canvas size, transparent, audio on)
- [ ] 4.2 Bundled alerts page: queue, per-kind animation, sound, text template, layout, duration
- [ ] 4.3 Inspector: per-kind settings, minimum amount, Test button
- [ ] 4.4 Default animations and sounds (from the project owner) or empty kinds

## 5. Verification
- [ ] 5.1 `cargo test --workspace`, `cargo clippy --workspace --all-targets`
- [ ] 5.2 Test alerts for every kind on a 1080p canvas: transparent, sound on the source's fader, one at a time
- [ ] 5.3 Live check with a real Twitch follow and a Kick follow
