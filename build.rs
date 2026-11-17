//! Teilt dem MSVC-Linker mit, dass Toolless das Datensymbol `RTSSHooksCompatibility`
//! exportiert. RivaTuner Statistics Server (MSI Afterburner) hängt sich daraufhin nicht
//! in Toolless ein – im Overlay erscheint das Programm nicht.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if os == "windows" && env == "msvc" {
        println!("cargo:rustc-link-arg-bins=/EXPORT:RTSSHooksCompatibility,DATA");
    }
}
