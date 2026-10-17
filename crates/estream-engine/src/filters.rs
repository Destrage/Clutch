//! Video filter chains of sources (video thread side). LUT and mask files are read on helper
//! threads into a cache; until a file arrives its filter is skipped, so rendering never waits.

use crossbeam_channel::{Receiver, bounded};
use estream_core::SourceId;
use estream_gpu::{
    Compositor, CubeLut, FilterChain, FilterOp, FilterOutput, FilterRenderer, GpuContext, wgpu,
};
use estream_project::{FilterDef, SourceDef};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

enum Loaded {
    Lut(CubeLut),
    Mask(u32, u32, Vec<u8>),
}

struct SourceFilters {
    /// The enabled filters this chain was built from.
    defs: Vec<FilterDef>,
    chain: FilterChain,
    /// File each chain step needs (LUT / mask), if any.
    files: Vec<Option<String>>,
}

pub(crate) struct Filters {
    renderer: FilterRenderer,
    sources: HashMap<SourceId, SourceFilters>,
    cache: HashMap<String, Arc<Loaded>>,
    loading: HashMap<String, Receiver<Result<Loaded, String>>>,
    failed: HashSet<String>,
}

fn op(def: &FilterDef) -> Option<FilterOp> {
    let value = |key: &str| {
        def.settings
            .get(key)
            .and_then(|v| v.as_f64())
            .map(|v| v as f32)
    };
    let code = def.settings.get("code").and_then(|c| c.as_str());
    FilterOp::from_params(&def.kind, value, code)
}

/// `kind:path` for filters that need a file (the cache key also says how to decode it).
fn file_of(def: &FilterDef) -> Option<String> {
    estream_gpu::filter::kind(&def.kind)?.file?;
    def.settings
        .get("file")
        .and_then(|f| f.as_str())
        .filter(|f| !f.is_empty())
        .map(|path| format!("{}:{path}", def.kind))
}

/// Reads and decodes a LUT or mask on a helper thread.
fn load(key: &str) -> Receiver<Result<Loaded, String>> {
    let (tx, rx) = bounded(1);
    let (kind, path) = key.split_once(':').unwrap_or(("", key));
    let (is_lut, path) = (kind == "lut", path.to_owned());
    let spawned = std::thread::Builder::new()
        .name("estream-filter-file".into())
        .spawn(move || {
            let result = if is_lut {
                std::fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|text| estream_gpu::parse_cube(&text))
                    .map(Loaded::Lut)
            } else {
                image::open(&path).map_err(|e| e.to_string()).map(|img| {
                    let rgba = img.to_rgba8();
                    Loaded::Mask(rgba.width(), rgba.height(), rgba.into_raw())
                })
            };
            if let Err(e) = &result {
                tracing::warn!(%path, error = %e, "filter file could not be loaded");
            }
            let _ = tx.send(result);
        });
    if let Err(e) = spawned {
        tracing::error!(error = %e, "couldn't start filter file loader");
    }
    rx
}

impl Filters {
    pub fn new(gpu: &GpuContext, compositor: Arc<Compositor>) -> Self {
        Self {
            renderer: FilterRenderer::new(gpu, compositor),
            sources: HashMap::new(),
            cache: HashMap::new(),
            loading: HashMap::new(),
            failed: HashSet::new(),
        }
    }

    /// Rebuilds chains whose filters changed; drops chains of sources without filters.
    pub fn sync(&mut self, gpu: &GpuContext, defs: &[SourceDef]) {
        self.sources.retain(|id, _| {
            defs.iter()
                .any(|d| d.id == *id && d.filters.iter().any(|f| f.enabled))
        });
        for def in defs {
            let enabled: Vec<FilterDef> = def
                .filters
                .iter()
                .filter(|f| f.enabled && op(f).is_some())
                .cloned()
                .collect();
            if enabled.is_empty() {
                self.sources.remove(&def.id);
                continue;
            }
            let entry = self.sources.entry(def.id).or_insert_with(|| SourceFilters {
                defs: Vec::new(),
                chain: FilterChain::new(),
                files: Vec::new(),
            });
            if entry.defs == enabled {
                continue;
            }
            let ops = enabled.iter().filter_map(op).collect();
            entry.chain.set_ops(&mut self.renderer, gpu, ops);
            entry.files = enabled.iter().map(file_of).collect();
            entry.defs = enabled;
        }
        // A changed file is read again (the user may have edited it).
        let wanted: HashSet<&String> = self
            .sources
            .values()
            .flat_map(|s| s.files.iter().flatten())
            .collect();
        self.cache.retain(|k, _| wanted.contains(k));
        self.failed.retain(|k| wanted.contains(k));
    }

    /// Collects finished loads and gives every step the file it needs.
    fn apply_files(&mut self, gpu: &GpuContext) {
        let done: Vec<String> = self
            .loading
            .iter()
            .filter_map(|(k, rx)| match rx.try_recv() {
                Ok(Ok(loaded)) => {
                    self.cache.insert(k.clone(), Arc::new(loaded));
                    Some(k.clone())
                }
                Ok(Err(_)) | Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.failed.insert(k.clone());
                    Some(k.clone())
                }
                Err(crossbeam_channel::TryRecvError::Empty) => None,
            })
            .collect();
        for k in done {
            self.loading.remove(&k);
        }
        for entry in self.sources.values_mut() {
            for (i, file) in entry.files.iter().enumerate() {
                let Some(key) = file else { continue };
                if entry.chain.aux_tag(i) == Some(key.as_str()) {
                    continue;
                }
                match self.cache.get(key).map(|l| &**l) {
                    Some(Loaded::Lut(lut)) => {
                        entry.chain.set_lut(&self.renderer, gpu, i, Some(lut))
                    }
                    Some(Loaded::Mask(w, h, rgba)) => {
                        entry
                            .chain
                            .set_mask(&self.renderer, gpu, i, Some((*w, *h, rgba)))
                    }
                    None => {
                        if !self.loading.contains_key(key) && !self.failed.contains(key) {
                            self.loading.insert(key.clone(), load(key));
                        }
                        continue;
                    }
                }
                entry.chain.set_aux_tag(i, Some(key.clone()));
            }
        }
    }

    /// Records every chain for this frame. `frame` gives a source's current frame.
    pub fn run<'a>(
        &mut self,
        gpu: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        time_s: f32,
        frame: impl Fn(SourceId) -> Option<FilterOutput<'a>>,
    ) {
        if self.sources.is_empty() {
            return;
        }
        self.apply_files(gpu);
        for (id, entry) in &mut self.sources {
            if let Some(input) = frame(*id) {
                entry.chain.run(&self.renderer, gpu, encoder, input, time_s);
            }
        }
    }

    /// The filtered frame of a source, if it has an active chain that produced one.
    pub fn output(&self, id: SourceId) -> Option<FilterOutput<'_>> {
        self.sources.get(&id)?.chain.output()
    }
}
