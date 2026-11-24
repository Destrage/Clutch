//! Checks that every text color in `ui/theme.slint` stays readable on every surface.

const THEME: &str = include_str!("../ui/theme.slint");
const TEXT: [&str; 3] = ["text", "text-muted", "text-faint"];
const SURFACES: [&str; 4] = ["bg", "surface", "surface-2", "surface-3"];
/// WCAG AA for normal-size text.
const MINIMUM: f64 = 4.5;

/// The (dark, light) hex values of a `<color> name: dark ? #… : #…;` token.
fn token(name: &str) -> (String, String) {
    let prefix = format!("out property <color> {name}: dark ? ");
    let line = THEME
        .lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix(prefix.as_str()))
        .unwrap_or_else(|| panic!("no dark/light token {name} in theme.slint"));
    let (dark, light) = line
        .trim_end_matches(';')
        .split_once(" : ")
        .unwrap_or_else(|| panic!("{name}: {line}"));
    (dark.trim().to_owned(), light.trim().to_owned())
}

fn luminance(hex: &str) -> f64 {
    let hex = hex.trim_start_matches('#');
    assert!(hex.len() >= 6, "{hex}");
    let channel = |i: usize| {
        let v = u8::from_str_radix(&hex[i..i + 2], 16).expect("hex color") as f64 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4)
}

fn contrast(a: &str, b: &str) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[test]
fn text_is_readable_on_every_surface() {
    let mut failures = Vec::new();
    for text in TEXT {
        let (td, tl) = token(text);
        for surface in SURFACES {
            let (sd, sl) = token(surface);
            for (theme, t, s) in [("dark", &td, &sd), ("light", &tl, &sl)] {
                let ratio = contrast(t, s);
                if ratio < MINIMUM {
                    failures.push(format!(
                        "{theme}: {text} {t} on {surface} {s} is {ratio:.2}:1"
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn contrast_matches_known_values() {
    assert!((contrast("#000000", "#ffffff") - 21.0).abs() < 1e-9);
    assert!((contrast("#777777", "#ffffff") - 4.48).abs() < 0.01);
}
