//! obs-websocket v5 wire protocol: op codes, close codes, status codes, event subscriptions,
//! message encodings and authentication.

use base64::Engine as _;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// The obs-websocket version eStream implements.
pub const OBS_WEBSOCKET_VERSION: &str = "5.5.0";
/// The OBS Studio version eStream behaves like (some clients check it).
pub const OBS_VERSION: &str = "31.0.0";
pub const RPC_VERSION: u64 = 1;

/// Message op codes.
pub mod op {
    pub const HELLO: u64 = 0;
    pub const IDENTIFY: u64 = 1;
    pub const IDENTIFIED: u64 = 2;
    pub const REIDENTIFY: u64 = 3;
    pub const EVENT: u64 = 5;
    pub const REQUEST: u64 = 6;
    pub const REQUEST_RESPONSE: u64 = 7;
    pub const REQUEST_BATCH: u64 = 8;
    pub const REQUEST_BATCH_RESPONSE: u64 = 9;
}

/// WebSocket close codes.
pub mod close {
    pub const UNKNOWN_REASON: u16 = 4000;
    pub const MESSAGE_DECODE_ERROR: u16 = 4002;
    pub const MISSING_DATA_FIELD: u16 = 4003;
    pub const INVALID_DATA_FIELD_TYPE: u16 = 4004;
    pub const INVALID_DATA_FIELD_VALUE: u16 = 4005;
    pub const UNKNOWN_OP_CODE: u16 = 4006;
    pub const NOT_IDENTIFIED: u16 = 4007;
    pub const ALREADY_IDENTIFIED: u16 = 4008;
    pub const AUTHENTICATION_FAILED: u16 = 4009;
    pub const UNSUPPORTED_RPC_VERSION: u16 = 4010;
    pub const SESSION_INVALIDATED: u16 = 4011;
    /// Normal close when eStream shuts down ("going away").
    pub const GOING_AWAY: u16 = 1001;
}

/// Request status codes.
pub mod status {
    pub const SUCCESS: u16 = 100;
    pub const MISSING_REQUEST_TYPE: u16 = 203;
    pub const UNKNOWN_REQUEST_TYPE: u16 = 204;
    pub const GENERIC_ERROR: u16 = 205;
    pub const UNSUPPORTED_REQUEST_BATCH_EXECUTION_TYPE: u16 = 206;
    pub const NOT_READY: u16 = 207;
    pub const MISSING_REQUEST_FIELD: u16 = 300;
    pub const MISSING_REQUEST_DATA: u16 = 301;
    pub const INVALID_REQUEST_FIELD: u16 = 400;
    pub const INVALID_REQUEST_FIELD_TYPE: u16 = 401;
    pub const REQUEST_FIELD_OUT_OF_RANGE: u16 = 402;
    pub const REQUEST_FIELD_EMPTY: u16 = 403;
    pub const OUTPUT_RUNNING: u16 = 500;
    pub const OUTPUT_NOT_RUNNING: u16 = 501;
    pub const OUTPUT_DISABLED: u16 = 504;
    pub const STUDIO_MODE_ACTIVE: u16 = 505;
    pub const STUDIO_MODE_NOT_ACTIVE: u16 = 506;
    pub const RESOURCE_NOT_FOUND: u16 = 600;
    pub const RESOURCE_ALREADY_EXISTS: u16 = 601;
    pub const INVALID_RESOURCE_TYPE: u16 = 602;
    pub const INVALID_RESOURCE_STATE: u16 = 604;
    pub const INVALID_INPUT_KIND: u16 = 605;
    pub const RESOURCE_NOT_CONFIGURABLE: u16 = 606;
    pub const RESOURCE_CREATION_FAILED: u16 = 700;
    pub const RESOURCE_ACTION_FAILED: u16 = 701;
    pub const REQUEST_PROCESSING_FAILED: u16 = 702;
}

/// Event subscription bits (`eventIntent`).
pub mod intent {
    pub const GENERAL: u32 = 1 << 0;
    pub const CONFIG: u32 = 1 << 1;
    pub const SCENES: u32 = 1 << 2;
    pub const INPUTS: u32 = 1 << 3;
    pub const TRANSITIONS: u32 = 1 << 4;
    pub const FILTERS: u32 = 1 << 5;
    pub const OUTPUTS: u32 = 1 << 6;
    pub const SCENE_ITEMS: u32 = 1 << 7;
    pub const MEDIA_INPUTS: u32 = 1 << 8;
    pub const VENDORS: u32 = 1 << 9;
    pub const UI: u32 = 1 << 10;
    /// Every non-high-volume category (the default subscription).
    pub const ALL: u32 = (1 << 11) - 1;
    pub const INPUT_VOLUME_METERS: u32 = 1 << 16;
    pub const INPUT_ACTIVE_STATE_CHANGED: u32 = 1 << 17;
    pub const INPUT_SHOW_STATE_CHANGED: u32 = 1 << 18;
    pub const SCENE_ITEM_TRANSFORM_CHANGED: u32 = 1 << 19;
}

/// How a session encodes messages (picked by WebSocket subprotocol).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Encoding {
    #[default]
    Json,
    MsgPack,
}

