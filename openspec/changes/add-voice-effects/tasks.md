# Tasks: add-voice-effects

## 1. DSP
- [ ] 1.1 Pitch shift (overlapping-grain delay lines, preallocated; semitones and mix)
- [ ] 1.2 Robot (ring modulation with tone)
- [ ] 1.3 Radio (band-pass biquads and soft clipping)
- [ ] 1.4 Echo (delay line with feedback, mix)
- [ ] 1.5 Reverb (Freeverb-style comb and all-pass network; size, damping, mix)
- [ ] 1.6 Tests: pitch shift moves a sine's frequency by the right ratio; filters keep silence
      silent and stay bounded; no allocation in `process`

## 2. Studio
- [ ] 2.1 "Voice" presets in the audio settings (Off, Deeper, Higher, Robot, Radio, Cave, Echo)
- [ ] 2.2 Hear yourself toggle (monitoring for the source)

## 3. Check
- [ ] 3.1 Clippy; in-app check of each preset on the microphone with Hear yourself
