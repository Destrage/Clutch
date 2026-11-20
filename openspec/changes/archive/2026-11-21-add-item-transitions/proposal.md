# Change: Add item transitions (move between scenes, show and hide animations)

## Why
A scene transition treats each scene as one flat picture: a Fade mixes the two whole frames. When
the same source sits in both scenes (a face camera in the corner of "Gameplay" and large in the
middle of "Just Chatting"), viewers see two copies of the camera crossfading in two places. Users
want that camera to travel from its old place to its new one, changing size and shape on the way
(a rounded rectangle becoming a circle). They also want items that are only in one scene to leave
or arrive with their own motion instead of only taking part in the scene's crossfade, and the same
motion when an item is shown or hidden with its eye button.

## What Changes
- **Move matching items** (an option on the scene transition): when the program changes from
  scene A to scene B and a source is in both, its item moves from its place in A to its place in B
  over the transition's duration, instead of crossfading:
  - Position, size, rotation (the short way round), crop and opacity change smoothly.
  - **Shape morph**: when either end has a mask, the outline changes smoothly from one shape to
    the other (rectangle → circle, circle → rounded rectangle, any shape → no mask), and the edge
    effects (border, glow, shadow) blend from one look to the other.
  - The motion follows an ease curve: **Smooth** (ease in and out, the default) or **Spring**
    (settles with a small overshoot), with an optional **curved path** so the item travels on an
    arc rather than a straight line.
  - The rest of the scene still uses the chosen transition (Fade, Slide, Wipe, …); moving items
    are drawn on top of it, so they stay whole and sharp during the whole move.
  - Items are matched by source within the same canvas. When a source appears more than once in a
    scene, items pair up in list order. An item can opt out ("Move between scenes" off) and then
    takes part in the scene transition like any other item.
  - Works for program switches, studio mode Take, hotkeys, macros and the remote API, in every
    output (preview, stream, recording, virtual camera), because it is part of the program render.
- **Show and hide animations per item**: an item can have an entry and an exit animation:
  None (default), Fade, Zoom, Slide (left, right, up, down; from or to the canvas edge),
  Pop (zoom with a small overshoot) and Wipe; each with a duration and the same ease curves.
  - They run when the item's visibility is toggled (eye button, hotkey, macro, remote API).
  - During a scene transition, an item that is only in the scene being left plays its exit
    animation, and an item only in the new scene plays its entry animation, when it has one;
    without one it takes part in the scene transition as today.
- **Settings**: the scene transition settings get "Move matching items" with the ease curve and
  curved path; the item inspector gets a **Transitions** panel (Show, Hide, duration, ease, and
  "Move between scenes").
- Every setting is a command (undo/redo, remote control) and is saved with the scene collection.
  Collections saved before this change load with moving off and no item animations, so existing
  scenes look exactly as before until the user turns the option on.

Not in this change: per-item keyframe animation or motion paths drawn by hand, moving items
between different canvases, matching different sources that look alike, animated filters, and
item animations triggered by stream events (alerts already cover those).

## Impact
- Affected specs: `transitions` (moving items, item show/hide animations), `studio-ui`
  (transition settings, item Transitions panel)
- Affected code: `estream-project` (settings on `Transition` and `SceneItem`, persistence),
  `estream-engine` (item matching, animation state in the view state, graph, video thread
  passes), `estream-gpu` (shape morph in the compositor), `apps/estream` (settings UI, inspector
  panel)
- Older Clutch versions ignore the new fields and switch scenes with the plain transition.
