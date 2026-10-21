//! `cargo xtask package` on Linux: `target/package/Clutch-<version>-x86_64.AppImage`.
//!
//! Everything Clutch ships sits next to the binaries in `usr/bin` (FFmpeg, CEF), found through
//! their `$ORIGIN` rpath, so no library paths need rewriting. glibc, the GPU driver, PipeWire,
//! ALSA, fontconfig and the libraries Chromium uses come from the system.

use super::{Result, VERSION, copy, icon, root, run, write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The desktop entry's name; must match `APP_ID` in `apps/estream/src/main.rs`.
const APP_ID: &str = "io.github.destrage.Clutch";
const APPIMAGETOOL_URL: &str =
    "https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage";

/// Files the app needs from the release folder besides `locales/`.
fn wanted(name: &str) -> bool {
    matches!(
        name,
        "clutch" | "clutch-browser" | "vk_swiftshader_icd.json"
    ) || name.contains(".so")
        || [".pak", ".bin", ".dat"]
            .iter()
            .any(|ext| name.ends_with(ext))
}

pub fn package() -> Result<()> {
    let root = root();
    // CARGO_TARGET_DIR when set (as for a build in WSL next to the Windows one), else `target`.
    let target = root.join(std::env::var_os("CARGO_TARGET_DIR").unwrap_or("target".into()));
    let release = target.join("release");
    let out = target.join("package");
    let appdir = out.join("Clutch.AppDir");
    let bin = appdir.join("usr/bin");

    // 1. Release build.
    run(Command::new(env!("CARGO")).current_dir(&root).args([
        "build",
        "--release",
        "-p",
        "estream",
        "-p",
        "estream-browser",
    ]))?;

    // 2. Stage the AppDir.
    let _ = std::fs::remove_dir_all(&appdir);
    for entry in std::fs::read_dir(&release).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_file() && wanted(&name) {
            copy(&entry.path(), &bin.join(&name))?;
        }
    }
    for entry in std::fs::read_dir(release.join("locales")).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        copy(&entry.path(), &bin.join("locales").join(entry.file_name()))?;
    }
    for required in ["clutch", "clutch-browser", "libcef.so", "libavcodec.so.63"] {
        if !bin.join(required).exists() {
            return Err(format!("{required} is missing from {}", release.display()));
        }
    }
    strip(&bin)?;

    let docs = appdir.join("usr/share/doc/clutch");
    copy(
        &root.join("third_party/ffmpeg/LICENSE.txt"),
        &docs.join("FFmpeg-LGPL.txt"),
    )?;
    for font in ["Jost", "Figtree"] {
        copy(
            &root.join(format!("apps/estream/ui/fonts/{font}-OFL.txt")),
            &docs.join(format!("{font}-OFL.txt")),
        )?;
    }
    copy(
        &release.join("CREDITS.html"),
        &docs.join("Chromium-CEF-credits.html"),
    )?;
    copy(&root.join("README.md"), &docs.join("README.md"))?;

    let icon_png = appdir.join(format!("{APP_ID}.png"));
    icon::logo(256)
        .save(&icon_png)
        .map_err(|e| format!("{}: {e}", icon_png.display()))?;
    copy(
        &icon_png,
        &appdir.join(format!("usr/share/icons/hicolor/256x256/apps/{APP_ID}.png")),
    )?;
    let desktop = desktop_entry();
    write(
        &appdir.join(format!("{APP_ID}.desktop")),
        desktop.as_bytes(),
    )?;
    write(
        &appdir.join(format!("usr/share/applications/{APP_ID}.desktop")),
        desktop.as_bytes(),
    )?;
    write(
        &appdir.join(format!("usr/share/metainfo/{APP_ID}.metainfo.xml")),
        metainfo().as_bytes(),
    )?;
    let apprun = appdir.join("AppRun");
    write(&apprun, APPRUN.as_bytes())?;
    make_executable(&apprun)?;

    // 3. Pack.
    let tool = appimagetool(&root)?;
    let image = out.join(format!("Clutch-{VERSION}-x86_64.AppImage"));
    // Extract-and-run: no FUSE needed (CI containers lack it).
    run(Command::new(&tool)
        .arg("--appimage-extract-and-run")
        .arg("--no-appstream")
        .arg(&appdir)
        .arg(&image)
        .env("ARCH", "x86_64"))?;
    let size = std::fs::metadata(&image).map(|m| m.len()).unwrap_or(0);
    println!("\nbuilt {} ({:.0} MB)", image.display(), size as f64 / 1e6);
    Ok(())
}

/// Starts Clutch from wherever the image is mounted.
const APPRUN: &str = r#"#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/clutch" "$@"
"#;

fn desktop_entry() -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Clutch\n\
         GenericName=Streaming Studio\n\
         Comment=Stream, record and clip with one app\n\
         Exec=clutch %U\n\
         Icon={APP_ID}\n\
         Terminal=false\n\
         Categories=AudioVideo;Video;Recorder;\n\
         Keywords=stream;record;clip;twitch;youtube;webcam;\n\
         StartupWMClass={APP_ID}\n"
    )
}

fn metainfo() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<component type="desktop-application">
  <id>{APP_ID}</id>
  <name>Clutch</name>
  <summary>Stream, record and clip with one app</summary>
  <metadata_license>CC0-1.0</metadata_license>
  <project_license>MIT OR Apache-2.0</project_license>
  <description>
    <p>Clutch is a streaming, recording and clipping studio: scenes with screen, window,
    camera, browser and media sources, live streaming to several platforms at once,
    recording, a replay buffer for clips, and a virtual camera.</p>
  </description>
  <launchable type="desktop-id">{APP_ID}.desktop</launchable>
  <url type="homepage">https://github.com/Destrage/Clutch</url>
  <releases>
    <release version="{VERSION}"/>
  </releases>
</component>
"#
    )
}

/// Strips debug symbols from the binaries and libraries (CEF ships with them: ~1.4 GB).
fn strip(dir: &Path) -> Result<()> {
    let files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("clutch") || n.contains(".so"))
        })
        .collect();
    run(Command::new("strip").arg("--strip-unneeded").args(&files))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(not(unix))]
fn make_executable(_: &Path) -> Result<()> {
    Ok(())
}

/// `appimagetool` in `third_party/appimage` (downloaded once, git-ignored).
fn appimagetool(root: &Path) -> Result<PathBuf> {
    let tool = root.join("third_party/appimage/appimagetool-1.9.1-x86_64.AppImage");
    if tool.exists() {
        return Ok(tool);
    }
    if let Some(dir) = tool.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    run(Command::new("curl")
        .args(["-L", "--fail", "-o"])
        .arg(&tool)
        .arg(APPIMAGETOOL_URL))?;
    make_executable(&tool)?;
    Ok(tool)
}
