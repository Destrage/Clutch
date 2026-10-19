//! Global hotkeys on Linux: the GlobalShortcuts portal (KDE, GNOME 48+, …), else an X11 key
//! grab on X11 sessions.
//!
//! The portal is one session for the whole process, shared by every [`GlobalHotkeys`] (macros
//! and the clip hotkey), so the user is asked once. Shortcut ids are the chord text, stable
//! across rebinds, so the desktop keeps the user's choices; the user may also assign other
//! keys in the desktop's settings, and presses arrive by id either way.

use super::keysym::{keysym, trigger};
use super::{GlobalHotkeys, Hotkey};
use crossbeam_channel::{Receiver, Sender};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// `CLUTCH_HOTKEYS=portal|x11|off` picks the way instead (to work around a desktop).
pub fn start() -> Option<Box<dyn GlobalHotkeys>> {
    let choice = std::env::var("CLUTCH_HOTKEYS").unwrap_or_default();
    if choice == "off" {
        return None;
    }
    if choice != "x11"
        && let Some(service) = portal::service()
    {
        return Some(Box::new(service.handle()));
    }
    if choice == "x11" || std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t == "x11") {
        return x11::Grabber::start().map(|g| Box::new(g) as Box<dyn GlobalHotkeys>);
    }
    tracing::info!("no global hotkeys on this desktop; hotkeys work while Clutch is in front");
    None
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ------------------------------------------------------------------ portal

mod portal {
    use super::*;
    use ashpd::desktop::Session;
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_util::StreamExt;

    pub(super) struct HandleState {
        pub(super) hotkeys: Vec<Hotkey>,
        pub(super) presses: Sender<u32>,
        pub(super) failed: Arc<Mutex<Vec<u32>>>,
    }

    enum Cmd {
        Set(u64, Vec<Hotkey>),
        Drop(u64),
    }

    pub struct Service {
        cmds: Sender<Cmd>,
        handles: Arc<Mutex<HashMap<u64, HandleState>>>,
        next: AtomicU64,
    }

