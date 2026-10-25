//! Hands files and links to the desktop: open them, show them in the file manager, move them to
//! the trash and copy them for pasting. Windows uses the shell; Linux the freedesktop tools.

use std::io;
use std::path::Path;
use std::process::Command;

/// What the system calls its trash.
pub const TRASH: &str = if cfg!(windows) {
    "Recycle Bin"
} else {
    "Trash"
};

/// Opens a web page in the default browser.
pub fn open_url(url: &str) -> io::Result<()> {
    // Explorer opens Documents for links with a query string; the shell opens any link.
    #[cfg(windows)]
    {
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        use windows::core::{HSTRING, w};
        // SAFETY: both strings outlive the call; no window handle is passed.
        let result = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                &HSTRING::from(url),
                None,
                None,
                SW_SHOWNORMAL,
            )
        };
        // Values above 32 mean success.
        if result.0 as usize > 32 {
            return Ok(());
        }
        Err(io::Error::last_os_error())
    }
    #[cfg(not(windows))]
    {
        spawn(Command::new(OPENER).arg(url))
    }
}

/// Opens a file in its default app, or a folder in the file manager.
pub fn open(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        spawn(Command::new("explorer").arg(path))
    }
    #[cfg(not(windows))]
    {
        spawn(Command::new(OPENER).arg(path))
    }
}

/// Opens the file manager with `path` selected.
pub fn reveal(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        spawn(Command::new("explorer").arg(format!("/select,{}", path.display())))
    }
    #[cfg(target_os = "macos")]
    {
        spawn(Command::new("open").arg("-R").arg(path))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        // The FileManager1 D-Bus interface selects the file (Nautilus, Dolphin, Nemo, Thunar…).
        // It can take a moment to start the file manager, so it runs off the UI thread and
        // falls back to opening the folder.
        let uri = file_uri(path);
        let folder = path.parent().unwrap_or(path).to_owned();
        std::thread::Builder::new()
            .name("estream-reveal".into())
            .spawn(move || {
                let shown = Command::new("dbus-send")
                    .args([
                        "--session",
                        "--print-reply",
                        "--dest=org.freedesktop.FileManager1",
                        "/org/freedesktop/FileManager1",
                        "org.freedesktop.FileManager1.ShowItems",
                    ])
                    .arg(format!("array:string:{uri}"))
                    .arg("string:")
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success());
                if !shown && let Err(e) = open(&folder) {
                    tracing::warn!(error = %e, "couldn't open the folder");
                }
            })
            .map(|_| ())
    }
}

/// Moves a file to the trash, so it can still be restored.
pub fn trash(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::UI::Shell::{
            FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT,
            SHFILEOPSTRUCTW, SHFileOperationW,
        };
        // A double-NUL-terminated list of one path.
        let from: Vec<u16> = path.as_os_str().encode_wide().chain([0, 0]).collect();
        let mut op = SHFILEOPSTRUCTW {
            wFunc: FO_DELETE,
            pFrom: windows::core::PCWSTR(from.as_ptr()),
            fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT).0 as u16,
            ..Default::default()
        };
        // SAFETY: `from` outlives the call and is double-NUL terminated.
        match unsafe { SHFileOperationW(&mut op) } {
            0 => Ok(()),
            code => Err(io::Error::other(format!("Windows error {code}"))),
        }
    }
    #[cfg(not(windows))]
    {
        ::trash::delete(path).map_err(io::Error::other)
    }
}

/// Puts `text` on the clipboard.
pub fn copy_text(text: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::System::Ole::CF_UNICODETEXT;
        // UTF-16 with its terminating NUL.
        let bytes: Vec<u8> = std::ffi::OsStr::new(text)
            .encode_wide()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect();
        set_clipboard(CF_UNICODETEXT.0 as u32, &bytes)
    }
    #[cfg(not(windows))]
    {
        use std::io::Write;
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
        let (tool, args, package): (&str, &[&str], &str) = if wayland {
            ("wl-copy", &[], "wl-clipboard")
        } else {
            ("xclip", &["-selection", "clipboard"], "xclip")
        };
        let mut child = Command::new(tool)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .map_err(|_| format!("copying needs {package} installed"))?;
        child
            .stdin
            .take()
            .ok_or("the clipboard tool didn't start")?
            .write_all(text.as_bytes())
            .map_err(|e| e.to_string())?;
        // Both tools stay in the background to serve the clipboard; don't wait for them.
        Ok(())
    }
}

