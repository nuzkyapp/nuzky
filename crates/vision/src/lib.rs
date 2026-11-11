//! Local face and subject analysis of the rendered timeline, for covers and thumbnails (which
//! frames are worth one and where the person is) and for reframing clips to follow a face, and a scene model that
//! tells agents what a file shows and where in it the thing some words name is. Runs ONNX models on the CPU with
//! ONNX Runtime; the models are downloaded by the app or the CLI, never here.
pub mod covered;
pub mod florence;
pub mod frame;
pub mod framing;
pub mod mask;
pub mod matte;
pub mod models;
mod nets;
pub mod reframe;
pub mod runtime;
pub mod scene;
pub mod source;
pub mod thumbnails;
mod timeline;

pub use framing::Format;
pub use mask::{Mask, cached_alpha, mask_path, segment_picture, segment_subject, subject_alpha};
pub use thumbnails::{Candidate, thumbnail_frames};
