//! The Go live window's data: each platform's stream details, loaded on worker threads (see
//! `openspec/changes/add-go-live-details`).

use crate::youtube::{Broadcast, BroadcastDetails};

/// What the YouTube section shows.
#[derive(Clone, Debug, PartialEq)]
pub struct YouTubeLoaded {
    /// The channel's live and upcoming broadcasts.
    pub broadcasts: Vec<Broadcast>,
    /// Categories a broadcast can use: (id, name).
    pub categories: Vec<(String, String)>,
    /// The form for a new broadcast, pre-filled.
    pub new: BroadcastDetails,
}

/// Loads the YouTube section: a new broadcast starts from `saved`, else the last broadcast.
pub fn load_youtube(
    session: &crate::youtube::Session,
    saved: Option<&BroadcastDetails>,
) -> Result<YouTubeLoaded, String> {
    let broadcasts = session.broadcasts()?;
    let categories = session.categories().unwrap_or_else(|e| {
        tracing::debug!(error = %e, "YouTube categories");
        Vec::new()
    });
    let new = saved.cloned().unwrap_or_else(|| session.last_details());
    Ok(YouTubeLoaded {
        broadcasts,
        categories,
        new,
    })
}

/// Loads the Twitch section: the channel as it is now, else the saved details.
pub fn load_twitch(
    session: &crate::twitch::Session,
    saved: Option<&crate::twitch::ChannelDetails>,
) -> Result<crate::twitch::ChannelDetails, String> {
    prefill(session.channel_details(), saved)
}

/// Loads the Kick section: the channel as it is now, else the saved details.
pub fn load_kick(
    session: &crate::kick::Session,
    saved: Option<&crate::kick::ChannelDetails>,
) -> Result<crate::kick::ChannelDetails, String> {
    prefill(session.channel_details(), saved)
}

/// The channel's current details (they may have changed on the platform since), else the saved
/// ones when the channel can't be read.
fn prefill<T: Clone>(current: Result<T, String>, saved: Option<&T>) -> Result<T, String> {
    current.or_else(|e| {
        tracing::debug!(error = %e, "channel details; using the saved ones");
        saved.cloned().ok_or(e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_channel_wins_over_the_saved_details() {
        let saved = "saved".to_owned();
        assert_eq!(
            prefill(Ok("now".to_owned()), Some(&saved)),
            Ok("now".into())
        );
        assert_eq!(
            prefill(Err("offline".into()), Some(&saved)),
            Ok("saved".into())
        );
        assert_eq!(
            prefill::<String>(Err("offline".into()), None),
            Err("offline".into())
        );
    }
}
