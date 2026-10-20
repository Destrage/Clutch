//! Reading Kick chat from Kick's public realtime feed (the one its website uses). It is not an
//! official API: Kick rotates the feed's app key now and then, so known keys are tried in turn
//! and the one that works is remembered; when none works the panel says so and it retries later.

use super::ws::{self, Next};
use super::{Context, Message, Platform, State, backoff};
use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// The feed's app keys, newest first (Kick changed it in October 2026).
const KEYS: [&str; 2] = ["34bf7a0ff419a2a775b9", "32cbd69e4b950bf97679"];
/// The key that last worked.
static KEY: AtomicUsize = AtomicUsize::new(0);
/// Kick drops quiet connections; ping before that.
const PING_EVERY: Duration = Duration::from_secs(60);

/// A Kick channel's chatroom and owner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Channel {
    pub chatroom: u64,
    pub user_id: u64,
}

/// Looks up a channel by its slug on Kick's public channel page.
pub fn channel(slug: &str) -> Option<Channel> {
    let mut response = ureq::get(&format!("https://kick.com/api/v2/channels/{slug}"))
        .header("Accept", "application/json")
        .header("User-Agent", concat!("Clutch/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| tracing::debug!(error = %e, slug, "Kick channel"))
        .ok()?;
    let body: Value = response.body_mut().read_json().ok()?;
    channel_of(&body)
}

fn channel_of(body: &Value) -> Option<Channel> {
    Some(Channel {
        chatroom: body.get("chatroom")?.get("id")?.as_u64()?,
        user_id: body.get("user_id")?.as_u64()?,
    })
}

/// Why a connection ended.
enum End {
    Stopped,
    /// Kick refused this key: try the next one.
    Refused,
    Dropped,
}

pub fn read(cx: &Context<'_>, channel_name: &str) {
    let Some(slug) = crate::chat::kick_slug(channel_name) else {
        cx.state(State::Waiting(
            "Add your Kick channel name in Settings → Stream".into(),
        ));
        return;
    };
    let mut failures = 0;
    while !cx.stopped() {
        cx.state(State::Connecting);
        let Some(room) = channel(&slug) else {
            cx.state(State::Waiting(format!("Couldn't find the Kick channel \"{slug}\"")));
            failures += 1;
            if !cx.pause(backoff(failures).max(Duration::from_secs(20))) {
                return;
            }
            continue;
        };
        // Try each key once, starting with the one that worked last.
        let first = KEY.load(Ordering::Relaxed);
        let mut refused = 0;
        for i in 0..KEYS.len() {
            let index = (first + i) % KEYS.len();
            match connect(cx, KEYS[index], room.chatroom, index) {
                End::Stopped => return,
                End::Refused => refused += 1,
                End::Dropped => {
                    failures = 0;
                    break;
                }
            }
        }
        if refused == KEYS.len() {
            tracing::warn!("Kick refused every known chat feed key");
            cx.state(State::Waiting(
                "Kick changed its chat feed, so Kick chat can't be read right now".into(),
            ));
            if !cx.pause(Duration::from_secs(300)) {
                return;
            }
            continue;
        }
        failures += 1;
        if !cx.pause(backoff(failures)) {
            return;
        }
    }
}

fn connect(cx: &Context<'_>, key: &str, chatroom: u64, index: usize) -> End {
    let url = format!(
        "wss://ws-us2.pusher.com/app/{key}?protocol=7&client=js&version=8.4.0&flash=false"
    );
    let mut socket = match ws::connect(&url) {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!(error = %e, "Kick chat connection");
            return End::Dropped;
        }
    };
    let mut last = Instant::now();
    loop {
        if cx.stopped() {
            let _ = socket.close(None);
            return End::Stopped;
        }
        let text = match ws::next(&mut socket) {
            Next::Text(t) => t,
            Next::Idle => {
                if last.elapsed() > PING_EVERY {
                    last = Instant::now();
                    if !ws::send(&mut socket, r#"{"event":"pusher:ping","data":{}}"#) {
                        return End::Dropped;
                    }
                }
                continue;
            }
            Next::Closed => return End::Dropped,
        };
        last = Instant::now();
        let Ok(event) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        match event.get("event").and_then(Value::as_str).unwrap_or_default() {
            "pusher:connection_established" => {
                let subscribe = serde_json::json!({
                    "event": "pusher:subscribe",
                    "data": { "auth": "", "channel": format!("chatrooms.{chatroom}.v2") },
                });
                if !ws::send(&mut socket, &subscribe.to_string()) {
                    return End::Dropped;
                }
            }
            "pusher_internal:subscription_succeeded" => {
                KEY.store(index, Ordering::Relaxed);
                cx.state(State::Reading);
            }
            "pusher:ping" => {
                let _ = ws::send(&mut socket, r#"{"event":"pusher:pong","data":{}}"#);
            }
            "pusher:error" => {
                let code = event
                    .get("data")
                    .and_then(|d| d.get("code"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                tracing::warn!(code, "Kick chat feed refused the connection");
                // 4000–4099: the app is refused (wrong key or cluster); others are transient.
                return if (4000..4100).contains(&code) {
                    End::Refused
                } else {
                    End::Dropped
                };
            }
            "App\\Events\\ChatMessageEvent" => {
                if let Some(message) = event
                    .get("data")
                    .and_then(Value::as_str)
                    .and_then(|d| serde_json::from_str::<Value>(d).ok())
                    .and_then(|d| parse(&d))
                {
                    cx.push(message);
                }
            }
            _ => {}
        }
    }
}

/// A chat message event's data.
fn parse(data: &Value) -> Option<Message> {
    let sender = data.get("sender")?;
    let author = sender.get("username")?.as_str()?.to_owned();
    let color = sender
        .get("identity")
        .and_then(|i| i.get("color"))
        .and_then(Value::as_str)
        .filter(|c| c.starts_with('#') && c.len() == 7)
        .map(str::to_owned)
        .unwrap_or_else(|| super::name_color(&author));
    let text = emotes_as_names(data.get("content")?.as_str()?);
    Some(Message {
        platform: Platform::Kick,
        author,
        color,
        text,
        time: chrono::Local::now(),
    })
}

/// `[emote:37234:Prayge]` becomes `Prayge`.
fn emotes_as_names(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(start) = rest.find("[emote:") {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        match tail.find(']') {
            Some(end) => {
                let inner = &tail[7..end];
                out.push_str(inner.split_once(':').map_or(inner, |(_, name)| name));
                rest = &tail[end + 1..];
            }
            None => {
                out.push_str(tail);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_events_become_messages() {
        let data = serde_json::json!({
            "id": "ab05c731", "chatroom_id": 668,
            "content": "[emote:37234:Prayge] MAX WIN [emote:37234:Prayge]",
            "type": "message",
            "sender": { "id": 1, "username": "EckTorro", "slug": "ecktorro",
                        "identity": { "color": "#BC66FF", "badges": [] } }
        });
        let m = parse(&data).unwrap();
        assert_eq!(m.author, "EckTorro");
        assert_eq!(m.color, "#BC66FF");
        assert_eq!(m.text, "Prayge MAX WIN Prayge");
        assert_eq!(m.platform, Platform::Kick);
    }

    #[test]
    fn emotes_read_as_their_names() {
        assert_eq!(emotes_as_names("hi [emote:1:KEKW]!"), "hi KEKW!");
        assert_eq!(emotes_as_names("no emotes"), "no emotes");
        assert_eq!(emotes_as_names("broken [emote:1:x"), "broken [emote:1:x");
    }

    #[test]
    fn the_channel_page_gives_chatroom_and_owner() {
        let body = serde_json::json!({ "id": 668, "user_id": 676, "chatroom": { "id": 668 } });
        assert_eq!(
            channel_of(&body),
            Some(Channel { chatroom: 668, user_id: 676 })
        );
    }
}
