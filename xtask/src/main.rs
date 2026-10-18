//! Developer tasks: `cargo xtask icon` and `cargo xtask package` (the Windows installer).

mod icon;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

type Result<T> = std::result::Result<T, String>;

const TARGET: &str = "x86_64-pc-windows-msvc";
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Never change: Windows uses it to recognise upgrades of eStream and Clutch (Clutch was called eStream up to 0.2).
const UPGRADE_CODE: &str = "6F1C2E8A-4B7D-4E21-9C3A-58D0E2B4A719";
/// Must match `APP_USER_MODEL_ID` in `apps/estream/src/main.rs`, so Start and the taskbar
/// tie the running window to the shortcuts (pinning, grouping).
const APP_USER_MODEL_ID: &str = "Clutch.Studio";
const WIX_URL: &str =
    "https://github.com/wixtoolset/wix3/releases/download/wix3141rtm/wix314-binaries.zip";

fn main() {
    let task = std::env::args().nth(1).unwrap_or_default();
    let result = match task.as_str() {
        "icon" => write_icon(),
        "package" => package(),
        _ => Err("usage: cargo xtask <icon|package>".into()),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

fn run(cmd: &mut Command) -> Result<()> {
    println!("> {cmd:?}");
    let status = cmd.status().map_err(|e| format!("{cmd:?}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{cmd:?} failed ({status})"))
    }
}

fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// `apps/estream/assets/clutch.ico` (embedded in clutch.exe, used by the installer).
fn write_icon() -> Result<()> {
    let path = root().join("apps/estream/assets/clutch.ico");
    write(&path, &icon::ico())?;
    println!("wrote {}", path.display());
    let preview = root().join("target/installer/icon-preview.png");
    let _ = std::fs::create_dir_all(preview.parent().unwrap_or(Path::new(".")));
    icon::logo(256).save(&preview).map_err(|e| e.to_string())?;
    println!("preview {}", preview.display());
    Ok(())
}

fn copy(from: &Path, to: &Path) -> Result<()> {
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::copy(from, to)
        .map(|_| ())
        .map_err(|e| format!("copy {} → {}: {e}", from.display(), to.display()))
}

/// Files the installed app needs from the release folder (ours, FFmpeg, CEF).
fn wanted(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "clutch.exe" | "clutch-browser.exe" | "estream_vcam.dll" | "vk_swiftshader_icd.json"
    ) || [".dll", ".pak", ".bin", ".dat"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

fn package() -> Result<()> {
    let root = root();
    let release = root.join("target").join(TARGET).join("release");
    let out = root.join("target/installer");
    let stage = out.join("stage");

    // 1. Release build with the C runtime linked in (no VC++ redistributable needed).
    run(Command::new(env!("CARGO"))
        .current_dir(&root)
        .args(["build", "--release", "--target", TARGET])
        .args([
            "-p",
            "estream",
            "-p",
            "estream-browser",
            "-p",
            "estream-vcam",
        ])
        .env("RUSTFLAGS", "-C target-feature=+crt-static"))?;

    // 2. Stage the files.
    let _ = std::fs::remove_dir_all(&stage);
    for entry in std::fs::read_dir(&release).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_file() && wanted(&name) {
            copy(&entry.path(), &stage.join(&name))?;
        }
    }
    for entry in std::fs::read_dir(release.join("locales")).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        copy(
            &entry.path(),
            &stage.join("locales").join(entry.file_name()),
        )?;
    }
    for required in [
        "clutch.exe",
        "clutch-browser.exe",
        "estream_vcam.dll",
        "libcef.dll",
    ] {
        if !stage.join(required).exists() {
            return Err(format!("{required} is missing from {}", release.display()));
        }
    }
    let notices = stage.join("licenses");
    copy(
        &root.join("third_party/ffmpeg/LICENSE.txt"),
        &notices.join("FFmpeg-LGPL.txt"),
    )?;
    copy(
        &release.join("CREDITS.html"),
        &notices.join("Chromium-CEF-credits.html"),
    )?;
    copy(&root.join("README.md"), &stage.join("README.md"))?;

    // 3. Installer artwork and texts.
    let assets = out.join("assets");
    write(&assets.join("clutch.ico"), &icon::ico())?;
    let (banner, dialog) = icon::installer_art();
    write(&assets.join("banner.bmp"), &banner)?;
    write(&assets.join("dialog.bmp"), &dialog)?;
    write(&assets.join("license.rtf"), license_rtf().as_bytes())?;

    // 4. WiX source and build.
    let wix = wix_tools(&root)?;
    let wxs = out.join("clutch.wxs");
    write(&wxs, wix_source(&stage, &assets)?.as_bytes())?;
    let obj = out.join("clutch.wixobj");
    let msi = out.join(format!("Clutch-{VERSION}-x64.msi"));
    run(Command::new(wix.join("candle.exe"))
        .args([
            "-nologo",
            "-arch",
            "x64",
            "-ext",
            "WixUIExtension",
            "-ext",
            "WixUtilExtension",
        ])
        .arg("-out")
        .arg(&obj)
        .arg(&wxs))?;
    run(Command::new(wix.join("light.exe"))
        .args([
            "-nologo",
            "-ext",
            "WixUIExtension",
            "-ext",
            "WixUtilExtension",
        ])
        .args(["-cultures:en-us", "-spdb"])
        .arg("-out")
        .arg(&msi)
        .arg(&obj))?;
    let size = std::fs::metadata(&msi).map(|m| m.len()).unwrap_or(0);
    println!("\nbuilt {} ({:.0} MB)", msi.display(), size as f64 / 1e6);
    Ok(())
}

