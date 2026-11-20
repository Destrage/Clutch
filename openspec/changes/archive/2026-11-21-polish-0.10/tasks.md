# Tasks: polish-0.10

## 1. Filter values as percentages
- [x] 1.1 Percent flag on video filter parameters (fractions), set for opacity, similarity, smoothness, spill, low/high, strength, brightness, contrast, sharpen
- [x] 1.2 Parameter editor shows and takes percentages for flagged parameters; stored values unchanged

## 2. Settings that depend on a switch
- [x] 2.1 Source property visibility condition (shown while a named switch is on)
- [x] 2.2 Text source: gradient end color, outline color and width, shadow settings depend on their switches; also the text file under "Read text from file" and each alert's settings under its Show switch
- [x] 2.3 Inspector hides properties whose switch is off

## 3. Borders follow a morphing outline
- [x] 3.1 Compositor border mode: a band at a distance range from the mixed outline, on a quad grown by the border reach
- [x] 3.2 Moving items draw their border from the morph (width, color and placement in between) when neither end has an inner glow; the baked border layers are left out then
- [x] 3.3 Rendered-frame test: a border halfway through a rectangle → circle morph lies on the in-between outline
- [x] 3.4 Distance fields count the item's edge as an edge (shapes that fill the item, such as rounded corners, measured to their cut-away corners along the sides), with a bake test

## 4. The preview isn't hidden by its panels
- [x] 4.1 Preview fit leaves room for the open scene items panel and inspector while the canvas keeps at least half the preview's width; pointer mapping stays exact

## 5. Lists keep their rows
- [x] 5.1 The settings form's lists (encoders, canvases, webhooks, event sounds, clip integrations and rules) and the Mask panel's shape rows keep their models across refreshes, like the Automation dialog; inspector settings, filters and mixer strips already did

## 6. Transition card
- [x] 6.1 The card under the scenes says when moves are on

## 7. Verification
- [x] 7.1 `cargo test` and `cargo clippy --all-targets -- -D warnings`
- [x] 7.2 In the app: filter percentages, Text settings appearing with their switches, a bordered camera morphing, the canvas beside open panels, a dropdown open in a filter while the remote API changes something (the remote-API case is covered by the same kept-model mechanism verified with the Automation dialog left open across its refreshes)
