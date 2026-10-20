//! Chat panel: one tab per destination that has a chat, each showing the platform's own
//! pop-out chat page in an interactive browser page. Sign-ins live in the browser profile.

use estream_capture::media::browser::{self, PanelPage, Pointer, mods};
use estream_media::Service;

/// The pop-out chat page for a destination, or why there is none.
pub fn chat_url(service: Service, channel: &str) -> Result<String, &'static str> {
    let channel = channel.trim();
    match service {
        Service::Kick => {
            let name = channel_name(channel, "kick.com")
                .ok_or("Add your Kick channel name in Settings → Stream")?;
            Ok(format!("https://kick.com/popout/{name}/chat"))
        }
        Service::Twitch => {
            let name = channel_name(channel, "twitch.tv")
                .ok_or("Add your Twitch channel name in Settings → Stream")?;
            Ok(format!("https://www.twitch.tv/popout/{name}/chat?popout="))
        }
        Service::YouTube => {
            let id = youtube_video_id(channel)
                .ok_or(
                    "Connect YouTube in Settings → Stream and your live chat opens here when you go live, or paste your live link there",
                )?;
            Ok(format!(
                "https://www.youtube.com/live_chat?is_popout=1&v={id}"
            ))
        }
        _ => Err("This platform has no chat in Clutch"),
    }
}

/// A Kick channel's slug from its name or link.
pub fn kick_slug(text: &str) -> Option<String> {
    channel_name(text, "kick.com")
}

/// A Twitch login from a channel name or link.
pub fn twitch_login(text: &str) -> Option<String> {
    channel_name(text, "twitch.tv")
}

/// A channel name from a bare name or a channel link on `host`.
fn channel_name(text: &str, host: &str) -> Option<String> {
    let text = text.trim().trim_end_matches('/');
    let name = match text.find(host) {
        Some(i) => text[i + host.len()..]
            .trim_start_matches('/')
            .split(['/', '?', '#'])
            .next()?,
        None => text.trim_start_matches('@'),
    };
    let ok = !name.is_empty()
        && name.len() <= 40
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    ok.then(|| name.to_ascii_lowercase())
}

/// The video ID from a YouTube watch, live or short link, or a bare 11-character ID.
fn youtube_video_id(text: &str) -> Option<String> {
    let text = text.trim();
    let valid = |id: &str| {
        id.len() == 11
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    };
    let candidate = if let Some(i) = text.find("v=") {
        text[i + 2..].split(['&', '#']).next()
    } else if let Some(i) = text.find("youtu.be/") {
        text[i + 9..].split(['?', '&', '#', '/']).next()
    } else if let Some(i) = text.find("/live/") {
        text[i + 6..].split(['?', '&', '#', '/']).next()
    } else {
        Some(text)
    };
    candidate.filter(|id| valid(id)).map(str::to_owned)
}

/// A chat tab: the destination it belongs to and its page (made when first shown).
pub struct ChatTab {
    pub name: String,
    pub service: Service,
    /// The chat page, or why there is none.
    pub url: Result<String, &'static str>,
    pub page: Option<PanelPage>,
}

/// The tabs, the selected one and the size the page is drawn at.
#[derive(Default)]
pub struct ChatUi {
    pub tabs: Vec<ChatTab>,
    pub selected: usize,
    pub size: (u32, u32),
    /// The left button is held (drags select text).
    held: bool,
    /// Bumped when the tabs are rebuilt, so page ids stay unique.
    generation: u32,
}

impl ChatUi {
    /// Rebuilds the tabs from the destinations (`(name, service, channel)` of each enabled one),
    /// keeping pages whose address didn't change.
    pub fn set_destinations(&mut self, dests: Vec<(String, Service, String)>) {
        let mut old: Vec<ChatTab> = std::mem::take(&mut self.tabs);
        self.generation += 1;
        for (name, service, channel) in dests {
            let url = chat_url(service, &channel);
            if url.is_err() && matches!(service, Service::Custom | Service::Facebook) {
                continue;
            }
            let reuse = old
                .iter()
                .position(|t| t.service == service && t.url == url)
                .map(|i| old.remove(i));
            self.tabs.push(ChatTab {
                name,
                service,
                url,
                page: reuse.and_then(|t| t.page),
            });
        }
        if self.selected >= self.tabs.len() {
            self.selected = 0;
        }
    }

    pub fn current(&mut self) -> Option<&mut ChatTab> {
        self.tabs.get_mut(self.selected)
    }

