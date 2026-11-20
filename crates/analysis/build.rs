include!("link_ggml.rs");

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    // whisper-rs-sys names the directory of whisper.cpp's shared libraries on Linux and Windows; macOS links them in.
    let dir = std::env::var("DEP_WHISPER_LIB_DIR").ok();
    if let Some(dir) = &dir {
        println!("cargo::rustc-env=NUZKY_GGML_DIR={dir}");
        println!("cargo::metadata=dir={dir}");
    }
    link_ggml(dir, None);
}
