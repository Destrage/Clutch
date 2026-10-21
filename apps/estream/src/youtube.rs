//! YouTube account: Google sign-in in the user's browser (loopback redirect with PKCE) and the
//! YouTube Data API, which finds the live broadcast (for its chat) and its viewer count.
//!
//! The client is Google's "Desktop app" kind; its ID is compiled in from `.secrets/clutch.env` (see
//! build.rs). Its secret is not in Clutch: the sign-in code and renewals go to Clutch's relay,
//! which adds the secret (`services/events-relay`), so it can be changed without a new Clutch.
//! Tokens are saved encrypted (see [`crate::secret`]).

use crate::http::{get_bearer, post_json, post_json_bearer, text};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const CLIENT_ID: Option<&str> = option_env!("CLUTCH_YOUTUBE_CLIENT_ID");
/// The live broadcast, its chat and viewer count, and sending chat messages (reading alone would
/// be youtube.readonly; force-ssl covers both).
const SCOPE: &str = "https://www.googleapis.com/auth/youtube.force-ssl";
const API: &str = "https://www.googleapis.com/youtube/v3";

/// A connected YouTube channel. The tokens live in memory only; they are saved encrypted.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct YouTubeAccount {
    /// The channel's name.
    pub channel: String,
    pub channel_id: String,
    #[serde(skip)]
    pub access: String,
    #[serde(skip)]
    pub refresh: String,
    pub access_protected: String,
    pub refresh_protected: String,
}

impl YouTubeAccount {
    fn set_tokens(&mut self, access: &str, refresh: &str) {
        self.access = access.to_owned();
        self.refresh = refresh.to_owned();
        self.access_protected = crate::secret::protect(access);
        self.refresh_protected = crate::secret::protect(refresh);
    }

    /// Decrypts the saved tokens (after loading settings).
    pub fn unprotect(&mut self) {
        self.access = crate::secret::unprotect(&self.access_protected);
        self.refresh = crate::secret::unprotect(&self.refresh_protected);
    }
}

// ---- Connecting ----

/// Where a sign-in is.
#[derive(Clone, Debug, PartialEq)]
pub enum Connect {
    /// Waiting for the user to approve in the browser.
    Waiting,
    Done(YouTubeAccount),
    Failed(String),
}

/// A sign-in in progress, run in the background. Dropping it cancels it.
pub struct Connecting {
    url: String,
    state: Arc<Mutex<Connect>>,
    cancel: Arc<AtomicBool>,
}

