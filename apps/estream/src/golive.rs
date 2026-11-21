//! The Go live window's data: each platform's stream details, loaded and applied on worker
//! threads before the video starts (see `openspec/changes/add-go-live-details`).

use crate::settings::StreamDetails;
use crate::youtube::{Broadcast, BroadcastDetails, Latency, StreamSetup};
use estream_media::Service;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

/// What the YouTube section shows.
#[derive(Clone, Debug, PartialEq)]
pub struct YouTubeLoaded {
    /// The channel's live and upcoming broadcasts.
    pub broadcasts: Vec<Broadcast>,
    /// Categories a broadcast can use: (id, name).
    pub categories: Vec<(String, String)>,
    /// The form for a new broadcast, pre-filled.
    pub new: BroadcastDetails,
}

/// Loads the YouTube section: a new broadcast starts from `saved`, else the last broadcast.
pub fn load_youtube(
    session: &crate::youtube::Session,
    saved: Option<&BroadcastDetails>,
) -> Result<YouTubeLoaded, String> {
    let broadcasts = session.broadcasts()?;
    let categories = session.categories().unwrap_or_else(|e| {
        tracing::debug!(error = %e, "YouTube categories");
        Vec::new()
    });
    let new = saved.cloned().unwrap_or_else(|| session.last_details());
    Ok(YouTubeLoaded {
        broadcasts,
        categories,
        new,
    })
}

/// Loads the Twitch section: the channel as it is now, else the saved details.
pub fn load_twitch(
    session: &crate::twitch::Session,
    saved: Option<&crate::twitch::ChannelDetails>,
) -> Result<crate::twitch::ChannelDetails, String> {
    prefill(session.channel_details(), saved)
}

/// Loads the Kick section: the channel as it is now, else the saved details. Kick reports an
/// empty title and no category while the channel is offline, so those count as unknown too.
pub fn load_kick(
    session: &crate::kick::Session,
    saved: Option<&crate::kick::ChannelDetails>,
) -> Result<crate::kick::ChannelDetails, String> {
    match session.channel_details() {
        Ok(d) if d.title.is_empty() && d.category_id == 0 => Ok(saved.cloned().unwrap_or(d)),
        current => prefill(current, saved),
    }
}

/// The channel's current details (they may have changed on the platform since), else the saved
/// ones when the channel can't be read.
fn prefill<T: Clone>(current: Result<T, String>, saved: Option<&T>) -> Result<T, String> {
    current.or_else(|e| {
        tracing::debug!(error = %e, "channel details; using the saved ones");
        saved.cloned().ok_or(e)
    })
}

// ---- The window ----

/// The accounts the window talks to.
#[derive(Clone)]
pub struct Sessions {
    pub youtube: crate::youtube::Session,
    pub twitch: crate::twitch::Session,
    pub kick: crate::kick::Session,
}

impl Sessions {
    /// Whether `service`'s account is connected (its section can be shown).
    pub fn connected(&self, service: Service) -> bool {
        match service {
            Service::YouTube => self.youtube.account().is_some(),
            Service::Twitch => self.twitch.account().is_some(),
            Service::Kick => self.kick.account().is_some(),
            _ => false,
        }
    }
}

/// Twitch's broadcaster languages offered in the window: (code, name).
const LANGUAGES: [(&str, &str); 20] = [
    ("en", "English"),
    ("es", "Spanish"),
    ("pt", "Portuguese"),
    ("de", "German"),
    ("fr", "French"),
    ("it", "Italian"),
    ("ja", "Japanese"),
    ("ko", "Korean"),
    ("zh", "Chinese"),
    ("ru", "Russian"),
    ("hi", "Hindi"),
    ("ar", "Arabic"),
    ("tr", "Turkish"),
    ("pl", "Polish"),
    ("nl", "Dutch"),
    ("sv", "Swedish"),
    ("th", "Thai"),
    ("id", "Indonesian"),
    ("vi", "Vietnamese"),
    ("other", "Other"),
];

const PRIVACY: [&str; 3] = ["public", "unlisted", "private"];

/// How long typing pauses before a category search runs.
const SEARCH_DELAY: Duration = Duration::from_millis(300);

#[derive(Clone, Debug, PartialEq)]
pub enum SectionState {
    Loading,
    Ready,
    LoadFailed(String),
    /// Applying the details failed (the form stays editable).
    ApplyFailed(String),
}

/// One platform's form.
#[derive(Clone, Debug)]
pub enum Form {
    YouTube {
        loaded: Option<YouTubeLoaded>,
        /// 0: a new broadcast; n: `loaded.broadcasts[n - 1]`.
        choice: usize,
        details: BroadcastDetails,
    },
    Twitch {
        details: crate::twitch::ChannelDetails,
        results: Vec<(String, String, String)>,
        /// A search waiting for typing to pause: the text and when it was typed.
        search: Option<(String, Instant)>,
        /// The search sent to the platform; its results count only while it's still wanted
        /// (picking a category drops it).
        asked: Option<String>,
        /// What is typed in the search field (kept when the window redraws).
        typed: String,
    },
    Kick {
        details: crate::kick::ChannelDetails,
        results: Vec<(u64, String, String)>,
        search: Option<(String, Instant)>,
        asked: Option<String>,
        /// What is typed in the search field (kept when the window redraws).
        typed: String,
    },
}

