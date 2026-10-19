## ADDED Requirements

### Requirement: Recordings in the Clips View
The Clips view SHALL offer a Recordings filter that lists the video files saved directly in the
recording folder, newest first, with thumbnail, length and date, leaving out any file still being
recorded. Recordings without details SHALL get their length, a thumbnail and the events from their
marker file in the background, saved next to the recording.

#### Scenario: Recordings are listed
- **WHEN** the user has two recordings and three clips and picks Recordings in the Show filter
- **THEN** only the two recordings are shown, newest first, with their lengths

#### Scenario: Recording in progress
- **WHEN** a recording is running and the user opens the Recordings filter
- **THEN** the file being written is not listed until the recording stops

### Requirement: Recording Actions
The user SHALL be able to play, rename, favorite, reveal, copy, delete and trim a recording from
the Clips view, and save part of a recording as a new clip in the clip library. Deleting a
recording SHALL also delete its marker file.

#### Scenario: Cut a clip from a recording
- **WHEN** the user selects a recording, sets the trim to 1:00–1:20 and saves it as a new clip
- **THEN** a 20-second clip appears in the clip library with the recording's events from that span
