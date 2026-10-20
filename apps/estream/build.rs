fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    if let Err(e) = slint_build::compile_with_config("ui/app.slint", config) {
        panic!("Slint UI failed to compile: {e:?}");
    }

    // Platform app credentials: from the environment, else `.secrets/clutch.env` (not in git).
    // Without them, connecting that platform says it isn't available in this build.
    println!("cargo:rerun-if-changed=../../.secrets/clutch.env");
    let file = std::fs::read_to_string("../../.secrets/clutch.env").unwrap_or_default();
    for key in [
        "CLUTCH_YOUTUBE_CLIENT_ID",
        "CLUTCH_YOUTUBE_CLIENT_SECRET",
        "CLUTCH_JAMENDO_CLIENT_ID",
        "CLUTCH_KICK_CLIENT_ID",
        "CLUTCH_KICK_CLIENT_SECRET",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
        let value = std::env::var(key).ok().or_else(|| {
            file.lines()
                .filter_map(|l| l.trim().split_once('='))
                .find(|(k, _)| k.trim() == key)
                .map(|(_, v)| v.trim().to_owned())
        });
        if let Some(value) = value.filter(|v| !v.is_empty()) {
            println!("cargo:rustc-env={key}={value}");
        }
    }

    // The app icon and version info in clutch.exe (Explorer, the taskbar, shortcuts).
    println!("cargo:rerun-if-changed=assets/clutch.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/clutch.ico")
            .set("ProductName", "Clutch")
            .set(
                "FileDescription",
                "Clutch streaming, recording and clipping studio",
            )
            .set("CompanyName", "Clutch");
        if let Err(e) = res.compile() {
            println!("cargo:warning=app icon not embedded: {e}");
        }
    }
}
