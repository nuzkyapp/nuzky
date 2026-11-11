//! The ONNX models, where they come from and how to tell they are installed. Downloading them is
//! the caller's job (`nuzky_mcp::model_download` checks size and SHA-256), so this crate never
//! touches the network.
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Model {
    /// File name in the models directory.
    pub file: &'static str,
    /// What it does, for progress and messages.
    pub label: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

impl Model {
    pub fn path(&self, dir: &Path) -> PathBuf {
        dir.join(self.file)
    }

    /// Installed means present with the expected size; the download checked its SHA-256.
    pub fn installed(&self, dir: &Path) -> bool {
        std::fs::metadata(self.path(dir)).is_ok_and(|file| file.is_file() && file.len() == self.size)
    }
}

// Sizes and SHA-256 checked 2026-10-09 against the pinned commits below. Licences: docs/BUILDING.md.

/// YuNet face detector, MIT, OpenCV Zoo. The 2026may export takes any input size.
pub const YUNET: Model = Model {
    file: "yunet-2026may.onnx",
    label: "face detector",
    url: "https://media.githubusercontent.com/media/opencv/opencv_zoo/26cc381e4d2594bb9f47a26eb8fd96c94a13660d/models/face_detection_yunet/face_detection_yunet_2026may.onnx",
    size: 229_738,
    sha256: "ebafce4e3c118d6554634be5c27ab333b4c047a9a8c3faf1d7cf93101c22f0f0",
};

/// MediaPipe Face Mesh V2, Apache-2.0, converted from TFLite by scripts/convert-mediapipe.py and
/// published with its licence and notice at github.com/tomasmach/nuzky-models.
pub const FACE_MESH: Model = Model {
    file: "face-landmarks-v2.onnx",
    label: "face landmarks",
    url: "https://raw.githubusercontent.com/tomasmach/nuzky-models/a4f7e3c99e0b1b971d69595a1c3efb64fbc18730/face-landmarks-v2.onnx",
    size: 4_831_591,
    sha256: "6fc4bae7e3e2c5c0870e8d1b764839c7dcefee7e96c874b454da8be514e19f47",
};

/// MediaPipe Blendshape V2, Apache-2.0, converted from TFLite by scripts/convert-mediapipe.py.
pub const BLENDSHAPES: Model = Model {
    file: "face-blendshapes-v2.onnx",
    label: "face expressions",
    url: "https://raw.githubusercontent.com/tomasmach/nuzky-models/a4f7e3c99e0b1b971d69595a1c3efb64fbc18730/face-blendshapes-v2.onnx",
    size: 1_822_957,
    sha256: "74029fcef4076695dd1129d9c45a3d766532745868bd1eb3cc1deff87e7d6d66",
};

/// MediaPipe Selfie Segmenter, Apache-2.0, converted from TFLite by scripts/convert-mediapipe.py.
pub const SELFIE: Model = Model {
    file: "selfie-segmenter.onnx",
    label: "person outline",
    url: "https://raw.githubusercontent.com/tomasmach/nuzky-models/a4f7e3c99e0b1b971d69595a1c3efb64fbc18730/selfie-segmenter.onnx",
    size: 445_668,
    sha256: "7759b1df460279b03bb39813ee1cec7bc3a1a4be38006e98ddd48294c92bd9f4",
};

/// BiRefNet_lite, MIT, exported by scripts/convert-birefnet.py with native `DeformConv`: the
/// onnx-community export needed 6-7 GB and 7 s a frame, this one about 2 GB and 2 s. Too large for
/// a file in git, so it is a release asset of the models repository.
pub const BIREFNET: Model = Model {
    file: "birefnet-lite.onnx",
    label: "subject mask",
    url: "https://github.com/tomasmach/nuzky-models/releases/download/birefnet-lite-1/birefnet-lite.onnx",
    size: 188_145_755,
    sha256: "8fd304fd859a8dc999a4a93f1fb58f4c9a6bf575de64d95c2a78027e7964d7be",
};

// Florence-2 base, fine-tuned (MIT, Microsoft), as onnx-community exported it to ONNX and quantised it to 8 bits: its
// picture encoder, token embeddings, text encoder and decoder, and the tokenizer that names its words.

pub const FLORENCE_VISION: Model = Model {
    file: "florence-2-base-ft-vision-int8.onnx",
    label: "scene model, picture part",
    url: "https://huggingface.co/onnx-community/Florence-2-base-ft/resolve/e88a44eaf3791a35eae0c5a47b3dbcd36e67eb6f/onnx/vision_encoder_int8.onnx",
    size: 93_746_478,
    sha256: "d7876c1ab0f7ec11998942ca189e99a775c5a4a912b813c7745d0f6fa9343487",
};

pub const FLORENCE_EMBED: Model = Model {
    file: "florence-2-base-ft-embed-int8.onnx",
    label: "scene model, word part",
    url: "https://huggingface.co/onnx-community/Florence-2-base-ft/resolve/e88a44eaf3791a35eae0c5a47b3dbcd36e67eb6f/onnx/embed_tokens_int8.onnx",
    size: 39_390_433,
    sha256: "6b2258db1c8ee9b160576ccde3cd3814d83a2edaed0dd1c6ca9ff3c38fa62214",
};

pub const FLORENCE_ENCODER: Model = Model {
    file: "florence-2-base-ft-encoder-int8.onnx",
    label: "scene model, reading part",
    url: "https://huggingface.co/onnx-community/Florence-2-base-ft/resolve/e88a44eaf3791a35eae0c5a47b3dbcd36e67eb6f/onnx/encoder_model_int8.onnx",
    size: 43_651_493,
    sha256: "f4ad7a68f1fb875d3bcf735ea14a7021b7ba7e83baf7cf10289881b4ed6d9b85",
};

pub const FLORENCE_DECODER: Model = Model {
    file: "florence-2-base-ft-decoder-int8.onnx",
    label: "scene model, writing part",
    url: "https://huggingface.co/onnx-community/Florence-2-base-ft/resolve/e88a44eaf3791a35eae0c5a47b3dbcd36e67eb6f/onnx/decoder_model_merged_int8.onnx",
    size: 98_177_697,
    sha256: "f22f52f980c33df0efa15932c2f3db6d9d3595ce6387eca938b8cfe23dc4c641",
};

pub const FLORENCE_TOKENIZER: Model = Model {
    file: "florence-2-base-ft-tokenizer.json",
    label: "scene model, vocabulary",
    url: "https://huggingface.co/onnx-community/Florence-2-base-ft/resolve/e88a44eaf3791a35eae0c5a47b3dbcd36e67eb6f/tokenizer.json",
    size: 2_297_961,
    sha256: "d69dcdb2323e124ac4f800cb9863ddccea0d7bb11e16125e8df3bd60f2f8aeac",
};

/// What describing a clip and following what words name read. Not part of `ALL`: Get set up offers the face and
/// person models, and an agent asks for this one when it first needs it.
pub const SCENE: &[Model] = &[FLORENCE_VISION, FLORENCE_EMBED, FLORENCE_ENCODER, FLORENCE_DECODER, FLORENCE_TOKENIZER];

/// What choosing thumbnail frames reads.
pub const FRAMES: &[Model] = &[YUNET, FACE_MESH, BLENDSHAPES, SELFIE];
/// What cutting the person out of every frame for a clip's background reads.
pub const BACKGROUND: &[Model] = &[SELFIE];
/// What following a face to reframe clips reads.
pub const REFRAME: &[Model] = &[YUNET];
/// What masking the subject reads.
pub const MASK: &[Model] = &[YUNET, BIREFNET];
pub const ALL: &[Model] = &[YUNET, FACE_MESH, BLENDSHAPES, SELFIE, BIREFNET];

pub fn missing(models: &[Model], dir: &Path) -> Vec<Model> {
    models.iter().filter(|model| !model.installed(dir)).copied().collect()
}

/// Fails with `VISION_UNAVAILABLE` when ONNX Runtime cannot load, so nobody downloads models that
/// could not run, then with `MODEL_MISSING` naming what is not installed; nothing here downloads.
pub fn require(models: &[Model], dir: &Path) -> Result<()> {
    crate::runtime::require()?;
    let missing = missing(models, dir);
    if !missing.is_empty() {
        let names: Vec<_> = missing.iter().map(|m| format!("{} ({})", m.label, m.file)).collect();
        let mb = missing.iter().map(|m| m.size).sum::<u64>().div_ceil(1_000_000);
        let command =
            if missing.iter().all(|m| SCENE.contains(m)) { "nuzky vision-models scene" } else { "nuzky vision-models" };
        bail!(
            "MODEL_MISSING: {} not installed in {}. Install them ({mb} MB) with `{command}`; tools never download models.",
            names.join(", "),
            dir.display()
        );
    }
    Ok(())
}