/// WiX 3.14 command-line tools in `third_party/wix` (downloaded once, git-ignored).
fn wix_tools(root: &Path) -> Result<PathBuf> {
    let dir = root.join("third_party/wix");
    if dir.join("candle.exe").exists() {
        return Ok(dir);
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let zip = dir.join("wix314-binaries.zip");
    run(Command::new("curl.exe")
        .args(["-L", "--fail", "-o"])
        .arg(&zip)
        .arg(WIX_URL))?;
    run(Command::new("tar.exe")
        .arg("-xf")
        .arg(&zip)
        .arg("-C")
        .arg(&dir))?;
    let _ = std::fs::remove_file(&zip);
    if dir.join("candle.exe").exists() {
        Ok(dir)
    } else {
        Err("the WiX download didn't contain candle.exe".into())
    }
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A stable component GUID for an installed path (the same file keeps its GUID across builds).
fn guid(path: &str) -> String {
    let ns = uuid::Uuid::from_u128(0x5b1f_6c2e_9a3d_4f80_b2c1_7e64_d0a9_3f15);
    uuid::Uuid::new_v5(&ns, path.to_ascii_lowercase().as_bytes())
        .to_string()
        .to_uppercase()
}

/// A WiX identifier from a path (letters, digits, `_` and `.`, at most 72 characters).
fn ident(prefix: &str, path: &str) -> String {
    let clean: String = path
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let id = format!("{prefix}_{clean}");
    if id.len() <= 72 {
        id
    } else {
        format!("{prefix}_{}", &guid(path).replace('-', "")[..24])
    }
}

/// Writes the files under `dir` (relative path `rel`) as Directory/Component elements.
fn add_dir(
    xml_out: &mut String,
    refs: &mut Vec<String>,
    dir: &Path,
    rel: &str,
    depth: usize,
) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .collect();
    entries.sort_by_key(|e| e.file_name());
    let pad = "  ".repeat(depth);
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}\\{name}")
        };
        if entry.path().is_dir() {
            let _ = writeln!(
                xml_out,
                "{pad}<Directory Id=\"{}\" Name=\"{}\">",
                ident("dir", &path),
                xml(&name)
            );
            add_dir(xml_out, refs, &entry.path(), &path, depth + 1)?;
            let _ = writeln!(xml_out, "{pad}</Directory>");
        } else {
            let component = ident("cmp", &path);
            let _ = writeln!(
                xml_out,
                "{pad}<Component Id=\"{component}\" Guid=\"{{{}}}\" Win64=\"yes\"><File Id=\"{}\" Source=\"{}\" KeyPath=\"yes\" /></Component>",
                guid(&path),
                ident("fil", &path),
                xml(&entry.path().to_string_lossy())
            );
            refs.push(component);
        }
    }
    Ok(())
}

