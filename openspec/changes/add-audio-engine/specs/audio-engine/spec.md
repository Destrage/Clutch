## ADDED Requirements

### Requirement: Audio Format and Block Processing
The audio engine SHALL mix all audio at 48 kHz, stereo, 32-bit float, in 10 ms blocks, timestamped
with the engine clock, and SHALL NOT allocate memory or block on locks on the audio thread in steady state.

#### Scenario: Steady block rate
- **WHEN** the engine runs for 10 seconds
- **THEN** 1000 ± 1 audio blocks of 480 frames are produced

### Requirement: Audio Capture Sources
The system SHALL capture audio from input devices, from output devices via loopback (desktop
audio), and on supported Windows versions from individual applications. Devices running at other
sample rates SHALL be resampled to 48 kHz.

#### Scenario: Default devices
- **WHEN** a new scene collection is created
- **THEN** it contains global "Desktop Audio" and "Microphone" sources using the default devices

#### Scenario: Device unplugged
- **WHEN** a capture device is disconnected
- **THEN** the source reports itself unavailable, contributes silence, and resumes when the device returns

#### Scenario: Application audio
- **WHEN** a source captures the application `game.exe`
- **THEN** only that application's audio is captured, and capture resumes automatically if the application restarts

### Requirement: Per-Source Mixing Controls
Each audio source SHALL support volume in dB, mute, balance, mono downmix, a positive sync offset,
monitoring mode, and routing to any combination of 6 tracks. All of these SHALL be undoable.

#### Scenario: Volume in dB
- **WHEN** a 0 dBFS sine source is set to -6 dB
- **THEN** its post-fader peak meter reads -6 dB ± 0.1 dB

#### Scenario: Mute
- **WHEN** a source is muted
- **THEN** it contributes silence to all tracks and its meter reads silence

#### Scenario: Track routing
- **WHEN** a source is routed to tracks 1 and 3 only
- **THEN** its signal appears in the mixes for tracks 1 and 3 and not in tracks 2, 4, 5 or 6

### Requirement: Active Audio Sources
Global audio sources SHALL always be mixed. Other sources SHALL be mixed while a visible item that
references them is in the active scene. Each source SHALL be mixed at most once per block.

#### Scenario: Scene switch
- **WHEN** the active scene changes to a scene without the music source
- **THEN** the music source no longer contributes to the mix

### Requirement: Meters
The system SHALL provide post-fader peak and RMS levels per channel for every active source and
for the master mix, updated at least 30 times per second.

#### Scenario: Silence
- **WHEN** a source produces digital silence
- **THEN** its meters report -inf dB (displayed as the bottom of the meter)

### Requirement: Audio Filters
Sources SHALL support an ordered, individually toggleable chain of filters: gain, noise gate,
compressor, limiter and noise suppression.

#### Scenario: Limiter
- **WHEN** a limiter with a -6 dB threshold processes a 0 dBFS signal
- **THEN** the output peak does not exceed -6 dBFS + 0.5 dB after the attack time

#### Scenario: Noise gate
- **WHEN** the input level stays below the close threshold for longer than the hold and release times
- **THEN** the gate output is silent

### Requirement: Monitoring
The system SHALL play the mix of sources whose monitoring mode is "monitor only" or "monitor and
output" on a selectable output device. "Monitor only" sources SHALL NOT be included in the output tracks.

#### Scenario: Monitor only
- **WHEN** a source is set to "monitor only"
- **THEN** it is audible on the monitoring device and absent from all tracks
