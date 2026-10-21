//! Updates: finds newer releases on GitHub, downloads the installer for this platform in the
//! background, checks it against GitHub's SHA-256, and installs it after Clutch closes.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// This build's version, beta number included (e.g. `0.3.5-beta.4`).
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

// ---------------------------------------------------------------- versions

/// A version such as `0.3.5` or `0.3.5-beta.4`. A beta comes before the release of the same
/// numbers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub numbers: [u64; 3],
    /// `("beta", 4)` for `-beta.4`.
    pub pre: Option<(String, u64)>,
}

impl Version {
    /// Parses `v0.3.5-beta.4`, `0.3.5` and the like.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().trim_start_matches('v');
        let (core, pre) = match text.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (text, None),
        };
        let mut numbers = [0u64; 3];
        let parts: Vec<&str> = core.split('.').collect();
        if parts.is_empty() || parts.len() > 3 {
            return None;
        }
        for (slot, part) in numbers.iter_mut().zip(&parts) {
            *slot = part.parse().ok()?;
        }
        let pre = pre.map(|p| match p.rsplit_once('.') {
            Some((name, n)) if n.parse::<u64>().is_ok() => {
                (name.to_owned(), n.parse().unwrap_or(0))
            }
            _ => (p.to_owned(), 0),
        });
        Some(Self { numbers, pre })
    }

    pub fn is_beta(&self) -> bool {
        self.pre.is_some()
    }

    /// "0.3.5 beta 4", for people.
    pub fn label(&self) -> String {
        let [a, b, c] = self.numbers;
        match &self.pre {
            Some((name, n)) => format!("{a}.{b}.{c} {name} {n}"),
            None => format!("{a}.{b}.{c}"),
        }
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.numbers
            .cmp(&other.numbers)
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub fn current() -> Version {
    Version::parse(CURRENT).unwrap_or(Version {
        numbers: [0, 0, 0],
        pre: None,
    })
}

// ---------------------------------------------------------------- releases

/// A release with an installer for this platform.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: Version,
    pub tag: String,
    /// The release page (What's new).
    pub page: String,
    pub asset_name: String,
    pub asset_url: String,
    pub size: u64,
    /// SHA-256 of the installer, lowercase hex.
    pub sha256: String,
}

/// The installer file type for this platform.
pub fn asset_extension() -> &'static str {
    if cfg!(windows) { ".msi" } else { ".AppImage" }
}

/// The newest release newer than `installed` with an installer ending in `extension`, from a
/// GitHub releases listing. Betas only count when `betas` is on. Assets without a checksum
/// are skipped (nothing installs unverified).
pub fn newest(
    listing: &Value,
    installed: &Version,
    betas: bool,
    extension: &str,
) -> Option<Release> {
    listing
        .as_array()?
        .iter()
        .filter(|r| !r["draft"].as_bool().unwrap_or(false))
        .filter_map(|r| {
            let tag = r["tag_name"].as_str()?;
            let version = Version::parse(tag)?;
            if (version.is_beta() || r["prerelease"].as_bool().unwrap_or(false)) && !betas {
                return None;
            }
            let asset = r["assets"]
                .as_array()?
                .iter()
                .find(|a| a["name"].as_str().is_some_and(|n| n.ends_with(extension)))?;
            let sha256 = asset["digest"]
                .as_str()?
                .strip_prefix("sha256:")?
                .to_lowercase();
            Some(Release {
                version,
                tag: tag.to_owned(),
                page: r["html_url"].as_str().unwrap_or_default().to_owned(),
                asset_name: asset["name"].as_str()?.to_owned(),
                asset_url: asset["browser_download_url"].as_str()?.to_owned(),
                size: asset["size"].as_u64().unwrap_or(0),
                sha256,
            })
        })
        .filter(|r| r.version > *installed)
        .max_by(|a, b| a.version.cmp(&b.version))
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(timeout))
        .user_agent(concat!("Clutch/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// Asks GitHub for the newest release for this platform.
pub fn check(betas: bool) -> Result<Option<Release>, String> {
    let url = format!(
        "https://api.github.com/repos/{}/releases?per_page=30",
        crate::services::releases_repo()
    );
    let mut response = agent(Duration::from_secs(20))
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("couldn't reach GitHub ({e})"))?;
    match response.status().as_u16() {
        200 => {}
        403 | 429 => return Err("GitHub is busy; Clutch will try again later".into()),
        404 => return Err("the releases aren't public yet".into()),
        s => return Err(format!("GitHub answered {s}")),
    }
    let listing: Value = response
        .body_mut()
        .read_json()
        .map_err(|e| format!("unexpected answer from GitHub ({e})"))?;
    Ok(newest(&listing, &current(), betas, asset_extension()))
}

/// Where the updater is, shared between its thread and the UI.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Phase {
    #[default]
    Idle,
    Checking,
    UpToDate,
    /// Found; waiting to be downloaded (Tell me mode).
    Available(Release),
    Downloading(Release),
    /// Downloaded and verified: installs when Clutch closes.
    Ready(Release, PathBuf),
    Failed(String),
}

