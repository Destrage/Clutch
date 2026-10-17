//! Copies the bundled FFmpeg DLLs next to built binaries (and test runners) so
//! they load at runtime without touching PATH.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");
    let Some(ffmpeg) = std::env::var_os("FFMPEG_DIR").map(PathBuf::from) else {
        return;
    };
    let bin = ffmpeg.join("bin");
    let Some(out) = std::env::var_os("OUT_DIR").map(PathBuf::from) else {
        return;
    };
    // OUT_DIR = target/<profile>/build/<crate>-<hash>/out
    let Some(profile) = out.ancestors().nth(3).map(PathBuf::from) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&bin) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("dll"))
        {
            for dir in [
                profile.clone(),
                profile.join("deps"),
                profile.join("examples"),
            ] {
                let _ = std::fs::create_dir_all(&dir);
                let target = dir.join(entry.file_name());
                if !target.exists() {
                    let _ = std::fs::copy(&path, &target);
                }
            }
        }
    }
}
