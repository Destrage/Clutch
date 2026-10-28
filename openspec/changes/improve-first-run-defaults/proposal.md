# Change: More first-run defaults, each one optional

## Why
The setup wizard builds a scene with the screen, game, webcam and audio, but a streamer still has
to find and add the chat overlay, alerts and a "starting soon" screen by hand, often without
knowing that Clutch has them. The wizard also adds everything it finds without asking: some
streamers don't want their webcam on screen, and there was no way to say so.

## What Changes
- **More in the first setup.** When the purpose is a live one, the wizard also adds:
  - the **Chat overlay**, in the lower left (on the break scenes too);
  - **Alerts**, over every scene;
  - **"Starting soon" and "Be right back" scenes**: a plain background with large text.
- **Every optional line is a checkbox.** "We'll set up" lists:
  - webcam, game capture, microphone, desktop audio;
  - chat overlay, alerts, break scenes.

  Each one can be unticked before Get started. Canvas, screen and encoder stay as plain lines.
- **Defaults follow the purpose.**
  - *Streaming games* and *Talking to camera*: everything on.
  - *Recording* and *Vertical video*: chat, alerts and break scenes start off but can be ticked.

  A box the user changed keeps its value when they switch purpose.
- **Without the webcam, the screen is used.** For camera-first purposes, an unticked webcam falls
  back to the screen, the same as when no webcam is found.

## Out of scope
- Picking a different camera, microphone or screen in the wizard. The inspector does that.
- Styling the break scenes (themes, countdowns). They are ordinary scenes and can be edited.

## Impact
- Affected specs: `onboarding`
- Affected code: `apps/estream` `presets.rs`, wizard state in `app.rs`, `SetupWizard` in `overlays.slint`