/// Runs a check (and, when `download`, the download of what it finds) on its own thread.
/// Does nothing while another run is busy.
pub fn spawn(
    phase: &Arc<Mutex<Phase>>,
    progress: &Arc<Progress>,
    dir: PathBuf,
    betas: bool,
    download_found: bool,
    release: Option<Release>,
) {
    {
        let mut p = phase.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(*p, Phase::Checking | Phase::Downloading(_)) {
            return;
        }
        *p = match &release {
            Some(r) => Phase::Downloading(r.clone()),
            None => Phase::Checking,
        };
    }
    let (phase, progress) = (phase.clone(), progress.clone());
    let _ = std::thread::Builder::new()
        .name("clutch-update".into())
        .spawn(move || {
            let set = |p: Phase| *phase.lock().unwrap_or_else(|e| e.into_inner()) = p;
            let found = match release {
                Some(r) => r,
                None => match check(betas) {
                    Ok(Some(r)) if download_found => r,
                    Ok(Some(r)) => return set(Phase::Available(r)),
                    Ok(None) => return set(Phase::UpToDate),
                    Err(e) => {
                        tracing::info!(error = %e, "update check failed");
                        return set(Phase::Failed(e));
                    }
                },
            };
            tracing::info!(version = %found.version.label(), "downloading update");
            set(Phase::Downloading(found.clone()));
            match download(&found, &dir, &progress) {
                Ok(path) => set(Phase::Ready(found, path)),
                Err(e) => {
                    tracing::warn!(error = %e, "update download failed");
                    set(Phase::Failed(e));
                }
            }
        });
}

// ---------------------------------------------------------------- download

/// Progress of a download, shared with the UI.
#[derive(Default)]
pub struct Progress {
    pub done: Mutex<(u64, u64)>,
}

/// Downloads `release` into `dir` and checks its SHA-256. Returns the verified file.
pub fn download(
    release: &Release,
    dir: &Path,
    progress: &Arc<Progress>,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("can't write the download ({e})"))?;
    let path = dir.join(&release.asset_name);
    // Already downloaded and intact (e.g. downloaded before a restart).
    if path.exists() && sha256_of(&path).is_ok_and(|h| h == release.sha256) {
        return Ok(path);
    }
    let part = dir.join(format!("{}.part", release.asset_name));
    let mut response = agent(Duration::from_secs(60 * 30))
        .get(&release.asset_url)
        .call()
        .map_err(|e| format!("couldn't download the update ({e})"))?;
    if response.status().as_u16() != 200 {
        return Err(format!(
            "the download answered {}",
            response.status().as_u16()
        ));
    }
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(4 * 1024 * 1024 * 1024)
        .reader();
    let mut file =
        std::fs::File::create(&part).map_err(|e| format!("can't write the download ({e})"))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("the download stopped ({e})"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])
            .map_err(|e| format!("can't write the download ({e})"))?;
        done += n as u64;
        if let Ok(mut p) = progress.done.lock() {
            *p = (done, release.size);
        }
    }
    drop(file);
    let hash = hex(&hasher.finalize());
    if hash != release.sha256 {
        let _ = std::fs::remove_file(&part);
        return Err(
            "the download was damaged (its checksum didn't match), so it was deleted".into(),
        );
    }
    std::fs::rename(&part, &path).map_err(|e| format!("can't keep the download ({e})"))?;
    Ok(path)
}

