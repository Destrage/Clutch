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

fn main() -> anyhow::Result<()> {
    let paths = Paths::new();
    std::fs::create_dir_all(paths.scenes())?;
    let _log = estream_engine::diagnostics::init_logging(&paths.logs())?;
    let Some(_lock) = acquire_instance_lock(&paths)? else {
        tracing::error!("eStream is already running");
        eprintln!("eStream is already running.");
        return Ok(());
    };

    let settings = Settings::load(&paths.settings());
    let gpu = GpuContext::new_headless(&GpuOptions::default())?;
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
