//! Edits to a montage, each returning its own inverse, and the undo/redo history.
//!
//! Like the studio's commands, every change is a serializable [`MontageCommand`]. Applying
//! one yields the command that reverts it, so undo and redo are just more commands. Rapid
//! repeats of the same edit (dragging a trim handle) coalesce into one undo step.

use crate::model::{
    Format, MAX_SPEED, MIN_SEGMENT, MIN_SPEED, Mix, Montage, Music, Segment, Title, Transition,
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MontageCommand {
    /// Inserts a segment at `index` with the transition before it (ignored at index 0).
    InsertSegment {
        index: usize,
        segment: Segment,
        #[serde(default)]
        transition: Transition,
    },
    RemoveSegment {
        index: usize,
    },
    /// Moves a segment (with the transition before it) to another position.
    MoveSegment {
        from: usize,
        to: usize,
    },
    /// Replaces a segment (trim, speed, crop, gain).
    SetSegment {
        index: usize,
        segment: Segment,
    },
    SetTransition {
        index: usize,
        transition: Transition,
    },
    AddTitle {
        title: Title,
    },
    RemoveTitle {
        id: u64,
    },
    SetTitle {
        title: Title,
    },
    SetMusic {
        music: Option<Music>,
    },
    SetMix {
        mix: Mix,
    },
    Rename {
        name: String,
    },
    /// Output size and frame rate (crops are kept; the editor recomputes them if asked).
    SetFormat {
        format: Format,
    },
    SetFadeOut {
        secs: f32,
    },
    /// Several edits as one undo step (applied in order, undone in reverse).
    Batch {
        commands: Vec<MontageCommand>,
    },
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error("there's no segment {0}")]
    NoSegment(usize),
    #[error("there's no title {0}")]
    NoTitle(u64),
    #[error("{0}")]
    Invalid(String),
}

/// What makes two edits "the same gesture" for coalescing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoalesceKey {
    Segment(u64),
    Transition(usize),
    Title(u64),
    Music,
    Mix,
}

impl MontageCommand {
    pub fn coalesce_key(&self, montage: &Montage) -> Option<CoalesceKey> {
        match self {
            MontageCommand::SetSegment { index, .. } => montage
                .segments
                .get(*index)
                .map(|s| CoalesceKey::Segment(s.id)),
            MontageCommand::SetTransition { index, .. } => Some(CoalesceKey::Transition(*index)),
            MontageCommand::SetTitle { title } => Some(CoalesceKey::Title(title.id)),
            MontageCommand::SetMusic { .. } => Some(CoalesceKey::Music),
            MontageCommand::SetMix { .. } => Some(CoalesceKey::Mix),
            _ => None,
        }
    }