fn sha256_of(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------- install

/// Starts installing `file` once this process has exited, then starts Clutch again if
/// `restart`. Returns right away; the caller should close Clutch.
#[cfg(windows)]
pub fn install_after_exit(file: &Path, restart: bool) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    let quote = |p: &Path| p.display().to_string().replace('\'', "''");
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // Wait for Clutch to close, run the installer with a progress window (Windows asks for
    // permission: Clutch is installed for all users), then start the new version.
    let mut script = format!(
        "Wait-Process -Id {pid} -ErrorAction SilentlyContinue; \
         $p = Start-Process msiexec.exe -ArgumentList '/i \"{msi}\" /passive /norestart' -Verb RunAs -Wait -PassThru; ",
        pid = std::process::id(),
        msi = quote(file),
    );
    if restart {
        script.push_str(&format!(
            "if ($p.ExitCode -eq 0) {{ Start-Process '{}' }}",
            quote(&exe)
        ));
    }
    std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("couldn't start the installer ({e})"))
}

/// Replaces the running AppImage with `file` (the running copy keeps working), then starts the
/// new one if `restart`.
#[cfg(not(windows))]
pub fn install_after_exit(file: &Path, restart: bool) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let target = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .ok_or("this copy of Clutch isn't an AppImage; download the update from the website")?;
    let staged = target.with_extension("AppImage.new");
    std::fs::copy(file, &staged).map_err(|e| format!("can't replace Clutch ({e})"))?;
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("can't replace Clutch ({e})"))?;
    std::fs::rename(&staged, &target).map_err(|e| format!("can't replace Clutch ({e})"))?;
    if restart {
        std::process::Command::new(&target)
            .spawn()
            .map_err(|e| format!("couldn't start the new version ({e})"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn versions_compare_with_betas_first() {
        assert!(v("0.3.5") > v("0.3.5-beta.4"));
        assert!(v("0.3.5-beta.4") > v("0.3.5-beta.3"));
        assert!(v("0.3.5-beta.10") > v("0.3.5-beta.9"));
        assert!(v("v0.3.5-beta.1") > v("0.3.4"));
        assert!(v("0.4.0") > v("0.3.99"));
        assert_eq!(v("v0.3.4-beta.3").label(), "0.3.4 beta 3");
        assert_eq!(v("1.2").numbers, [1, 2, 0]);
        assert!(Version::parse("latest").is_none());
    }

    fn listing() -> Value {
        let asset = |name: &str| {
            json!({"name": name, "size": 10, "digest": "sha256:ABCD",
                   "browser_download_url": format!("https://example.com/{name}")})
        };
        json!([
            {"tag_name": "v0.3.6-beta.1", "prerelease": true, "draft": false, "html_url": "p1",
             "assets": [asset("Clutch-0.3.6-beta.1-x64.msi"), asset("Clutch-0.3.6-beta.1-x86_64.AppImage")]},
            {"tag_name": "v0.3.5", "prerelease": false, "draft": false, "html_url": "p2",
             "assets": [asset("Clutch-0.3.5-x64.msi")]},
            {"tag_name": "v0.4.0", "prerelease": false, "draft": true, "html_url": "p3",
             "assets": [asset("Clutch-0.4.0-x64.msi")]},
            {"tag_name": "v0.3.4-beta.3", "prerelease": true, "draft": false, "html_url": "p4",
             "assets": [asset("Clutch-0.3.4-x64.msi")]}
        ])
    }

    #[test]
    fn picks_the_newest_matching_release() {
        let installed = v("0.3.4");
        let r = newest(&listing(), &installed, true, ".msi").unwrap();
        assert_eq!(r.tag, "v0.3.6-beta.1");
        assert_eq!(r.sha256, "abcd");
        // Without betas, the stable 0.3.5; drafts never count.
        let r = newest(&listing(), &installed, false, ".msi").unwrap();
        assert_eq!(r.tag, "v0.3.5");
        // Linux only has an AppImage in the beta.
        let r = newest(&listing(), &installed, false, ".AppImage");
        assert!(r.is_none());
        // Nothing newer than what's installed.
        assert!(newest(&listing(), &v("0.3.6-beta.1"), true, ".msi").is_none());
    }

    #[test]
    fn assets_without_a_checksum_are_skipped() {
        let listing = json!([{"tag_name": "v9.0.0", "prerelease": false, "draft": false,
            "assets": [{"name": "Clutch-9.0.0-x64.msi", "size": 1,
                        "browser_download_url": "https://example.com/x.msi"}]}]);
        assert!(newest(&listing, &v("0.1.0"), true, ".msi").is_none());
    }

    #[test]
    fn hex_is_lowercase() {
        assert_eq!(hex(&[0xab, 0x01]), "ab01");
    }
}
