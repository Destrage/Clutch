# performance Specification

## Purpose

How much recording, clips and streaming cost a game and the connection: performance profiles, Optimize for this PC, and stream bitrates fitted to the measured upload speed.

## Requirements

### Requirement: Upload Speed Test
The studio SHALL measure the connection's upload speed when the user runs Optimize for this
PC, opens the Go Live box to add a destination, or clicks Test again on the Performance page,
and SHALL NOT measure otherwise or while streaming. The result and its time SHALL be saved and
shown on the Performance page. If the test can't complete, the studio SHALL say so and keep its
other recommendations.

#### Scenario: Optimize measures the upload
- **WHEN** the user clicks Optimize for this PC
- **THEN** the upload speed is measured first and shown with what was found about the PC

#### Scenario: Live
- **WHEN** the user is streaming and opens the Performance page
- **THEN** Test again is unavailable and Optimize uses the last saved result

#### Scenario: Offline
- **WHEN** the test server can't be reached
- **THEN** the Performance page says the upload couldn't be tested, and the bitrates follow the
  platform table as before

### Requirement: Bandwidth-Aware Stream Bitrate
When an upload result exists, the recommended video bitrate of each enabled stream destination
SHALL fit its share of the upload. That share is the lower of 75% of the measured speed and the
measured speed minus 1.5 Mb/s, split evenly between the enabled destinations, less each
destination's audio bitrate. The bitrate SHALL NOT exceed the platform's ceiling for the
destination's canvas, and SHALL NOT go below 1.5 Mb/s. A custom server's bitrate SHALL only ever
be lowered.

#### Scenario: Fast upload to YouTube
- **WHEN** the upload measures 100 Mb/s and the only destination is YouTube at 1080p60
- **THEN** its recommended bitrate is YouTube's ceiling for 1080p60

#### Scenario: Slow upload
- **WHEN** the upload measures 5 Mb/s and the only destination is Twitch at 1080p60 with 160 kb/s audio
- **THEN** its recommended bitrate is 3340 kb/s

#### Scenario: Two destinations share the upload
- **WHEN** the upload measures 20 Mb/s and Twitch and Kick are both enabled at 1080p60
- **THEN** each gets at most half of 15 Mb/s less its audio, capped at 6 Mb/s for Twitch and
  8 Mb/s for Kick

### Requirement: Bitrate Reasons and Upload Warnings
Each recommended bitrate change SHALL say whether the upload or the platform's ceiling set it.
When a destination's share is below what its canvas needs for a clear picture, the studio SHALL
suggest a lower resolution or frame rate, and SHALL NOT change the canvas itself.

#### Scenario: Upload is the limit
- **WHEN** the upload measures 5 Mb/s and the only destination is Twitch at 1080p60
- **THEN** the bitrate change is marked as limited by the upload, the optimizer suggests 720p or
  30 fps, and the canvas stays 1080p60

#### Scenario: Network alert while live
- **WHEN** the network can't keep up with a destination streaming at 6000 kb/s that gets 4 Mb/s through
- **THEN** the alert offers 3200 kb/s for that destination, used the next time the user goes live

#### Scenario: Platform is the limit
- **WHEN** the upload measures 100 Mb/s and the only destination is Twitch
- **THEN** the bitrate change is marked as the platform maximum
