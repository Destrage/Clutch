## 1. Measuring
- [x] 1.1 `speedtest.rs`: parallel upload workers, growing body sizes, rate from completed requests without each connection's first, errors
- [x] 1.2 Rate calculation unit tests
- [x] 1.3 Saved result in settings (`upload.kbps`, `upload.tested_at`), lenient parsing

## 2. Budget
- [x] 2.1 `Service::ceiling_kbps` and `Service::minimum_kbps` per canvas, checked against each platform's published guidance
- [x] 2.2 Budget function (usable upload, split per destination, audio, floor, ceiling, Custom only lowered) with a reason per destination
- [x] 2.3 Budget unit tests

## 3. Wiring
- [x] 3.1 Optimize for this PC runs the test, shows the upload in the "found" line and the bitrate reasons in the change list
- [x] 3.2 "Not enough upload" note suggesting 720p or 30 fps
- [x] 3.3 Performance page: last result, its time, Test again
- [x] 3.4 Go Live box: test while a destination is added, use the result when saving it
- [x] 3.5 Network congestion alert action: offer the bitrate that got through (no test while live)

## 4. Verify
- [x] 4.1 `cargo clippy` and tests for `estream`, `estream-media`
- [x] 4.2 In-app: Optimize with a real test, a slow upload (5 Mb/s saved result), the Go Live box while it tests and after
- [ ] 4.3 In-app: offline or blocked test server, and a real throttled connection
