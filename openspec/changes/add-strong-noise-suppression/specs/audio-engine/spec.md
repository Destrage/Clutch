## MODIFIED Requirements

### Requirement: Audio Filters
Sources SHALL support an ordered, individually toggleable chain of filters: gain, noise gate,
compressor, limiter and noise suppression. Noise suppression SHALL offer two strengths: Light
(RNNoise) and Strong (DeepFilterNet), and an Amount from 0% to 100% that blends the suppressed
signal with the time-aligned original. A noise suppression filter saved without a strength SHALL
run Light at 100%.

#### Scenario: Limiter
- **WHEN** a limiter with a -6 dB threshold processes a 0 dBFS signal
- **THEN** the output peak does not exceed -6 dBFS + 0.5 dB after the attack time

#### Scenario: Noise gate
- **WHEN** the input level stays below the close threshold for longer than the hold and release times
- **THEN** the gate output is silent

#### Scenario: Strong suppression removes clicks
- **WHEN** a speech-like signal with keyboard-like clicks passes through noise suppression
- **THEN** Strong leaves less click energy than Light

#### Scenario: Existing filters keep their sound
- **WHEN** a collection saved before strengths existed is loaded
- **THEN** its noise suppression filters run Light at 100%

## ADDED Requirements

### Requirement: Strong Noise Suppression Runtime
Strong noise suppression SHALL run on the CPU only, on at most one thread per filter, and SHALL NOT
allocate or block on the audio thread in steady state. Its model SHALL load off the audio thread;
until it is ready, or if it fails to load, the filter SHALL run Light. It SHALL process a mono mix
of the source and output it on both channels, with a fixed delay that the filter settings show.

#### Scenario: Switching strength without a gap
- **WHEN** a mic's noise suppression is switched from Light to Strong while streaming
- **THEN** every block until the model is ready is processed by Light, and no block is silent or unprocessed

#### Scenario: Model fails to load
- **WHEN** the Strong model cannot be built
- **THEN** the filter runs Light and the failure is logged once
