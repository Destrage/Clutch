//! Reading YouTube chat for the connected channel's live broadcast. It uses YouTube's streaming
//! read (one long call that keeps sending new messages, a JSON array arriving piece by piece),
//! and falls back to asking at the interval YouTube gives when that isn't available.
//!
//! YouTube's API quota is shared by everyone using Clutch: a streaming call costs as much as one
//! poll, so streaming keeps the cost low.

use super::{Context, Message, Platform, State, backoff};
use serde_json::Value;
use std::io::Read;
use std::time::Duration;

const API: &str = "https://youtube.googleapis.com/youtube/v3/liveChat/messages";

pub fn read(cx: &Context<'_>, session: &crate::youtube::Session) {
    let mut failures = 0;
    while !cx.stopped() {
        if session.account().is_none() {
            cx.state(State::Waiting(
                "Connect YouTube in Settings → Stream to see its chat here".into(),
            ));
            return;
        }
        cx.state(State::Connecting);
        let Some(chat) = session.live_chat_id() else {
            cx.state(State::Waiting(
                "YouTube chat appears here when your broadcast is live".into(),
            ));
            if !cx.pause(Duration::from_secs(20)) {
                return;
            }
            continue;
        };
        match stream(cx, session, &chat) {
            Ended::Stopped => return,
            Ended::Unsupported => {
                if poll(cx, session, &chat) {
                    return;
                }
            }
            Ended::Quota => {
                cx.state(State::Waiting(
                    "YouTube chat paused: Clutch's daily YouTube limit is used up".into(),
                ));
                if !cx.pause(Duration::from_secs(600)) {
                    return;
                }
                continue;
            }
            Ended::Dropped => {}
        }
        failures += 1;
        if !cx.pause(backoff(failures)) {
            return;
        }
    }
}

enum Ended {
    Stopped,
    /// The streaming read isn't available: poll instead.
    Unsupported,
    Quota,
    Dropped,
}

