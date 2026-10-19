//! A snapshot of the PipeWire graph: the default sink and source, and the apps playing sound.

use pipewire::{
    self as pw, context::ContextRc, core::PW_ID_CORE, main_loop::MainLoopRc, metadata::Metadata,
    types::ObjectType,
};
use std::cell::RefCell;
use std::rc::Rc;

/// An app's playback stream.
#[derive(Clone, Debug, PartialEq)]
pub struct AppStream {
    /// The global id (valid while the stream exists).
    pub id: u32,
    /// The app's executable (`application.process.binary`), or its name.
    pub app: String,
    /// What the app calls itself (`application.name`).
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Graph {
    /// `node.name` of the default sink and source.
    pub default_sink: Option<String>,
    pub default_source: Option<String>,
    pub apps: Vec<AppStream>,
}

/// The app an app stream belongs to, from its node properties.
pub fn app_of(props: &pw::spa::utils::dict::DictRef) -> Option<String> {
    props
        .get("application.process.binary")
        .or_else(|| props.get("application.name"))
        .map(str::to_owned)
}

/// `{"name": "alsa_output…"}` → `alsa_output…`
fn metadata_name(value: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(value).ok()?;
    v.get("name")?.as_str().map(str::to_owned)
}

/// Reads the graph once (two round trips). `None` without a PipeWire server.
pub fn snapshot() -> Option<Graph> {
    pw::init();
    let mainloop = MainLoopRc::new(None).ok()?;
    let context = ContextRc::new(&mainloop, None).ok()?;
    let core = context.connect_rc(None).ok()?;
    let registry = core.get_registry_rc().ok()?;
    let graph = Rc::new(RefCell::new(Graph::default()));
    let bound: Rc<RefCell<Vec<Box<dyn std::any::Any>>>> = Rc::default();
    let pending = Rc::new(RefCell::new(vec![core.sync(0).ok()?]));

    let _core_listener = core
        .add_listener_local()
        .done({
            let (pending, mainloop) = (pending.clone(), mainloop.clone());
            move |id, seq| {
                let mut list = pending.borrow_mut();
                if id == PW_ID_CORE
                    && let Some(i) = list.iter().position(|s| *s == seq)
                {
                    list.remove(i);
                    if list.is_empty() {
                        mainloop.quit();
                    }
                }
            }
        })
        .register();
    let _registry_listener = registry
        .add_listener_local()
        .global({
            let (graph, bound, pending, core, registry) = (
                graph.clone(),
                bound.clone(),
                pending.clone(),
                core.clone(),
                registry.clone(),
            );
            move |global| {
                let Some(props) = global.props else { return };
                match global.type_ {
                    ObjectType::Node if props.get("media.class") == Some("Stream/Output/Audio") => {
                        if let Some(app) = app_of(props) {
                            graph.borrow_mut().apps.push(AppStream {
                                id: global.id,
                                name: props.get("application.name").unwrap_or(&app).to_owned(),
                                app,
                            });
                        }
                    }
                    ObjectType::Metadata if props.get("metadata.name") == Some("default") => {
                        let Ok(metadata) = registry.bind::<Metadata, _>(global) else {
                            return;
                        };
                        let listener = metadata
                            .add_listener_local()
                            .property({
                                let graph = graph.clone();
                                move |_, key, _, value| {
                                    let name = value.and_then(metadata_name);
                                    match key {
                                        Some("default.audio.sink") => {
                                            graph.borrow_mut().default_sink = name
                                        }
                                        Some("default.audio.source") => {
                                            graph.borrow_mut().default_source = name
                                        }
                                        _ => {}
                                    }
                                    0
                                }
                            })
                            .register();
                        // Another round trip delivers the metadata's properties.
                        if let Ok(seq) = core.sync(0) {
                            pending.borrow_mut().push(seq);
                        }
                        bound.borrow_mut().push(Box::new((metadata, listener)));
                    }
                    _ => {}
                }
            }
        })
        .register();
    mainloop.run();
    let graph = graph.borrow().clone();
    Some(graph)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_values_parse() {
        assert_eq!(
            metadata_name(r#"{ "name": "alsa_output.usb-x.analog-stereo" }"#).as_deref(),
            Some("alsa_output.usb-x.analog-stereo")
        );
        assert_eq!(metadata_name("not json"), None);
    }
}
