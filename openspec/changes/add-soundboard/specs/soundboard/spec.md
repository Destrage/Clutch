## ADDED Requirements

### Requirement: Soundboard Pads
The system SHALL offer a soundboard of named pads on the Live deck and as a studio dock. Each
pad SHALL have a name, an icon and a colour, and SHALL play a sound, show a visual effect, or
both when clicked or when its hotkey is pressed. Pads SHALL be grouped into pages, saved with
the setup, and editable without leaving the deck.

#### Scenario: Sound and visual
- **WHEN** the user clicks an "Air horn" pad with a sound and a confetti WebM set to the top of the canvas
- **THEN** the sound plays on stream and in the user's monitoring, and the confetti appears over every scene at the top for its set duration

#### Scenario: Visual only
- **WHEN** a pad has a GIF and no sound
- **THEN** clicking it shows the GIF and plays nothing

#### Scenario: Add from files
- **WHEN** the user drops three MP3 files on the soundboard grid
- **THEN** three pads are added, named after the files

### Requirement: Soundboard Audio Routing
Soundboard sounds SHALL go through one mixer source named "Soundboard", with its own volume,
mute and track routing. A pad SHALL be heard by the stream and the user, the stream only, or
the user only.

#### Scenario: Me only
- **WHEN** a pad set to "me only" is played while streaming
- **THEN** the user hears it in monitoring and the stream and recordings do not contain it

#### Scenario: Keep it off the VOD track
- **WHEN** the Soundboard source is routed away from the recording's track 2
- **THEN** track 2 of the recording contains no soundboard sounds

### Requirement: Soundboard Playback Behaviour
Pressing a pad that is already playing SHALL restart, stop or layer it, as set for that pad.
A Stop all button and hotkey SHALL stop every soundboard sound and visual at once.

#### Scenario: Stop all
- **WHEN** two pads are playing and the user presses Stop all
- **THEN** both sounds stop and their visuals leave the canvas immediately

### Requirement: Soundboard Control
Each pad SHALL be playable from its own global hotkey, from a macro step "Play soundboard
pad", and from the remote API, the same as clicking it.

#### Scenario: Stream Deck button
- **WHEN** a Stream Deck button sends the play-pad request for "Bruh"
- **THEN** the "Bruh" pad plays as if it had been clicked

### Requirement: Soundboard Sharing
A soundboard SHALL be exportable and importable as one `.clutchsound` file that holds its pads,
pages and media.

#### Scenario: Import a shared board
- **WHEN** the user imports a `.clutchsound` file from a friend
- **THEN** its pages and pads are added to the current setup and play the same sounds and visuals
