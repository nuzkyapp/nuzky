//! whisper.cpp links the Vulkan loader, which Windows has only with a graphics driver. Loading it on first use lets
//! Nuzky start without one, and crates/analysis/src/no_vulkan.rs then keeps speech on the processor.
fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc") {
        println!("cargo::rustc-link-arg=/DELAYLOAD:vulkan-1.dll");
        println!("cargo::rustc-link-arg=/INCLUDE:__pfnDliFailureHook2");
        println!("cargo::rustc-link-arg=delayimp.lib");
    }
}
