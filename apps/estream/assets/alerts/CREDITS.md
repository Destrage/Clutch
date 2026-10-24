# Built-in alert pack

## Animations

Drawn for Clutch by `make-animations.mjs` (part of Clutch, same licence): per kind an emblem
in the Clutch logo's colours that pops in with a spark burst (confetti for gifts) and a
shockwave, then a banner the alerts page writes the text on. 960×540, 30 fps, VP9 WebM with
alpha. Each kind's `alert.json` holds the banner's text box and the text timing.

To rebuild: `node make-animations.mjs <pack folder>` from a folder where `sharp` resolves
(for example `npm i --no-save sharp` in a temporary folder), with `ffmpeg` on the PATH.

## Sounds

Made with `make-sounds.sh` from these packs by Kenney (kenney.nl), released under
Creative Commons Zero (CC0 1.0, public domain):

- Music Jingles: https://kenney.nl/assets/music-jingles
- Interface Sounds: https://kenney.nl/assets/interface-sounds
- Digital Audio: https://kenney.nl/assets/digital-audio
- Casino Audio: https://kenney.nl/assets/casino-audio

The originals are layered, pitched, given a convolution reverb and levelled (follow -19,
bits -18, raid -15, the rest -16 LUFS; peaks below -1.5 dBTP), at 48 kHz stereo in Ogg Vorbis.

To rebuild: extract each pack's zip into one folder, then
`bash make-sounds.sh <that folder> <output folder>` and copy `<kind>.ogg` to `<kind>/sound.ogg`
(`follow-2.ogg` and `bits-2.ogg` to `sound-2.ogg`).
