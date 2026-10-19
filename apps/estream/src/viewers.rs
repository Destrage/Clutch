//! Viewer counts while live, polled in the background.
//!
//! Kick: the public channel page's data (no sign-in). Twitch: the official API through the
//! connected account, which also lists who is in chat. YouTube reports nothing yet.

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

pub struct ViewerCounts {
    targets: Arc<Mutex<Vec<Target>>>,
    counts: Arc<Mutex<HashMap<String, u64>>>,
    chatters: Arc<Mutex<HashMap<String, Chatters>>>,
    /// Fetch now rather than at the next 30 s mark.
    wake: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

impl ViewerCounts {
    pub fn start(twitch: crate::twitch::Session) -> Self {
        let counter = Self {
            targets: Arc::new(Mutex::new(Vec::new())),
            counts: Arc::new(Mutex::new(HashMap::new())),
            chatters: Arc::new(Mutex::new(HashMap::new())),
            wake: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(false)),
        };
        let (targets, counts, chatters, wake, stop) = (
            counter.targets.clone(),
            counter.counts.clone(),
            counter.chatters.clone(),
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
                        let (mut fresh, mut in_chat) = (HashMap::new(), HashMap::new());
                        for (name, service, channel) in &now {
                            if let Some(n) = fetch(&twitch, *service, channel) {
                                fresh.insert(name.clone(), n);
                            }
                            if *service == Service::Twitch
                                && let Some(list) = twitch.chatters(channel)
                            {
                                in_chat.insert(name.clone(), list);
                            }
                        }
                        if let Ok(mut c) = counts.lock() {
                            *c = fresh;
                        }
                        if let Ok(mut c) = chatters.lock() {
                            *c = in_chat;
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
            if targets.is_empty() {
                if let Ok(mut c) = self.counts.lock() {
                    c.clear();
                }
                if let Ok(mut c) = self.chatters.lock() {
                    c.clear();
                }
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
        self.counts.lock().ok()?.get(name).copied()
    }

    /// Who is in a destination's chat, if its platform lists them.
    pub fn chatters(&self, name: &str) -> Option<Chatters> {
        self.chatters.lock().ok()?.get(name).cloned()
    }
}

impl Drop for ViewerCounts {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

fn fetch(twitch: &crate::twitch::Session, service: Service, channel: &str) -> Option<u64> {
    match service {
        Service::Kick => kick(channel),
        Service::Twitch => twitch.viewers(channel),
        _ => None,
    }
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
