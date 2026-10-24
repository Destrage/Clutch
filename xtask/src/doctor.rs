//! `cargo xtask doctor` — check that every required build tool is present and report
//! pass/fail with install instructions.  When tools are missing it offers to install them
//! automatically via `winget`, one-by-one with confirmation, or just print instructions.

use std::io::Write as _;
use std::path::Path;
use std::process::Command;

use crate::root;

// ── Check model ──────────────────────────────────────────────────────────────

/// A single dependency check result.
struct Check {
    name: &'static str,
    passed: bool,
    detail: String,
    help: &'static str,
    /// If `Some`, the tool can be installed automatically with this shell command.
    /// `None` means it requires manual steps (e.g. downloading an archive).
    install_cmd: Option<&'static str>,
}

impl Check {
    fn pass(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            passed: true,
            detail: detail.into(),
            help: "",
            install_cmd: None,
        }
    }
    fn fail(
        name: &'static str,
        detail: impl Into<String>,
        help: &'static str,
        install_cmd: Option<&'static str>,
    ) -> Self {
        Self {
            name,
            passed: false,
            detail: detail.into(),
            help,
            install_cmd,
        }
    }
}

// ── Entry point ──────────────────────────────────────────────────────────────

/// Run the doctor and print a summary.  Returns `Ok(())` when everything passed.
pub fn doctor() -> Result<(), String> {
    println!("Clutch doctor \u{2014} checking build dependencies\n");

    let checks = vec![
        check_rust(),
        check_cargo(),
        check_msvc(),
        check_llvm(),
        check_cmake(),
        check_ninja(),
        check_ffmpeg(),
    ];

    // ── Print the report ─────────────────────────────────────────────────
    let width = checks.iter().map(|c| c.name.len()).max().unwrap_or(0);
    let mut failed: Vec<&Check> = Vec::new();

    for c in &checks {
        let icon = if c.passed { "\u{2714}" } else { "\u{2718}" };
        println!("  {icon}  {:width$}  {}", c.name, c.detail, width = width);
        if !c.passed {
            println!("     \u{2192} {}", c.help);
            failed.push(c);
        }
    }
    println!();

    if failed.is_empty() {
        println!("All checks passed. You\u{2019}re ready to build!");
        return Ok(());
    }

    // ── Offer to install ─────────────────────────────────────────────────
    let (auto_installable, manual_only): (Vec<&&Check>, Vec<&&Check>) =
        failed.iter().partition(|c| c.install_cmd.is_some());

    if auto_installable.is_empty() {
        // Nothing we can auto-install; just show the summary.
        print_manual_summary(&manual_only);
        return summary_error(failed.len());
    }

    println!(
        "{} tool{} can be installed automatically:\n",
        auto_installable.len(),
        if auto_installable.len() == 1 { "" } else { "s" }
    );
    for (i, c) in auto_installable.iter().enumerate() {
        println!(
            "  {}. {}  \u{2192}  {}",
            i + 1,
            c.name,
            c.install_cmd.unwrap_or("?")
        );
    }

    if !manual_only.is_empty() {
        println!();
        print_manual_summary(&manual_only);
    }

    println!();
    println!("What would you like to do?");
    println!("  [a] Install all missing tools automatically");
    println!("  [o] Go one-by-one (confirm each)");
    println!("  [s] Skip \u{2014} I\u{2019}ll install them myself");
    print!("\nYour choice [a/o/s]: ");
    let _ = std::io::stdout().flush();

    let choice = read_line().trim().to_ascii_lowercase();

    match choice.as_str() {
        "a" => install_all(&auto_installable),
        "o" => install_one_by_one(&auto_installable),
        _ => {
            println!("\nSkipped. Run `cargo xtask doctor` again after installing.");
        }
    }

    summary_error(failed.len())
}

// ── Install flows ────────────────────────────────────────────────────────────

fn install_all(checks: &[&&Check]) {
    println!();
    for c in checks {
        let cmd = c.install_cmd.unwrap_or("");
        println!("\u{25B6} Installing {} ...", c.name);
        println!("  > {cmd}");
        run_shell(cmd);
        println!();
    }
    println!("Done. Run `cargo xtask doctor` again to verify.\n");
}

fn install_one_by_one(checks: &[&&Check]) {
    println!();
    for c in checks {
        let cmd = c.install_cmd.unwrap_or("");
        print!("Install {}? ({})  [y/n]: ", c.name, cmd);
        let _ = std::io::stdout().flush();
        let answer = read_line().trim().to_ascii_lowercase();
        if answer == "y" || answer == "yes" {
            println!("  > {cmd}");
            run_shell(cmd);
            println!();
        } else {
            println!("  Skipped {}\n", c.name);
        }
    }
    println!("Done. Run `cargo xtask doctor` again to verify.\n");
}

// ── Printing helpers ─────────────────────────────────────────────────────────

