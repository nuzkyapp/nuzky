//! A wav2vec2 CTC acoustic model, as Hugging Face's `Wav2Vec2ForCTC` computes it, on the CPU from a GGUF file.
//! It turns 16 kHz mono sound into, for every 20 ms frame, how likely each letter of the model's alphabet is;
//! `align` reads the words' times from that.
//!
//! Sound and every layer's result are frames × channels, one frame after another. A convolution's window over
//! several frames is then a run of neighbouring values, so each layer is one matrix product (gemm, on all cores)
//! that reads its input where it lies.

use std::{fs::File, io::BufReader, path::Path};

use anyhow::{Context, Result, bail, ensure};
use candle_core::{DType, Device, Tensor, quantized::QTensor, quantized::gguf_file};
use gemm::Parallelism;
use rayon::prelude::*;

/// Samples per output frame and the samples one frame hears: the strides and kernels of the
/// feature encoder multiplied out.
pub(crate) const STRIDE: usize = 320;
pub(crate) const RECEPTIVE: usize = 400;

const ALL_CORES: Parallelism = Parallelism::Rayon(0);

pub(crate) struct Wav2Vec2 {
    convs: Vec<Conv>,
    /// Layer norm after every convolution (large models), or a group norm after the first only.
    layer_norm_convs: bool,
    projection_norm: Norm,
    projection: Linear,
    /// The positional convolution, one per group of channels.
    position: Vec<Conv>,
    encoder_norm: Norm,
    layers: Vec<Layer>,
    /// Large models normalise before attention and the feed-forward, base models after.
    stable: bool,
    hidden: usize,
    heads: usize,
    head: Linear,
    /// The model's alphabet, by token id.
    pub tokens: Vec<String>,
    /// The CTC blank: the model's padding token.
    pub blank: usize,
    eps: f32,
}

struct Conv {
    /// `kernel` × `inputs` rows of `outputs` values: an output frame is its window times this.
    weight: Vec<f32>,
    bias: Option<Vec<f32>>,
    kernel: usize,
    inputs: usize,
    outputs: usize,
    stride: usize,
    norm: Option<Norm>,
}

struct Norm {
    weight: Vec<f32>,
    bias: Vec<f32>,
}

/// Weights stay quantised in memory and are expanded one layer at a time, so the model takes
/// little more memory than its file while the matrix products run at full float speed.
struct Linear {
    weight: QTensor,
    bias: Vec<f32>,
}

/// A [`Linear`] with its weights expanded: `outputs` rows of `inputs` values.
struct Expanded<'a> {
    weight: Vec<f32>,
    bias: &'a [f32],
}

struct Layer {
    q: Linear,
    k: Linear,
    v: Linear,
    out: Linear,
    norm1: Norm,
    fc1: Linear,
    fc2: Linear,
    norm2: Norm,
}

/// Values read where they lie in a slice: `rows` × `cols` of them, `row_step` and `col_step` apart.
#[derive(Clone, Copy)]
struct Matrix<'a> {
    data: &'a [f32],
    rows: usize,
    cols: usize,
    row_step: usize,
    col_step: usize,
}

impl<'a> Matrix<'a> {
    /// `data` as rows of `cols` values.
    fn rows_of(data: &'a [f32], cols: usize) -> Self {
        Self::columns(data, cols, 0, cols)
    }

    /// Columns `from..from + cols` of `data`, whose rows hold `width` values.
    fn columns(data: &'a [f32], width: usize, from: usize, cols: usize) -> Self {
        Self { data: &data[from..], rows: data.len() / width, cols, row_step: width, col_step: 1 }
    }

    fn transposed(self) -> Self {
        Self { rows: self.cols, cols: self.rows, row_step: self.col_step, col_step: self.row_step, ..self }
    }
}

