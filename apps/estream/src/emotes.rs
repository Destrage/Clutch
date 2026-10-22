//! Emote images for the chat panel, and laying out chat lines that mix text and emotes.
//!
//! - **Images:** each address is downloaded once into a disk cache (capped at 200 MB), on four
//!   worker threads, decoded (animated GIF, WebP and PNG frame by frame) and scaled to display
//!   size. The UI thread turns finished ones into Slint images.
//! - **Layout:** Slint can't put images inside text, so a line with emotes is laid out here: words
//!   are measured with `cosmic-text` (the same shaping, with font fallback for emoji and other
//!   scripts) and broken into rows at the chat panel's width.

use slint::{Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};
use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use crate::livechat::emotes::{Emote, Part};

/// Emotes are drawn this tall (logical pixels), and stored at twice that for sharp scaling.
pub const EMOTE_HEIGHT: f32 = 26.0;
const STORED_HEIGHT: u32 = 52;
/// Animations longer than this keep their first frames only.
const MAX_FRAMES: usize = 240;
const MAX_BYTES: u64 = 8 * 1024 * 1024;
const CACHE_CAP: u64 = 200 * 1024 * 1024;
const WORKERS: usize = 4;

/// A decoded emote, ready to become Slint images on the UI thread.
struct Decoded {
    frames: Vec<SharedPixelBuffer<Rgba8Pixel>>,
    /// Average time per frame (0 for a still image).
    frame_ms: u32,
}

/// An emote's images on the UI thread.
#[derive(Clone)]
pub struct Loaded {
    pub frames: ModelRc<Image>,
    pub frame_ms: i32,
    /// Width over height.
    pub aspect: f32,
}

enum State {
    Loading,
    Ready(Loaded),
    Failed,
}

pub struct Images {
    jobs: Sender<String>,
    done: Receiver<(String, Option<Decoded>)>,
    state: HashMap<String, State>,
}

impl Images {
    pub fn new() -> Self {
        let (jobs, rx) = channel::<String>();
        let (done_tx, done) = channel();
        let rx = Arc::new(Mutex::new(rx));
        let dir = cache_dir();
        if let Some(dir) = &dir {
            let dir = dir.clone();
            let _ = std::thread::Builder::new()
                .name("estream-emote-prune".into())
                .spawn(move || prune(&dir));
        }
        for i in 0..WORKERS {
            let (rx, tx, dir) = (rx.clone(), done_tx.clone(), dir.clone());
            let _ = std::thread::Builder::new()
                .name(format!("estream-emote-{i}"))
                .spawn(move || {
                    loop {
                        let Ok(url) = rx
                            .lock()
                            .map_err(|_| ())
                            .and_then(|r| r.recv().map_err(|_| ()))
                        else {
                            return;
                        };
                        let decoded = fetch(&url, dir.as_deref()).and_then(|b| decode(&b));
                        if decoded.is_none() {
                            tracing::debug!(url, "emote image");
                        }
                        if tx.send((url, decoded)).is_err() {
                            return;
                        }
                    }
                });
        }
        Self {
            jobs,
            done,
            state: HashMap::new(),
        }
    }

    /// The images for `url`, asking for them the first time (`None` while loading or failed).
    pub fn get(&mut self, url: &str) -> Option<&Loaded> {
        if !self.state.contains_key(url) {
            self.state.insert(url.to_owned(), State::Loading);
            let _ = self.jobs.send(url.to_owned());
        }
        match self.state.get(url) {
            Some(State::Ready(l)) => Some(l),
            _ => None,
        }
    }

    /// Takes in finished downloads. `true` when any arrived (lines showing them need laying out
    /// again).
    pub fn poll(&mut self) -> bool {
        let mut any = false;
        while let Ok((url, decoded)) = self.done.try_recv() {
            any = true;
            let state = match decoded {
                Some(d) if !d.frames.is_empty() => {
                    let (w, h) = (d.frames[0].width(), d.frames[0].height());
                    let images: Vec<Image> = d.frames.into_iter().map(Image::from_rgba8).collect();
                    State::Ready(Loaded {
                        frames: ModelRc::new(VecModel::from(images)),
                        frame_ms: d.frame_ms as i32,
                        aspect: w as f32 / h.max(1) as f32,
                    })
                }
                _ => State::Failed,
            };
            self.state.insert(url, state);
        }
        any
    }

