//! One file's own picture at its own times, as Nuzky draws it: upright, mirrored as the file asks and HDR tone
//! mapped, without any clip's framing, colour, effects or background.
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use nuzky_engine::{
    Project, Renderer, Wait,
    edit::media,
    model::{Asset, AssetKind, Clip, ClipContent, Transform},
};

use crate::frame::Frame;

pub struct Source {
    project: Project,
    size: (u32, u32),
    duration_us: i64,
    renderer: Option<Renderer>,
}

impl Source {
    /// The file of `asset` drawn with its long side `long_side` pixels.
    pub fn new(asset: &Asset, long_side: u32) -> Result<Self> {
        if asset.kind == AssetKind::Audio {
            bail!("NO_PICTURE: {} is a sound file; it has no picture", asset.name);
        }
        ensure!(Path::new(&asset.path).is_file(), "MEDIA_MISSING: {}", asset.path);
        let k = long_side as f32 / asset.width.max(asset.height).max(1) as f32;
        let even = |v: u32| (((v as f32 * k).round() as u32).max(16) + 1) & !1;
        let size = (even(asset.width), even(asset.height));
        let duration_us = if asset.kind == AssetKind::Image { 1_000_000 } else { asset.duration_us.max(1) };
        let mut project = Project::new("source");
        (project.canvas.width, project.canvas.height) = size;
        project.canvas.background_blur = 0.0;
        project.assets = vec![asset.clone()];
        let mut content = media(asset, Transform::default(), None);
        if let ClipContent::Media { source_in_us, .. } = &mut content {
            *source_in_us = 0;
        }
        project.tracks[0].clips = vec![Clip::new("source".into(), 0, duration_us, content)];
        Ok(Self { project, size, duration_us, renderer: None })
    }

    /// The file alone on a canvas of its picture's shape, at its own times.
    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The picture at `t_us` of the file, its last frame past its end.
    pub fn frame(&mut self, t_us: i64) -> Result<Frame> {
        let t = t_us.clamp(0, self.duration_us - 1);
        let renderer = match &mut self.renderer {
            Some(renderer) => renderer,
            None => self.renderer.insert(Renderer::new().context("Starting frame renderer")?),
        };
        let (width, height) = self.size;
        let rgba = renderer
            .render(&self.project, t, width, height, Wait::Exact, false)
            .context("Drawing the file's picture")?;
        Ok(Frame { width, height, rgba })
    }
}