    /// The process-wide portal session, if this desktop has the portal.
    pub fn service() -> Option<&'static Service> {
        static SERVICE: OnceLock<Option<Service>> = OnceLock::new();
        SERVICE.get_or_init(Service::start).as_ref()
    }

    impl Service {
        fn start() -> Option<Self> {
            let version: Option<u32> =
                pollster::block_on(async { Some(GlobalShortcuts::new().await.ok()?.version()) });
            let Some(version) = version.filter(|v| *v >= 1) else {
                tracing::info!("no GlobalShortcuts portal");
                return None;
            };
            tracing::info!(version, "global hotkeys through the GlobalShortcuts portal");
            let (cmds, rx) = crossbeam_channel::unbounded();
            let handles: Arc<Mutex<HashMap<u64, HandleState>>> = Arc::default();
            {
                let handles = handles.clone();
                std::thread::Builder::new()
                    .name("estream-shortcuts".into())
                    .spawn(move || bind_loop(&rx, &handles))
                    .ok()?;
            }
            {
                let handles = handles.clone();
                std::thread::Builder::new()
                    .name("estream-shortcuts-presses".into())
                    .spawn(move || pollster::block_on(activations(&handles)))
                    .ok()?;
            }
            Some(Self {
                cmds,
                handles,
                next: AtomicU64::new(1),
            })
        }

        pub fn handle(&self) -> Handle {
            let id = self.next.fetch_add(1, Ordering::Relaxed);
            let (presses_tx, presses) = crossbeam_channel::unbounded();
            let failed = Arc::new(Mutex::new(Vec::new()));
            lock(&self.handles).insert(
                id,
                HandleState {
                    hotkeys: Vec::new(),
                    presses: presses_tx,
                    failed: failed.clone(),
                },
            );
            Handle {
                id,
                cmds: self.cmds.clone(),
                presses,
                failed,
            }
        }
    }

    pub struct Handle {
        id: u64,
        cmds: Sender<Cmd>,
        presses: Receiver<u32>,
        failed: Arc<Mutex<Vec<u32>>>,
    }

    impl GlobalHotkeys for Handle {
        fn register(&self, hotkeys: Vec<Hotkey>) {
            let _ = self.cmds.send(Cmd::Set(self.id, hotkeys));
        }

        fn failed(&self) -> Vec<u32> {
            lock(&self.failed).clone()
        }

        fn try_press(&self) -> Option<u32> {
            self.presses.try_recv().ok()
        }
    }

    impl Drop for Handle {
        fn drop(&mut self) {
            let _ = self.cmds.send(Cmd::Drop(self.id));
        }
    }

    /// One shortcut per chord: `(id, trigger, description)`.
    pub(super) fn wanted(
        handles: &HashMap<u64, HandleState>,
    ) -> Vec<(String, Option<String>, String)> {
        let mut by_chord: Vec<(String, Option<String>, String)> = Vec::new();
        for h in handles.values().flat_map(|s| &s.hotkeys) {
            let id = h.chord.to_string();
            match by_chord.iter_mut().find(|(i, _, _)| *i == id) {
                Some((_, _, label)) if !label.contains(h.label.as_str()) => {
                    label.push_str(", ");
                    label.push_str(&h.label);
                }
                Some(_) => {}
                None => by_chord.push((id, trigger(&h.chord), h.label.clone())),
            }
        }
        by_chord.sort();
        by_chord
    }

    /// Rebinds the session whenever the wanted shortcuts change.
    fn bind_loop(rx: &Receiver<Cmd>, handles: &Mutex<HashMap<u64, HandleState>>) {
        let Some(portal) = pollster::block_on(GlobalShortcuts::new()).ok() else {
            return;
        };
        let mut session: Option<Session<GlobalShortcuts>> = None;
        let mut bound = Vec::new();
        while let Ok(cmd) = rx.recv() {
            {
                let mut map = lock(handles);
                match cmd {
                    Cmd::Set(id, hotkeys) => {
                        if let Some(h) = map.get_mut(&id) {
                            h.hotkeys = hotkeys;
                        }
                    }
                    Cmd::Drop(id) => {
                        map.remove(&id);
                    }
                }
            }
            // Take the latest state when several changes queued up.
            while let Ok(cmd) = rx.try_recv() {
                let mut map = lock(handles);
                match cmd {
                    Cmd::Set(id, hotkeys) => {
                        if let Some(h) = map.get_mut(&id) {
                            h.hotkeys = hotkeys;
                        }
                    }
                    Cmd::Drop(id) => {
                        map.remove(&id);
                    }
                }
            }
            let want = wanted(&lock(handles));
            if want == bound {
                continue;
            }
            if let Some(old) = session.take() {
                let _ = pollster::block_on(old.close());
            }
            let assigned = if want.is_empty() {
                Some(HashMap::new())
            } else {
                pollster::block_on(bind(&portal, &want)).map(|(s, assigned)| {
                    session = Some(s);
                    assigned
                })
            };
            // Hotkeys the desktop gave no keys (or refused) count as failed.
            for state in lock(handles).values() {
                *lock(&state.failed) = state
                    .hotkeys
                    .iter()
                    .filter(|h| {
                        assigned.as_ref().is_none_or(|a| {
                            !a.get(&h.chord.to_string()).is_some_and(|t| !t.is_empty())
                        })
                    })
                    .map(|h| h.id)
                    .collect();
            }
            bound = want;
        }
    }

    /// Creates a session and binds `want`; the desktop may ask the user first. Returns the
    /// session and each shortcut's assigned keys.
    async fn bind(
        portal: &GlobalShortcuts,
        want: &[(String, Option<String>, String)],
    ) -> Option<(Session<GlobalShortcuts>, HashMap<String, String>)> {
        let session = portal
            .create_session(Default::default())
            .await
            .inspect_err(|e| tracing::warn!(error = %e, "couldn't start a shortcuts session"))
            .ok()?;
        let shortcuts: Vec<NewShortcut> = want
            .iter()
            .map(|(id, trigger, label)| {
                NewShortcut::new(id.clone(), format!("{label} ({id})"))
                    .preferred_trigger(trigger.as_deref())
            })
            .collect();
        let response = portal
            .bind_shortcuts(&session, &shortcuts, None, Default::default())
            .await
            .and_then(|r| r.response())
            .inspect_err(|e| tracing::warn!(error = %e, "global hotkeys not bound"))
            .ok()?;
        let assigned: HashMap<String, String> = response
            .shortcuts()
            .iter()
            .map(|s| (s.id().to_owned(), s.trigger_description().to_owned()))
            .collect();
        tracing::info!(?assigned, "global hotkeys bound");
        Some((session, assigned))
    }

    /// Passes every activation to the hotkeys with that chord. The portal sends its signals
    /// to the session's owner only, and there is one session at a time.
    async fn activations(handles: &Mutex<HashMap<u64, HandleState>>) {
        let Ok(portal) = GlobalShortcuts::new().await else {
            return;
        };
        let Ok(mut stream) = portal.receive_activated().await else {
            return;
        };
        while let Some(activated) = stream.next().await {
            let chord = activated.shortcut_id();
            for state in lock(handles).values() {
                for h in state
                    .hotkeys
                    .iter()
                    .filter(|h| h.chord.to_string() == chord)
                {
                    let _ = state.presses.send(h.id);
                }
            }
        }
    }
}

