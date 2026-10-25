# Recording benchmark

Measures what Clutch costs while it records a game, next to obs-studio with the same setup.
Windows only. Results go to `results/` (git-ignored).

| Script | Does |
|---|---|
| `bench.ps1` | Samples one running app once a second for N seconds: obs-websocket `GetStats` plus CPU, private memory, GPU 3D and GPU encode of the app's whole process tree, measured from outside. `-Record` starts recording first and stops it afterwards. |
| `run-pair.ps1` | Runs `bench.ps1 -Record` on Clutch, then on obs-studio, with matched settings, and restores both apps' settings afterwards. |
| `compare.ps1` | Side-by-side table of the newest Clutch and obs-studio summaries per label. |

## Matched setup

`run-pair.ps1` sets both apps up the same way:

- one game capture of the game window (`-Window`, Apex Legends by default), desktop audio and mic;
- 1920×1080 canvas at 60 fps;
- NVENC H.264 CBR 12 Mbps, AAC 192 kbps, mkv;
- no replay or clip buffer.

Clutch gets a generated `Bench` scene collection and edited settings (clip mode off, CBR, no
game-audio prompt). obs-studio gets an edited `Untitled` profile and uses the scene collection
it already has, which must contain a game capture of the same game. Both apps' files are backed
up first and restored in a `finally` block.

Start the game first, close both apps, then:

```powershell
pwsh -File tools/bench/run-pair.ps1 -ClutchPassword <remote control password>
```

The obs-websocket password is read from obs-studio's config. Each app writes a ~450 MB
recording to its usual folder.

## Baseline (2026-10-25)

Clutch 0.5.0-beta.1 and obs-studio 32.2.2, RTX 4070 Ti SUPER, 20 cores, 64 GB, Windows 11,
Apex Legends in the Firing Range, 300 s each. Both recorded every frame with none dropped.

| Metric (average) | Clutch | obs-studio |
|---|---|---|
| CPU, process tree (% of all cores) | 3.23 | 0.73 |
| Private memory | 1206 MB | 490 MB |
| GPU 3D engine, process | 10.7% | 8.5% |
| GPU encode engine, process | 20.3% | 20.8% |
| Render time, avg / p95 | 0.94 / 1.67 ms | 0.36 / 0.37 ms |

Whole-system CPU and GPU include the game, so they move with what happens in it; compare the
per-process numbers.

### After capture fences and GPU frames (2026-10-25)

`improve-pipeline-performance` sections 2 and 3 (release build), same setup, Clutch only,
against the obs-studio baseline above. 23004 frames, none dropped; NVENC fed GPU frames.

| Metric (average) | Clutch before | Clutch now | obs-studio |
|---|---|---|---|
| CPU, process tree (% of all cores) | 3.23 | 2.94 | 0.73 |
| Private memory | 1206 MB | 872 MB | 490 MB |
| GPU 3D engine, process | 10.7% | 1.7% | 8.5% |
| GPU encode engine, process | 20.3% | 20.3% | 20.8% |
| Render time, avg / p95 | 0.94 / 1.67 ms | 0.67 / 1.04 ms | 0.36 / 0.37 ms |

Video-thread stages (average CPU ms per frame): submit 0.52, render 0.09, sources 0.02,
convert 0.01, deliver 0.01, poll 0.01. `queue.submit` is most of the frame: wgpu holds the
device lock across the UI's present (see `fix-gpu-contention`). The video thread is only
~0.2% of all cores; the rest of Clutch's CPU is other threads (UI loop, audio, encoder).

Targets for `improve-pipeline-performance`: see that change's proposal.
