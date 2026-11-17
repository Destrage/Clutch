//! CPU side of item masks: the shapes become an alpha (R8) image, and edge effects become two
//! straight-alpha sRGB images, one drawn under the item (shadow, outer glow) and one over it
//! (inner glow, border). Runs off the video thread, only when a mask or an item's size changes.

use estream_core::Rgba8;
use estream_project::mask::{combine, coverage_start, invert};
use estream_project::{BorderPlacement, GlowPlacement, ItemMask, MaskGeometry};
use glam::Vec2;
use tiny_skia::{FillRule, PathBuilder, Transform};

/// Longest side of a baked image (the item plus its effects).
pub const MAX_SIDE: f32 = 2048.0;

/// A baked image, rows top to bottom.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// One byte per pixel for a mask, four (RGBA, straight alpha, sRGB) for an effect layer.
    pub pixels: Vec<u8>,
}

/// An effect layer: an image covering the item grown by `pad` on each side (item units).
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub image: Image,
    pub pad: Vec2,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Baked {
    /// The item's alpha; `None` when the shapes don't cut it (off, or effects only).
    pub mask: Option<Image>,
    pub under: Option<Layer>,
    pub over: Option<Layer>,
}

/// Bakes `mask` for an item drawn `size` canvas pixels large.
pub fn bake(mask: &ItemMask, size: Vec2) -> Baked {
    bake_with(mask, size, MAX_SIDE)
}

/// [`bake`] with images at most `max_side` pixels on their long side (a quick, small look).
pub fn bake_with(mask: &ItemMask, size: Vec2, max_side: f32) -> Baked {
    let size = size.max(Vec2::ONE);
    let effects = &mask.effects;
    let extent = if effects.is_empty() {
        0.0
    } else {
        effects.extent()
    };
    // Texture pixels per canvas pixel.
    let s = (max_side.max(1.0) / (size.max_element() + 2.0 * extent)).min(1.0);
    let w = ((size.x * s).round() as usize).max(1);
    let h = ((size.y * s).round() as usize).max(1);

    let coverage = mask.cuts().then(|| shapes(mask, w, h, s));
    let mut baked = Baked {
        mask: coverage.as_ref().map(|c| Image {
            width: w as u32,
            height: h as u32,
            pixels: c.iter().map(|v| to_byte(*v)).collect(),
        }),
        ..Baked::default()
    };
    if effects.is_empty() {
        return baked;
    }

    // The effects grid: the item with `p` pixels around it (at least one, so an edge on the
    // item's border has an outside).
    let p = (extent * s).ceil() as usize + 1;
    let (gw, gh) = (w + 2 * p, h + 2 * p);
    let mut c = vec![0.0f32; gw * gh];
    for y in 0..h {
        let row = &mut c[(y + p) * gw + p..][..w];
        match &coverage {
            Some(cov) => row.copy_from_slice(&cov[y * w..][..w]),
            None => row.fill(1.0),
        }
    }
    let pad = Vec2::new(p as f32 / w as f32, p as f32 / h as f32);
    let layer = |paints: &[(Rgba8, &[f32])]| Layer {
        image: Image {
            width: gw as u32,
            height: gh as u32,
            pixels: composite(paints, gw * gh),
        },
        pad,
    };

    let mut under: Vec<(Rgba8, Vec<f32>)> = Vec::new();
    let mut over: Vec<(Rgba8, Vec<f32>)> = Vec::new();
    if let Some(shadow) = effects.shadow {
        let mut a = c.clone();
        blur(&mut a, gw, gh, reach_radius(shadow.softness * s), 0.0);
        let a = shift(&a, gw, gh, (shadow.offset * s).round());
        let strength = shadow.opacity.clamp(0.0, 1.0);
        under.push((
            shadow.color,
            a.iter()
                .zip(&c)
                .map(|(a, c)| a * strength * (1.0 - c))
                .collect(),
        ));
    }
    if let Some(glow) = effects.glow {
        let radius = reach_radius(glow.size * s);
        let gain = 2.0 * glow.strength.clamp(0.0, 1.0);
        if glow.placement != GlowPlacement::Inside {
            let mut a = c.clone();
            blur(&mut a, gw, gh, radius, 0.0);
            under.push((
                glow.color,
                a.iter()
                    .zip(&c)
                    .map(|(a, c)| (a * gain).min(1.0) * (1.0 - c))
                    .collect(),
            ));
        }
        if glow.placement != GlowPlacement::Outside {
            let mut a: Vec<f32> = c.iter().map(|c| 1.0 - c).collect();
            blur(&mut a, gw, gh, radius, 1.0);
            over.push((
                glow.color,
                a.iter()
                    .zip(&c)
                    .map(|(a, c)| (a * gain).min(1.0) * c)
                    .collect(),
            ));
        }
    }
    if let Some(border) = effects.border {
        let d = signed_distance(&c, gw, gh);
        let width = border.width.max(0.0) * s;
        let alpha = d
            .iter()
            .zip(&c)
            .map(|(d, c)| match border.placement {
                BorderPlacement::Outside => (width - d + 0.5).clamp(0.0, 1.0) * (1.0 - c),
                BorderPlacement::Inside => (width + d + 0.5).clamp(0.0, 1.0) * c,
                BorderPlacement::Center => (width * 0.5 - d.abs() + 0.5).clamp(0.0, 1.0),
            })
            .collect();
        over.push((border.color, alpha));
    }
    if effects.has_under() {
        baked.under = Some(layer(&refs(&under)));
    }
    if effects.has_over() {
        baked.over = Some(layer(&refs(&over)));
    }
    baked
}