    /// Applies the edit and returns its inverse. On error the montage is unchanged.
    pub fn apply(self, m: &mut Montage) -> Result<MontageCommand, EditError> {
        use MontageCommand::*;
        m.normalize();
        let inverse = match self {
            InsertSegment {
                index,
                segment,
                transition,
            } => {
                check_segment(&segment)?;
                let index = index.min(m.segments.len());
                m.segments.insert(index, segment);
                if m.segments.len() > 1 {
                    // The transition goes before the new segment, or after it when it's first.
                    m.transitions
                        .insert(index.saturating_sub(1).min(m.transitions.len()), transition);
                }
                RemoveSegment { index }
            }
            RemoveSegment { index } => {
                if index >= m.segments.len() {
                    return Err(EditError::NoSegment(index));
                }
                let segment = m.segments.remove(index);
                let transition = if m.transitions.is_empty() {
                    Transition::Cut
                } else {
                    m.transitions
                        .remove(index.saturating_sub(1).min(m.transitions.len() - 1))
                };
                InsertSegment {
                    index,
                    segment,
                    transition,
                }
            }
            MoveSegment { from, to } => {
                let n = m.segments.len();
                if from >= n || to >= n {
                    return Err(EditError::NoSegment(from.max(to)));
                }
                let segment = m.segments.remove(from);
                m.segments.insert(to, segment);
                // Transitions belong to the gaps, not the segments: they stay in place.
                MoveSegment { from: to, to: from }
            }
            SetSegment { index, segment } => {
                check_segment(&segment)?;
                let slot = m
                    .segments
                    .get_mut(index)
                    .ok_or(EditError::NoSegment(index))?;
                let old = std::mem::replace(slot, segment);
                SetSegment {
                    index,
                    segment: old,
                }
            }
            SetTransition { index, transition } => {
                let slot = m
                    .transitions
                    .get_mut(index)
                    .ok_or(EditError::NoSegment(index + 1))?;
                let old = std::mem::replace(slot, transition);
                SetTransition {
                    index,
                    transition: old,
                }
            }
            AddTitle { title } => {
                let id = title.id;
                m.titles.push(title);
                RemoveTitle { id }
            }
            RemoveTitle { id } => {
                let i = m
                    .titles
                    .iter()
                    .position(|t| t.id == id)
                    .ok_or(EditError::NoTitle(id))?;
                AddTitle {
                    title: m.titles.remove(i),
                }
            }
            SetTitle { title } => {
                let slot = m
                    .titles
                    .iter_mut()
                    .find(|t| t.id == title.id)
                    .ok_or(EditError::NoTitle(title.id))?;
                SetTitle {
                    title: std::mem::replace(slot, title),
                }
            }
            SetMusic { music } => SetMusic {
                music: std::mem::replace(&mut m.music, music),
            },
            SetMix { mix } => SetMix {
                mix: std::mem::replace(&mut m.mix, mix),
            },
            Rename { name } => Rename {
                name: std::mem::replace(&mut m.name, name),
            },
            SetFormat { format } => {
                if format.width < 16 || format.height < 16 || format.fps == 0 {
                    return Err(EditError::Invalid("that format is too small".into()));
                }
                SetFormat {
                    format: std::mem::replace(&mut m.format, format),
                }
            }
            SetFadeOut { secs } => SetFadeOut {
                secs: std::mem::replace(&mut m.fade_out, secs.clamp(0.0, 10.0)),
            },
            Batch { commands } => {
                let before = m.clone();
                let mut inverses = Vec::with_capacity(commands.len());
                for c in commands {
                    match c.apply(m) {
                        Ok(inv) => inverses.push(inv),
                        Err(e) => {
                            *m = before; // all or nothing
                            return Err(e);
                        }
                    }
                }
                inverses.reverse();
                Batch { commands: inverses }
            }
        };
        m.normalize();
        Ok(inverse)
    }

    /// Splits segment `index` at `offset` seconds into it (timeline seconds).
    pub fn split(m: &mut Montage, index: usize, offset: f64) -> Result<MontageCommand, EditError> {
        let seg = m.segments.get(index).ok_or(EditError::NoSegment(index))?;
        if offset < MIN_SEGMENT || offset > seg.length() - MIN_SEGMENT {
            return Err(EditError::Invalid(
                "too close to the segment's edge to split".into(),
            ));
        }
        let at = seg.source_time(offset);
        let mut left = seg.clone();
        left.src_out = at;
        let mut right = seg.clone();
        right.src_in = at;
        right.id = m.new_id();
        left.events.retain(|e| *e < at);
        right.events.retain(|e| *e >= at);
        Ok(MontageCommand::Batch {
            commands: vec![
                MontageCommand::SetSegment {
                    index,
                    segment: left,
                },
                MontageCommand::InsertSegment {
                    index: index + 1,
                    segment: right,
                    transition: Transition::Cut,
                },
            ],
        })
    }
}

fn check_segment(s: &Segment) -> Result<(), EditError> {
    if !(MIN_SPEED..=MAX_SPEED).contains(&s.speed) {
        return Err(EditError::Invalid(format!(
            "speed must be between {MIN_SPEED}× and {MAX_SPEED}×"
        )));
    }
    let ordered = s.src_out.partial_cmp(&s.src_in) == Some(std::cmp::Ordering::Greater);
    if !ordered || s.length() < MIN_SEGMENT - 1e-9 {
        return Err(EditError::Invalid("the segment is too short".into()));
    }
    Ok(())
}

