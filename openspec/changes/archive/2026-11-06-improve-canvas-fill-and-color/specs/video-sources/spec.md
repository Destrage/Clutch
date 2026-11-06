## ADDED Requirements

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
