//! The eStream document: project model (canvases, scenes, items, sources) and
//! its persistence (versioned JSON scene collections, migrations, recovery files).

pub mod model;
pub mod persist;

pub use model::{Canvas, ItemContent, Project, Scene, SceneItem, SourceDef};
pub use persist::{CURRENT_SCHEMA, LoadWarning, Loaded, PersistError};
