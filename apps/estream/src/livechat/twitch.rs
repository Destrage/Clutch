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
                "366" | "ROOMSTATE" => cx.state(State::Reading),
                "RECONNECT" => return false,
                "PRIVMSG" => {
                    if let Some(m) = parse(line) {
                        cx.push(m);
                    }
                }
                _ => {}
            }
        }
    }
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

/// A `PRIVMSG` line with tags.
fn parse(line: &str) -> Option<Message> {
    let (tags, rest) = line.strip_prefix('@')?.split_once(' ')?;
    let tag = |key: &str| {
        tags.split(';')
            .filter_map(|t| t.split_once('='))
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
    Some(Message {
        platform: Platform::Twitch,
        author,
        color,
        text,
        time: chrono::Local::now(),
    })
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
    fn me_messages_lose_their_wrapper() {
        let line = "@color=;display-name=A :a!a@a.tmi.twitch.tv PRIVMSG #c :\u{1}ACTION waves\u{1}";
        let m = parse(line).unwrap();
        assert_eq!(m.text, "waves");
        assert!(m.color.starts_with('#'));
    }

    #[test]
    fn commands_without_tags() {
        assert_eq!(command(":tmi.twitch.tv 366 justinfan1 #c :End of /NAMES list"), "366");
        assert_eq!(command("PING :tmi.twitch.tv"), "PING");
    }
}
