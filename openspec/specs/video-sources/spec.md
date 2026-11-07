# video-sources Specification

## Purpose
Capture sources (display, window, game capture, camera) and how they behave, including game capture sound and window latching on Windows.

## Requirements

### Requirement: Asking about game capture sound
A new Game capture SHALL start with "Capture audio" off. When it is added while a Desktop audio or
Application audio source exists, the app SHALL ask whether to capture the game's sound, offering
to remember the answer; with no such source, the capture's audio SHALL be turned on without
asking. Window capture and existing sources SHALL keep their setting.

#### Scenario: Desktop audio already present
- **WHEN** the user adds a Game capture and Desktop audio is in the collection
- **THEN** the app asks whether to capture the game's sound, and the capture's audio follows the
  answer

#### Scenario: Remembered answer
- **WHEN** the user ticks "Don't ask again" and answers
- **THEN** later Game captures follow that answer without asking, until it is changed in Settings

#### Scenario: No other sound source
- **WHEN** the user adds a Game capture and no Desktop or Application audio source exists
- **THEN** the capture's audio is on and no question is asked

### Requirement: Game capture window latching
In automatic fullscreen mode, Game capture SHALL latch onto an acquired game window and remain
attached while that window exists and its process is running. Focus shifts, alt-tabbing, and
temporary minimization SHALL NOT switch capture to another application.

#### Scenario: User alt-tabs during gameplay
- **WHEN** Game capture is capturing a fullscreen game and the user alt-tabs to another window
- **THEN** Game capture remains attached to the game window and does not capture the foreground app

#### Scenario: Game process exits
- **WHEN** the latched game process terminates
- **THEN** Game capture releases the latch and waits for the next foreground fullscreen application

### Requirement: Windows process priority
On Windows systems, the application SHALL configure its process priority to Above Normal on
startup to prevent the operating system scheduler from starving compositing workloads.

#### Scenario: Process startup on Windows
- **WHEN** the application starts on Windows
- **THEN** the process priority class is set to Above Normal without requiring elevated privileges

### Requirement: Color Source Gradients
The Color source SHALL have a Style setting with the choices Solid, Linear gradient and Radial
gradient. A gradient SHALL have 2 to 8 color stops, each with a color, opacity and position; a
linear gradient SHALL have an angle and a radial gradient a center and radius. A Color source
saved by an earlier version SHALL show its solid color unchanged.

#### Scenario: Linear gradient background
- **WHEN** the user sets a Color source's Style to Linear gradient with stops orange at 0% and
  pink at 100% and an angle of 90°
- **THEN** the source fades from orange on the left to pink on the right

#### Scenario: Adding a stop
- **WHEN** the user clicks the gradient bar halfway between two stops
- **THEN** a third stop is added there with the color the gradient has at that point

#### Scenario: Old Color source
- **WHEN** a scene collection saved before this change has a red Color source
- **THEN** it opens with Style Solid and shows the same red

### Requirement: Gradient Rendering
Gradients SHALL blend between stops in linear light, SHALL be rendered once when their settings
change rather than every frame, and SHALL show no visible banding in streams and recordings.

#### Scenario: Smooth dark gradient
- **WHEN** a 1080 × 1920 Color source fades from near black to dark blue and is recorded
- **THEN** the recording shows a smooth fade without visible steps

#### Scenario: No per-frame cost
- **WHEN** a gradient Color source is on screen and its settings don't change
- **THEN** the gradient is not redrawn each frame

### Requirement: Warn About Sound Captured Twice
The mixer SHALL warn on a source's strip when its sound reaches the same canvas mix twice:
when two Application audio sources capture the same application, or when Desktop audio carries
an application that another source also captures because Desktop audio is leaving out a
different program. The warning SHALL name the programs and say how to fix it.

#### Scenario: Game capture, Discord and Desktop audio
- **WHEN** a game capture captures Apex's audio, an Application audio source captures Discord
  and Desktop audio is on
- **THEN** the Desktop audio strip warns that it also carries Discord because it leaves out
  Apex, and suggests muting Desktop audio or removing the Discord source

#### Scenario: Two sources on the same app
- **WHEN** two Application audio sources capturing Discord.exe are in one canvas's mix
- **THEN** both strips warn that Discord is captured twice
