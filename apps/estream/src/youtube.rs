//! YouTube account: Google sign-in in the user's browser (loopback redirect with PKCE) and the
//! YouTube Data API, which finds the live broadcast (for its chat) and its viewer count.
//!
//! The client is Google's "Desktop app" kind; its ID and secret are compiled in from
//! `.secrets/clutch.env` (see build.rs). Tokens are saved encrypted (see [`crate::secret`]).

use crate::http::{get_bearer, post_form, text};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CLIENT_ID: Option<&str> = option_env!("CLUTCH_YOUTUBE_CLIENT_ID");
const CLIENT_SECRET: Option<&str> = option_env!("CLUTCH_YOUTUBE_CLIENT_SECRET");
/// Read-only: the live broadcast, its chat and viewer count.
const SCOPE: &str = "https://www.googleapis.com/auth/youtube.readonly";
const API: &str = "https://www.googleapis.com/youtube/v3";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// How long to wait for the user to finish in the browser.
const SIGN_IN_TIME: Duration = Duration::from_secs(10 * 60);

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
        let verifier = random_token();
        let challenge =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(&verifier));
        let csrf = random_token();
        let url = format!(
            "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}\
             &response_type=code&scope={}&code_challenge={challenge}\
             &code_challenge_method=S256&state={csrf}&access_type=offline&prompt=consent",
            encode(client_id),
            encode(&redirect),
            encode(SCOPE),
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

/// 43 random URL-safe characters (a PKCE verifier, or the anti-forgery state).
fn random_token() -> String {
    let mut bytes = [0u8; 32];
    if getrandom::fill(&mut bytes).is_err() {
        bytes = std::array::from_fn(|_| fastrand::u8(..));
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn encode(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn decode(s: &str) -> String {
    percent_encoding::percent_decode_str(&s.replace('+', " "))
        .decode_utf8_lossy()
        .into_owned()
}

/// Waits for Google to send the browser back with a code, then trades it for tokens.
fn sign_in(
    listener: &TcpListener,
    cancel: &AtomicBool,
    csrf: &str,
    verifier: &str,
    redirect: &str,
) -> Result<YouTubeAccount, String> {
    let until = Instant::now() + SIGN_IN_TIME;
    let code = loop {
        if cancel.load(Ordering::Acquire) {
            return Err("Cancelled".into());
        }
        if Instant::now() >= until {
            return Err("The YouTube sign-in timed out. Try again.".into());
        }
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        // Other requests (the browser's icon) are answered and ignored.
        if let Some(result) = redirect_request(stream, csrf) {
            break result?;
        }
    };
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
    let (_, channels) = get_bearer(&format!("{API}/channels?part=snippet&mine=true"), &access, &[])
        .ok_or("Couldn't reach YouTube")?;
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

/// Reads one request to the loopback address: `Some(code or error)` for Google's redirect,
/// `None` for anything else.
fn redirect_request(mut stream: TcpStream, csrf: &str) -> Option<Result<String, String>> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buffer = [0u8; 8192];
    let mut read = 0;
    while read < buffer.len() {
        match stream.read(&mut buffer[read..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => read += n,
        }
        if buffer[..read].windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let request = String::from_utf8_lossy(&buffer[..read]);
    let target = request
        .lines()
        .next()
        .and_then(|l| l.strip_prefix("GET "))
        .and_then(|l| l.split(' ').next())
        .unwrap_or_default();
    let result = redirect_result(target, csrf);
    let page = match &result {
        None => {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            return None;
        }
        Some(Ok(_)) => "Clutch is connecting to YouTube. You can close this tab and go back to Clutch.",
        Some(Err(_)) => "YouTube wasn't connected. You can close this tab; Clutch shows what happened.",
    };
    let html = format!(
        "<!doctype html><meta charset=utf-8><title>Clutch</title>\
         <body style=\"font:16px system-ui;background:#0f1115;color:#e8eaf0;display:grid;\
         place-items:center;height:90vh\"><p>{page}</p>"
    );
    let _ = stream.write_all(
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
            html.len()
        )
        .as_bytes(),
    );
    result
}

/// The code (or the error) in Google's redirect to `target` (`/?state=…&code=…`).
fn redirect_result(target: &str, csrf: &str) -> Option<Result<String, String>> {
    let query = target.strip_prefix("/?")?;
    let param = |key: &str| {
        query
            .split('&')
            .filter_map(|p| p.split_once('='))
            .find(|(k, _)| *k == key)
            .map(|(_, v)| decode(v))
    };
    if let Some(error) = param("error") {
        return Some(Err(if error == "access_denied" {
            "The sign-in was declined on Google".into()
        } else {
            format!("Google sign-in failed ({error})")
        }));
    }
    let code = param("code")?;
    if param("state").as_deref() != Some(csrf) {
        return Some(Err("The sign-in didn't match this attempt. Try again.".into()));
    }
    Some(Ok(code))
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
        let broadcasts = self.api("liveBroadcasts?part=id&broadcastStatus=active&broadcastType=all")?;
        let video_id = text(broadcasts.get("items")?.get(0)?, "id");
        if video_id.is_empty() {
            return None;
        }
        let viewers = self
            .api(&format!("videos?part=liveStreamingDetails&id={video_id}"))
            .and_then(|v| concurrent_viewers(&v));
        Some(Live { video_id, viewers })
    }
}

fn concurrent_viewers(body: &Value) -> Option<u64> {
    // A string in the API ("1234").
    let v = body
        .get("items")?
        .get(0)?
        .get("liveStreamingDetails")?
        .get("concurrentViewers")?;
    v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_u64())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_redirect_carries_the_code() {
        assert_eq!(
            redirect_result("/?state=abc&code=4%2F0Ab_x&scope=x", "abc"),
            Some(Ok("4/0Ab_x".into()))
        );
        assert!(matches!(
            redirect_result("/?state=other&code=x", "abc"),
            Some(Err(_))
        ));
        assert!(matches!(
            redirect_result("/?error=access_denied&state=abc", "abc"),
            Some(Err(_))
        ));
        assert_eq!(redirect_result("/favicon.ico", "abc"), None);
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
