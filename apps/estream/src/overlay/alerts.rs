//! The Alerts source: stream events on the canvas, with an animation, a sound and live text per
//! event kind. The page (`alerts.html`) runs in the browser source; this side builds its settings
//! from the source's settings and an optional alert pack, and turns stream events into alerts.
//!
//! A pack is a folder with `pack.json` and, per kind, `<kind>/alert.json` (text box, timing,
//! style), an animation (`alert.webm`, `.webp`, `.png`, `.gif`) and sounds (`sound*.wav|ogg|mp3`),
//! optionally a bigger variant in `<kind>/big/` used from `big_at`. Files chosen on the source
//! override the pack's.

use super::OverlayServer;
use crate::livechat::Platform;
use crate::livechat::event::{StreamEvent, StreamEventKind};
use estream_engine::{PropKind, Property, PropertySchema, SelectOption, SourceType};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub const KIND: &str = "alerts";

const ANIMATIONS: &[&str] = &["webm", "webp", "png", "apng", "gif"];
const SOUNDS: &[&str] = &["wav", "ogg", "mp3"];

/// The settings key and default template of each kind.
fn key(kind: StreamEventKind) -> &'static str {
    match kind {
        StreamEventKind::Follow => "follow",
        StreamEventKind::Sub => "sub",
        StreamEventKind::Resub => "resub",
        StreamEventKind::Gift => "gift",
        StreamEventKind::Bits => "bits",
        StreamEventKind::Raid => "raid",
        StreamEventKind::Paid => "paid",
    }
}

fn default_template(kind: StreamEventKind) -> &'static str {
    match kind {
        StreamEventKind::Follow => "{name} followed",
        StreamEventKind::Sub => "{name} subscribed[ ({tier})]",
        StreamEventKind::Resub => "{name} resubscribed[ for {months} months]",
        StreamEventKind::Gift => "{name} gifted {amount}",
        StreamEventKind::Bits => "{name} cheered {amount}",
        StreamEventKind::Raid => "{name} is raiding with {amount}",
        StreamEventKind::Paid => "{name} sent {amount}",
    }
}

/// Kinds with an amount (they get a minimum).
fn has_amount(kind: StreamEventKind) -> bool {
    matches!(
        kind,
        StreamEventKind::Gift
            | StreamEventKind::Bits
            | StreamEventKind::Raid
            | StreamEventKind::Paid
    )
}

fn path_prop(key: &str, label: &str, filters: &[&str]) -> Property {
    Property::new(
        key,
        label,
        PropKind::Path {
            filters: filters.iter().map(|s| (*s).to_owned()).collect(),
        },
        "",
    )
}

fn color(r: u8, g: u8, b: u8) -> Value {
    json!({ "r": r, "g": g, "b": b, "a": 255 })
}

/// The inspector: canvas size, pack, text style, then the settings of the kind being edited.
fn schema(settings: &Value) -> PropertySchema {
    let editing = settings
        .get("edit")
        .and_then(Value::as_str)
        .and_then(|k| StreamEventKind::ALL.into_iter().find(|x| key(*x) == k))
        .unwrap_or(StreamEventKind::Follow);
    let mut props = vec![
        Property::int("width", "Width", 16, 7680, 1920),
        Property::int("height", "Height", 16, 4320, 1080),
        path_prop("pack", "Alert pack", &["json"])
            .hint("A pack's pack.json (empty: Clutch's built-in alerts)"),
        Property::new("font", "Font", PropKind::Font, "Segoe UI"),
        Property::int("size", "Text size", 12, 200, 56),
        Property::new(
            "color",
            "Text colour",
            PropKind::Color,
            color(255, 255, 255),
        ),
        Property::new(
            "name_color",
            "Name colour",
            PropKind::Color,
            color(142, 197, 255),
        ),
        Property::select(
            "edit",
            "Alert",
            StreamEventKind::ALL
                .iter()
                .map(|k| SelectOption::new(key(*k), k.label()))
                .collect(),
            "follow",
        ),
    ];
    props.extend(kind_props(editing));
    PropertySchema::new(props)
}