impl Connecting {
    /// Starts listening for Google's redirect; open [`Self::url`] in the browser.
    pub fn start() -> Result<Self, String> {
        let Some(client_id) = CLIENT_ID else {
            return Err("Connecting YouTube isn't available in this build".into());
        };
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let redirect = format!("http://127.0.0.1:{port}");
        let verifier = crate::oauth::random_token();
        let challenge = crate::oauth::challenge(&verifier);
        let csrf = crate::oauth::random_token();
        let url = format!(
            "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}\
             &response_type=code&scope={}&code_challenge={challenge}\
             &code_challenge_method=S256&state={csrf}&access_type=offline&prompt=consent",
            crate::oauth::encode(client_id),
            crate::oauth::encode(&redirect),
            crate::oauth::encode(SCOPE),
        );
        let state = Arc::new(Mutex::new(Connect::Waiting));
        let cancel = Arc::new(AtomicBool::new(false));
        let (s, c) = (state.clone(), cancel.clone());
        std::thread::Builder::new()
            .name("estream-youtube-connect".into())
            .spawn(move || {
                let result = sign_in(&listener, &c, &csrf, &verifier, &redirect);
                if !c.load(Ordering::Acquire)
                    && let Ok(mut state) = s.lock()
                {
                    *state = match result {
                        Ok(account) => Connect::Done(account),
                        Err(e) => Connect::Failed(e),
                    };
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { url, state, cancel })
    }

    /// Google's sign-in page for this attempt.
    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn state(&self) -> Connect {
        self.state
            .lock()
            .map(|s| s.clone())
            .unwrap_or(Connect::Waiting)
    }
}

impl Drop for Connecting {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

/// Waits for Google to send the browser back with a code, then trades it for tokens.
fn sign_in(
    listener: &TcpListener,
    cancel: &AtomicBool,
    csrf: &str,
    verifier: &str,
    redirect: &str,
) -> Result<YouTubeAccount, String> {
    let code = crate::oauth::wait_for_code(listener, cancel, csrf, "Google")?;
    let request = serde_json::json!({
        "code": code,
        "code_verifier": verifier,
        "redirect_uri": redirect,
    });
    let (status, body) = post_json(&crate::services::events_url("/google/token"), &request)
        .ok_or("Couldn't reach Clutch's sign-in service")?;
    let access = text(&body, "access_token");
    if status != 200 || access.is_empty() {
        return Err(format!(
            "Google refused the sign-in ({})",
            text(&body, "error")
        ));
    }
    let (status, channels) = get_bearer(
        &format!("{API}/channels?part=snippet&mine=true"),
        &access,
        &[],
    )
    .ok_or("Couldn't reach YouTube")?;
    if status != 200 {
        let reason = channels
            .get("error")
            .map(|e| text(e, "message"))
            .unwrap_or_default();
        return Err(format!("YouTube refused the request ({status}): {reason}"));
    }
    let (channel_id, channel) = channel_of(&channels)
        .ok_or("This Google account has no YouTube channel. Pick the account that owns it.")?;
    let mut account = YouTubeAccount {
        channel,
        channel_id,
        ..Default::default()
    };
    account.set_tokens(&access, &text(&body, "refresh_token"));
    Ok(account)
}

fn channel_of(body: &Value) -> Option<(String, String)> {
    let channel = body.get("items")?.get(0)?;
    let title = channel.get("snippet")?.get("title")?.as_str()?.to_owned();
    Some((text(channel, "id"), title))
}

// ---- Using the account ----

/// The account shared by the UI and the background fetches, which renew its token.
pub type Session = crate::account::Session<YouTubeAccount>;

/// The channel's live broadcast.
#[derive(Clone, Debug, PartialEq)]
pub struct Live {
    pub video_id: String,
    /// Watching now (`None` when YouTube hides it).
    pub viewers: Option<u64>,
}

impl Session {
    /// GETs a YouTube Data API path, renewing the token once if it has expired.
    fn api(&self, path: &str) -> Option<Value> {
        let account = self.account()?;
        let url = format!("{API}/{path}");
        let (mut status, mut body) = get_bearer(&url, &account.access, &[])?;
        if status == 401 {
            let renewed = self.renew(&account)?;
            (status, body) = get_bearer(&url, &renewed.access, &[])?;
        }
        if status != 200 {
            tracing::debug!(status, path, "YouTube API");
            return None;
        }
        Some(body)
    }

    fn renew(&self, old: &YouTubeAccount) -> Option<YouTubeAccount> {
        let request = serde_json::json!({ "refresh_token": old.refresh });
        let (status, body) = post_json(&crate::services::events_url("/google/refresh"), &request)?;
        let renewed = if status == 200 {
            let mut account = old.clone();
            // Google keeps the refresh token unless it sends a new one.
            let refresh = Some(text(&body, "refresh_token"))
                .filter(|r| !r.is_empty())
                .unwrap_or_else(|| old.refresh.clone());
            account.set_tokens(&text(&body, "access_token"), &refresh);
            Some(account)
        } else {
            tracing::warn!(status, "YouTube sign-in could not be renewed");
            None
        };
        self.renewed(old, renewed)
    }

    /// The connected channel's live broadcast, if it is live.
    pub fn live(&self) -> Option<Live> {
        let broadcasts =
            self.api("liveBroadcasts?part=id&broadcastStatus=active&broadcastType=all")?;
        let video_id = text(broadcasts.get("items")?.get(0)?, "id");
        if video_id.is_empty() {
            return None;
        }
        let viewers = self
            .api(&format!("videos?part=liveStreamingDetails&id={video_id}"))
            .and_then(|v| concurrent_viewers(&v));
        Some(Live { video_id, viewers })
    }

    /// The access token to use now (`renew`: after it was refused).
    pub fn token(&self, renew: bool) -> Option<String> {
        let account = self.account()?;
        if renew {
            return self.renew(&account).map(|a| a.access);
        }
        Some(account.access)
    }

    /// The live chat of the connected channel's current broadcast.
    pub fn live_chat_id(&self) -> Option<String> {
        let broadcasts =
            self.api("liveBroadcasts?part=snippet&broadcastStatus=active&broadcastType=all")?;
        let id = broadcasts
            .get("items")?
            .get(0)?
            .get("snippet")?
            .get("liveChatId")?
            .as_str()?;
        (!id.is_empty()).then(|| id.to_owned())
    }

    /// Sends a message to the current broadcast's chat.
    pub fn send_chat(&self, message: &str) -> Result<(), String> {
        if self.account().is_none() {
            return Err("Connect YouTube in Settings → Stream to reply there".into());
        }
        let chat = self
            .live_chat_id()
            .ok_or("YouTube: your broadcast isn't live yet")?;
        let body = serde_json::json!({
            "snippet": {
                "liveChatId": chat,
                "type": "textMessageEvent",
                "textMessageDetails": { "messageText": message },
            }
        });
        let url = format!("{API}/liveChat/messages?part=snippet");
        let token = self.token(false).ok_or("YouTube isn't connected")?;
        let (mut status, mut reply) =
            post_json_bearer(&url, &token, &[], &body).ok_or("Couldn't reach YouTube")?;
        if status == 401
            && let Some(token) = self.token(true)
        {
            (status, reply) =
                post_json_bearer(&url, &token, &[], &body).ok_or("Couldn't reach YouTube")?;
        }
        match status {
            200 => Ok(()),
            401 | 403 if crate::http::error_message(&reply).contains("scope") => {
                Err("Connect YouTube again in Settings → Stream to reply from Clutch".into())
            }
            _ => Err(format!(
                "YouTube didn't send it ({})",
                crate::http::error_message(&reply)
            )),
        }
    }
}

fn concurrent_viewers(body: &Value) -> Option<u64> {
    // A string in the API ("1234").
    let v = body
        .get("items")?
        .get(0)?
        .get("liveStreamingDetails")?
        .get("concurrentViewers")?;
    v.as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| v.as_u64())
}

// ---- Stream details and live status ----

/// Where to stream to YouTube and with which key.
#[derive(Clone, Debug, PartialEq)]
pub struct StreamSetup {
    pub server: String,
    pub key: String,
}

/// What YouTube is doing with the stream Clutch sends (see [`Session::live_state`]).
#[derive(Clone, Debug, PartialEq)]
pub enum LiveState {
    Live,
    /// Starting the broadcast (auto-start, or a start in progress).
    Starting,
    /// Receiving video, but the broadcast waits to be started (auto-start is off).
    WaitingToStart {
        broadcast: String,
    },
    /// The stream exists but YouTube isn't getting video on it yet.
    NoVideo,
    /// No broadcast uses this stream.
    NoBroadcast,
    /// The key isn't one of the channel's streams.
    NotYourKey,
}

impl LiveState {
    /// One line for the YouTube row of the Live deck.
    pub fn line(&self) -> &'static str {
        match self {
            LiveState::Live => "Live on YouTube",
            LiveState::Starting => "YouTube is starting your broadcast…",
            LiveState::WaitingToStart { .. } => "Waiting for you to start the broadcast",
            LiveState::NoVideo => "YouTube isn't receiving video yet",
            LiveState::NoBroadcast => "No YouTube broadcast uses this stream key",
            LiveState::NotYourKey => "This stream key isn't one of your YouTube streams",
        }
    }
}

/// Broadcasts that are on or can still go on (not finished or removed), newest first as listed.
fn open_broadcasts(broadcasts: &Value) -> Vec<&Value> {
    broadcasts
        .get("items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|b| {
                    let life = b["status"]["lifeCycleStatus"].as_str().unwrap_or_default();
                    !matches!(life, "complete" | "revoked" | "")
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The stream to use: the one bound to a live (or else a ready) broadcast, else the channel's
/// default stream, else the first.
fn pick_stream<'a>(streams: &'a Value, broadcasts: &Value) -> Option<&'a Value> {
    let items = streams.get("items")?.as_array()?;
    let open = open_broadcasts(broadcasts);
    let bound = open
        .iter()
        .find(|b| b["status"]["lifeCycleStatus"] == "live")
        .or_else(|| open.first())
        .and_then(|b| b["contentDetails"]["boundStreamId"].as_str());
    bound
        .and_then(|id| items.iter().find(|s| s["id"] == id))
        .or_else(|| {
            items.iter().find(|s| {
                s["snippet"]["title"]
                    .as_str()
                    .is_some_and(|t| t.to_ascii_lowercase().contains("default"))
            })
        })
        .or_else(|| items.first())
}

fn setup_of(stream: &Value) -> Option<StreamSetup> {
    let info = &stream["cdn"]["ingestionInfo"];
    let key = info["streamName"].as_str().filter(|k| !k.is_empty())?;
    let server = info["ingestionAddress"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("rtmp://a.rtmp.youtube.com/live2");
    Some(StreamSetup {
        server: server.to_owned(),
        key: key.to_owned(),
    })
}

/// What YouTube is doing with the stream whose key is `key`.
fn state_of(streams: &Value, broadcasts: &Value, key: &str) -> LiveState {
    let stream = streams
        .get("items")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|s| s["cdn"]["ingestionInfo"]["streamName"].as_str() == Some(key))
        });
    let Some(stream) = stream else {
        return LiveState::NotYourKey;
    };
    let id = stream["id"].as_str().unwrap_or_default();
    let open = open_broadcasts(broadcasts);
    let mine: Vec<&&Value> = open
        .iter()
        .filter(|b| b["contentDetails"]["boundStreamId"].as_str() == Some(id))
        .collect();
    if mine
        .iter()
        .any(|b| b["status"]["lifeCycleStatus"] == "live")
    {
        return LiveState::Live;
    }
    if stream["status"]["streamStatus"] != "active" {
        return LiveState::NoVideo;
    }
    let Some(broadcast) = mine.first() else {
        return LiveState::NoBroadcast;
    };
    match broadcast["status"]["lifeCycleStatus"]
        .as_str()
        .unwrap_or_default()
    {
        "liveStarting" | "testStarting" => LiveState::Starting,
        _ if broadcast["contentDetails"]["enableAutoStart"] == true => LiveState::Starting,
        _ => LiveState::WaitingToStart {
            broadcast: broadcast["id"].as_str().unwrap_or_default().to_owned(),
        },
    }
}

