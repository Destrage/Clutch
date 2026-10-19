## ADDED Requirements

### Requirement: Voice Effects
Sources SHALL support voice effect filters — pitch shift, robot, radio, echo and reverb — in their
filter chain, and the audio settings SHALL offer one-click voice presets built from them.

#### Scenario: Deeper voice
- **WHEN** a pitch shift of −5 semitones processes a 440 Hz sine
- **THEN** the output's dominant frequency is within 2% of 440 × 2^(−5/12) Hz

#### Scenario: Choosing a preset
- **WHEN** the user picks the Robot preset for the microphone
- **THEN** a Voice filter group with the robot effect is added to the microphone's chain, and
  picking Off removes it again

#### Scenario: Silence stays silent
- **WHEN** any voice effect processes silence after its tail has decayed
- **THEN** the output is silent
