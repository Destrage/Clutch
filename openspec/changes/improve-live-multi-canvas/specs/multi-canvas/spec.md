## MODIFIED Requirements

### Requirement: Audio Across Canvases
Each canvas SHALL have its own audio mix holding the global audio sources and the sources
visible in that canvas's active scene (and in the scene being left while a transition runs).
Every output SHALL encode the mix of the canvas it outputs. A source shown on several canvases
SHALL be read and filtered once and added to each of those mixes once. The mixer SHALL show
the sources and master level of the canvas being edited.

#### Scenario: Camera on two canvases
- **WHEN** the same camera source is visible on the horizontal and the vertical canvas
- **THEN** its audio is heard once in each canvas's mix

#### Scenario: Source only in one canvas's scene
- **WHEN** an application audio source is only in "Gameplay", the 1080p canvas is on
  "Be right back" and the 1440p canvas is on "Gameplay"
- **THEN** the streams of the 1080p canvas don't carry that audio, the streams of the 1440p
  canvas do, and the mixer doesn't list it while the 1080p canvas is being edited

## ADDED Requirements

### Requirement: Scenes Switch Together
A scene switch on one canvas SHALL also switch every other canvas that follows scene switches
and has a scene with the same name, using the same transition. Canvases SHALL follow by
default, and the user SHALL be able to turn this off per canvas. This SHALL apply to every way
of switching: scene cards, the Live deck, studio mode, hotkeys, automation and the remote API.

#### Scenario: Duplicated canvas follows
- **WHEN** the 1080p canvas and its 1440p duplicate are live and the user picks
  "Be right back"
- **THEN** both canvases fade to their "Be right back" scenes

#### Scenario: No matching scene
- **WHEN** a vertical canvas has no scene named "Be right back" and the user switches to it
  on the 1080p canvas
- **THEN** the vertical canvas keeps its current scene

#### Scenario: Canvas switching on its own
- **WHEN** the user turns off "Switch scenes with other canvases" on the vertical canvas
- **THEN** scene switches on other canvases no longer change it, and switches on it change
  only it

### Requirement: Canvas Per Destination In Go Live
With more than one canvas, the Go live window SHALL let the user pick the canvas of each
destination, save the choice to that destination, and keep a choice that sets every
destination at once.

#### Scenario: Split by platform
- **WHEN** the user sets Kick and Twitch to "Main" and YouTube to "Main 2560 × 1440" in the
  Go live window and goes live
- **THEN** Kick and Twitch stream 1920 × 1080, YouTube streams 2560 × 1440, and the window
  shows the same choices next time
