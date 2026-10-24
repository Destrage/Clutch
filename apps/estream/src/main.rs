//! Clutch desktop app.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod account;
mod actions;
mod app;
mod chat;
mod clips;
mod emotes;
mod golive;
mod http;
mod interact;
mod kick;
mod livechat;
mod montage;
mod oauth;
mod overlay;
mod performance;
mod presets;
mod secret;
mod services;
mod settings;
mod shell;
mod twitch;
mod update;
mod viewers;
mod youtube;

#[allow(clippy::all, clippy::unwrap_used, clippy::expect_used)]
pub mod ui {
    slint::include_modules!();
}

use estream_gpu::{GpuContext, GpuOptions};
use settings::{Paths, Settings};
use slint::ComponentHandle;

/// Holds an exclusive lock on the instance file for the life of the process.
fn acquire_instance_lock(paths: &Paths) -> anyhow::Result<Option<std::fs::File>> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths.instance_lock())?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
    }
}

/// Must match `APP_USER_MODEL_ID` in `xtask` (set on the installer's shortcuts).
#[cfg(windows)]
const APP_USER_MODEL_ID: windows::core::PCWSTR = windows::core::w!("Clutch.Studio");

/// Gives the process the same AppUserModelID as its shortcuts, so Start and the taskbar treat
/// the window and the shortcuts as one app (pinning, grouping). Must run before any window exists.
#[cfg(windows)]
fn set_app_user_model_id() {
    // SAFETY: the ID is a static, NUL-terminated wide string.
    if let Err(e) = unsafe {
        windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(APP_USER_MODEL_ID)
    } {
        tracing::warn!("couldn't set the AppUserModelID: {e}");
    }
}

/// The desktop entry's name, which identifies Clutch to the desktop.
#[cfg(target_os = "linux")]
pub const APP_ID: &str = "io.github.destrage.Clutch";

/// Tells the desktop's portals which app this is, so permissions (global hotkeys, screen
/// sharing) are kept for Clutch. Must come before any other portal call. The portal only
/// accepts an id with a desktop entry, so one is kept up to date first.
#[cfg(target_os = "linux")]
fn register_portal_app() {
    if let Err(e) = keep_desktop_entry() {
        tracing::info!(error = %e, "couldn't write Clutch's desktop entry");
    }
    let Ok(id) = ashpd::AppID::try_from(APP_ID) else {
        return;
    };
    if let Err(e) = pollster::block_on(ashpd::register_host_app(id)) {
        tracing::info!(error = %e, "couldn't register with the desktop portals");
    }
}

/// Marks the desktop entry Clutch writes itself (it never touches one it didn't write).
#[cfg(target_os = "linux")]
const GENERATED: &str = "X-Clutch-Generated=true";

/// Writes a hidden desktop entry for [`APP_ID`] that starts this executable (the AppImage
/// when run from one), unless the system or the user already has one.
#[cfg(target_os = "linux")]
fn keep_desktop_entry() -> std::io::Result<()> {
    let file = format!("{APP_ID}.desktop");
    let home = dirs::data_dir().ok_or_else(|| std::io::Error::other("no data folder"))?;
    let ours = home.join("applications").join(&file);
    let system = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".into())
        .split(':')
        .any(|d| {
            std::path::Path::new(d)
                .join("applications")
                .join(&file)
                .exists()
        });
    let existing = std::fs::read_to_string(&ours).ok();
    if system || existing.as_ref().is_some_and(|t| !t.contains(GENERATED)) {
        return Ok(());
    }
    let exe = std::env::var_os("APPIMAGE")
        .map(std::path::PathBuf::from)
        .map_or_else(std::env::current_exe, Ok)?;
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=Clutch\n\
         Comment=Streaming, recording and clipping studio\nExec=\"{}\" %U\n\
         Icon={APP_ID}\nTerminal=false\nCategories=AudioVideo;Video;Recorder;\n\
         NoDisplay=true\n{GENERATED}\n",
        exe.display()
    );
    if existing.as_deref() != Some(entry.as_str()) {
        std::fs::create_dir_all(home.join("applications"))?;
        std::fs::write(&ours, entry)?;
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let paths = Paths::new();
    // Clutch was called eStream up to 0.2: take its settings folder over on the first start.
    let legacy = Paths::legacy_root();
    let adopted = legacy.as_deref().map(|l| paths.adopt(l));
    std::fs::create_dir_all(paths.scenes())?;
    let _log = estream_engine::diagnostics::init_logging(&paths.logs())?;
    let Some(_lock) = acquire_instance_lock(&paths)? else {
        tracing::error!("Clutch is already running");
        eprintln!("Clutch is already running.");
        return Ok(());
    };
    #[cfg(windows)]
    set_app_user_model_id();
    estream_capture::set_state_dir(paths.root.clone());
    #[cfg(target_os = "linux")]
    register_portal_app();

    let mut settings = Settings::load(&paths.settings());
    match (adopted, legacy) {
        (Some(Ok(true)), Some(legacy)) => {
            tracing::info!(from = %legacy.display(), "took over the eStream settings folder");
            if settings.rebase(&legacy, &paths.root) {
                settings.save(&paths.settings());
            }
        }
        (Some(Err(e)), _) => tracing::warn!(error = %e, "couldn't take over the eStream settings"),
        _ => {}
    }
    let gpu = GpuContext::new_headless(&GpuOptions::default())?;
    // Slint keeps the device and queue in a thread-local that is dropped during process
    // exit, after wgpu's own thread-locals are gone (which panics in wgpu's lock tracing).
    // Holding one reference for the life of the process means the GPU objects are never
    // torn down there; the OS reclaims them when the process exits.
    std::mem::forget(gpu.clone());
    slint::BackendSelector::new()
        .require_wgpu_30(slint::wgpu_30::WGPUConfiguration::Manual {
            instance: gpu.instance.clone(),
            adapter: gpu.adapter.clone(),
            device: gpu.device.clone(),
            queue: gpu.queue.clone(),
        })
        .select()?;

    let window = ui::AppWindow::new()?;
    // Ties the window to the desktop entry (icon, taskbar grouping) on Wayland and X11.
    #[cfg(target_os = "linux")]
    if let Err(e) = slint::set_xdg_app_id(APP_ID) {
        tracing::warn!(error = %e, "couldn't set the app id");
    }
    if let Some(state) = settings.window {
        window.window().set_size(slint::PhysicalSize::new(
            state.width.max(800),
            state.height.max(560),
        ));
        window
            .window()
            .set_position(slint::PhysicalPosition::new(state.x, state.y));
    }
    let maximized = settings.window.is_some_and(|s| s.maximized);

    let app = app::App::new(&window, paths, settings, gpu);
    let _timers = app::install(&app, &window);
    app.borrow_mut().begin();

    window.show()?;
    if maximized {
        window.window().set_maximized(true);
    }
    slint::run_event_loop()?;
    window.hide()?;

    // If the window was closed some other way, still shut down cleanly.
    if let Ok(mut a) = app.try_borrow_mut() {
        a.shutdown();
    }
    Ok(())
}
