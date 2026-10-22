//! Reading Twitch chat through Twitch's chat connection, signed in anonymously: any channel's
//! chat can be read without an account (replying uses the connected account; see `twitch.rs`).

use super::ws::{self, Next};
use super::{Context, Message, Platform, State, backoff};
use std::time::{Duration, Instant};

const URL: &str = "wss://irc-ws.chat.twitch.tv:443";
/// Twitch pings about every five minutes; a connection silent for longer is dead.
const SILENT_LIMIT: Duration = Duration::from_secs(6 * 60);

pub fn read(cx: &Context<'_>, channel_name: &str) {
    let Some(login) = crate::chat::twitch_login(channel_name) else {
        cx.state(State::Waiting(
            "Add your Twitch channel name in Settings → Stream".into(),
        ));
        return;
    };
    let mut failures = 0;
    while !cx.stopped() {
        cx.state(State::Connecting);
        if connect(cx, &login) {
            return;
        }
        failures += 1;
        if !cx.pause(backoff(failures)) {
            return;
        }
    }
}

/// One connection; `true` when stopped.
fn connect(cx: &Context<'_>, login: &str) -> bool {
    let mut socket = match ws::connect(URL) {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!(error = %e, "Twitch chat connection");
            return false;
        }
    };
    let nick = format!("justinfan{}", fastrand::u32(10_000..99_999));
    for line in [
        "CAP REQ :twitch.tv/tags twitch.tv/commands".to_owned(),
        "PASS SCHMOOPIIE".to_owned(),
        format!("NICK {nick}"),
        format!("JOIN #{login}"),
    ] {
        if !ws::send(&mut socket, &line) {
            return false;
        }
    }
    let mut heard = Instant::now();
    let third_party = super::ThirdParty::default();
    let mut room_id = String::new();
    loop {
        if cx.stopped() {
            let _ = socket.close(None);
            return true;
        }
        let text = match ws::next(&mut socket) {
            Next::Text(t) => t,
            Next::Idle => {
                if heard.elapsed() > SILENT_LIMIT {
                    return false;
                }
                continue;
            }
            Next::Closed => return false,
        };
        heard = Instant::now();
        for line in text.split("\r\n").filter(|l| !l.is_empty()) {
            if let Some(rest) = line.strip_prefix("PING") {
                let _ = ws::send(&mut socket, &format!("PONG{rest}"));
                continue;
            }
            match command(line) {
                // The end of the joined channel's member list: we're in.
                "366" => cx.state(State::Reading),
                "ROOMSTATE" => {
                    cx.state(State::Reading);
                    // The channel's id, for its 7TV, BetterTTV and FrankerFaceZ emotes.
                    if let Some(id) = tags(line).find(|(k, _)| *k == "room-id").map(|(_, v)| v)
                        && !id.is_empty()
                        && id != room_id
                    {
                        room_id = id.to_owned();
                        third_party.load(super::emotes::Owner::Twitch, room_id.clone());
                    }
                }
                "RECONNECT" => return false,
                "PRIVMSG" => {
                    if let Some(m) = parse(line) {
                        let parts = third_party.apply(if m.parts.is_empty() {
                            vec![super::emotes::Part::Text(m.text.clone())]
                        } else {
                            m.parts.clone()
                        });
                        cx.push(m.with_parts(parts));
                    }
                }
                _ => {}
            }
        }
    }
}

// ---- Follows, subs, gifts, cheers and raids (EventSub, with the connected account) ----

const EVENTSUB: &str = "wss://eventsub.wss.twitch.tv/ws";

enum EventsEnd {
    Stopped,
    /// Twitch asked to move to another address (subscriptions carry over).
    Moved(String),
    /// The sign-in can't receive events (connect Twitch again).
    NoPermission,
    Dropped,
}

/// Receives the connected channel's events while the chat is open. Quiet: problems only reach
/// the log, and the chat keeps working without them.
pub fn events(cx: &Context<'_>, session: &crate::twitch::Session) {
    let mut url = EVENTSUB.to_owned();
    let mut failures = 0;
    while !cx.stopped() {
        match events_connection(cx, session, &url) {
            EventsEnd::Stopped => return,
            EventsEnd::Moved(next) => url = next,
            EventsEnd::NoPermission => {
                tracing::info!("Twitch events need the account connected again (new permissions)");
                // Try again once the user connects again (a new sign-in).
                let before = session.account().map(|a| a.access);
                if !super::wait_for_change(cx, || session.account().map(|a| a.access), before) {
                    return;
                }
            }
            EventsEnd::Dropped => {
                url = EVENTSUB.to_owned();
                failures += 1;
                if !cx.pause(backoff(failures)) {
                    return;
                }
            }
        }
    }
}

