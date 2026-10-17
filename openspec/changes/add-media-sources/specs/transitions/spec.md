## ADDED Requirements

### Requirement: Stinger Transitions
The system SHALL offer a stinger transition: a video file played over the scene change, with the
program switching at a configurable transition point, and the stinger's audio mixed in.

#### Scenario: Stinger switch point
- **WHEN** a stinger transition with a 400 ms transition point runs
- **THEN** frames before 400 ms show the outgoing scene under the stinger, frames after show the
  new scene under it, and the stinger's transparent areas reveal the scenes