    /// Whether any emote in use is animated (the animation clock only runs then).
    pub fn any_animated(&self, urls: &HashSet<&str>) -> bool {
        urls.iter().any(|u| {
            matches!(self.state.get(*u), Some(State::Ready(l)) if l.frame_ms > 0 && l.frames.row_count() > 1)
        })
    }

    /// Forgets images no line shows any more (keeps memory bounded as the chat scrolls on).
    pub fn retain(&mut self, urls: &HashSet<&str>) {
        self.state
            .retain(|url, s| matches!(s, State::Loading) || urls.contains(url.as_str()));
    }
}

fn cache_dir() -> Option<PathBuf> {
    let dir = dirs::cache_dir()?.join("Clutch").join("emotes");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn cache_file(dir: &std::path::Path, url: &str) -> PathBuf {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(url.as_bytes());
    let name: String = hash[..16].iter().map(|b| format!("{b:02x}")).collect();
    dir.join(name)
}

/// The image's bytes, from the disk cache or downloaded into it.
fn fetch(url: &str, dir: Option<&std::path::Path>) -> Option<Vec<u8>> {
    let file = dir.map(|d| cache_file(d, url));
    if let Some(file) = &file
        && let Ok(bytes) = std::fs::read(file)
    {
        // Recently used files survive pruning.
        if let Ok(f) = std::fs::File::options().append(true).open(file) {
            let _ = f.set_modified(std::time::SystemTime::now());
        }
        return Some(bytes);
    }
    let mut response = crate::http::agent().get(url).call().ok()?;
    if response.status().as_u16() != 200 {
        return None;
    }
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_BYTES)
        .read_to_vec()
        .ok()?;
    if let Some(file) = &file {
        let _ = std::fs::write(file, &bytes);
    }
    Some(bytes)
}

