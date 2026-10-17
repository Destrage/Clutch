# Tasks: add-multistream

## 1. Media (`estream-media`)
- [x] 1.1 `DestinationStats` and per-destination stop flag in the sender
- [x] 1.2 Session network target as a list of destinations; OR-ed forced keyframes; packets
      shared by reference (`av_packet_ref`), not copied
- [x] 1.3 `Control::AddDestination` for joining a running encoder session
- [x] 1.4 `MultiStream`: grouping by encoder settings, per-destination validation, stop/start one
      destination, retiring empty groups, background stop

## 2. Studio
- [x] 2.1 Destinations in settings (per-destination encrypted key) + migration from single stream
- [x] 2.2 Settings → Stream: destination list (add / remove / rename / enable) + editor
- [x] 2.3 Go Live with aggregate state; live panel with per-destination Stop / Start
- [x] 2.4 Status bar summary per destination

## 3. Verification
- [x] 3.1 Shared encoder: two destinations, one encoder, both servers receive
- [x] 3.2 Different settings → separate encoders
- [x] 3.3 One destination fails, the other continues
- [x] 3.4 Stop one destination while live; add one while live (joins the running encoder,
      starts at a keyframe)
- [x] 3.5 Settings migration test
- [x] 3.6 In-app check with two local servers: shared encoder, stop one from the live panel
      (received 7.9 s vs 14.2 s for the one that kept going)

## Known issues / not covered
- Removing a destination that is currently live is refused (stop it first).
- Settings changes to a live destination apply the next time it starts.
- Per-destination canvas (vertical streams) waits for multi-canvas support in the studio.
