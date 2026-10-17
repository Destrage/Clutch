//! The eStream document: project model (canvases, scenes, items, sources) and
//! its persistence (versioned JSON scene collections, migrations, recovery files).

pub mod model;
pub mod persist;
pub mod transition;

pub use model::{
    AudioConfig, AudioFilterDef, Canvas, FilterDef, ItemContent, Monitoring, Project, ProjectAudio,
    Scene, SceneItem, SourceDef,
};
pub use persist::{CURRENT_SCHEMA, LoadWarning, Loaded, PersistError};
pub use transition::{Direction, Transition, TransitionKind};