/// Keeps the cache under its cap by removing the least recently used files.
fn prune(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= CACHE_CAP {
        return;
    }
    files.sort_by_key(|f| f.0);
    for (_, len, path) in files {
        if total <= CACHE_CAP * 3 / 4 {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

/// Decodes a still or animated image and scales it to the stored height.
fn decode(bytes: &[u8]) -> Option<Decoded> {
    use image::codecs::{gif::GifDecoder, png::PngDecoder, webp::WebPDecoder};
    use image::{AnimationDecoder, ImageFormat};
    let format = image::guess_format(bytes).ok()?;
    let animated = match format {
        ImageFormat::Gif => Some(GifDecoder::new(Cursor::new(bytes)).ok()?.into_frames()),
        ImageFormat::WebP => {
            let d = WebPDecoder::new(Cursor::new(bytes)).ok()?;
            d.has_animation().then(|| d.into_frames())
        }
        ImageFormat::Png => {
            let d = PngDecoder::new(Cursor::new(bytes)).ok()?;
            if d.is_apng().unwrap_or(false) {
                Some(d.apng().ok()?.into_frames())
            } else {
                None
            }
        }
        _ => None,
    };
    if let Some(frames) = animated {
        let mut out = Vec::new();
        let mut total_ms = 0.0f64;
        for frame in frames.take(MAX_FRAMES) {
            let frame = frame.ok()?;
            let (n, d) = frame.delay().numer_denom_ms();
            // Browsers treat very short delays as 100 ms; so do we.
            let ms = if d == 0 { 100.0 } else { n as f64 / d as f64 };
            total_ms += if ms < 20.0 { 100.0 } else { ms };
            out.push(scaled(frame.into_buffer()));
        }
        if out.len() > 1 {
            let frame_ms = (total_ms / out.len() as f64).round() as u32;
            return Some(Decoded {
                frames: out,
                frame_ms,
            });
        }
        if let Some(only) = out.pop() {
            return Some(Decoded {
                frames: vec![only],
                frame_ms: 0,
            });
        }
    }
    let still = image::load_from_memory_with_format(bytes, format)
        .ok()?
        .to_rgba8();
    Some(Decoded {
        frames: vec![scaled(still)],
        frame_ms: 0,
    })
}

fn scaled(img: image::RgbaImage) -> SharedPixelBuffer<Rgba8Pixel> {
    let (w, h) = img.dimensions();
    let img = if h > STORED_HEIGHT {
        let nw = ((w as f32 * STORED_HEIGHT as f32 / h as f32).round() as u32).max(1);
        image::imageops::resize(
            &img,
            nw,
            STORED_HEIGHT,
            image::imageops::FilterType::Triangle,
        )
    } else {
        img
    };
    SharedPixelBuffer::clone_from_slice(img.as_raw(), img.width(), img.height())
}

// ---- The chat panel's emote lines ----

/// Keeps the chat list's emote lines laid out: each line's parts (next to the list's rows), the
/// images, the panel width, and the animation clock.
pub struct ChatEmotes {
    images: Images,
    measure: Option<Measure>,
    /// One entry per row of the chat list (empty for lines without emotes).
    parts: std::collections::VecDeque<Vec<Part>>,
    width: f32,
    /// Lay every emote line out again at this time (the width changed or images arrived).
    relayout_at: Option<std::time::Instant>,
    clock: std::time::Instant,
    /// Pictures asked for outside chat lines (kept when unused chat images are dropped).
    pictures: HashSet<String>,
}

/// Lines are laid out again this long after the width last changed (dragging a splitter).
const RELAYOUT_DELAY: std::time::Duration = std::time::Duration::from_millis(100);
const ANIMATION_STEP_MS: u128 = 50;

impl ChatEmotes {
    pub fn new() -> Self {
        Self {
            images: Images::new(),
            measure: None,
            parts: Default::default(),
            width: 300.0,
            relayout_at: None,
            clock: std::time::Instant::now(),
            pictures: HashSet::new(),
        }
    }

    /// The pieces and height for a new row; `parts` is empty for a line without emotes or when
    /// emotes aren't shown as images.
    pub fn push(&mut self, parts: Vec<Part>) -> (ModelRc<crate::ui::ChatPiece>, f32) {
        let laid = self.lay_out(&parts);
        self.parts.push_back(parts);
        laid
    }

    /// The chat list dropped its first row.
    pub fn pop_front(&mut self) {
        self.parts.pop_front();
    }

    pub fn set_width(&mut self, width: f32) {
        if (width - self.width).abs() >= 1.0 && width > 0.0 {
            self.width = width;
            self.relayout_at = Some(std::time::Instant::now() + RELAYOUT_DELAY);
        }
    }

    /// Takes in images and lays lines out again when needed; returns the animation clock (ms)
    /// to show, or `None` to leave it (nothing animated on screen, chat hidden or animation
    /// off).
    pub fn tick(
        &mut self,
        lines: &VecModel<crate::ui::ChatLine>,
        visible: bool,
        animate: bool,
    ) -> Option<i32> {
        self.poll();
        if self
            .relayout_at
            .is_some_and(|at| std::time::Instant::now() >= at)
        {
            self.relayout_at = None;
            for (row, parts) in self.parts.clone().iter().enumerate() {
                if parts.is_empty() {
                    continue;
                }
                let (pieces, height) = self.lay_out(parts);
                if let Some(mut line) = lines.row_data(row) {
                    line.pieces = pieces;
                    line.body_height = height;
                    lines.set_row_data(row, line);
                }
            }
            let mut keep = urls_of(&self.parts);
            keep.extend(self.pictures.iter().map(String::as_str));
            self.images.retain(&keep);
        }
        if !animate {
            return Some(0); // every emote on its first frame
        }
        if !visible {
            return None;
        }
        let urls = self.urls();
        // 20 steps a second: the panel only redraws when the value changes.
        self.images.any_animated(&urls).then(|| {
            let ms = self.clock.elapsed().as_millis() % (i32::MAX as u128);
            (ms / ANIMATION_STEP_MS * ANIMATION_STEP_MS) as i32
        })
    }

    /// Takes in finished images; `true` when any arrived. Lines with emotes are laid out again
    /// shortly after (several images often land together).
    pub fn poll(&mut self) -> bool {
        let any = self.images.poll();
        if any && self.relayout_at.is_none() {
            self.relayout_at = Some(std::time::Instant::now() + RELAYOUT_DELAY);
        }
        any
    }

    /// A picture by address (category box art), using the same downloads and cache. `None`
    /// while it loads.
    pub fn picture(&mut self, url: &str) -> Option<Image> {
        if self.pictures.len() > 200 {
            self.pictures.clear();
        }
        self.pictures.insert(url.to_owned());
        self.images.get(url).and_then(|l| l.frames.row_data(0))
    }

    fn urls(&self) -> HashSet<&str> {
        urls_of(&self.parts)
    }

    fn lay_out(&mut self, parts: &[Part]) -> (ModelRc<crate::ui::ChatPiece>, f32) {
        if parts.is_empty() {
            return (ModelRc::default(), 0.0);
        }
        let measure = self.measure.get_or_insert_with(Measure::new);
        let images = &mut self.images;
        let (pieces, height) = layout(parts, self.width, measure, |e| {
            images
                .get(&e.url)
                .map(|l| (EMOTE_HEIGHT * l.aspect.clamp(0.25, 4.0), EMOTE_HEIGHT))
        });
        let pieces: Vec<crate::ui::ChatPiece> = pieces
            .into_iter()
            .map(|p| {
                let loaded = p.emote.as_ref().and_then(|e| images.get(&e.url).cloned());
                crate::ui::ChatPiece {
                    x: p.x,
                    y: p.y,
                    w: p.w,
                    h: p.h,
                    tip: p
                        .emote
                        .as_ref()
                        .map(|e| format!("{} · {}", e.name, e.source.name()))
                        .unwrap_or_default()
                        .into(),
                    text: p.text.into(),
                    frame_ms: loaded.as_ref().map_or(0, |l| l.frame_ms),
                    frames: loaded.map(|l| l.frames).unwrap_or_default(),
                }
            })
            .collect();
        (ModelRc::new(VecModel::from(pieces)), height)
    }
}

/// The emote addresses in `lines`.
fn urls_of(lines: &std::collections::VecDeque<Vec<Part>>) -> HashSet<&str> {
    lines
        .iter()
        .flatten()
        .filter_map(|p| match p {
            Part::Emote(e) => Some(e.url.as_str()),
            Part::Text(_) => None,
        })
        .collect()
}

// ---- Laying out a line ----

/// One placed piece of a chat line (logical pixels, relative to the line's text area).
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Text, or the emote's name (shown while its image loads or if it failed).
    pub text: String,
    /// The emote's address (`None` for text).
    pub emote: Option<Emote>,
}

/// Measures text the way the chat panel draws it (13 px Figtree, with fallback fonts).
pub struct Measure {
    system: cosmic_text::FontSystem,
    widths: HashMap<String, f32>,
}

/// The chat text size (Theme.font-m) and the height of a text row.
pub const FONT_SIZE: f32 = 13.0;
pub const TEXT_ROW: f32 = 18.0;

impl Measure {
    pub fn new() -> Self {
        let mut system = cosmic_text::FontSystem::new();
        system
            .db_mut()
            .load_font_data(include_bytes!("../ui/fonts/Figtree.ttf").to_vec());
        Self {
            system,
            widths: HashMap::new(),
        }
    }

    pub fn width(&mut self, text: &str) -> f32 {
        if let Some(w) = self.widths.get(text) {
            return *w;
        }
        use cosmic_text::{Attrs, Buffer, Family, Metrics, Shaping};
        let mut buffer = Buffer::new(&mut self.system, Metrics::new(FONT_SIZE, TEXT_ROW));
        buffer.set_size(None, None);
        buffer.set_text(
            text,
            &Attrs::new().family(Family::Name("Figtree")),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut self.system, false);
        let w = buffer.layout_runs().map(|r| r.line_w).fold(0.0, f32::max);
        // Words repeat a lot in chat; keep the cache from growing without bound.
        if self.widths.len() > 20_000 {
            self.widths.clear();
        }
        self.widths.insert(text.to_owned(), w);
        w
    }
}

/// Lays out `parts` in rows at most `max_w` wide. `size_of` gives an emote's size when its image
/// is ready (`None`: it's shown by name for now). Returns the pieces and the total height.
pub fn layout(
    parts: &[Part],
    max_w: f32,
    measure: &mut Measure,
    mut size_of: impl FnMut(&Emote) -> Option<(f32, f32)>,
) -> (Vec<Piece>, f32) {
    // Slint and cosmic-text round slightly differently: keep a little room.
    let max_w = (max_w - 4.0).max(40.0);
    // Tokens: words with the spaces after them, and emotes.
    enum Token<'a> {
        Word(&'a str),
        Emote(&'a Emote, f32, f32),
    }
    let mut tokens = Vec::new();
    for part in parts {
        match part {
            Part::Text(t) => {
                let mut start = 0;
                let mut in_space = false;
                for (i, c) in t.char_indices() {
                    let space = c.is_whitespace();
                    if !space && in_space {
                        tokens.push(Token::Word(&t[start..i]));
                        start = i;
                    }
                    in_space = space;
                }
                if start < t.len() {
                    tokens.push(Token::Word(&t[start..]));
                }
            }
            Part::Emote(e) => match size_of(e) {
                Some((w, h)) => tokens.push(Token::Emote(e, w, h)),
                None => tokens.push(Token::Word(&e.name)),
            },
        }
    }

    // Rows of (piece, its width); text runs on a row merge into one piece.
    let mut rows: Vec<Vec<Piece>> = vec![Vec::new()];
    let mut x = 0.0f32;
    let push_text = |rows: &mut Vec<Vec<Piece>>, x: &mut f32, text: &str, w: f32| {
        let Some(row) = rows.last_mut() else { return };
        if let Some(last) = row.last_mut()
            && last.emote.is_none()
        {
            last.text.push_str(text);
            last.w += w;
        } else {
            row.push(Piece {
                x: *x,
                y: 0.0,
                w,
                h: TEXT_ROW,
                text: text.to_owned(),
                emote: None,
            });
        }
        *x += w;
    };
    for token in tokens {
        match token {
            Token::Word(word) => {
                let trimmed = word.trim_end();
                let w_trim = measure.width(trimmed);
                if x > 0.0 && x + w_trim > max_w {
                    rows.push(Vec::new());
                    x = 0.0;
                }
                if w_trim > max_w {
                    // A long word (a link): break it where it fills the row.
                    let mut chunk = String::new();
                    for c in word.chars() {
                        chunk.push(c);
                        if measure.width(&chunk) > max_w - x && chunk.chars().count() > 1 {
                            let last = chunk.pop().unwrap_or(' ');
                            let w = measure.width(&chunk);
                            push_text(&mut rows, &mut x, &chunk, w);
                            rows.push(Vec::new());
                            x = 0.0;
                            chunk = last.to_string();
                        }
                    }
                    let w = measure.width(&chunk);
                    push_text(&mut rows, &mut x, &chunk, w);
                } else {
                    let w = measure.width(word);
                    push_text(&mut rows, &mut x, word, w);
                }
            }
            Token::Emote(e, w, h) => {
                if x > 0.0 && x + w > max_w {
                    rows.push(Vec::new());
                    x = 0.0;
                }
                if let Some(row) = rows.last_mut() {
                    row.push(Piece {
                        x,
                        y: 0.0,
                        w,
                        h,
                        text: e.name.clone(),
                        emote: Some(e.clone()),
                    });
                }
                x += w;
            }
        }
    }

    // Stack the rows; each is as tall as its tallest piece, pieces centred in it.
    let mut out = Vec::new();
    let mut y = 0.0f32;
    for row in rows.into_iter().filter(|r| !r.is_empty()) {
        let h = row.iter().map(|p| p.h).fold(TEXT_ROW, f32::max);
        for mut p in row {
            p.y = y + (h - p.h) / 2.0;
            out.push(p);
        }
        y += h;
    }
    (out, y.max(TEXT_ROW))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::livechat::emotes::EmoteSource;

    fn emote(name: &str) -> Part {
        Part::Emote(Emote {
            name: name.into(),
            url: format!("u/{name}"),
            source: EmoteSource::Twitch,
        })
    }

    #[test]
    fn a_short_line_stays_on_one_row() {
        let mut m = Measure::new();
        let parts = vec![
            Part::Text("hi ".into()),
            emote("Kappa"),
            Part::Text(" there".into()),
        ];
        let (pieces, h) = layout(&parts, 400.0, &mut m, |_| Some((26.0, 26.0)));
        assert_eq!(pieces.len(), 3);
        assert_eq!(h, 26.0);
        assert!(pieces.iter().all(|p| p.y + p.h <= h));
        assert!(pieces[1].x > pieces[0].x && pieces[2].x >= pieces[1].x + 26.0);
        // Text sits in the middle of the taller row.
        assert_eq!(pieces[0].y, (26.0 - TEXT_ROW) / 2.0);
    }

    #[test]
    fn lines_wrap_at_the_width() {
        let mut m = Measure::new();
        let parts: Vec<Part> = (0..20).map(|_| emote("Kappa")).collect();
        let (pieces, h) = layout(&parts, 140.0, &mut m, |_| Some((26.0, 26.0)));
        assert!(pieces.iter().all(|p| p.x + p.w <= 140.0));
        assert!(h >= 26.0 * 4.0, "{h}");
        // Words too.
        let long = vec![Part::Text("word ".repeat(40)), emote("Kappa")];
        let (pieces, _) = layout(&long, 200.0, &mut m, |_| Some((26.0, 26.0)));
        assert!(
            pieces.iter().all(|p| p.x + p.w <= 200.0 + 0.5),
            "{pieces:?}"
        );
    }

    #[test]
    fn emotes_without_images_show_their_names() {
        let mut m = Measure::new();
        let (pieces, h) = layout(&[emote("Kappa")], 300.0, &mut m, |_| None);
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].text, "Kappa");
        assert!(pieces[0].emote.is_none());
        assert_eq!(h, TEXT_ROW);
    }

    #[test]
    fn long_links_break_inside() {
        let mut m = Measure::new();
        let url = format!("https://example.com/{}", "a".repeat(200));
        let (pieces, _) = layout(
            &[Part::Text(url.clone()), emote("x")],
            200.0,
            &mut m,
            |_| Some((26.0, 26.0)),
        );
        let text: String = pieces
            .iter()
            .filter(|p| p.emote.is_none())
            .map(|p| p.text.as_str())
            .collect();
        assert_eq!(text, url);
        assert!(pieces.iter().all(|p| p.x + p.w <= 200.0 + 0.5));
    }

    #[test]
    fn still_and_animated_images_decode() {
        // A 2-frame GIF and a PNG, made here.
        let mut gif = Vec::new();
        {
            use image::codecs::gif::GifEncoder;
            use image::{Delay, Frame, RgbaImage};
            let mut enc = GifEncoder::new(&mut gif);
            for c in [255u8, 0] {
                let img = RgbaImage::from_pixel(8, 8, image::Rgba([c, 0, 0, 255]));
                enc.encode_frame(Frame::from_parts(
                    img,
                    0,
                    0,
                    Delay::from_numer_denom_ms(50, 1),
                ))
                .unwrap();
            }
        }
        let d = decode(&gif).unwrap();
        assert_eq!((d.frames.len(), d.frame_ms), (2, 50));
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(112, 112, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let d = decode(&png).unwrap();
        assert_eq!(d.frames.len(), 1);
        assert_eq!(d.frames[0].height(), STORED_HEIGHT);
        assert!(decode(b"not an image").is_none());
    }
}
