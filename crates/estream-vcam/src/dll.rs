//! COM server entry points of `estream_vcam.dll` (loaded by Windows' Frame Server).

use crate::shared::MAPPING_NAME;
use crate::source::Activator;
use crate::{CLSID, CLSID_STRING, FRIENDLY_NAME};
use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_POINTER, ERROR_SUCCESS, HMODULE, S_FALSE,
    S_OK,
};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    GetModuleFileNameW, GetModuleHandleExW,
};
use windows::Win32::System::Ole::SELFREG_E_CLASS;
use windows::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW,
};
use windows::core::{
    BOOL, ComObject, GUID, HRESULT, HSTRING, IUnknown, Interface, PCWSTR, Ref, implement,
};

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        riid: *const GUID,
        ppv: *mut *mut core::ffi::c_void,
    ) -> windows::core::Result<()> {
        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let activator: IUnknown = ComObject::new(Activator::new(MAPPING_NAME)?).into_interface();
        // SAFETY: standard QueryInterface out-parameter.
        let result = unsafe { activator.query(riid, ppv).ok() };
        // SAFETY: riid is valid per the COM contract.
        crate::log(&format!("CreateInstance {:?}: {result:?}", unsafe {
            *riid
        }));
        result
    }

    fn LockServer(&self, _: BOOL) -> windows::core::Result<()> {
        Ok(())
    }
}

#[implement(IClassFactory)]
struct DShowFactory;

impl IClassFactory_Impl for DShowFactory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        riid: *const GUID,
        ppv: *mut *mut core::ffi::c_void,
    ) -> windows::core::Result<()> {
        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let filter: IUnknown =
            ComObject::new(crate::dshow::DShowFilter::new(MAPPING_NAME)).into_interface();
        let result = unsafe { filter.query(riid, ppv).ok() };
        crate::log(&format!("DShow CreateInstance {:?}: {result:?}", unsafe {
            *riid
        }));
        result
    }

    fn LockServer(&self, _: BOOL) -> windows::core::Result<()> {
        Ok(())
    }
}

/// # Safety
/// Called by COM with valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    clsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut core::ffi::c_void,
) -> HRESULT {
    if clsid.is_null() || riid.is_null() || ppv.is_null() {
        return E_POINTER;
    }
    // SAFETY: checked above.
    unsafe {
        *ppv = std::ptr::null_mut();
        if *clsid == CLSID {
            crate::log("DllGetClassObject (MF)");
            let factory: IUnknown = ComObject::new(Factory).into_interface();
            factory.query(riid, ppv)
        } else if *clsid == crate::dshow::DSHOW_CLSID {
            crate::log("DllGetClassObject (DShow)");
            let factory: IUnknown = ComObject::new(DShowFactory).into_interface();
            factory.query(riid, ppv)
        } else {
            CLASS_E_CLASSNOTAVAILABLE
        }
    }
}

/// The DLL may unload once no camera objects are left (so it isn't kept locked).
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if crate::source::objects_alive() || crate::dshow::dshow_objects_alive() {
        S_FALSE
    } else {
        S_OK
    }
}

/// Path of this DLL.
fn module_path() -> Option<String> {
    let mut module = HMODULE::default();
    // SAFETY: looks up the module containing this function; no refcount change.
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(module_path as *const () as *const u16),
            &mut module,
        )
        .ok()?;
        let mut buf = [0u16; 1024];
        let n = GetModuleFileNameW(Some(module), &mut buf) as usize;
        (n > 0 && n < buf.len()).then(|| String::from_utf16_lossy(&buf[..n]))
    }
}

fn set_string(key: HKEY, name: Option<&str>, value: &str) -> bool {
    let wide: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
    // SAFETY: the byte view covers the UTF-16 string including its terminator.
    let bytes = unsafe { std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2) };
    let name = name.map(HSTRING::from);
    // SAFETY: valid open key and data.
    unsafe {
        RegSetValueExW(
            key,
            name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr())),
            None,
            REG_SZ,
            Some(bytes),
        ) == ERROR_SUCCESS
    }
}

