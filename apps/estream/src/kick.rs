//! Kick account: OAuth 2.1 sign-in in the user's browser (PKCE, loopback redirect) and Kick's
//! public API, used to reply in chat. Reading chat needs no account (see `livechat::kick`).
//!
//! The client ID is compiled in from `.secrets/clutch.env` (see build.rs). The app's secret is not
//! in Clutch at all: the sign-in code and renewals go to Clutch's relay, which adds the secret and
//! asks Kick for tokens (`services/events-relay`), so the secret can be changed without a new
//! Clutch. Kick only accepts the exact redirect registered for the app, so the loopback port is
//! fixed.

use crate::http::{get_bearer, post_json, post_json_bearer, send_json_bearer, text};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const CLIENT_ID: Option<&str> = option_env!("CLUTCH_KICK_CLIENT_ID");
const SCOPES: &str =
    "user:read channel:read chat:write events:subscribe streamkey:read channel:write";
/// The events Clutch asks Kick to send to the relay (see `livechat::kick::events`).
const EVENTS: [&str; 5] = [
    "channel.followed",
    "channel.subscription.new",
    "channel.subscription.renewal",
    "channel.subscription.gifts",
    "kicks.gifted",
];
const API: &str = "https://api.kick.com/public/v1";
/// Registered with the Kick app; Kick refuses any other.
const PORT: u16 = 47615;
const REDIRECT: &str = "http://localhost:47615/callback";

/// A connected Kick account. The tokens live in memory only; they are saved encrypted.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KickAccount {
    pub name: String,
    pub user_id: u64,
    #[serde(skip)]
    pub access: String,
    #[serde(skip)]
    pub refresh: String,
    pub access_protected: String,
    pub refresh_protected: String,
}