/// One platform's section: a destination with a connected account.
#[derive(Clone, Debug)]
pub struct Section {
    /// Index into `settings.destinations`.
    pub config: usize,
    pub name: String,
    pub state: SectionState,
    /// "Go live without <platform>" was clicked: not applied, not streamed to.
    pub skipped: bool,
    /// Going live without the window, applying failed: streamed to anyway (the error is in
    /// [`Window::errors`]).
    given_up: bool,
    /// Applying has started at least once.
    tried: bool,
    /// Its details are being applied.
    pub applying: bool,
    /// The destination's stream key (YouTube: which stream the broadcast goes on).
    key: String,
    /// What the form is filled in from: the open setup's details, else the last ones used.
    saved: StreamDetails,
    /// The open setup has its own details for this platform (they win over the channel's and a
    /// waiting broadcast's).
    own: bool,
    pub form: Form,
}

/// What applying a section did.
#[derive(Clone, Debug, PartialEq)]
pub struct Applied {
    /// One line for a toast ("YouTube: new broadcast …").
    pub message: String,
    /// The YouTube broadcast to open from the toast.
    pub link: Option<String>,
    /// A broadcast Clutch created (deleted again if the stream ends before it goes live).
    pub created: Option<String>,
    /// The chosen broadcast uses another of the channel's streams: switch the destination to it.
    pub switch: Option<StreamSetup>,
    /// What to remember for the destination.
    pub details: StreamDetails,
}

enum Event {
    YouTube(usize, Result<YouTubeLoaded, String>),
    Twitch(usize, Result<crate::twitch::ChannelDetails, String>),
    Kick(usize, Result<crate::kick::ChannelDetails, String>),
    TwitchFound(usize, String, Vec<(String, String, String)>),
    KickFound(usize, String, Vec<(u64, String, String)>),
    Applied(usize, Box<Result<Applied, String>>),
}

/// What the app should do after [`Window::poll`].
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// The sections changed: show them again.
    Changed,
    /// Every section that wasn't skipped is applied: go live (or, while live, it's saved).
    Done(Vec<(usize, Applied)>),
}

/// What the window is for.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    /// Before going live, in the window.
    GoLive,
    /// Going live without the window.
    Auto,
    /// Editing the running stream's details.
    Live,
    /// Saving the open setup's details without going live.
    Save,
}

/// The Go live window, or the same work without a window (remembered details).
pub struct Window {
    pub sections: Vec<Section>,
    /// Enabled destinations without a connected account (streamed with their key).
    pub keys_only: Vec<String>,
    /// Editing while live (Save, no broadcast choice, no video start).
    pub live: bool,
    /// Only saving the details in the open setup (offline Stream details): nothing is sent to
    /// the platforms, and YouTube's form is the new broadcast's.
    pub saving: bool,
    sessions: Sessions,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    /// Results of the sections applied so far.
    applied: Vec<(usize, Applied)>,
    /// Going live without the window: each section is applied as soon as it's loaded, and a
    /// failure doesn't hold the stream back.
    auto: bool,
    /// What failed without the window ("YouTube: …"), for a toast.
    pub errors: Vec<String>,
}

impl Window {
    /// Opens the window for `dests` (settings index, destination) before going live in `setup`,
    /// and starts loading each connected platform's section.
    pub fn open(
        dests: &[(usize, &crate::settings::DestinationConfig)],
        sessions: Sessions,
        setup: &str,
    ) -> Self {
        Self::start(dests, sessions, setup, Mode::GoLive)
    }

    /// The window for editing the running stream's details (the channels as they are now).
    pub fn edit_live(
        dests: &[(usize, &crate::settings::DestinationConfig)],
        sessions: Sessions,
    ) -> Self {
        Self::start(dests, sessions, "", Mode::Live)
    }

    /// The window for saving `setup`'s details without going live.
    pub fn saving(
        dests: &[(usize, &crate::settings::DestinationConfig)],
        sessions: Sessions,
        setup: &str,
    ) -> Self {
        Self::start(dests, sessions, setup, Mode::Save)
    }

    /// The same work without the window, with `setup`'s details (else the ones used last):
    /// YouTube gets a broadcast (the one waiting on the key, else a new one), and Twitch and
    /// Kick get the saved details if there are any (else they're left as they are).
    pub fn remembered(
        dests: &[(usize, &crate::settings::DestinationConfig)],
        sessions: Sessions,
        setup: &str,
    ) -> Self {
        Self::start(dests, sessions, setup, Mode::Auto)
    }

