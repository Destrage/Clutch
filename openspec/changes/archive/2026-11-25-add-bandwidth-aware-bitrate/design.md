# Design: Upload test and bitrate budget

## Measuring
- Four worker threads, each with its own `ureq` agent (separate TCP+TLS connections), POST
  bodies of random bytes to `https://speed.cloudflare.com/__up`. The bytes come from one 8 MiB
  buffer filled once from a fast PRNG, so the data can't be compressed on the way.
- Body sizes start at 64 KiB and double while a request finishes in under 0.5 s, up to 8 MiB
  (a slice of one shared random buffer), so slow and fast links both get several completed
  requests.
- Each completed request records `(bytes, duration)`. A connection's rate is its bytes over its
  time, leaving out its first request (TCP ramp-up) when it has later ones; the connections run
  side by side, so their rates add up. Counting completed requests (the server has read the
  body) avoids counting data still sitting in socket buffers.
- Workers start no new request after 8 s. The test runs on its own thread
  and the UI polls its progress and result on its tick.
- Failure (DNS, TLS, HTTP status other than 200, or under 2 completed requests) returns an error
  string for the UI. There is no partial result.

## Budget
```
usable    = min(measured × 0.75, measured − 1500)            (kb/s)
share     = usable / enabled_destinations
video[i]  = clamp(share − audio_kbps[i], 1500, ceiling(service, canvas))
```
`ceiling` and `minimum` are new `Service` methods next to `recommended_kbps`, which stays as the
no-test fallback. A Custom server's ceiling is its current bitrate, so a custom setup is only
ever lowered.

`Covered::with_profile` already takes one optional bitrate per destination. The app computes
those from the budget when a fresh result exists, otherwise from `recommended_kbps`. A reason per
destination (`Upload`, `Platform`, `Kept`; none for the table) feeds the change list wording.

## Stored result
`settings.upload = { kbps, tested_at }` with lenient parsing. Optimize always runs a new
test. The Go Live box reuses a result less than 10 minutes old.

## Testing
- Budget tests: slow (3 Mb/s), medium (15), fast (100) uploads; one and three destinations;
  each platform's cap; the 1.5 Mb/s floor; the Custom server only being lowered.
- Rate calculation tests from synthetic `(bytes, time)` records, including the ramp being dropped.
- Manual: Optimize on a normal connection, and on a throttled one (Windows network
  throttling or a phone hotspot); offline shows the error and keeps the table values.

## While live
No test runs while streaming (it would take the stream's upload), and going live stops a running
test. The network alert can only be raised while live, so its action doesn't test: it offers a
bitrate that fits the rate the destination actually got through
(`fit_to_sent`: 85% of the sent rate, less audio, rounded down to 100 kb/s, at least 1.5 Mb/s,
only when lower than now). Settings don't change a running stream, so it applies the next time
the user goes live.
