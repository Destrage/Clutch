//! YouTube account: Google sign-in in the user's browser (loopback redirect with PKCE) and the
//! YouTube Data API, which finds the live broadcast (for its chat) and its viewer count.
//!
//! The client is Google's "Desktop app" kind; its ID is compiled in from `.secrets/clutch.env` (see
//! build.rs). Its secret is not in Clutch: the sign-in code and renewals go to Clutch's relay,
//! which adds the secret (`services/events-relay`), so it can be changed without a new Clutch.
//! Tokens are saved encrypted (see [`crate::secret`]).

use crate::http::{delete_bearer, get_bearer, post_json, post_json_bearer, send_json_bearer, text};
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
    /// No broadcast uses this key, but an open broadcast uses another of the channel's streams:
    /// its server and key.
    OtherKey(StreamSetup),
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
            LiveState::OtherKey(_) => "Your YouTube broadcast uses a different stream key",
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
    pick_stream_of(streams, broadcasts, true)
}

/// [`pick_stream`], with or without the default stream when no open broadcast uses one.
fn pick_stream_of<'a>(streams: &'a Value, broadcasts: &Value, fallback: bool) -> Option<&'a Value> {
    let items = streams.get("items")?.as_array()?;
    let open = open_broadcasts(broadcasts);
    let bound = open
        .iter()
        .find(|b| b["status"]["lifeCycleStatus"] == "live")
        .or_else(|| open.first())
        .and_then(|b| b["contentDetails"]["boundStreamId"].as_str());
    let bound = bound.and_then(|id| items.iter().find(|s| s["id"] == id));
    if !fallback {
        return bound;
    }
    bound
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
    let stream = stream_with_key(streams, key);
    // The stream an open broadcast uses (as in `pick_stream`), when its key isn't `key`.
    let other = || {
        pick_stream_of(streams, broadcasts, false)
            .and_then(setup_of)
            .filter(|setup| setup.key != key)
    };
    let Some(stream) = stream else {
        return other().map_or(LiveState::NotYourKey, LiveState::OtherKey);
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
    if mine.is_empty()
        && let Some(setup) = other()
    {
        return LiveState::OtherKey(setup);
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

    /// Makes sure YouTube has a broadcast to go live on with the stream whose key is `key`. When
    /// no broadcast is live or upcoming at all (YouTube ends each one when the stream stops, and
    /// Studio only makes the next when its Stream page is opened), creates one titled and shared
    /// like the channel's last broadcast, with auto-start and auto-stop, and binds it to the
    /// stream. Returns the new broadcast's id and title, or `None` when there was one already.
    pub fn ensure_broadcast(&self, key: &str) -> Result<Option<(String, String)>, String> {
        let (streams, broadcasts) = self
            .streams_and_broadcasts()
            .ok_or("Couldn't read your YouTube streams")?;
        let Some(stream) = stream_with_key(&streams, key) else {
            // Not one of the channel's streams: the live status says so.
            return Ok(None);
        };
        if !open_broadcasts(&broadcasts).is_empty() {
            return Ok(None);
        }
        let stream_id = stream["id"].as_str().unwrap_or_default().to_owned();
        let last = self
            .api("liveBroadcasts?part=snippet,status&mine=true&broadcastType=all&maxResults=1")
            .and_then(|b| b["items"].get(0).cloned());
        let channel = self.account().map(|a| a.channel).unwrap_or_default();
        let (title, privacy) = new_broadcast_look(last.as_ref(), &channel);
        let body = serde_json::json!({
            "snippet": {
                "title": title,
                "scheduledStartTime": chrono::Utc::now()
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            },
            "status": { "privacyStatus": privacy, "selfDeclaredMadeForKids": false },
            "contentDetails": {
                "enableAutoStart": true,
                "enableAutoStop": true,
                "monitorStream": { "enableMonitorStream": false },
            },
        });
        let created = self
            .post("liveBroadcasts?part=snippet,status,contentDetails", &body)
            .map_err(|e| format!("YouTube didn't create a broadcast ({e})"))?;
        let id = created["id"].as_str().unwrap_or_default().to_owned();
        self.post(
            &format!("liveBroadcasts/bind?id={id}&part=id,contentDetails&streamId={stream_id}"),
            &serde_json::json!({}),
        )
        .map_err(|e| format!("YouTube didn't attach the new broadcast to your stream ({e})"))?;
        Ok(Some((id, title)))
    }

    /// Starts a broadcast that waits for it (auto-start off).
    pub fn start_broadcast(&self, broadcast: &str) -> Result<(), String> {
        self.post(
            &format!("liveBroadcasts/transition?broadcastStatus=live&id={broadcast}&part=status"),
            &serde_json::json!({}),
        )
        .map(|_| ())
        .map_err(|e| format!("YouTube didn't start the broadcast ({e})"))
    }

    /// Deletes a broadcast Clutch created that never went live, so it doesn't stay on the channel
    /// as an upcoming stream that holds the stream key.
    pub fn delete_broadcast(&self, broadcast: &str) -> Result<(), String> {
        let url = format!("{API}/liveBroadcasts?id={broadcast}");
        let token = self.token(false).ok_or("YouTube isn't connected")?;
        let mut status = delete_bearer(&url, &token).ok_or("Couldn't reach YouTube")?;
        if status == 401
            && let Some(token) = self.token(true)
        {
            status = delete_bearer(&url, &token).ok_or("Couldn't reach YouTube")?;
        }
        match status {
            200 | 204 => Ok(()),
            _ => Err(format!("YouTube didn't delete the broadcast ({status})")),
        }
    }

    /// POSTs `body` to the API (renewing the sign-in once if it expired): the reply, or YouTube's
    /// error message.
    fn post(&self, path: &str, body: &Value) -> Result<Value, String> {
        self.send("POST", path, body)
    }

    /// Sends `body` with `method` to the API (renewing the sign-in once if it expired): the
    /// reply, or YouTube's error message.
    fn send(&self, method: &str, path: &str, body: &Value) -> Result<Value, String> {
        let url = format!("{API}/{path}");
        let token = self.token(false).ok_or("YouTube isn't connected")?;
        let (mut status, mut reply) =
            send_json_bearer(method, &url, &token, &[], body).ok_or("Couldn't reach YouTube")?;
        if status == 401
            && let Some(token) = self.token(true)
        {
            (status, reply) = send_json_bearer(method, &url, &token, &[], body)
                .ok_or("Couldn't reach YouTube")?;
        }
        match status {
            200 => Ok(reply),
            _ => Err(crate::http::error_message(&reply)),
        }
    }
}

// ---- Broadcast details (the Go live window) ----

/// How soon viewers see the stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Latency {
    #[default]
    Normal,
    Low,
    UltraLow,
}

