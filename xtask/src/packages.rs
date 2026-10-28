//! `cargo xtask package` on Linux, after the AppImage: `Clutch-<version>-amd64.deb` and
//! `Clutch-<version>-x86_64.rpm`, built by nfpm from the same AppDir.
//!
//! Clutch goes in `/opt/clutch` (its FFmpeg and CEF are private copies that must not mix with
//! the system's), with `/usr/bin/clutch` and the desktop entry, icon and AppStream metadata
//! under `/usr/share`. `/opt/clutch/package` says which format installed it, for the updater.

use super::{Result, VERSION, run, write};
use std::path::{Path, PathBuf};
use std::process::Command;

const NFPM_VERSION: &str = "2.48.0";
const NFPM_SHA256: &str = "6d7f9a28b2f397934403950fcbba83f598b08d02edbe79a0815b3b4897341750";

/// System libraries Clutch links or opens at runtime, with the Debian/Ubuntu package of each
/// (`a | b` covers Ubuntu 24.04's `t64` renames). The .rpm requires the libraries themselves,
/// which dnf and zypper resolve on any RPM distro. glibc is added from the binaries.
const LIBS: &[(&str, &str)] = &[
    ("libasound.so.2", "libasound2t64 | libasound2"),
    ("libatk-1.0.so.0", "libatk1.0-0t64 | libatk1.0-0"),
    (
        "libatk-bridge-2.0.so.0",
        "libatk-bridge2.0-0t64 | libatk-bridge2.0-0",
    ),
    ("libatspi.so.0", "libatspi2.0-0t64 | libatspi2.0-0"),
    ("libcairo.so.2", "libcairo2"),
    ("libcups.so.2", "libcups2t64 | libcups2"),
    ("libdbus-1.so.3", "libdbus-1-3"),
    ("libEGL.so.1", "libegl1"),
    ("libexpat.so.1", "libexpat1"),
    ("libfontconfig.so.1", "libfontconfig1"),
    ("libgbm.so.1", "libgbm1"),
    ("libgcc_s.so.1", "libgcc-s1"),
    ("libgio-2.0.so.0", "libglib2.0-0t64 | libglib2.0-0"),
    ("libglib-2.0.so.0", "libglib2.0-0t64 | libglib2.0-0"),
    ("libgobject-2.0.so.0", "libglib2.0-0t64 | libglib2.0-0"),
    ("libnspr4.so", "libnspr4"),
    ("libnss3.so", "libnss3"),
    ("libnssutil3.so", "libnss3"),
    ("libsmime3.so", "libnss3"),
    ("libpango-1.0.so.0", "libpango-1.0-0"),
    // Built against PipeWire 1.0 (Ubuntu 24.04); the `t64` package is always newer.
    (
        "libpipewire-0.3.so.0",
        "libpipewire-0.3-0t64 | libpipewire-0.3-0 (>= 1.0)",
    ),
    ("libudev.so.1", "libudev1"),
    ("libwayland-client.so.0", "libwayland-client0"),
    ("libwayland-egl.so.1", "libwayland-egl1"),
    ("libX11.so.6", "libx11-6"),
    ("libX11-xcb.so.1", "libx11-xcb1"),
    ("libxcb.so.1", "libxcb1"),
    ("libXcomposite.so.1", "libxcomposite1"),
    ("libXcursor.so.1", "libxcursor1"),
    ("libXdamage.so.1", "libxdamage1"),
    ("libXext.so.6", "libxext6"),
    ("libXfixes.so.3", "libxfixes3"),
    ("libXi.so.6", "libxi6"),
    ("libxkbcommon.so.0", "libxkbcommon0"),
    ("libxkbcommon-x11.so.0", "libxkbcommon-x11-0"),
    ("libXrandr.so.2", "libxrandr2"),
];

/// Wanted but not required: sound and screen capture services, GPU drivers, virtual camera.
const RECOMMENDS: &[&str] = &["pipewire", "xdg-desktop-portal", "mesa-vulkan-drivers"];
const DEB_SUGGESTS: &str = "v4l2loopback-dkms";
const RPM_SUGGESTS: &str = "v4l2loopback";

