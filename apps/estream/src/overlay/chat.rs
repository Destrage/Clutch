//! The Chat overlay source: the combined chat from every platform on the canvas. The page
//! (`chat.html`) runs in the browser source; this side builds its settings, decides which
//! messages each source shows, and turns messages and moderator removals into page messages.

use crate::livechat::emotes::Part;
use crate::livechat::{Message, Platform, Removal};
use estream_engine::{PropKind, Property, PropertySchema, SelectOption, SourceType};
use serde_json::{Value, json};

pub const KIND: &str = "chat_overlay";

/// Chat bots most channels run; their lines are hidden unless the list is changed.
const BOTS: &str = "Nightbot, StreamElements, Streamlabs, Moobot, Fossabot, BotRix, Sery_Bot";

fn color(r: u8, g: u8, b: u8, a: u8) -> Value {
    json!({ "r": r, "g": g, "b": b, "a": a })
}

fn schema(_: &Value) -> PropertySchema {
    PropertySchema::new(vec![
        Property::int("width", "Width", 100, 3840, 500),
        Property::int("height", "Height", 100, 2160, 700),
        Property::select(
            "style",
            "Style",
            vec![
                SelectOption::new("boxed", "Boxed lines"),
                SelectOption::new("plain", "Text with outline"),
                SelectOption::new("compact", "Compact"),
            ],
            "boxed",
        ),
        Property::new("font", "Font", PropKind::Font, "Segoe UI"),
        Property::int("size", "Text size", 10, 120, 26),
        Property::new(
            "text_color",
            "Text colour",
            PropKind::Color,
            color(255, 255, 255, 255),
        ),
        Property::new(
            "background",
            "Line background",
            PropKind::Color,
            color(16, 18, 24, 170),
        )
        .hint("Boxed lines only"),
        Property::int("radius", "Corner rounding", 0, 40, 10),
        Property::int("max_lines", "Most messages shown", 1, 50, 12),
        Property::int("fade_after", "Fade out after (seconds)", 0, 3600, 60)
            .hint("0: messages stay until newer ones push them out"),
        Property::bool("badges", "Platform badges", true),
        Property::bool("twitch", "Twitch", true),
        Property::bool("kick", "Kick", true),
        Property::bool("youtube", "YouTube", true),
        Property::bool("events", "Follows, subs and other events", true),
        Property::bool(
            "hide_commands",
            "Hide commands (messages starting with !)",
            true,
        ),
        Property::text("hide_names", "Hide these names", false, BOTS)
            .hint("Separated by commas, e.g. chat bots"),
    ])
}

/// Browser source settings for a Chat overlay's page.
fn page(settings: &Value) -> Value {
    let int = |k: &str, d: u64| settings.get(k).and_then(Value::as_u64).unwrap_or(d);
    let source = settings
        .get("_source_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    json!({
        "url": super::page_url("chat", source),
        "width": int("width", 500),
        "height": int("height", 700),
        "fps": 30,
        "css": "body { background-color: rgba(0, 0, 0, 0); margin: 0; overflow: hidden; }",
    })
}

pub fn source_type() -> SourceType {
    estream_capture::media::browser::page_source_type(
        KIND,
        "Chat overlay",
        "The chat from Twitch, Kick and YouTube on stream, in one list",
        schema,
        page,
    )
    .without_audio()
}

fn flag(settings: &Value, key: &str) -> bool {
    settings.get(key).and_then(Value::as_bool).unwrap_or(true)
}

/// The page's settings for one Chat overlay source.
pub fn config(settings: &Value) -> Value {
    let int = |k: &str, d: u64| settings.get(k).and_then(Value::as_u64).unwrap_or(d);
    let rgba = |k: &str, d: &str| {
        settings
            .get(k)
            .filter(|v| v.is_object())
            .map(|c| {
                let n = |x: &str| c.get(x).and_then(Value::as_u64).unwrap_or(255);
                format!(
                    "rgba({}, {}, {}, {:.3})",
                    n("r"),
                    n("g"),
                    n("b"),
                    n("a") as f64 / 255.0
                )
            })
            .unwrap_or_else(|| d.to_owned())
    };
    json!({
        "style": settings.get("style").and_then(Value::as_str).unwrap_or("boxed"),
        "font": settings.get("font").and_then(Value::as_str).unwrap_or("Segoe UI"),
        "size": int("size", 26),
        "text_color": rgba("text_color", "#ffffff"),
        "background": rgba("background", "rgba(16, 18, 24, 0.667)"),
        "radius": int("radius", 10),
        "max_lines": int("max_lines", 12),
        "fade_after": int("fade_after", 60),
        "badges": flag(settings, "badges"),
    })
}

