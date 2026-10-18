## ADDED Requirements

### Requirement: Highlight Scan
The system SHALL find highlight candidates in a recording from its marker file (game events) and
from offline audio hype detection, merging overlapping candidates and giving each a score and
the reasons it was found. The user SHALL be able to choose which candidates to keep.

#### Scenario: Recording with markers
- **WHEN** the user scans a 20-minute recording whose marker file lists 6 kills and an ace
- **THEN** the candidates include a window around each event, and the ace scores highest

#### Scenario: Recording without markers
- **WHEN** the user scans a recording that has no marker file and contains one very loud moment
- **THEN** a candidate around that moment is found from the audio alone

### Requirement: Auto-Montage
The system SHALL assemble a montage from selected clips and highlight candidates, given a target
length, an output format (16:9, 9:16 or 1:1), an order and optional music. Each segment SHALL be
trimmed around its main event, and when music is present cuts SHALL be aligned to the music's
beats. Items SHALL be dropped, lowest score first, when they don't fit the target length. The same
inputs SHALL always produce the same montage.

#### Scenario: Fit to target length
- **WHEN** the user creates a 60 s montage from 20 clips
- **THEN** the montage lasts about 60 s and leaves out the lowest-scoring clips

#### Scenario: Beat-aligned cuts
- **WHEN** the user creates a montage with a 120 BPM music track
- **THEN** every cut lies within 0.25 s of a beat

### Requirement: Timeline Editing
The system SHALL let the user edit a montage on a timeline with one video track, one music track
and text titles: reorder, trim, split and delete segments; set segment speed between 0.25× and 2×;
choose transitions (cut, crossfade, dip to black, whip); add text titles; set per-segment crop
for the output format; and set clip and music volume, fades and ducking. Every edit SHALL be
undoable and redoable.

#### Scenario: Split and reorder
- **WHEN** the user splits a segment at the playhead and drags the second half to the start
- **THEN** the timeline plays the second half first, and undo twice restores the original

#### Scenario: Slow motion
- **WHEN** the user sets a segment's speed to 0.5×
- **THEN** the segment lasts twice as long on the timeline and its audio keeps its pitch

### Requirement: Montage Preview
The system SHALL play the montage in real time using the same renderer as export, dropping
frames rather than slowing down when it can't keep up, and SHALL show the frame at the
playhead while scrubbing.

#### Scenario: Preview matches export
- **WHEN** the user previews a crossfade and then exports the montage
- **THEN** the exported frame at the crossfade midpoint matches the preview frame within the
  golden-image tolerance

### Requirement: Montage Export
The system SHALL export a montage to MP4 with a selectable preset (YouTube 1080p60,
Shorts/TikTok 1080×1920, Source) and codec, in the background with progress and cancel. A
cancelled export SHALL leave no partial file. The user SHALL be able to add the result to the
clip library.

#### Scenario: Vertical export
- **WHEN** the user exports with the Shorts/TikTok preset
- **THEN** the file is 1080×1920, and its duration matches the timeline with audio in sync

#### Scenario: Cancel
- **WHEN** the user cancels an export halfway
- **THEN** export stops and no output file remains

### Requirement: Montage Projects
The system SHALL save montages as project files that can be reopened and edited later. When a
source file is missing, the system SHALL flag the affected segments and let the user relink
them.

#### Scenario: Moved clips
- **WHEN** the user moves the clips folder and opens a montage that uses those clips
- **THEN** the affected segments are flagged as missing, and relinking one folder restores all of
  them
