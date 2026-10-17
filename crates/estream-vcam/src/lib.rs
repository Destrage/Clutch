//! eStream's virtual camera (Windows 11, Media Foundation).
//!
//! The crate builds two things:
//! - `estream_vcam.dll`: the camera source that Windows' Frame Server loads (COM, registered
//!   once for all users);
//! - a library for eStream: the shared-memory protocol ([`shared`]) and the controls that
//!   install the source and turn the camera on and off ([`control`]).

pub mod shared;

#[cfg(windows)]
pub mod control;
#[cfg(windows)]
mod dll;
#[cfg(windows)]
pub mod source;

/// The camera's name in apps.
pub const FRIENDLY_NAME: &str = "eStream Virtual Camera";

/// COM class of the camera source.
#[cfg(windows)]
pub const CLSID: windows::core::GUID =
    windows::core::GUID::from_u128(0x8a5f2e61_3c4b_4d7e_9b1a_6e2d0c7f5a93);
pub const CLSID_STRING: &str = "{8A5F2E61-3C4B-4D7E-9B1A-6E2D0C7F5A93}";

/// Where the installed copy of the camera source lives (`%ProgramData%\eStream\VirtualCamera`).
/// Frame Server loads that copy, so rebuilding or updating eStream never fights a locked file.
/// It also holds `vcam.log`, which the source writes from inside Frame Server.
pub fn install_dir() -> std::path::PathBuf {
    std::env::var_os("ProgramData")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("C:\\ProgramData"))
        .join("eStream")
        .join("VirtualCamera")
}

/// Identifies compatible builds: eStream's version and the shared memory protocol. Installed
/// copies carry it in their file name; a copy with the same tag counts as installed.
pub fn build_tag() -> String {
    format!("{}p{}", env!("CARGO_PKG_VERSION"), shared::VERSION)
}

/// FNV-1a of a file's bytes: keeps installed copies of different builds apart.
pub fn content_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Appends a line to `vcam.log` in [`install_dir`] (kept under 1 MB), for diagnosing the
/// source inside Frame Server.
pub fn log(message: &str) {
    use std::io::Write;
    let path = install_dir().join("vcam.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1 << 20) {
        let _ = std::fs::remove_file(&path);
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    let _ = writeln!(file, "{secs:.3} [{}] {message}", std::process::id());
}
