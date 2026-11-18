/// Lets a binary that links nuzky-analysis find whisper.cpp's shared libraries on Linux and Windows; src/ggml.rs then
/// loads ggml's backends from the same directory. `dir` is where the build put them and `installed` where an install
/// keeps them, relative to the binary. The build scripts of every crate with such binaries include this file.
fn link_ggml(dir: Option<String>, installed: Option<&str>) {
    println!("cargo::rerun-if-changed={}", concat!(env!("CARGO_MANIFEST_DIR"), "/../analysis/link_ggml.rs"));
    let Some(dir) = dir else { return };
    let debug = std::env::var("PROFILE").is_ok_and(|profile| profile == "debug");
    if std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc") {
        // Loaded when speech first needs them, from the directory src/ggml.rs picks, so a binary started from the
        // build finds them too and Nuzky starts even without them.
        for library in ["whisper.dll", "ggml.dll", "ggml-base.dll"] {
            println!("cargo::rustc-link-arg=/DELAYLOAD:{library}");
        }
        println!("cargo::rustc-link-arg=delayimp.lib");
    } else if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "linux") {
        if let Some(installed) = installed {
            println!("cargo::rustc-link-arg=-Wl,-rpath,$ORIGIN/{installed}");
        }
        // Only a debug build looks in the build's own directory, as with ONNX Runtime (crates/vision/src/runtime.rs).
        if debug {
            println!("cargo::rustc-link-arg=-Wl,-rpath,{dir}");
        }
    }
}
