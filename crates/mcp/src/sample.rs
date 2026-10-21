//! The sample video of the tour for new users, assets/sample: its credit, the words Nuzky's transcribe tool
//! recognised in it, so the tour cuts and captions it without a speech model, and how to tell it apart. The video
//! itself is in the app, which writes it into Nuzky's data folder.
use std::path::Path;
use std::sync::OnceLock;

use nuzky_engine::model::{Asset, Credit};
use nuzky_session::transcripts::Record;
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    /// As the project's asset keeps it, so Copy credits and the export's credits name the author.
    pub credit: Credit,
    pub bytes: u64,
    pub sha256: String,
}

pub fn manifest() -> &'static Manifest {
    static MANIFEST: OnceLock<Manifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        serde_json::from_str(include_str!("../../../assets/sample/manifest.json")).expect("valid sample manifest")
    })
}

pub fn transcript() -> Record {
    serde_json::from_str(include_str!("../../../assets/sample/transcript.json")).expect("valid sample transcript")
}

/// The asset's file is the sample, byte for byte. Only a file of the sample's size is read.
pub fn is_sample(asset: &Asset) -> bool {
    let manifest = manifest();
    let path = Path::new(&asset.path);
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() == manifest.bytes)
        && std::fs::read(path).is_ok_and(|bytes| {
            Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect::<String>() == manifest.sha256
        })
}
