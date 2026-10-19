# Tasks: add-voice-effects

## 1. DSP
- [x] 1.1 Pitch shift (overlapping-grain delay lines, preallocated; semitones and mix)
- [x] 1.2 Robot (ring modulation with tone)
- [x] 1.3 Radio (band-pass biquads and soft clipping)
- [x] 1.4 Echo (delay line with feedback, mix)
- [x] 1.5 Reverb (Freeverb-style comb and all-pass network; size, damping, mix)
- [x] 1.6 Tests: pitch shift moves a sine's frequency by the right ratio; filters keep silence
      silent and stay bounded; no allocation in `process`

## 2. Studio
- [x] 2.1 "Voice" presets in the audio settings (Off, Deeper, Higher, Robot, Radio, Cave, Echo)
- [x] 2.2 Hear yourself: the "Hear on my speakers" toggle above the presets, with a hint

## 3. Check
- [ ] 3.1 Clippy; in-app check of each preset on the microphone with Hear yourself
