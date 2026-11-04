//! FFmpeg describes HEIC photos made of tiles from 7.1 on and their rotation and mirroring from 8.0 on; Ubuntu 24.04,
//! which CI and the Linux release build on, ships 6.1, whose headers lack those fields, and Mageia 10 ships 7.1.
fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    for version in ["7_1", "8_0"] {
        println!("cargo::rustc-check-cfg=cfg(ffmpeg_{version})");
        if std::env::var(format!("DEP_FFMPEG_FFMPEG_{version}")).is_ok_and(|v| v == "true") {
            println!("cargo::rustc-cfg=ffmpeg_{version}");
        }
    }
}
