//! Montages for eStream (see the `add-montage-editor` OpenSpec change): the timeline model and
//! its edits, finding highlights, beat detection, assembling a montage, rendering and export.
//!
//! Nothing here touches the live studio engine; montage work runs on its own threads.

pub mod assemble;
pub mod audio;
pub mod beats;
pub mod export;
pub mod history;
pub mod model;
pub mod preview;
pub mod render;
pub mod scan;

pub use history::{History, MontageCommand};
pub use model::{
    Anchor, Crop, Format, FrameAt, Mix, Montage, MontageError, Music, Placed, Segment, Title,
    TitleStyle, Transition,
};