/// The combined coverage of the mask's shapes on a `w`×`h` grid (`s` texture px per canvas px).
fn shapes(mask: &ItemMask, w: usize, h: usize, s: f32) -> Vec<f32> {
    let mut acc = vec![coverage_start(&mask.shapes); w * h];
    let mut scratch = tiny_skia::Mask::new(w as u32, h as u32);
    for shape in &mask.shapes {
        let mut cov = match scratch.as_mut() {
            Some(m) => {
                m.clear();
                fill(m, &shape.geometry, w, h, s);
                m.data().iter().map(|v| f32::from(*v) / 255.0).collect()
            }
            None => vec![0.0; w * h],
        };
        blur(&mut cov, w, h, width_radius(shape.feather * s), 0.0);
        combine(&mut acc, &cov, shape.mode);
    }
    if mask.invert {
        invert(&mut acc);
    }
    acc
}

/// Rasterizes one shape (anti-aliased) into `m`, with item units scaled to `w`×`h`.
fn fill(m: &mut tiny_skia::Mask, geometry: &MaskGeometry, w: usize, h: usize, s: f32) {
    let scale = Vec2::new(w as f32, h as f32);
    let mut pb = PathBuilder::new();
    match geometry {
        MaskGeometry::Rect { min, max, radius } => {
            let (lo, hi) = (min.min(*max) * scale, min.max(*max) * scale);
            let r = (radius * s).min(0.5 * (hi - lo).min_element()).max(0.0);
            rounded_rect(&mut pb, lo, hi, r);
        }
        MaskGeometry::Ellipse { center, radii } => {
            let (c, r) = (*center * scale, radii.abs() * scale);
            if let Some(rect) =
                tiny_skia::Rect::from_ltrb(c.x - r.x, c.y - r.y, c.x + r.x, c.y + r.y)
            {
                pb.push_oval(rect);
            }
        }
        MaskGeometry::Polygon { points } => {
            for (i, p) in points.iter().enumerate() {
                let p = *p * scale;
                if i == 0 {
                    pb.move_to(p.x, p.y);
                } else {
                    pb.line_to(p.x, p.y);
                }
            }
            pb.close();
        }
    }
    if let Some(path) = pb.finish() {
        let b = path.bounds();
        if b.width() > 1e-3 && b.height() > 1e-3 {
            m.fill_path(&path, FillRule::Winding, true, Transform::identity());
        }
    }
}

fn rounded_rect(pb: &mut PathBuilder, lo: Vec2, hi: Vec2, r: f32) {
    if r <= 0.0 {
        if let Some(rect) = tiny_skia::Rect::from_ltrb(lo.x, lo.y, hi.x, hi.y) {
            pb.push_rect(rect);
        }
        return;
    }
    // Cubic quarter circles.
    let k = r * (1.0 - 0.552_284_8);
    pb.move_to(lo.x + r, lo.y);
    pb.line_to(hi.x - r, lo.y);
    pb.cubic_to(hi.x - k, lo.y, hi.x, lo.y + k, hi.x, lo.y + r);
    pb.line_to(hi.x, hi.y - r);
    pb.cubic_to(hi.x, hi.y - k, hi.x - k, hi.y, hi.x - r, hi.y);
    pb.line_to(lo.x + r, hi.y);
    pb.cubic_to(lo.x + k, hi.y, lo.x, hi.y - k, lo.x, hi.y - r);
    pb.line_to(lo.x, lo.y + r);
    pb.cubic_to(lo.x, lo.y + k, lo.x + k, lo.y, lo.x + r, lo.y);
    pb.close();
}