    fn start(
        dests: &[(usize, &crate::settings::DestinationConfig)],
        sessions: Sessions,
        setup: &str,
        mode: Mode,
    ) -> Self {
        let (live, auto) = (mode == Mode::Live, mode == Mode::Auto);
        let (tx, rx) = channel();
        let mut window = Self {
            sections: Vec::new(),
            keys_only: Vec::new(),
            live,
            saving: mode == Mode::Save,
            sessions,
            tx,
            rx,
            applied: Vec::new(),
            auto,
            errors: Vec::new(),
        };
        let mut ready = Vec::new();
        for (config, d) in dests {
            let service = d.stream.service;
            if !window.sessions.connected(service) {
                window.keys_only.push(d.name.clone());
                continue;
            }
            // While live the channels as they are now count; otherwise the setup's own details
            // come first, then the ones used last.
            let own = if live {
                StreamDetails::default()
            } else {
                d.own_details(setup)
            };
            let saved = own.clone().or(&d.details);
            let (has_saved, has_own) = match service {
                Service::YouTube => (saved.youtube.is_some(), own.youtube.is_some()),
                Service::Twitch => (saved.twitch.is_some(), own.twitch.is_some()),
                _ => (saved.kick.is_some(), own.kick.is_some()),
            };
            // Twitch and Kick with the setup's own details, and without the window with any saved
            // ones, are used as they are (nothing to load). Without the window and nothing saved,
            // they're left as they are.
            if service != Service::YouTube {
                if auto && !has_saved {
                    continue;
                }
                if auto || has_own {
                    ready.push(window.sections.len());
                }
            }
            let form = match service {
                Service::YouTube => Form::YouTube {
                    loaded: None,
                    choice: 0,
                    details: saved.youtube.clone().unwrap_or_default(),
                },
                Service::Twitch => Form::Twitch {
                    details: saved.twitch.clone().unwrap_or_default(),
                    results: Vec::new(),
                    search: None,
                    asked: None,
                    typed: String::new(),
                },
                _ => Form::Kick {
                    details: saved.kick.clone().unwrap_or_default(),
                    results: Vec::new(),
                    search: None,
                    asked: None,
                    typed: String::new(),
                },
            };
            window.sections.push(Section {
                config: *config,
                name: d.name.clone(),
                state: SectionState::Loading,
                skipped: false,
                given_up: false,
                tried: false,
                applying: false,
                key: d.stream.key.clone(),
                saved,
                own: has_own,
                form,
            });
        }
        for i in 0..window.sections.len() {
            if ready.contains(&i) {
                window.sections[i].state = SectionState::Ready;
            } else {
                let saved = window.sections[i].saved.clone();
                window.load_one(i, saved);
            }
        }
        window
    }

    /// Whether any destination has a connected account (else there's nothing to ask).
    pub fn has_sections(&self) -> bool {
        !self.sections.is_empty()
    }

    /// (Re)loads section `i` in the background.
    fn load_one(&mut self, i: usize, saved: StreamDetails) {
        let Some(section) = self.sections.get_mut(i) else {
            return;
        };
        section.state = SectionState::Loading;
        let tx = self.tx.clone();
        let sessions = self.sessions.clone();
        let form = section.form.clone();
        spawn("estream-golive-load", move || {
            let event = match form {
                Form::YouTube { .. } => {
                    Event::YouTube(i, load_youtube(&sessions.youtube, saved.youtube.as_ref()))
                }
                Form::Twitch { .. } => {
                    Event::Twitch(i, load_twitch(&sessions.twitch, saved.twitch.as_ref()))
                }
                Form::Kick { .. } => Event::Kick(i, load_kick(&sessions.kick, saved.kick.as_ref())),
            };
            let _ = tx.send(event);
        });
    }

    /// "Try again" on section `i`: reload it if it couldn't load, else apply it again.
    pub fn retry(&mut self, i: usize) {
        match self.sections.get(i).map(|s| s.state.clone()) {
            Some(SectionState::LoadFailed(_)) => {
                let saved = self.form_details(i);
                self.load_one(i, saved);
            }
            Some(SectionState::ApplyFailed(_)) => self.apply_one(i),
            _ => {}
        }
    }

    /// "Go live without <platform>": section `i` is left out.
    pub fn skip(&mut self, i: usize) {
        if let Some(s) = self.sections.get_mut(i) {
            s.skipped = true;
            s.applying = false;
        }
    }

    /// Settings indices of the skipped destinations (not streamed to).
    pub fn skipped(&self) -> Vec<usize> {
        self.sections
            .iter()
            .filter(|s| s.skipped)
            .map(|s| s.config)
            .collect()
    }

    /// Whether details are being applied (the button shows it).
    pub fn busy(&self) -> Option<String> {
        let names: Vec<&str> = self
            .sections
            .iter()
            .filter(|s| s.applying)
            .map(|s| platform_name(&s.form))
            .collect();
        (!names.is_empty()).then(|| format!("Setting up {}…", names.join(", ")))
    }

    /// Whether a platform's details are still loading (Go live waits for them).
    pub fn loading(&self) -> bool {
        self.sections
            .iter()
            .any(|s| !s.skipped && matches!(s.state, SectionState::Loading))
    }