/// Replaces the clipboard's contents with `data` in `format`.
#[cfg(windows)]
fn set_clipboard(format: u32, data: &[u8]) -> Result<(), String> {
    use windows::Win32::Foundation::{HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};

    // SAFETY: the clipboard is opened and closed here. The global block is sized for `data`
    // and written while locked; the clipboard owns it once SetClipboardData succeeds.
    unsafe {
        OpenClipboard(None).map_err(|e| e.to_string())?;
        let result = (|| -> Result<(), String> {
            EmptyClipboard().map_err(|e| e.to_string())?;
            let mem: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, data.len()).map_err(|e| e.to_string())?;
            let ptr = GlobalLock(mem).cast::<u8>();
            if ptr.is_null() {
                return Err("couldn't lock clipboard memory".into());
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
            let _ = GlobalUnlock(mem);
            SetClipboardData(format, Some(HANDLE(mem.0)))
                .map(|_| ())
                .map_err(|e| e.to_string())
        })();
        let _ = CloseClipboard();
        result
    }
}

/// Puts `path` on the clipboard as a file, so it pastes into the file manager and chats.
pub fn copy_file(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        copy_file_windows(path)
    }
    #[cfg(not(windows))]
    {
        use std::io::Write;
        // A text/uri-list through the desktop's clipboard tool (Slint's clipboard is text only).
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
        let (tool, args, package): (&str, &[&str], &str) = if wayland {
            ("wl-copy", &["--type", "text/uri-list"], "wl-clipboard")
        } else {
            (
                "xclip",
                &["-selection", "clipboard", "-t", "text/uri-list"],
                "xclip",
            )
        };
        let mut child = Command::new(tool)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .map_err(|_| format!("copying files needs {package} installed"))?;
        let uri = format!("{}\r\n", file_uri(path));
        child
            .stdin
            .take()
            .ok_or("the clipboard tool didn't start")?
            .write_all(uri.as_bytes())
            .map_err(|e| e.to_string())?;
        // Both tools stay in the background to serve the clipboard; don't wait for them.
        Ok(())
    }
}

#[cfg(windows)]
fn copy_file_windows(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::System::Ole::CF_HDROP;
    use windows::Win32::UI::Shell::DROPFILES;

    // DROPFILES, then the path as UTF-16, then two NULs (the end of a one-file list).
    let header = size_of::<DROPFILES>();
    let drop = DROPFILES {
        pFiles: header as u32,
        fWide: true.into(),
        ..Default::default()
    };
    // SAFETY: DROPFILES is a plain C struct; its bytes are read for its size only.
    let mut data =
        unsafe { std::slice::from_raw_parts((&raw const drop).cast::<u8>(), header) }.to_vec();
    for unit in path.as_os_str().encode_wide().chain([0, 0]) {
        data.extend_from_slice(&unit.to_le_bytes());
    }
    set_clipboard(CF_HDROP.0 as u32, &data)
}

#[cfg(not(windows))]
const OPENER: &str = if cfg!(target_os = "macos") {
    "open"
} else {
    "xdg-open"
};

fn spawn(cmd: &mut Command) -> io::Result<()> {
    cmd.spawn().map(|_| ())
}

/// `file://` URI of an absolute path, with the bytes a URI (or a `dbus-send` list) can't hold
/// percent-encoded.
#[cfg_attr(windows, allow(dead_code))]
fn file_uri(path: &Path) -> String {
    use percent_encoding::{AsciiSet, CONTROLS, percent_encode};
    const ESCAPE: &AsciiSet = &CONTROLS
        .add(b' ')
        .add(b'"')
        .add(b'#')
        .add(b'%')
        .add(b',')
        .add(b'<')
        .add(b'>')
        .add(b'?')
        .add(b'[')
        .add(b']')
        .add(b'`')
        .add(b'{')
        .add(b'}');
    #[cfg(unix)]
    let bytes = std::os::unix::ffi::OsStrExt::as_bytes(path.as_os_str());
    #[cfg(not(unix))]
    let bytes = path.to_string_lossy().into_owned().into_bytes();
    #[cfg(not(unix))]
    let bytes = bytes.as_slice();
    format!("file://{}", percent_encode(bytes, ESCAPE))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn file_uris_escape_spaces_commas_and_unicode() {
        assert_eq!(
            file_uri(Path::new("/home/me/Videos/Clip, #1 é.mp4")),
            "file:///home/me/Videos/Clip%2C%20%231%20%C3%A9.mp4"
        );
    }
}