// ------------------------------------------------------------------ X11

mod x11 {
    use super::*;
    use std::time::Duration;
    use x11rb::connection::Connection;
    use x11rb::protocol::Event;
    use x11rb::protocol::xproto::{ConnectionExt, GrabMode, ModMask};

    /// Shift, Control, Mod1 (Alt), Mod4 (Super).
    fn modifiers(chord: &estream_project::Chord) -> u16 {
        let mut m = 0;
        for (on, mask) in [
            (chord.shift, 1),
            (chord.ctrl, 4),
            (chord.alt, 8),
            (chord.win, 0x40),
        ] {
            if on {
                m |= mask;
            }
        }
        m
    }
    /// Lock (Caps Lock) and Mod2 (Num Lock) must not stop a hotkey.
    const IGNORED: [u16; 4] = [0, 0x02, 0x10, 0x12];

    pub struct Grabber {
        cmds: Sender<Vec<Hotkey>>,
        presses: Receiver<u32>,
        failed: Arc<Mutex<Vec<u32>>>,
    }

    impl Grabber {
        pub fn start() -> Option<Self> {
            let (conn, screen) = x11rb::connect(None)
                .inspect_err(|e| tracing::info!(error = %e, "no X11 display for hotkeys"))
                .ok()?;
            let root = conn.setup().roots.get(screen)?.root;
            let (cmds, rx) = crossbeam_channel::unbounded();
            let (presses_tx, presses) = crossbeam_channel::unbounded();
            let failed: Arc<Mutex<Vec<u32>>> = Arc::default();
            let thread_failed = failed.clone();
            std::thread::Builder::new()
                .name("estream-hotkeys-x11".into())
                .spawn(move || run(&conn, root, &rx, &presses_tx, &thread_failed))
                .ok()?;
            tracing::info!("global hotkeys through X11 key grabs");
            Some(Self {
                cmds,
                presses,
                failed,
            })
        }
    }

    impl GlobalHotkeys for Grabber {
        fn register(&self, hotkeys: Vec<Hotkey>) {
            let _ = self.cmds.send(hotkeys);
        }

        fn failed(&self) -> Vec<u32> {
            lock(&self.failed).clone()
        }

        fn try_press(&self) -> Option<u32> {
            self.presses.try_recv().ok()
        }
    }

