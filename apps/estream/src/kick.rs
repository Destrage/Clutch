//! Kick account: OAuth 2.1 sign-in in the user's browser (PKCE, loopback redirect) and Kick's
//! public API, used to reply in chat. Reading chat needs no account (see `livechat::kick`).
//!
//! The client ID and secret are compiled in from `.secrets/clutch.env` (see build.rs). Kick only
//! accepts the exact redirect registered for the app, so the loopback port is fixed.

use crate::http::{get_bearer, post_form, post_json_bearer, text};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const CLIENT_ID: Option<&str> = option_env!("CLUTCH_KICK_CLIENT_ID");
const CLIENT_SECRET: Option<&str> = option_env!("CLUTCH_KICK_CLIENT_SECRET");
const SCOPES: &str = "user:read channel:read chat:write";
const API: &str = "https://api.kick.com/public/v1";
const TOKEN_URL: &str = "https://id.kick.com/oauth/token";
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

/// Whether this build can connect Kick (it has the app's ID and secret).
pub fn available() -> bool {
    CLIENT_ID.is_some() && CLIENT_SECRET.is_some()
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
    let (Some(client_id), Some(secret)) = (CLIENT_ID, CLIENT_SECRET) else {
        return Err("Connecting Kick isn't available in this build".into());
    };
    let (status, body) = post_form(
        TOKEN_URL,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", client_id),
            ("client_secret", secret),
            ("redirect_uri", REDIRECT),
            ("code_verifier", verifier),
        ],
    )?;
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
        let (Some(client_id), Some(secret)) = (CLIENT_ID, CLIENT_SECRET) else {
            return None;
        };
        let (status, body) = post_form(
            TOKEN_URL,
            &[
                ("grant_type", "refresh_token"),
                ("client_id", client_id),
                ("client_secret", secret),
                ("refresh_token", &old.refresh),
            ],
        )
        .ok()?;
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
            (status, reply) = post_json_bearer(&url, &renewed.access, &[], &body)
                .ok_or("Couldn't reach Kick")?;
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
}