/// Adds `scale` × `a` × `b` to `out`, whose rows of `b.cols` values lie one after another.
fn add_product(out: &mut [f32], a: Matrix, b: Matrix, scale: f32, cores: Parallelism) {
    assert_eq!(a.cols, b.rows);
    assert_eq!(out.len(), a.rows * b.cols);
    if out.is_empty() || a.cols == 0 {
        return;
    }
    for m in [a, b] {
        assert!((m.rows - 1) * m.row_step + (m.cols - 1) * m.col_step < m.data.len());
    }
    // SAFETY: the asserts keep every read inside `a` and `b` and every write inside `out`, which
    // nothing else refers to.
    unsafe {
        gemm::gemm(
            a.rows,
            b.cols,
            a.cols,
            out.as_mut_ptr(),
            1,
            b.cols as isize,
            true,
            a.data.as_ptr(),
            a.col_step as isize,
            a.row_step as isize,
            b.data.as_ptr(),
            b.col_step as isize,
            b.row_step as isize,
            1.0,
            scale,
            false,
            false,
            false,
            cores,
        );
    }
}

impl Wav2Vec2 {
    pub fn load(path: &Path) -> Result<Self> {
        let mut file = Gguf::open(path)?;
        ensure!(
            file.content.metadata.get("general.architecture").and_then(|v| v.to_string().ok()).map(String::as_str)
                == Some("wav2vec2"),
            "Word timing model is not a wav2vec2 model"
        );
        let hidden = file.number("hidden_size")?;
        let heads = file.number("num_attention_heads")?;
        ensure!(heads > 0 && hidden % heads == 0, "Word timing model has an invalid attention shape");
        let mut convs = Vec::new();
        for i in 0..file.number("num_feat_extract_layers")? {
            let has_bias = file.content.tensor_infos.contains_key(&format!("cnn.{i}.conv.bias"));
            let bias = if has_bias { Some(file.values(&format!("cnn.{i}.conv.bias"))?) } else { None };
            let stride = file.number(&format!("conv_stride_{i}"))?;
            let norm = if file.number(&format!("cnn_has_norm_{i}"))? == 1 {
                Some(file.norm(&format!("cnn.{i}.norm"))?)
            } else {
                None
            };
            let mut groups = file.convs(&format!("cnn.{i}.conv.weight"), bias, 1, stride)?;
            ensure!(
                stride > 0 && groups[0].inputs == convs.last().map_or(1, |c: &Conv| c.outputs),
                "Word timing model has an invalid feature layer"
            );
            convs.push(Conv { norm, ..groups.remove(0) });
        }
        let mut layers = Vec::new();
        for i in 0..file.number("num_hidden_layers")? {
            let p = format!("enc.{i}");
            layers.push(Layer {
                q: file.linear(&format!("{p}.attn.q"))?,
                k: file.linear(&format!("{p}.attn.k"))?,
                v: file.linear(&format!("{p}.attn.v"))?,
                out: file.linear(&format!("{p}.attn.out"))?,
                fc1: file.linear(&format!("{p}.ffn.fc1"))?,
                fc2: file.linear(&format!("{p}.ffn.fc2"))?,
                norm1: file.norm(&format!("{p}.ln1"))?,
                norm2: file.norm(&format!("{p}.ln2"))?,
            });
        }
        let tokens: Vec<String> = file
            .content
            .metadata
            .get("tokenizer.ggml.tokens")
            .context("Word timing model lacks its alphabet")?
            .to_vec()?
            .iter()
            .map(|v| v.to_string().cloned())
            .collect::<candle_core::Result<_>>()?;
        let blank = file
            .content
            .metadata
            .get("tokenizer.ggml.padding_token_id")
            .context("Word timing model lacks its blank")?
            .to_u32()? as usize;
        let position_bias = file.values("pos_conv.bias")?;
        let position_groups = file.number("num_conv_pos_embedding_groups")?;
        let model = Self {
            convs,
            layer_norm_convs: file.number("feat_extract_norm_type")? == 1,
            projection_norm: file.norm("feat_proj.ln")?,
            projection: file.linear("feat_proj")?,
            position: file.convs("pos_conv.weight", Some(position_bias), position_groups, 1)?,
            encoder_norm: file.norm("enc.ln")?,
            layers,
            stable: file.number("do_stable_layer_norm")? == 1,
            hidden,
            heads,
            head: file.linear("lm_head")?,
            tokens,
            blank,
            eps: file.meta("layer_norm_eps")?.to_f32()?,
        };
        ensure!(
            model.blank < model.tokens.len() && model.head.bias.len() == model.tokens.len(),
            "Word timing model's alphabet does not match its output"
        );
        let features = model.convs.last().map_or(1, |conv| conv.outputs);
        let fits = |linear: &Linear, inputs| linear.weight.shape().dims() == [linear.bias.len(), inputs];
        let wide = |norm: &Norm, width| norm.weight.len() == width && norm.bias.len() == width;
        ensure!(
            model.convs.iter().all(|conv| conv.norm.as_ref().is_none_or(|norm| wide(norm, conv.outputs)))
                && wide(&model.projection_norm, features)
                && fits(&model.projection, features)
                && model.projection.bias.len() == hidden
                && model.position.iter().all(|group| group.inputs == group.outputs)
                && model.position.iter().map(|group| group.inputs).sum::<usize>() == hidden
                && wide(&model.encoder_norm, hidden)
                && model.layers.iter().all(|layer| {
                    [&layer.q, &layer.k, &layer.v, &layer.out, &layer.fc2]
                        .iter()
                        .all(|linear| linear.bias.len() == hidden)
                        && [&layer.q, &layer.k, &layer.v, &layer.out, &layer.fc1]
                            .iter()
                            .all(|linear| fits(linear, hidden))
                        && fits(&layer.fc2, layer.fc1.bias.len())
                        && wide(&layer.norm1, hidden)
                        && wide(&layer.norm2, hidden)
                })
                && fits(&model.head, hidden),
            "Word timing model's layers do not fit each other"
        );
        Ok(model)
    }