    /// Opens (or resizes) the selected tab's page.
    pub fn show_current(&mut self) {
        let (w, h) = self.size;
        let generation = self.generation;
        let selected = self.selected;
        let Some(tab) = self.tabs.get_mut(selected) else {
            return;
        };
        let Ok(url) = &tab.url else { return };
        let page = tab
            .page
            .get_or_insert_with(|| PanelPage::new(format!("panel:chat:{generation}:{selected}")));
        page.show(url, w.max(200), h.max(200));
    }

    pub fn page(&mut self) -> Option<&mut PanelPage> {
        self.tabs.get_mut(self.selected)?.page.as_mut()
    }

    /// Reloads the open chat pages of a platform (after signing in to it).
    pub fn reload_service(&self, service: Service) {
        for tab in &self.tabs {
            if tab.service == service
                && let Some(page) = &tab.page
            {
                page.reload();
            }
        }
    }

    pub fn pointer(&mut self, kind: &str, x: f32, y: f32, button: i32, dx: f32, dy: f32) {
        let mut held = self.held;
        hold(&mut held, kind, button);
        self.held = held;
        if let Some(page) = self.page() {
            send_pointer(page.id(), held, kind, x, y, button, dx, dy);
        }
    }

    /// A key from the UI: `text` as Slint reports it.
    pub fn key(&mut self, text: &str, down: bool, ctrl: bool, shift: bool, alt: bool) {
        if let Some(page) = self.page() {
            send_key(page.id(), text, down, ctrl, shift, alt);
        }
    }
}

/// A page shown in a dialog (signing in to a platform): its input and the size it is drawn at.
pub struct WebDialog {
    page: PanelPage,
    url: String,
    held: bool,
    size: (u32, u32),
    shown: bool,
}

impl WebDialog {
    pub fn new(id: &str, url: &str) -> Self {
        Self {
            page: PanelPage::new(id.to_owned()),
            url: url.to_owned(),
            held: false,
            size: (0, 0),
            shown: false,
        }
    }

    /// The dialog's page area changed size: draw the page at exactly that size.
    pub fn resized(&mut self, width: f32, height: f32) {
        let size = (
            width.max(0.0).round() as u32,
            height.max(0.0).round() as u32,
        );
        if size != self.size && size.0 > 0 && size.1 > 0 {
            self.size = size;
            self.page.show(&self.url, size.0, size.1);
        }
    }

    pub fn pointer(&mut self, kind: &str, x: f32, y: f32, button: i32, dx: f32, dy: f32) {
        hold(&mut self.held, kind, button);
        send_pointer(self.page.id(), self.held, kind, x, y, button, dx, dy);
    }

    pub fn key(&mut self, text: &str, down: bool, ctrl: bool, shift: bool, alt: bool) {
        send_key(self.page.id(), text, down, ctrl, shift, alt);
    }

    /// The newest frame (if one arrived), and what to show instead of the page ("" for none).
    pub fn frame(&mut self) -> (Option<slint::Image>, String) {
        let image = self.page.latest().map(|(width, height, rgba)| {
            let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                rgba, width, height,
            );
            slint::Image::from_rgba8(buffer)
        });
        self.shown |= image.is_some();
        let message = match self.page.error() {
            Some(e) => format!("The page didn't load: {e}"),
            None if !self.shown => "Loading…".to_owned(),
            None => String::new(),
        };
        (image, message)
    }
}

/// Tracks whether the left button is held (drags select text).
pub fn hold(held: &mut bool, kind: &str, button: i32) {
    match kind {
        "down" if button == 0 => *held = true,
        "up" if button == 0 => *held = false,
        "leave" => *held = false,
        _ => {}
    }
}

/// A pointer event from the UI, in page pixels.
#[allow(clippy::too_many_arguments)]
pub fn send_pointer(
    page: &str,
    held: bool,
    kind: &str,
    x: f32,
    y: f32,
    button: i32,
    dx: f32,
    dy: f32,
) {
    browser::host().mouse(
        page,
        Pointer {
            kind,
            x: x.round() as i32,
            y: y.round() as i32,
            button: button.clamp(0, 2) as u8,
            clicks: 1,
            dx: dx.round() as i32,
            dy: dy.round() as i32,
            mods: if held { mods::LEFT_BUTTON } else { 0 },
        },
    );
}