fn events_connection(cx: &Context<'_>, session: &crate::twitch::Session, url: &str) -> EventsEnd {
    use serde_json::Value;
    let mut socket = match ws::connect(url) {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!(error = %e, "Twitch events connection");
            return EventsEnd::Dropped;
        }
    };
    let moved = url != EVENTSUB;
    let mut quiet_limit = Duration::from_secs(30);
    let mut heard = Instant::now();
    loop {
        if cx.stopped() {
            let _ = socket.close(None);
            return EventsEnd::Stopped;
        }
        let text = match ws::next(&mut socket) {
            Next::Text(t) => t,
            Next::Idle => {
                if heard.elapsed() > quiet_limit {
                    return EventsEnd::Dropped;
                }
                continue;
            }
            Next::Closed => return EventsEnd::Dropped,
        };
        heard = Instant::now();
        let Ok(message) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let kind = message["metadata"]["message_type"]
            .as_str()
            .unwrap_or_default();
        let payload = &message["payload"];
        match kind {
            "session_welcome" => {
                let keepalive = payload["session"]["keepalive_timeout_seconds"]
                    .as_u64()
                    .unwrap_or(10);
                quiet_limit = Duration::from_secs(keepalive + 10);
                // After a move the subscriptions come along; otherwise ask for them now.
                if !moved {
                    let id = payload["session"]["id"].as_str().unwrap_or_default();
                    match session.subscribe_events(id) {
                        Ok(()) => {}
                        Err(true) => return EventsEnd::NoPermission,
                        Err(false) => return EventsEnd::Dropped,
                    }
                }
            }
            "session_reconnect" => {
                if let Some(next) = payload["session"]["reconnect_url"].as_str() {
                    return EventsEnd::Moved(next.to_owned());
                }
            }
            "notification" => {
                let kind = message["metadata"]["subscription_type"]
                    .as_str()
                    .unwrap_or_default();
                if let Some(line) = event_line(kind, &payload["event"]) {
                    cx.push(line);
                }
            }
            _ => {}
        }
    }
}

/// The chat line for an EventSub notification (`None` for ones not shown).
fn event_line(kind: &str, e: &serde_json::Value) -> Option<Message> {
    let s = |key: &str| e[key].as_str().unwrap_or_default().to_owned();
    let n = |key: &str| e[key].as_u64().unwrap_or(0);
    let tier = match e["tier"].as_str() {
        Some("2000") => " (Tier 2)",
        Some("3000") => " (Tier 3)",
        _ => "",
    };
    let or_anonymous = |name: String| {
        if name.is_empty() {
            "Anonymous".to_owned()
        } else {
            name
        }
    };
    let with_message = |head: String| {
        let text = e["message"]["text"]
            .as_str()
            .or(e["message"].as_str())
            .unwrap_or_default();
        if text.is_empty() {
            head
        } else {
            format!("{head}: {text}")
        }
    };
    let (author, text) = match kind {
        "channel.follow" => (s("user_name"), "followed".to_owned()),
        // Gifted ones are announced once, by the gift event.
        "channel.subscribe" if e["is_gift"].as_bool() == Some(true) => return None,
        "channel.subscribe" => (s("user_name"), format!("subscribed{tier}")),
        "channel.subscription.message" => (
            s("user_name"),
            with_message(format!(
                "resubscribed for {} months{tier}",
                n("cumulative_months")
            )),
        ),
        "channel.subscription.gift" => {
            let total = n("total").max(1);
            let subs = if total == 1 {
                "a sub".to_owned()
            } else {
                format!("{total} subs")
            };
            (or_anonymous(s("user_name")), format!("gifted {subs}{tier}"))
        }
        "channel.cheer" => (
            or_anonymous(s("user_name")),
            with_message(format!("cheered {} bits", n("bits"))),
        ),
        "channel.raid" => (
            s("from_broadcaster_user_name"),
            format!("raided with {} viewers", n("viewers")),
        ),
        _ => return None,
    };
    Some(Message {
        platform: Platform::Twitch,
        color: super::name_color(&author),
        author,
        text,
        parts: Vec::new(),
        time: chrono::Local::now(),
        event: true,
    })
}

/// The command of a chat line (`@tags :prefix COMMAND params`).
fn command(line: &str) -> &str {
    let mut rest = line;
    if rest.starts_with('@') {
        rest = rest.split_once(' ').map_or("", |(_, r)| r);
    }
    if rest.starts_with(':') {
        rest = rest.split_once(' ').map_or("", |(_, r)| r);
    }
    rest.split(' ').next().unwrap_or_default()
}

/// The tags of a line (`@key=value;… `), values still escaped.
fn tags(line: &str) -> impl Iterator<Item = (&str, &str)> {
    line.strip_prefix('@')
        .and_then(|l| l.split_once(' '))
        .map_or("", |(tags, _)| tags)
        .split(';')
        .filter_map(|t| t.split_once('='))
}