impl Session {
    fn streams_and_broadcasts(&self) -> Option<(Value, Value)> {
        // Only broadcasts that are on or about to be: a plain "mine" list also holds every finished
        // one, which on a busy channel pushes the current broadcast out of the first page.
        let list = |status: &str| {
            self.api(&format!(
                "liveBroadcasts?part=status,contentDetails&broadcastStatus={status}&broadcastType=all&maxResults=50"
            ))
        };
        let mut items = Vec::new();
        for status in ["active", "upcoming"] {
            if let Some(Value::Array(found)) = list(status).and_then(|b| b.get("items").cloned()) {
                items.extend(found);
            }
        }
        let broadcasts = serde_json::json!({ "items": items });
        let streams = self.api("liveStreams?part=cdn,snippet,status&mine=true&maxResults=50")?;
        Some((streams, broadcasts))
    }

    /// The server and key of the channel's stream (see [`pick_stream`]).
    pub fn stream_setup(&self) -> Result<StreamSetup, String> {
        if self.account().is_none() {
            return Err("Connect YouTube in Settings → Stream first".into());
        }
        let (streams, broadcasts) = self
            .streams_and_broadcasts()
            .ok_or("Couldn't read your YouTube streams. Connect YouTube again and retry.")?;
        pick_stream(&streams, &broadcasts)
            .and_then(setup_of)
            .ok_or_else(|| {
                "Your YouTube channel has no stream yet: open YouTube Studio → Go live once, then try again"
                    .into()
            })
    }