impl Latency {
    fn api(self) -> &'static str {
        match self {
            Latency::Normal => "normal",
            Latency::Low => "low",
            Latency::UltraLow => "ultraLow",
        }
    }

    fn from_api(s: &str) -> Self {
        match s {
            "low" => Latency::Low,
            "ultraLow" => Latency::UltraLow,
            _ => Latency::Normal,
        }
    }
}

/// What a broadcast shows and how it runs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BroadcastDetails {
    pub title: String,
    pub description: String,
    /// "public", "unlisted" or "private".
    pub privacy: String,
    /// A YouTube video category id ("20" is Gaming); "" leaves YouTube's default.
    pub category: String,
    pub made_for_kids: bool,
    pub latency: Latency,
    pub auto_start: bool,
    pub auto_stop: bool,
}

impl Default for BroadcastDetails {
    fn default() -> Self {
        Self {
            title: String::new(),
            description: String::new(),
            privacy: "public".into(),
            category: String::new(),
            made_for_kids: false,
            latency: Latency::Normal,
            auto_start: true,
            auto_stop: true,
        }
    }
}

/// A broadcast that is live or can still go live.
#[derive(Clone, Debug, PartialEq)]
pub struct Broadcast {
    pub id: String,
    pub details: BroadcastDetails,
    /// When it's scheduled to start (RFC 3339; "" when not set).
    pub scheduled: String,
    /// YouTube's lifeCycleStatus: "created", "ready", "testing", "live", …
    pub status: String,
    /// The stream it's bound to: its id and its server and key.
    pub stream: Option<(String, StreamSetup)>,
}