    /// The keycode that produces `sym` (unshifted or shifted).
    fn keycode(conn: &impl Connection, sym: u32) -> Option<u8> {
        let setup = conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let map = conn
            .get_keyboard_mapping(min, max - min + 1)
            .ok()?
            .reply()
            .ok()?;
        let per = map.keysyms_per_keycode as usize;
        map.keysyms
            .chunks(per.max(1))
            .position(|syms| syms.iter().take(2).any(|s| *s == sym))
            .map(|i| min + i as u8)
    }

    fn run(
        conn: &impl Connection,
        root: u32,
        rx: &Receiver<Vec<Hotkey>>,
        presses: &Sender<u32>,
        failed: &Mutex<Vec<u32>>,
    ) {
        // (keycode, modifiers) → hotkey id.
        let mut grabbed: Vec<(u8, u16, u32)> = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(list) => {
                    for (code, mods, _) in grabbed.drain(..) {
                        for extra in IGNORED {
                            let _ = conn.ungrab_key(code, root, ModMask::from(mods | extra));
                        }
                    }
                    let mut refused = Vec::new();
                    for h in list {
                        let Some(code) =
                            keysym(&h.chord.key).and_then(|(_, sym)| keycode(conn, sym))
                        else {
                            refused.push(h.id);
                            continue;
                        };
                        let mods = modifiers(&h.chord);
                        // Another client holding the keys answers with BadAccess.
                        let ok = IGNORED.iter().all(|extra| {
                            conn.grab_key(
                                false,
                                root,
                                ModMask::from(mods | extra),
                                code,
                                GrabMode::ASYNC,
                                GrabMode::ASYNC,
                            )
                            .ok()
                            .is_some_and(|c| c.check().is_ok())
                        });
                        if ok {
                            grabbed.push((code, mods, h.id));
                        } else {
                            // Release the variants grabbed before the refusal.
                            for extra in IGNORED {
                                let _ = conn.ungrab_key(code, root, ModMask::from(mods | extra));
                            }
                            refused.push(h.id);
                        }
                    }
                    *lock(failed) = refused;
                    let _ = conn.flush();
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => return,
                Err(crossbeam_channel::TryRecvError::Empty) => {}
            }
            match conn.poll_for_event() {
                Ok(Some(Event::KeyPress(e))) => {
                    let state = u16::from(e.state) & !(0x02 | 0x10);
                    for (_, _, id) in grabbed
                        .iter()
                        .filter(|(code, mods, _)| *code == e.detail && *mods == state)
                    {
                        let _ = presses.send(*id);
                    }
                }
                Ok(Some(_)) => {}
                Ok(None) => std::thread::sleep(Duration::from_millis(15)),
                Err(_) => return,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use estream_project::Chord;

    fn hotkey(id: u32, chord: &str, label: &str) -> Hotkey {
        Hotkey {
            id,
            chord: Chord::parse(chord).unwrap_or_default(),
            label: label.into(),
        }
    }

    #[test]
    fn shared_chords_become_one_shortcut() {
        let (presses, _) = crossbeam_channel::unbounded();
        let mut handles = HashMap::new();
        for (n, list) in [
            (1, vec![hotkey(1, "Ctrl+Shift+R", "Save a clip")]),
            (
                2,
                vec![
                    hotkey(1, "Ctrl+Shift+R", "Mark"),
                    hotkey(2, "F9", "Scene 2"),
                ],
            ),
        ] {
            let state = portal::HandleState {
                hotkeys: list,
                presses: presses.clone(),
                failed: Arc::default(),
            };
            handles.insert(n, state);
        }
        let want = portal::wanted(&handles);
        assert_eq!(want.len(), 2);
        let (id, trigger, label) = &want[0];
        assert_eq!(id, "Ctrl+Shift+R");
        assert_eq!(trigger.as_deref(), Some("CTRL+SHIFT+r"));
        assert!(label.contains("Save a clip") && label.contains("Mark"));
        assert_eq!(want[1].1.as_deref(), Some("F9"));
    }
}
