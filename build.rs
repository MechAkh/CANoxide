//! Windows resources of both exes: the app icon (assets/icon/canoxide.ico,
//! icon group 1, the one Explorer shows) and the version information from
//! Cargo.toml. winresource finds rc.exe of the Windows SDK (MSVC) itself.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon/canoxide.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon/canoxide.ico");
        res.set("ProductName", "CANoxide");
        res.set("FileDescription", "CANoxide CANopen bench tester");
        if let Err(e) = res.compile() {
            panic!("Windows resource (icon) not compiled: {e}");
        }
    }
}
