//! Scene collection files: versioned JSON, migrations, atomic saves and recovery files.

use crate::model::{ItemContent, Project, SceneItem};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Schema version written by this build.
pub const CURRENT_SCHEMA: u32 = 1;

/// Migrates a document from version `index + 1` to `index + 2`.
pub type Migration = fn(&mut Value) -> Result<(), String>;

/// Registered migrations; `MIGRATIONS[i]` upgrades schema `i + 1` to `i + 2`.
pub const MIGRATIONS: &[Migration] = &[];

const _: () = assert!(MIGRATIONS.len() as u32 + 1 == CURRENT_SCHEMA);

#[derive(Debug, Error)]
pub enum PersistError {
    #[error("i/o error on {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid scene collection JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("scene collection has no schema version")]
    MissingSchema,
    #[error(
        "scene collection uses schema v{found}, but this version of eStream only supports up to v{supported}; please update eStream"
    )]
    NewerSchema { found: u32, supported: u32 },
    #[error("migration from schema v{from} failed: {message}")]
    Migration { from: u32, message: String },
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> PersistError + '_ {
    move |source| PersistError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[derive(Serialize)]
struct FileRef<'a> {
    schema: u32,
    #[serde(flatten)]
    project: &'a Project,
}

/// Problems found while loading that did not prevent the load.
#[derive(Clone, Debug, PartialEq)]
pub enum LoadWarning {
    /// A scene referenced a canvas that does not exist; the scene was dropped.
    OrphanScene { scene: String },
    /// An item referenced a source that does not exist; the item was dropped.
    MissingSource { scene: String, item: String },
    /// A canvas's active scene did not exist or belonged elsewhere; it was reset.
    InvalidActiveScene { canvas: String },
}

#[derive(Debug)]
pub struct Loaded {
    pub project: Project,
    pub warnings: Vec<LoadWarning>,
    /// Schema version the file was stored with (before migration).
    pub original_schema: u32,
}

/// Serializes a project to the scene collection JSON format.
pub fn to_json(project: &Project) -> Result<String, PersistError> {
    Ok(serde_json::to_string_pretty(&FileRef {
        schema: CURRENT_SCHEMA,
        project,
    })?)
}

/// Parses, migrates and validates scene collection JSON.
pub fn from_json(text: &str) -> Result<Loaded, PersistError> {
    let mut value: Value = serde_json::from_str(text)?;
    let original_schema = value
        .get("schema")
        .and_then(Value::as_u64)
        .ok_or(PersistError::MissingSchema)? as u32;
    migrate(&mut value, original_schema, CURRENT_SCHEMA, MIGRATIONS)?;
    if let Some(obj) = value.as_object_mut() {
        obj.remove("schema");
    }
    let mut project: Project = serde_json::from_value(value)?;
    let warnings = validate_references(&mut project);
    for warning in &warnings {
        tracing::warn!(?warning, "scene collection repaired on load");
    }
    Ok(Loaded {
        project,
        warnings,
        original_schema,
    })
}

/// Runs `migrations` to upgrade `value` from schema `from` to `to`.
pub fn migrate(
    value: &mut Value,
    from: u32,
    to: u32,
    migrations: &[Migration],
) -> Result<(), PersistError> {
    if from > to {
        return Err(PersistError::NewerSchema {
            found: from,
            supported: to,
        });
    }
    if from == 0 {
        return Err(PersistError::MissingSchema);
    }
    for version in from..to {
        let step = migrations
            .get(version as usize - 1)
            .ok_or_else(|| PersistError::Migration {
                from: version,
                message: "no migration registered".into(),
            })?;
        step(value).map_err(|message| PersistError::Migration {
            from: version,
            message,
        })?;
        value["schema"] = Value::from(version + 1);
        tracing::info!(
            from = version,
            to = version + 1,
            "migrated scene collection"
        );
    }
    Ok(())
}