    /// What YouTube is doing with the stream Clutch sends with `key` (`None`: couldn't tell).
    pub fn live_state(&self, key: &str) -> Option<LiveState> {
        let (streams, broadcasts) = self.streams_and_broadcasts()?;
        Some(state_of(&streams, &broadcasts, key))
    }

    /// Starts a broadcast that waits for it (auto-start off).
    pub fn start_broadcast(&self, broadcast: &str) -> Result<(), String> {
        let url = format!(
            "{API}/liveBroadcasts/transition?broadcastStatus=live&id={broadcast}&part=status"
        );
        let token = self.token(false).ok_or("YouTube isn't connected")?;
        let body = serde_json::json!({});
        let (mut status, mut reply) =
            post_json_bearer(&url, &token, &[], &body).ok_or("Couldn't reach YouTube")?;
        if status == 401
            && let Some(token) = self.token(true)
        {
            (status, reply) =
                post_json_bearer(&url, &token, &[], &body).ok_or("Couldn't reach YouTube")?;
        }
        match status {
            200 => Ok(()),
            _ => Err(format!(
                "YouTube didn't start the broadcast ({})",
                crate::http::error_message(&reply)
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stream(id: &str, title: &str, key: &str, status: &str) -> Value {
        serde_json::json!({
            "id": id,
            "snippet": { "title": title },
            "cdn": { "ingestionInfo": { "streamName": key,
                "ingestionAddress": "rtmp://a.rtmp.youtube.com/live2" } },
            "status": { "streamStatus": status }
        })
    }

    fn broadcast(id: &str, life: &str, bound: &str, auto: bool) -> Value {
        serde_json::json!({
            "id": id,
            "status": { "lifeCycleStatus": life },
            "contentDetails": { "boundStreamId": bound, "enableAutoStart": auto }
        })
    }

    #[test]
    fn the_bound_stream_is_picked() {
        let streams = serde_json::json!({ "items": [
            stream("s1", "Default stream key", "aaaa-1111", "inactive"),
            stream("s2", "Scheduled", "bbbb-2222", "inactive"),
        ]});
        let bound = serde_json::json!({ "items": [broadcast("b1", "ready", "s2", false)] });
        assert_eq!(
            pick_stream(&streams, &bound)
                .and_then(setup_of)
                .unwrap()
                .key,
            "bbbb-2222"
        );
        let none = serde_json::json!({ "items": [broadcast("b0", "complete", "s2", false)] });
        assert_eq!(
            pick_stream(&streams, &none).and_then(setup_of).unwrap().key,
            "aaaa-1111"
        );
    }

    #[test]
    fn live_states() {
        let streams = |status: &str| serde_json::json!({ "items": [stream("s1", "Default stream key", "k1", status)] });
        let list = |b: Value| serde_json::json!({ "items": [b] });
        assert_eq!(
            state_of(
                &streams("active"),
                &list(broadcast("b", "ready", "s1", true)),
                "nope"
            ),
            LiveState::NotYourKey
        );
        assert_eq!(
            state_of(
                &streams("inactive"),
                &list(broadcast("b", "ready", "s1", true)),
                "k1"
            ),
            LiveState::NoVideo
        );
        assert_eq!(
            state_of(
                &streams("active"),
                &serde_json::json!({ "items": [] }),
                "k1"
            ),
            LiveState::NoBroadcast
        );
        assert_eq!(
            state_of(
                &streams("active"),
                &list(broadcast("b", "ready", "s1", false)),
                "k1"
            ),
            LiveState::WaitingToStart {
                broadcast: "b".into()
            }
        );
        assert_eq!(
            state_of(
                &streams("active"),
                &list(broadcast("b", "ready", "s1", true)),
                "k1"
            ),
            LiveState::Starting
        );
        assert_eq!(
            state_of(
                &streams("active"),
                &list(broadcast("b", "live", "s1", false)),
                "k1"
            ),
            LiveState::Live
        );
    }

    #[test]
    fn viewers_are_read_from_the_live_details() {
        let body = serde_json::json!({
            "items": [{ "liveStreamingDetails": { "concurrentViewers": "1234" } }]
        });
        assert_eq!(concurrent_viewers(&body), Some(1234));
        let hidden = serde_json::json!({ "items": [{ "liveStreamingDetails": {} }] });
        assert_eq!(concurrent_viewers(&hidden), None);
    }

    #[test]
    fn tokens_are_not_saved_in_plain_text() {
        let mut account = YouTubeAccount {
            channel: "Me".into(),
            ..Default::default()
        };
        account.set_tokens("access-token", "refresh-token");
        let json = serde_json::to_string(&account).unwrap();
        assert!(!json.contains("access-token") && !json.contains("refresh-token"));
    }
}