    /// Frames of `samples` samples: what the strided convolutions leave.
    pub fn frames(&self, samples: usize) -> usize {
        self.convs.iter().fold(samples, |len, conv| conv.frames(len))
    }

    /// Log-probabilities of every token in every 20 ms frame of 16 kHz mono `audio`, frames × tokens.
    /// `cancelled` is checked between layers.
    pub fn log_probs(&self, audio: &[f32], cancelled: &dyn Fn() -> bool) -> Result<Vec<f32>> {
        let stop = || -> Result<()> {
            if cancelled() {
                bail!("CANCELLED: transcription cancelled");
            }
            Ok(())
        };
        ensure!(audio.len() >= RECEPTIVE, "Too little sound for the word timing model");
        // The feature extractor's normalisation: zero mean and unit variance over the whole input.
        let mean = audio.iter().map(|&s| s as f64).sum::<f64>() / audio.len() as f64;
        let var = audio.iter().map(|&s| (s as f64 - mean).powi(2)).sum::<f64>() / audio.len() as f64;
        let scale = 1.0 / (var + 1e-7).sqrt();
        let mut x: Vec<f32> = audio.iter().map(|&s| ((s as f64 - mean) * scale) as f32).collect();
        for conv in &self.convs {
            stop()?;
            x = conv.apply(&x, ALL_CORES);
            if let Some(norm) = &conv.norm {
                if self.layer_norm_convs { norm.apply(&mut x, self.eps) } else { norm.apply_over_time(&mut x) }
            }
            gelu(&mut x);
        }
        self.projection_norm.apply(&mut x, self.eps);
        let mut h = self.projection.expand()?.apply(&x);
        stop()?;
        self.add_position(&mut h);
        if !self.stable {
            self.encoder_norm.apply(&mut h, self.eps);
        }
        for layer in &self.layers {
            stop()?;
            let [q, k, v, out, fc1, fc2] = layer.expand()?;
            let feed_forward = |x: &[f32]| {
                let mut inner = fc1.apply(x);
                gelu(&mut inner);
                fc2.apply(&inner)
            };
            if self.stable {
                let mut normed = h.clone();
                layer.norm1.apply(&mut normed, self.eps);
                add(&mut h, &out.apply(&self.attention(&q.apply(&normed), &k.apply(&normed), &v.apply(&normed))));
                normed.copy_from_slice(&h);
                layer.norm2.apply(&mut normed, self.eps);
                add(&mut h, &feed_forward(&normed));
            } else {
                let attended = out.apply(&self.attention(&q.apply(&h), &k.apply(&h), &v.apply(&h)));
                add(&mut h, &attended);
                layer.norm1.apply(&mut h, self.eps);
                let fed = feed_forward(&h);
                add(&mut h, &fed);
                layer.norm2.apply(&mut h, self.eps);
            }
        }
        if self.stable {
            self.encoder_norm.apply(&mut h, self.eps);
        }
        stop()?;
        let mut logits = self.head.expand()?.apply(&h);
        logits.par_chunks_mut(self.tokens.len()).for_each(|row| {
            let max = row.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let log_sum = row.iter().map(|&v| (v - max).exp()).sum::<f32>().ln();
            row.iter_mut().for_each(|v| *v -= max + log_sum);
        });
        Ok(logits)
    }