fn print_manual_summary(manual: &[&&Check]) {
    if manual.is_empty() {
        return;
    }
    let s = if manual.len() == 1 {
        "requires"
    } else {
        "require"
    };
    println!(
        "The following {} manual setup:\n",
        if manual.len() == 1 {
            format!("tool {s}")
        } else {
            format!("{} tools {s}", manual.len())
        }
    );
    for c in manual {
        println!("  \u{2022} {}:  {}", c.name, c.help);
    }
}

fn summary_error(count: usize) -> Result<(), String> {
    let s = if count == 1 { "" } else { "s" };
    Err(format!(
        "{count} check{s} failed \u{2014} fix the items above and run `cargo xtask doctor` again"
    ))
}

// ── Individual checks ────────────────────────────────────────────────────────

/// Rust compiler, must be ≥ 1.90.
fn check_rust() -> Check {
    match version_output("rustc", &["--version"]) {
        Some(v) => {
            // e.g. "rustc 1.90.0 (abcdef 2026-01-01)"
            let ver = v.split_whitespace().nth(1).unwrap_or("?");
            let major_minor: Vec<u32> = ver.split('.').filter_map(|s| s.parse().ok()).collect();
            if major_minor.len() >= 2 && (major_minor[0] > 1 || major_minor[1] >= 90) {
                Check::pass("Rust", format!("rustc {ver}"))
            } else {
                Check::fail(
                    "Rust",
                    format!("rustc {ver} (need \u{2265} 1.90)"),
                    "Run `rustup update stable`",
                    // rustup update isn't a winget command; treat as manual.
                    None,
                )
            }
        }
        None => Check::fail("Rust", "not found", "Install from https://rustup.rs", None),
    }
}

/// Cargo (presence check — if rustc works, cargo usually does too, but verify).
fn check_cargo() -> Check {
    match version_output("cargo", &["--version"]) {
        Some(v) => Check::pass("Cargo", v),
        None => Check::fail("Cargo", "not found", "Install from https://rustup.rs", None),
    }
}

/// MSVC linker (`link.exe`) and `cl.exe` — these come with VS Build Tools.
fn check_msvc() -> Check {
    if cfg!(target_os = "windows") {
        match version_output("cl.exe", &[]) {
            Some(_) => Check::pass("MSVC (cl.exe)", "found"),
            None => {
                let vswhere =
                    r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe";
                if Path::new(vswhere).exists() {
                    match version_output(
                        vswhere,
                        &[
                            "-latest",
                            "-products",
                            "*",
                            "-requires",
                            "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
                            "-property",
                            "installationPath",
                        ],
                    ) {
                        Some(path) if !path.is_empty() => {
                            Check::pass("MSVC", format!("VS found at {path}"))
                        }
                        _ => Check::fail(
                            "MSVC",
                            "Visual Studio Build Tools not found",
                            "winget install Microsoft.VisualStudio.2022.BuildTools  \
                             \u{2192} select \"Desktop development with C++\"",
                            Some(
                                "winget install --id Microsoft.VisualStudio.2022.BuildTools --override \"--add Microsoft.VisualStudio.Workload.VCTools --passive --wait\"",
                            ),
                        ),
                    }
                } else {
                    Check::fail(
                        "MSVC",
                        "Visual Studio Build Tools not found",
                        "winget install Microsoft.VisualStudio.2022.BuildTools  \
                         \u{2192} select \"Desktop development with C++\"",
                        Some(
                            "winget install --id Microsoft.VisualStudio.2022.BuildTools --override \"--add Microsoft.VisualStudio.Workload.VCTools --passive --wait\"",
                        ),
                    )
                }
            }
        }
    } else {
        match version_output("cc", &["--version"]) {
            Some(v) => {
                let first = v.lines().next().unwrap_or(&v);
                Check::pass("C compiler", first.to_string())
            }
            None => Check::fail(
                "C compiler",
                "not found",
                "Install build-essential (Ubuntu/Debian) or base-devel (Arch)",
                None,
            ),
        }
    }
}

/// LLVM / Clang — needed by bindgen for FFmpeg and CEF C bindings.
fn check_llvm() -> Check {
    let default_win = r"C:\Program Files\LLVM\bin\clang.exe";
    let from_env = std::env::var("LIBCLANG_PATH").ok();

    if cfg!(target_os = "windows") {
        if Path::new(default_win).exists() {
            match version_output(default_win, &["--version"]) {
                Some(v) => {
                    let ver = v.lines().next().unwrap_or(&v);
                    Check::pass("LLVM / Clang", ver.to_string())
                }
                None => Check::pass("LLVM / Clang", format!("found at {default_win}")),
            }
        } else if let Some(ref env_path) = from_env {
            Check::pass("LLVM / Clang", format!("LIBCLANG_PATH = {env_path}"))
        } else {
            Check::fail(
                "LLVM / Clang",
                "not found at C:\\Program Files\\LLVM",
                "Run: winget install LLVM.LLVM",
                Some(
                    "winget install --id LLVM.LLVM --accept-source-agreements --accept-package-agreements",
                ),
            )
        }
    } else {
        match version_output("clang", &["--version"]) {
            Some(v) => {
                let ver = v.lines().next().unwrap_or(&v);
                Check::pass("LLVM / Clang", ver.to_string())
            }
            None => Check::fail(
                "LLVM / Clang",
                "not found",
                "Install clang and libclang-dev from your package manager",
                None,
            ),
        }
    }
}

