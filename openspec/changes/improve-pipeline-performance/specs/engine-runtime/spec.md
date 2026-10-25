## ADDED Requirements

### Requirement: GPU frames for hardware encoders
On Windows, the engine SHALL deliver canvas frames to hardware video encoders as GPU textures,
ordered by GPU fences, without copying frame data through CPU memory. Software encoders SHALL
receive CPU frames as before.

#### Scenario: Recording with NVENC
- **WHEN** a recording uses an NVENC encoder on Windows
- **THEN** its frames reach the encoder as D3D11 textures and no NV12 readback runs for that canvas

#### Scenario: Software encoder
- **WHEN** an output uses a software encoder
- **THEN** it receives NV12 frames in CPU memory

### Requirement: GPU encode fallback
If the GPU frame path cannot be set up for a canvas or the encoder fails to open with GPU
frames, the output SHALL switch to CPU frames for that session before writing anything and log
the reason. A GPU-fed encoder that stops receiving GPU frames SHALL end its session with an
error instead of writing a frozen picture.

#### Scenario: GPU frames rejected
- **WHEN** an encoder fails to open with GPU frames
- **THEN** the session records from CPU frames and the log names the cause

#### Scenario: Planes unavailable
- **WHEN** GPU planes cannot be created for a canvas
- **THEN** every consumer of that canvas receives CPU frames

### Requirement: Preview rendered on demand
The engine SHALL render a canvas's UI preview only while a UI view shows it, and no more often
than the display's refresh rate. Encoded output SHALL NOT depend on whether a preview is
rendered.

#### Scenario: Window minimized while recording
- **WHEN** the window is minimized during a recording
- **THEN** no preview conversion runs and the recording keeps its full frame rate

### Requirement: Single-source pass-through
When a canvas's program scene is one visible, opaque, unfiltered source that covers the whole
canvas and no transition is running, the engine SHALL produce the canvas's outputs from that
source without compositing it first. The result SHALL match the composited frame within the
golden-image tolerance.

#### Scenario: Fullscreen game capture only
- **WHEN** the program scene shows just a full-canvas game capture
- **THEN** frames are converted straight from the capture texture

#### Scenario: Overlay added
- **WHEN** an overlay is made visible on that scene
- **THEN** the next frame is fully composited

### Requirement: Video stage timings
The engine SHALL measure the time each video-thread stage takes per frame and publish rolling
averages and 95th percentiles to the stats panel, without allocating on the video thread.

#### Scenario: Stats panel
- **WHEN** the stats panel is open
- **THEN** it shows average and p95 time for sources, filters, render, convert, submit, poll and deliver, and for the whole frame