struct Entry {
    undo: MontageCommand,
    key: Option<CoalesceKey>,
    at: Instant,
}

/// Undo and redo for one montage.
pub struct History {
    undo: Vec<Entry>,
    redo: Vec<MontageCommand>,
    limit: usize,
    window: Duration,
    sealed: bool,
}

impl Default for History {
    fn default() -> Self {
        Self::new(500, Duration::from_millis(500))
    }
}

impl History {
    pub fn new(limit: usize, window: Duration) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            limit,
            window,
            sealed: false,
        }
    }

    /// Applies `command` and records it. Edits with the same key within the coalescing
    /// window form one undo step (undo goes back to before the whole gesture).
    pub fn apply(
        &mut self,
        montage: &mut Montage,
        command: MontageCommand,
        now: Instant,
    ) -> Result<(), EditError> {
        let key = command.coalesce_key(montage);
        let inverse = command.apply(montage)?;
        self.redo.clear();
        if !self.sealed
            && let (Some(key), Some(top)) = (&key, self.undo.last_mut())
            && top.key.as_ref() == Some(key)
            && now.saturating_duration_since(top.at) <= self.window
        {
            top.at = now;
            return Ok(());
        }
        self.sealed = false;
        self.undo.push(Entry {
            undo: inverse,
            key,
            at: now,
        });
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
        Ok(())
    }

    /// Ends the current gesture (e.g. on mouse release), so the next edit is a new step.
    pub fn seal(&mut self) {
        self.sealed = true;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self, montage: &mut Montage) -> Result<bool, EditError> {
        let Some(entry) = self.undo.pop() else {
            return Ok(false);
        };
        let redo = entry.undo.apply(montage)?;
        self.redo.push(redo);
        self.sealed = true;
        Ok(true)
    }

    pub fn redo(&mut self, montage: &mut Montage) -> Result<bool, EditError> {
        let Some(command) = self.redo.pop() else {
            return Ok(false);
        };
        let key = command.coalesce_key(montage);
        let undo = command.apply(montage)?;
        self.undo.push(Entry {
            undo,
            key,
            at: Instant::now(),
        });
        self.sealed = true;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Anchor, Format, TitleStyle};

    fn base() -> Montage {
        let mut m = Montage::new("T", Format::LANDSCAPE);
        for i in 0..3 {
            let id = m.new_id();
            m.segments
                .push(Segment::new(id, format!("C:/c/{i}.mp4"), 0.0, 4.0));
        }
        m.normalize();
        m.transitions[0] = Transition::Crossfade { secs: 0.5 };
        m
    }

    /// Applies `command` through a history, undoes and redoes, checking each state.
    fn round_trip(command: MontageCommand) {
        let mut m = base();
        let before = m.clone();
        let mut h = History::default();
        h.apply(&mut m, command, Instant::now())
            .unwrap_or_else(|e| panic!("{e}"));
        let after = m.clone();
        assert_ne!(after, before, "the edit changed something");
        assert_eq!(h.undo(&mut m), Ok(true));
        // Ids are never handed out twice, so `next_id` may stay ahead after an undo.
        let same = |a: &Montage, b: &Montage| {
            Montage {
                next_id: 0,
                ..a.clone()
            } == Montage {
                next_id: 0,
                ..b.clone()
            }
        };
        assert!(same(&m, &before), "undo restores");
        assert_eq!(h.redo(&mut m), Ok(true));
        assert!(same(&m, &after), "redo re-applies");
    }

    #[test]
    fn every_edit_round_trips() {
        let mut extra = Segment::new(99, "C:/c/x.mp4", 1.0, 3.0);
        round_trip(MontageCommand::InsertSegment {
            index: 1,
            segment: extra.clone(),
            transition: Transition::Whip { secs: 0.3 },
        });
        extra.speed = 0.5;
        round_trip(MontageCommand::InsertSegment {
            index: 0,
            segment: extra,
            transition: Transition::Cut,
        });
        round_trip(MontageCommand::RemoveSegment { index: 0 });
        round_trip(MontageCommand::RemoveSegment { index: 2 });
        round_trip(MontageCommand::MoveSegment { from: 2, to: 0 });
        let mut trimmed = base().segments[1].clone();
        trimmed.src_in = 1.0;
        trimmed.speed = 2.0;
        round_trip(MontageCommand::SetSegment {
            index: 1,
            segment: trimmed,
        });
        round_trip(MontageCommand::SetTransition {
            index: 1,
            transition: Transition::DipToBlack { secs: 0.4 },
        });
        round_trip(MontageCommand::AddTitle {
            title: Title {
                id: 50,
                text: "Highlights".into(),
                style: TitleStyle::Bold,
                start: 0.0,
                dur: 2.0,
                anchor: Anchor::Center,
            },
        });
        round_trip(MontageCommand::SetMix {
            mix: Mix {
                clip_gain_db: -3.0,
                duck_db: 12.0,
            },
        });
        round_trip(MontageCommand::Rename { name: "New".into() });
        round_trip(MontageCommand::SetFormat {
            format: Format::VERTICAL,
        });
        round_trip(MontageCommand::SetFadeOut { secs: 1.5 });
    }

    #[test]
    fn split_and_reorder_then_undo_twice() {
        let mut m = base();
        let original = m.clone();
        let mut h = History::default();
        let split = MontageCommand::split(&mut m, 1, 1.5).unwrap_or_else(|e| panic!("{e}"));
        h.apply(&mut m, split, Instant::now())
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(m.segments.len(), 4);
        assert!((m.segments[1].src_out - 1.5).abs() < 1e-9);
        assert!((m.segments[2].src_in - 1.5).abs() < 1e-9);
        assert_eq!(m.transitions.len(), 3);
        // Drag the second half to the start.
        h.apply(
            &mut m,
            MontageCommand::MoveSegment { from: 2, to: 0 },
            Instant::now(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert!((m.segments[0].src_in - 1.5).abs() < 1e-9);
        assert_eq!(h.undo(&mut m), Ok(true));
        assert_eq!(h.undo(&mut m), Ok(true));
        assert_eq!(m.segments, original.segments);
        assert_eq!(m.transitions, original.transitions);
    }

    #[test]
    fn rapid_edits_coalesce_until_sealed() {
        let mut m = base();
        let before = m.clone();
        let mut h = History::default();
        let t0 = Instant::now();
        for (k, src_in) in [0.5, 1.0, 1.5].into_iter().enumerate() {
            let mut s = m.segments[0].clone();
            s.src_in = src_in;
            h.apply(
                &mut m,
                MontageCommand::SetSegment {
                    index: 0,
                    segment: s,
                },
                t0 + Duration::from_millis(100 * k as u64),
            )
            .unwrap_or_else(|e| panic!("{e}"));
        }
        assert_eq!(h.undo(&mut m), Ok(true));
        assert_eq!(m, before, "one step for the whole drag");
        assert_eq!(h.undo(&mut m), Ok(false));
    }

    #[test]
    fn bad_edits_leave_the_montage_unchanged() {
        let mut m = base();
        let before = m.clone();
        let mut fast = m.segments[0].clone();
        fast.speed = 4.0;
        assert!(
            MontageCommand::SetSegment {
                index: 0,
                segment: fast
            }
            .apply(&mut m)
            .is_err()
        );
        let batch = MontageCommand::Batch {
            commands: vec![
                MontageCommand::RemoveSegment { index: 0 },
                MontageCommand::RemoveSegment { index: 9 },
            ],
        };
        assert!(batch.apply(&mut m).is_err());
        assert_eq!(m, before);
        assert!(
            MontageCommand::split(&mut m, 0, 0.05).is_err(),
            "too close to the edge"
        );
    }
}