/// A key from the UI: `text` as Slint reports it.
pub fn send_key(page: &str, text: &str, down: bool, ctrl: bool, shift: bool, alt: bool) {
    let mut flags = 0;
    if ctrl {
        flags |= mods::CONTROL;
    }
    if shift {
        flags |= mods::SHIFT;
    }
    if alt {
        flags |= mods::ALT;
    }
    let (code, ch) = key_code(text, ctrl);
    if code != 0 {
        browser::host().key(
            page,
            if down { "down" } else { "up" },
            code,
            ch.unwrap_or(0),
            flags,
        );
    }
    // Text goes in as characters (not with Ctrl/Alt shortcuts).
    if down
        && !ctrl
        && !alt
        && let Some(c) = ch
        && (c >= 0x20 || c == 0x0d)
    {
        browser::host().key(page, "char", code, c, flags);
    }
}

/// The Windows virtual-key code and the character for a key as Slint reports it.
fn key_code(text: &str, ctrl: bool) -> (i32, Option<u32>) {
    use slint::platform::Key as K;
    let is = |k: K| text == slint::SharedString::from(k).as_str();
    let special = [
        (K::Backspace, 0x08, None),
        (K::Tab, 0x09, None),
        (K::Return, 0x0d, Some(0x0d)),
        (K::Escape, 0x1b, None),
        (K::Delete, 0x2e, None),
        (K::LeftArrow, 0x25, None),
        (K::UpArrow, 0x26, None),
        (K::RightArrow, 0x27, None),
        (K::DownArrow, 0x28, None),
        (K::Home, 0x24, None),
        (K::End, 0x23, None),
        (K::PageUp, 0x21, None),
        (K::PageDown, 0x22, None),
        // Modifier keys arrive as control characters too (Shift is U+0010); they type nothing.
        (K::Shift, 0x10, None),
        (K::ShiftR, 0x10, None),
        (K::Control, 0x11, None),
        (K::ControlR, 0x11, None),
        (K::Alt, 0x12, None),
        (K::AltGr, 0x12, None),
        (K::CapsLock, 0x14, None),
        (K::Meta, 0x5b, None),
        (K::MetaR, 0x5c, None),
    ];
    for (key, code, ch) in special {
        if is(key) {
            return (code, ch);
        }
    }
    let mut chars = text.chars();
    let Some(c) = chars.next() else {
        return (0, None);
    };
    if chars.next().is_some() {
        return (0, None);
    }
    // Ctrl+letter may arrive as an ASCII control character.
    let c = match c as u32 {
        n @ 1..=26 if ctrl => char::from(b'a' + n as u8 - 1),
        // Other control and private-use characters (function keys, …) type nothing.
        0..=0x1f | 0xf700..=0xf8ff => return (0, None),
        _ => c,
    };
    let code = match c {
        'a'..='z' | 'A'..='Z' => c.to_ascii_uppercase() as i32,
        '0'..='9' => c as i32,
        ' ' => 0x20,
        _ => 0,
    };
    (code, Some(c as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_links() {
        assert_eq!(
            chat_url(Service::Kick, "https://kick.com/SomeOne/").as_deref(),
            Ok("https://kick.com/popout/someone/chat")
        );
        assert_eq!(
            chat_url(Service::Twitch, "@my_channel").as_deref(),
            Ok("https://www.twitch.tv/popout/my_channel/chat?popout=")
        );
        assert_eq!(
            chat_url(
                Service::YouTube,
                "https://www.youtube.com/live/abcDEF12345?si=x"
            )
            .as_deref(),
            Ok("https://www.youtube.com/live_chat?is_popout=1&v=abcDEF12345")
        );
        assert_eq!(
            chat_url(Service::YouTube, "https://youtu.be/abcDEF12345").as_deref(),
            Ok("https://www.youtube.com/live_chat?is_popout=1&v=abcDEF12345")
        );
        assert!(chat_url(Service::YouTube, "@mychannel").is_err());
        assert!(chat_url(Service::Kick, "").is_err());
        assert!(chat_url(Service::Kick, "bad name!").is_err());
    }

    #[test]
    fn keys_map_to_windows_codes() {
        use slint::platform::Key;
        let key = |k: Key| slint::SharedString::from(k);
        assert_eq!(key_code("a", false), (0x41, Some('a' as u32)));
        assert_eq!(key_code("\u{1}", true), (0x41, Some('a' as u32))); // Ctrl+A
        assert_eq!(key_code("!", false), (0, Some('!' as u32)));
        assert_eq!(key_code(&key(Key::Backspace), false), (0x08, None));
        // Shift and the Windows key are modifiers, not Ctrl+P / Ctrl+W.
        assert_eq!(key_code(&key(Key::Shift), false), (0x10, None));
        assert_eq!(key_code(&key(Key::Meta), false), (0x5b, None));
        assert_eq!(key_code(&key(Key::F1), false), (0, None));
    }
}