/// Removes dangling references so a partially broken file still loads.
pub fn validate_references(project: &mut Project) -> Vec<LoadWarning> {
    let mut warnings = Vec::new();
    let canvas_ids: Vec<_> = project.canvases.iter().map(|c| c.id).collect();
    project.scenes.retain(|scene| {
        let ok = canvas_ids.contains(&scene.canvas);
        if !ok {
            warnings.push(LoadWarning::OrphanScene {
                scene: scene.name.clone(),
            });
        }
        ok
    });

    let source_ids: Vec<_> = project.sources.iter().map(|s| s.id).collect();
    fn prune(
        items: &mut Vec<SceneItem>,
        sources: &[estream_core::SourceId],
        scene: &str,
        warnings: &mut Vec<LoadWarning>,
    ) {
        items.retain_mut(|item| match &mut item.content {
            ItemContent::Source { source } => {
                let ok = sources.contains(source);
                if !ok {
                    warnings.push(LoadWarning::MissingSource {
                        scene: scene.to_owned(),
                        item: item.id.to_string(),
                    });
                }
                ok
            }
            ItemContent::Group { items } => {
                prune(items, sources, scene, warnings);
                true
            }
        });
    }
    for scene in &mut project.scenes {
        let name = scene.name.clone();
        prune(&mut scene.items, &source_ids, &name, &mut warnings);
    }

    for canvas in &mut project.canvases {
        let valid = canvas.active_scene.is_none_or(|id| {
            project
                .scenes
                .iter()
                .any(|s| s.id == id && s.canvas == canvas.id)
        });
        if !valid {
            warnings.push(LoadWarning::InvalidActiveScene {
                canvas: canvas.name.clone(),
            });
            canvas.active_scene = project
                .scenes
                .iter()
                .find(|s| s.canvas == canvas.id)
                .map(|s| s.id);
        }
    }
    warnings
}

/// Writes `contents` to `path` atomically (temp file + fsync + rename).
pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), PersistError> {
    use std::io::Write;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(io_err(dir))?;
    }
    let mut tmp_name = path.as_os_str().to_owned();
    tmp_name.push(".tmp");
    let tmp = PathBuf::from(tmp_name);
    {
        let mut file = std::fs::File::create(&tmp).map_err(io_err(&tmp))?;
        file.write_all(contents).map_err(io_err(&tmp))?;
        file.sync_all().map_err(io_err(&tmp))?;
    }
    std::fs::rename(&tmp, path).map_err(io_err(path))
}

/// Saves a project as a scene collection (atomic).
pub fn save(path: &Path, project: &Project) -> Result<(), PersistError> {
    write_atomic(path, to_json(project)?.as_bytes())
}

/// Loads a scene collection file.
pub fn load(path: &Path) -> Result<Loaded, PersistError> {
    let text = std::fs::read_to_string(path).map_err(io_err(path))?;
    from_json(&text)
}

/// Path of the autosave/recovery file that belongs to a scene collection.
pub fn recovery_path(path: &Path) -> PathBuf {
    let mut name = path.file_stem().unwrap_or_default().to_owned();
    name.push(".recovery.json");
    path.with_file_name(name)
}

/// Writes the recovery (autosave) file for `path`.
pub fn save_recovery(path: &Path, project: &Project) -> Result<(), PersistError> {
    save(&recovery_path(path), project)
}

/// A recovery file exists only after an unclean shutdown (clean saves remove it).
pub fn find_recovery(path: &Path) -> Option<PathBuf> {
    let recovery = recovery_path(path);
    recovery.is_file().then_some(recovery)
}

