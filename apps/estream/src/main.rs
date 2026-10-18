//! eStream desktop app.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod app;
mod interact;
mod presets;
mod secret;
mod settings;

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
const APP_USER_MODEL_ID: windows::core::PCWSTR = windows::core::w!("eStream.Studio");

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

fn main() -> anyhow::Result<()> {
    let paths = Paths::new();
    std::fs::create_dir_all(paths.scenes())?;
    let _log = estream_engine::diagnostics::init_logging(&paths.logs())?;
    let Some(_lock) = acquire_instance_lock(&paths)? else {
        tracing::error!("eStream is already running");
        eprintln!("eStream is already running.");
        return Ok(());
    };
    #[cfg(windows)]
    set_app_user_model_id();

    let settings = Settings::load(&paths.settings());
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
