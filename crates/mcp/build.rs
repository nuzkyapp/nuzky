include!("../analysis/link_ggml.rs");

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    link_ggml(std::env::var("DEP_NUZKY_GGML_DIR").ok(), None);
}
