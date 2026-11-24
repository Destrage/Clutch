//! Upload speed test: random data sent to Cloudflare's speed test over a few parallel
//! connections, timed per completed request.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const URL: &str = "https://speed.cloudflare.com/__up";
const WORKERS: usize = 4;
const TEST_TIME: Duration = Duration::from_secs(8);
const FIRST_BODY: usize = 64 * 1024;
const MAX_BODY: usize = 8 * 1024 * 1024;
/// A body is doubled while a request finishes faster than this.
const GROW_BELOW: Duration = Duration::from_millis(500);

/// One finished upload request.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Request {
    pub bytes: u64,
    pub seconds: f64,
}

/// The upload rate in kb/s from each connection's finished requests. Each connection's first
/// request is left out while it has later ones (TCP is still ramping up then); connections run
/// side by side, so their rates add up.
pub fn rate_kbps(connections: &[Vec<Request>]) -> Option<f64> {
    let mut total = 0.0;
    let mut any = false;
    for requests in connections {
        let steady = if requests.len() > 1 {
            &requests[1..]
        } else {
            &requests[..]
        };
        let bytes: u64 = steady.iter().map(|r| r.bytes).sum();
        let seconds: f64 = steady.iter().map(|r| r.seconds).sum();
        if bytes > 0 && seconds > 0.0 {
            total += bytes as f64 * 8.0 / 1000.0 / seconds;
            any = true;
        }
    }
    any.then_some(total)
}

/// A running test. Poll [`Test::result`] from the UI; drop or [`Test::stop`] to end it early.
pub struct Test {
    stop: Arc<AtomicBool>,
    /// Progress in percent.
    progress: Arc<AtomicU32>,
    result: Arc<Mutex<Option<Result<f64, String>>>>,
}

impl Test {
    pub fn start() -> Self {
        let test = Self {
            stop: Arc::default(),
            progress: Arc::default(),
            result: Arc::default(),
        };
        let (stop, progress, result) = (
            test.stop.clone(),
            test.progress.clone(),
            test.result.clone(),
        );
        let spawned = std::thread::Builder::new()
            .name("upload test".into())
            .spawn(move || {
                let outcome = run(&stop, &progress);
                match &outcome {
                    Ok(kbps) => tracing::info!(kbps = kbps.round(), "upload speed measured"),
                    Err(e) => tracing::warn!(error = %e, "upload speed test failed"),
                }
                if let Ok(mut slot) = result.lock() {
                    *slot = Some(outcome);
                }
            });
        if let Err(e) = spawned
            && let Ok(mut slot) = test.result.lock()
        {
            *slot = Some(Err(e.to_string()));
        }
        test
    }

    pub fn progress(&self) -> u32 {
        self.progress.load(Relaxed)
    }

    /// The result once the test has finished (taken once).
    pub fn result(&self) -> Option<Result<f64, String>> {
        self.result.lock().ok()?.take()
    }

    pub fn stop(&self) {
        self.stop.store(true, Relaxed);
    }
}

impl Drop for Test {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run(stop: &Arc<AtomicBool>, progress: &Arc<AtomicU32>) -> Result<f64, String> {
    // Random bytes, so nothing on the way can compress them; every body is a slice of this.
    let data: Arc<Vec<u8>> = Arc::new(random_bytes(MAX_BODY));
    let start = Instant::now();
    let workers: Vec<_> = (0..WORKERS)
        .map(|_| {
            let (data, stop) = (data.clone(), stop.clone());
            std::thread::spawn(move || worker(&data, &stop, start))
        })
        .collect();
    while workers.iter().any(|w| !w.is_finished()) {
        let pct = (start.elapsed().as_secs_f64() / TEST_TIME.as_secs_f64() * 100.0).min(99.0);
        progress.store(pct as u32, Relaxed);
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut connections = Vec::new();
    let mut error = None;
    for w in workers {
        match w.join() {
            Ok(Ok(requests)) => connections.push(requests),
            Ok(Err(e)) => error = Some(e),
            Err(_) => error = Some("the test stopped unexpectedly".to_owned()),
        }
    }
    progress.store(100, Relaxed);
    if stop.load(Relaxed) {
        return Err("stopped".to_owned());
    }
    let finished: usize = connections.iter().map(Vec::len).sum();
    if finished < 2 {
        return Err(error.unwrap_or_else(|| "too little data got through".to_owned()));
    }
    rate_kbps(&connections).ok_or_else(|| "too little data got through".to_owned())
}

fn worker(data: &[u8], stop: &AtomicBool, start: Instant) -> Result<Vec<Request>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(15)))
        .user_agent(concat!("Clutch/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut size = FIRST_BODY;
    let mut requests = Vec::new();
    while start.elapsed() < TEST_TIME && !stop.load(Relaxed) {
        let began = Instant::now();
        let response = agent
            .post(URL)
            .content_type("application/octet-stream")
            .send(&data[..size]);
        let took = began.elapsed();
        match response {
            Ok(r) if r.status().is_success() => {}
            Ok(r) => return Err(format!("the test server answered {}", r.status().as_u16())),
            // A slow request cut off near the end doesn't spoil what was measured.
            Err(e) if requests.len() >= 2 => {
                tracing::debug!(error = %e, "upload test request");
                break;
            }
            Err(e) => return Err(e.to_string()),
        }
        requests.push(Request {
            bytes: size as u64,
            seconds: took.as_secs_f64(),
        });
        if took < GROW_BELOW {
            size = (size * 2).min(MAX_BODY);
        }
    }
    Ok(requests)
}

/// xorshift64* bytes: fast and incompressible, not for anything secret.
fn random_bytes(len: usize) -> Vec<u8> {
    let mut state = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0x9e37_79b9_7f4a_7c15, |d| d.as_nanos() as u64)
        | 1;
    let mut out = Vec::with_capacity(len + 8);
    while out.len() < len {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        out.extend_from_slice(&state.wrapping_mul(0x2545_f491_4f6c_dd1d).to_le_bytes());
    }
    out.truncate(len);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(bytes: u64, seconds: f64) -> Request {
        Request { bytes, seconds }
    }

    #[test]
    fn first_request_is_ramp_up() {
        // 1 MB in 1 s while ramping, then 1 MB per 0.5 s: 16 Mb/s steady.
        let one = vec![r(1_000_000, 1.0), r(1_000_000, 0.5), r(2_000_000, 1.0)];
        let kbps = rate_kbps(&[one]).unwrap();
        assert!((kbps - 16_000.0).abs() < 1e-6, "{kbps}");
    }

    #[test]
    fn connections_add_up() {
        let c = vec![r(500_000, 2.0), r(500_000, 1.0)];
        let kbps = rate_kbps(&[c.clone(), c.clone(), c.clone(), c]).unwrap();
        assert!((kbps - 16_000.0).abs() < 1e-6, "{kbps}");
    }

    #[test]
    fn a_single_request_still_counts() {
        let kbps = rate_kbps(&[vec![r(250_000, 1.0)], vec![]]).unwrap();
        assert!((kbps - 2_000.0).abs() < 1e-6);
        assert_eq!(rate_kbps(&[vec![], vec![]]), None);
    }

    #[test]
    fn random_bytes_do_not_repeat() {
        let b = random_bytes(4096);
        assert_eq!(b.len(), 4096);
        assert_ne!(b[..2048], b[2048..]);
        let zeros = b.iter().filter(|x| **x == 0).count();
        assert!(zeros < 64, "{zeros}");
    }
}
