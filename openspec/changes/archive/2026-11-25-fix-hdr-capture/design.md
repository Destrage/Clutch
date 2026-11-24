# Design: HDR screen capture

## Detecting HDR
For the target monitor (`MonitorFromWindow` for window and game capture), enumerate DXGI
outputs and match `DXGI_OUTPUT_DESC1.Monitor`. HDR is active when `ColorSpace` is
`DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020`. `MaxLuminance` gives the peak (nits).

The SDR white level comes from `DisplayConfigGetDeviceInfo` with
`DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL` for the matching path:
`nits = SDRWhiteLevel / 1000 × 80`. If the call fails, use 80 nits × 2.5 = 200 nits, which is
Windows' default slider position.

Detection runs when a capture starts and whenever the frame pool is recreated (size changes
already trigger that). A check once a second in the frame-arrived callback (the capture thread, not the video thread)
catches HDR being toggled and windows moving between monitors.

## Capture format
- HDR: `DirectXPixelFormat::R16G16B16A16Float` frame pool; the shared D3D11/D3D12 texture is
  `DXGI_FORMAT_R16G16B16A16_FLOAT` and is viewed in wgpu as `Rgba16Float`.
- SDR, or the option set to "As Windows gives them": unchanged (`B8G8R8A8`, viewed as
  `Bgra8UnormSrgb`).

The shared texture ring keeps one format per capture. A format change recreates the ring, the
same way a size change does today.

## Mapping to SDR (GPU)
A small render pass in `estream-gpu` (`hdr.rs`) reads the scRGB texture and writes an
`Rgba16Float` linear texture that the compositor samples like any linear source:

```
c      = max(scrgb.rgb, 0) × (80 / sdr_white_nits)     // SDR white → 1.0
peak   = max(max_luminance_nits / sdr_white_nits, 1)
m      = max(c.r, c.g, c.b)
if m > knee:                                            // knee = 0.9
    m' = knee + (1 - knee) × shoulder((m - knee) / (peak - knee))
    c *= m' / m                                         // hue-preserving (max-RGB)
```

`shoulder(x)` is a smooth curve with value and slope matched at the knee (an extended Reinhard
form), so SDR content below 90% white passes through unchanged and SDR white itself lands at about
95% linear (249 of 255 in sRGB): a little room has to be taken from SDR white for highlights to
keep any detail, and 0.9 keeps the desktop looking white. Scaling by max-RGB keeps hues
instead of shifting bright colors towards white per channel.

The parameters live in a uniform buffer that is updated only when detection changes, so the pass
does no per-frame allocation. It is encoded into the source's tick encoder only when a new frame
arrives (after the copy fence wait), so a still desktop costs nothing. The pipeline is built once
per device on the capture start thread, never on the video thread. The pass reads the slot with
`textureLoad` into an `Rgba16Float` texture that the compositor samples like any linear source.

## Why not let Windows do it
WGC's 8-bit path is the clipping that causes the bug. DXGI Desktop Duplication has the same
behavior in 8-bit and offers no tone mapping. A shader mapping is the only path that keeps HDR
highlights.

## Testing
- Unit tests for the curve: identity below the knee, monotonic, reaches 1.0 at `peak`, never
  exceeds 1.0, and keeps hue (channel ratios) when it compresses.
- A headless GPU test feeds a synthetic scRGB texture (SDR white at 2.5, highlights at 10.0)
  through the pass and checks the output against the CPU reference.
- Manual check on an HDR monitor: the desktop, an SDR game with Auto HDR and an HDR game,
  compared with what the screen shows.

## Testing without an HDR monitor
Windows Graphics Capture delivers 16-bit scRGB from an SDR monitor too (white at 1.0). A test
hook makes a capture treat its monitor as HDR with an 80-nit SDR white, so the whole path runs
on any machine: 16-bit frames, HDR ring slots, the reader's mapping pass, and a readback that
must be lit and stay within SDR range. No window opens.
