//! Draws eStream's icon (a broadcast glyph on a violet-to-blue rounded square) and the
//! installer artwork. Everything is drawn at 4× and downsampled, for clean edges.

// Encoding images into memory buffers cannot fail for these fixed sizes and formats.
#![allow(clippy::expect_used)]

use image::imageops::FilterType;
use image::{Rgba, RgbaImage};

const TOP: [f32; 3] = [124.0, 92.0, 255.0]; // #7c5cff
const BOTTOM: [f32; 3] = [108.0, 140.0, 255.0]; // #6c8cff
const SS: u32 = 4;

/// Coverage (0–1) of the logo at a point in unit coordinates (0..1 square).
fn logo_coverage(x: f32, y: f32) -> (f32, f32) {
    // Rounded square background.
    let r = 0.22;
    let dx = (x - 0.5).abs() - (0.5 - r);
    let dy = (y - 0.5).abs() - (0.5 - r);
    let outside = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() - r;
    let background = if outside <= 0.0 { 1.0 } else { 0.0 };
    // White broadcast glyph: a dot and two arcs on each side.
    let (cx, cy) = (x - 0.5, y - 0.5);
    let dist = (cx * cx + cy * cy).sqrt();
    let mut glyph = if dist <= 0.085 { 1.0 } else { 0.0 };
    let angle = cy.atan2(cx.abs()); // 0 = horizontal, either side
    if angle.abs() <= 0.80 {
        for radius in [0.205, 0.325] {
            if (dist - radius).abs() <= 0.036 {
                glyph = 1.0;
            }
        }
    }
    (background, glyph * background)
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
        let (bg, glyph) = logo_coverage(x, y);
        if bg == 0.0 {
            *p = Rgba([0, 0, 0, 0]);
            continue;
        }
        let t = y;
        let mut c = [0u8; 3];
        for i in 0..3 {
            let base = TOP[i] + (BOTTOM[i] - TOP[i]) * t;
            c[i] = (base + (255.0 - base) * glyph).round() as u8;
        }
        *p = Rgba([c[0], c[1], c[2], 255]);
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
            // Welcome image: a violet panel on the left (the text sits on the white right side).
            for py in 0..h {
                for px in 0..164 {
                    let t = py as f32 / h as f32;
                    let c: Vec<u8> = (0..3)
                        .map(|i| (TOP[i] + (BOTTOM[i] - TOP[i]) * t) as u8)
                        .collect();
                    img.put_pixel(px, py, image::Rgb([c[0], c[1], c[2]]));
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