    /// Adds to `h` what the positional convolution reads around every frame, with as many frames of
    /// silence before the first as half its kernel, so frame `t` sits in the middle of its window.
    fn add_position(&self, h: &mut [f32]) {
        let hidden = self.hidden;
        let frames = h.len() / hidden;
        let parts: Vec<Vec<f32>> = self
            .position
            .par_iter()
            .enumerate()
            .map(|(index, group)| {
                let (width, from) = (group.inputs, index * group.inputs);
                let mut padded = vec![0.0; (frames + group.kernel - 1) * width];
                for (t, frame) in h.chunks(hidden).enumerate() {
                    let at = (group.kernel / 2 + t) * width;
                    padded[at..at + width].copy_from_slice(&frame[from..from + width]);
                }
                let mut part = group.apply(&padded, Parallelism::None);
                gelu(&mut part);
                part
            })
            .collect();
        h.par_chunks_mut(hidden).enumerate().for_each(|(t, frame)| {
            let mut from = 0;
            for (group, part) in self.position.iter().zip(&parts) {
                let width = group.outputs;
                add(&mut frame[from..from + width], &part[t * width..(t + 1) * width]);
                from += width;
            }
        });
    }

    /// What every frame takes from the others: `q`, `k` and `v` are frames × hidden, each head
    /// its own columns of them.
    fn attention(&self, q: &[f32], k: &[f32], v: &[f32]) -> Vec<f32> {
        let hidden = self.hidden;
        let (frames, width) = (q.len() / hidden, hidden / self.heads);
        let heads: Vec<Vec<f32>> = (0..self.heads)
            .into_par_iter()
            .map(|head| {
                let own = |values| Matrix::columns(values, hidden, head * width, width);
                let mut weights = vec![0.0; frames * frames];
                add_product(&mut weights, own(q), own(k).transposed(), (width as f32).powf(-0.5), Parallelism::None);
                for row in weights.chunks_mut(frames) {
                    let max = row.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                    row.iter_mut().for_each(|w| *w = (*w - max).exp());
                    let sum: f32 = row.iter().sum();
                    row.iter_mut().for_each(|w| *w /= sum);
                }
                let mut mixed = vec![0.0; frames * width];
                add_product(&mut mixed, Matrix::rows_of(&weights, frames), own(v), 1.0, Parallelism::None);
                mixed
            })
            .collect();
        let mut joined = vec![0.0; frames * hidden];
        joined.par_chunks_mut(hidden).enumerate().for_each(|(t, frame)| {
            for (head, mixed) in heads.iter().enumerate() {
                frame[head * width..(head + 1) * width].copy_from_slice(&mixed[t * width..(t + 1) * width]);
            }
        });
        joined
    }
}

struct Gguf {
    content: gguf_file::Content,
    reader: BufReader<File>,
}

impl Gguf {
    fn open(path: &Path) -> Result<Self> {
        let mut reader = BufReader::new(File::open(path).context("Opening word timing model")?);
        let content = gguf_file::Content::read(&mut reader).context("Reading word timing model")?;
        Ok(Self { content, reader })
    }

    fn meta(&self, key: &str) -> Result<&gguf_file::Value> {
        self.content.metadata.get(&format!("wav2vec2.{key}")).with_context(|| format!("Word timing model lacks {key}"))
    }

    fn number(&self, key: &str) -> Result<usize> {
        Ok(self.meta(key)?.to_u32()? as usize)
    }

    fn quantised(&mut self, name: &str) -> Result<QTensor> {
        self.content
            .tensor(&mut self.reader, name, &Device::Cpu)
            .with_context(|| format!("Reading {name} of the word timing model"))
    }

    fn values(&mut self, name: &str) -> Result<Vec<f32>> {
        values(&self.quantised(name)?.dequantize(&Device::Cpu)?)
    }

    fn norm(&mut self, name: &str) -> Result<Norm> {
        Ok(Norm { weight: self.values(&format!("{name}.weight"))?, bias: self.values(&format!("{name}.bias"))? })
    }