impl Encoding {
    pub const JSON_PROTOCOL: &str = "obswebsocket.json";
    pub const MSGPACK_PROTOCOL: &str = "obswebsocket.msgpack";

    /// Picks the encoding from a `Sec-WebSocket-Protocol` header value.
    pub fn negotiate(offered: Option<&str>) -> Option<Self> {
        let Some(offered) = offered else {
            return Some(Encoding::Json);
        };
        let protocols: Vec<&str> = offered.split(',').map(str::trim).collect();
        if protocols.contains(&Self::JSON_PROTOCOL) {
            Some(Encoding::Json)
        } else if protocols.contains(&Self::MSGPACK_PROTOCOL) {
            Some(Encoding::MsgPack)
        } else if protocols.iter().all(|p| p.is_empty()) {
            Some(Encoding::Json)
        } else {
            None
        }
    }

    pub fn protocol(self) -> &'static str {
        match self {
            Encoding::Json => Self::JSON_PROTOCOL,
            Encoding::MsgPack => Self::MSGPACK_PROTOCOL,
        }
    }
}

/// Wraps `d` in a message envelope.
pub fn message(op: u64, d: Value) -> Value {
    json!({ "op": op, "d": d })
}

/// The Hello message, with an authentication challenge when a password is set.
pub fn hello(auth: Option<(&str, &str)>) -> Value {
    let mut d = json!({
        "obsWebSocketVersion": OBS_WEBSOCKET_VERSION,
        "rpcVersion": RPC_VERSION,
    });
    if let Some((challenge, salt)) = auth {
        d["authentication"] = json!({ "challenge": challenge, "salt": salt });
    }
    message(op::HELLO, d)
}

fn sha256_base64(input: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(Sha256::digest(input.as_bytes()))
}

/// The `authentication` string a client sends for `password`, `salt` and `challenge`.
pub fn auth_response(password: &str, salt: &str, challenge: &str) -> String {
    let secret = sha256_base64(&format!("{password}{salt}"));
    sha256_base64(&format!("{secret}{challenge}"))
}

/// A random base64 string of `bytes` random bytes (salts and challenges).
pub fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    if getrandom::fill(&mut buf).is_err() {
        // No OS randomness: fall back to time-based bytes (still unique per call).
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        for (i, b) in buf.iter_mut().enumerate() {
            *b = (nanos >> ((i % 16) * 8)) as u8 ^ i as u8;
        }
    }
    base64::engine::general_purpose::STANDARD.encode(buf)
}

/// Encodes a message for the wire.
pub fn encode(
    value: &Value,
    encoding: Encoding,
) -> Result<tokio_tungstenite::tungstenite::Message, String> {
    use tokio_tungstenite::tungstenite::Message;
    match encoding {
        Encoding::Json => Ok(Message::text(value.to_string())),
        Encoding::MsgPack => rmp_serde::to_vec_named(value)
            .map(Message::binary)
            .map_err(|e| e.to_string()),
    }
}

/// Decodes a message from the wire (`None` for frames that carry no message).
pub fn decode(
    message: &tokio_tungstenite::tungstenite::Message,
    encoding: Encoding,
) -> Option<Result<Value, String>> {
    use tokio_tungstenite::tungstenite::Message;
    match (message, encoding) {
        (Message::Text(text), Encoding::Json) => {
            Some(serde_json::from_str(text.as_str()).map_err(|e| e.to_string()))
        }
        (Message::Binary(bytes), Encoding::MsgPack) => {
            Some(rmp_serde::from_slice(bytes).map_err(|e| e.to_string()))
        }
        (Message::Text(_) | Message::Binary(_), _) => {
            Some(Err("wrong frame type for the negotiated encoding".into()))
        }
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn auth_matches_the_reference_algorithm() {
        // base64(sha256("supersecret" + "salt")) then base64(sha256(secret + "challenge")).
        let secret = sha256_base64("supersecretsalt");
        assert_eq!(secret.len(), 44);
        let expected = sha256_base64(&format!("{secret}challenge"));
        assert_eq!(auth_response("supersecret", "salt", "challenge"), expected);
        assert_ne!(auth_response("wrong", "salt", "challenge"), expected);
    }

    #[test]
    fn encodings_round_trip() {
        let value = json!({ "op": 6, "d": { "requestType": "GetVersion", "requestId": "1" } });
        for encoding in [Encoding::Json, Encoding::MsgPack] {
            let msg = encode(&value, encoding).unwrap();
            assert_eq!(decode(&msg, encoding).unwrap().unwrap(), value);
        }
    }

    #[test]
    fn subprotocol_negotiation() {
        assert_eq!(Encoding::negotiate(None), Some(Encoding::Json));
        assert_eq!(
            Encoding::negotiate(Some("obswebsocket.msgpack")),
            Some(Encoding::MsgPack)
        );
        assert_eq!(
            Encoding::negotiate(Some("foo, obswebsocket.json")),
            Some(Encoding::Json)
        );
        assert_eq!(Encoding::negotiate(Some("foo")), None);
    }

    #[test]
    fn random_tokens_differ() {
        assert_ne!(random_token(32), random_token(32));
    }
}