/// Removes the recovery file (after a clean save, or when the user declines restoring).
pub fn discard_recovery(path: &Path) -> Result<(), PersistError> {
    let recovery = recovery_path(path);
    match std::fs::remove_file(&recovery) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(io_err(&recovery)(e)),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Scene, SourceDef};
    use estream_core::Rational;

    fn sample() -> Project {
        let mut p = Project::new_default(1920, 1080, Rational::fps(60));
        let src = SourceDef::new("Bars", "test_pattern", serde_json::json!({ "width": 640 }));
        let item = SceneItem::source(src.id);
        p.scenes[0].items.push(item.clone());
        p.scenes[0]
            .items
            .push(SceneItem::group("G", vec![SceneItem::source(src.id)]));
        p.sources.push(src);
        p
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("estream-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn round_trip() {
        let p = sample();
        let loaded = from_json(&to_json(&p).unwrap()).unwrap();
        assert_eq!(loaded.project, p);
        assert!(loaded.warnings.is_empty());
    }

    #[test]
    fn file_round_trip_and_atomic_overwrite() {
        let dir = temp_dir("save");
        let path = dir.join("main.json");
        let p = sample();
        save(&path, &p).unwrap();
        save(&path, &p).unwrap(); // overwrite existing
        assert_eq!(load(&path).unwrap().project, p);
        assert!(!dir.join("main.json.tmp").exists());
    }

    #[test]
    fn interrupted_save_keeps_previous_file() {
        let dir = temp_dir("crash");
        let path = dir.join("main.json");
        let p = sample();
        save(&path, &p).unwrap();
        // Simulate a crash mid-write: a stale, truncated temp file is left behind.
        std::fs::write(dir.join("main.json.tmp"), "{\"schema\": 1, \"canv").unwrap();
        assert_eq!(load(&path).unwrap().project, p);
    }

    #[test]
    fn newer_schema_is_refused_with_both_versions() {
        let json = format!("{{\"schema\": {}, \"canvases\": []}}", CURRENT_SCHEMA + 4);
        let err = from_json(&json).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains(&format!("v{}", CURRENT_SCHEMA + 4)), "{msg}");
        assert!(msg.contains(&format!("v{CURRENT_SCHEMA}")), "{msg}");
    }

    #[test]
    fn missing_schema_is_refused() {
        assert!(matches!(
            from_json("{\"canvases\": []}"),
            Err(PersistError::MissingSchema)
        ));
    }

    #[test]
    fn migrations_run_in_order() {
        fn v1_to_v2(v: &mut Value) -> Result<(), String> {
            v["log"] = Value::from("1>2");
            Ok(())
        }
        fn v2_to_v3(v: &mut Value) -> Result<(), String> {
            let prev = v["log"].as_str().ok_or("missing log")?.to_owned();
            v["log"] = Value::from(format!("{prev},2>3"));
            Ok(())
        }
        let mut value = serde_json::json!({ "schema": 1 });
        migrate(&mut value, 1, 3, &[v1_to_v2, v2_to_v3]).unwrap();
        assert_eq!(value["log"], "1>2,2>3");
        assert_eq!(value["schema"], 3);
    }

    #[test]
    fn broken_references_are_repaired() {
        let mut p = sample();
        p.sources.clear(); // every source item now dangles
        let orphan = Scene::new("Orphan", estream_core::CanvasId::new());
        p.canvases[0].active_scene = Some(orphan.id);
        p.scenes.push(orphan);

        let loaded = from_json(&to_json(&p).unwrap()).unwrap();
        let scene = &loaded.project.scenes[0];
        assert_eq!(loaded.project.scenes.len(), 1);
        assert_eq!(
            scene.items.len(),
            1,
            "group kept, top-level source item dropped"
        );
        assert_eq!(
            loaded.project.canvases[0].active_scene,
            Some(scene.id),
            "active scene reset to first valid scene"
        );
        assert_eq!(loaded.warnings.len(), 4);
    }

    #[test]
    fn recovery_lifecycle() {
        let dir = temp_dir("recovery");
        let path = dir.join("show.json");
        assert!(find_recovery(&path).is_none());
        save_recovery(&path, &sample()).unwrap();
        assert_eq!(find_recovery(&path), Some(dir.join("show.recovery.json")));
        discard_recovery(&path).unwrap();
        discard_recovery(&path).unwrap(); // idempotent
        assert!(find_recovery(&path).is_none());
    }
}
