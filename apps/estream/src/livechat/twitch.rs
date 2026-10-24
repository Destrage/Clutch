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
                "CLEARMSG" | "CLEARCHAT" => {
                    if let Some(r) = removal(line) {
                        cx.push(Message::removal(Platform::Twitch, r));
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
    let message = e["message"]["text"]
        .as_str()
        .or(e["message"].as_str())
        .unwrap_or_default()
        .to_owned();
    let with_message = |head: String| {
        if message.is_empty() {
            head
        } else {
            format!("{head}: {message}")
        }
    };
    let tier_name = tier.trim().trim_start_matches('(').trim_end_matches(')');
    use super::event::{StreamEvent, StreamEventKind as K};
    let (author, text, event) = match kind {
        "channel.follow" => {
            let name = s("user_name");
            (
                name.clone(),
                "followed".to_owned(),
                StreamEvent::new(K::Follow, name),
            )
        }
        // Gifted ones are announced once, by the gift event.
        "channel.subscribe" if e["is_gift"].as_bool() == Some(true) => return None,
        "channel.subscribe" => {
            let name = s("user_name");
            let event = StreamEvent::new(K::Sub, name.clone()).tier(tier_name);
            (name, format!("subscribed{tier}"), event)
        }
        "channel.subscription.message" => {
            let name = s("user_name");
            let months = n("cumulative_months");
            let event = StreamEvent::new(K::Resub, name.clone())
                .months(months as u32)
                .tier(tier_name)
                .message(message.clone());
            let text = with_message(format!("resubscribed for {months} months{tier}"));
            (name, text, event)
        }
        "channel.subscription.gift" => {
            let total = n("total").max(1);
            let subs = if total == 1 {
                "a sub".to_owned()
            } else {
                format!("{total} subs")
            };
            let name = or_anonymous(s("user_name"));
            let event = StreamEvent::new(K::Gift, name.clone())
                .amount(total as f64)
                .tier(tier_name);
            (name, format!("gifted {subs}{tier}"), event)
        }
        "channel.cheer" => {
            let name = or_anonymous(s("user_name"));
            let bits = n("bits");
            let event = StreamEvent::new(K::Bits, name.clone())
                .amount(bits as f64)
                .message(message.clone());
            (name, with_message(format!("cheered {bits} bits")), event)
        }
        "channel.raid" => {
            let name = s("from_broadcaster_user_name");
            let viewers = n("viewers");
            let event = StreamEvent::new(K::Raid, name.clone()).amount(viewers as f64);
            (name, format!("raided with {viewers} viewers"), event)
        }
        _ => return None,
    };
    Some(Message {
        platform: Platform::Twitch,
        color: super::name_color(&author),
        author,
        text,
        parts: Vec::new(),
        time: chrono::Local::now(),
        event: Some(event),
        id: String::new(),
        removal: None,
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
            event: None,
            id: tag("id"),
            removal: None,
        }
        .with_parts(parts),
    )
}

/// A `CLEARMSG` (one message deleted) or `CLEARCHAT` (a viewer timed out or banned, or the
/// whole chat cleared) line.
fn removal(line: &str) -> Option<super::Removal> {
    use super::Removal;
    match command(line) {
        "CLEARMSG" => tags(line)
            .find(|(k, _)| *k == "target-msg-id")
            .map(|(_, id)| Removal::Message(id.to_owned())),
        "CLEARCHAT" => Some(match line.split_once(" CLEARCHAT ")?.1.split_once(" :") {
            Some((_, login)) if !login.trim().is_empty() => Removal::User(login.trim().to_owned()),
            _ => Removal::All,
        }),
        _ => None,
    }
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
        assert!(
            event_line("channel.follow", &follow)
                .unwrap()
                .event
                .is_some()
        );
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
    fn notifications_carry_typed_events() {
        use crate::livechat::event::StreamEventKind as K;
        let event =
            |kind: &str, e: serde_json::Value| event_line(kind, &e).and_then(|m| m.event).unwrap();
        let resub = event(
            "channel.subscription.message",
            serde_json::json!({ "user_name": "Sam", "cumulative_months": 3, "tier": "2000",
                "message": { "text": "love it" } }),
        );
        assert_eq!(resub.kind, K::Resub);
        assert_eq!(resub.name, "Sam");
        assert_eq!(resub.months, Some(3));
        assert_eq!(resub.tier.as_deref(), Some("Tier 2"));
        assert_eq!(resub.message.as_deref(), Some("love it"));
        let gift = event(
            "channel.subscription.gift",
            serde_json::json!({ "user_name": null, "total": 5, "tier": "1000" }),
        );
        assert_eq!(
            (gift.kind, gift.name.as_str(), gift.amount),
            (K::Gift, "Anonymous", Some(5.0))
        );
        assert_eq!(gift.tier, None, "Tier 1 is the default and not named");
        let bits = event(
            "channel.cheer",
            serde_json::json!({ "user_name": "Jo", "bits": 500 }),
        );
        assert_eq!(
            (bits.kind, bits.amount, bits.message),
            (K::Bits, Some(500.0), None)
        );
        let raid = event(
            "channel.raid",
            serde_json::json!({ "from_broadcaster_user_name": "Big", "viewers": 40 }),
        );
        assert_eq!((raid.kind, raid.amount), (K::Raid, Some(40.0)));
    }

    #[test]
    fn moderation_lines_become_removals() {
        use super::super::Removal;
        let one = "@login=ali;room-id=;target-msg-id=abc-123;tmi-sent-ts=1 :tmi.twitch.tv CLEARMSG #chan :bad words";
        assert_eq!(removal(one), Some(Removal::Message("abc-123".into())));
        let user =
            "@ban-duration=600;room-id=1;target-user-id=2 :tmi.twitch.tv CLEARCHAT #chan :ali";
        assert_eq!(removal(user), Some(Removal::User("ali".into())));
        let all = "@room-id=1;tmi-sent-ts=1 :tmi.twitch.tv CLEARCHAT #chan";
        assert_eq!(removal(all), Some(Removal::All));
        let line = "@color=;display-name=A;id=m-1 :a!a@a.tmi.twitch.tv PRIVMSG #c :hi";
        assert_eq!(parse(line).unwrap().id, "m-1");
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
