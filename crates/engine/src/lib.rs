//! Nuzky engine: project model, editing, decoding, compositing, mixing and export.
//! The desktop app and the `nuzky` CLI are thin layers over this crate.

pub mod audio;
pub mod check;
pub mod credits;
pub mod dereverb;
pub mod edit;
pub mod effects;
pub mod export;
pub mod gpu;
mod hevc;
pub mod join;
pub mod level;
pub mod loudness;
pub mod matte;
pub mod media;
pub mod model;
pub mod plugin_effects;
pub mod proxy;
pub mod recap;
pub mod render;
pub mod speech;
mod stretch;
pub mod text;
pub mod thumbnail;
pub mod voice;
pub mod worker;

pub use model::Project;
pub use render::{Pending, Renderer, Wait};

/// Nuzky's own data folder: projects, models, transcripts, styles and sounds. `NUZKY_DATA_DIR` moves it,
/// also on Windows, where no environment variable moves the system's application data.
pub fn data_dir() -> Option<std::path::PathBuf> {
    match std::env::var_os("NUZKY_DATA_DIR") {
        Some(dir) if !dir.is_empty() => Some(dir.into()),
        _ => dirs::data_dir().map(|dir| dir.join("nuzky")),
    }
}
