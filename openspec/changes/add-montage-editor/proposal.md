# Change: Montage editor (highlight scan, auto-montage, timeline editing)

## Why
After `add-clips`, users end up with a library of tagged clips and long recordings with event
markers. The next step for most of them is a montage: a short highlight video set to music, often
vertical for Shorts or TikTok. Today that means exporting everything into a separate editor.

Medal and similar apps showed that a **one-click montage** plus a **simple editor** covers most of
what people want, without a full NLE. eStream already decodes, composites on the GPU and encodes,
which are the three hard parts. This change puts them together for editing.

## What Changes
- **montage** capability (new):
  - **Highlight scan:** finds highlight segments in a full recording using:
    - its marker file (game events from `add-clips`);
    - the audio hype detector, run offline on the recording's audio.

    Each candidate segment has a score and a reason. The user reviews the candidates and picks
    which ones to keep.
  - **Auto-montage:**
    - the inputs are clips from the library and/or highlights from recordings, plus optional
      music;
    - the options are a target length (e.g. 60 s), format (16:9, 9:16, 1:1) and order
      (chronological or best first);
    - it builds a montage project:
      - each segment is trimmed around its event;
      - cuts snap to the music's beats;
      - default transitions are added, with an optional title card;
      - clip audio is ducked under the music.
  - **Timeline editor** (one video track, one music track, and text):
    - edit: reorder (drag), trim handles, split at playhead, delete, speed 0.25×–2×
      (including slow motion around an event);
    - transitions: cut, crossfade, dip to black, whip/zoom;
    - text titles: preset styles, position and timing;
    - framing: per-segment crop/pan for vertical output;
    - audio: clip volume, music volume with fades, and automatic ducking;
    - navigation: zoom, snapping to cuts, events and beats, and keyboard shortcuts;
    - full undo/redo.
  - **Preview:** real-time playback of the timeline with the same renderer that exports, so what
    you see is what you get.
  - **Export:** MP4 (H.264/HEVC/AV1) with the existing encoders.
    - presets: YouTube 1080p60, Shorts/TikTok 1080×1920, and Source;
    - export runs in the background with progress and cancel;
    - the result can be added to the clip library.
  - **Montage projects** are saved as `.estream-montage` JSON files and reopened from a
    Montages list. Missing source files are reported, and the user can relink them.
- **Project context:** the non-goal "Video editing / NLE features beyond simple trimming of
  recordings" is narrowed to "Multi-track NLE features (multiple video tracks, keyframed
  effects, color grading) beyond the single-track montage editor". The two new changes are
  added to the roadmap.

## Out of scope
- More than one video track, picture-in-picture, keyframed effects, color grading, LUT
  per segment.
- A built-in music library (licensing). The user brings their own audio files.
- Auto-captions on montages. That depends on `add-ai-features`.
- Uploading to platforms.

## Impact
- Affected specs: `montage` (new)
- Affected code:
  - new `crates/estream-montage`:
    - project model and commands with undo;
    - highlight scan;
    - beat detection;
    - auto-assembly;
    - timeline renderer;
    - export;
  - `estream-media`: frame-accurate seeking decoder and offline audio decode, reusing the
    `player` code;
  - `estream-gpu`: transition and crop shaders;
  - `apps/estream`: Montages view, timeline widget, export dialog;
  - `openspec/project.md`: non-goal and roadmap.
- Depends on `add-clips` (library, markers, hype detector).
