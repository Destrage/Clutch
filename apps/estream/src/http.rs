//! Small HTTP helpers for the platform APIs (status codes are returned, not errors).

use serde_json::Value;
use std::time::Duration;

pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(15)))
        .user_agent(concat!("Clutch/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// POSTs a form: `(status, body)` (`Null` when the body isn't JSON).
pub fn post_form(url: &str, form: &[(&str, &str)]) -> Result<(u16, Value), String> {
    let mut response = agent()
        .post(url)
        .send_form(form.iter().copied())
        .map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .read_json::<Value>()
        .unwrap_or(Value::Null);
    Ok((status, body))
}

/// GETs JSON with a bearer token: `(status, body)`.
pub fn get_bearer(url: &str, token: &str, headers: &[(&str, &str)]) -> Option<(u16, Value)> {
    let mut request = agent()
        .get(url)
        .header("Authorization", &format!("Bearer {token}"));
    for (k, v) in headers {
        request = request.header(*k, *v);
    }
    let mut response = request
        .call()
        .map_err(|e| tracing::debug!(error = %e, url, "platform API"))
        .ok()?;
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .read_json::<Value>()
        .unwrap_or(Value::Null);
    Some((status, body))
}

/// DELETEs with a bearer token: the status.
pub fn delete_bearer(url: &str, token: &str) -> Option<u16> {
    let response = agent()
        .delete(url)
        .header("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|e| tracing::debug!(error = %e, url, "platform API"))
        .ok()?;
    Some(response.status().as_u16())
}

/// POSTs JSON: `(status, body)` (`Null` when the reply isn't JSON).
pub fn post_json(url: &str, json: &Value) -> Option<(u16, Value)> {
    let mut response = agent()
        .post(url)
        .send_json(json)
        .map_err(|e| tracing::debug!(error = %e, url, "platform API"))
        .ok()?;
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .read_json::<Value>()
        .unwrap_or(Value::Null);
    Some((status, body))
}

/// POSTs JSON with a bearer token: `(status, body)` (`Null` when the reply isn't JSON).
pub fn post_json_bearer(
    url: &str,
    token: &str,
    headers: &[(&str, &str)],
    json: &Value,
) -> Option<(u16, Value)> {
    let mut request = agent()
        .post(url)
        .header("Authorization", &format!("Bearer {token}"));
    for (k, v) in headers {
        request = request.header(*k, *v);
    }
    let mut response = request
        .send_json(json)
        .map_err(|e| tracing::debug!(error = %e, url, "platform API"))
        .ok()?;
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .read_json::<Value>()
        .unwrap_or(Value::Null);
    Some((status, body))
}

/// The error message in a platform's JSON reply ("" when there is none).
pub fn error_message(body: &Value) -> String {
    let error = body.get("error");
    let nested = error.and_then(|e| e.get("message")).and_then(Value::as_str);
    nested
        .or_else(|| body.get("message").and_then(Value::as_str))
        .or_else(|| error.and_then(Value::as_str))
        .unwrap_or_default()
        .to_owned()
}

/// A string field of a JSON object ("" when missing).
pub fn text(body: &Value, key: &str) -> String {
    body.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
