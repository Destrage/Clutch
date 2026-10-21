//! Where Clutch's own online services are. Each address has a default, and can be changed without
//! touching the code: when building (`CLUTCH_EVENTS_URL`, `CLUTCH_RELEASES_REPO`, `CLUTCH_SITE_URL` in the
//! environment or `.secrets/clutch.env`, see build.rs) or when running (the same variables in the
//! environment, for testing against a staging relay or after a move).

/// The events relay (Kick sign-in swaps and Kick events, see `services/events-relay`).
const EVENTS_DEFAULT: &str = "https://events.destrosphere.com";
/// The GitHub repository whose releases are Clutch's updates (and where bugs are reported).
const RELEASES_DEFAULT: &str = "Destrage/clutch-beta";
/// Clutch's website (docs, downloads, changelog).
const SITE_DEFAULT: &str = "https://clutch.destrosphere.com";

/// A setting from the running environment, else the build's, else the default.
fn setting(name: &str, built: Option<&'static str>, default: &'static str) -> String {
    std::env::var(name)
        .ok()
        .or(built.map(str::to_owned))
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default.to_owned())
}

/// The events relay's base address (`https://…`, no trailing slash).
fn events_base() -> String {
    setting(
        "CLUTCH_EVENTS_URL",
        option_env!("CLUTCH_EVENTS_URL"),
        EVENTS_DEFAULT,
    )
    .trim_end_matches('/')
    .to_owned()
}

/// An HTTPS address on the events relay (`path` starts with `/`).
pub fn events_url(path: &str) -> String {
    format!("{}{path}", events_base())
}

/// A WebSocket address on the events relay (`wss://` for `https://`).
pub fn events_socket(path: &str) -> String {
    let base = events_base();
    let socket = base
        .strip_prefix("https://")
        .map(|rest| format!("wss://{rest}"))
        .or_else(|| {
            base.strip_prefix("http://")
                .map(|rest| format!("ws://{rest}"))
        })
        .unwrap_or(base);
    format!("{socket}{path}")
}

/// A page on Clutch's website (`path` starts with `/`).
pub fn site_url(path: &str) -> String {
    let base = setting("CLUTCH_SITE_URL", option_env!("CLUTCH_SITE_URL"), SITE_DEFAULT);
    format!("{}{path}", base.trim_end_matches('/'))
}

/// Where people report bugs and suggest things.
pub fn issues_url() -> String {
    format!("https://github.com/{}/issues/new/choose", releases_repo())
}

/// The GitHub repository updates come from (`owner/name`).
pub fn releases_repo() -> String {
    setting(
        "CLUTCH_RELEASES_REPO",
        option_env!("CLUTCH_RELEASES_REPO"),
        RELEASES_DEFAULT,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_overrides() {
        assert_eq!(setting("CLUTCH_TEST_UNSET_X", None, "d"), "d");
        assert_eq!(setting("CLUTCH_TEST_UNSET_X", Some("built"), "d"), "built");
        assert_eq!(setting("CLUTCH_TEST_UNSET_X", Some("  "), "d"), "d");
    }

    #[test]
    fn sockets_follow_the_scheme() {
        let base = events_base();
        let socket = events_socket("/kick/live");
        assert!(socket.ends_with("/kick/live"));
        if base.starts_with("https://") {
            assert!(socket.starts_with("wss://"));
        }
    }
}
