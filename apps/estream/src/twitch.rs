//! Twitch account: device code sign-in and the Helix API (viewer count, who's in chat).
//!
//! Clutch is a public client: the Client ID ships in the app and there is no client secret.
//! Tokens are saved encrypted (see [`crate::secret`]) and renewed in the background.

use crate::http::{agent, get_bearer, post_form, post_json_bearer, text};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const CLIENT_ID: &str = "5t3ur0gkcowdbgst7zo0415yo1quzo";
/// The chatters list and replying from Clutch's chat (viewer counts need no scope).
const SCOPES: &str = "moderator:read:chatters user:write:chat";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// A connected Twitch account. The tokens live in memory only; they are saved encrypted.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TwitchAccount {
    pub login: String,
    pub user_id: String,
    #[serde(skip)]
    pub access: String,
    #[serde(skip)]
    pub refresh: String,
    pub access_protected: String,
    pub refresh_protected: String,
}

impl TwitchAccount {
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

/// Who a token belongs to: `(login, user_id)`.
fn validate(access: &str) -> Result<(String, String), String> {
    let mut response = agent()
        .get("https://id.twitch.tv/oauth2/validate")
        .header("Authorization", &format!("OAuth {access}"))
        .call()
        .map_err(|e| e.to_string())?;
    let body: Value = response.body_mut().read_json().map_err(|e| e.to_string())?;
    let login = text(&body, "login");
    if login.is_empty() {
        return Err("Twitch didn't accept the sign-in".into());
    }
    Ok((login, text(&body, "user_id")))
}

// ---- Connecting ----

/// Where a sign-in is.
#[derive(Clone, Debug, PartialEq)]
pub enum Connect {
    Starting,
    /// The user enters `code` at `uri` (which already carries it).
    Code {
        code: String,
        uri: String,
    },
    Done(TwitchAccount),
    Failed(String),
}

/// A sign-in in progress (device code flow), run in the background. Dropping it cancels it.
pub struct Connecting {
    state: Arc<Mutex<Connect>>,
    cancel: Arc<AtomicBool>,
}

impl Connecting {
    pub fn start() -> Self {
        let state = Arc::new(Mutex::new(Connect::Starting));
        let cancel = Arc::new(AtomicBool::new(false));
        let (s, c) = (state.clone(), cancel.clone());
        let _ = std::thread::Builder::new()
            .name("estream-twitch-connect".into())
            .spawn(move || {
                let result = sign_in(&s, &c);
                if !c.load(Ordering::Acquire)
                    && let Ok(mut state) = s.lock()
                {
                    *state = match result {
                        Ok(account) => Connect::Done(account),
                        Err(e) => Connect::Failed(e),
                    };
                }
            });
        Self { state, cancel }
    }

