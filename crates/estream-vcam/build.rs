//! COM entry points are exported PRIVATE (loaded by name, never linked against).

fn main() {
    println!("cargo:rerun-if-changed=estream_vcam.def");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
        && let Ok(dir) = std::env::var("CARGO_MANIFEST_DIR")
    {
        println!("cargo:rustc-cdylib-link-arg=/DEF:{dir}/estream_vcam.def");
    }
}