/// Box radius for three passes whose combined ramp is `width` pixels wide (a feather).
fn width_radius(width: f32) -> usize {
    (width / 6.0).round().max(0.0) as usize
}

/// Box radius for a blur that reaches `reach` pixels past an edge (a glow or shadow).
fn reach_radius(reach: f32) -> usize {
    // Three passes of radius r reach 3r pixels.
    (reach / 3.0).floor().max(0.0) as usize
}

/// Three box blur passes each way (close to a Gaussian). Pixels outside the grid are `edge`.
fn blur(data: &mut [f32], w: usize, h: usize, radius: usize, edge: f32) {
    if radius == 0 {
        return;
    }
    let mut line = Vec::with_capacity(w.max(h) + 2 * radius);
    let mut out = vec![0.0; w.max(h)];
    for _ in 0..3 {
        for y in 0..h {
            let row = &mut data[y * w..][..w];
            box_line(row.iter().copied(), w, radius, edge, &mut line, &mut out);
            row.copy_from_slice(&out[..w]);
        }
        for x in 0..w {
            box_line(
                (0..h).map(|y| data[y * w + x]),
                h,
                radius,
                edge,
                &mut line,
                &mut out,
            );
            for (y, v) in out[..h].iter().enumerate() {
                data[y * w + x] = *v;
            }
        }
    }
}

fn box_line(
    values: impl Iterator<Item = f32>,
    n: usize,
    r: usize,
    edge: f32,
    line: &mut Vec<f32>,
    out: &mut [f32],
) {
    line.clear();
    line.extend(std::iter::repeat_n(edge, r));
    line.extend(values);
    line.extend(std::iter::repeat_n(edge, r));
    let span = (2 * r + 1) as f32;
    let mut sum: f32 = line[..2 * r + 1].iter().sum();
    for (i, o) in out[..n].iter_mut().enumerate() {
        *o = sum / span;
        if i + 1 < n {
            sum += line[i + 2 * r + 1] - line[i];
        }
    }
}

/// `data` moved by `by` whole pixels; uncovered pixels are empty.
fn shift(data: &[f32], w: usize, h: usize, by: Vec2) -> Vec<f32> {
    let (dx, dy) = (by.x as isize, by.y as isize);
    let mut out = vec![0.0; w * h];
    for y in 0..h as isize {
        let sy = y - dy;
        if !(0..h as isize).contains(&sy) {
            continue;
        }
        for x in 0..w as isize {
            let sx = x - dx;
            if (0..w as isize).contains(&sx) {
                out[(y * w as isize + x) as usize] = data[(sy * w as isize + sx) as usize];
            }
        }
    }
    out
}

/// Distance from the shape's edge for each pixel: positive outside, negative inside.
fn signed_distance(c: &[f32], w: usize, h: usize) -> Vec<f32> {
    let inside: Vec<bool> = c.iter().map(|v| *v >= 0.5).collect();
    let to_inside = distance(&inside, w, h, true);
    let to_outside = distance(&inside, w, h, false);
    c.iter()
        .enumerate()
        .map(|(i, v)| {
            if *v > 0.0 && *v < 1.0 {
                0.5 - v
            } else if inside[i] {
                0.5 - to_outside[i]
            } else {
                to_inside[i] - 0.5
            }
        })
        .collect()
}

/// Euclidean distance to the nearest pixel whose `inside` equals `target` (Felzenszwalb).
fn distance(inside: &[bool], w: usize, h: usize, target: bool) -> Vec<f32> {
    const FAR: f32 = 1e20;
    let mut d: Vec<f32> = inside
        .iter()
        .map(|i| if *i == target { 0.0 } else { FAR })
        .collect();
    let n = w.max(h);
    let (mut f, mut out) = (vec![0.0; n], vec![0.0; n]);
    let (mut v, mut z) = (vec![0usize; n], vec![0.0f32; n + 1]);
    for x in 0..w {
        for y in 0..h {
            f[y] = d[y * w + x];
        }
        edt_1d(&f[..h], &mut out[..h], &mut v, &mut z);
        for y in 0..h {
            d[y * w + x] = out[y];
        }
    }
    for y in 0..h {
        f[..w].copy_from_slice(&d[y * w..][..w]);
        edt_1d(&f[..w], &mut out[..w], &mut v, &mut z);
        d[y * w..][..w].copy_from_slice(&out[..w]);
    }
    d.iter_mut().for_each(|v| *v = v.sqrt());
    d
}