const SUMMARY: &str = "Streaming, recording and clipping studio";

/// Builds the .deb and .rpm from the staged AppDir into `out`.
pub fn package(root: &Path, appdir: &Path, out: &Path) -> Result<()> {
    let bin = appdir.join("usr/bin");
    let glibc = glibc_floor(&bin)?;
    let tool = nfpm(root)?;
    for format in ["deb", "rpm"] {
        write(
            &out.join(format!("package-{format}")),
            format!("{format}\n").as_bytes(),
        )?;
    }
    let config = out.join("nfpm.yaml");
    write(&config, nfpm_config(appdir, out, &glibc).as_bytes())?;
    for (format, file) in [
        ("deb", format!("Clutch-{VERSION}-amd64.deb")),
        ("rpm", format!("Clutch-{VERSION}-x86_64.rpm")),
    ] {
        let target = out.join(&file);
        let _ = std::fs::remove_file(&target);
        run(Command::new(&tool)
            .args(["package", "--packager", format, "--config"])
            .arg(&config)
            .arg("--target")
            .arg(&target))?;
        let size = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
        println!("built {} ({:.0} MB)", target.display(), size as f64 / 1e6);
    }
    Ok(())
}

/// The .deb's dependencies: glibc, then each package once, in order.
fn deb_depends(glibc: &str) -> Vec<String> {
    let mut out = vec![format!("libc6 (>= {glibc})")];
    for (_, pkg) in LIBS {
        if !out.iter().any(|p| p == pkg) {
            out.push((*pkg).to_owned());
        }
    }
    out
}

/// The .rpm's dependencies: the libraries by soname (64-bit), glibc by symbol version.
fn rpm_depends(glibc: &str) -> Vec<String> {
    let mut out = vec![format!("libc.so.6(GLIBC_{glibc})(64bit)")];
    out.extend(LIBS.iter().map(|(so, _)| format!("{so}()(64bit)")));
    out
}

fn yaml_list(items: &[String], indent: &str) -> String {
    items
        .iter()
        .map(|i| format!("{indent}- \"{}\"\n", i.replace('"', "\\\"")))
        .collect()
}

fn nfpm_config(appdir: &Path, out: &Path, glibc: &str) -> String {
    let recommends: Vec<String> = RECOMMENDS.iter().map(|s| (*s).to_owned()).collect();
    let share = appdir.join("usr/share");
    let app = super::appimage::APP_ID;
    format!(
        r#"name: clutch
arch: amd64
platform: linux
version: "{VERSION}"
section: video
priority: optional
maintainer: "Destrosphere Clutch <clutch@destrosphere.com>"
vendor: Destrosphere
homepage: https://clutch.destrosphere.com
license: MIT OR Apache-2.0
description: |-
  {SUMMARY}
  Scenes with screen, window, camera, browser and media sources, live streaming to several
  platforms at once, recording, a replay buffer for clips, and a virtual camera.
contents:
  - src: {bin}
    dst: /opt/clutch
    type: tree
  - src: {deb_marker}
    dst: /opt/clutch/package
    packager: deb
  - src: {rpm_marker}
    dst: /opt/clutch/package
    packager: rpm
  - src: /opt/clutch/clutch
    dst: /usr/bin/clutch
    type: symlink
  - src: {share}/applications/{app}.desktop
    dst: /usr/share/applications/{app}.desktop
  - src: {share}/icons/hicolor/256x256/apps/{app}.png
    dst: /usr/share/icons/hicolor/256x256/apps/{app}.png
  - src: {share}/metainfo/{app}.metainfo.xml
    dst: /usr/share/metainfo/{app}.metainfo.xml
  - src: {share}/doc/clutch
    dst: /usr/share/doc/clutch
    type: tree
overrides:
  deb:
    depends:
{deb_depends}    recommends:
{recommends}    suggests:
      - "{DEB_SUGGESTS}"
  rpm:
    depends:
{rpm_depends}    recommends:
{recommends}    suggests:
      - "{RPM_SUGGESTS}"
rpm:
  summary: "{SUMMARY}"
  group: Applications/Multimedia
"#,
        bin = appdir.join("usr/bin").display(),
        share = share.display(),
        deb_marker = out.join("package-deb").display(),
        rpm_marker = out.join("package-rpm").display(),
        deb_depends = yaml_list(&deb_depends(glibc), "      "),
        rpm_depends = yaml_list(&rpm_depends(glibc), "      "),
        recommends = yaml_list(&recommends, "      "),
    )
}