    /// Save: the details in each section that has loaded, by settings index, to keep in the
    /// open setup. Sections still loading or that couldn't load are left out, so their saved
    /// details aren't replaced with an empty form.
    pub fn to_save(&self) -> Vec<(usize, StreamDetails)> {
        self.sections
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                !s.skipped && matches!(s.state, SectionState::Ready | SectionState::ApplyFailed(_))
            })
            .map(|(i, s)| (s.config, self.form_details(i)))
            .collect()
    }

    /// The form's current details, as they'd be remembered.
    fn form_details(&self, i: usize) -> StreamDetails {
        let mut saved = StreamDetails::default();
        match self.sections.get(i).map(|s| &s.form) {
            Some(Form::YouTube { details, .. }) => saved.youtube = Some(details.clone()),
            Some(Form::Twitch { details, .. }) => saved.twitch = Some(details.clone()),
            Some(Form::Kick { details, .. }) => saved.kick = Some(details.clone()),
            None => {}
        }
        saved
    }

    // ---- Editing ----

    /// A text field changed: "title", "description", "tags" or "category-search".
    pub fn text(&mut self, i: usize, field: &str, value: &str) {
        let Some(s) = self.sections.get_mut(i) else {
            return;
        };
        match (&mut s.form, field) {
            (Form::YouTube { details, .. }, "title") => details.title = value.to_owned(),
            (Form::YouTube { details, .. }, "description") => {
                details.description = value.to_owned();
            }
            (Form::Twitch { details, .. }, "title") => details.title = value.to_owned(),
            (Form::Kick { details, .. }, "title") => details.title = value.to_owned(),
            (Form::Twitch { details, .. }, "tags") => details.tags = split_tags(value),
            (
                Form::Twitch { search, typed, .. } | Form::Kick { search, typed, .. },
                "category-search",
            ) => {
                *typed = value.to_owned();
                *search = Some((value.to_owned(), Instant::now()));
            }
            _ => {}
        }
    }

    /// A switch changed: "kids", "auto-start" or "auto-stop" (YouTube).
    pub fn flag(&mut self, i: usize, field: &str, value: bool) {
        if let Some(Section {
            form: Form::YouTube { details, .. },
            ..
        }) = self.sections.get_mut(i)
        {
            match field {
                "kids" => details.made_for_kids = value,
                "auto-start" => details.auto_start = value,
                "auto-stop" => details.auto_stop = value,
                _ => {}
            }
        }
    }

    /// A list choice changed: "broadcast", "privacy", "category", "latency" or "language".
    pub fn choose(&mut self, i: usize, field: &str, index: i32) {
        let Some(s) = self.sections.get_mut(i) else {
            return;
        };
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        match (&mut s.form, field) {
            (
                Form::YouTube {
                    loaded: Some(loaded),
                    choice,
                    details,
                },
                "broadcast",
            ) => {
                if index == 0 {
                    *details = loaded.new.clone();
                } else if let Some(b) = loaded.broadcasts.get(index - 1) {
                    *details = b.details.clone();
                } else {
                    return;
                }
                *choice = index;
            }
            (Form::YouTube { details, .. }, "privacy") => {
                if let Some(p) = PRIVACY.get(index) {
                    details.privacy = (*p).to_owned();
                }
            }
            (
                Form::YouTube {
                    details, loaded, ..
                },
                "category",
            ) => {
                if let Some((id, _)) = loaded.as_ref().and_then(|l| l.categories.get(index)) {
                    details.category = id.clone();
                }
            }
            (Form::YouTube { details, .. }, "latency") => {
                details.latency = match index {
                    1 => Latency::Low,
                    2 => Latency::UltraLow,
                    _ => Latency::Normal,
                };
            }
            (
                Form::Twitch {
                    details,
                    results,
                    search,
                    asked,
                    typed,
                },
                "category",
            ) => {
                if let Some((id, name, _)) = results.get(index) {
                    details.category_id = id.clone();
                    details.category = name.clone();
                    results.clear();
                    (*search, *asked) = (None, None);
                    typed.clear();
                }
            }
            (
                Form::Kick {
                    details,
                    results,
                    search,
                    asked,
                    typed,
                },
                "category",
            ) => {
                if let Some((id, name, _)) = results.get(index) {
                    details.category_id = *id;
                    details.category = name.clone();
                    results.clear();
                    (*search, *asked) = (None, None);
                    typed.clear();
                }
            }
            (Form::Twitch { details, .. }, "language") => {
                if let Some(code) = languages(&details.language).get(index) {
                    details.language = code.0.clone();
                }
            }
            _ => {}
        }
    }

    // ---- Background work ----

    /// Runs the category searches whose typing has paused.
    fn search(&mut self) {
        for (i, s) in self.sections.iter_mut().enumerate() {
            let tx = self.tx.clone();
            match &mut s.form {
                Form::Twitch { search, asked, .. } => {
                    if let Some(query) = due(search) {
                        *asked = Some(query.clone());
                        let session = self.sessions.twitch.clone();
                        spawn("estream-golive-search", move || {
                            let found = session.search_categories(&query).unwrap_or_default();
                            let _ = tx.send(Event::TwitchFound(i, query, found));
                        });
                    }
                }
                Form::Kick { search, asked, .. } => {
                    if let Some(query) = due(search) {
                        *asked = Some(query.clone());
                        let session = self.sessions.kick.clone();
                        spawn("estream-golive-search", move || {
                            let found = session.search_categories(&query).unwrap_or_default();
                            let _ = tx.send(Event::KickFound(i, query, found));
                        });
                    }
                }
                Form::YouTube { .. } => {}
            }
        }
    }

    /// Applies every section that isn't skipped. Returns false when a section isn't ready yet.
    pub fn apply(&mut self) -> bool {
        let ready = self.sections.iter().all(|s| {
            s.skipped || matches!(s.state, SectionState::Ready | SectionState::ApplyFailed(_))
        });
        if !ready {
            return false;
        }
        self.applied.clear();
        for i in 0..self.sections.len() {
            if !self.sections[i].skipped {
                self.apply_one(i);
            }
        }
        true
    }

    fn apply_one(&mut self, i: usize) {
        let Some(s) = self.sections.get_mut(i) else {
            return;
        };
        s.applying = true;
        s.tried = true;
        s.state = SectionState::Ready;
        let tx = self.tx.clone();
        let sessions = self.sessions.clone();
        let (form, live, key) = (s.form.clone(), self.live, s.key.clone());
        spawn("estream-golive-apply", move || {
            let result = match form {
                Form::YouTube {
                    loaded,
                    choice,
                    details,
                } => {
                    let chosen = choice
                        .checked_sub(1)
                        .and_then(|n| loaded.and_then(|l| l.broadcasts.get(n).cloned()));
                    apply_youtube(&sessions.youtube, chosen, &details, &key, live)
                }
                Form::Twitch { details, .. } => sessions
                    .twitch
                    .set_channel_details(&details)
                    .map(|()| channel_applied("Twitch", &details.title, &details.category))
                    .map(|mut a| {
                        a.details.twitch = Some(details);
                        a
                    }),
                Form::Kick { details, .. } => sessions
                    .kick
                    .set_channel_details(&details)
                    .map(|()| channel_applied("Kick", &details.title, &details.category))
                    .map(|mut a| {
                        a.details.kick = Some(details);
                        a
                    }),
            };
            let _ = tx.send(Event::Applied(i, Box::new(result)));
        });
    }

    /// Takes in the background results and runs due searches.
    pub fn poll(&mut self) -> Option<Outcome> {
        self.search();
        let mut changed = false;
        let (live, saving) = (self.live, self.saving);
        while let Ok(event) = self.rx.try_recv() {
            changed = true;
            match event {
                Event::YouTube(i, result) => self.loaded(i, result, |s, loaded| {
                    // Saving: the setup's details are a new broadcast's.
                    let first = if saving {
                        None
                    } else {
                        first_choice(&loaded.broadcasts, &s.key, live)
                    };
                    let own = s.saved.youtube.clone().filter(|_| s.own);
                    if let Form::YouTube {
                        loaded: l,
                        choice,
                        details,
                    } = &mut s.form
                    {
                        *choice = first.map_or(0, |n| n + 1);
                        *details = match first {
                            Some(n) => {
                                let b = &loaded.broadcasts[n];
                                match own {
                                    // One waiting on the key (as Studio makes, a copy of the last
                                    // stream's) gets what the setup says viewers see.
                                    Some(own) if !live && can_edit(b) => {
                                        with_setup_details(&b.details, &own)
                                    }
                                    _ => b.details.clone(),
                                }
                            }
                            None => loaded.new.clone(),
                        };
                        *l = Some(loaded);
                    }
                }),
                Event::Twitch(i, result) => self.loaded(i, result, |s, loaded| {
                    if let Form::Twitch { details, .. } = &mut s.form {
                        *details = loaded;
                    }
                }),
                Event::Kick(i, result) => self.loaded(i, result, |s, loaded| {
                    if let Form::Kick { details, .. } = &mut s.form {
                        *details = loaded;
                    }
                }),
                Event::TwitchFound(i, query, found) => {
                    if let Some(Section {
                        form: Form::Twitch { results, asked, .. },
                        ..
                    }) = self.sections.get_mut(i)
                        && asked.as_deref() == Some(query.as_str())
                    {
                        tracing::debug!(%query, n = found.len(), "Twitch categories");
                        *results = found;
                        *asked = None;
                    }
                }
                Event::KickFound(i, query, found) => {
                    if let Some(Section {
                        form: Form::Kick { results, asked, .. },
                        ..
                    }) = self.sections.get_mut(i)
                        && asked.as_deref() == Some(query.as_str())
                    {
                        tracing::debug!(%query, n = found.len(), "Kick categories");
                        *results = found;
                        *asked = None;
                    }
                }
                Event::Applied(i, result) => {
                    let Some(s) = self.sections.get_mut(i) else {
                        continue;
                    };
                    s.applying = false;
                    match *result {
                        Ok(applied) => self.applied.push((s.config, applied)),
                        Err(e) => {
                            tracing::warn!(platform = platform_name(&s.form), error = %e, "stream details not applied");
                            s.state = SectionState::ApplyFailed(e);
                        }
                    }
                }
            }
        }
        if self.auto {
            self.advance_auto();
        }
        // Done once every section is applied, skipped or given up, and Go live was pressed (or
        // there's no window).
        let settled = |s: &Section| {
            s.skipped || s.given_up || self.applied.iter().any(|(c, _)| *c == s.config)
        };
        let pressed = self.auto || self.sections.iter().any(|s| s.tried || s.skipped);
        if pressed && self.sections.iter().all(settled) {
            return Some(Outcome::Done(std::mem::take(&mut self.applied)));
        }
        changed.then_some(Outcome::Changed)
    }

    /// Without the window: applies each section once it's loaded; a failure is noted and the
    /// destination streams anyway.
    fn advance_auto(&mut self) {
        for i in 0..self.sections.len() {
            let s = &self.sections[i];
            if s.given_up || s.skipped {
                continue;
            }
            match &s.state {
                SectionState::Ready if !s.tried => self.apply_one(i),
                SectionState::LoadFailed(e) | SectionState::ApplyFailed(e) => {
                    let error = format!("{}: {e}", platform_name(&s.form));
                    self.errors.push(error);
                    self.sections[i].given_up = true;
                }
                _ => {}
            }
        }
    }

    fn loaded<T>(
        &mut self,
        i: usize,
        result: Result<T, String>,
        fill: impl FnOnce(&mut Section, T),
    ) {
        let Some(s) = self.sections.get_mut(i) else {
            return;
        };
        match result {
            Ok(loaded) => {
                fill(s, loaded);
                s.state = SectionState::Ready;
            }
            Err(e) => s.state = SectionState::LoadFailed(e),
        }
    }

    // ---- Showing ----

    /// The sections for the window.
    /// `image` gives a category picture by address (`None` while it loads).
    pub fn rows(
        &self,
        image: &mut dyn FnMut(&str) -> Option<slint::Image>,
    ) -> Vec<crate::ui::DetailsSection> {
        self.sections
            .iter()
            .filter(|s| !s.skipped)
            .map(|s| row(s, self.live, image))
            .collect()
    }

    /// Maps a row of [`Self::rows`] back to its section index.
    pub fn section_of_row(&self, row: i32) -> Option<usize> {
        let row = usize::try_from(row).ok()?;
        self.sections
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.skipped)
            .nth(row)
            .map(|(i, _)| i)
    }
}

