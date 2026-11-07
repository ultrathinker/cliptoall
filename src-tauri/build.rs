fn main() {
    // `cfg(plugins)` — the one switch for "the plugin system is compiled in".
    // Code gates on this alias instead of repeating the platform/feature
    // predicate everywhere. It is set:
    //   * on Windows — always (the plugin system has always shipped there);
    //   * on macOS   — only with the `personal` Cargo feature (see the comment
    //     above `[features]` in Cargo.toml for why the default is off).
    // Read from the CARGO_CFG_* / CARGO_FEATURE_* env vars, not `#[cfg]`, so
    // the decision follows the TARGET being built, not the host running this
    // script.
    println!("cargo::rustc-check-cfg=cfg(plugins)");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let personal = std::env::var_os("CARGO_FEATURE_PERSONAL").is_some();
    if target_os == "windows" || (target_os == "macos" && personal) {
        println!("cargo:rustc-cfg=plugins");
    }

    // macOS-only framework link for `SMAppService` (ServiceManagement).
    // The `objc2` family of crates doesn't ship with framework link
    // directives — they assume the host provides them — so we add the
    // ServiceManagement framework here. No-op on non-macOS targets.
    // See src/utils/autorun.rs for the binding.
    #[cfg(target_os = "macos")]
    println!("cargo:rustc-link-lib=framework=ServiceManagement");

    // Vision framework — used by commands/ocr.rs (`recognize_text`,
    // `VNRecognizeTextRequest`). Same rationale as ServiceManagement: the
    // objc2-vision crate does not emit a framework link directive, and the
    // framework is OS-supplied (no entitlement, no per-use cost). Vision
    // ships in macOS 10.15+; our floor is 14.
    #[cfg(target_os = "macos")]
    println!("cargo:rustc-link-lib=framework=Vision");

    tauri_build::build()
}