/// Whether a Chat overlay with these settings shows `m` (removals always go through).
pub fn shows(settings: &Value, m: &Message) -> bool {
    if m.removal.is_some() {
        return true;
    }
    let platform = match m.platform {
        Platform::Twitch => "twitch",
        Platform::Kick => "kick",
        Platform::YouTube => "youtube",
    };
    if !flag(settings, platform) {
        return false;
    }
    if m.event.is_some() {
        return flag(settings, "events");
    }
    if flag(settings, "hide_commands") && m.text.trim_start().starts_with('!') {
        return false;
    }
    let hidden = settings
        .get("hide_names")
        .and_then(Value::as_str)
        .unwrap_or(BOTS);
    !hidden
        .split([',', '\n'])
        .map(str::trim)
        .any(|n| !n.is_empty() && n.eq_ignore_ascii_case(&m.author))
}

/// The page message for a chat line or a removal.
pub fn page_message(m: &Message) -> String {
    let platform = m.platform.key();
    match &m.removal {
        Some(Removal::Message(id)) => json!({ "type": "remove", "platform": platform, "id": id }),
        Some(Removal::User(name)) => {
            json!({ "type": "clear_user", "platform": platform, "name": name })
        }
        Some(Removal::All) => json!({ "type": "clear", "platform": platform }),
        None => {
            let parts: Vec<Value> = if m.parts.is_empty() {
                vec![json!({ "text": m.text })]
            } else {
                m.parts
                    .iter()
                    .map(|p| match p {
                        Part::Text(t) => json!({ "text": t }),
                        Part::Emote(e) => json!({ "emote": e.url, "alt": e.name }),
                    })
                    .collect()
            };
            json!({
                "type": "chat",
                "id": m.id,
                "platform": platform,
                "name": m.author,
                "color": m.color,
                "parts": parts,
                "event": m.event.is_some(),
                "time": m.time.timestamp_millis(),
            })
        }
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(platform: Platform, author: &str, text: &str) -> Message {
        Message {
            platform,
            author: author.into(),
            color: "#8ec5ff".into(),
            text: text.into(),
            parts: Vec::new(),
            time: chrono::Local::now(),
            event: None,
            id: "m1".into(),
            removal: None,
        }
    }

    #[test]
    fn filters_follow_the_settings() {
        let defaults = Value::Null;
        assert!(shows(&defaults, &message(Platform::Twitch, "ali", "gg")));
        assert!(!shows(
            &defaults,
            &message(Platform::Twitch, "ali", "!discord")
        ));
        assert!(!shows(
            &defaults,
            &message(Platform::Kick, "nightbot", "Follow us")
        ));
        let custom = json!({ "kick": false, "hide_commands": false, "hide_names": "botty, other" });
        assert!(!shows(&custom, &message(Platform::Kick, "ali", "hi")));
        assert!(shows(
            &custom,
            &message(Platform::Twitch, "ali", "!discord")
        ));
        assert!(shows(&custom, &message(Platform::Twitch, "Nightbot", "hi")));
        assert!(!shows(&custom, &message(Platform::YouTube, "Botty", "hi")));

        let mut event = message(Platform::Twitch, "ali", "followed");
        event.event = Some(crate::livechat::event::StreamEvent::new(
            crate::livechat::event::StreamEventKind::Follow,
            "ali",
        ));
        assert!(shows(&defaults, &event));
        assert!(!shows(&json!({ "events": false }), &event));
    }

    #[test]
    fn messages_and_removals_become_page_messages() {
        let mut m = message(Platform::Twitch, "ali", "Kappa hi");
        m.parts = vec![
            Part::Emote(crate::livechat::emotes::Emote {
                name: "Kappa".into(),
                url: "https://e/25".into(),
                source: crate::livechat::emotes::EmoteSource::Twitch,
            }),
            Part::Text(" hi".into()),
        ];
        let v: Value = serde_json::from_str(&page_message(&m)).unwrap();
        assert_eq!(v["type"], "chat");
        assert_eq!(v["id"], "m1");
        assert_eq!(v["platform"], "twitch");
        assert_eq!(
            v["parts"][0],
            json!({ "emote": "https://e/25", "alt": "Kappa" })
        );
        assert_eq!(v["parts"][1], json!({ "text": " hi" }));

        let plain: Value =
            serde_json::from_str(&page_message(&message(Platform::Kick, "a", "<b>x</b>"))).unwrap();
        assert_eq!(plain["parts"], json!([{ "text": "<b>x</b>" }]));

        let mut removal = message(Platform::Kick, "", "");
        removal.removal = Some(Removal::User("ali".into()));
        assert!(shows(&json!({ "kick": false }), &removal));
        let v: Value = serde_json::from_str(&page_message(&removal)).unwrap();
        assert_eq!(
            v,
            json!({ "type": "clear_user", "platform": "kick", "name": "ali" })
        );
    }
}