fn wix_source(stage: &Path, assets: &Path) -> Result<String> {
    let mut files = String::new();
    let mut refs = Vec::new();
    add_dir(&mut files, &mut refs, stage, "", 4)?;
    let component_refs: String = refs
        .iter()
        .map(|c| format!("      <ComponentRef Id=\"{c}\" />\n"))
        .collect();
    let asset = |name: &str| xml(&assets.join(name).to_string_lossy());
    let exe = ident("fil", "clutch.exe");
    let regsvr = "&quot;[System64Folder]regsvr32.exe&quot;";
    let vcam = "&quot;[INSTALLFOLDER]estream_vcam.dll&quot;";
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Wix xmlns="http://schemas.microsoft.com/wix/2006/wi">
  <Product Id="*" Name="Clutch" Language="1033" Version="{VERSION}" Manufacturer="Clutch" UpgradeCode="{{{UPGRADE_CODE}}}">
    <Package InstallerVersion="500" Compressed="yes" InstallScope="perMachine" Platform="x64" Description="Clutch streaming, recording and clipping studio" />
    <MajorUpgrade AllowSameVersionUpgrades="yes" DowngradeErrorMessage="A newer version of Clutch is already installed." />
    <MediaTemplate EmbedCab="yes" CompressionLevel="high" />

    <Icon Id="clutch.ico" SourceFile="{icon}" />
    <Property Id="ARPPRODUCTICON" Value="clutch.ico" />
    <Property Id="ARPURLINFOABOUT" Value="https://github.com/Destrage/Clutch" />
    <Property Id="WIXUI_INSTALLDIR" Value="INSTALLFOLDER" />
    <WixVariable Id="WixUILicenseRtf" Value="{license}" />
    <WixVariable Id="WixUIBannerBmp" Value="{banner}" />
    <WixVariable Id="WixUIDialogBmp" Value="{dialog}" />
    <UIRef Id="WixUI_InstallDir" />

    <Property Id="WIXUI_EXITDIALOGOPTIONALCHECKBOXTEXT" Value="Launch Clutch" />
    <Property Id="WIXUI_EXITDIALOGOPTIONALCHECKBOX" Value="1" />
    <Property Id="WixShellExecTarget" Value="[#{exe}]" />
    <CustomAction Id="LaunchApp" BinaryKey="WixCA" DllEntry="WixShellExec" Impersonate="yes" />
    <UI>
      <Publish Dialog="ExitDialog" Control="Finish" Event="DoAction" Value="LaunchApp">WIXUI_EXITDIALOGOPTIONALCHECKBOX = 1 and NOT Installed</Publish>
    </UI>

    <Directory Id="TARGETDIR" Name="SourceDir">
      <Directory Id="ProgramFiles64Folder">
        <Directory Id="INSTALLFOLDER" Name="Clutch">
{files}        </Directory>
      </Directory>
      <Directory Id="ProgramMenuFolder" />
      <Directory Id="DesktopFolder" />
    </Directory>

    <DirectoryRef Id="ProgramMenuFolder">
      <Component Id="cmp_StartMenu" Guid="{{{start}}}" Win64="yes">
        <Shortcut Id="StartMenuShortcut" Name="Clutch" Description="Stream, record and clip" Target="[INSTALLFOLDER]clutch.exe" WorkingDirectory="INSTALLFOLDER" Icon="clutch.ico">
          <ShortcutProperty Key="System.AppUserModel.ID" Value="{aumid}" />
        </Shortcut>
        <RegistryValue Root="HKCU" Key="Software\Clutch" Name="StartMenuShortcut" Type="integer" Value="1" KeyPath="yes" />
      </Component>
    </DirectoryRef>
    <DirectoryRef Id="DesktopFolder">
      <Component Id="cmp_Desktop" Guid="{{{desktop}}}" Win64="yes">
        <Shortcut Id="DesktopShortcut" Name="Clutch" Description="Stream, record and clip" Target="[INSTALLFOLDER]clutch.exe" WorkingDirectory="INSTALLFOLDER" Icon="clutch.ico">
          <ShortcutProperty Key="System.AppUserModel.ID" Value="{aumid}" />
        </Shortcut>
        <RegistryValue Root="HKCU" Key="Software\Clutch" Name="DesktopShortcut" Type="integer" Value="1" KeyPath="yes" />
      </Component>
    </DirectoryRef>

    <!-- The virtual camera source is registered for all users here, so the app never needs to
         ask for permission later. -->
    <CustomAction Id="RegisterVcam" Directory="INSTALLFOLDER" ExeCommand="{regsvr} /s {vcam}" Execute="deferred" Impersonate="no" Return="ignore" />
    <CustomAction Id="UnregisterVcam" Directory="INSTALLFOLDER" ExeCommand="{regsvr} /u /s {vcam}" Execute="deferred" Impersonate="no" Return="ignore" />
    <InstallExecuteSequence>
      <Custom Action="UnregisterVcam" Before="RemoveFiles">REMOVE="ALL"</Custom>
      <Custom Action="RegisterVcam" After="InstallFiles">NOT REMOVE</Custom>
    </InstallExecuteSequence>

    <Feature Id="Main" Title="Clutch" Level="1" Absent="disallow">
{component_refs}      <ComponentRef Id="cmp_StartMenu" />
    </Feature>
    <Feature Id="DesktopShortcut" Title="Desktop shortcut" Level="1">
      <ComponentRef Id="cmp_Desktop" />
    </Feature>
  </Product>
</Wix>
"#,
        icon = asset("clutch.ico"),
        license = asset("license.rtf"),
        banner = asset("banner.bmp"),
        dialog = asset("dialog.bmp"),
        start = guid("clutch:shortcut:startmenu"),
        desktop = guid("clutch:shortcut:desktop"),
        aumid = APP_USER_MODEL_ID,
    ))
}

fn license_rtf() -> String {
    let paragraphs = [
        r"\b Clutch\b0",
        "Copyright (c) the Clutch contributors. Licensed under the MIT License or the Apache License, Version 2.0, at your option.",
        "THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.",
        r"\b Third-party components\b0",
        "FFmpeg (libavcodec, libavformat and related libraries) is licensed under the GNU Lesser General Public License, version 2.1 or later, and is included as unmodified shared libraries. Its license text is in the licenses folder of the installation.",
        "Chromium Embedded Framework (CEF) and Chromium are licensed under BSD-style licenses; their credits are in the licenses folder of the installation.",
    ];
    let body: String = paragraphs
        .iter()
        .map(|p| format!(r"{p}\par\par "))
        .collect();
    format!(r"{{\rtf1\ansi\deff0{{\fonttbl{{\f0 Segoe UI;}}}}\f0\fs18 {body}}}")
}