/// An agent without timeouts for the long streaming call.
fn stream_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(15)))
        .user_agent(concat!("Clutch/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn stream(cx: &Context<'_>, session: &crate::youtube::Session, chat: &str) -> Ended {
    let mut page: Option<String> = None;
    // The first reply starts with recent history; only show what arrives from now on.
    let mut skip_before = Some(chrono::Utc::now() - chrono::Duration::seconds(30));
    let mut renewed = false;
    loop {
        if cx.stopped() {
            return Ended::Stopped;
        }
        let Some(token) = session.token(renewed) else {
            return Ended::Dropped;
        };
        let mut url =
            format!("{API}/stream?liveChatId={chat}&part=snippet,authorDetails&maxResults=200");
        if let Some(p) = &page {
            url.push_str(&format!("&pageToken={p}"));
        }
        let response = stream_agent()
            .get(&url)
            .header("Authorization", &format!("Bearer {token}"))
            .call();
        let Ok(mut response) = response else {
            return Ended::Dropped;
        };
        match response.status().as_u16() {
            200 => {}
            401 if !renewed => {
                renewed = true;
                continue;
            }
            403 => {
                let body: Value = response.body_mut().read_json().unwrap_or(Value::Null);
                return if crate::http::error_message(&body)
                    .to_lowercase()
                    .contains("quota")
                {
                    Ended::Quota
                } else {
                    Ended::Unsupported
                };
            }
            status => {
                tracing::debug!(status, "YouTube chat stream");
                return Ended::Unsupported;
            }
        }
        renewed = false;
        cx.state(State::Reading);
        let mut reader = response.into_body().into_reader();
        let mut splitter = Objects::default();
        let mut buffer = [0u8; 8192];
        loop {
            if cx.stopped() {
                return Ended::Stopped;
            }
            let n = match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            for object in splitter.feed(&buffer[..n]) {
                let Ok(reply) = serde_json::from_slice::<Value>(&object) else {
                    continue;
                };
                if let Some(next) = reply.get("nextPageToken").and_then(Value::as_str) {
                    page = Some(next.to_owned());
                }
                for message in messages(&reply, skip_before) {
                    cx.push(message);
                }
                skip_before = None;
            }
        }
        // The call ended (YouTube closes it now and then): continue from the last page.
        if !cx.pause(Duration::from_secs(1)) {
            return Ended::Stopped;
        }
    }
}

/// Asks for new messages at the interval YouTube gives. `true` when stopped.
fn poll(cx: &Context<'_>, session: &crate::youtube::Session, chat: &str) -> bool {
    let mut page: Option<String> = None;
    let mut skip_before = Some(chrono::Utc::now() - chrono::Duration::seconds(30));
    loop {
        if cx.stopped() {
            return true;
        }
        let Some(token) = session.token(false) else {
            return false;
        };
        let mut url = format!("{API}?liveChatId={chat}&part=snippet,authorDetails&maxResults=200");
        if let Some(p) = &page {
            url.push_str(&format!("&pageToken={p}"));
        }
        let Some((status, reply)) = crate::http::get_bearer(&url, &token, &[]) else {
            return false;
        };
        match status {
            200 => {}
            401 => {
                if session.token(true).is_none() {
                    return false;
                }
                continue;
            }
            _ => {
                tracing::debug!(status, "YouTube chat poll");
                return false;
            }
        }
        cx.state(State::Reading);
        for message in messages(&reply, skip_before) {
            cx.push(message);
        }
        skip_before = None;
        page = reply
            .get("nextPageToken")
            .and_then(Value::as_str)
            .map(str::to_owned);
        // At least five seconds apart: each ask costs part of the shared daily quota.
        let wait = reply
            .get("pollingIntervalMillis")
            .and_then(Value::as_u64)
            .unwrap_or(5000)
            .max(5000);
        if !cx.pause(Duration::from_millis(wait)) {
            return true;
        }
    }
}

/// The chat messages in a reply, leaving out those sent before `skip_before`.
fn messages(reply: &Value, skip_before: Option<chrono::DateTime<chrono::Utc>>) -> Vec<Message> {
    let Some(items) = reply.get("items").and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let snippet = item.get("snippet")?;
            let (text, event) = describe(snippet)?;
            let published = snippet
                .get("publishedAt")
                .and_then(Value::as_str)
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok());
            if let (Some(before), Some(at)) = (skip_before, published)
                && at < before
            {
                return None;
            }
            let author = item
                .get("authorDetails")
                .and_then(|a| a.get("displayName"))
                .and_then(Value::as_str)
                .unwrap_or("Someone")
                .to_owned();
            Some(Message {
                platform: Platform::YouTube,
                color: super::name_color(&author),
                author,
                text,
                time: published
                    .map_or_else(chrono::Local::now, |t| t.with_timezone(&chrono::Local)),
                event,
            })
        })
        .collect()
}

/// A chat item's text and whether it is a stream event (Super Chat, membership, …): `None` for
/// kinds that aren't shown.
fn describe(snippet: &Value) -> Option<(String, bool)> {
    let s = |v: Option<&Value>| v.and_then(Value::as_str).unwrap_or_default().to_owned();
    let details = |key: &str| snippet.get(key);
    let with_comment = |head: String, comment: String| {
        if comment.is_empty() {
            head
        } else {
            format!("{head}: {comment}")
        }
    };
    let kind = snippet
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("textMessageEvent");
    let line = match kind {
        "textMessageEvent" => {
            let text = s(snippet.get("displayMessage"));
            return (!text.is_empty()).then_some((text, false));
        }
        "superChatEvent" => {
            let d = details("superChatDetails")?;
            with_comment(
                format!("sent a {} Super Chat", s(d.get("amountDisplayString"))),
                s(d.get("userComment")),
            )
        }
        "superStickerEvent" => {
            let d = details("superStickerDetails")?;
            format!("sent a {} Super Sticker", s(d.get("amountDisplayString")))
        }
        "newSponsorEvent" => {
            let level = s(details("newSponsorDetails").and_then(|d| d.get("memberLevelName")));
            if level.is_empty() {
                "became a member".to_owned()
            } else {
                format!("became a member ({level})")
            }
        }
        "memberMilestoneChatEvent" => {
            let d = details("memberMilestoneChatDetails")?;
            let months = d.get("memberMonth").and_then(Value::as_u64).unwrap_or(0);
            with_comment(
                format!("has been a member for {months} months"),
                s(d.get("userComment")),
            )
        }
        "membershipGiftingEvent" => {
            let d = details("membershipGiftingDetails")?;
            let count = d
                .get("giftMembershipsCount")
                .and_then(Value::as_u64)
                .unwrap_or(1);
            if count == 1 {
                "gifted a membership".to_owned()
            } else {
                format!("gifted {count} memberships")
            }
        }
        _ => return None,
    };
    Some((line, true))
}