impl Session {
    /// The channel's live and upcoming broadcasts, with their details and bound streams.
    pub fn broadcasts(&self) -> Result<Vec<Broadcast>, String> {
        const READ: &str = "Couldn't read your YouTube broadcasts";
        let mut items = Vec::new();
        for status in ["active", "upcoming"] {
            let list = self
                .api(&format!(
                    "liveBroadcasts?part=snippet,status,contentDetails&broadcastStatus={status}\
                     &broadcastType=all&maxResults=50"
                ))
                .ok_or(READ)?;
            if let Some(Value::Array(found)) = list.get("items") {
                items.extend(found.iter().cloned());
            }
        }
        let broadcasts = serde_json::json!({ "items": items });
        let streams = self
            .api("liveStreams?part=cdn,snippet,status&mine=true&maxResults=50")
            .ok_or(READ)?;
        let videos = self.videos(&items);
        Ok(broadcasts_of(&broadcasts, &videos, &streams))
    }

    /// The details of the channel's last broadcast (for a new one), or YouTube's own defaults.
    pub fn last_details(&self) -> BroadcastDetails {
        let last = self
            .api("liveBroadcasts?part=snippet,status,contentDetails&mine=true&broadcastType=all&maxResults=1")
            .and_then(|b| b["items"].get(0).cloned());
        let channel = self.account().map(|a| a.channel).unwrap_or_default();
        match last {
            Some(b) => {
                let videos = self.videos(std::slice::from_ref(&b));
                details_of(&b, video_in(&videos, text(&b, "id").as_str()))
            }
            None => BroadcastDetails {
                title: format!("{channel} Live Stream").trim().to_owned(),
                ..Default::default()
            },
        }
    }

    /// The video categories a broadcast can use in the channel's country: (id, name).
    pub fn categories(&self) -> Result<Vec<(String, String)>, String> {
        let country = self
            .api("channels?part=snippet&mine=true")
            .and_then(|c| {
                c["items"][0]["snippet"]["country"]
                    .as_str()
                    .map(str::to_owned)
            })
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| "US".into());
        let list = self
            .api(&format!(
                "videoCategories?part=snippet&regionCode={country}&hl=en"
            ))
            .ok_or("Couldn't read YouTube's categories")?;
        Ok(categories_of(&list))
    }

    /// Creates a broadcast with `details` and binds it to the stream `stream_id`: its id.
    pub fn create_broadcast(
        &self,
        details: &BroadcastDetails,
        stream_id: &str,
    ) -> Result<String, String> {
        let body = serde_json::json!({
            "snippet": {
                "title": details.title,
                "description": details.description,
                "scheduledStartTime": chrono::Utc::now()
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            },
            "status": {
                "privacyStatus": details.privacy,
                "selfDeclaredMadeForKids": details.made_for_kids,
            },
            "contentDetails": {
                "enableAutoStart": details.auto_start,
                "enableAutoStop": details.auto_stop,
                "latencyPreference": details.latency.api(),
                "monitorStream": { "enableMonitorStream": false },
            },
        });
        let created = self
            .post("liveBroadcasts?part=snippet,status,contentDetails", &body)
            .map_err(|e| format!("YouTube didn't create the broadcast ({e})"))?;
        let id = text(&created, "id");
        self.bind(&id, stream_id)?;
        self.set_video(&id, details)?;
        Ok(id)
    }

    /// Saves `details` to an existing broadcast. Latency and auto-start/stop only change before
    /// it goes live (YouTube refuses after).
    pub fn update_broadcast(&self, id: &str, details: &BroadcastDetails) -> Result<(), String> {
        let current = self
            .api(&format!(
                "liveBroadcasts?part=snippet,status,contentDetails&id={id}"
            ))
            .and_then(|b| b["items"].get(0).cloned())
            .ok_or("Couldn't read the YouTube broadcast")?;
        let (body, parts) = updated_broadcast(current, details);
        self.send("PUT", &format!("liveBroadcasts?part={parts}"), &body)
            .map_err(|e| format!("YouTube didn't save the broadcast ({e})"))?;
        self.set_video(id, details)
    }

    /// Binds broadcast `id` to the stream `stream_id`.
    pub fn bind(&self, id: &str, stream_id: &str) -> Result<(), String> {
        self.post(
            &format!("liveBroadcasts/bind?id={id}&part=id,contentDetails&streamId={stream_id}"),
            &serde_json::json!({}),
        )
        .map(|_| ())
        .map_err(|e| format!("YouTube didn't attach the broadcast to your stream ({e})"))
    }

    /// Deletes broadcast `id` if YouTube says it never went live: whether it did.
    pub fn delete_if_never_live(&self, id: &str) -> Result<bool, String> {
        let current = self
            .api(&format!("liveBroadcasts?part=snippet,status&id={id}"))
            .ok_or("Couldn't read the YouTube broadcast")?;
        match current["items"].get(0) {
            Some(b) if never_went_live(b) => self.delete_broadcast(id).map(|()| true),
            _ => Ok(false),
        }
    }

    /// The videos behind `broadcasts` (category and made-for-kids live there).
    fn videos(&self, broadcasts: &[Value]) -> Value {
        let ids: Vec<String> = broadcasts.iter().map(|b| text(b, "id")).collect();
        if ids.is_empty() {
            return Value::Null;
        }
        self.api(&format!(
            "videos?part=snippet,status&id={}&maxResults=50",
            ids.join(",")
        ))
        .unwrap_or(Value::Null)
    }

    /// Saves the category, title, description and made-for-kids on the broadcast's video,
    /// keeping its other fields (tags, language).
    fn set_video(&self, id: &str, details: &BroadcastDetails) -> Result<(), String> {
        let current = self
            .api(&format!("videos?part=snippet,status&id={id}"))
            .and_then(|v| v["items"].get(0).cloned())
            .ok_or("Couldn't read the broadcast's video")?;
        let body = updated_video(&current, details);
        self.send("PUT", "videos?part=snippet,status", &body)
            .map(|_| ())
            .map_err(|e| format!("YouTube didn't save the category ({e})"))
    }
}

