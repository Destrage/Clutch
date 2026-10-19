//! Copies the bundled FFmpeg libraries next to built binaries (and test runners) so they load
//! at runtime without touching PATH: the DLLs from `bin` on Windows, the `.so.<major>` files
//! from `lib` on Linux (found there through the `$ORIGIN` rpath in `.cargo/config.toml`).

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");
    let Some(ffmpeg) = std::env::var_os("FFMPEG_DIR").map(PathBuf::from) else {
        return;
    };
    let windows = std::env::var("CARGO_CFG_WINDOWS").is_ok();
    if !windows && std::path::Path::new("/usr/include/libavcodec").exists() {
        // bindgen can pick these up instead of FFMPEG_DIR's headers, and codec ids then
        // don't match the bundled libraries.
        println!(
            "cargo:warning=the system's FFmpeg headers (/usr/include/libavcodec) can shadow \
             FFMPEG_DIR's; remove the distro's libav*-dev packages"
        );
    }
    let libs = ffmpeg.join(if windows { "bin" } else { "lib" });
    let Some(out) = std::env::var_os("OUT_DIR").map(PathBuf::from) else {
        return;
    };
    // OUT_DIR = target/<profile>/build/<crate>-<hash>/out
    let Some(profile) = out.ancestors().nth(3).map(PathBuf::from) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&libs) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !runtime_library(&name.to_string_lossy(), windows) {
            continue;
        }
        for dir in [
            profile.clone(),
            profile.join("deps"),
            profile.join("examples"),
        ] {
            let _ = std::fs::create_dir_all(&dir);
            let target = dir.join(&name);
            if !target.exists() {
                let _ = std::fs::copy(entry.path(), &target);
            }
        }
    }
}

/// `avcodec-63.dll` on Windows; `libavcodec.so.63` (the soname, not the `.so` link name or the
/// full version) elsewhere.
fn runtime_library(name: &str, windows: bool) -> bool {
    if windows {
        return name.to_ascii_lowercase().ends_with(".dll");
    }
    name.split_once(".so.")
        .is_some_and(|(_, v)| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
}
