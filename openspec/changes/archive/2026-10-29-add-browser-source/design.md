# Design: Browser source

## Processes
```
estream.exe ──stdin (JSON lines)──▶ estream-browser.exe ──▶ CEF renderer/GPU subprocesses
            ◀─stdout (JSON lines)──            │  (same exe re-launched with --type=…)
            ◀─── shared memory: frames + audio ┘
```
- **Lifecycle:** the host starts one helper on the first browser source.
  - **Normal end:** closing stdin makes the helper close its pages, shut CEF down and exit.
  - **Helper crash:** the host's stdout reader sees EOF, marks every page lost, and restarts the
    helper after 1 s (with back-off). Sources re-create their pages when they see the new
    generation.

### Commands (host → helper)
| op | fields |
|---|---|
| `create` | `id, url, width, height, fps, css` |
| `update` | same fields (re-navigates if the URL changes, resizes if the size changes) |
| `reload` | `id` |
| `close` | `id` |

### Events (helper → host)
| ev | fields |
|---|---|
| `ready` | — |
| `frame_buffer` | `id, name, width, height` |
| `audio_buffer` | `id, name` |
| `loaded` | `id` |
| `error` | `id, message` |

## Shared memory (Windows file mappings)
Names are `Local\estream-browser-<helper pid>-<id>-<kind>-<n>`, created by the helper.
- **Video:** a 64-byte header `{ seq: u64, width: u32, height: u32 }`, then `width×height×4` RGBA (the helper converts from CEF's premultiplied BGRA, so eStream does no per-pixel work).
  The writer sets `seq` odd, copies, then sets it even (a seqlock). The reader copies, and
  retries on the next frame if `seq` changed or is odd. The helper un-premultiplies alpha while
  copying (CEF paints premultiplied; sources hand the compositor straight alpha). A resize creates
  a new mapping (new `n`), announced by `frame_buffer`.
- **Audio:** a header `{ write: u64 (frames), capacity: u32, channels: u32 = 2, rate: u32 = 48000 }`,
  then `capacity` interleaved stereo f32 frames (1 s). The single writer advances `write`. The
  reader keeps its own position and jumps forward if it fell more than `capacity` behind.

## Helper (`apps/estream-browser`)
- `main`:
  1. Pin the API version (`api_hash`).
  2. Call `execute_process`; if it returns ≥ 0 this is a subprocess, so exit with that code.
  3. Otherwise call `initialize` with `windowless_rendering_enabled`,
     `multi_threaded_message_loop` (CEF's own UI thread), `no_sandbox`, and a `root_cache_path`
     under the user's local app data.
- Browsers are created with `browser_host_create_browser` and windowless `WindowInfo` (transparent
  background). `on_after_created` stores each `Browser` by id.
- `RenderHandler`:
  - `view_rect` returns the configured size;
  - `on_paint` (view only) copies into the video mapping.
- `AudioHandler`: `get_audio_parameters` asks for 48 kHz stereo; `on_audio_stream_packet` takes
  planar f32 and writes it into the ring.
- `LoadHandler`: `on_load_end` injects the custom CSS as a `<style>` element, then reports `loaded`.
  `on_load_error` reports `error`.
- **Default CSS:** `body { background-color: rgba(0,0,0,0); margin: 0 auto; overflow: hidden; }`
  (what overlay pages expect).

## Host (`estream-capture::browser`)
- `BrowserHost` (process-wide): owns the helper child, its stdin, a stdout reader thread and the
  per-page state `{ video mapping, audio mapping, loaded, error }` behind a mutex. Every page-side
  call is non-blocking (`try_lock`).
- **`BrowserVideo` (VideoSource):**
  - `update` sends create/update;
  - `tick` opens a mapping when its name changes, copies a new frame (seq changed and even) into
    the shared straight-alpha RGBA upload path;
  - status: loading, error, or restarting.
- **`BrowserAudio` (AudioSource):** reads the ring with a 50 ms pre-buffer, like media sources.
- Sources find each other by `_source_id`, and the page id is the source id.

## Build
- `.cargo/config.toml`: `CEF_PATH = "third_party/cef"` (relative). cef-dll-sys downloads the
  archive there on first build and copies the runtime files next to the binaries.
- `apps/estream-browser` depends on `cef` with default features off. The main app finds
  `estream-browser.exe` next to its own executable.

## Testing
- **Shared memory:** seqlock and ring round trips, plus a ring overrun jump (unit tests).
- **Helper end to end** (headless): launch the built helper with a `data:` URL page (a
  half-transparent red background), wait for a frame, and check its size and un-premultiplied
  colour. Then kill the helper and confirm the host reports the loss and restarts it.
- **In-app:** a browser source showing a local HTML file with an animated, transparent overlay.
