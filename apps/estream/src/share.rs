//! Sharing the stream: watch links, the share message, share pages and Discord webhooks.

use estream_media::Service;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// The share message used until the user writes their own.
pub const DEFAULT_TEMPLATE: &str = "Live now: {title}\n{links}";

/// The placeholders a template can use, for the settings hint.
pub const PLACEHOLDERS: &str = "{title} {category} {links} {twitch} {youtube} {kick}";

/// No automatic announcement to a webhook within this long of the last one.
pub const ANNOUNCE_GAP: Duration = Duration::from_secs(10 * 60);

/// Discord refuses longer messages.
const DISCORD_MAX_CHARS: usize = 2000;

/// A watch link for one streamed destination.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub service: Service,
    /// The destination's name.
    pub dest: String,
    pub url: String,
}

pub fn platform_name(service: Service) -> &'static str {
    match service {
        Service::Twitch => "Twitch",
        Service::YouTube => "YouTube",
        Service::Kick => "Kick",
        Service::Facebook => "Facebook",
        Service::Custom => "Custom",
    }
}

/// The watch link of a destination: `Ok(None)` for platforms without one (custom servers,
/// Facebook), `Err` with where to add what's missing. `youtube` is the broadcast's watch link
/// from the Go live window, if any; else a live link in `channel` is used.
pub fn watch_link(
    service: Service,
    channel: &str,
    youtube: Option<&str>,
) -> Result<Option<String>, &'static str> {
    match service {
        Service::Twitch => crate::chat::channel_name(channel, "twitch.tv")
            .map(|n| Some(format!("https://www.twitch.tv/{n}")))
            .ok_or("Add your Twitch channel name in Settings → Stream to share a link"),
        Service::Kick => crate::chat::channel_name(channel, "kick.com")
            .map(|n| Some(format!("https://kick.com/{n}")))
            .ok_or("Add your Kick channel name in Settings → Stream to share a link"),
        Service::YouTube => youtube
            .and_then(crate::chat::youtube_video_id)
            .or_else(|| crate::chat::youtube_video_id(channel))
            .map(|id| Some(format!("https://www.youtube.com/watch?v={id}")))
            .ok_or(
                "YouTube: go live through the Go live window, or paste your live link in Settings → Stream",
            ),
        Service::Facebook | Service::Custom => Ok(None),
    }
}

/// What a template is filled with.
#[derive(Clone, Debug, Default)]
pub struct Details<'a> {
    pub title: &'a str,
    pub category: &'a str,
    pub links: &'a [Link],
}

/// Fills `template`. A line whose placeholders all came out empty is left out, so a stream
/// without a title doesn't say a bare "Live now: ".
pub fn fill(template: &str, details: &Details) -> String {
    let first = |service: Service| {
        details
            .links
            .iter()
            .find(|l| l.service == service)
            .map_or(String::new(), |l| l.url.clone())
    };
    let links = links_text(details.links);
    let values: [(&str, String); 6] = [
        ("{title}", details.title.trim().to_owned()),
        ("{category}", details.category.trim().to_owned()),
        ("{links}", links),
        ("{twitch}", first(Service::Twitch)),
        ("{youtube}", first(Service::YouTube)),
        ("{kick}", first(Service::Kick)),
    ];
    let mut out = Vec::new();
    for line in template.lines() {
        let used: Vec<&(&str, String)> = values.iter().filter(|(k, _)| line.contains(k)).collect();
        if !used.is_empty() && used.iter().all(|(_, v)| v.is_empty()) {
            continue;
        }
        let mut filled = line.to_owned();
        for (k, v) in &used {
            filled = filled.replace(k, v);
        }
        out.push(filled);
    }
    out.join("\n").trim().to_owned()
}