    fn linear(&mut self, name: &str) -> Result<Linear> {
        Ok(Linear { weight: self.quantised(&format!("{name}.weight"))?, bias: self.values(&format!("{name}.bias"))? })
    }

    /// The convolution `name`, stored as outputs × inputs × kernel, as one [`Conv`] per group of channels.
    fn convs(&mut self, name: &str, bias: Option<Vec<f32>>, groups: usize, stride: usize) -> Result<Vec<Conv>> {
        let weight = self.quantised(name)?.dequantize(&Device::Cpu)?;
        let (all_outputs, inputs, kernel) = weight.dims3()?;
        let weight = values(&weight)?;
        ensure!(
            groups > 0
                && all_outputs % groups == 0
                && all_outputs * inputs * kernel > 0
                && bias.as_ref().is_none_or(|bias| bias.len() == all_outputs),
            "Word timing model has an invalid {name}"
        );
        let outputs = all_outputs / groups;
        Ok((0..groups)
            .map(|group| {
                let own = &weight[group * outputs * inputs * kernel..];
                let mut by_window = vec![0.0; kernel * inputs * outputs];
                for (output, taps) in own.chunks(inputs * kernel).take(outputs).enumerate() {
                    for step in 0..kernel {
                        for input in 0..inputs {
                            by_window[(step * inputs + input) * outputs + output] = taps[input * kernel + step];
                        }
                    }
                }
                let bias = bias.as_ref().map(|bias| bias[group * outputs..(group + 1) * outputs].to_vec());
                Conv { weight: by_window, bias, kernel, inputs, outputs, stride, norm: None }
            })
            .collect())
    }
}

fn values(tensor: &Tensor) -> Result<Vec<f32>> {
    Ok(tensor.to_dtype(DType::F32)?.flatten_all()?.to_vec1()?)
}

impl Conv {
    fn frames(&self, length: usize) -> usize {
        if length < self.kernel { 0 } else { (length - self.kernel) / self.stride + 1 }
    }

    /// `x` is frames × inputs; the result frames × outputs, without padding.
    fn apply(&self, x: &[f32], cores: Parallelism) -> Vec<f32> {
        let frames = self.frames(x.len() / self.inputs);
        let mut out = match &self.bias {
            Some(bias) => bias.repeat(frames),
            None => vec![0.0; frames * self.outputs],
        };
        let windows = Matrix {
            data: x,
            rows: frames,
            cols: self.kernel * self.inputs,
            row_step: self.stride * self.inputs,
            col_step: 1,
        };
        add_product(&mut out, windows, Matrix::rows_of(&self.weight, self.outputs), 1.0, cores);
        out
    }
}

impl Norm {
    /// Layer norm of every frame over its channels.
    fn apply(&self, x: &mut [f32], eps: f32) {
        x.par_chunks_mut(self.weight.len()).for_each(|frame| {
            let mean = frame.iter().sum::<f32>() / frame.len() as f32;
            let var = frame.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / frame.len() as f32;
            let scale = 1.0 / (var + eps).sqrt();
            for ((v, weight), bias) in frame.iter_mut().zip(&self.weight).zip(&self.bias) {
                *v = (*v - mean) * scale * weight + bias;
            }
        });
    }

    /// A group norm with one group per channel: each channel over all frames.
    fn apply_over_time(&self, x: &mut [f32]) {
        let channels = self.weight.len();
        let frames = (x.len() / channels).max(1) as f64;
        let mut mean = vec![0f64; channels];
        for frame in x.chunks(channels) {
            mean.iter_mut().zip(frame).for_each(|(sum, &v)| *sum += f64::from(v));
        }
        mean.iter_mut().for_each(|sum| *sum /= frames);
        let mut scale = vec![0f64; channels];
        for frame in x.chunks(channels) {
            scale.iter_mut().zip(frame).zip(&mean).for_each(|((sum, &v), mean)| *sum += (f64::from(v) - mean).powi(2));
        }
        scale.iter_mut().for_each(|sum| *sum = 1.0 / (*sum / frames + 1e-5).sqrt());
        x.par_chunks_mut(channels).for_each(|frame| {
            for (c, v) in frame.iter_mut().enumerate() {
                *v = ((f64::from(*v) - mean[c]) * scale[c]) as f32 * self.weight[c] + self.bias[c];
            }
        });
    }
}

