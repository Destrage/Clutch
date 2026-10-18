//! Turning picked clips and highlights into a montage.
//!
//! 1. **Allocate time:** each item gets a share of the target length weighted by its score,
//!    at least [`MIN_ITEM`] and at most the item itself. If the target can't fit everything,
//!    the lowest-scoring items are dropped first.
//! 2. **Trim around the main moment:** 70 % of an item's time before its anchor, 30 % after.
//! 3. **Slow motion** (optional): the second around the anchor of strong moments (aces,
//!    multi-kills) plays at half speed.
//! 4. **Framing:** a centered crop that fills the output format.
//! 5. **Transitions:** cuts, with a quick whip between items from different games or
//!    recordings; a fade to black at the end.
//! 6. **Music:** starts so its first downbeat lands on the first cut; each later cut snaps to
//!    the nearest beat within [`SNAP`] (a downbeat for strong moments, if one is in reach).
//!
//! The same inputs always give the same montage.

use crate::beats::{Beats, nearest};
use crate::model::{Anchor, Crop, Format, Montage, Music, Segment, Title, TitleStyle, Transition};
use std::path::PathBuf;

/// The shortest share an item gets.
pub const MIN_ITEM: f64 = 2.0;
/// How far a cut may move to land on a beat.
pub const SNAP: f64 = 0.25;
/// Items scoring at least this get slow motion and prefer downbeats.
pub const STRONG: f32 = 0.8;
const WHIP: f32 = 0.3;
const SLOW_HALF: f64 = 0.5;
const FADE_OUT: f32 = 0.6;
const TITLE_SECS: f64 = 2.0;

/// A clip or highlight to use.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub source: PathBuf,
    /// The usable range in the source (seconds).
    pub start: f64,
    pub end: f64,
    /// The main moment (source seconds).
    pub anchor: f64,
    pub score: f32,
    /// Event times in the source.
    pub events: Vec<f64>,
    /// For chronological order (e.g. seconds since an epoch, or the time in a recording).
    pub when: f64,
    /// The game or recording; a whip marks the change between groups.
    pub group: String,
    /// Width ÷ height of the source picture.
    pub aspect: f64,
}