/// Whether a broadcast hasn't started yet (its latency and auto-start can still change).
fn can_edit(b: &Broadcast) -> bool {
    matches!(b.status.as_str(), "created" | "ready")
}

/// A waiting broadcast's details with what a setup says viewers see: title, description,
/// privacy, category and made-for-kids. How it runs (latency, auto-start, auto-stop) stays.
fn with_setup_details(broadcast: &BroadcastDetails, setup: &BroadcastDetails) -> BroadcastDetails {
    BroadcastDetails {
        title: setup.title.clone(),
        description: setup.description.clone(),
        privacy: setup.privacy.clone(),
        category: setup.category.clone(),
        made_for_kids: setup.made_for_kids,
        ..broadcast.clone()
    }
}

/// The broadcast the YouTube form starts on (`None`: a new one). While live, the one that's on
/// with this key; before, one waiting on this key (as Studio's Stream page makes), so Clutch
/// doesn't make a second one.
fn first_choice(broadcasts: &[Broadcast], key: &str, live: bool) -> Option<usize> {
    let on_key = |b: &Broadcast| b.stream.as_ref().is_some_and(|(_, s)| s.key == key);
    if live {
        return broadcasts
            .iter()
            .position(|b| b.status == "live" && on_key(b))
            .or_else(|| broadcasts.iter().position(|b| b.status == "live"));
    }
    broadcasts
        .iter()
        .position(|b| b.status != "live" && on_key(b))
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name(name.into()).spawn(work) {
        tracing::warn!(error = %e, "couldn't start a Go live worker");
    }
}