fn video_in<'a>(videos: &'a Value, id: &str) -> Option<&'a Value> {
    videos["items"]
        .as_array()?
        .iter()
        .find(|v| v["id"].as_str() == Some(id))
}

/// A broadcast's details from the API (`video` adds the category).
fn details_of(b: &Value, video: Option<&Value>) -> BroadcastDetails {
    let content = &b["contentDetails"];
    let kids = &b["status"]["selfDeclaredMadeForKids"];
    BroadcastDetails {
        title: text(&b["snippet"], "title"),
        description: text(&b["snippet"], "description"),
        privacy: Some(text(&b["status"], "privacyStatus"))
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| "public".into()),
        category: video
            .map(|v| text(&v["snippet"], "categoryId"))
            .unwrap_or_default(),
        made_for_kids: kids
            .as_bool()
            .or_else(|| video.and_then(|v| v["status"]["selfDeclaredMadeForKids"].as_bool()))
            .unwrap_or(false),
        latency: Latency::from_api(content["latencyPreference"].as_str().unwrap_or_default()),
        auto_start: content["enableAutoStart"].as_bool().unwrap_or(false),
        auto_stop: content["enableAutoStop"].as_bool().unwrap_or(false),
    }
}

/// The open broadcasts in a listing, with their details and bound streams.
fn broadcasts_of(broadcasts: &Value, videos: &Value, streams: &Value) -> Vec<Broadcast> {
    open_broadcasts(broadcasts)
        .into_iter()
        .map(|b| {
            let id = text(b, "id");
            let stream = b["contentDetails"]["boundStreamId"]
                .as_str()
                .and_then(|sid| {
                    streams["items"]
                        .as_array()?
                        .iter()
                        .find(|s| s["id"].as_str() == Some(sid))
                })
                .and_then(|s| Some((text(s, "id"), setup_of(s)?)));
            Broadcast {
                details: details_of(b, video_in(videos, &id)),
                scheduled: text(&b["snippet"], "scheduledStartTime"),
                status: text(&b["status"], "lifeCycleStatus"),
                stream,
                id,
            }
        })
        .collect()
}

fn categories_of(list: &Value) -> Vec<(String, String)> {
    list["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter(|c| c["snippet"]["assignable"].as_bool().unwrap_or(false))
                .map(|c| (text(c, "id"), text(&c["snippet"], "title")))
                .collect()
        })
        .unwrap_or_default()
}

/// `current` with `details` applied, and the parts to send. Latency and auto-start/stop are only
/// sent while YouTube still accepts them (before the broadcast is live).
fn updated_broadcast(mut current: Value, details: &BroadcastDetails) -> (Value, &'static str) {
    current["snippet"]["title"] = details.title.clone().into();
    current["snippet"]["description"] = details.description.clone().into();
    current["status"]["privacyStatus"] = details.privacy.clone().into();
    let before_live = matches!(
        current["status"]["lifeCycleStatus"].as_str(),
        Some("created" | "ready")
    );
    if !before_live {
        return (current, "id,snippet,status");
    }
    let content = &mut current["contentDetails"];
    content["enableAutoStart"] = details.auto_start.into();
    content["enableAutoStop"] = details.auto_stop.into();
    content["latencyPreference"] = details.latency.api().into();
    (current, "id,snippet,status,contentDetails")
}

