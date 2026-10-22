//! Emotes in chat messages: a message's pieces (text and emotes), read from Twitch's emote
//! positions and Kick's emote codes, plus the 7TV, BetterTTV and FrankerFaceZ emotes a channel
//! uses, matched word by word. The images themselves are fetched by the app (`crate::emotes`).

use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Settings → Chat → "7TV, BetterTTV and FrankerFaceZ emotes".
static THIRD_PARTY: AtomicBool = AtomicBool::new(true);

pub fn set_third_party(on: bool) {
    THIRD_PARTY.store(on, Ordering::Relaxed);
}

pub fn third_party_on() -> bool {
    THIRD_PARTY.load(Ordering::Relaxed)
}

/// Where an emote comes from (shown on hover).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmoteSource {
    Twitch,
    Kick,
    SevenTv,
    Bttv,
    Ffz,
}

impl EmoteSource {
    pub fn name(self) -> &'static str {
        match self {
            EmoteSource::Twitch => "Twitch",
            EmoteSource::Kick => "Kick",
            EmoteSource::SevenTv => "7TV",
            EmoteSource::Bttv => "BetterTTV",
            EmoteSource::Ffz => "FrankerFaceZ",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Emote {
    pub name: String,
    /// The image (PNG, GIF or WebP; animated ones are decoded frame by frame).
    pub url: String,
    pub source: EmoteSource,
}

/// One piece of a message.
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Text(String),
    Emote(Emote),
}

/// Whether `parts` has any emote (otherwise the plain text says it all).
pub fn has_emotes(parts: &[Part]) -> bool {
    parts.iter().any(|p| matches!(p, Part::Emote(_)))
}

// ---- Platform emotes ----

/// Splits a Twitch message by its `emotes` tag (`id:start-end,start-end/id:…`). Positions count
/// characters (code points), not bytes.
pub fn twitch_parts(text: &str, tag: &str) -> Vec<Part> {
    let chars: Vec<char> = text.chars().collect();
    let mut spans: Vec<(usize, usize, &str)> = Vec::new();
    for entry in tag.split('/').filter(|e| !e.is_empty()) {
        let Some((id, ranges)) = entry.split_once(':') else {
            continue;
        };
        for range in ranges.split(',') {
            if let Some((a, b)) = range.split_once('-')
                && let (Ok(a), Ok(b)) = (a.parse::<usize>(), b.parse::<usize>())
                && a <= b
                && b < chars.len()
            {
                spans.push((a, b, id));
            }
        }
    }
    spans.sort_by_key(|s| s.0);
    let mut parts = Vec::new();
    let mut at = 0;
    for (a, b, id) in spans {
        if a < at {
            continue; // overlapping ranges: keep the first
        }
        if a > at {
            parts.push(Part::Text(chars[at..a].iter().collect()));
        }
        parts.push(Part::Emote(Emote {
            name: chars[a..=b].iter().collect(),
            url: format!("https://static-cdn.jtvnw.net/emoticons/v2/{id}/default/dark/2.0"),
            source: EmoteSource::Twitch,
        }));
        at = b + 1;
    }
    if at < chars.len() {
        parts.push(Part::Text(chars[at..].iter().collect()));
    }
    parts
}

/// Splits Kick message content by its `[emote:id:name]` codes.
pub fn kick_parts(content: &str) -> Vec<Part> {
    let mut parts = Vec::new();
    let mut rest = content;
    while let Some(start) = rest.find("[emote:") {
        let tail = &rest[start..];
        let Some(end) = tail.find(']') else { break };
        let inner = &tail[7..end];
        let Some((id, name)) = inner
            .split_once(':')
            .filter(|(id, _)| id.bytes().all(|b| b.is_ascii_digit()))
        else {
            // Not an emote code after all: keep it as text.
            push_text(&mut parts, &rest[..start + end + 1]);
            rest = &tail[end + 1..];
            continue;
        };
        push_text(&mut parts, &rest[..start]);
        parts.push(Part::Emote(Emote {
            name: name.to_owned(),
            url: format!("https://files.kick.com/emotes/{id}/fullsize"),
            source: EmoteSource::Kick,
        }));
        rest = &tail[end + 1..];
    }
    push_text(&mut parts, rest);
    parts
}

/// The plain text of `parts`, emotes by name.
pub fn plain(parts: &[Part]) -> String {
    parts
        .iter()
        .map(|p| match p {
            Part::Text(t) => t.as_str(),
            Part::Emote(e) => e.name.as_str(),
        })
        .collect()
}

fn push_text(parts: &mut Vec<Part>, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(Part::Text(last)) = parts.last_mut() {
        last.push_str(text);
    } else {
        parts.push(Part::Text(text.to_owned()));
    }
}

// ---- 7TV, BetterTTV and FrankerFaceZ ----

/// A channel's third-party emotes by name (the channel's own win over the global ones).
#[derive(Clone, Debug, Default)]
pub struct Sets {
    by_name: HashMap<String, Emote>,
}

impl Sets {
    /// Shows matching words in the text parts as emotes.
    pub fn apply(&self, parts: Vec<Part>) -> Vec<Part> {
        if self.by_name.is_empty() {
            return parts;
        }
        let mut out = Vec::with_capacity(parts.len());
        for part in parts {
            let Part::Text(text) = part else {
                out.push(part);
                continue;
            };
            // Words split on spaces; the spaces stay in the text around them.
            let mut word_start = None;
            let mut pending = 0;
            for (i, c) in text
                .char_indices()
                .chain(std::iter::once((text.len(), ' ')))
            {
                if c.is_whitespace() {
                    if let Some(s) = word_start.take()
                        && let Some(emote) = self.by_name.get(&text[s..i])
                    {
                        push_text(&mut out, &text[pending..s]);
                        out.push(Part::Emote(emote.clone()));
                        pending = i;
                    }
                } else if word_start.is_none() {
                    word_start = Some(i);
                }
            }
            push_text(&mut out, &text[pending..]);
        }
        out
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    /// Adds `emotes` where no emote of that name is known yet.
    fn add(&mut self, emotes: Vec<Emote>) {
        for e in emotes {
            self.by_name.entry(e.name.clone()).or_insert(e);
        }
    }
}

/// Which platform's channel id is meant (7TV and BetterTTV link accounts per platform).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Owner {
    Twitch,
    Kick,
}

/// The third-party emotes for a channel: its own sets, then the global ones. Requests that fail
/// leave that service out (logged); the chat works without them.
pub fn load(owner: Owner, channel_id: &str) -> Sets {
    let mut sets = Sets::default();
    match owner {
        Owner::Twitch => {
            sets.add(
                get(&format!("https://7tv.io/v3/users/twitch/{channel_id}"))
                    .map_or_else(Vec::new, |b| seventv_user(&b)),
            );
            sets.add(
                get(&format!(
                    "https://api.betterttv.net/3/cached/users/twitch/{channel_id}"
                ))
                .map_or_else(Vec::new, |b| bttv_user(&b)),
            );
            sets.add(
                get(&format!(
                    "https://api.frankerfacez.com/v1/room/id/{channel_id}"
                ))
                .map_or_else(Vec::new, |b| ffz_sets(&b)),
            );
        }
        Owner::Kick => {
            sets.add(
                get(&format!("https://7tv.io/v3/users/kick/{channel_id}"))
                    .map_or_else(Vec::new, |b| seventv_user(&b)),
            );
        }
    }
    sets.add(global(owner));
    tracing::info!(
        ?owner,
        channel_id,
        emotes = sets.len(),
        "third-party emotes"
    );
    sets
}

/// The global sets, fetched once per platform and kept for the session.
fn global(owner: Owner) -> Vec<Emote> {
    static GLOBAL: OnceLock<Mutex<HashMap<Owner, Arc<Vec<Emote>>>>> = OnceLock::new();
    let cache = GLOBAL.get_or_init(Default::default);
    if let Some(found) = cache.lock().ok().and_then(|c| c.get(&owner).cloned()) {
        return found.to_vec();
    }
    let mut all =
        get("https://7tv.io/v3/emote-sets/global").map_or_else(Vec::new, |b| seventv_set(&b));
    if owner == Owner::Twitch {
        all.extend(
            get("https://api.betterttv.net/3/cached/emotes/global")
                .map_or_else(Vec::new, |b| bttv_list(&b)),
        );
        all.extend(
            get("https://api.frankerfacez.com/v1/set/global")
                .map_or_else(Vec::new, |b| ffz_sets(&b)),
        );
    }
    if let Ok(mut c) = cache.lock() {
        c.insert(owner, Arc::new(all.clone()));
    }
    all
}

fn get(url: &str) -> Option<Value> {
    let mut response = crate::http::agent()
        .get(url)
        .call()
        .map_err(|e| tracing::debug!(error = %e, url, "emote set"))
        .ok()?;
    if response.status().as_u16() != 200 {
        tracing::debug!(status = response.status().as_u16(), url, "emote set");
        return None;
    }
    response.body_mut().read_json().ok()
}

/// A 7TV user: their active emote set.
fn seventv_user(body: &Value) -> Vec<Emote> {
    seventv_set(&body["emote_set"])
}

fn seventv_set(set: &Value) -> Vec<Emote> {
    set["emotes"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|e| {
                    let host = e["data"]["host"]["url"].as_str()?;
                    Some(Emote {
                        name: e["name"].as_str()?.to_owned(),
                        url: format!("https:{host}/2x.webp"),
                        source: EmoteSource::SevenTv,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A BetterTTV channel: its own and its shared emotes.
fn bttv_user(body: &Value) -> Vec<Emote> {
    let mut out = bttv_list(&body["channelEmotes"]);
    out.extend(bttv_list(&body["sharedEmotes"]));
    out
}

fn bttv_list(list: &Value) -> Vec<Emote> {
    list.as_array()
        .map(|list| {
            list.iter()
                .filter_map(|e| {
                    Some(Emote {
                        name: e["code"].as_str()?.to_owned(),
                        url: format!(
                            "https://cdn.betterttv.net/emote/{}/2x.webp",
                            e["id"].as_str()?
                        ),
                        source: EmoteSource::Bttv,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// FrankerFaceZ room or global sets: `sets.{id}.emoticons`.
fn ffz_sets(body: &Value) -> Vec<Emote> {
    let Some(sets) = body["sets"].as_object() else {
        return Vec::new();
    };
    sets.values()
        .flat_map(|set| set["emoticons"].as_array().cloned().unwrap_or_default())
        .filter_map(|e| {
            let urls = &e["urls"];
            let url = urls["2"].as_str().or(urls["1"].as_str())?;
            Some(Emote {
                name: e["name"].as_str()?.to_owned(),
                url: if url.starts_with("//") {
                    format!("https:{url}")
                } else {
                    url.to_owned()
                },
                source: EmoteSource::Ffz,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn names(parts: &[Part]) -> Vec<String> {
        parts
            .iter()
            .map(|p| match p {
                Part::Text(t) => format!("text:{t}"),
                Part::Emote(e) => format!("emote:{}", e.name),
            })
            .collect()
    }

    #[test]
    fn twitch_positions_count_characters() {
        // "Kappa" at 0-4 and 12-16; the é and the emoji are one character each, more bytes.
        let text = "Kappa café 😀 Kappa!";
        let parts = twitch_parts(text, "25:0-4,13-17");
        assert_eq!(
            names(&parts),
            ["emote:Kappa", "text: café 😀 ", "emote:Kappa", "text:!"]
        );
        let Part::Emote(e) = &parts[0] else { panic!() };
        assert_eq!(
            e.url,
            "https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/2.0"
        );
        assert_eq!(plain(&parts), text);
    }

    #[test]
    fn twitch_ignores_ranges_outside_the_text() {
        assert_eq!(names(&twitch_parts("hi", "25:0-9")), ["text:hi"]);
        assert_eq!(names(&twitch_parts("hi", "")), ["text:hi"]);
    }

    #[test]
    fn kick_codes_become_emotes() {
        let parts = kick_parts("[emote:37234:Prayge] MAX WIN [emote:37234:Prayge]");
        assert_eq!(
            names(&parts),
            ["emote:Prayge", "text: MAX WIN ", "emote:Prayge"]
        );
        let Part::Emote(e) = &parts[0] else { panic!() };
        assert_eq!(e.url, "https://files.kick.com/emotes/37234/fullsize");
        assert_eq!(
            names(&kick_parts("broken [emote:1:x")),
            ["text:broken [emote:1:x"]
        );
        assert_eq!(
            names(&kick_parts("[emote:abc:x] hi")),
            ["text:[emote:abc:x] hi"]
        );
    }

    #[test]
    fn third_party_words_match_whole_words_only() {
        let mut sets = Sets::default();
        sets.add(vec![Emote {
            name: "catJAM".into(),
            url: "u".into(),
            source: EmoteSource::SevenTv,
        }]);
        let parts = sets.apply(vec![Part::Text("catJAM vibes catJAMs catJAM".into())]);
        assert_eq!(
            names(&parts),
            ["emote:catJAM", "text: vibes catJAMs ", "emote:catJAM"]
        );
        // Platform emotes are left alone.
        let kept = sets.apply(kick_parts("[emote:1:catJAM]"));
        assert!(matches!(&kept[0], Part::Emote(e) if e.source == EmoteSource::Kick));
    }

    #[test]
    fn the_channel_set_wins_over_the_global_one() {
        let mut sets = Sets::default();
        let emote = |url: &str| Emote {
            name: "Clap".into(),
            url: url.into(),
            source: EmoteSource::SevenTv,
        };
        sets.add(vec![emote("channel")]);
        sets.add(vec![emote("global")]);
        let Part::Emote(e) = &sets.apply(vec![Part::Text("Clap".into())])[0] else {
            panic!()
        };
        assert_eq!(e.url, "channel");
    }

    #[test]
    fn service_replies_are_read() {
        let seventv = json!({ "emote_set": { "emotes": [
            { "name": "catJAM", "data": { "host": { "url": "//cdn.7tv.app/emote/60ae" } } }
        ] } });
        assert_eq!(
            seventv_user(&seventv)[0].url,
            "https://cdn.7tv.app/emote/60ae/2x.webp"
        );
        let bttv = json!({ "channelEmotes": [{ "id": "5f1b", "code": "pepeD" }],
            "sharedEmotes": [{ "id": "5e0f", "code": "ddHuh" }] });
        let b = bttv_user(&bttv);
        assert_eq!(
            (b[1].name.as_str(), b[0].url.as_str()),
            ("ddHuh", "https://cdn.betterttv.net/emote/5f1b/2x.webp")
        );
        let ffz = json!({ "sets": { "3": { "emoticons": [
            { "name": "LilZ", "urls": { "1": "//cdn.frankerfacez.com/emote/28136/1",
                                        "2": "//cdn.frankerfacez.com/emote/28136/2" } }
        ] } } });
        assert_eq!(
            ffz_sets(&ffz)[0].url,
            "https://cdn.frankerfacez.com/emote/28136/2"
        );
        assert!(seventv_user(&json!({})).is_empty());
    }
}
