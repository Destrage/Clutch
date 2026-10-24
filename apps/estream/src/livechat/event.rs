//! Stream events (follows, subs, gifts, bits, raids, paid messages) as typed values, shared by
//! the chat panel, event sounds and the alerts overlay.

use serde::{Deserialize, Serialize};

/// The kinds of stream event, the same on every platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamEventKind {
    Follow,
    /// A new subscription or YouTube membership.
    Sub,
    /// A renewed subscription or a YouTube membership milestone.
    Resub,
    /// Gifted subscriptions or memberships.
    Gift,
    /// Twitch Bits.
    Bits,
    Raid,
    /// A YouTube Super Chat or Super Sticker, or Kick's Kicks.
    Paid,
}

impl StreamEventKind {
    pub const ALL: [StreamEventKind; 7] = [
        StreamEventKind::Follow,
        StreamEventKind::Sub,
        StreamEventKind::Resub,
        StreamEventKind::Gift,
        StreamEventKind::Bits,
        StreamEventKind::Raid,
        StreamEventKind::Paid,
    ];

    pub fn label(self) -> &'static str {
        match self {
            StreamEventKind::Follow => "Follow",
            StreamEventKind::Sub => "New sub or member",
            StreamEventKind::Resub => "Resub or milestone",
            StreamEventKind::Gift => "Gifted subs",
            StreamEventKind::Bits => "Bits",
            StreamEventKind::Raid => "Raid",
            StreamEventKind::Paid => "Super Chat, Sticker or Kicks",
        }
    }
}

/// One stream event, as far as the platform describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct StreamEvent {
    pub kind: StreamEventKind,
    /// The viewer ("Anonymous" for anonymous gifts).
    pub name: String,
    /// Bits, gift count, raid viewers, Kicks, or the paid amount.
    pub amount: Option<f64>,
    /// The amount as the platform shows it, e.g. "$5.00" or "500 Kicks".
    pub amount_text: Option<String>,
    pub months: Option<u32>,
    /// "Tier 2", or a YouTube membership level.
    pub tier: Option<String>,
    pub message: Option<String>,
}

impl StreamEvent {
    pub fn new(kind: StreamEventKind, name: impl Into<String>) -> Self {
        Self {
            kind,
            name: name.into(),
            amount: None,
            amount_text: None,
            months: None,
            tier: None,
            message: None,
        }
    }

    pub fn amount(mut self, amount: f64) -> Self {
        self.amount = Some(amount);
        self
    }

    pub fn amount_text(mut self, text: impl Into<String>) -> Self {
        let text = text.into();
        self.amount_text = (!text.is_empty()).then_some(text);
        self
    }

    pub fn months(mut self, months: u32) -> Self {
        self.months = Some(months);
        self
    }

    pub fn tier(mut self, tier: impl Into<String>) -> Self {
        let tier = tier.into();
        self.tier = (!tier.is_empty()).then_some(tier);
        self
    }

    pub fn message(mut self, message: impl Into<String>) -> Self {
        let message = message.into();
        self.message = (!message.trim().is_empty()).then_some(message);
        self
    }
}
