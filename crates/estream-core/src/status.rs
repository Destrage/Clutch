use serde::{Deserialize, Serialize};

/// Availability of a source, shown in the UI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "reason", rename_all = "snake_case")]
pub enum SourceStatus {
    Active,
    /// The device, window or file is not available right now.
    Unavailable(String),
    Error(String),
}
