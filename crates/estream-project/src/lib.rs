//! The Clutch document: project model (canvases, scenes, items, sources) and
//! its persistence (versioned JSON scene collections, migrations, recovery files).

pub mod animation;
pub mod automation;
pub mod mask;
pub mod model;
pub mod persist;
pub mod transition;

pub use animation::{AnimationKind, Ease, ItemAnimation, ItemAnimations, MoveSettings};
pub use automation::{Automation, Chord, GameEventKind, Macro, OutputKind, Step, Toggle, Trigger};
pub use mask::{
    Border, BorderPlacement, Glow, GlowPlacement, ItemMask, MaskEffects, MaskGeometry, MaskMode,
    MaskShape, Shadow,
};
pub use model::{
    AudioConfig, AudioFilterDef, Canvas, FilterDef, ItemContent, ItemFill, Monitoring, Project,
    ProjectAudio, Scene, SceneItem, SourceDef,
};
pub use persist::{CURRENT_SCHEMA, LoadWarning, Loaded, PersistError};
pub use transition::{Direction, Transition, TransitionKind};
