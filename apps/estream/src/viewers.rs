//! Viewer counts while live, polled in the background.
//!
//! Kick: the public channel page's data (no sign-in). Twitch: the official API through the
//! connected account, which also lists who is in chat. YouTube: the connected channel's live
//! broadcast, which also gives its chat.

use estream_media::Service;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How often counts are fetched.
const EVERY: Duration = Duration::from_secs(30);

/// A destination to count: `(name, service, channel)`.
pub type Target = (String, Service, String);

/// Who is in a chat: `(total, names)`.
pub type Chatters = (u64, Vec<String>);

/// The connected accounts the counts are fetched with.
#[derive(Clone)]
pub struct Accounts {
    pub twitch: crate::twitch::Session,
    pub youtube: crate::youtube::Session,
}

/// What the last fetch found, by destination name.
#[derive(Default)]
struct Found {
    counts: HashMap<String, u64>,
    chatters: HashMap<String, Chatters>,
    /// The live video of YouTube destinations (for the chat tab).
    videos: HashMap<String, String>,
}

pub struct ViewerCounts {
    targets: Arc<Mutex<Vec<Target>>>,
    found: Arc<Mutex<Found>>,
    /// Fetch now rather than at the next 30 s mark.
    wake: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

impl ViewerCounts {
    pub fn start(accounts: Accounts) -> Self {
        let counter = Self {
            targets: Arc::new(Mutex::new(Vec::new())),
            found: Arc::new(Mutex::new(Found::default())),
            wake: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(false)),
        };
        let (targets, found, wake, stop) = (
            counter.targets.clone(),
            counter.found.clone(),
            counter.wake.clone(),
            counter.stop.clone(),
        );
        let _ = std::thread::Builder::new()
            .name("estream-viewers".into())
            .spawn(move || {
                let mut last: Option<(Vec<Target>, Instant)> = None;
                while !stop.load(Ordering::Acquire) {
                    let now = targets.lock().map(|t| t.clone()).unwrap_or_default();
                    // Fetch right away when the targets change, else every 30 s.
                    let due = last
                        .as_ref()
                        .is_none_or(|(t, at)| *t != now || at.elapsed() >= EVERY)
                        || wake.swap(false, Ordering::AcqRel);
                    if due {
                        let fresh = fetch_all(&accounts, &now);
                        if let Ok(mut f) = found.lock() {
                            *f = fresh;
                        }
                        last = Some((now, Instant::now()));
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
            });
        counter
    }

    /// The destinations that are live now (empty when off air).
    pub fn set_targets(&self, targets: Vec<Target>) {
        if let Ok(mut t) = self.targets.lock()
            && *t != targets
        {
            if targets.is_empty()
                && let Ok(mut f) = self.found.lock()
            {
                *f = Found::default();
            }
            *t = targets;
        }
    }

    /// Fetches again soon (after connecting an account).
    pub fn refresh(&self) {
        self.wake.store(true, Ordering::Release);
    }

    /// The latest count for a destination, if its platform reports one.
    pub fn get(&self, name: &str) -> Option<u64> {
        self.found.lock().ok()?.counts.get(name).copied()
    }

    /// Who is in a destination's chat, if its platform lists them.
    pub fn chatters(&self, name: &str) -> Option<Chatters> {
        self.found.lock().ok()?.chatters.get(name).cloned()
    }

    /// The live video of a YouTube destination, found through the connected channel.
    pub fn video(&self, name: &str) -> Option<String> {
        self.found.lock().ok()?.videos.get(name).cloned()
    }
}

impl Drop for ViewerCounts {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

fn fetch_all(accounts: &Accounts, targets: &[Target]) -> Found {
    let mut found = Found::default();
    for (name, service, channel) in targets {
        match service {
            Service::Kick => {
                if let Some(n) = kick(channel) {
                    found.counts.insert(name.clone(), n);
                }
            }
            Service::Twitch => {
                if let Some(n) = accounts.twitch.viewers(channel) {
                    found.counts.insert(name.clone(), n);
                }
                if let Some(list) = accounts.twitch.chatters(channel) {
                    found.chatters.insert(name.clone(), list);
                }
            }
            Service::YouTube => {
                if let Some(live) = accounts.youtube.live() {
                    if let Some(n) = live.viewers {
                        found.counts.insert(name.clone(), n);
                    }
                    found.videos.insert(name.clone(), live.video_id);
                }
            }
            _ => {}
        }
    }
    found
}

/// Kick's current viewer count for a channel (`None` when offline or unreachable).
fn kick(channel: &str) -> Option<u64> {
    let slug = crate::chat::kick_slug(channel)?;
    let url = format!("https://kick.com/api/v2/channels/{slug}");
    let mut response = ureq::get(&url)
        .header("Accept", "application/json")
        .header("User-Agent", concat!("Clutch/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| tracing::debug!(error = %e, "Kick viewer count"))
        .ok()?;
    let body: serde_json::Value = response.body_mut().read_json().ok()?;
    kick_count(&body)
}

fn kick_count(body: &serde_json::Value) -> Option<u64> {
    body.get("livestream")?.get("viewer_count")?.as_u64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kick_count_comes_from_the_livestream() {
        let live = serde_json::json!({ "slug": "x", "livestream": { "viewer_count": 812 } });
        assert_eq!(kick_count(&live), Some(812));
        let offline = serde_json::json!({ "slug": "x", "livestream": null });
        assert_eq!(kick_count(&offline), None);
    }
}
