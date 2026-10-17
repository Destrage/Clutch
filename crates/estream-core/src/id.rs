//! Stable, globally unique identifiers (UUID v7) for project objects.

use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            /// Creates a new time-ordered unique ID.
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            /// A deterministic ID (e.g. for engine-made helper objects).
            pub fn from_u128(value: u128) -> Self {
                Self(Uuid::from_u128(value))
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, ":{}"), self.0.simple())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

define_id!(
    /// Identifies a canvas (an output video surface).
    CanvasId,
    "canvas"
);
define_id!(
    /// Identifies a scene.
    SceneId,
    "scene"
);
define_id!(
    /// Identifies a scene item (a placement of a source or a group in a scene).
    ItemId,
    "item"
);
define_id!(
    /// Identifies an automation macro.
    MacroId,
    "macro"
);

define_id!(
    /// Identifies a source.
    SourceId,
    "source"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_ordered() {
        let a = SourceId::new();
        let b = SourceId::new();
        assert_ne!(a, b);
        assert!(a < b, "UUID v7 ids are time ordered");
    }

    #[test]
    fn serializes_as_plain_uuid() {
        let id = SceneId::new();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{}\"", id.0));
        let back: SceneId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }
}
