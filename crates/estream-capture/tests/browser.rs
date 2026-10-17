//! End-to-end test of the CEF browser helper (`cargo build -p estream-browser` first).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use estream_capture::media::browser::{PageConfig, PageState, host};
use estream_shm::FrameReader;
use std::time::{Duration, Instant};

fn wait_for<T>(timeout: Duration, mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Some(v) = f() {
            return Some(v);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

/// Waits for page `id` to have a frame mapping, newer than `after_generation`.
fn wait_frame(id: &str, after_generation: u64) -> (PageState, u64) {
    wait_for(Duration::from_secs(30), || {
        let (page, _, generation) = host().page(id)?;
        (generation > after_generation && page.video.is_some()).then_some((page, generation))
    })
    .unwrap_or_else(|| panic!("no frame from the helper: {:?}", host().page(id)))
}

/// Reads a frame whose centre pixel isn't empty (the page may paint blank first).
fn read_centre(name: &str, w: u32, h: u32) -> [u8; 4] {
    let mut reader = FrameReader::open(name, w, h).expect("open frame mapping");
    let mut buf = Vec::new();
    wait_for(Duration::from_secs(15), || {
        if !reader.read(&mut buf) {
            return None;
        }
        let i = (((h / 2) * w + w / 2) * 4) as usize;
        let px = [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]];
        (px[3] != 0).then_some(px)
    })
    .expect("a painted frame")
}

#[test]
fn renders_pages_and_recovers_from_crashes() {
    let page = PageConfig {
        url: "data:text/html,<html><body style='margin:0;background:rgba(255,0,0,0.5)'></body></html>"
            .into(),
        width: 320,
        height: 180,
        fps: 30,
        css: "body { margin: 0 }".into(),
    };
    host().set_page("t1", page);

    let (state, generation) = wait_frame("t1", 0);
    let (name, w, h) = state.video.unwrap();
    assert_eq!((w, h), (320, 180));
    let [r, g, b, a] = read_centre(&name, w, h);
    // Straight alpha: full red at half opacity.
    assert!((126..=129).contains(&a), "alpha {a}");
    assert!(r >= 250 && g <= 3 && b <= 3, "colour {r},{g},{b}");

    // A crashed helper is restarted and the page comes back.
    assert!(host().kill_helper_for_test(), "kill the helper");
    let (state, _) = wait_frame("t1", generation);
    let (name, w, h) = state.video.unwrap();
    let [r, _, _, a] = read_centre(&name, w, h);
    assert!(r >= 250 && (126..=129).contains(&a));

    host().remove_page("t1");
}