/// One link per line, named by platform when there are several (and by destination when a
/// platform has more than one).
fn links_text(links: &[Link]) -> String {
    if links.len() == 1 {
        return links[0].url.clone();
    }
    links
        .iter()
        .map(|l| {
            let same = links.iter().filter(|o| o.service == l.service).count();
            if same > 1 {
                format!("{} ({}): {}", platform_name(l.service), l.dest, l.url)
            } else {
                format!("{}: {}", platform_name(l.service), l.url)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A site's share page with `text` filled in. Reddit takes a link and a title instead.
pub fn share_page(site: &str, text: &str, title: &str, link: Option<&str>) -> Option<String> {
    let e = encode;
    Some(match site {
        "x" => format!("https://x.com/intent/post?text={}", e(text)),
        "bluesky" => format!("https://bsky.app/intent/compose?text={}", e(text)),
        "whatsapp" => format!("https://wa.me/?text={}", e(text)),
        "reddit" => {
            let title = if title.trim().is_empty() {
                text.lines().next().unwrap_or_default()
            } else {
                title.trim()
            };
            match link {
                Some(url) => format!(
                    "https://www.reddit.com/submit?url={}&title={}",
                    e(url),
                    e(title)
                ),
                None => format!("https://www.reddit.com/submit?title={}", e(title)),
            }
        }
        _ => return None,
    })
}

/// Percent-encodes everything but unreserved characters (RFC 3986).
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

// ------------------------------------------------------------------ Discord

/// Whether `url` is a Discord webhook URL (`https://discord.com/api/webhooks/<id>/<token>`).
pub fn is_webhook_url(url: &str) -> bool {
    let Some(rest) = url.trim().strip_prefix("https://") else {
        return false;
    };
    let Some((host, path)) = rest.split_once('/') else {
        return false;
    };
    let host_ok = matches!(
        host,
        "discord.com"
            | "discordapp.com"
            | "ptb.discord.com"
            | "canary.discord.com"
            | "ptb.discordapp.com"
            | "canary.discordapp.com"
    );
    let mut parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    // An API version ("api/v10/webhooks/…") is fine too.
    if parts.len() == 5
        && parts[1].starts_with('v')
        && parts[1][1..].bytes().all(|b| b.is_ascii_digit())
    {
        parts.remove(1);
    }
    host_ok
        && parts.len() == 4
        && parts[0] == "api"
        && parts[1] == "webhooks"
        && !parts[2].is_empty()
        && parts[2].bytes().all(|b| b.is_ascii_digit())
        && !parts[3].is_empty()
        && parts[3]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Where to find a webhook URL, for a refused one.
pub const WEBHOOK_HELP: &str = "That isn't a Discord webhook URL. In Discord: Server settings → Integrations → Webhooks → New webhook → Copy webhook URL";

/// The JSON Discord gets. Mentions only ping when `mention_everyone` is on, so a title with
/// "@everyone" in it pings no one.
pub fn webhook_body(text: &str, mention_everyone: bool) -> serde_json::Value {
    let content = if mention_everyone {
        format!("@everyone {text}")
    } else {
        text.to_owned()
    };
    let content: String = content.chars().take(DISCORD_MAX_CHARS).collect();
    let parse: Vec<&str> = if mention_everyone {
        vec!["everyone"]
    } else {
        Vec::new()
    };
    serde_json::json!({
        "content": content,
        "allowed_mentions": { "parse": parse },
    })
}

/// Posts `text` to a webhook. Blocking: call it on a worker thread. Errors never contain the
/// URL (it's a secret).
pub fn post(url: &str, text: &str, mention_everyone: bool) -> Result<(), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(10)))
        .user_agent(concat!("Clutch/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let body = webhook_body(text, mention_everyone);
    let target = format!("{}?wait=true", url.trim());
    for attempt in 0..2 {
        let mut response = agent.post(&target).send_json(&body).map_err(|e| match e {
            ureq::Error::Timeout(_) => "Discord didn't answer in time".to_owned(),
            _ => "Couldn't reach Discord; check your internet connection".to_owned(),
        })?;
        let status = response.status().as_u16();
        let header_wait = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<f64>().ok());
        let reply = response
            .body_mut()
            .read_json::<serde_json::Value>()
            .unwrap_or(serde_json::Value::Null);
        match status {
            200..=299 => return Ok(()),
            429 => {
                let wait = reply
                    .get("retry_after")
                    .and_then(serde_json::Value::as_f64)
                    .or(header_wait)
                    .unwrap_or(f64::MAX);
                if attempt == 0 && wait <= 10.0 {
                    std::thread::sleep(Duration::from_secs_f64(wait.max(0.0)));
                    continue;
                }
                return Err("Discord is limiting posts; try again in a minute".into());
            }
            401 | 403 | 404 => {
                return Err(
                    "This webhook was deleted in Discord; add a new webhook URL in Settings → Stream"
                        .into(),
                );
            }
            _ => {
                return Err(match reply.get("message").and_then(|m| m.as_str()) {
                    Some(m) => format!("Discord refused the post ({m})"),
                    None => format!("Discord answered {status}"),
                });
            }
        }
    }
    Err("Discord is limiting posts; try again in a minute".into())
}

/// Decides which webhooks get the go-live announcement: once per stream, and not again within
/// [`ANNOUNCE_GAP`] of the last one to the same webhook.
#[derive(Default)]
pub struct Announcer {
    /// Last automatic announcement per webhook URL (in memory only).
    last: HashMap<String, Instant>,
    /// This stream was announced (or had nothing to announce to).
    done: bool,
}

impl Announcer {
    /// The stream ended: the next one is announced again.
    pub fn stream_ended(&mut self) {
        self.done = false;
    }

    /// The stream is on the air: the webhooks (by index) to announce it to now. `webhooks` gives
    /// each webhook's URL and whether it posts on go-live.
    pub fn due(&mut self, webhooks: &[(&str, bool)], now: Instant) -> Vec<usize> {
        if self.done {
            return Vec::new();
        }
        self.done = true;
        let mut out = Vec::new();
        for (i, (url, on)) in webhooks.iter().enumerate() {
            if !on || url.is_empty() {
                continue;
            }
            let recent = self
                .last
                .get(*url)
                .is_some_and(|t| now.duration_since(*t) < ANNOUNCE_GAP);
            if !recent {
                self.last.insert((*url).to_owned(), now);
                out.push(i);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn link(service: Service, url: &str) -> Link {
        Link {
            service,
            dest: platform_name(service).into(),
            url: url.into(),
        }
    }

    #[test]
    fn watch_links() {
        assert_eq!(
            watch_link(Service::Twitch, "https://twitch.tv/SomeOne/", None),
            Ok(Some("https://www.twitch.tv/someone".into()))
        );
        assert_eq!(
            watch_link(Service::Kick, "someone", None),
            Ok(Some("https://kick.com/someone".into()))
        );
        assert_eq!(
            watch_link(
                Service::YouTube,
                "",
                Some("https://www.youtube.com/watch?v=abcdefghijk")
            ),
            Ok(Some("https://www.youtube.com/watch?v=abcdefghijk".into()))
        );
        assert_eq!(
            watch_link(Service::YouTube, "https://youtu.be/abcdefghijk", None),
            Ok(Some("https://www.youtube.com/watch?v=abcdefghijk".into()))
        );
        assert!(watch_link(Service::Twitch, "", None).is_err());
        assert!(watch_link(Service::YouTube, "", None).is_err());
        assert_eq!(watch_link(Service::Custom, "", None), Ok(None));
    }

    #[test]
    fn fills_the_template() {
        let links = [
            link(Service::Twitch, "https://www.twitch.tv/someone"),
            link(
                Service::YouTube,
                "https://www.youtube.com/watch?v=abcdefghijk",
            ),
        ];
        let d = Details {
            title: "Ranked grind",
            category: "Apex Legends",
            links: &links,
        };
        assert_eq!(
            fill(DEFAULT_TEMPLATE, &d),
            "Live now: Ranked grind\nTwitch: https://www.twitch.tv/someone\nYouTube: https://www.youtube.com/watch?v=abcdefghijk"
        );
        assert_eq!(
            fill("{category} on {twitch}", &d),
            "Apex Legends on https://www.twitch.tv/someone"
        );
        // One link: no platform name.
        let one = &links[..1];
        let d = Details { links: one, ..d };
        assert_eq!(fill("{links}", &d), "https://www.twitch.tv/someone");
    }

    #[test]
    fn drops_lines_left_empty() {
        let links = [link(Service::Kick, "https://kick.com/someone")];
        let d = Details {
            title: "",
            category: "",
            links: &links,
        };
        assert_eq!(fill(DEFAULT_TEMPLATE, &d), "https://kick.com/someone");
        // Text without placeholders stays.
        assert_eq!(
            fill("Come hang out\n{title}\n{links}", &d),
            "Come hang out\nhttps://kick.com/someone"
        );
    }

    #[test]
    fn same_platform_twice_is_named_by_destination() {
        let links = [
            Link {
                service: Service::Twitch,
                dest: "Main".into(),
                url: "https://www.twitch.tv/a".into(),
            },
            Link {
                service: Service::Twitch,
                dest: "Alt".into(),
                url: "https://www.twitch.tv/b".into(),
            },
        ];
        assert_eq!(
            links_text(&links),
            "Twitch (Main): https://www.twitch.tv/a\nTwitch (Alt): https://www.twitch.tv/b"
        );
    }

    #[test]
    fn share_pages() {
        assert_eq!(
            share_page("x", "Live now: a&b\nhttps://kick.com/x", "", None).as_deref(),
            Some(
                "https://x.com/intent/post?text=Live%20now%3A%20a%26b%0Ahttps%3A%2F%2Fkick.com%2Fx"
            )
        );
        assert_eq!(
            share_page(
                "reddit",
                "Live now: hi",
                "My stream",
                Some("https://kick.com/x")
            )
            .as_deref(),
            Some("https://www.reddit.com/submit?url=https%3A%2F%2Fkick.com%2Fx&title=My%20stream")
        );
        assert_eq!(
            share_page("reddit", "Live now: hi\nlink", "", None).as_deref(),
            Some("https://www.reddit.com/submit?title=Live%20now%3A%20hi")
        );
        assert!(share_page("myspace", "", "", None).is_none());
        assert_eq!(encode("é"), "%C3%A9");
    }

    #[test]
    fn webhook_urls() {
        assert!(is_webhook_url(
            "https://discord.com/api/webhooks/123456/abc-DEF_9"
        ));
        assert!(is_webhook_url(" https://discordapp.com/api/webhooks/1/x "));
        assert!(is_webhook_url(
            "https://canary.discord.com/api/v10/webhooks/1/x"
        ));
        assert!(!is_webhook_url("http://discord.com/api/webhooks/1/x"));
        assert!(!is_webhook_url(
            "https://discord.com.evil.io/api/webhooks/1/x"
        ));
        assert!(!is_webhook_url("https://discord.com/api/webhooks/abc/x"));
        assert!(!is_webhook_url("https://discord.com/api/webhooks/1"));
        assert!(!is_webhook_url("https://discord.gg/invite"));
        assert!(!is_webhook_url(
            "https://discord.com/api/webhooks/1/x?token=y"
        ));
    }

    #[test]
    fn mentions_only_when_asked() {
        let body = webhook_body("@everyone come watch", false);
        assert_eq!(body["allowed_mentions"]["parse"], serde_json::json!([]));
        assert_eq!(body["content"], "@everyone come watch");
        let body = webhook_body("come watch", true);
        assert_eq!(
            body["allowed_mentions"]["parse"],
            serde_json::json!(["everyone"])
        );
        assert_eq!(body["content"], "@everyone come watch");
        let long = "x".repeat(3000);
        assert_eq!(
            webhook_body(&long, false)["content"].as_str().map(str::len),
            Some(DISCORD_MAX_CHARS)
        );
    }

    #[test]
    fn announces_once_per_stream_and_not_right_after() {
        let mut a = Announcer::default();
        let t0 = Instant::now();
        let hooks = [
            ("https://a", true),
            ("https://b", false),
            ("https://c", true),
        ];
        assert_eq!(a.due(&hooks, t0), vec![0, 2]);
        // Reconnects and later ticks of the same stream.
        assert!(a.due(&hooks, t0 + Duration::from_secs(5)).is_empty());
        // A quick restart.
        a.stream_ended();
        assert!(a.due(&hooks, t0 + Duration::from_secs(180)).is_empty());
        // Much later.
        a.stream_ended();
        assert_eq!(
            a.due(&hooks, t0 + ANNOUNCE_GAP + Duration::from_secs(1)),
            vec![0, 2]
        );
    }

    /// A one-request HTTP server answering `status` with `body`; returns its URL and the
    /// request it got.
    fn serve(
        responses: Vec<(u16, &'static str)>,
    ) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/api/webhooks/1/x", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                // Read the headers, then the body by Content-Length.
                loop {
                    let n = stream.read(&mut chunk).unwrap();
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf);
                    if let Some(end) = text.find("\r\n\r\n") {
                        let len = text[..end]
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if buf.len() >= end + 4 + len {
                            break;
                        }
                    }
                    if n == 0 {
                        break;
                    }
                }
                requests.push(String::from_utf8_lossy(&buf).into_owned());
                let reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(reply.as_bytes()).unwrap();
            }
            requests
        });
        (url, handle)
    }

    #[test]
    fn posts_to_a_webhook() {
        let (url, server) = serve(vec![(200, "{\"id\":\"1\"}")]);
        assert_eq!(post(&url, "Live now", false), Ok(()));
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("POST /api/webhooks/1/x?wait=true "));
        let body = requests[0].split_once("\r\n\r\n").map_or("", |(_, b)| b);
        let json: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(json["content"], "Live now");
        assert_eq!(json["allowed_mentions"]["parse"], serde_json::json!([]));
    }

    #[test]
    fn retries_once_when_limited() {
        let (url, server) = serve(vec![(429, "{\"retry_after\":0.1}"), (204, "")]);
        assert_eq!(post(&url, "Live now", false), Ok(()));
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[test]
    fn deleted_webhook_says_so_without_the_url() {
        let (url, server) = serve(vec![(404, "{\"message\":\"Unknown Webhook\"}")]);
        let error = post(&url, "Live now", false).unwrap_err();
        assert!(error.contains("deleted"));
        assert!(!error.contains("webhooks/1"));
        server.join().unwrap();
    }
}