/// The video update for `details`: title, description and category on the snippet (keeping its
/// tags and language), and made-for-kids with the current privacy on the status.
fn updated_video(current: &Value, details: &BroadcastDetails) -> Value {
    let snippet = &current["snippet"];
    let mut body = serde_json::json!({
        "id": current["id"],
        "snippet": {
            "title": details.title,
            "description": details.description,
            "categoryId": if details.category.is_empty() {
                snippet["categoryId"].clone()
            } else {
                details.category.clone().into()
            },
        },
        "status": {
            "privacyStatus": details.privacy,
            "selfDeclaredMadeForKids": details.made_for_kids,
        },
    });
    for keep in ["tags", "defaultLanguage", "defaultAudioLanguage"] {
        if !snippet[keep].is_null() {
            body["snippet"][keep] = snippet[keep].clone();
        }
    }
    body
}

/// Whether a broadcast never went live (so deleting it loses no recording).
fn never_went_live(b: &Value) -> bool {
    let waiting = matches!(
        b["status"]["lifeCycleStatus"].as_str(),
        Some("created" | "ready" | "testing" | "testStarting")
    );
    waiting && b["snippet"]["actualStartTime"].is_null()
}

/// The channel's stream whose key is `key`.
fn stream_with_key<'a>(streams: &'a Value, key: &str) -> Option<&'a Value> {
    streams["items"]
        .as_array()?
        .iter()
        .find(|s| s["cdn"]["ingestionInfo"]["streamName"].as_str() == Some(key))
}

