# Design: Studio mode and transitions

## Model (`estream-project`)
```rust
pub struct Transition { pub kind: TransitionKind, pub duration_ms: u32 }
pub enum TransitionKind {
    Cut,
    Fade,
    FadeToColor { color: Rgba8 },
    Slide { direction: Direction },   // both scenes move
    Swipe { direction: Direction },   // new scene moves in over the old one
    Wipe { direction: Direction, softness: f32 },
    Shader { code: String },          // user WGSL, see below
}
Project.transition: Transition       // default: Fade, 300 ms (serde default for old files)
```
`Command::SetTransition { transition }` is undoable. A `Shader` code is validated before it is
applied (see GPU).

## View state (engine, not saved)
The control thread keeps runtime view state next to the project:
- `preview: HashMap<CanvasId, SceneId>`: the studio-mode Preview scene per canvas.
- `transitions: HashMap<CanvasId, ActiveTransition { from: Option<SceneId>, transition, start }>`.

New engine requests:
- `transition_to(canvas, scene, Option<Transition>)`: applies `SetActiveScene` (as before) and
  records an `ActiveTransition` that starts at `clock.now()`. It uses the project transition when
  none is given. Cut, or a duration of 0, records nothing.
- `set_preview_scene(canvas, Option<SceneId>)`: turns the Preview on or off.

`RenderGraph::build(project, generation, view)`:
- The program items stay in `items`.
- `transition: Option<TransitionGraph { from: Vec<FlatItem>, kind, start, duration }>`.
- `preview: Option<Vec<FlatItem>>`.

The graph has its own generation counter, so view changes also reach the video and audio threads.

The audio graph mixes the union of the from- and to-scene sources while a transition runs. The
control thread wakes when the transition ends, using the same deadline loop as autosave, and
rebuilds the graph without it.

## Rendering (video thread)
For a canvas with a transition and `start <= t < start + duration`, where `t` is the frame
timestamp:
1. Render the from-items into scratch A and the program items into scratch B, using separate
   `CanvasRenderer`s because each owns a uniform buffer that `write_buffer` would overwrite
   within one submission.
2. A `TransitionRenderer` pass samples A and B and writes the canvas slot texture. Progress is
   `(t - start) / duration`; Slide, Swipe and Wipe ease in and out, Fade is linear.
3. The sRGB preview and NV12 readback continue as before, so encoders see the transition.

Scratch textures (2 × `Rgba16Float` at canvas size) are allocated on the first transition and
kept.

**Studio Preview:** when the graph has `preview` items, the canvas also renders them into a
second `CanvasOutput` (its own ring and sRGB preview textures) with its own renderer. It is
published as `Engine::preview_output(canvas)` and never feeds encoders. Its resources are dropped
when studio mode is turned off.

All blending happens in linear light on premultiplied colors, like the compositor.

## GPU (`estream-gpu::transition`)
- One built-in WGSL "uber" shader. A full-screen triangle samples `from` and `to` and switches on
  a `kind` uniform (`progress`, `direction`, `color`, `softness`). Output is `CANVAS_FORMAT` with
  no blending.
- **Custom shader:** the user writes
  ```wgsl
  fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32>
  ```
  and may call `from_color(uv)`, `to_color(uv)`, `resolution()`. The code is placed between a
  fixed prelude (bindings and helpers) and an epilogue (vertex shader and `fs_main` that calls
  `transition`).
- `validate_custom_transition(code) -> Result<(), String>` parses and validates the assembled
  module with naga. Error positions are mapped back to the user's lines. The engine calls it in
  `validate()` for `SetTransition`, so a broken shader is rejected with a readable message
  before it is applied.
- The video thread compiles custom pipelines on sync and caches them by a hash of the code. If
  compiling fails anyway, it falls back to Fade and logs the error.

## UI
- **Studio mode** is off by default and toggled from a top-bar button and the palette.
  - When on, the center area shows **Preview** (editable, left) and **Program** (right), with a
    narrow column between them:
    - a Transition button;
    - quick Cut / Fade buttons;
    - the transition picker and duration.
  - Scene clicks set the Preview. The scene list marks the program scene as live.
  - All editing (selection, handles, adding sources) acts on the Preview scene.
- **Outside studio mode**, a compact transition bar at the bottom of the Scenes panel holds the
  type and duration. Scene clicks transition with it.
- Choosing **Custom shader…** opens an editor dialog with a text area seeded with an example,
  Apply, and the compiler error shown inline.
- Palette actions:
  - Toggle studio mode (Ctrl+Shift+S).
  - Transition to program (Enter in studio mode). This is not a global Enter, to avoid
    accidental cuts; it is only active when the Preview has focus or via the button.
  - Cut to program.

## Testing
- **Model:** default transition, serde of old files without `transition`, `SetTransition` undo.
- **GPU:**
  - Pixel tests with solid red → solid blue scenes rendered headless at fixed timestamps:
    - Fade at 50 % is the linear midpoint;
    - Fade to black at 50 % is black;
    - Slide at 50 % has half of each scene;
    - Wipe moves its edge with progress.
  - The custom shader compiles and runs.
  - A broken custom shader produces an error that names the line.
- **Engine:**
  - `transition_to` sets the program immediately.
  - Frames before the end blend the two scenes, and frames after the end show only the new
    scene.
  - The audio graph holds both scenes' sources during the transition and only the new one after.
- **Studio:** the preview output renders the preview scene while the program output keeps the
  program scene.
- **In-app:** studio mode with two scenes, a Fade transition captured mid-way, and the
  custom-shader error display.
