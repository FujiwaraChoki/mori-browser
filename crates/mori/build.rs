// Point the dynamic loader at libcef.so: next to the binary in packaged
// builds ($ORIGIN), or in the downloaded CEF distribution during development.
fn main() {
    println!("cargo::rustc-link-arg-bins=-Wl,-rpath,$ORIGIN");
    if let Ok(dir) = std::env::var("DEP_CEF_DLL_WRAPPER_CEF_DIR") {
        println!("cargo::rustc-link-arg-bins=-Wl,-rpath,{dir}");
    }
}