fn kind_props(kind: StreamEventKind) -> Vec<Property> {
    let k = key(kind);
    let mut props = vec![
        Property::bool(&format!("{k}_on"), "Show", true),
        Property::text(&format!("{k}_template"), "Text", false, default_template(kind)).hint(
            "{name} {platform} {amount} {months} {tier} {message}; [ … ] is left out when its value is missing",
        ),
        path_prop(&format!("{k}_animation"), "Animation", ANIMATIONS)
            .hint("WebM with transparency is best; empty: the pack's"),
        path_prop(&format!("{k}_sound"), "Sound", SOUNDS).hint("Empty: the pack's"),
        Property::float(&format!("{k}_volume"), "Volume", 0.0, 1.0, 0.05, 0.8),
    ];
    if has_amount(kind) {
        props.push(Property::int(
            &format!("{k}_min"),
            "Minimum amount",
            0,
            1_000_000,
            0,
        ));
    }
    props.push(Property::button(&format!("{k}_test"), "", "Test"));
    props
}

/// Every key of every kind, for validating commands.
fn validation_schema(_: &Value) -> PropertySchema {
    let mut props = schema(&Value::Null).properties;
    for kind in StreamEventKind::ALL {
        if kind != StreamEventKind::Follow {
            props.extend(kind_props(kind));
        }
    }
    PropertySchema::new(props)
}

/// Browser source settings for an Alerts source's page.
fn page(settings: &Value) -> Value {
    let int = |k: &str, d: u64| settings.get(k).and_then(Value::as_u64).unwrap_or(d);
    let source = settings
        .get("_source_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    json!({
        "url": super::page_url("alerts", source),
        "width": int("width", 1920),
        "height": int("height", 1080),
        "fps": 60,
        "css": "body { background-color: rgba(0, 0, 0, 0); margin: 0; overflow: hidden; }",
    })
}

pub fn source_type() -> SourceType {
    estream_capture::media::browser::page_source_type(
        KIND,
        "Alerts",
        "Follows, subs, gifts and more on stream, with animation and sound",
        schema,
        page,
    )
    .with_validation(validation_schema)
}

/// The test button presses of an Alerts source, per kind.
pub fn test_presses(settings: &Value) -> Vec<(StreamEventKind, i64)> {
    StreamEventKind::ALL
        .into_iter()
        .map(|k| {
            let n = settings
                .get(format!("{}_test", key(k)))
                .and_then(Value::as_i64)
                .unwrap_or(0);
            (k, n)
        })
        .collect()
}

/// A pack kind's files and settings.
#[derive(Default)]
struct PackKind {
    settings: Value,
    animation: Option<PathBuf>,
    sounds: Vec<PathBuf>,
    big: Option<Box<PackKind>>,
}

