# Tasks: add-remote-api

## 1. Protocol and server (`crates/estream-api`)
- [x] 1.1 Protocol constants (ops, close codes, status codes, event bits), JSON/MessagePack, auth
- [x] 1.2 Server: tokio listener, per-client sessions, Hello/Identify/Reidentify, close codes
- [x] 1.3 Requests and batches routed to the app thread; `Sleep`; broadcast with subscriptions

## 2. Requests
- [x] 2.1 General, stats, studio mode, scenes, program/preview
- [x] 2.2 Inputs (list, kinds, create/remove/rename, settings, audio properties)
- [x] 2.3 Transitions
- [x] 2.4 Scene items (list, id, source, create/remove, enabled, locked, index, transform)
- [x] 2.5 Outputs (stream, record, replay buffer), media inputs, filters

## 3. Events
- [x] 3.1 Snapshot diff → scene, input, item, transition and filter events
- [x] 3.2 App state polling → studio, preview, outputs; volume meters; ExitStarted

## 4. Studio
- [x] 4.1 `RemoteSettings` (encrypted password), Settings "Remote control" section with status
- [x] 4.2 App implements `Studio`; server lifecycle (start, restart on change, shutdown)

## 5. Verification
- [x] 5.1 Protocol, handler and event unit tests
- [x] 5.2 End-to-end WebSocket test (auth, requests, batch, events, MessagePack)
- [x] 5.3 In-app check driven by a scripted client