/// Squared distance transform of one line (lower envelope of parabolas).
fn edt_1d(f: &[f32], out: &mut [f32], v: &mut [usize], z: &mut [f32]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    let mut k = 0;
    v[0] = 0;
    z[0] = f32::NEG_INFINITY;
    z[1] = f32::INFINITY;
    for q in 1..n {
        let intersect = |p: usize| {
            let (pf, qf) = (p as f32, q as f32);
            ((f[q] + qf * qf) - (f[p] + pf * pf)) / (2.0 * (qf - pf))
        };
        let mut s = intersect(v[k]);
        // z[0] is -inf, so this stops at the first parabola.
        while s <= z[k] {
            k -= 1;
            s = intersect(v[k]);
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f32::INFINITY;
    }
    k = 0;
    for (q, o) in out.iter_mut().enumerate() {
        let qf = q as f32;
        while z[k + 1] < qf {
            k += 1;
        }
        let p = v[k] as f32;
        *o = (qf - p) * (qf - p) + f[v[k]];
    }
}

/// Stacks colored alpha planes (first at the bottom) into straight-alpha RGBA8.
fn composite(paints: &[(Rgba8, &[f32])], n: usize) -> Vec<u8> {
    let mut out = vec![0u8; n * 4];
    for (i, px) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (mut rgb, mut a) = ([0.0f32; 3], 0.0f32);
        for (color, alpha) in paints {
            let pa = alpha[i].clamp(0.0, 1.0) * f32::from(color.a) / 255.0;
            if pa <= 0.0 {
                continue;
            }
            let c = [color.r, color.g, color.b].map(|v| f32::from(v) / 255.0);
            for k in 0..3 {
                rgb[k] = c[k] * pa + rgb[k] * (1.0 - pa);
            }
            a = pa + a * (1.0 - pa);
        }
        if a > 0.0 {
            for k in 0..3 {
                px[k] = to_byte(rgb[k] / a);
            }
            px[3] = to_byte(a);
        }
    }
    out
}

fn refs(list: &[(Rgba8, Vec<f32>)]) -> Vec<(Rgba8, &[f32])> {
    list.iter().map(|(c, a)| (*c, a.as_slice())).collect()
}