/// Title and privacy for a broadcast Clutch creates: the last broadcast's, else
/// "<channel> Live Stream" (YouTube's own default) and public.
fn new_broadcast_look(last: Option<&Value>, channel: &str) -> (String, String) {
    let field = |v: &Value| {
        v.as_str()
            .filter(|s| !s.trim().is_empty())
            .map(str::to_owned)
    };
    let title = last
        .and_then(|b| field(&b["snippet"]["title"]))
        .unwrap_or_else(|| format!("{channel} Live Stream").trim().to_owned());
    let privacy = last
        .and_then(|b| field(&b["status"]["privacyStatus"]))
        .unwrap_or_else(|| "public".into());
    (title, privacy)
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
                &list(broadcast("b", "complete", "s1", true)),
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
    fn a_broadcast_on_another_stream_gives_its_key() {
        let streams = serde_json::json!({ "items": [
            stream("s1", "Default stream key", "k1", "active"),
            stream("s2", "Variable", "k2", "inactive"),
        ]});
        let on_s2 = serde_json::json!({ "items": [broadcast("b", "ready", "s2", true)] });
        let k2 = LiveState::OtherKey(StreamSetup {
            server: "rtmp://a.rtmp.youtube.com/live2".into(),
            key: "k2".into(),
        });
        assert_eq!(state_of(&streams, &on_s2, "k1"), k2);
        assert_eq!(state_of(&streams, &on_s2, "old-key"), k2);
        // Already the right key: nothing to switch.
        assert_eq!(state_of(&streams, &on_s2, "k2"), LiveState::NoVideo);
    }

    #[test]
    fn a_new_broadcast_looks_like_the_last() {
        let last = serde_json::json!({
            "snippet": { "title": "Ranked tonight" },
            "status": { "privacyStatus": "unlisted" }
        });
        assert_eq!(
            new_broadcast_look(Some(&last), "Me"),
            ("Ranked tonight".into(), "unlisted".into())
        );
        assert_eq!(
            new_broadcast_look(None, "Me"),
            ("Me Live Stream".into(), "public".into())
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

    #[test]
    fn broadcasts_are_listed_with_details_and_stream() {
        let broadcasts = serde_json::json!({ "items": [
            {
                "id": "b1",
                "snippet": { "title": "Ranked", "description": "d",
                    "scheduledStartTime": "2026-10-21T14:35:00Z" },
                "status": { "lifeCycleStatus": "ready", "privacyStatus": "unlisted",
                    "selfDeclaredMadeForKids": false },
                "contentDetails": { "boundStreamId": "s1", "enableAutoStart": true,
                    "enableAutoStop": false, "latencyPreference": "ultraLow" }
            },
            broadcast("b2", "complete", "s1", true)
        ]});
        let videos = serde_json::json!({ "items": [
            { "id": "b1", "snippet": { "categoryId": "20" } }
        ]});
        let streams = serde_json::json!({ "items": [stream("s1", "Default", "k1", "active")] });
        let list = broadcasts_of(&broadcasts, &videos, &streams);
        assert_eq!(list.len(), 1, "finished broadcasts are left out");
        let b = &list[0];
        assert_eq!(b.id, "b1");
        assert_eq!(b.status, "ready");
        assert_eq!(b.scheduled, "2026-10-21T14:35:00Z");
        assert_eq!(
            b.details,
            BroadcastDetails {
                title: "Ranked".into(),
                description: "d".into(),
                privacy: "unlisted".into(),
                category: "20".into(),
                made_for_kids: false,
                latency: Latency::UltraLow,
                auto_start: true,
                auto_stop: false,
            }
        );
        let (id, setup) = b.stream.clone().unwrap();
        assert_eq!((id.as_str(), setup.key.as_str()), ("s1", "k1"));
    }

    #[test]
    fn only_assignable_categories() {
        let list = serde_json::json!({ "items": [
            { "id": "20", "snippet": { "title": "Gaming", "assignable": true } },
            { "id": "18", "snippet": { "title": "Short Movies", "assignable": false } }
        ]});
        assert_eq!(categories_of(&list), vec![("20".into(), "Gaming".into())]);
    }

    #[test]
    fn a_live_broadcast_keeps_its_latency_and_auto_settings() {
        let details = BroadcastDetails {
            title: "New".into(),
            latency: Latency::Low,
            ..Default::default()
        };
        let current = |life: &str| {
            serde_json::json!({
                "id": "b1",
                "snippet": { "title": "Old", "scheduledStartTime": "t" },
                "status": { "lifeCycleStatus": life, "privacyStatus": "public" },
                "contentDetails": { "latencyPreference": "normal",
                    "monitorStream": { "enableMonitorStream": false } }
            })
        };
        let (body, parts) = updated_broadcast(current("live"), &details);
        assert_eq!(parts, "id,snippet,status");
        assert_eq!(body["snippet"]["title"], "New");
        assert_eq!(body["snippet"]["scheduledStartTime"], "t");
        let (body, parts) = updated_broadcast(current("ready"), &details);
        assert_eq!(parts, "id,snippet,status,contentDetails");
        assert_eq!(body["contentDetails"]["latencyPreference"], "low");
        assert_eq!(
            body["contentDetails"]["monitorStream"]["enableMonitorStream"],
            false
        );
    }

    #[test]
    fn the_video_update_keeps_tags_and_category() {
        let current = serde_json::json!({
            "id": "b1",
            "snippet": { "title": "Old", "categoryId": "24", "tags": ["a"] }
        });
        let details = BroadcastDetails {
            title: "New".into(),
            made_for_kids: true,
            ..Default::default()
        };
        let body = updated_video(&current, &details);
        assert_eq!(body["snippet"]["categoryId"], "24");
        assert_eq!(body["snippet"]["tags"][0], "a");
        assert_eq!(body["status"]["selfDeclaredMadeForKids"], true);
        let gaming = BroadcastDetails {
            category: "20".into(),
            ..details
        };
        assert_eq!(
            updated_video(&current, &gaming)["snippet"]["categoryId"],
            "20"
        );
    }

    #[test]
    fn only_broadcasts_that_never_went_live_are_deleted() {
        let b = |life: &str, started: Option<&str>| {
            serde_json::json!({
                "status": { "lifeCycleStatus": life },
                "snippet": { "actualStartTime": started }
            })
        };
        assert!(never_went_live(&b("ready", None)));
        assert!(never_went_live(&b("created", None)));
        assert!(!never_went_live(&b(
            "complete",
            Some("2026-10-21T14:35:00Z")
        )));
        assert!(!never_went_live(&b("live", Some("2026-10-21T14:35:00Z"))));
        assert!(!never_went_live(&b("ready", Some("2026-10-21T14:35:00Z"))));
    }
}
