//! YouTube account: Google sign-in in the user's browser (loopback redirect with PKCE) and the
//! YouTube Data API, which finds the live broadcast (for its chat) and its viewer count.
//!
//! The client is Google's "Desktop app" kind; its ID and secret are compiled in from
//! `.secrets/clutch.env` (see build.rs). Tokens are saved encrypted (see [`crate::secret`]).

use crate::http::{get_bearer, post_form, post_json_bearer, text};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const CLIENT_ID: Option<&str> = option_env!("CLUTCH_YOUTUBE_CLIENT_ID");
const CLIENT_SECRET: Option<&str> = option_env!("CLUTCH_YOUTUBE_CLIENT_SECRET");
/// The live broadcast, its chat and viewer count, and sending chat messages (reading alone would
/// be youtube.readonly; force-ssl covers both).
const SCOPE: &str = "https://www.googleapis.com/auth/youtube.force-ssl";
const API: &str = "https://www.googleapis.com/youtube/v3";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

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
        let (Some(client_id), Some(_)) = (CLIENT_ID, CLIENT_SECRET) else {
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
    let (Some(client_id), Some(secret)) = (CLIENT_ID, CLIENT_SECRET) else {
        return Err("Connecting YouTube isn't available in this build".into());
    };
    let (status, body) = post_form(
        TOKEN_URL,
        &[
            ("code", &code),
            ("client_id", client_id),
            ("client_secret", secret),
            ("redirect_uri", redirect),
            ("grant_type", "authorization_code"),
            ("code_verifier", verifier),
        ],
    )?;
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
        let (Some(client_id), Some(secret)) = (CLIENT_ID, CLIENT_SECRET) else {
            return None;
        };
        let (status, body) = post_form(
            TOKEN_URL,
            &[
                ("client_id", client_id),
                ("client_secret", secret),
                ("refresh_token", &old.refresh),
                ("grant_type", "refresh_token"),
            ],
        )
        .ok()?;
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

#[cfg(test)]
mod tests {
    use super::*;

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
