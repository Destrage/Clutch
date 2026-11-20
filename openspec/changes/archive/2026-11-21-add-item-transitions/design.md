# Design: add-item-transitions

## Model (`estream-project`)
```rust
pub struct Transition {
    pub kind: TransitionKind,
    pub duration_ms: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moves: Option<MoveSettings>,      // None = matching items crossfade as today
}
pub struct MoveSettings {
    pub ease: Ease,                       // Smooth | Spring | Linear
    pub curved: bool,                     // travel on an arc
}

pub struct SceneItem {
    /* … */
    #[serde(default, skip_serializing_if = "ItemAnimations::is_default")]
    pub animations: ItemAnimations,
}
pub struct ItemAnimations {
    pub show: Option<ItemAnimation>,
    pub hide: Option<ItemAnimation>,
    pub moves: bool,                      // "Move between scenes", default true
}
pub struct ItemAnimation {
    pub kind: AnimationKind,              // Fade | Zoom | Slide{direction} | Pop | Wipe{direction}
    pub duration_ms: u32,                 // 50..=5000
    pub ease: Ease,
}
```
- Missing fields load as "no moves, no animations": old collections render exactly as before.
- Edits go through the existing commands: `moves` on the transition settings command and an
  `animations` field on the `UpdateItem` patch, so undo, coalescing and the remote API come free.

## Runtime state
Animations are view state, not document state, like running scene transitions today.

```rust
pub struct ViewState {
    pub preview: HashMap<CanvasId, SceneId>,
    pub transitions: HashMap<CanvasId, ActiveTransition>,
    pub item_animations: HashMap<ItemId, ActiveItemAnimation>,   // eye-button show/hide
}
pub struct ActiveItemAnimation { pub showing: bool, pub anim: ItemAnimation, pub start: Timestamp }
```
- `UpdateItem { visible }` on an item with a matching animation records one; the graph keeps a
  hiding item in the item list until its animation ends. Toggling again mid-animation reverses
  from the current progress (the new start is back-dated) so there is no jump.
- The engine already rebuilds the graph when a transition ends; the same timer covers item
  animations.

## Matching (engine thread, when a transition starts)
`TransitionGraph` gains:
```rust
pub moving: Vec<MovingItem>,   // pairs (from FlatItem, to FlatItem)
pub leaving: Vec<(usize, ItemAnimation)>,   // index into `from`
pub arriving: Vec<(usize, ItemAnimation)>,  // index into `items`
```
- Pairs are found once per transition, in the graph build: for every source, the visible items
  that reference it in A and in B (flattened, groups resolved), paired in list order; extra items
  on either side are unpaired. Items with `moves: false` on either side are left out.
- Paired items are removed from the `from` and `items` lists used for the background pass.

## Rendering (video thread)
1. **Background pass**: as today, render `from` (minus moving items, with exit animations) and
   `items` (minus moving items, with entry animations) into the two scratch targets and blend
   them with the scene transition shader into the canvas.
2. **Moving pass**: draw the moving items on top with a load (not clear) pass of the compositor,
   in the new scene's stacking order.

The interpolation is done per frame from each end's final matrix (`parent * layout.matrix`,
computed from the live source size so bounds modes are respected):
- Decompose each `Affine2` into translation, rotation and scale; interpolate translation (on a
  quadratic arc whose control point is offset perpendicular to the path by 20 % of its length when
  `curved`), rotation the short way, and scale in log space so growing looks even.
- Crop (UV rectangle) and opacity interpolate linearly. Blend mode switches at the halfway point.
- Ease: Smooth = cubic ease-in-out; Spring = critically under-damped spring
  (`1 - e^(-6t)·cos(9t)`, normalized to land at 1); Linear.
- Stinger transitions: moving items follow the stinger's duration and draw under the stinger
  video.

### Shape morph
The baked R8 coverage masks can't be blended into a real morph (a crossfade of a square and a
circle shows both outlines). Masks also bake asynchronously at the item's on-canvas size, so they
can't be rebaked for every frame of a move.

So every masked item also gets a **signed distance field** baked with its mask: R16Float,
256 px on the long side, in the item's unit square, distances in item units, from the same
coverage (exact Euclidean distance transform, Felzenszwalb–Huttenlocher, both sides of the edge).
An unmasked item uses an analytic rectangle distance (no texture).
- The compositor gets a morph mask mode: two SDF bindings and a weight `t`; coverage is
  `smoothstep(-f, f, -mix(d_a, d_b, t))` where `f` is the interpolated feather (at least 1 px in
  item units). Invert and cut shapes are already folded into each SDF.
- Distance interpolation turns a rectangle into a circle smoothly, with no double outline.
- Edge effects: both ends' under and over layers are drawn at the interpolated transform with
  weights `1 − t` and `t`. They are soft by nature, so a crossfade reads as one changing effect.
- SDFs bake for every masked item in any scene of a canvas whose transition has moves on, not only
  for visible ones, so they are ready when a transition starts. A pair whose SDF is still baking
  crossfades its coverage masks for that transition (logged at debug level).

## Show and hide animations
Drawn as transform, opacity or UV changes of the item's own draw (Zoom and Pop scale around the
item's center; Slide moves to the nearest canvas edge or in the chosen direction until the item's
box is fully off canvas; Wipe narrows the UV rectangle and the quad together). Effect layers move
with the item. No extra passes or textures.

## Performance
- No allocation per frame: the moving and animation lists are built with the graph (engine
  thread); the video thread reuses its scratch vectors.
- The moving pass is one extra render pass only while a transition with moving items runs.
- SDF bakes are small (256² R16F ≈ 128 KB per masked item) and only on mask changes.

## Alternatives considered
- **Rebake the coverage mask each frame of a move**: the worker is asynchronous and lags one or
  more frames behind; the shape would visibly trail the item.
- **Morph polygon outlines** (resample both outlines to N points and lerp): breaks with cut shapes,
  invert and multiple shapes, which SDF interpolation handles without special cases.
- **Moves as their own transition kind**: would force the background into one look; an option
  next to every kind lets Slide or Wipe carry the rest of the scene.
