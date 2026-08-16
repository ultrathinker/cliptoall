fn main() {
    // macOS-only framework link for `SMAppService` (ServiceManagement).
    // The `objc2` family of crates doesn't ship with framework link
    // directives — they assume the host provides them — so we add the
    // ServiceManagement framework here. No-op on non-macOS targets.
    // See src/utils/autorun.rs for the binding.
    #[cfg(target_os = "macos")]
    println!("cargo:rustc-link-lib=framework=ServiceManagement");

    tauri_build::build()
}
