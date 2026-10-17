# Change: Video filters

## Why
Sources come in as they are. Creators need to fix colors, key out green screens, apply looks
(LUTs), crop or pad, blur, sharpen and mask, as OBS filters do. eStream runs all of this on the
GPU in its linear-light pipeline, and makes custom filters a short WGSL function instead of a
plugin.

## What Changes
- **video-filters** capability (new):
  - An ordered chain of filters per source: add, remove, reorder, enable or disable, with
    parameters. It applies wherever the source appears.
  - Built-in filters:
    - Color correction (brightness, contrast, saturation, gamma, hue shift, opacity);
    - LUT (`.cube` file, strength);
    - Chroma key (key hue, similarity, smoothness, spill);
    - Luma key;
    - Crop / Pad (negative values pad, which changes the source size);
    - Blur (separable Gaussian, radius);
    - Sharpen;
    - Image mask (alpha or luminance of an image file).
  - **Custom shader filter:** a user WGSL function `effect(uv) -> vec4<f32>` with
    `source(uv)`, `size()`, `time()` and four slider parameters. It is validated with readable,
    line-numbered errors (shared with custom transitions).
- Engine:
  - Filter chains run on the video thread after sources tick, through ping-pong render targets.
  - The compositor draws the chain output.
  - LUT and mask files load on a helper thread; the filter passes the image through until loaded.
  - `Command::SetSourceFilters` is undoable, and slider drags coalesce into one step.
- Studio: a Filters section in the inspector for the selected source, with a filter picker,
  parameter sliders, file browse for LUT and mask, and the shader editor for custom filters.

## Impact
- Affected specs: `video-filters` (new)
- Affected code:
  - `estream-project` (filter defs on sources);
  - `estream-engine` (command, filter chains in `VideoCore`);
  - `estream-gpu` (filter renderer, shaders, `.cube` parser, validation);
  - `apps/estream` (inspector UI).
