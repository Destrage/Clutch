# Design: Video filters

## Model
`SourceDef.filters: Vec<FilterDef>`. `FilterDef` is the existing `{ id, kind, enabled, settings }`
shape; `AudioFilterDef` becomes an alias of it. The field is omitted from JSON when empty, so old
collections are unchanged.

`Command::SetSourceFilters { source, filters }` replaces the chain and is undoable. Its coalesce
key is per source, so dragging a slider is one undo step. The engine validates custom shader code
(`settings.code`) before applying.

## Catalog (`estream-gpu::filter::KINDS`)
Each kind has numeric params (key, label, min, max, step, default, unit), an optional file
(`settings.file` with an extension filter) and an optional `code` flag. The UI and validation are
generic over the catalog, as for audio filters.

| kind | params | notes |
|---|---|---|
| `color_correction` | brightness, contrast, saturation, gamma, hue_shift, opacity | perceptual (sRGB-encoded) space |
| `lut` | strength + `.cube` file | 3D LUT on sRGB-encoded values, trilinear |
| `chroma_key` | hue, similarity, smoothness, spill | distance in CbCr from the key color |
| `luma_key` | low, high, smoothness | |
| `crop_pad` | left, top, right, bottom (px) | negative = pad; output size changes |
| `blur` | radius (px) | two passes (horizontal, vertical), Gaussian, linear light |
| `sharpen` | amount | unsharp mask |
| `image_mask` | use_luminance + image file | mask stretched to the source |
| `custom_shader` | p1..p4 + code | user WGSL, see below |

## GPU (`estream-gpu::filter`)
Bind groups:
- **group 0:** uniform `{ in_size, out_size, offset, time, kind, params[12] }`;
- **group 1:** the input, laid out exactly like the compositor's source binding (texture +
  sampler). This lets a pass read a source's own bind group or a previous pass output, and lets
  the compositor draw the final output;
- **group 2:** auxiliary inputs: a 3D LUT texture, a 2D mask texture and a sampler, with 1×1
  dummies when a filter doesn't use them.

Colors are straight alpha, linear light, as the compositor expects. Built-in kinds share one uber
shader with a full-screen triangle. Blur records two passes.

**Custom filter:** a prelude provides the bindings and helpers (`source(uv)`, `size()`, `time()`,
`param(i)`, `to_srgb`, `to_linear`), then the user's `fn effect(uv: vec2<f32>) -> vec4<f32>`, then
an epilogue. It uses the same naga validation and line mapping as custom transitions (shared
helper), and falls back to pass-through if compiling fails.

## Engine (`VideoCore`)
- `chains: HashMap<SourceId, FilterChain>`, rebuilt in `sync` when a source's enabled filter list
  changes. Parameter-only changes rewrite uniforms without reallocating.
- Each frame, after `tick_sources`, every source with a chain and a frame records its passes into
  ping-pong targets (`Rgba16Float`, reallocated only when the output size changes). `push_items`
  uses the chain output's binding and size instead of the raw frame.
- LUT and mask files are read and decoded on a helper thread. The result is uploaded on the next
  frame. Until then, and on failure (which is logged), the pass is skipped.

## UI
A Filters section in the inspector, below the source properties, for the selected item's source:
- "Add filter" select;
- rows with enable toggle, name, move up/down and remove;
- sliders for numeric params;
- a Browse button and file name for LUT and mask;
- "Edit shader…" for custom filters, opening the shader dialog in filter mode with an example.

## Testing
- **GPU:** pixel tests per kind on a known input:
  - color correction moves values as expected;
  - chroma key removes green and keeps red;
  - luma key;
  - crop/pad output size and placement;
  - blur spreads a single bright column symmetrically;
  - LUT identity and inverted LUTs;
  - image mask;
  - a custom filter runs;
  - a broken custom filter names the line.
- **Parser:** `.cube` size and values, plus errors.
- **Engine:** a source with a chroma key filter on a canvas renders keyed; the command is undoable;
  disabling a filter restores the original.
- **In-app:** add a color correction and a blur to the test pattern.
