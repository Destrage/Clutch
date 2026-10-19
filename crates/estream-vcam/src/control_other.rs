//! The camera controls on systems without a virtual camera backend yet: same API as the
//! Windows controls, reporting the camera as unsupported.

use std::path::{Path, PathBuf};

/// Why the camera can't be used on this system.
pub const UNSUPPORTED: &str = "isn't available on this system yet";

/// Whether the camera source is installed for this copy of Clutch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Registration {
    Installed,
    Missing,
    /// Installed, but pointing at another copy of the camera component.
    Elsewhere(PathBuf),
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("the permission prompt was declined")]
    Declined,
    #[error("installing the camera failed: {0}")]
    Failed(String),
}

pub fn dll_path() -> Option<PathBuf> {
    None
}

pub fn registration() -> Registration {
    Registration::Missing
}

pub fn install(_: &Path) -> Result<(), InstallError> {
    Err(InstallError::Failed(UNSUPPORTED.into()))
}

pub fn supported() -> bool {
    false
}
