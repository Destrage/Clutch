# Change: Choose the graphics card, and encode on a different one

## Why
Clutch asks for the "high performance" GPU and runs compositing, capture and encoding on it.
There is no way to change that. This hurts on:
- **Hybrid laptops** (for example a GTX 1650 Ti with Intel UHD): the screen is usually wired to
  the integrated GPU, so display capture on the discrete GPU copies every frame across, and
  Clutch competes with the game for the discrete GPU.
- **Low-end PCs:** letting the integrated GPU's encoder (QuickSync, AMD VCN) encode while the
  game keeps the discrete GPU is one of the cheapest ways to stream without losing game fps.
- **Desktops with two GPUs**, where one is meant for the game and one for streaming.

A free choice is also easy to get wrong (black display capture, extra copies that cost more
than they save), so the default stays automatic and the choice explains its trade-off.

## What Changes
- **Graphics card** on Settings → Performance: **Automatic (recommended)** or any GPU by name.
  It applies after a restart, which the row says. The chosen GPU is remembered by its name and
  vendor/device id, not by its index.
- **Smarter Automatic:** prefer the GPU that drives the main display when it is also where the
  common capture path is cheapest, unless that GPU has no usable encoder and another GPU does;
  otherwise the high-performance GPU, as now. The choice and the reason are logged and shown on
  the Performance page ("Using Intel UHD 630: it drives your main screen").
- **Warning** under the row when the chosen GPU doesn't drive the main display: "Screen capture
  copies each frame from the other GPU. Game capture is not affected."
- **Encoder on another GPU:** the encoder list also offers encoders on other GPUs (for example
  "QuickSync (Intel UHD 630)" while compositing on NVIDIA). Frames are copied to that GPU through
  a shared texture, once per output frame at the output size.
- **Missing GPU:** if the chosen GPU isn't there at startup (external GPU unplugged, driver
  failure), Clutch uses Automatic and shows a toast once.
- **Optimize for this PC** on a hybrid laptop recommends the integrated GPU's encoder when it
  has one, and lists that change like the others.

## Out of scope
- Different GPUs per canvas.
- Switching GPU without a restart.

## Impact
- Affected specs: `performance` (ADDED: Graphics Card Choice, Encoding on Another GPU).
- Affected code: `crates/estream-gpu/src/context.rs` (adapter enumeration and selection),
  `crates/estream-capture/src/windows/d3d.rs` (capture interop on the chosen adapter),
  `crates/estream-media` (encoder devices per adapter, cross-adapter shared textures),
  `apps/estream/src/performance.rs` (probe per GPU, recommendation),
  `apps/estream/src/settings.rs` and the Performance page.
- Builds on `add-performance-profiles`; complements `add-low-end-profile`.
- Shared work: branch `add-gpu-selection`.
