//! Finding a Steam game's install folder (for writing Game State Integration configs).

use std::path::{Path, PathBuf};

/// Steam's own folder: the first of [`steam_candidates`] with a `steamapps` folder.
pub fn steam_root() -> Option<PathBuf> {
    steam_candidates(dirs::home_dir().as_deref())
        .into_iter()
        .find(|p| p.join("steamapps").is_dir())
}

/// Where Steam may live: the registry and default installs on Windows; on Linux the native
/// install (and its `~/.steam` links), Flatpak and Snap.
fn steam_candidates(home: Option<&Path>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = registry_steam_path()
        .map(PathBuf::from)
        .into_iter()
        .collect();
    if cfg!(windows) {
        out.extend([r"C:\Program Files (x86)\Steam", r"C:\Program Files\Steam"].map(PathBuf::from));
    } else if let Some(home) = home {
        out.extend(
            [
                ".steam/steam",
                ".steam/root",
                ".local/share/Steam",
                ".var/app/com.valvesoftware.Steam/.local/share/Steam",
                "snap/steam/common/.local/share/Steam",
            ]
            .map(|p| home.join(p)),
        );
    }
    out
}

#[cfg(windows)]
fn registry_steam_path() -> Option<String> {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
    use windows::core::w;
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: `buf` and `size` outlive the call; `size` is the buffer length in bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Valve\Steam"),
            w!("SteamPath"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status.is_err() {
        return None;
    }
    let len = (size as usize / 2).saturating_sub(1).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]).replace('/', "\\"))
}

#[cfg(not(windows))]
fn registry_steam_path() -> Option<String> {
    None
}

/// The quoted strings on one line of a VDF (Valve KeyValues) file, unescaped.
fn quoted(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut s = String::new();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => {
                    if let Some(n) = chars.next() {
                        s.push(n);
                    }
                }
                c => s.push(c),
            }
        }
        out.push(s);
    }
    out
}

/// Library folders listed in `steamapps/libraryfolders.vdf`, Steam's own folder first.
pub fn library_folders(steam_root: &Path) -> Vec<PathBuf> {
    let mut folders = vec![steam_root.to_path_buf()];
    let vdf = steam_root.join("steamapps").join("libraryfolders.vdf");
    let Ok(text) = std::fs::read_to_string(vdf) else {
        return folders;
    };
    for line in text.lines() {
        let q = quoted(line);
        let path = match q.as_slice() {
            [key, value] if key == "path" => Some(value),
            // Old format: "1"  "D:\\SteamLibrary"
            [key, value] if key.parse::<u32>().is_ok() && value.contains(['\\', '/', ':']) => {
                Some(value)
            }
            _ => None,
        };
        if let Some(p) = path {
            let p = PathBuf::from(p);
            if !folders.iter().any(|f| same_path(f, &p)) {
                folders.push(p);
            }
        }
    }
    folders
}

fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        p.to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    };
    norm(a) == norm(b)
}

/// Where Steam app `app_id` is installed, searching every library folder.
pub fn app_install_dir(steam_root: &Path, app_id: u32) -> Option<PathBuf> {
    library_folders(steam_root).into_iter().find_map(|lib| {
        let apps = lib.join("steamapps");
        let manifest =
            std::fs::read_to_string(apps.join(format!("appmanifest_{app_id}.acf"))).ok()?;
        let dir = manifest.lines().find_map(|l| match quoted(l).as_slice() {
            [k, v] if k.eq_ignore_ascii_case("installdir") => Some(v.clone()),
            _ => None,
        })?;
        let path = apps.join("common").join(dir);
        path.is_dir().then_some(path)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("estream-steam-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn finds_an_app_in_a_second_library() {
        let root = temp("root");
        let lib = temp("lib");
        let _ = std::fs::create_dir_all(root.join("steamapps"));
        let lib_text = lib.to_string_lossy().replace('\\', "\\\\");
        let vdf = format!(
            "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{lib_text}\"\n\t\t\"apps\"\n\t\t{{\n\t\t\t\"730\"\t\t\"123\"\n\t\t}}\n\t}}\n}}\n",
            root.to_string_lossy().replace('\\', "\\\\")
        );
        let _ = std::fs::write(root.join("steamapps").join("libraryfolders.vdf"), vdf);
        let _ =
            std::fs::create_dir_all(lib.join("steamapps/common/Counter-Strike Global Offensive"));
        let _ = std::fs::write(
            lib.join("steamapps/appmanifest_730.acf"),
            "\"AppState\"\n{\n\t\"appid\"\t\t\"730\"\n\t\"installdir\"\t\t\"Counter-Strike Global Offensive\"\n}\n",
        );
        let folders = library_folders(&root);
        assert_eq!(folders.len(), 2, "{folders:?}");
        let dir = app_install_dir(&root, 730);
        assert_eq!(
            dir,
            Some(
                lib.join("steamapps")
                    .join("common")
                    .join("Counter-Strike Global Offensive")
            )
        );
        assert_eq!(app_install_dir(&root, 570), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn finds_steam_in_a_linux_home() {
        let home = temp("home");
        assert_eq!(steam_candidates(Some(&home)).len(), 5);
        let flatpak = home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam");
        let _ = std::fs::create_dir_all(flatpak.join("steamapps"));
        let found = steam_candidates(Some(&home))
            .into_iter()
            .find(|p| p.join("steamapps").is_dir());
        assert_eq!(found, Some(flatpak));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn reads_quoted_values() {
        assert_eq!(
            quoted(r#"	"path"		"D:\\Steam Library""#),
            ["path", r"D:\Steam Library"]
        );
        assert!(quoted("{").is_empty());
    }
}