impl KickAccount {
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

/// Whether this build can connect Kick (it has the app's ID).
pub fn available() -> bool {
    CLIENT_ID.is_some()
}

// ---- Connecting ----

/// Where a sign-in is.
#[derive(Clone, Debug, PartialEq)]
pub enum Connect {
    /// Waiting for the user to approve in the browser.
    Waiting,
    Done(KickAccount),
    Failed(String),
}

/// A sign-in in progress, run in the background. Dropping it cancels it.
pub struct Connecting {
    url: String,
    state: Arc<Mutex<Connect>>,
    cancel: Arc<AtomicBool>,
}

impl Connecting {
    /// Starts listening for Kick's redirect; open [`Self::url`] in the browser.
    pub fn start() -> Result<Self, String> {
        let Some(client_id) = CLIENT_ID.filter(|_| available()) else {
            return Err("Connecting Kick isn't available in this build".into());
        };
        let listener = TcpListener::bind(("127.0.0.1", PORT)).map_err(|_| {
            format!("Port {PORT} is busy, so Kick can't send the sign-in back. Close the app using it and try again.")
        })?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let verifier = crate::oauth::random_token();
        let csrf = crate::oauth::random_token();
        let url = format!(
            "https://id.kick.com/oauth/authorize?client_id={}&response_type=code&redirect_uri={}\
             &scope={}&code_challenge={}&code_challenge_method=S256&state={csrf}",
            crate::oauth::encode(client_id),
            crate::oauth::encode(REDIRECT),
            crate::oauth::encode(SCOPES),
            crate::oauth::challenge(&verifier),
        );
        let state = Arc::new(Mutex::new(Connect::Waiting));
        let cancel = Arc::new(AtomicBool::new(false));
        let (s, c) = (state.clone(), cancel.clone());
        std::thread::Builder::new()
            .name("estream-kick-connect".into())
            .spawn(move || {
                let result = crate::oauth::wait_for_code(&listener, &c, &csrf, "Kick")
                    .and_then(|code| exchange(&code, &verifier));
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

    /// Kick's sign-in page for this attempt.
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

/// Trades the code for tokens and finds who signed in.
fn exchange(code: &str, verifier: &str) -> Result<KickAccount, String> {
    let request = serde_json::json!({
        "code": code,
        "code_verifier": verifier,
        "redirect_uri": REDIRECT,
    });
    let (status, body) = post_json(&crate::services::events_url("/kick/token"), &request)
        .ok_or("Couldn't reach Clutch's sign-in service")?;
    let access = text(&body, "access_token");
    if status != 200 || access.is_empty() {
        return Err(format!(
            "Kick refused the sign-in ({})",
            crate::http::error_message(&body)
        ));
    }
    let (status, users) =
        get_bearer(&format!("{API}/users"), &access, &[]).ok_or("Couldn't reach Kick")?;
    let user = users.get("data").and_then(|d| d.get(0));
    let (Some(user), 200) = (user, status) else {
        return Err(format!("Kick refused the request ({status})"));
    };
    let mut account = KickAccount {
        name: text(user, "name"),
        user_id: user.get("user_id").and_then(Value::as_u64).unwrap_or(0),
        ..Default::default()
    };
    account.set_tokens(&access, &text(&body, "refresh_token"));
    Ok(account)
}

// ---- Using the account ----

/// The account shared by the UI and the background work, which renews its token.
pub type Session = crate::account::Session<KickAccount>;

impl Session {
    fn renew(&self, old: &KickAccount) -> Option<KickAccount> {
        let request = serde_json::json!({ "refresh_token": old.refresh });
        let (status, body) = post_json(&crate::services::events_url("/kick/refresh"), &request)?;
        let renewed = if status == 200 {
            let mut account = old.clone();
            let refresh = Some(text(&body, "refresh_token"))
                .filter(|r| !r.is_empty())
                .unwrap_or_else(|| old.refresh.clone());
            account.set_tokens(&text(&body, "access_token"), &refresh);
            Some(account)
        } else {
            tracing::warn!(status, "Kick sign-in could not be renewed");
            None
        };
        self.renewed(old, renewed)
    }

    /// The access token to use now (`renew`: after it was refused).
    pub fn token(&self, renew: bool) -> Option<String> {
        let account = self.account()?;
        if renew {
            return self.renew(&account).map(|a| a.access);
        }
        Some(account.access)
    }

    /// Asks Kick to send the connected channel's follows, subs, gifts and Kicks to Clutch's
    /// events relay (Kick sends them to the webhook address set for the app), skipping the ones
    /// already asked for. `Err(true)` when the sign-in lacks the permission.
    pub fn subscribe_events(&self) -> Result<(), bool> {
        let account = self.account().ok_or(true)?;
        let url = format!("{API}/events/subscriptions");
        let mut token = account.access.clone();
        let (mut status, mut existing) = get_bearer(&url, &token, &[]).ok_or(false)?;
        if status == 401 {
            token = self.renew(&account).ok_or(true)?.access;
            (status, existing) = get_bearer(&url, &token, &[]).ok_or(false)?;
        }
        if status == 401 || status == 403 {
            return Err(true);
        }
        let have: Vec<String> = existing
            .get("data")
            .and_then(Value::as_array)
            .map(|subs| {
                subs.iter()
                    .filter(|s| {
                        s.get("broadcaster_user_id").and_then(Value::as_u64)
                            == Some(account.user_id)
                    })
                    .map(|s| text(s, "event"))
                    .collect()
            })
            .unwrap_or_default();
        let wanted: Vec<Value> = EVENTS
            .iter()
            .filter(|name| !have.iter().any(|h| h == *name))
            .map(|name| serde_json::json!({ "name": name, "version": 1 }))
            .collect();
        if wanted.is_empty() {
            return Ok(());
        }
        let body = serde_json::json!({ "events": wanted, "method": "webhook" });
        let (status, reply) = post_json_bearer(&url, &token, &[], &body).ok_or(false)?;
        match status {
            200..=299 => Ok(()),
            401 | 403 => Err(true),
            _ => {
                tracing::warn!(status, error = %crate::http::error_message(&reply), "Kick event subscription");
                Err(false)
            }
        }
    }

    /// The connected channel's stream server and key: `(server, key)`.
    pub fn stream_setup(&self) -> Result<(String, String), String> {
        let (status, body) = self.get("channels")?;
        if status == 401 || status == 403 {
            return Err("Connect Kick again in Settings → Stream to get the stream key".into());
        }
        let stream = body
            .get("data")
            .and_then(|d| d.get(0))
            .and_then(|c| c.get("stream"));
        let (server, key) = stream
            .map(|s| (text(s, "url"), text(s, "key")))
            .unwrap_or_default();
        if key.is_empty() {
            return Err(
                "Kick didn't give a stream key. Connect Kick again in Settings → Stream.".into(),
            );
        }
        Ok((server, key))
    }

    /// Sends a message to a channel's chat as the connected account.
    pub fn send_chat(&self, channel: &str, message: &str) -> Result<(), String> {
        let account = self
            .account()
            .ok_or("Connect Kick in Settings → Stream to reply there")?;
        let slug = crate::chat::kick_slug(channel).ok_or("Kick: no channel set")?;
        let broadcaster = crate::livechat::kick::channel(&slug)
            .map(|c| c.user_id)
            .ok_or("Couldn't find the Kick channel")?;
        let body = serde_json::json!({
            "type": "user",
            "content": message,
            "broadcaster_user_id": broadcaster,
        });
        let url = format!("{API}/chat");
        let (mut status, mut reply) =
            post_json_bearer(&url, &account.access, &[], &body).ok_or("Couldn't reach Kick")?;
        if status == 401
            && let Some(renewed) = self.renew(&account)
        {
            (status, reply) =
                post_json_bearer(&url, &renewed.access, &[], &body).ok_or("Couldn't reach Kick")?;
        }
        match status {
            200 => Ok(()),
            401 | 403 => Err("Connect Kick again in Settings → Stream to reply from Clutch".into()),
            _ => Err(format!(
                "Kick didn't send it ({})",
                crate::http::error_message(&reply)
            )),
        }
    }

    /// GETs an API path, renewing the sign-in once if it expired: `(status, body)`.
    fn get(&self, path: &str) -> Result<(u16, Value), String> {
        let account = self
            .account()
            .ok_or("Connect Kick in Settings → Stream first")?;
        let url = format!("{API}/{path}");
        let (mut status, mut body) =
            get_bearer(&url, &account.access, &[]).ok_or("Couldn't reach Kick")?;
        if status == 401
            && let Some(renewed) = self.renew(&account)
        {
            (status, body) = get_bearer(&url, &renewed.access, &[]).ok_or("Couldn't reach Kick")?;
        }
        Ok((status, body))
    }

    /// The connected channel's stream title and category.
    pub fn channel_details(&self) -> Result<ChannelDetails, String> {
        let (status, body) = self.get("channels")?;
        if status != 200 {
            return Err("Couldn't read your Kick channel".into());
        }
        details_of(&body).ok_or_else(|| "Couldn't read your Kick channel".into())
    }

    /// Saves the connected channel's stream title and category.
    pub fn set_channel_details(&self, details: &ChannelDetails) -> Result<(), String> {
        let account = self
            .account()
            .ok_or("Connect Kick in Settings → Stream first")?;
        let url = format!("{API}/channels");
        let body = channel_update(details);
        let (mut status, mut reply) = send_json_bearer("PATCH", &url, &account.access, &[], &body)
            .ok_or("Couldn't reach Kick")?;
        if status == 401
            && let Some(renewed) = self.renew(&account)
        {
            (status, reply) = send_json_bearer("PATCH", &url, &renewed.access, &[], &body)
                .ok_or("Couldn't reach Kick")?;
        }
        match status {
            200..=299 => Ok(()),
            401 | 403 => Err(
                "Connect Kick again in Settings → Stream to change your title from Clutch".into(),
            ),
            _ => Err(format!(
                "Kick didn't save the title ({})",
                crate::http::error_message(&reply)
            )),
        }
    }

    /// Kick categories matching `query`: (id, name).
    pub fn search_categories(&self, query: &str) -> Result<Vec<(u64, String)>, String> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let (status, body) = self.get(&format!(
            "categories?q={}",
            crate::oauth::encode(query.trim())
        ))?;
        if status != 200 {
            return Err("Couldn't search Kick's categories".into());
        }
        Ok(categories_of(&body))
    }
}

// ---- Channel details (the Go live window) ----

/// What viewers see about the channel's stream.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelDetails {
    pub title: String,
    /// Kick's category id and name (0 and "" for none).
    pub category_id: u64,
    pub category: String,
}

fn details_of(body: &Value) -> Option<ChannelDetails> {
    let c = body.get("data")?.get(0)?;
    Some(ChannelDetails {
        title: text(c, "stream_title"),
        category_id: c["category"]["id"].as_u64().unwrap_or(0),
        category: text(&c["category"], "name"),
    })
}

fn channel_update(details: &ChannelDetails) -> Value {
    let mut body = serde_json::json!({ "stream_title": details.title });
    if details.category_id != 0 {
        body["category_id"] = details.category_id.into();
    }
    body
}

fn categories_of(body: &Value) -> Vec<(u64, String)> {
    body["data"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|c| Some((c["id"].as_u64()?, text(c, "name"))))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_not_saved_in_plain_text() {
        let mut account = KickAccount {
            name: "me".into(),
            ..Default::default()
        };
        account.set_tokens("access-token", "refresh-token");
        let json = serde_json::to_string(&account).unwrap();
        assert!(!json.contains("access-token") && !json.contains("refresh-token"));
    }

    #[test]
    fn channel_details_are_read_and_written() {
        let body = serde_json::json!({ "data": [{
            "slug": "destragex", "stream_title": "Ranked",
            "category": { "id": 15, "name": "Just Chatting", "thumbnail": "t" }
        }]});
        let details = details_of(&body).unwrap();
        assert_eq!(
            details,
            ChannelDetails {
                title: "Ranked".into(),
                category_id: 15,
                category: "Just Chatting".into(),
            }
        );
        let update = channel_update(&details);
        assert_eq!(update["stream_title"], "Ranked");
        assert_eq!(update["category_id"], 15);
        let no_category = channel_update(&ChannelDetails {
            title: "x".into(),
            ..Default::default()
        });
        assert!(no_category.get("category_id").is_none());
    }

    #[test]
    fn categories_come_from_the_search() {
        let body = serde_json::json!({ "data": [
            { "id": 28, "name": "Grand Theft Auto V" },
            { "name": "no id" }
        ]});
        assert_eq!(
            categories_of(&body),
            vec![(28, "Grand Theft Auto V".into())]
        );
    }
}
