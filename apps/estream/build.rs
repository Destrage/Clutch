fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    if let Err(e) = slint_build::compile_with_config("ui/app.slint", config) {
        panic!("Slint UI failed to compile: {e:?}");
    }

    // The app icon and version info in estream.exe (Explorer, the taskbar, shortcuts).
    println!("cargo:rerun-if-changed=assets/estream.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/estream.ico")
            .set("ProductName", "eStream")
            .set("FileDescription", "eStream streaming and recording studio")
            .set("CompanyName", "eStream");
        if let Err(e) = res.compile() {
            println!("cargo:warning=app icon not embedded: {e}");
        }
    }
}
