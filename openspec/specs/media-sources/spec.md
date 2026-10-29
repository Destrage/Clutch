# media-sources Specification

## Purpose
Video and audio files as sources: playback, playback controls, and video with transparency.

## Requirements

### Requirement: Media Source Playback
The system SHALL provide a media source that plays video, audio and still-image files from a
playlist. Its video SHALL appear like any source and its audio SHALL go through the mixer, with
audio and video in sync. Decoding SHALL NOT block the video or audio threads.

#### Scenario: Play a video file
- **WHEN** a media source points at a video with sound
- **THEN** the frames advance at the file's frame rate and its sound is mixed in sync

#### Scenario: Playlist with images
- **WHEN** the playlist holds an image and then a video
- **THEN** the image shows for the configured time and then the video plays

#### Scenario: End of playlist
- **WHEN** the last item ends and looping is off
- **THEN** playback stops and the last frame stays visible

#### Scenario: Missing file
- **WHEN** a playlist file does not exist
- **THEN** the source reports an error naming the file and other items still play

### Requirement: Media Controls
The user SHALL be able to pause, resume, restart and skip to the previous or next item, and see
the current item and position.

#### Scenario: Pause
- **WHEN** the user pauses a playing media source
- **THEN** the position stops advancing and the audio goes silent until resumed

### Requirement: Transparent Video
Video files with an alpha channel SHALL keep their transparency.

#### Scenario: Alpha video
- **WHEN** a media source plays a video with transparent areas
- **THEN** those areas show the scene beneath
