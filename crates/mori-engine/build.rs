// Bake the CEF distribution directory into the binary so development builds
// find libcef's resources without copying them next to the executable.
fn main() {
    if let Ok(dir) = std::env::var("DEP_CEF_DLL_WRAPPER_CEF_DIR") {
        println!("cargo::rustc-env=MORI_CEF_DIR={dir}");
    }
    println!("cargo::rerun-if-env-changed=DEP_CEF_DLL_WRAPPER_CEF_DIR");
}