fn files_with(dir: &Path, stem: &str, exts: &[&str]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
                    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or_default();
                    name.starts_with(stem) && exts.iter().any(|x| x.eq_ignore_ascii_case(ext))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

fn load_kind(dir: &Path, nested: bool) -> PackKind {
    let settings = std::fs::read_to_string(dir.join("alert.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    let big =
        (!nested && dir.join("big").is_dir()).then(|| Box::new(load_kind(&dir.join("big"), true)));
    PackKind {
        settings,
        animation: files_with(dir, "alert", ANIMATIONS).into_iter().next(),
        sounds: files_with(dir, "sound", SOUNDS),
        big,
    }
}

/// The page's settings for one Alerts source.
pub fn config(settings: &Value, server: &OverlayServer) -> Value {
    let s = |k: &str| settings.get(k).and_then(Value::as_str).unwrap_or_default();
    let rgba = |k: &str, d: &str| {
        settings
            .get(k)
            .filter(|v| v.is_object())
            .map(|c| {
                let n = |x: &str| c.get(x).and_then(Value::as_u64).unwrap_or(255);
                format!(
                    "rgba({}, {}, {}, {:.3})",
                    n("r"),
                    n("g"),
                    n("b"),
                    n("a") as f64 / 255.0
                )
            })
            .unwrap_or_else(|| d.to_owned())
    };
    let pack_dir = Path::new(s("pack"))
        .parent()
        .filter(|_| !s("pack").is_empty());
    let mut kinds = serde_json::Map::new();
    for kind in StreamEventKind::ALL {
        let k = key(kind);
        let pack = pack_dir
            .map(|d| load_kind(&d.join(k), false))
            .unwrap_or_default();
        let own = |suffix: &str| s(&format!("{k}_{suffix}")).to_owned();
        let file_url = |p: &Path| server.asset_url(p);
        // Videos and images are shown differently; the page can't tell from the address.
        let media = |p: &Path| {
            let video = p
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("webm"));
            json!({ "url": server.asset_url(p), "video": video })
        };
        let animation = Some(own("animation"))
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .or(pack.animation.clone());
        let sounds: Vec<PathBuf> = match own("sound") {
            p if !p.is_empty() => vec![PathBuf::from(p)],
            _ => pack.sounds.clone(),
        };
        let template = Some(own("template"))
            .filter(|t| !t.trim().is_empty())
            .or_else(|| {
                pack.settings
                    .get("template")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| default_template(kind).to_owned());
        let big = pack.big.as_ref().map(|b| {
            json!({
                "animation": b.animation.as_deref().map(media),
                "sounds": b.sounds.iter().map(|p| file_url(p)).collect::<Vec<_>>(),
                "settings": b.settings,
            })
        });
        kinds.insert(
            k.to_owned(),
            json!({
                "on": settings.get(format!("{k}_on")).and_then(Value::as_bool).unwrap_or(true),
                "template": template,
                "animation": animation.as_deref().map(media),
                "sounds": sounds.iter().map(|p| file_url(p)).collect::<Vec<_>>(),
                "volume": settings.get(format!("{k}_volume")).and_then(Value::as_f64).unwrap_or(0.8),
                "min": settings.get(format!("{k}_min")).and_then(Value::as_f64).unwrap_or(0.0),
                "pack": pack.settings,
                "big": big,
            }),
        );
    }
    json!({
        "font": s("font"),
        "size": settings.get("size").and_then(Value::as_u64).unwrap_or(56),
        "color": rgba("color", "#ffffff"),
        "name_color": rgba("name_color", "#8ec5ff"),
        "kinds": kinds,
    })
}

/// "5 subs", "500 bits", "$5.00": the amount as it reads in an alert.
fn amount_display(event: &StreamEvent, platform: Platform) -> Option<String> {
    let n = event.amount?;
    let count = n.round() as u64;
    let plural = |one: &str, many: &str| {
        if count == 1 {
            format!("a {one}")
        } else {
            format!("{count} {many}")
        }
    };
    Some(match event.kind {
        StreamEventKind::Gift if platform == Platform::YouTube => {
            plural("membership", "memberships")
        }
        StreamEventKind::Gift => plural("sub", "subs"),
        StreamEventKind::Bits => format!("{count} bits"),
        StreamEventKind::Raid => format!("{count} viewers"),
        _ => event.amount_text.clone().unwrap_or_else(|| format!("{n}")),
    })
}

/// The page message for a stream event.
pub fn alert_message(event: &StreamEvent, platform: Platform, test: bool) -> String {
    json!({
        "type": "alert",
        "kind": key(event.kind),
        "name": event.name,
        "platform": platform.key(),
        "platform_name": platform.name(),
        "amount": event.amount,
        "amount_display": amount_display(event, platform),
        "months": event.months,
        "tier": event.tier,
        "message": event.message,
        "test": test,
    })
    .to_string()
}

/// A sample event for the Test button.
pub fn sample(kind: StreamEventKind) -> StreamEvent {
    let e = StreamEvent::new(kind, "Clutch");
    match kind {
        StreamEventKind::Resub => e.months(6).tier("Tier 1").message("Test resub message"),
        StreamEventKind::Gift => e.amount(5.0),
        StreamEventKind::Bits => e.amount(500.0).message("Test cheer"),
        StreamEventKind::Raid => e.amount(42.0),
        StreamEventKind::Paid => e.amount(5.0).amount_text("$5.00").message("Test message"),
        _ => e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_read_naturally() {
        let gift = StreamEvent::new(StreamEventKind::Gift, "jo").amount(5.0);
        assert_eq!(
            amount_display(&gift, Platform::Kick).as_deref(),
            Some("5 subs")
        );
        assert_eq!(
            amount_display(&gift, Platform::YouTube).as_deref(),
            Some("5 memberships")
        );
        let one = StreamEvent::new(StreamEventKind::Gift, "jo").amount(1.0);
        assert_eq!(
            amount_display(&one, Platform::Twitch).as_deref(),
            Some("a sub")
        );
        let paid = StreamEvent::new(StreamEventKind::Paid, "a")
            .amount(5.0)
            .amount_text("$5.00");
        assert_eq!(
            amount_display(&paid, Platform::YouTube).as_deref(),
            Some("$5.00")
        );
        let bits = StreamEvent::new(StreamEventKind::Bits, "a").amount(500.0);
        assert_eq!(
            amount_display(&bits, Platform::Twitch).as_deref(),
            Some("500 bits")
        );
    }

    #[test]
    fn inspector_shows_the_kind_being_edited() {
        let keys = |s: &Value| -> Vec<String> {
            schema(s).properties.iter().map(|p| p.key.clone()).collect()
        };
        let follow = keys(&json!({}));
        assert!(follow.contains(&"follow_template".to_owned()));
        assert!(
            !follow.contains(&"follow_min".to_owned()),
            "follows have no amount"
        );
        let bits = keys(&json!({ "edit": "bits" }));
        assert!(bits.contains(&"bits_min".to_owned()) && !bits.contains(&"follow_on".to_owned()));
        let all: Vec<String> = validation_schema(&Value::Null)
            .properties
            .iter()
            .map(|p| p.key.clone())
            .collect();
        assert!(all.contains(&"raid_sound".to_owned()) && all.contains(&"follow_test".to_owned()));
    }

    #[test]
    fn pack_files_and_own_overrides() {
        let dir = std::env::temp_dir().join(format!("estream-alert-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub").join("big")).unwrap();
        std::fs::write(dir.join("pack.json"), "{}").unwrap();
        std::fs::write(dir.join("sub").join("alert.webm"), b"x").unwrap();
        std::fs::write(dir.join("sub").join("sound.wav"), b"x").unwrap();
        std::fs::write(dir.join("sub").join("sound-2.wav"), b"x").unwrap();
        std::fs::write(
            dir.join("sub").join("alert.json"),
            r#"{"template":"{name} joined!","text_in":0.5}"#,
        )
        .unwrap();
        std::fs::write(dir.join("sub").join("big").join("alert.webm"), b"x").unwrap();
        let server = OverlayServer::start(0, "t").unwrap();
        let settings = json!({
            "pack": dir.join("pack.json").to_string_lossy(),
            "sub_sound": "C:/mine.ogg",
        });
        let cfg = config(&settings, &server);
        let sub = &cfg["kinds"]["sub"];
        assert_eq!(sub["template"], "{name} joined!", "the pack's template");
        assert!(
            sub["animation"]["url"]
                .as_str()
                .is_some_and(|u| u.starts_with("/asset/"))
        );
        assert_eq!(sub["animation"]["video"], true);
        assert_eq!(
            sub["sounds"].as_array().map(Vec::len),
            Some(1),
            "own sound replaces the pack's two"
        );
        assert_eq!(sub["pack"]["text_in"], 0.5);
        assert!(sub["big"]["animation"]["url"].is_string());
        assert_eq!(
            cfg["kinds"]["follow"]["template"], "{name} followed",
            "default without a pack kind"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
