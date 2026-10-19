//! Typed property schemas for source settings. Each source type declares its
//! settings once; the UI renders them generically and the engine validates
//! settings against them before applying commands.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectOption {
    pub value: Value,
    pub label: String,
}

impl SelectOption {
    pub fn new(value: impl Into<Value>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PropKind {
    Bool,
    Int {
        min: i64,
        max: i64,
        step: i64,
    },
    Float {
        min: f64,
        max: f64,
        step: f64,
    },
    Text {
        multiline: bool,
    },
    /// `Rgba8` object (`{"r":..,"g":..,"b":..,"a":..}`).
    Color,
    /// File path with extension filters (e.g. `["png", "jpg"]`).
    Path {
        filters: Vec<String>,
    },
    /// One of a list of values (devices, windows, modes…).
    Select {
        options: Vec<SelectOption>,
    },
    /// Font family name.
    Font,
    /// A button with this text. Pressing it adds one to the (integer) value, which tells the
    /// source to act (e.g. "Choose again…" reopens the system's screen picker).
    Button {
        text: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Property {
    pub key: String,
    pub label: String,
    pub kind: PropKind,
    pub default: Value,
    /// Short help shown under the field.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hint: String,
}

impl Property {
    pub fn new(key: &str, label: &str, kind: PropKind, default: impl Into<Value>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            kind,
            default: default.into(),
            hint: String::new(),
        }
    }

    pub fn hint(mut self, hint: &str) -> Self {
        self.hint = hint.into();
        self
    }

    pub fn bool(key: &str, label: &str, default: bool) -> Self {
        Self::new(key, label, PropKind::Bool, default)
    }

    pub fn int(key: &str, label: &str, min: i64, max: i64, default: i64) -> Self {
        Self::new(key, label, PropKind::Int { min, max, step: 1 }, default)
    }

    pub fn float(key: &str, label: &str, min: f64, max: f64, step: f64, default: f64) -> Self {
        Self::new(key, label, PropKind::Float { min, max, step }, default)
    }

    pub fn text(key: &str, label: &str, multiline: bool, default: &str) -> Self {
        Self::new(key, label, PropKind::Text { multiline }, default)
    }

    pub fn button(key: &str, label: &str, text: &str) -> Self {
        Self::new(key, label, PropKind::Button { text: text.into() }, 0)
    }

    pub fn select(
        key: &str,
        label: &str,
        options: Vec<SelectOption>,
        default: impl Into<Value>,
    ) -> Self {
        Self::new(key, label, PropKind::Select { options }, default)
    }

    /// Checks a single value against this property.
    pub fn check(&self, value: &Value) -> Result<(), String> {
        let label = &self.label;
        match &self.kind {
            PropKind::Bool => value
                .as_bool()
                .map(|_| ())
                .ok_or(format!("{label} must be on or off")),
            PropKind::Int { min, max, .. } => {
                let v = value
                    .as_i64()
                    .ok_or(format!("{label} must be a whole number"))?;
                if v < *min || v > *max {
                    return Err(format!("{label} must be between {min} and {max} (got {v})"));
                }
                Ok(())
            }
            PropKind::Float { min, max, .. } => {
                let v = value.as_f64().ok_or(format!("{label} must be a number"))?;
                if v < *min || v > *max {
                    return Err(format!("{label} must be between {min} and {max} (got {v})"));
                }
                Ok(())
            }
            PropKind::Text { .. } | PropKind::Font | PropKind::Path { .. } => value
                .as_str()
                .map(|_| ())
                .ok_or(format!("{label} must be text")),
            PropKind::Button { .. } => value
                .as_i64()
                .map(|_| ())
                .ok_or(format!("{label} must be a whole number")),
            PropKind::Color => serde_json::from_value::<estream_core::Rgba8>(value.clone())
                .map(|_| ())
                .map_err(|_| format!("{label} must be a color")),
            // Options are dynamic (devices come and go), so any value of a known
            // option type is accepted; sources report unavailability at runtime.
            PropKind::Select { options } => {
                if options.is_empty()
                    || options.iter().any(|o| o.value == *value)
                    || value.is_string()
                {
                    Ok(())
                } else {
                    Err(format!("{label} has an unsupported value"))
                }
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PropertySchema {
    pub properties: Vec<Property>,
}

impl PropertySchema {
    pub fn new(properties: Vec<Property>) -> Self {
        Self { properties }
    }

    pub fn get(&self, key: &str) -> Option<&Property> {
        self.properties.iter().find(|p| p.key == key)
    }

    /// Validates every known key present in `settings`. Unknown keys are kept
    /// (forward compatibility) and missing keys use defaults.
    pub fn validate(&self, settings: &Value) -> Result<(), String> {
        let Some(map) = settings.as_object() else {
            return if settings.is_null() {
                Ok(())
            } else {
                Err("settings must be an object".into())
            };
        };
        for prop in &self.properties {
            if let Some(value) = map.get(&prop.key) {
                prop.check(value)?;
            }
        }
        Ok(())
    }

    /// `settings` with defaults filled in for missing keys.
    pub fn with_defaults(&self, settings: &Value) -> Value {
        let mut map = settings.as_object().cloned().unwrap_or_else(Map::new);
        for prop in &self.properties {
            map.entry(prop.key.clone())
                .or_insert_with(|| prop.default.clone());
        }
        Value::Object(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> PropertySchema {
        PropertySchema::new(vec![
            Property::int("fps", "Frame rate", 1, 60, 30),
            Property::bool("cursor", "Show cursor", true),
            Property::new(
                "color",
                "Color",
                PropKind::Color,
                json!({"r":1,"g":2,"b":3,"a":255}),
            ),
        ])
    }

    #[test]
    fn rejects_out_of_range_values() {
        let err = schema().validate(&json!({ "fps": 240 })).unwrap_err();
        assert!(err.contains("between 1 and 60"), "{err}");
        assert!(
            schema()
                .validate(&json!({ "fps": 60, "cursor": false }))
                .is_ok()
        );
        assert!(schema().validate(&json!({ "cursor": "yes" })).is_err());
        assert!(schema().validate(&json!({ "color": {"r": 0} })).is_err());
    }

    #[test]
    fn defaults_and_unknown_keys() {
        let filled = schema().with_defaults(&json!({ "fps": 10, "future": true }));
        assert_eq!(filled["fps"], 10);
        assert_eq!(filled["cursor"], true);
        assert_eq!(filled["future"], true);
        assert!(schema().validate(&Value::Null).is_ok());
    }
}