impl Item {
    pub fn length(&self) -> f64 {
        (self.end - self.start).max(0.0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Order {
    #[default]
    Chronological,
    BestFirst,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MusicInput {
    pub source: PathBuf,
    pub beats: Beats,
    pub gain_db: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub name: String,
    /// Target length in seconds.
    pub target: f64,
    pub format: Format,
    pub order: Order,
    pub music: Option<MusicInput>,
    pub slow_motion: bool,
    /// A title card over the first seconds.
    pub title: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            name: "Montage".into(),
            target: 60.0,
            format: Format::LANDSCAPE,
            order: Order::Chronological,
            music: None,
            slow_motion: true,
            title: None,
        }
    }
}

/// How long each item gets (timeline seconds, before slow motion), and which are kept.
fn allocate(items: &[Item], target: f64) -> Vec<Option<f64>> {
    let mut keep: Vec<bool> = items.iter().map(|i| i.length() >= 0.5).collect();
    // Drop the lowest scores until every kept item can have its minimum.
    let mut by_score: Vec<usize> = (0..items.len()).filter(|&i| keep[i]).collect();
    by_score.sort_by(|&a, &b| {
        items[a]
            .score
            .total_cmp(&items[b].score)
            .then(items[b].when.total_cmp(&items[a].when))
    });
    let min_of = |i: usize| MIN_ITEM.min(items[i].length());
    while keep
        .iter()
        .enumerate()
        .filter(|(_, k)| **k)
        .map(|(i, _)| min_of(i))
        .sum::<f64>()
        > target
    {
        match by_score.first() {
            Some(&lowest) => {
                keep[lowest] = false;
                by_score.remove(0);
            }
            None => break,
        }
    }
    // Water-filling: share the time by weight; items whose share falls below their minimum
    // or above their length are pinned there, and the rest is shared again among the others.
    let weight = |i: usize| 0.5 + items[i].score as f64;
    let mut share = vec![0.0; items.len()];
    let mut free: Vec<usize> = (0..items.len()).filter(|&i| keep[i]).collect();
    let mut pinned_total = 0.0;
    while !free.is_empty() {
        let left = (target - pinned_total).max(0.0);
        let weights: f64 = free.iter().map(|&i| weight(i)).sum();
        let mut pinned = Vec::new();
        for &i in &free {
            let want = left * weight(i) / weights;
            if want < min_of(i) {
                pinned.push((i, min_of(i)));
            } else if want > items[i].length() {
                pinned.push((i, items[i].length()));
            } else {
                share[i] = want;
            }
        }
        if pinned.is_empty() {
            break;
        }
        for (i, value) in pinned {
            share[i] = value;
            pinned_total += value;
            free.retain(|&f| f != i);
        }
    }
    (0..items.len())
        .map(|i| keep[i].then_some(share[i]))
        .collect()
}

/// The source window of `len` seconds around the item's anchor (70 % before), kept inside it.
fn window(item: &Item, len: f64) -> (f64, f64) {
    let len = len.min(item.length());
    let mut start = item.anchor - 0.7 * len;
    start = start.clamp(item.start, item.end - len);
    (start, start + len)
}

pub fn assemble(items: &[Item], options: &Options) -> Montage {
    let mut m = Montage::new(options.name.clone(), options.format);
    let shares = allocate(items, options.target);
    let mut chosen: Vec<(&Item, f64)> = items
        .iter()
        .zip(shares)
        .filter_map(|(item, share)| share.map(|s| (item, s)))
        .collect();
    match options.order {
        Order::Chronological => chosen.sort_by(|a, b| a.0.when.total_cmp(&b.0.when)),
        Order::BestFirst => chosen.sort_by(|a, b| {
            b.0.score
                .total_cmp(&a.0.score)
                .then(a.0.when.total_cmp(&b.0.when))
        }),
    }

    let mut groups = Vec::new();
    let mut strong = Vec::new();
    for (item, share) in &chosen {
        let crop = Crop::fill(item.aspect, options.format.aspect(), 0.0);
        let slow = options.slow_motion && item.score >= STRONG && *share >= 2.0 * SLOW_HALF + 1.0;
        let events: Vec<f64> = item
            .events
            .iter()
            .copied()
            .filter(|e| (item.start..=item.end).contains(e))
            .collect();
        let make = |m: &mut Montage, a: f64, b: f64, speed: f32| Segment {
            speed,
            crop,
            events: events
                .iter()
                .copied()
                .filter(|e| (a..=b).contains(e))
                .collect(),
            ..Segment::new(m.new_id(), item.source.clone(), a, b)
        };
        if slow {
            // The slow second takes two on the timeline: give back one second of source.
            let (a, b) = window(item, share - 2.0 * SLOW_HALF);
            let anchor = item.anchor.clamp(a + SLOW_HALF, b - SLOW_HALF);
            let pieces = [
                (a, anchor - SLOW_HALF, 1.0),
                (anchor - SLOW_HALF, anchor + SLOW_HALF, 0.5),
                (anchor + SLOW_HALF, b, 1.0),
            ];
            let mut first = true;
            for (x, y, speed) in pieces {
                if y - x < 0.2 {
                    continue;
                }
                let seg = make(&mut m, x, y, speed);
                m.segments.push(seg);
                groups.push((item.group.clone(), first));
                strong.push(item.score >= STRONG);
                first = false;
            }
        } else {
            let (a, b) = window(item, *share);
            let seg = make(&mut m, a, b, 1.0);
            m.segments.push(seg);
            groups.push((item.group.clone(), true));
            strong.push(item.score >= STRONG);
        }
    }
    m.normalize();
    // Whips where the game or recording changes (only at the start of an item).
    for i in 0..m.transitions.len() {
        let (group, starts_item) = &groups[i + 1];
        if *starts_item && *group != groups[i].0 {
            m.transitions[i] = Transition::Whip { secs: WHIP };
        }
    }
    m.fade_out = FADE_OUT;

    if let Some(music) = &options.music {
        snap_to_music(&mut m, music, &strong);
    }
    if let Some(text) = options.title.as_ref().filter(|t| !t.trim().is_empty()) {
        let id = m.new_id();
        m.titles.push(Title {
            id,
            text: text.trim().to_owned(),
            style: TitleStyle::Bold,
            start: 0.0,
            dur: TITLE_SECS.min(m.duration()),
            anchor: Anchor::Center,
        });
    }
    m
}

/// Starts the music so its first downbeat lands on the first cut, then moves each cut onto
/// the nearest beat within [`SNAP`] by changing the out point of the segment before it.
fn snap_to_music(m: &mut Montage, music: &MusicInput, strong: &[bool]) {
    let beats = &music.beats;
    let first_cut = m.layout().get(1).map(|p| p.start);
    let first_down = beats.downbeats.first().or(beats.beats.first()).copied();
    let src_in = match (first_cut, first_down) {
        (Some(cut), Some(down)) if down >= cut => down - cut,
        // The first downbeat comes too early: line up a later one.
        (Some(cut), Some(_)) => beats
            .downbeats
            .iter()
            .find(|d| **d >= cut)
            .map_or(0.0, |d| d - cut),
        _ => 0.0,
    };
    // Beat times on the timeline.
    let on_timeline = |list: &[f64]| -> Vec<f64> {
        list.iter()
            .map(|b| b - src_in)
            .filter(|t| *t >= 0.0)
            .collect()
    };
    let beat_times = on_timeline(&beats.beats);
    let down_times = on_timeline(&beats.downbeats);
    for i in 0..m.segments.len().saturating_sub(1) {
        let layout = m.layout();
        let Some(next) = layout.get(i + 1) else { break };
        let cut = next.start;
        let prefer_down = strong.get(i + 1).copied().unwrap_or(false);
        let target = prefer_down
            .then(|| nearest(&down_times, cut, SNAP))
            .flatten()
            .or_else(|| nearest(&beat_times, cut, SNAP));
        let Some(target) = target else { continue };
        let delta = target - cut;
        let seg = &mut m.segments[i];
        let speed = seg.speed as f64;
        let new_out = seg.src_out + delta * speed;
        // Only if the source has the room and the segment stays long enough.
        if new_out > seg.src_in + 0.3 && delta.abs() > 1e-6 {
            seg.src_out = new_out;
        }
    }
    m.music = Some(Music {
        source: music.source.clone(),
        source_rel: None,
        src_in,
        gain_db: music.gain_db,
        fade_in: 0.0,
        fade_out: 2.0,
        beats: beats.beats.clone(),
        missing: false,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `n` items of 30 s, an event at 20 s each, scores rising with the index.
    fn items(n: usize) -> Vec<Item> {
        (0..n)
            .map(|i| Item {
                source: PathBuf::from(format!("C:/clips/{i:02}.mp4")),
                start: 0.0,
                end: 30.0,
                anchor: 20.0,
                score: 0.3 + 0.6 * i as f32 / n.max(1) as f32,
                events: vec![20.0],
                when: i as f64,
                group: if i < n / 2 {
                    "CS2".into()
                } else {
                    "Valorant".into()
                },
                aspect: 16.0 / 9.0,
            })
            .collect()
    }

    fn beats_at(bpm: f64, secs: f64) -> Beats {
        let period = 60.0 / bpm;
        let beats: Vec<f64> = (0..)
            .map(|k| 0.2 + k as f64 * period)
            .take_while(|t| *t < secs)
            .collect();
        Beats {
            bpm,
            downbeats: beats.iter().step_by(4).copied().collect(),
            beats,
        }
    }

    #[test]
    fn fits_the_target_and_drops_the_weakest() {
        let items = items(40);
        let m = assemble(
            &items,
            &Options {
                target: 60.0,
                slow_motion: false,
                ..Options::default()
            },
        );
        assert!((m.duration() - 60.0).abs() < 1.0, "{}", m.duration());
        // 40 items × 2 s won't fit 60 s: the weakest are gone.
        let used: std::collections::HashSet<_> =
            m.segments.iter().map(|s| s.source.clone()).collect();
        assert!(used.len() <= 30);
        assert!(!used.contains(&items[0].source), "lowest score dropped");
        assert!(used.contains(&items[39].source), "best kept");
        assert!(m.segments.iter().all(|s| s.length() >= MIN_ITEM - 1e-6));
    }

    #[test]
    fn segments_center_on_the_anchor_and_stronger_items_get_more() {
        let items = items(4);
        let m = assemble(
            &items,
            &Options {
                target: 20.0,
                slow_motion: false,
                ..Options::default()
            },
        );
        assert_eq!(m.segments.len(), 4);
        for s in &m.segments {
            let before = 20.0 - s.src_in;
            let after = s.src_out - 20.0;
            assert!((before / (before + after) - 0.7).abs() < 0.01, "{s:?}");
        }
        assert!(m.segments[3].length() > m.segments[0].length());
    }

    #[test]
    fn whips_between_games_and_vertical_crops() {
        let m = assemble(
            &items(4),
            &Options {
                target: 20.0,
                format: Format::VERTICAL,
                slow_motion: false,
                ..Options::default()
            },
        );
        assert_eq!(
            m.transitions,
            [
                Transition::Cut,
                Transition::Whip { secs: WHIP },
                Transition::Cut
            ]
        );
        let c = m.segments[0].crop;
        assert!(
            c.w < 0.35 && (c.x - (1.0 - c.w) / 2.0).abs() < 1e-6,
            "{c:?}"
        );
        assert!(m.fade_out > 0.0);
    }

    #[test]
    fn strong_moments_get_slow_motion() {
        let mut items = items(2);
        items[1].score = 1.0;
        let m = assemble(
            &items,
            &Options {
                target: 16.0,
                ..Options::default()
            },
        );
        let slow: Vec<&Segment> = m.segments.iter().filter(|s| s.speed < 1.0).collect();
        assert_eq!(slow.len(), 1, "{:?}", m.segments);
        assert!((slow[0].src_in - 19.5).abs() < 1e-6 && (slow[0].src_out - 20.5).abs() < 1e-6);
        assert!((m.duration() - 16.0).abs() < 0.5, "{}", m.duration());
    }

    #[test]
    fn cuts_land_on_beats() {
        let items = items(12);
        let music = MusicInput {
            source: PathBuf::from("C:/music/song.mp3"),
            beats: beats_at(120.0, 200.0),
            gain_db: -4.0,
        };
        let m = assemble(
            &items,
            &Options {
                target: 60.0,
                music: Some(music.clone()),
                ..Options::default()
            },
        );
        let src_in = m.music.as_ref().map_or(0.0, |mu| mu.src_in);
        let timeline: Vec<f64> = music.beats.beats.iter().map(|b| b - src_in).collect();
        let layout = m.layout();
        for p in layout.iter().skip(1) {
            assert!(
                nearest(&timeline, p.start, SNAP).is_some(),
                "cut at {} isn't near a beat",
                p.start
            );
        }
        // The first cut sits exactly on a downbeat.
        let down: Vec<f64> = music.beats.downbeats.iter().map(|d| d - src_in).collect();
        assert!(nearest(&down, layout[1].start, 1e-6).is_some());
        assert!((m.duration() - 60.0).abs() < 2.5, "{}", m.duration());
    }

    #[test]
    fn same_inputs_same_montage_and_best_first_order() {
        let items = items(6);
        let options = Options {
            target: 30.0,
            title: Some("CS2 highlights".into()),
            ..Options::default()
        };
        assert_eq!(assemble(&items, &options), assemble(&items, &options));
        let best = assemble(
            &items,
            &Options {
                order: Order::BestFirst,
                slow_motion: false,
                ..options
            },
        );
        assert_eq!(best.segments[0].source, items[5].source);
        assert_eq!(best.titles.len(), 1);
        assert_eq!(best.titles[0].text, "CS2 highlights");
    }

    #[test]
    fn short_items_and_nothing() {
        let mut items = items(2);
        items[0].end = 1.0; // a one-second item gets at most one second
        let m = assemble(
            &items,
            &Options {
                target: 30.0,
                slow_motion: false,
                ..Options::default()
            },
        );
        assert!(m.segments.iter().any(|s| s.length() <= 1.0 + 1e-6));
        assert!(assemble(&[], &Options::default()).segments.is_empty());
    }
}