fn create_key(path: &str) -> Option<HKEY> {
    let mut key = HKEY::default();
    // SAFETY: creates or opens a key under HKLM (needs elevation).
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(path),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        )
    };
    (status == ERROR_SUCCESS).then_some(key)
}

pub(crate) fn class_key() -> String {
    format!("SOFTWARE\\Classes\\CLSID\\{CLSID_STRING}")
}

/// Creates the install folder: SYSTEM and Administrators full control, LOCAL SERVICE
/// (Frame Server) may write its log, users may read.
fn create_install_dir(dir: &std::path::Path) -> bool {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows::Win32::Storage::FileSystem::CreateDirectoryW;
    if let Some(parent) = dir.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if dir.is_dir() {
        return true;
    }
    let sddl = "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1301bf;;;LS)(A;OICI;FRFX;;;BU)";
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // SAFETY: valid SDDL; the descriptor is freed below.
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            &HSTRING::from(sddl),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    };
    if converted.is_err() {
        return std::fs::create_dir_all(dir).is_ok();
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    // SAFETY: valid path and attributes.
    let made = unsafe { CreateDirectoryW(&HSTRING::from(dir.as_os_str()), Some(&attributes)) };
    // SAFETY: allocated by the conversion above.
    unsafe {
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
    }
    made.is_ok() || dir.is_dir()
}

/// Copies this DLL into the install folder (named by its content) and returns the copy.
fn install_copy() -> Option<String> {
    let source = std::path::PathBuf::from(module_path()?);
    let bytes = std::fs::read(&source).ok()?;
    let dir = crate::install_dir();
    if !create_install_dir(&dir) {
        return None;
    }
    let name = format!(
        "estream_vcam-{}-{:016x}.dll",
        crate::build_tag(),
        crate::content_hash(&bytes)
    );
    let dest = dir.join(&name);
    if !dest.exists() && std::fs::write(&dest, &bytes).is_err() {
        return None;
    }
    // Older copies go once Frame Server has let go of them.
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let file = entry.file_name().to_string_lossy().into_owned();
            if file.starts_with("estream_vcam-") && file.ends_with(".dll") && file != name {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    Some(dest.to_string_lossy().into_owned())
}

/// `regsvr32 estream_vcam.dll` (elevated): installs a copy for all users and registers it.
#[unsafe(no_mangle)]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    let Some(path) = install_copy() else {
        return SELFREG_E_CLASS;
    };
    crate::log(&format!("registered {path}"));
    let Some(class) = create_key(&class_key()) else {
        return SELFREG_E_CLASS;
    };
    let ok = set_string(class, None, FRIENDLY_NAME);
    // SAFETY: closes the key we opened.
    unsafe {
        let _ = RegCloseKey(class);
    }
    let Some(server) = create_key(&format!("{}\\InprocServer32", class_key())) else {
        return SELFREG_E_CLASS;
    };
    let ok =
        ok && set_string(server, None, &path) && set_string(server, Some("ThreadingModel"), "Both");
    // SAFETY: closes the key we opened.
    unsafe {
        let _ = RegCloseKey(server);
    }
    let dshow_ok = crate::dshow::register(&path);
    crate::log(&format!("dshow register: {dshow_ok}"));
    if ok { S_OK } else { SELFREG_E_CLASS }
}

/// `regsvr32 /u estream_vcam.dll`.
#[unsafe(no_mangle)]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    // DirectShow filter unregistration
    crate::dshow::unregister();
    // SAFETY: deletes our own class key.
    let status = unsafe { RegDeleteTreeW(HKEY_LOCAL_MACHINE, &HSTRING::from(class_key())) };
    // Remove the installed copies and the log (a copy Frame Server still holds stays behind).
    let dir = crate::install_dir();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let _ = std::fs::remove_dir(&dir);
    if status == ERROR_SUCCESS || status.0 == 2 {
        S_OK
    } else {
        SELFREG_E_CLASS
    }
}

// The class factory must be reachable through `IClassFactory`.
const _: fn() = || {
    let _ = IClassFactory::IID;
};
