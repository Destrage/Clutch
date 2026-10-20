//! The combined chat: a reader per destination that has chat (Twitch, YouTube, Kick), each on
//! its own thread, putting messages into one shared feed the chat panel shows; and replying to
//! one platform or all of them through each platform's API.

pub mod kick;
pub mod twitch;
pub mod youtube;
mod ws;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Messages kept for the panel (older ones drop off).
const KEEP: usize = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Platform {
    Twitch,
    YouTube,
    Kick,
}

impl Platform {
    pub fn key(self) -> &'static str {
        match self {
            Platform::Twitch => "twitch",
            Platform::YouTube => "youtube",
            Platform::Kick => "kick",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Platform::Twitch => "Twitch",
            Platform::YouTube => "YouTube",
            Platform::Kick => "Kick",
        }
    }
}

/// One chat message.
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub platform: Platform,
    pub author: String,
    /// The name's colour as `#rrggbb` ("" for none).
    pub color: String,
    pub text: String,
    pub time: chrono::DateTime<chrono::Local>,
}

/// How a destination's chat reader is doing.
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Connecting,
    Reading,
    /// Not reading, and why (shown in the panel).
    Waiting(String),
}

/// The messages and every reader's state, shared by the readers and the UI.
#[derive(Clone, Default)]
pub struct Feed(Arc<Mutex<FeedInner>>);

#[derive(Default)]
struct FeedInner {
    messages: VecDeque<Message>,
    /// Messages ever added (the UI asks for those after the last one it showed).
    added: u64,
    /// `(destination name, platform, state)` for each reader.
    states: Vec<(String, Platform, State)>,
}

impl Feed {
    fn push(&self, message: Message) {
        if let Ok(mut f) = self.0.lock() {
            f.messages.push_back(message);
            if f.messages.len() > KEEP {
                f.messages.pop_front();
            }
            f.added += 1;
        }
    }

    fn set_state(&self, name: &str, state: State) {
        if let Ok(mut f) = self.0.lock()
            && let Some(entry) = f.states.iter_mut().find(|(n, _, _)| n == name)
        {
            entry.2 = state;
        }
    }

    /// Messages added after `seen` (the count from an earlier call), and the new count.
    pub fn since(&self, seen: u64) -> (Vec<Message>, u64) {
        let Ok(f) = self.0.lock() else {
            return (Vec::new(), seen);
        };
        let new = (f.added - seen.min(f.added)).min(f.messages.len() as u64) as usize;
        let start = f.messages.len() - new;
        (f.messages.range(start..).cloned().collect(), f.added)
    }

    pub fn states(&self) -> Vec<(String, Platform, State)> {
        self.0.lock().map(|f| f.states.clone()).unwrap_or_default()
    }
}

/// A destination whose chat to read.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub name: String,
    pub platform: Platform,
    /// Kick slug or Twitch login; for YouTube, the connected account is used.
    pub channel: String,
    /// The platform account is connected (a new sign-in restarts the reader).
    pub connected: bool,
}

/// The connected accounts the readers and replies use.
#[derive(Clone)]
pub struct Accounts {
    pub twitch: crate::twitch::Session,
    pub youtube: crate::youtube::Session,
    pub kick: crate::kick::Session,
}

/// A reader thread; dropping it asks the thread to stop (it notices within a second or so).
struct Reader {
    target: Target,
    stop: Arc<AtomicBool>,
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

pub struct LiveChat {
    feed: Feed,
    readers: Vec<Reader>,
    accounts: Accounts,
}

impl LiveChat {
    pub fn new(accounts: Accounts) -> Self {
        Self {
            feed: Feed::default(),
            readers: Vec::new(),
            accounts,
        }
    }

    pub fn feed(&self) -> &Feed {
        &self.feed
    }

    /// The destinations being read.
    pub fn targets(&self) -> Vec<Target> {
        self.readers.iter().map(|r| r.target.clone()).collect()
    }