fn to_byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use estream_project::{Border, Glow, MaskEffects, MaskShape, Shadow};

    fn at(img: &Image, x: usize, y: usize) -> u8 {
        img.pixels[y * img.width as usize + x]
    }

    fn rgba(layer: &Layer, x: usize, y: usize) -> [u8; 4] {
        let i = (y * layer.image.width as usize + x) * 4;
        layer.image.pixels[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn circle_keeps_the_middle_and_clears_the_corners() {
        let m = bake(&ItemMask::circle(1.0), Vec2::splat(100.0))
            .mask
            .unwrap();
        assert_eq!((m.width, m.height), (100, 100));
        assert_eq!(at(&m, 50, 50), 255);
        assert_eq!(at(&m, 2, 2), 0);
        assert_eq!(at(&m, 97, 97), 0);
        // The edge is anti-aliased: something between at the rim.
        let rim: Vec<u8> = (0..100).map(|i| at(&m, i, i)).collect();
        assert!(rim.iter().any(|v| *v > 0 && *v < 255), "{rim:?}");
    }

    #[test]
    fn effects_only_bake_no_mask_and_big_items_shrink() {
        let mask = ItemMask {
            effects: MaskEffects::thin_border(),
            ..ItemMask::default()
        };
        let b = bake(&mask, Vec2::new(40.0, 20.0));
        assert!(b.mask.is_none() && b.under.is_none());
        let over = b.over.unwrap();
        assert_eq!((over.image.width, over.image.height), (48, 28));
        assert_eq!(over.pad, Vec2::new(4.0 / 40.0, 4.0 / 20.0));
        // Border just outside the item, clear inside it and beyond the border.
        assert_eq!(rgba(&over, 2, 14), [255, 255, 255, 255]);
        assert_eq!(rgba(&over, 0, 14)[3], 0);
        assert_eq!(rgba(&over, 24, 14)[3], 0);

        let big = bake(&ItemMask::circle(1.0), Vec2::splat(5000.0))
            .mask
            .unwrap();
        assert_eq!(big.width, MAX_SIDE as u32);
    }

    #[test]
    fn feather_ramps_over_its_width() {
        let mut mask = ItemMask::rounded(0.0);
        mask.shapes[0].geometry = MaskGeometry::Rect {
            min: Vec2::new(0.25, 0.0),
            max: Vec2::new(0.75, 1.0),
            radius: 0.0,
        };
        mask.shapes[0].feather = 30.0;
        let m = bake(&mask, Vec2::new(200.0, 200.0)).mask.unwrap();
        let row: Vec<u8> = (0..200).map(|x| at(&m, x, 100)).collect();
        // Edge at x = 50: half there, near opaque 15 px inside, near clear 15 px outside.
        assert!((100..=155).contains(&row[50]), "{}", row[50]);
        assert!(row[68] > 240 && row[32] < 15, "{} {}", row[68], row[32]);
        assert_eq!(row[100], 255);
        assert_eq!(row[5], 0);
    }

    #[test]
    fn cut_and_invert() {
        let mut mask = ItemMask::rounded(0.0);
        mask.shapes.push(MaskShape::cut(MaskGeometry::Ellipse {
            center: Vec2::splat(0.5),
            radii: Vec2::splat(0.25),
        }));
        let m = bake(&mask, Vec2::splat(64.0)).mask.unwrap();
        assert_eq!((at(&m, 32, 32), at(&m, 4, 4)), (0, 255));
        mask.invert = true;
        let m = bake(&mask, Vec2::splat(64.0)).mask.unwrap();
        assert_eq!((at(&m, 32, 32), at(&m, 4, 4)), (255, 0));
    }

    #[test]
    fn glow_and_shadow_reach_outside_only() {
        let red = Rgba8::new(255, 0, 0, 255);
        let mut mask = ItemMask::circle(1.0);
        mask.effects.glow = Some(Glow {
            color: red,
            size: 20.0,
            strength: 1.0,
            placement: GlowPlacement::Outside,
        });
        let b = bake(&mask, Vec2::splat(100.0));
        let under = b.under.unwrap();
        assert_eq!(under.image.width, 142);
        // Center of the item: none (the item covers it). Just outside the circle: strong red.
        assert_eq!(rgba(&under, 71, 71)[3], 0);
        let near = rgba(&under, 71 + 52, 71);
        assert!(near[0] == 255 && near[3] > 150, "{near:?}");
        // Fades with distance, gone at the reach.
        let far = rgba(&under, 71 + 66, 71);
        assert!(far[3] < near[3] / 4, "{far:?}");
        assert_eq!(rgba(&under, 0, 71)[3], 0);

        // Shadow moves with its offset.
        mask.effects = MaskEffects {
            shadow: Some(Shadow {
                color: Rgba8::BLACK,
                offset: Vec2::new(0.0, 10.0),
                softness: 0.0,
                opacity: 1.0,
            }),
            ..MaskEffects::default()
        };
        let under = bake(&mask, Vec2::splat(100.0)).under.unwrap();
        // Circle center (61, 61), radius 50: rows 11..111; the shadow covers 21..121.
        let (top, bottom) = (rgba(&under, 61, 8), rgba(&under, 61, 116));
        assert_eq!(top[3], 0, "nothing above");
        assert_eq!(bottom[3], 255, "below the circle");
    }

    #[test]
    fn border_placements() {
        let measure = |placement| {
            let mut mask = ItemMask::rounded(0.0);
            mask.effects.border = Some(Border {
                color: Rgba8::WHITE,
                width: 4.0,
                placement,
            });
            let b = bake(&mask, Vec2::splat(40.0));
            let over = b.over.unwrap();
            let p = (over.pad.x * 40.0).round() as usize;
            // Opaque pixels along the middle row, relative to the item's left edge.
            (0..over.image.width as usize)
                .filter(|x| rgba(&over, *x, over.image.height as usize / 2)[3] > 127)
                .map(|x| x as isize - p as isize)
                .filter(|x| *x < 20)
                .collect::<Vec<_>>()
        };
        assert_eq!(measure(BorderPlacement::Outside), [-4, -3, -2, -1]);
        assert_eq!(measure(BorderPlacement::Inside), [0, 1, 2, 3]);
        assert_eq!(measure(BorderPlacement::Center), [-2, -1, 0, 1]);
    }

    #[test]
    fn distance_transform_matches_brute_force() {
        let (w, h) = (9, 7);
        let inside: Vec<bool> = (0..w * h).map(|i| i == 3 * w + 4 || i == 5).collect();
        let d = distance(&inside, w, h, true);
        for y in 0..h {
            for x in 0..w {
                let want = [(4.0f32, 3.0f32), (5.0, 0.0)]
                    .iter()
                    .map(|(px, py)| ((x as f32 - px).powi(2) + (y as f32 - py).powi(2)).sqrt())
                    .fold(f32::MAX, f32::min);
                assert!(
                    (d[y * w + x] - want).abs() < 1e-4,
                    "({x},{y}) {} vs {want}",
                    d[y * w + x]
                );
            }
        }
    }
}
