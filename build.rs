//! Gives ad2edit.exe its icon in Explorer: on Windows with the MSVC
//! toolchain the linker takes a compiled resource file as it is, and
//! assets/icon/pack.py writes one, so no resource compiler is needed.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon/ad2edit.res");
    let target = |name: &str| std::env::var(name).unwrap_or_default();
    if target("CARGO_CFG_TARGET_OS") == "windows" && target("CARGO_CFG_TARGET_ENV") == "msvc" {
        let resource = std::path::Path::new(&target("CARGO_MANIFEST_DIR")).join("assets").join("icon").join("ad2edit.res");
        if resource.is_file() {
            println!("cargo:rustc-link-arg-bin=ad2edit={}", resource.display());
        }
    }
}
