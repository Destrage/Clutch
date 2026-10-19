//! Used by Clutch: installs the camera source (once, elevated) and turns the session camera
//! on and off with `MFCreateVirtualCamera`.

use crate::{CLSID_STRING, FRIENDLY_NAME};
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, WAIT_OBJECT_0};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
use windows::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
use windows::Win32::UI::Shell::{
    SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::{HSTRING, PCWSTR};

/// Whether the camera source is installed for this copy of Clutch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Registration {
    Installed,
    Missing,
    /// Installed, but pointing at another copy of the DLL.
    Elsewhere(PathBuf),
}

/// `estream_vcam.dll` next to the running executable.
pub fn dll_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [
        dir.join("estream_vcam.dll"),
        dir.parent()?.join("estream_vcam.dll"),
    ]
    .into_iter()
    .find(|p| p.exists())
}

/// The DLL path registered for the camera CLSID (all users), if any.
pub fn registered_path() -> Option<PathBuf> {
    let key = HSTRING::from(format!(
        "SOFTWARE\\Classes\\CLSID\\{CLSID_STRING}\\InprocServer32"
    ));
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: reads a string value into our buffer.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            &key,
            PCWSTR::null(),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status.is_err() {
        return None;
    }
    let len = (size as usize / 2).saturating_sub(1).min(buf.len());
    Some(PathBuf::from(String::from_utf16_lossy(&buf[..len])))
}

/// Installed means a copy of a compatible build (same [`crate::build_tag`]) is registered and
/// present (installs copy the DLL into [`crate::install_dir`]).
pub fn registration() -> Registration {
    let tag = format!("estream_vcam-{}-", crate::build_tag());
    match registered_path() {
        None => Registration::Missing,
        Some(p)
            if p.exists()
                && p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with(&tag)) =>
        {
            Registration::Installed
        }
        Some(p) => Registration::Elsewhere(p),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("the permission prompt was declined")]
    Declined,
    #[error("installing the camera failed: {0}")]
    Failed(String),
}

/// Registers `dll` for all users with `regsvr32`, showing the system's elevation prompt.
/// Blocks until it finishes (run it off the UI thread).
pub fn install(dll: &Path) -> Result<(), InstallError> {
    let verb = HSTRING::from("runas");
    let file = HSTRING::from("regsvr32.exe");
    let params = HSTRING::from(format!("/s \"{}\"", dll.display()));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    // SAFETY: valid structure with live strings.
    if let Err(e) = unsafe { ShellExecuteExW(&mut info) } {
        return Err(
            if e.code() == windows::core::HRESULT::from_win32(ERROR_CANCELLED.0) {
                InstallError::Declined
            } else {
                InstallError::Failed(e.message())
            },
        );
    }
    let mut code = 1u32;
    // SAFETY: we own the process handle (SEE_MASK_NOCLOSEPROCESS).
    unsafe {
        if WaitForSingleObject(info.hProcess, INFINITE) == WAIT_OBJECT_0 {
            let _ = GetExitCodeProcess(info.hProcess, &mut code);
        }
        let _ = CloseHandle(info.hProcess);
    }
    if code == 0 {
        Ok(())
    } else {
        Err(InstallError::Failed(format!(
            "regsvr32 exited with code {code}"
        )))
    }
}

type CreateVirtualCamera = unsafe extern "system" fn(
    MFVirtualCameraType,
    MFVirtualCameraLifetime,
    MFVirtualCameraAccess,
    PCWSTR,
    PCWSTR,
    *const windows::core::GUID,
    u32,
    *mut Option<IMFVirtualCamera>,
) -> windows::core::HRESULT;

/// `MFCreateVirtualCamera`, looked up at run time: it only exists on Windows 11, and a
/// static import would stop Clutch from starting on Windows 10.
fn create_function() -> Option<CreateVirtualCamera> {
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    // SAFETY: loads a system DLL and looks up an export with the signature above.
    unsafe {
        let module = LoadLibraryW(&HSTRING::from("mfsensorgroup.dll")).ok()?;
        let f = GetProcAddress(module, windows::core::s!("MFCreateVirtualCamera"))?;
        Some(std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            CreateVirtualCamera,
        >(f))
    }
}

/// Why the camera can't be used when [`supported`] is false.
pub const UNSUPPORTED: &str = "needs Windows 11";

/// True if this Windows version can create virtual cameras (Windows 11).
pub fn supported() -> bool {
    create_function().is_some()
}

/// The running session camera. Dropping it removes the camera from the system.
pub struct VirtualCamera {
    camera: IMFVirtualCamera,
}

impl VirtualCamera {
    /// Creates and starts the camera (COM must be initialized on this thread).
    pub fn start() -> windows::core::Result<Self> {
        // SAFETY: MF startup is reference counted; the camera API needs it.
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL)? };
        let create = create_function().ok_or_else(|| {
            windows::core::Error::new(
                windows::Win32::Foundation::E_NOTIMPL,
                "the virtual camera needs Windows 11",
            )
        })?;
        let name = HSTRING::from(FRIENDLY_NAME);
        let id = HSTRING::from(CLSID_STRING);
        let mut camera = None;
        // SAFETY: the documented signature; strings outlive the call.
        unsafe {
            create(
                MFVirtualCameraType_SoftwareCameraSource,
                MFVirtualCameraLifetime_Session,
                MFVirtualCameraAccess_CurrentUser,
                PCWSTR(name.as_ptr()),
                PCWSTR(id.as_ptr()),
                std::ptr::null(),
                0,
                &mut camera,
            )
            .ok()?
        };
        let camera = camera
            .ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_POINTER))?;
        // SAFETY: starts the camera we created.
        unsafe { camera.Start(None)? };
        Ok(Self { camera })
    }
}

impl VirtualCamera {
    /// The camera's media source as Frame Server runs it (diagnostics).
    pub fn media_source(&self) -> windows::core::Result<IMFMediaSource> {
        // SAFETY: asks our own camera for its source.
        unsafe { self.camera.GetMediaSource() }
    }
}

impl Drop for VirtualCamera {
    fn drop(&mut self) {
        // SAFETY: removes and releases our own camera.
        unsafe {
            let _ = self.camera.Remove();
            let _ = self.camera.Shutdown();
            let _ = MFShutdown();
        }
    }
}

/// COM initialized (multithreaded) for the current thread while this lives.
pub struct ComApartment(bool);

impl ComApartment {
    pub fn multithreaded() -> Self {
        use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
        // SAFETY: per-thread init, balanced in Drop when it succeeded.
        Self(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok())
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: balances the successful CoInitializeEx.
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    }
}
