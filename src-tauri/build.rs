fn main() {
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