/// CMake — needed to build CEF's C++ wrapper.
fn check_cmake() -> Check {
    match version_output("cmake", &["--version"]) {
        Some(v) => {
            let ver = v.lines().next().unwrap_or(&v);
            Check::pass("CMake", ver.to_string())
        }
        None => Check::fail(
            "CMake",
            "not found",
            "Run: winget install Kitware.CMake",
            Some(
                "winget install --id Kitware.CMake --accept-source-agreements --accept-package-agreements",
            ),
        ),
    }
}

/// Ninja — used alongside CMake for building CEF.
fn check_ninja() -> Check {
    match version_output("ninja", &["--version"]) {
        Some(v) => Check::pass("Ninja", format!("ninja {v}")),
        None => Check::fail(
            "Ninja",
            "not found",
            "Run: winget install Ninja-build.Ninja",
            Some(
                "winget install --id Ninja-build.Ninja --accept-source-agreements --accept-package-agreements",
            ),
        ),
    }
}

/// FFmpeg 9 shared LGPL build in `third_party/ffmpeg`.
fn check_ffmpeg() -> Check {
    let ffmpeg_dir = root().join("third_party/ffmpeg");
    let lib = ffmpeg_dir.join("lib");
    let include = ffmpeg_dir.join("include");
    let bin = ffmpeg_dir.join("bin");

    if !ffmpeg_dir.exists() {
        return Check::fail(
            "FFmpeg",
            format!("{} does not exist", ffmpeg_dir.display()),
            "Download ffmpeg-n9.*-win64-lgpl-shared from https://github.com/BtbN/FFmpeg-Builds/releases \
             and extract so that third_party/ffmpeg/{bin,include,lib} exist",
            // Can't auto-install: it's a zip to extract into the right place.
            None,
        );
    }

    let mut missing = Vec::new();
    for dir in [&lib, &include, &bin] {
        if !dir.exists() {
            missing.push(
                dir.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
            );
        }
    }
    if !missing.is_empty() {
        return Check::fail(
            "FFmpeg",
            format!("missing subdirectories: {}", missing.join(", ")),
            "Download ffmpeg-n9.*-win64-lgpl-shared from https://github.com/BtbN/FFmpeg-Builds/releases \
             and extract so that third_party/ffmpeg/{bin,include,lib} exist",
            None,
        );
    }

    // Try to find a version by running the ffmpeg binary if present.
    let ffmpeg_bin = if cfg!(target_os = "windows") {
        bin.join("ffmpeg.exe")
    } else {
        bin.join("ffmpeg")
    };
    if ffmpeg_bin.exists()
        && let Some(v) = version_output(ffmpeg_bin.to_str().unwrap_or("ffmpeg"), &["-version"])
    {
        let ver = v.lines().next().unwrap_or(&v);
        return Check::pass("FFmpeg", ver.to_string());
    }

    Check::pass(
        "FFmpeg",
        format!("{} (bin, include, lib present)", ffmpeg_dir.display()),
    )
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Run a command and return the first line(s) of its stdout, trimmed.
/// Returns `None` if the command doesn't exist or fails.
fn version_output(program: &str, args: &[&str]) -> Option<String> {
    Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .ok()
        .and_then(|o| {
            // Some tools (e.g. cl.exe) write version to stderr.
            let text = if o.stdout.is_empty() {
                String::from_utf8_lossy(&o.stderr).to_string()
            } else {
                String::from_utf8_lossy(&o.stdout).to_string()
            };
            let trimmed = text.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
}

/// Run a shell command string (inheriting stdin/stdout/stderr so the user sees progress).
fn run_shell(cmd: &str) {
    let shell = if cfg!(target_os = "windows") {
        ("cmd.exe", "/C")
    } else {
        ("sh", "-c")
    };
    let status = Command::new(shell.0).args([shell.1, cmd]).status();
    match status {
        Ok(s) if s.success() => println!("  \u{2714} Success"),
        Ok(s) => println!("  \u{2718} Command exited with {s}"),
        Err(e) => println!("  \u{2718} Failed to run: {e}"),
    }
}

/// Read one line from stdin.
fn read_line() -> String {
    let mut buf = String::new();
    let _ = std::io::stdin().read_line(&mut buf);
    buf
}
