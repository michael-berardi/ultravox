fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "macos" {
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }
    tauri_build::build()
}