/// The search text when typing has paused long enough (and clears it).
fn due(search: &mut Option<(String, Instant)>) -> Option<String> {
    match search {
        Some((_, at)) if at.elapsed() >= SEARCH_DELAY => search.take().map(|(q, _)| q),
        _ => None,
    }
}

fn platform_name(form: &Form) -> &'static str {
    match form {
        Form::YouTube { .. } => "YouTube",
        Form::Twitch { .. } => "Twitch",
        Form::Kick { .. } => "Kick",
    }
}

/// "a, b ,, c" → ["a", "b", "c"].
fn split_tags(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The language list, with `current` added when it isn't one of them.
fn languages(current: &str) -> Vec<(String, String)> {
    let mut list: Vec<(String, String)> = LANGUAGES
        .iter()
        .map(|(c, n)| ((*c).to_owned(), (*n).to_owned()))
        .collect();
    if !current.is_empty() && !list.iter().any(|(c, _)| c == current) {
        list.push((current.to_owned(), current.to_owned()));
    }
    list
}

fn channel_applied(platform: &str, title: &str, category: &str) -> Applied {
    let category = if category.is_empty() {
        String::new()
    } else {
        format!(" · {category}")
    };
    Applied {
        message: format!("{platform}: \"{title}\"{category}"),
        link: None,
        created: None,
        switch: None,
        details: StreamDetails::default(),
    }
}

/// Creates or updates the YouTube broadcast and ties it to the destination's stream.
fn apply_youtube(
    session: &crate::youtube::Session,
    chosen: Option<Broadcast>,
    details: &BroadcastDetails,
    key: &str,
    live: bool,
) -> Result<Applied, String> {
    if details.title.trim().is_empty() {
        return Err("Give the YouTube broadcast a title".into());
    }
    let remembered = StreamDetails {
        youtube: Some(details.clone()),
        ..Default::default()
    };
    let watch = |id: &str| Some(format!("https://www.youtube.com/watch?v={id}"));
    match chosen {
        None if live => Err("No YouTube broadcast is live to edit".into()),
        None => {
            let stream = session.stream_id_for_key(key)?;
            let id = session.create_broadcast(details, &stream)?;
            Ok(Applied {
                message: format!(
                    "YouTube: new broadcast \"{}\" ({})",
                    details.title, details.privacy
                ),
                link: watch(&id),
                created: Some(id),
                switch: None,
                details: remembered,
            })
        }
        Some(b) => {
            session.update_broadcast(&b.id, details)?;
            let switch = match &b.stream {
                Some((_, setup)) if setup.key != key => Some(setup.clone()),
                Some(_) => None,
                None => {
                    let stream = session.stream_id_for_key(key)?;
                    session.bind(&b.id, &stream)?;
                    None
                }
            };
            Ok(Applied {
                message: format!("YouTube: \"{}\" ({})", details.title, details.privacy),
                link: watch(&b.id),
                created: None,
                switch,
                details: remembered,
            })
        }
    }
}

/// A section as the window shows it.
fn row(
    s: &Section,
    live: bool,
    image: &mut dyn FnMut(&str) -> Option<slint::Image>,
) -> crate::ui::DetailsSection {
    use slint::{ModelRc, SharedString, VecModel};
    let strings = |items: Vec<String>| {
        ModelRc::new(VecModel::from(
            items
                .into_iter()
                .map(SharedString::from)
                .collect::<Vec<_>>(),
        ))
    };
    let (state, error) = match &s.state {
        SectionState::Loading => (0, String::new()),
        SectionState::Ready => (1, String::new()),
        SectionState::LoadFailed(e) => (2, e.clone()),
        SectionState::ApplyFailed(e) => (3, e.clone()),
    };
    let mut out = crate::ui::DetailsSection {
        name: s.name.clone().into(),
        state,
        error: error.into(),
        category_index: -1,
        ..Default::default()
    };
    match &s.form {
        Form::YouTube {
            loaded,
            choice,
            details,
        } => {
            out.kind = "youtube".into();
            let mut options = vec!["New broadcast".to_owned()];
            let mut categories = Vec::new();
            let mut locked = false;
            if let Some(l) = loaded {
                options.extend(l.broadcasts.iter().map(broadcast_label));
                categories = l.categories.clone();
                locked = choice
                    .checked_sub(1)
                    .and_then(|n| l.broadcasts.get(n))
                    .is_some_and(|b| !can_edit(b));
            }
            out.broadcasts = strings(options);
            out.broadcast = i32::try_from(*choice).unwrap_or(0);
            out.category_index = categories
                .iter()
                .position(|(id, _)| *id == details.category)
                .and_then(|p| i32::try_from(p).ok())
                .unwrap_or(-1);
            out.categories = strings(categories.into_iter().map(|(_, n)| n).collect());
            out.title = details.title.clone().into();
            out.description = details.description.clone().into();
            out.privacy = PRIVACY
                .iter()
                .position(|p| *p == details.privacy)
                .and_then(|p| i32::try_from(p).ok())
                .unwrap_or(0);
            out.kids = details.made_for_kids;
            out.latency = match details.latency {
                Latency::Normal => 0,
                Latency::Low => 1,
                Latency::UltraLow => 2,
            };
            out.auto_start = details.auto_start;
            out.auto_stop = details.auto_stop;
            out.locked = locked || live;
        }
        Form::Twitch {
            details,
            results,
            typed,
            ..
        } => {
            out.kind = "twitch".into();
            out.title = details.title.clone().into();
            out.category = details.category.clone().into();
            out.category_query = typed.clone().into();
            out.categories = strings(results.iter().map(|(_, n, _)| n.clone()).collect());
            out.category_images = images_of(results.iter().map(|(_, _, u)| u.as_str()), image);
            out.tags = details.tags.join(", ").into();
            let langs = languages(&details.language);
            out.language = langs
                .iter()
                .position(|(c, _)| *c == details.language)
                .and_then(|p| i32::try_from(p).ok())
                .unwrap_or(-1);
            out.languages = strings(langs.into_iter().map(|(_, n)| n).collect());
        }
        Form::Kick {
            details,
            results,
            typed,
            ..
        } => {
            out.kind = "kick".into();
            out.title = details.title.clone().into();
            out.category = details.category.clone().into();
            out.category_query = typed.clone().into();
            out.categories = strings(results.iter().map(|(_, n, _)| n.clone()).collect());
            out.category_images = images_of(results.iter().map(|(_, _, u)| u.as_str()), image);
        }
    }
    out
}

/// The pictures for a list of addresses (empty images while they load).
fn images_of<'a>(
    urls: impl Iterator<Item = &'a str>,
    image: &mut dyn FnMut(&str) -> Option<slint::Image>,
) -> slint::ModelRc<slint::Image> {
    let images: Vec<slint::Image> = urls
        .map(|u| {
            if u.is_empty() {
                slint::Image::default()
            } else {
                image(u).unwrap_or_default()
            }
        })
        .collect();
    slint::ModelRc::new(slint::VecModel::from(images))
}