    pub fn state(&self) -> Connect {
        self.state
            .lock()
            .map(|s| s.clone())
            .unwrap_or(Connect::Starting)
    }
}

impl Drop for Connecting {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

fn sign_in(state: &Mutex<Connect>, cancel: &AtomicBool) -> Result<TwitchAccount, String> {
    let (status, body) = post_form(
        "https://id.twitch.tv/oauth2/device",
        &[("client_id", CLIENT_ID), ("scopes", SCOPES)],
    )?;
    let device_code = text(&body, "device_code");
    if status != 200 || device_code.is_empty() {
        return Err(format!("Twitch refused the sign-in ({})", message(&body)));
    }
    let expires = Instant::now()
        + Duration::from_secs(
            body.get("expires_in")
                .and_then(Value::as_u64)
                .unwrap_or(1800),
        );
    let mut interval = body.get("interval").and_then(Value::as_u64).unwrap_or(5);
    if let Ok(mut s) = state.lock() {
        *s = Connect::Code {
            code: text(&body, "user_code"),
            uri: text(&body, "verification_uri"),
        };
    }
    loop {
        // Wait in small steps so cancelling is quick.
        let until = Instant::now() + Duration::from_secs(interval);
        while Instant::now() < until {
            if cancel.load(Ordering::Acquire) {
                return Err("Cancelled".into());
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        if Instant::now() >= expires {
            return Err("The code expired before it was entered. Try again.".into());
        }
        let (status, body) = post_form(
            "https://id.twitch.tv/oauth2/token",
            &[
                ("client_id", CLIENT_ID),
                ("scopes", SCOPES),
                ("device_code", &device_code),
                ("grant_type", DEVICE_GRANT),
            ],
        )?;
        if status == 200 {
            let access = text(&body, "access_token");
            let (login, user_id) = validate(&access)?;
            let mut account = TwitchAccount {
                login,
                user_id,
                ..Default::default()
            };
            account.set_tokens(&access, &text(&body, "refresh_token"));
            return Ok(account);
        }
        match message(&body).as_str() {
            "authorization_pending" => {}
            "slow_down" => interval += 5,
            "access_denied" => return Err("The sign-in was declined on Twitch".into()),
            other => return Err(format!("Twitch sign-in failed ({other})")),
        }
    }
}

fn message(body: &Value) -> String {
    let m = text(body, "message");
    if m.is_empty() { text(body, "error") } else { m }
}

// ---- Using the account ----

/// The account shared by the UI and the background fetches, which renew its token.
pub type Session = crate::account::Session<TwitchAccount>;

impl Session {
    /// GETs a Helix API path, renewing the token once if it has expired.
    fn helix(&self, path: &str) -> Option<Value> {
        let account = self.account()?;
        let url = format!("https://api.twitch.tv/helix/{path}");
        let headers = [("Client-Id", CLIENT_ID)];
        let (mut status, mut body) = get_bearer(&url, &account.access, &headers)?;
        if status == 401 {
            let renewed = self.renew(&account)?;
            (status, body) = get_bearer(&url, &renewed.access, &headers)?;
        }
        if status != 200 {
            tracing::debug!(status, path, "Twitch API");
            return None;
        }
        Some(body)
    }

    fn renew(&self, old: &TwitchAccount) -> Option<TwitchAccount> {
        let (status, body) = post_form(
            "https://id.twitch.tv/oauth2/token",
            &[
                ("client_id", CLIENT_ID),
                ("grant_type", "refresh_token"),
                ("refresh_token", &old.refresh),
            ],
        )
        .ok()?;
        let renewed = if status == 200 {
            let mut account = old.clone();
            account.set_tokens(&text(&body, "access_token"), &text(&body, "refresh_token"));
            Some(account)
        } else {
            tracing::warn!(status, "Twitch sign-in could not be renewed");
            None
        };
        self.renewed(old, renewed)
    }

    /// The live viewer count for a channel (`None` when offline, unknown or not connected).
    pub fn viewers(&self, channel: &str) -> Option<u64> {
        let login = self.channel_login(channel)?;
        let body = self.helix(&format!("streams?user_login={login}"))?;
        stream_viewers(&body)
    }

    /// Who is in the connected account's own chat: `(total, names)`. Other channels' lists
    /// need moderator rights, so they report nothing.
    pub fn chatters(&self, channel: &str) -> Option<(u64, Vec<String>)> {
        let account = self.account()?;
        let login = self.channel_login(channel)?;
        if !login.eq_ignore_ascii_case(&account.login) {
            return None;
        }
        let id = &account.user_id;
        let body = self.helix(&format!(
            "chat/chatters?broadcaster_id={id}&moderator_id={id}&first=1000"
        ))?;
        chatter_list(&body)
    }

    /// Sends a message to a channel's chat as the connected account.
    pub fn send_chat(&self, channel: &str, message: &str) -> Result<(), String> {
        let account = self
            .account()
            .ok_or("Connect Twitch in Settings → Stream to reply there")?;
        let login = self.channel_login(channel).ok_or("Twitch: no channel set")?;
        let broadcaster = self
            .helix(&format!("users?login={login}"))
            .and_then(|b| Some(text(b.get("data")?.get(0)?, "id")))
            .filter(|id| !id.is_empty())
            .ok_or("Couldn't find the Twitch channel")?;
        let body = serde_json::json!({
            "broadcaster_id": broadcaster,
            "sender_id": account.user_id,
            "message": message,
        });
        let url = "https://api.twitch.tv/helix/chat/messages";
        let headers = [("Client-Id", CLIENT_ID)];
        let (mut status, mut reply) = post_json_bearer(url, &account.access, &headers, &body)
            .ok_or("Couldn't reach Twitch")?;
        if status == 401
            && let Some(renewed) = self.renew(&account)
        {
            (status, reply) = post_json_bearer(url, &renewed.access, &headers, &body)
                .ok_or("Couldn't reach Twitch")?;
        }
        match status {
            200 => {
                // Twitch can accept the call and still hold the message back (AutoMod, rules).
                let sent = reply
                    .get("data")
                    .and_then(|d| d.get(0))
                    .and_then(|d| d.get("is_sent"))
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true);
                if sent {
                    Ok(())
                } else {
                    Err("Twitch held the message back (the channel's chat rules)".into())
                }
            }
            401 | 403 => {
                Err("Connect Twitch again in Settings → Stream to reply from Clutch".into())
            }
            _ => Err(format!(
                "Twitch didn't send it ({})",
                crate::http::error_message(&reply)
            )),
        }
    }

    /// The destination's channel, or the connected account's when none is set.
    fn channel_login(&self, channel: &str) -> Option<String> {
        crate::chat::twitch_login(channel)
            .or_else(|| self.account().map(|a| a.login))
            .filter(|l| !l.is_empty())
            .map(|l| l.to_ascii_lowercase())
    }
}

fn stream_viewers(body: &Value) -> Option<u64> {
    body.get("data")?.get(0)?.get("viewer_count")?.as_u64()
}

fn chatter_list(body: &Value) -> Option<(u64, Vec<String>)> {
    let names: Vec<String> = body
        .get("data")?
        .as_array()?
        .iter()
        .filter_map(|c| {
            c.get("user_name")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    let total = body
        .get("total")
        .and_then(Value::as_u64)
        .unwrap_or(names.len() as u64);
    Some((total, names))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewers_come_from_the_live_stream() {
        let live = serde_json::json!({ "data": [{ "user_login": "x", "viewer_count": 41 }] });
        assert_eq!(stream_viewers(&live), Some(41));
        let offline = serde_json::json!({ "data": [] });
        assert_eq!(stream_viewers(&offline), None);
    }

    #[test]
    fn chatters_have_names_and_a_total() {
        let body = serde_json::json!({
            "data": [{ "user_login": "a", "user_name": "A" }, { "user_login": "b", "user_name": "B" }],
            "total": 7
        });
        assert_eq!(chatter_list(&body), Some((7, vec!["A".into(), "B".into()])));
    }

    #[test]
    fn tokens_are_not_saved_in_plain_text() {
        let mut account = TwitchAccount {
            login: "me".into(),
            ..Default::default()
        };
        account.set_tokens("access-token", "refresh-token");
        let json = serde_json::to_string(&account).unwrap();
        assert!(!json.contains("access-token") && !json.contains("refresh-token"));
    }
}
