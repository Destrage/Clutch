//! The virtual camera on Linux: a `v4l2loopback` device Clutch writes YUYV frames to, which
//! webcam apps open like any camera. There is nothing to install with Clutch; the kernel
//! module comes from the system, and starting the camera explains how to load it.

use std::io::Write;
use std::os::fd::{FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use v4l::video::Output;

/// How to get the loopback device (shown when there is none).
pub const UNSUPPORTED: &str = "needs the v4l2loopback kernel module. Install it (Ubuntu: \
    sudo apt install v4l2loopback-dkms) and load it with: sudo modprobe v4l2loopback \
    card_label=\"Clutch\" exclusive_caps=1";

/// Whether a loopback device is there for the camera.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Registration {
    Installed,
    Missing,
    /// Installed, but pointing at another copy of the camera component (Windows only).
    Elsewhere(PathBuf),
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("the permission prompt was declined")]
    Declined,
    #[error("the virtual camera {0}")]
    Failed(String),
}

/// Linux has no camera component to install.
pub fn dll_path() -> Option<PathBuf> {
    None
}

pub fn registration() -> Registration {
    if loopback_device().is_some() {
        Registration::Installed
    } else {
        Registration::Missing
    }
}

pub fn install(_: &Path) -> Result<(), InstallError> {
    Err(InstallError::Failed(UNSUPPORTED.into()))
}

pub fn supported() -> bool {
    true
}

/// The loopback device to write to: the one labelled "Clutch" if there is one, else the
/// first (they are the `video4linux` devices with no hardware behind them).
pub fn loopback_device() -> Option<PathBuf> {
    let mut found: Vec<(bool, u32, PathBuf)> = std::fs::read_dir("/sys/class/video4linux")
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let index: u32 = name.strip_prefix("video")?.parse().ok()?;
            let real = std::fs::canonicalize(e.path()).ok()?;
            if !real.to_string_lossy().contains("/devices/virtual/") {
                return None;
            }
            let label = std::fs::read_to_string(e.path().join("name")).unwrap_or_default();
            Some((
                label.trim() == "Clutch",
                index,
                PathBuf::from("/dev").join(name),
            ))
        })
        .collect();
    found.sort_by_key(|(clutch, index, _)| (!clutch, *index));
    found.into_iter().next().map(|(_, _, path)| path)
}

/// An open loopback device taking frames of one size.
pub struct Loopback {
    out: std::fs::File,
    // Keeps the device (and the format set on it) open.
    _device: v4l::Device,
    pub width: u32,
    pub height: u32,
    yuyv: Vec<u8>,
}

impl Loopback {
    /// Opens the loopback device for `width`×`height` YUYV (both even).
    pub fn open(width: u32, height: u32) -> Result<Self, String> {
        let path = loopback_device().ok_or_else(|| UNSUPPORTED.to_owned())?;
        let device = v4l::Device::with_path(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::PermissionDenied => format!(
                "no permission to open {} (are you in the “video” group?)",
                path.display()
            ),
            _ => format!("couldn't open {}: {e}", path.display()),
        })?;
        let mut format = Output::format(&device).map_err(|e| e.to_string())?;
        format.width = width & !1;
        format.height = height & !1;
        format.fourcc = v4l::FourCC::new(b"YUYV");
        format.stride = format.width * 2;
        format.size = format.stride * format.height;
        let format = Output::set_format(&device, &format).map_err(|e| match e.raw_os_error() {
            Some(16) => "another app is already sending video to the virtual camera".to_owned(),
            _ => format!("couldn't set the camera format: {e}"),
        })?;
        // Frames go in with write() on the same file, which makes Clutch the producer.
        // SAFETY: `dup` gives a new descriptor we own; the device keeps the original.
        let fd = unsafe { libc::dup(device.handle().fd()) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        // SAFETY: `fd` is a fresh descriptor nothing else owns.
        let out = std::fs::File::from(unsafe { OwnedFd::from_raw_fd(fd) });
        tracing::info!(device = %path.display(), format.width, format.height, "virtual camera device open");
        Ok(Self {
            out,
            _device: device,
            width: format.width,
            height: format.height,
            yuyv: Vec::new(),
        })
    }

    /// Writes one NV12 frame of the device's size.
    pub fn write_nv12(&mut self, nv12: &[u8]) -> std::io::Result<()> {
        nv12_to_yuyv(nv12, self.width, self.height, &mut self.yuyv);
        self.out.write_all(&self.yuyv)
    }
}

/// Tight NV12 → YUYV (each chroma row serves the two luma rows it covers).
pub fn nv12_to_yuyv(nv12: &[u8], width: u32, height: u32, out: &mut Vec<u8>) {
    let (w, h) = (width as usize, height as usize);
    out.resize(w * h * 2, 0);
    if nv12.len() < w * h * 3 / 2 {
        return;
    }
    let (y, uv) = nv12.split_at(w * h);
    for (row, dst) in out.chunks_exact_mut(w * 2).enumerate() {
        let luma = &y[row * w..row * w + w];
        let chroma = &uv[row / 2 * w..row / 2 * w + w];
        for ((d, l), c) in dst
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(luma.as_chunks::<2>().0)
            .zip(chroma.as_chunks::<2>().0)
        {
            *d = [l[0], c[0], l[1], c[1]];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nv12_becomes_yuyv() {
        // 2×2: Y = 1 2 / 3 4, CbCr = 9 8.
        let mut out = Vec::new();
        nv12_to_yuyv(&[1, 2, 3, 4, 9, 8], 2, 2, &mut out);
        assert_eq!(out, [1, 9, 2, 8, 3, 9, 4, 8]);
    }
}