/// "Ranked · 8:05 PM · public · uses this key" style label for the broadcast list.
fn broadcast_label(b: &Broadcast) -> String {
    let mut parts = vec![if b.details.title.is_empty() {
        "Untitled".to_owned()
    } else {
        b.details.title.clone()
    }];
    if b.status == "live" {
        parts.push("live now".into());
    } else if let Ok(at) = chrono::DateTime::parse_from_rfc3339(&b.scheduled) {
        parts.push(
            at.with_timezone(&chrono::Local)
                .format("%b %-d, %-I:%M %p")
                .to_string(),
        );
    }
    parts.push(b.details.privacy.clone());
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_channel_wins_over_the_saved_details() {
        let saved = "saved".to_owned();
        assert_eq!(
            prefill(Ok("now".to_owned()), Some(&saved)),
            Ok("now".into())
        );
        assert_eq!(
            prefill(Err("offline".into()), Some(&saved)),
            Ok("saved".into())
        );
        assert_eq!(
            prefill::<String>(Err("offline".into()), None),
            Err("offline".into())
        );
    }

    #[test]
    fn tags_are_split_on_commas() {
        assert_eq!(split_tags("a, b ,, c"), vec!["a", "b", "c"]);
        assert!(split_tags("  ").is_empty());
    }

    #[test]
    fn an_unknown_language_is_kept() {
        assert_eq!(languages("en").len(), LANGUAGES.len());
        let more = languages("fi");
        assert_eq!(more.last().unwrap().0, "fi");
    }

    #[test]
    fn searches_wait_for_typing_to_pause() {
        let mut fresh = Some(("gta".to_owned(), Instant::now()));
        assert_eq!(due(&mut fresh), None);
        assert!(fresh.is_some());
        let mut old = Some(("gta".to_owned(), Instant::now() - SEARCH_DELAY));
        assert_eq!(due(&mut old), Some("gta".into()));
        assert!(old.is_none());
    }

    #[test]
    fn broadcasts_are_labelled() {
        let b = Broadcast {
            id: "b".into(),
            details: BroadcastDetails {
                title: "Ranked".into(),
                privacy: "unlisted".into(),
                ..Default::default()
            },
            scheduled: String::new(),
            status: "live".into(),
            stream: None,
        };
        assert_eq!(broadcast_label(&b), "Ranked · live now · unlisted");
    }

    /// Kick and Twitch connected (with made-up accounts), YouTube not.
    fn sessions() -> Sessions {
        Sessions {
            youtube: crate::youtube::Session::new(None),
            twitch: crate::twitch::Session::new(Some(Default::default())),
            kick: crate::kick::Session::new(Some(Default::default())),
        }
    }

    fn kick(title: &str, category: &str) -> crate::kick::ChannelDetails {
        crate::kick::ChannelDetails {
            title: title.into(),
            category_id: 7,
            category: category.into(),
        }
    }

    fn kick_destination() -> crate::settings::DestinationConfig {
        let mut d = crate::settings::DestinationConfig::default();
        d.stream.service = Service::Kick;
        d.details.kick = Some(kick("Apex with friends", "Apex Legends"));
        d.keep_setup_details(
            "PUBG",
            StreamDetails {
                kick: Some(kick("PUBG squads", "PUBG: Battlegrounds")),
                ..Default::default()
            },
        );
        d
    }

    fn kick_form(w: &Window) -> crate::kick::ChannelDetails {
        match &w.sections[0].form {
            Form::Kick { details, .. } => details.clone(),
            other => panic!("not Kick: {other:?}"),
        }
    }

    #[test]
    fn the_setup_details_fill_the_window_without_asking_the_channel() {
        let d = kick_destination();
        let w = Window::open(&[(0, &d)], sessions(), "PUBG");
        assert_eq!(w.sections.len(), 1);
        // Ready at once: the channel (still showing the last game) isn't read.
        assert_eq!(w.sections[0].state, SectionState::Ready);
        assert_eq!(kick_form(&w), kick("PUBG squads", "PUBG: Battlegrounds"));
        assert_eq!(
            w.to_save(),
            vec![(
                0,
                StreamDetails {
                    kick: Some(kick("PUBG squads", "PUBG: Battlegrounds")),
                    ..Default::default()
                }
            )]
        );

        let w = Window::saving(&[(0, &d)], sessions(), "PUBG");
        assert!(w.saving && !w.live);
        assert_eq!(kick_form(&w), kick("PUBG squads", "PUBG: Battlegrounds"));
    }

    #[test]
    fn without_the_window_the_setup_details_come_first_then_the_last_ones() {
        let d = kick_destination();
        let w = Window::remembered(&[(0, &d)], sessions(), "PUBG");
        assert_eq!(kick_form(&w), kick("PUBG squads", "PUBG: Battlegrounds"));
        // A setup without its own details uses the ones used last.
        let w = Window::remembered(&[(0, &d)], sessions(), "Apex");
        assert_eq!(w.sections[0].state, SectionState::Ready);
        assert_eq!(kick_form(&w), kick("Apex with friends", "Apex Legends"));
        // Nothing saved at all: Kick is left as it is.
        let mut empty = kick_destination();
        empty.details = StreamDetails::default();
        empty.setup_details.clear();
        let w = Window::remembered(&[(0, &empty)], sessions(), "Apex");
        assert!(!w.has_sections());
    }

    #[test]
    fn a_waiting_broadcast_gets_what_the_setup_says_viewers_see() {
        let waiting = BroadcastDetails {
            title: "Apex with friends".into(),
            description: "Apex".into(),
            latency: Latency::UltraLow,
            auto_start: false,
            ..Default::default()
        };
        let setup = BroadcastDetails {
            title: "PUBG squads".into(),
            description: "PUBG".into(),
            privacy: "unlisted".into(),
            category: "20".into(),
            ..Default::default()
        };
        let both = with_setup_details(&waiting, &setup);
        assert_eq!(both.title, "PUBG squads");
        assert_eq!(both.description, "PUBG");
        assert_eq!(both.privacy, "unlisted");
        assert_eq!(both.category, "20");
        // How it runs is the broadcast's.
        assert_eq!(both.latency, Latency::UltraLow);
        assert!(!both.auto_start);
    }
}
