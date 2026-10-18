fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    if let Err(e) = slint_build::compile_with_config("ui/app.slint", config) {
        panic!("Slint UI failed to compile: {e:?}");
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
