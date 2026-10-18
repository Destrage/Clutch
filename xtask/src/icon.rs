//! Draws the app icon (a faceted silver C on a slate rounded square) and the installer
//! artwork. Everything is drawn at 4× and downsampled, for clean edges.

// Encoding images into memory buffers cannot fail for these fixed sizes and formats.
#![allow(clippy::expect_used)]

use image::imageops::FilterType;
use image::{Rgba, RgbaImage};

/// Slate background: a radial gradient lit from the top right.
const SLATE: [[f32; 3]; 3] = [
    [91.0, 117.0, 144.0], // #5b7590
    [42.0, 54.0, 68.0],   // #2a3644
    [22.0, 29.0, 37.0],   // #161d25
];
/// Small sizes use one flat slate, so the C keeps its contrast.
const SLATE_FLAT: [f32; 3] = [38.0, 49.0, 61.0]; // #26313d
/// Silver C: a diagonal gradient from white through silver to steel.
const SILVER: [[f32; 3]; 3] = [
    [255.0, 255.0, 255.0],
    [220.0, 227.0, 234.0], // #dce3ea
    [143.0, 161.0, 179.0], // #8fa1b3
];
const SS: u32 = 4;

/// The C outline in a 100×100 box (see the Faceted C board on the logo canvas).
const C_SHAPE: [(f32, f32); 12] = [
    (70.0, 24.0),
    (40.0, 20.0),
    (22.0, 37.0),
    (22.0, 63.0),
    (40.0, 80.0),
    (70.0, 76.0),
    (72.0, 62.0),
    (46.0, 64.0),
    (37.0, 56.0),
    (37.0, 44.0),
    (46.0, 36.0),
    (72.0, 38.0),
];
/// A bolder C for 32 px and below.
const C_BOLD: [(f32, f32); 12] = [
    (72.0, 22.0),
    (40.0, 18.0),
    (20.0, 36.0),
    (20.0, 64.0),
    (40.0, 82.0),
    (72.0, 78.0),
    (74.0, 62.0),
    (48.0, 65.0),
    (39.0, 57.0),
    (39.0, 43.0),
    (48.0, 35.0),
    (74.0, 38.0),
];
/// The lit facet (top left) and the shaded facet (bottom left).
const FACET_LIT: [(f32, f32); 4] = [(40.0, 20.0), (22.0, 37.0), (37.0, 44.0), (46.0, 36.0)];
const FACET_DARK: [(f32, f32); 4] = [(22.0, 63.0), (40.0, 80.0), (46.0, 64.0), (37.0, 56.0)];

/// Even-odd point-in-polygon test.
fn inside(poly: &[(f32, f32)], x: f32, y: f32) -> bool {
    let mut hit = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            hit = !hit;
        }
        j = i;
    }
    hit
}

/// Linear interpolation through three stops at 0, `mid` and 1.
fn ramp(stops: &[[f32; 3]; 3], mid: f32, t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    let (a, b, u) = if t < mid {
        (stops[0], stops[1], t / mid)
    } else {
        (stops[1], stops[2], (t - mid) / (1.0 - mid))
    };
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * u)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

/// The colour of the logo at a point in unit coordinates (0..1 square), or `None` outside
/// the rounded square. `size` is the final pixel size, which picks the level of detail.
fn logo_color(x: f32, y: f32, size: u32) -> Option<[f32; 3]> {
    let r = 0.24;
    let dx = (x - 0.5).abs() - (0.5 - r);
    let dy = (y - 0.5).abs() - (0.5 - r);
    if (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() > r {
        return None;
    }
    let small = size <= 32;
    let detailed = size >= 64;
    let background = if small {
        SLATE_FLAT
    } else {
        let d = ((x - 0.75).powi(2) + (y - 0.3).powi(2)).sqrt() / 0.9;
        ramp(&SLATE, 0.5, d)
    };
    let (px, py) = (x * 100.0, y * 100.0);
    let shape: &[(f32, f32)] = if small { &C_BOLD } else { &C_SHAPE };
    if !inside(shape, px, py) {
        return Some(background);
    }
    let mut c = if detailed {
        ramp(&SILVER, 0.45, (x + y) / 2.0)
    } else {
        SILVER[1]
    };
    if !small && inside(&FACET_LIT, px, py) {
        c = mix(c, [255.0; 3], 0.3);
    }
    if detailed && inside(&FACET_DARK, px, py) {
        c = mix(c, SLATE[1], 0.25);
    }
    Some(c)
}

/// The logo as an RGBA image of `size`×`size`.
pub fn logo(size: u32) -> RgbaImage {
    let big = size * SS;
    let mut img = RgbaImage::new(big, big);
    for (px, py, p) in img.enumerate_pixels_mut() {
        let (x, y) = (
            (px as f32 + 0.5) / big as f32,
            (py as f32 + 0.5) / big as f32,
        );
        *p = match logo_color(x, y, size) {
            Some(c) => Rgba([c[0].round() as u8, c[1].round() as u8, c[2].round() as u8, 255]),
            None => Rgba([0, 0, 0, 0]),
        };
    }
    image::imageops::resize(&img, size, size, FilterType::Lanczos3)
}

/// A multi-size Windows icon (16–256 px).
pub fn ico() -> Vec<u8> {
    use image::codecs::ico::{IcoEncoder, IcoFrame};
    let frames: Vec<IcoFrame> = [16u32, 24, 32, 48, 64, 128, 256]
        .iter()
        .map(|&s| {
            let img = logo(s);
            IcoFrame::as_png(img.as_raw(), s, s, image::ExtendedColorType::Rgba8)
                .expect("icon frame")
        })
        .collect();
    let mut out = Vec::new();
    IcoEncoder::new(&mut out)
        .encode_images(&frames)
        .expect("icon encoding");
    out
}

/// Installer artwork: the banner (493×58) and the welcome/finish image (493×312), as BMP.
pub fn installer_art() -> (Vec<u8>, Vec<u8>) {
    let paint = |w: u32, h: u32, logo_size: u32, x: u32, y: u32| -> Vec<u8> {
        let mut img = image::RgbImage::from_pixel(w, h, image::Rgb([255, 255, 255]));
        if h > 100 {
            // Welcome image: a slate panel on the left (the text sits on the white right side).
            for py in 0..h {
                for px in 0..164 {
                    let d = ((px as f32 / 164.0 - 0.75).powi(2)
                        + (py as f32 / h as f32 - 0.3).powi(2))
                    .sqrt()
                        / 0.9;
                    let c = ramp(&SLATE, 0.5, d);
                    img.put_pixel(px, py, image::Rgb(c.map(|v| v as u8)));
                }
            }
        }
        let logo = logo(logo_size);
        for (lx, ly, p) in logo.enumerate_pixels() {
            let a = p[3] as f32 / 255.0;
            let dst = img.get_pixel_mut(x + lx, y + ly);
            for i in 0..3 {
                dst[i] = (p[i] as f32 * a + dst[i] as f32 * (1.0 - a)) as u8;
            }
        }
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Bmp)
            .expect("bmp encoding");
        out.into_inner()
    };
    (paint(493, 58, 44, 438, 7), paint(493, 312, 112, 26, 100))
}
