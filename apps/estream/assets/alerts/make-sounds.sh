#!/bin/bash
# Builds Clutch's built-in alert sounds from Kenney's CC0 packs.
# usage: make-sounds.sh <downloads dir> <out dir>
set -e
DL="$1"; OUT="$2"; mkdir -p "$OUT"
J="$DL/kenney_music-jingles/Audio/Hit jingles"
D=$(dirname "$(find "$DL/kenney_digital-audio" -name powerUp7.ogg)")
I=$(dirname "$(find "$DL/kenney_interface-sounds" -name glass_004.ogg)")
C=$(dirname "$(find "$DL/kenney_casino-audio" -name chips-handle-5.ogg)")
TMP="$OUT/.tmp"; mkdir -p "$TMP"

# Stereo impulse responses: decorrelated noise with an exponential decay, darkened.
ir() { # name seconds decay
  ffmpeg -v error -y -f lavfi -i "aevalsrc=(2*random(0)-1)*exp(-$3*t)|(2*random(1)-1)*exp(-$3*t):s=48000:d=$2" \
    -af "lowpass=f=6500,highpass=f=180,volume=0.05" "$TMP/$1.wav"
}
ir room 1.2 5
ir hall 2.4 2.6

# build <out name> <ir> <wet> <tail seconds> <filter for layers> <inputs...>
# The layer filter turns inputs [0..n-1] into [dry]; reverb, limiting and loudness follow.
build() {
  local name="$1" irn="$2" wet="$3" tail="$4" layers="$5"; shift 5
  local args=() n=0
  for f in "$@"; do args+=(-i "$f"); n=$((n+1)); done
  args+=(-i "$TMP/$irn.wav")
  local pre=""
  for ((k=0; k<n; k++)); do pre+="[$k:a]aformat=sample_rates=48000:channel_layouts=stereo[s$k];"; done
  local l="${layers//\[I/[s}"
  ffmpeg -v error -y "${args[@]}" -filter_complex \
    "$pre$l;[dry]apad=pad_dur=$tail,highpass=f=35,asplit[d][w];[w][$n:a]afir=irnorm=-1[r];[d][r]amix=inputs=2:weights='1 $wet':normalize=0,stereowiden=delay=12:feedback=0.15:crossfeed=0.2:drymix=0.9,alimiter=limit=0.85:level=false[o]" \
    -map "[o]" -ar 48000 "$TMP/$name.raw.wav"
  # Trim the reverb's silent end, fade out, then two-pass linear loudness to -16 LUFS / -1.5 dBTP.
  ffmpeg -v error -y -i "$TMP/$name.raw.wav" -af "areverse,silenceremove=start_periods=1:start_threshold=-60dB,afade=t=in:d=0.12,areverse" "$TMP/$name.trim.wav"
  local lufs=-16; case "$name" in follow*) lufs=-19;; bits*) lufs=-18;; raid) lufs=-15;; esac
  local m; m=$(ffmpeg -hide_banner -i "$TMP/$name.trim.wav" -af loudnorm=I=$lufs:TP=-1.5:LRA=11:print_format=json -f null - 2>&1 | sed -n '/^{/,/^}/p')
  val() { echo "$m" | grep "\"$1\"" | sed -E 's/.*: "([^"]+)".*/\1/'; }
  ffmpeg -v error -y -i "$TMP/$name.trim.wav" -af "loudnorm=I=$lufs:TP=-1.5:LRA=11:linear=true:measured_I=$(val input_i):measured_TP=$(val input_tp):measured_LRA=$(val input_lra):measured_thresh=$(val input_thresh):offset=$(val target_offset),aresample=48000" \
    -c:a libvorbis -q:a 6 "$OUT/$name.ogg"
  printf "%-14s %ss\n" "$name" "$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$OUT/$name.ogg")"
}

# Follow: a light rising blip with a glass sparkle; two takes so follow trains don't repeat.
build follow room 0.25 0.6 "[I0][I1]amix=inputs=2:weights='1 0.4':normalize=0[dry]" \
  "$I/confirmation_002.ogg" "$I/glass_004.ogg"
build follow-2 room 0.25 0.6 "[I1]adelay=40|40[g];[I0][g]amix=inputs=2:weights='1 0.4':normalize=0[dry]" \
  "$I/confirmation_004.ogg" "$I/glass_001.ogg"

# Sub: an electronic hit with a three-tone lift and a late shimmer.
build sub hall 0.35 1.0 "[I1]volume=0.3[t];[I2]adelay=350|350,volume=0.35[g];[I0][t][g]amix=inputs=3:normalize=0[dry]" \
  "$J/jingles_HIT01.ogg" "$D/threeTone1.ogg" "$I/glass_004.ogg"

# Resub: the sub hit a major third higher, answered by a two-tone sparkle.
build resub hall 0.35 1.0 "[I0]rubberband=pitch=1.2599:formant=preserved[h];[I1]adelay=380|380,volume=0.35[t];[h][t]amix=inputs=2:normalize=0[dry]" \
  "$J/jingles_HIT01.ogg" "$D/twoTone2.ogg"

# Gift: a rising sweep into a hit, glass sparkles falling around it.
build gift hall 0.4 1.1 "[I0]volume=0.5[sw];[I1]adelay=180|180[h];[I2]asplit[ga][gb];[ga]adelay=380|380,volume=0.35[g1];[gb]adelay=560|560,rubberband=pitch=1.335,volume=0.3[g2];[sw][h][g1][g2]amix=inputs=4:normalize=0[dry]" \
  "$I/maximize_004.ogg" "$J/jingles_HIT14.ogg" "$I/glass_004.ogg"

# Bits: a bright power-up with a crystal ping; two takes.
build bits room 0.3 0.7 "[I1]adelay=120|120,volume=0.5[g];[I0][g]amix=inputs=2:normalize=0[dry]" \
  "$D/powerUp7.ogg" "$I/glass_004.ogg"
build bits-2 room 0.3 0.7 "[I1]adelay=90|90,volume=0.5[g];[I0][g]amix=inputs=2:normalize=0[dry]" \
  "$D/powerUp8.ogg" "$I/glass_001.ogg"

# Raid: a riser, then two hits a beat apart, in a long hall.
build raid hall 0.45 1.6 "[I0]volume=0.6[sw];[I1]adelay=260|260[h1];[I2]adelay=780|780[h2];[I3]adelay=780|780,volume=0.3[g];[sw][h1][h2][g]amix=inputs=4:normalize=0[dry]" \
  "$I/maximize_005.ogg" "$J/jingles_HIT06.ogg" "$J/jingles_HIT15.ogg" "$I/glass_004.ogg"

# Paid: chips handled, a confirmation and a bright ding.
build paid room 0.3 0.8 "[I0]volume=0.6[c];[I1]adelay=150|150[k];[I2]adelay=320|320,volume=0.5[g];[c][k][g]amix=inputs=3:normalize=0[dry]" \
  "$C/chips-handle-5.ogg" "$I/confirmation_004.ogg" "$I/glass_004.ogg"