    /// Reads exactly these destinations: keeps readers whose target didn't change, starts new
    /// ones and stops the rest.
    pub fn set_targets(&mut self, targets: Vec<Target>) {
        if targets == self.targets() {
            return;
        }
        let mut old = std::mem::take(&mut self.readers);
        if let Ok(mut f) = self.feed.0.lock() {
            f.states
                .retain(|(name, platform, _)| targets.iter().any(|t| &t.name == name && t.platform == *platform));
        }
        for target in targets {
            if let Some(i) = old.iter().position(|r| r.target == target) {
                self.readers.push(old.remove(i));
                continue;
            }
            if let Ok(mut f) = self.feed.0.lock() {
                f.states.retain(|(n, _, _)| *n != target.name);
                f.states
                    .push((target.name.clone(), target.platform, State::Connecting));
            }
            let stop = Arc::new(AtomicBool::new(false));
            let (feed, accounts, t, s) = (
                self.feed.clone(),
                self.accounts.clone(),
                target.clone(),
                stop.clone(),
            );
            let _ = std::thread::Builder::new()
                .name(format!("estream-chat-{}", target.platform.key()))
                .spawn(move || {
                    let reader = Context {
                        name: &t.name,
                        feed: &feed,
                        stop: &s,
                    };
                    match t.platform {
                        Platform::Kick => kick::read(&reader, &t.channel),
                        Platform::Twitch => twitch::read(&reader, &t.channel),
                        Platform::YouTube => youtube::read(&reader, &accounts.youtube),
                    }
                });
            self.readers.push(Reader { target, stop });
        }
        // Dropping the rest stops their threads.
        drop(old);
    }

    /// Sends `text` to one platform (or every one, for `None`) in the background. The result
    /// is one line per platform that couldn't send it.
    pub fn send(&self, to: Option<Platform>, text: String) -> std::sync::mpsc::Receiver<Vec<String>> {
        let (tx, rx) = std::sync::mpsc::channel();
        let targets: Vec<Target> = self
            .targets()
            .into_iter()
            .filter(|t| to.is_none_or(|p| p == t.platform))
            .collect();
        let accounts = self.accounts.clone();
        let _ = std::thread::Builder::new()
            .name("estream-chat-send".into())
            .spawn(move || {
                let problems: Vec<String> = targets
                    .iter()
                    .filter_map(|t| {
                        let result = match t.platform {
                            Platform::Twitch => accounts.twitch.send_chat(&t.channel, &text),
                            Platform::YouTube => accounts.youtube.send_chat(&text),
                            Platform::Kick => accounts.kick.send_chat(&t.channel, &text),
                        };
                        result.err()
                    })
                    .collect();
                let _ = tx.send(problems);
            });
        rx
    }
}

/// What a reader thread works with.
pub struct Context<'a> {
    name: &'a str,
    feed: &'a Feed,
    stop: &'a AtomicBool,
}

impl Context<'_> {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    fn state(&self, state: State) {
        self.feed.set_state(self.name, state);
    }

    fn push(&self, message: Message) {
        self.feed.push(message);
    }

    /// Waits, waking early when stopped. `false` when stopped.
    fn pause(&self, wait: Duration) -> bool {
        let until = std::time::Instant::now() + wait;
        while std::time::Instant::now() < until {
            if self.stopped() {
                return false;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        !self.stopped()
    }
}

/// The pause before reconnecting after `failures` attempts in a row (2 s doubling to a minute).
fn backoff(failures: u32) -> Duration {
    Duration::from_secs((2u64 << failures.min(5)).min(60))
}

/// A steady colour for a name that has none (Twitch users who never picked one).
fn name_color(name: &str) -> String {
    const COLORS: [&str; 8] = [
        "#ff7a85", "#ffb347", "#f5d76e", "#7ddc7a", "#5ccfe6", "#8ec5ff", "#c792ea", "#ff9ec8",
    ];
    let hash = name
        .bytes()
        .fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    COLORS[hash as usize % COLORS.len()].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(text: &str) -> Message {
        Message {
            platform: Platform::Kick,
            author: "a".into(),
            color: String::new(),
            text: text.into(),
            time: chrono::Local::now(),
        }
    }

    #[test]
    fn the_feed_hands_out_only_new_messages() {
        let feed = Feed::default();
        feed.push(message("one"));
        let (first, seen) = feed.since(0);
        assert_eq!(first.len(), 1);
        feed.push(message("two"));
        feed.push(message("three"));
        let (next, seen) = feed.since(seen);
        assert_eq!(
            next.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
            ["two", "three"]
        );
        assert!(feed.since(seen).0.is_empty());
    }

    #[test]
    fn the_feed_keeps_the_newest() {
        let feed = Feed::default();
        for i in 0..KEEP + 50 {
            feed.push(message(&i.to_string()));
        }
        let (all, _) = feed.since(0);
        assert_eq!(all.len(), KEEP);
        assert_eq!(all.last().unwrap().text, (KEEP + 49).to_string());
    }

    #[test]
    fn backoff_grows_to_a_minute() {
        assert_eq!(backoff(0), Duration::from_secs(2));
        assert_eq!(backoff(2), Duration::from_secs(8));
        assert_eq!(backoff(20), Duration::from_secs(60));
    }
}
