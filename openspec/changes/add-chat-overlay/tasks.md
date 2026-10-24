# Tasks: add-chat-overlay

## 1. Feed to overlay
- [ ] 1.1 Send new chat messages (with emote parts) and optional stream events to the overlay server
- [ ] 1.2 Forward removals and timeouts as `remove` / `clear_user`
- [ ] 1.3 Recent-message backlog for newly connected pages

## 2. Chat overlay source
- [ ] 2.1 `chat_overlay` source type built on the browser source (transparent)
- [ ] 2.2 Bundled chat page: styles, fading, max lines, emotes, live config updates
- [ ] 2.3 Inspector: style, font, colours, width, lines, fade time, platforms, filters, events

## 3. Verification
- [ ] 3.1 Tests: message and removal JSON, filters (commands, bot names, platforms), backlog
- [ ] 3.2 `cargo test --workspace`, `cargo clippy --workspace --all-targets`
- [ ] 3.3 Live check with Twitch and Kick chat at once: order, emotes, fading, a deleted message disappears