/// The exact GELU, x·Φ(x), on all cores: one call covers millions of values.
fn gelu(x: &mut [f32]) {
    x.par_chunks_mut(1 << 14).for_each(|chunk| {
        for v in chunk {
            *v = 0.5 * *v * (1.0 + libm::erff(*v * std::f32::consts::FRAC_1_SQRT_2));
        }
    });
}

fn add(to: &mut [f32], from: &[f32]) {
    to.iter_mut().zip(from).for_each(|(to, from)| *to += from);
}

impl Linear {
    fn expand(&self) -> Result<Expanded<'_>> {
        Ok(Expanded { weight: values(&self.weight.dequantize(&Device::Cpu)?)?, bias: &self.bias })
    }
}

impl Layer {
    /// The weights of `q`, `k`, `v`, `out`, `fc1` and `fc2`, each expanded on a core of its own.
    fn expand(&self) -> Result<[Expanded<'_>; 6]> {
        let linears = [&self.q, &self.k, &self.v, &self.out, &self.fc1, &self.fc2];
        let expanded = linears.into_par_iter().map(Linear::expand).collect::<Result<Vec<_>>>()?;
        expanded.try_into().map_err(|_| anyhow::anyhow!("Word timing layer did not expand"))
    }
}

impl Expanded<'_> {
    /// `x` is frames × inputs.
    fn apply(&self, x: &[f32]) -> Vec<f32> {
        let inputs = self.weight.len() / self.bias.len();
        let mut out = self.bias.repeat(x.len() / inputs);
        add_product(
            &mut out,
            Matrix::rows_of(x, inputs),
            Matrix::rows_of(&self.weight, inputs).transposed(),
            1.0,
            ALL_CORES,
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 16 kHz mono sound of a fixture as alignment hears it.
    fn heard(root: &Path, file: &str) -> Vec<f32> {
        let asset = nuzky_engine::media::probe(&root.join(file), "oracle".into()).unwrap();
        let cache = root.join("tmp-test/analysis").join(format!("oracle-{}", std::process::id()));
        let pcm = crate::audio::open_pcm(&asset, &cache, &|| false).unwrap();
        crate::audio::speech_rate(pcm.samples())
    }

    /// Our model against what the official one computes on the same sound: the same most likely
    /// token in nearly every frame, and nearly its log-probability. The 8-bit Czech weights move
    /// a frame whose two best tokens were almost tied; the 16-bit English ones match closely.
    #[test]
    #[ignore = "needs tmp-test/krysar-cs.wav and the word timing models from scripts/fixtures.sh"]
    fn hears_what_the_official_models_hear() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let audio = heard(&root, "tmp-test/krysar-cs.wav");
        let models = root.join("tmp-test/xdg/data/nuzky/models");
        for (model, oracle, mean) in [
            ("wav2vec2-xls-r-300m-cs-250-q8_0.gguf", "krysar-cs-oracle.tsv", 0.01),
            ("wav2vec2-base-960h.gguf", "krysar-cs-en-oracle.tsv", 0.001),
        ] {
            let model = Wav2Vec2::load(&models.join(model)).unwrap();
            let ours = model.log_probs(&audio, &|| false).unwrap();
            let text = std::fs::read_to_string(root.join("crates/analysis/tests/data").join(oracle)).unwrap();
            let expected: Vec<(usize, f32)> = text
                .lines()
                .filter(|l| !l.starts_with('#'))
                .map(|l| {
                    let (token, p) = l.split_once('\t').unwrap();
                    (token.parse().unwrap(), p.parse().unwrap())
                })
                .collect();
            let tokens = model.tokens.len();
            assert_eq!(ours.len(), expected.len() * tokens, "{oracle}: frames");
            let argmax = |row: &[f32]| row.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
            let rows: Vec<&[f32]> = ours.chunks(tokens).collect();
            let same = rows.iter().zip(&expected).filter(|(row, (token, _))| argmax(row) == *token).count();
            let drift = rows.iter().zip(&expected).map(|(row, &(t, p))| (row[t] - p).abs() as f64).sum::<f64>()
                / rows.len() as f64;
            assert!(
                same * 100 >= expected.len() * 99,
                "{oracle}: same top token in {same} of {} frames",
                expected.len()
            );
            assert!(drift <= mean, "{oracle}: mean log-probability drift {drift}");
        }
    }
}
