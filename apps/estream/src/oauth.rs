//! Signing in through the user's browser: PKCE values and the loopback page the platform sends
//! the browser back to (shared by YouTube and Kick).

use base64::Engine as _;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// How long to wait for the user to finish in the browser.
pub const SIGN_IN_TIME: Duration = Duration::from_secs(10 * 60);

/// 43 random URL-safe characters (a PKCE verifier, or the anti-forgery state).
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    if getrandom::fill(&mut bytes).is_err() {
        bytes = std::array::from_fn(|_| fastrand::u8(..));
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// The PKCE challenge for a verifier (S256).
pub fn challenge(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier))
}

pub fn encode(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn decode(s: &str) -> String {
    percent_encoding::percent_decode_str(&s.replace('+', " "))
        .decode_utf8_lossy()
        .into_owned()
}

/// Waits for the platform to send the browser back with a code (`platform` names it in messages).
pub fn wait_for_code(
    listener: &TcpListener,
    cancel: &AtomicBool,
    csrf: &str,
    platform: &str,
) -> Result<String, String> {
    let until = Instant::now() + SIGN_IN_TIME;
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err("Cancelled".into());
        }
        if Instant::now() >= until {
            // The browser never came back: the window was closed, or the platform showed an
            // error page (for example when the app has reached its user limit).
            return Err(format!(
                "{platform} didn't finish the sign-in. If you closed the window, try again; \
                 if {platform} showed an error, connecting may be unavailable for now."
            ));
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
        if let Some(result) = redirect_request(stream, csrf, platform) {
            return result;
        }
    }
}

/// Reads one request to the loopback address: `Some(code or error)` for the platform's
/// redirect, `None` for anything else.
fn redirect_request(
    mut stream: TcpStream,
    csrf: &str,
    platform: &str,
) -> Option<Result<String, String>> {
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
    let result = redirect_result(target, csrf, platform);
    let page = match &result {
        None => {
            let _ = stream.write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
            return None;
        }
        Some(Ok(_)) => format!(
            "Clutch is connecting to {platform}. You can close this tab and go back to Clutch."
        ),
        Some(Err(_)) => {
            format!("{platform} wasn't connected. You can close this tab; Clutch shows what happened.")
        }
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

/// The code (or the error) in a redirect to `target` (`/?state=…&code=…`, or the same on
/// `/callback`).
fn redirect_result(target: &str, csrf: &str, platform: &str) -> Option<Result<String, String>> {
    let query = target
        .strip_prefix("/?")
        .or_else(|| target.strip_prefix("/callback?"))?;
    let param = |key: &str| {
        query
            .split('&')
            .filter_map(|p| p.split_once('='))
            .find(|(k, _)| *k == key)
            .map(|(_, v)| decode(v))
    };
    if let Some(error) = param("error") {
        return Some(Err(if error == "access_denied" {
            format!("The sign-in was declined on {platform}")
        } else {
            format!("{platform} sign-in failed ({error})")
        }));
    }
    let code = param("code")?;
    if param("state").as_deref() != Some(csrf) {
        return Some(Err(
            "The sign-in didn't match this attempt. Try again.".into()
        ));
    }
    Some(Ok(code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_redirect_carries_the_code() {
        assert_eq!(
            redirect_result("/?state=abc&code=4%2F0Ab_x&scope=x", "abc", "Google"),
            Some(Ok("4/0Ab_x".into()))
        );
        assert_eq!(
            redirect_result("/callback?code=k1&state=abc", "abc", "Kick"),
            Some(Ok("k1".into()))
        );
        assert!(matches!(
            redirect_result("/?state=other&code=x", "abc", "Google"),
            Some(Err(_))
        ));
        assert!(matches!(
            redirect_result("/?error=access_denied&state=abc", "abc", "Google"),
            Some(Err(_))
        ));
        assert_eq!(redirect_result("/favicon.ico", "abc", "Google"), None);
    }

    #[test]
    fn pkce_challenge_is_url_safe() {
        let c = challenge(&random_token());
        assert_eq!(c.len(), 43);
        assert!(c.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }
}