/// The newest glibc version the binaries require (weak symbols left out: they are only used
/// when present), e.g. "2.39".
fn glibc_floor(bin: &Path) -> Result<String> {
    let mut newest = (2u32, 17u32);
    for entry in std::fs::read_dir(bin).map_err(|e| e.to_string())?.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if !(name.starts_with("clutch") || name.contains(".so")) || !path.is_file() {
            continue;
        }
        let output = Command::new("objdump")
            .arg("-T")
            .arg(&path)
            .output()
            .map_err(|e| format!("objdump: {e}"))?;
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Some(v) = required_glibc(line) {
                newest = newest.max(v);
            }
        }
    }
    Ok(format!("{}.{}", newest.0, newest.1))
}

/// The glibc version an `objdump -T` line requires: an undefined, non-weak `GLIBC_x.y` symbol.
fn required_glibc(line: &str) -> Option<(u32, u32)> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    if !line.contains("*UND*") || fields.get(1) == Some(&"w") {
        return None;
    }
    let version = line.split("(GLIBC_").nth(1)?.split(')').next()?;
    let mut parts = version.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

/// `nfpm` in `third_party/nfpm` (downloaded once and checked, git-ignored).
fn nfpm(root: &Path) -> Result<PathBuf> {
    let dir = root.join(format!("third_party/nfpm/{NFPM_VERSION}"));
    let tool = dir.join("nfpm");
    if tool.exists() {
        return Ok(tool);
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let archive = dir.join("nfpm.tar.gz");
    run(Command::new("curl")
        .args(["-L", "--fail", "-o"])
        .arg(&archive)
        .arg(format!(
            "https://github.com/goreleaser/nfpm/releases/download/v{NFPM_VERSION}/nfpm_{NFPM_VERSION}_Linux_x86_64.tar.gz"
        )))?;
    let sum = Command::new("sha256sum")
        .arg(&archive)
        .output()
        .map_err(|e| format!("sha256sum: {e}"))?;
    if !String::from_utf8_lossy(&sum.stdout).starts_with(NFPM_SHA256) {
        let _ = std::fs::remove_file(&archive);
        return Err("the nfpm download doesn't match its checksum".into());
    }
    run(Command::new("tar")
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(&dir)
        .arg("nfpm"))?;
    Ok(tool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependencies_list_each_package_once() {
        let deb = deb_depends("2.39");
        assert_eq!(deb[0], "libc6 (>= 2.39)");
        assert_eq!(deb.iter().filter(|p| p.contains("libnss3")).count(), 1);
        assert_eq!(deb.len(), 1 + 32);
        let rpm = rpm_depends("2.39");
        assert_eq!(rpm[0], "libc.so.6(GLIBC_2.39)(64bit)");
        assert!(rpm.contains(&"libnss3.so()(64bit)".to_owned()));
        assert_eq!(rpm.len(), 1 + LIBS.len());
    }

    #[test]
    fn glibc_floor_skips_weak_symbols() {
        let strong =
            "0000000000000000      DF *UND*\t0000000000000000 (GLIBC_2.38) __isoc23_strtol";
        let weak = "0000000000000000  w   DF *UND*\t0000000000000000 (GLIBC_2.39) pidfd_spawnp";
        let defined = "00000000000a1b20 g    DF .text\t0000000000000010  Base        my_fn";
        assert_eq!(required_glibc(strong), Some((2, 38)));
        assert_eq!(required_glibc(weak), None);
        assert_eq!(required_glibc(defined), None);
    }
}