/// A `PRIVMSG` line with tags.
fn parse(line: &str) -> Option<Message> {
    let (_, rest) = line.strip_prefix('@')?.split_once(' ')?;
    let tag = |key: &str| {
        tags(line)
            .find(|(k, _)| *k == key)
            .map(|(_, v)| unescape(v))
            .unwrap_or_default()
    };
    let (prefix, rest) = rest.strip_prefix(':')?.split_once(' ')?;
    let text = rest.split_once(" :")?.1;
    // "/me" messages arrive wrapped as \x01ACTION …\x01.
    let text = text
        .strip_prefix("\u{1}ACTION ")
        .and_then(|t| t.strip_suffix('\u{1}'))
        .unwrap_or(text)
        .to_owned();
    let mut author = tag("display-name");
    if author.is_empty() {
        author = prefix.split('!').next().unwrap_or_default().to_owned();
    }
    let color = Some(tag("color"))
        .filter(|c| c.len() == 7)
        .unwrap_or_else(|| super::name_color(&author));
    let parts = super::emotes::twitch_parts(&text, &tag("emotes"));
    Some(
        Message {
            platform: Platform::Twitch,
            author,
            color,
            text,
            parts: Vec::new(),
            time: chrono::Local::now(),
            event: false,
        }
        .with_parts(parts),
    )
}

/// Undoes the escaping of tag values.
fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some(':') => out.push(';'),
            Some('\\') => out.push('\\'),
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = "@badge-info=;color=#1E90FF;display-name=Some\\sOne;id=abc;mod=0 \
        :someone!someone@someone.tmi.twitch.tv PRIVMSG #chan :hello there: friend";

    #[test]
    fn chat_lines_become_messages() {
        let m = parse(LINE).unwrap();
        assert_eq!(m.author, "Some One");
        assert_eq!(m.color, "#1E90FF");
        assert_eq!(m.text, "hello there: friend");
        assert_eq!(command(LINE), "PRIVMSG");
    }

    #[test]
    fn emote_tags_become_parts() {
        let line = "@color=;display-name=A;emotes=25:0-4 :a!a@a.tmi.twitch.tv PRIVMSG #c :Kappa hi";
        let m = parse(line).unwrap();
        assert_eq!(m.text, "Kappa hi");
        assert!(
            matches!(&m.parts[0], crate::livechat::emotes::Part::Emote(e) if e.name == "Kappa")
        );
        // No emotes: no parts.
        assert!(parse(LINE).unwrap().parts.is_empty());
        let room = "@emote-only=0;room-id=12826;slow=0 :tmi.twitch.tv ROOMSTATE #c";
        assert_eq!(
            tags(room).find(|(k, _)| *k == "room-id").map(|(_, v)| v),
            Some("12826")
        );
    }

    #[test]
    fn me_messages_lose_their_wrapper() {
        let line = "@color=;display-name=A :a!a@a.tmi.twitch.tv PRIVMSG #c :\u{1}ACTION waves\u{1}";
        let m = parse(line).unwrap();
        assert_eq!(m.text, "waves");
        assert!(m.color.starts_with('#'));
    }

    #[test]
    fn notifications_become_event_lines() {
        let follow = serde_json::json!({ "user_name": "Ali" });
        assert_eq!(
            event_line("channel.follow", &follow).unwrap().text,
            "followed"
        );
        assert!(event_line("channel.follow", &follow).unwrap().event);
        let gift = serde_json::json!({ "user_name": null, "total": 5, "tier": "1000", "is_anonymous": true });
        let line = event_line("channel.subscription.gift", &gift).unwrap();
        assert_eq!(
            (line.author.as_str(), line.text.as_str()),
            ("Anonymous", "gifted 5 subs")
        );
        let resub = serde_json::json!({ "user_name": "Sam", "cumulative_months": 3, "tier": "2000",
            "message": { "text": "love it" } });
        assert_eq!(
            event_line("channel.subscription.message", &resub)
                .unwrap()
                .text,
            "resubscribed for 3 months (Tier 2): love it"
        );
        let raid = serde_json::json!({ "from_broadcaster_user_name": "Jo", "viewers": 40 });
        assert_eq!(
            event_line("channel.raid", &raid).unwrap().text,
            "raided with 40 viewers"
        );
        let gifted_sub = serde_json::json!({ "user_name": "X", "is_gift": true });
        assert!(event_line("channel.subscribe", &gifted_sub).is_none());
    }

    #[test]
    fn commands_without_tags() {
        assert_eq!(
            command(":tmi.twitch.tv 366 justinfan1 #c :End of /NAMES list"),
            "366"
        );
        assert_eq!(command("PING :tmi.twitch.tv"), "PING");
    }
}