/// Splits a JSON array arriving in pieces into its whole top-level objects.
#[derive(Default)]
struct Objects {
    current: Vec<u8>,
    depth: u32,
    in_string: bool,
    escaped: bool,
}

impl Objects {
    fn feed(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut done = Vec::new();
        for &b in bytes {
            if self.depth == 0 {
                // Between objects: the array's brackets and commas.
                if b == b'{' {
                    self.depth = 1;
                    self.current.clear();
                    self.current.push(b);
                }
                continue;
            }
            self.current.push(b);
            if self.in_string {
                if self.escaped {
                    self.escaped = false;
                } else if b == b'\\' {
                    self.escaped = true;
                } else if b == b'"' {
                    self.in_string = false;
                }
                continue;
            }
            match b {
                b'"' => self.in_string = true,
                b'{' | b'[' => self.depth += 1,
                b'}' | b']' => {
                    self.depth -= 1;
                    if self.depth == 0 {
                        done.push(std::mem::take(&mut self.current));
                    }
                }
                _ => {}
            }
        }
        done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paid_and_member_messages_are_events() {
        let chat = serde_json::json!({ "type": "superChatEvent",
            "superChatDetails": { "amountDisplayString": "$5.00", "userComment": "great stream" } });
        assert_eq!(
            describe(&chat),
            Some(("sent a $5.00 Super Chat: great stream".into(), true))
        );
        let gift = serde_json::json!({ "type": "membershipGiftingEvent",
            "membershipGiftingDetails": { "giftMembershipsCount": 5 } });
        assert_eq!(describe(&gift), Some(("gifted 5 memberships".into(), true)));
        let text = serde_json::json!({ "type": "textMessageEvent", "displayMessage": "hi" });
        assert_eq!(describe(&text), Some(("hi".into(), false)));
        let other = serde_json::json!({ "type": "pollEvent" });
        assert_eq!(describe(&other), None);
    }

    #[test]
    fn objects_split_across_pieces() {
        let mut s = Objects::default();
        let text = br#"[{"a":"x}y","b":[1,{"c":2}]}
,{"d":"\"q\""}]"#;
        let (one, two) = text.split_at(9);
        let mut got = s.feed(one);
        got.extend(s.feed(two));
        assert_eq!(got.len(), 2);
        let first: Value = serde_json::from_slice(&got[0]).unwrap();
        assert_eq!(first["a"], "x}y");
        let second: Value = serde_json::from_slice(&got[1]).unwrap();
        assert_eq!(second["d"], "\"q\"");
    }

    #[test]
    fn replies_become_messages() {
        let reply = serde_json::json!({
            "items": [
                { "snippet": { "displayMessage": "old", "publishedAt": "2020-01-01T00:00:00Z" },
                  "authorDetails": { "displayName": "A" } },
                { "snippet": { "displayMessage": "hi", "publishedAt": "2999-01-01T00:00:00Z" },
                  "authorDetails": { "displayName": "B" } }
            ]
        });
        let all = messages(&reply, None);
        assert_eq!(all.len(), 2);
        let fresh = messages(&reply, Some(chrono::Utc::now()));
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].author, "B");
        assert_eq!(fresh[0].text, "hi");
    }
}
