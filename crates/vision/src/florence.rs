//! Florence-2 base, fine-tuned (`models::SCENE`): it describes a picture in words, finds the region words name and
//! names what a region shows. One look at a picture (its encoder, about half a second on a laptop's CPU) serves
//! every question about it; each answer is a few hundred milliseconds of the text encoder and decoder.
//!
//! The model answers greedily, as transformers' Florence-2 does with one beam, and never repeats three tokens in a
//! row, as its generation config asks. Prompts are the model's own task prompts (its `processing_florence2.py`).
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, bail, ensure};
use ort::session::{Session, SessionInputValue};
use ort::value::{DynValue, Tensor};

use crate::frame::Frame;
use crate::models::{FLORENCE_DECODER, FLORENCE_EMBED, FLORENCE_ENCODER, FLORENCE_TOKENIZER, FLORENCE_VISION};
use crate::nets::{cancellable, ort_error, session};

/// The picture side the model was trained on; any picture is squeezed to it.
const SIDE: u32 = 768;
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];
const LAYERS: usize = 6;
const HEADS: usize = 12;
const HEAD_SIZE: usize = 64;
const BOS: i64 = 0;
const EOS: i64 = 2;
/// The decoder starts from EOS and is made to say BOS first.
const DECODER_START: i64 = EOS;
/// `<loc_0>` to `<loc_999>`: a coordinate in thousandths of the picture.
const FIRST_LOC: i64 = 50_269;
const LOCS: i64 = 1000;
/// Tokens added after the vocabulary, such as `<od>` or `<and>`, that are not coordinates.
const FIRST_ADDED: i64 = 50_265;
const NO_REPEAT: usize = 3;
/// The longest words to follow, in characters.
pub const MAX_WORDS: usize = 120;

/// "Describe in detail what is shown in the image.", the `<DETAILED_CAPTION>` task.
const DESCRIBE: &[i64] = &[BOS, 47066, 21700, 11, 4617, 99, 16, 2343, 11, 5, 2274, 4, EOS];
/// "What is the region " and "?", around the four coordinates of `<REGION_TO_CATEGORY>`.
const REGION: &[i64] = &[BOS, 2264, 16, 5, 976, 1437];
const REGION_END: &[i64] = &[116, EOS];

/// A region as fractions of the picture: left, top, width and height.
pub type Region = [f32; 4];

pub struct Florence {
    vision: Session,
    embed: Session,
    encoder: Session,
    decoder: Session,
    tokens: Tokenizer,
}

/// The picture and a prompt as the decoder reads them.
struct Encoded {
    hidden: DynValue,
    mask: DynValue,
}

/// What the model saw in one picture, asked about as often as needed.
pub struct Look {
    features: Vec<f32>,
    count: usize,
}

impl Florence {
    pub fn load(dir: &Path) -> Result<Self> {
        Ok(Self {
            vision: session(&FLORENCE_VISION.path(dir), 0)?,
            embed: session(&FLORENCE_EMBED.path(dir), 1)?,
            encoder: session(&FLORENCE_ENCODER.path(dir), 0)?,
            decoder: session(&FLORENCE_DECODER.path(dir), 0)?,
            tokens: Tokenizer::load(&FLORENCE_TOKENIZER.path(dir))?,
        })
    }

    pub fn look(&mut self, frame: &Frame, cancel: &AtomicBool) -> Result<Look> {
        let picture = frame.resized(SIDE, SIDE);
        let plane = (SIDE * SIDE) as usize;
        let mut input = vec![0f32; 3 * plane];
        for (i, p) in picture.rgba.as_chunks::<4>().0.iter().enumerate() {
            for c in 0..3 {
                input[c * plane + i] = (f32::from(p[c]) / 255.0 - MEAN[c]) / STD[c];
            }
        }
        let side = SIDE as usize;
        let mut outputs =
            run(&mut self.vision, vec![("pixel_values", tensor(vec![1, 3, side, side], input)?)], cancel)?;
        let (shape, features) = floats(take(&mut outputs, "image_features")?)?;
        ensure!(shape.len() == 3 && shape[2] == 768, "Florence-2 gave picture features of shape {shape:?}");
        Ok(Look { count: shape[1] as usize, features })
    }

    /// A few sentences on what the picture shows, in English.
    pub fn describe(&mut self, look: &Look, cancel: &AtomicBool) -> Result<String> {
        let answer = self.answer(look, DESCRIBE, 128, cancel)?;
        Ok(clean(&self.tokens.decode(&answer).text))
    }

    /// Every region the model finds for `words`, best first. It always finds something, even for what the picture
    /// lacks; ask `name` what a region shows before trusting it.
    pub fn locate(&mut self, look: &Look, words: &str, cancel: &AtomicBool) -> Result<Vec<Region>> {
        let words = words.split_whitespace().collect::<Vec<_>>().join(" ");
        ensure!(!words.is_empty(), "INVALID_ARGUMENTS: say what to follow in a few words");
        ensure!(
            words.chars().count() <= MAX_WORDS,
            "INVALID_ARGUMENTS: say what to follow in at most {MAX_WORDS} characters"
        );
        let prompt = self.tokens.encode(&format!("Locate {words} in the image."));
        let answer = self.answer(look, &prompt, 64, cancel)?;
        Ok(self.tokens.decode(&answer).regions())
    }

    /// What the model calls the region, such as "dog" or "houseplant", and whether `words` name it about as well:
    /// their tokens are at most `margin` nats a token less likely than the model's own name for it.
    pub fn name(
        &mut self,
        look: &Look,
        region: Region,
        words: &str,
        margin: f32,
        cancel: &AtomicBool,
    ) -> Result<(String, bool)> {
        let mut prompt = REGION.to_vec();
        // The inverse of `Decoded::regions`, which puts a coordinate in the middle of its thousandth.
        let bin = |v: f32| FIRST_LOC + ((v * LOCS as f32 - 0.5).round() as i64).clamp(0, LOCS - 1);
        prompt.extend([bin(region[0]), bin(region[1]), bin(region[0] + region[2]), bin(region[1] + region[3])]);
        prompt.extend(REGION_END);
        let encoded = self.encode(look, &prompt, cancel)?;
        let answer = self.generate(&encoded, 16, cancel)?;
        let named: Vec<i64> = answer.iter().copied().take_while(|&id| id < FIRST_ADDED).collect();
        let name = clean(&self.tokens.decode(&named).text);
        let asked = self.tokens.encode(words.trim());
        let asked = &asked[1..asked.len() - 1];
        let close = !named.is_empty()
            && !asked.is_empty()
            && self.likelihood(&encoded, asked, cancel)? >= self.likelihood(&encoded, &named, cancel)? - margin;
        Ok((name, close))
    }

    /// The model's tokens after `prompt` about the picture, up to `limit` of them, without BOS and EOS.
    fn answer(&mut self, look: &Look, prompt: &[i64], limit: usize, cancel: &AtomicBool) -> Result<Vec<i64>> {
        let encoded = self.encode(look, prompt, cancel)?;
        self.generate(&encoded, limit, cancel)
    }

    /// The picture and the prompt through the text encoder, which every answer about them reads.
    fn encode(&mut self, look: &Look, prompt: &[i64], cancel: &AtomicBool) -> Result<Encoded> {
        let prompt_embeds = self.embed(prompt, cancel)?;
        let length = look.count + prompt.len();
        let mut embeds = look.features.clone();
        embeds.extend(prompt_embeds);
        let mask = || tensor(vec![1, length], vec![1i64; length]);
        let mut encoded = run(
            &mut self.encoder,
            vec![("attention_mask", mask()?), ("inputs_embeds", tensor(vec![1, length, 768], embeds)?)],
            cancel,
        )?;
        Ok(Encoded { hidden: take(&mut encoded, "last_hidden_state")?, mask: mask()? })
    }

    /// One decoder run over `tokens`, with the keys and values of the tokens before them in `past`, which it
    /// replaces with theirs and those of `tokens`; the logits of every token.
    fn decode_step(
        &mut self,
        encoded: &Encoded,
        tokens: &[i64],
        past: &mut [(String, DynValue)],
        first: bool,
        cancel: &AtomicBool,
    ) -> Result<(Vec<f32>, usize)> {
        let embeds = self.embed(tokens, cancel)?;
        let mut inputs: Vec<(Cow<'_, str>, SessionInputValue<'_>)> = vec![
            ("encoder_attention_mask".into(), (&encoded.mask).into()),
            ("encoder_hidden_states".into(), (&encoded.hidden).into()),
            ("inputs_embeds".into(), tensor(vec![1, tokens.len(), 768], embeds)?.into()),
            ("use_cache_branch".into(), Tensor::from_array((vec![1usize], vec![!first])).map_err(ort_error)?.into()),
        ];
        inputs.extend(past.iter().map(|(name, value)| (Cow::from(name.as_str()), value.into())));
        let mut outputs = cancellable(cancel, |options| {
            let mut outputs = self.decoder.run_with_options(inputs, options).map_err(ort_error)?;
            let names: Vec<String> = outputs.keys().map(str::to_owned).collect();
            Ok(names.into_iter().filter_map(|name| outputs.remove(&name).map(|v| (name, v))).collect::<Vec<_>>())
        })?;
        let (shape, logits) = floats(take(&mut outputs, "logits")?)?;
        let vocabulary = *shape.last().context("Florence-2 gave no logits")? as usize;
        // The encoder's keys stay those of the first run; later runs only add the decoder's.
        for (name, value) in past.iter_mut() {
            if first || name.contains(".decoder.") {
                *value = take(&mut outputs, &name.replace("past_key_values", "present"))?;
            }
        }
        Ok((logits, vocabulary))
    }

    /// Keys and values of no tokens yet, for the decoder's first run.
    fn no_past() -> Result<Vec<(String, DynValue)>> {
        let mut past = Vec::with_capacity(LAYERS * 4);
        for layer in 0..LAYERS {
            for part in ["decoder", "encoder"] {
                for kind in ["key", "value"] {
                    past.push((
                        format!("past_key_values.{layer}.{part}.{kind}"),
                        tensor(vec![1, HEADS, 0, HEAD_SIZE], Vec::<f32>::new())?,
                    ));
                }
            }
        }
        Ok(past)
    }

    /// The answer the model gives greedily, up to `limit` tokens, without BOS and EOS.
    fn generate(&mut self, encoded: &Encoded, limit: usize, cancel: &AtomicBool) -> Result<Vec<i64>> {
        let mut past = Self::no_past()?;
        let mut said: Vec<i64> = Vec::new();
        let mut next = DECODER_START;
        for step in 0..=limit {
            let (logits, vocabulary) = self.decode_step(encoded, &[next], &mut past, step == 0, cancel)?;
            let mut last = logits[logits.len() - vocabulary..].to_vec();
            next = if step == 0 {
                BOS
            } else {
                forbid_repeats(&said, &mut last);
                last.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i as i64).unwrap_or(EOS)
            };
            if next == EOS {
                break;
            }
            if step > 0 {
                said.push(next);
            }
        }
        Ok(said)
    }

    /// The mean log-probability of each token of `answer` as the start of the model's answer.
    fn likelihood(&mut self, encoded: &Encoded, answer: &[i64], cancel: &AtomicBool) -> Result<f32> {
        let mut tokens = vec![DECODER_START, BOS];
        tokens.extend(answer);
        let (logits, vocabulary) =
            self.decode_step(encoded, &tokens[..tokens.len() - 1], &mut Self::no_past()?, true, cancel)?;
        let mut sum = 0.0;
        // Row i predicts token i + 1; the first predicts the BOS the model is made to say.
        for (i, &token) in answer.iter().enumerate() {
            let row = &logits[(i + 1) * vocabulary..(i + 2) * vocabulary];
            let max = row.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let total: f32 = row.iter().map(|l| (l - max).exp()).sum();
            sum += row[token as usize] - max - total.ln();
        }
        Ok(sum / answer.len() as f32)
    }

    fn embed(&mut self, ids: &[i64], cancel: &AtomicBool) -> Result<Vec<f32>> {
        let mut outputs = run(&mut self.embed, vec![("input_ids", tensor(vec![1, ids.len()], ids.to_vec())?)], cancel)?;
        Ok(floats(take(&mut outputs, "inputs_embeds")?)?.1)
    }
}

/// Keeps the decoder from saying the same three tokens twice: the token that would end a repeat is never chosen.
fn forbid_repeats(said: &[i64], logits: &mut [f32]) {
    if said.len() + 1 < NO_REPEAT {
        return;
    }
    let tail = &said[said.len() + 1 - NO_REPEAT..];
    for window in said.windows(NO_REPEAT) {
        if window[..NO_REPEAT - 1] == *tail
            && let Some(logit) = logits.get_mut(window[NO_REPEAT - 1] as usize)
        {
            *logit = f32::NEG_INFINITY;
        }
    }
}

fn clean(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn tensor<T: ort::value::PrimitiveTensorElementType + std::fmt::Debug + Clone + 'static>(
    shape: Vec<usize>,
    data: Vec<T>,
) -> Result<DynValue> {
    Ok(Tensor::from_array((shape, data)).map_err(ort_error)?.into_dyn())
}

fn run(
    session: &mut Session,
    inputs: Vec<(&'static str, DynValue)>,
    cancel: &AtomicBool,
) -> Result<Vec<(String, DynValue)>> {
    cancellable(cancel, |options| {
        let inputs: Vec<(Cow<'_, str>, SessionInputValue<'_>)> =
            inputs.into_iter().map(|(name, value)| (Cow::from(name), value.into())).collect();
        let mut outputs = session.run_with_options(inputs, options).map_err(ort_error)?;
        let names: Vec<String> = outputs.keys().map(str::to_owned).collect();
        Ok(names.into_iter().filter_map(|name| outputs.remove(&name).map(|v| (name, v))).collect())
    })
}

fn take(outputs: &mut Vec<(String, DynValue)>, name: &str) -> Result<DynValue> {
    let at = outputs.iter().position(|(n, _)| n == name).with_context(|| format!("Florence-2 gave no {name}"))?;
    Ok(outputs.swap_remove(at).1)
}

fn floats(value: DynValue) -> Result<(Vec<i64>, Vec<f32>)> {
    let (shape, data) = value.try_extract_tensor::<f32>().map_err(ort_error)?;
    Ok((shape.to_vec(), data.to_vec()))
}

/// Words and coordinates of an answer: the text with coordinates left out, and every coordinate in order.
pub struct Decoded {
    pub text: String,
    pub locs: Vec<i64>,
}

impl Decoded {
    /// Each run of four coordinates as a region, the order the model said them in.
    fn regions(&self) -> Vec<Region> {
        let at = |bin: i64| (bin as f32 + 0.5) / LOCS as f32;
        self.locs
            .as_chunks::<4>()
            .0
            .iter()
            .filter_map(|c| {
                let (x0, y0, x1, y1) = (at(c[0]), at(c[1]), at(c[2]), at(c[3]));
                (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
            })
            .collect()
    }
}

/// Byte-level BPE as RoBERTa's tokenizer, which Florence-2 uses: GPT-2's split into words, each word's UTF-8 bytes as
/// printable characters, then the merges in their order. Added tokens in the text are not looked for, so words the
/// user typed never become a coordinate.
pub struct Tokenizer {
    vocabulary: HashMap<String, i64>,
    names: HashMap<i64, String>,
    merges: HashMap<(String, String), usize>,
    byte_char: [char; 256],
    char_byte: HashMap<char, u8>,
}

impl Tokenizer {
    pub fn load(path: &Path) -> Result<Self> {
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).with_context(|| format!("Reading {}", path.display()))?)
                .context("Florence-2's tokenizer is not JSON")?;
        let model = &json["model"];
        let vocabulary: HashMap<String, i64> =
            serde_json::from_value(model["vocab"].clone()).context("Florence-2's tokenizer has no vocabulary")?;
        let mut merges = HashMap::new();
        for (rank, merge) in
            model["merges"].as_array().context("Florence-2's tokenizer has no merges")?.iter().enumerate()
        {
            let pair = match merge {
                serde_json::Value::String(text) => text.split_once(' ').map(|(a, b)| (a.to_owned(), b.to_owned())),
                serde_json::Value::Array(pair) => match pair.as_slice() {
                    [a, b] => {
                        Some((a.as_str().unwrap_or_default().to_owned(), b.as_str().unwrap_or_default().to_owned()))
                    }
                    _ => None,
                },
                _ => None,
            };
            let pair = pair.context("Florence-2's tokenizer has a broken merge")?;
            merges.entry(pair).or_insert(rank);
        }
        let mut names: HashMap<i64, String> = vocabulary.iter().map(|(name, &id)| (id, name.clone())).collect();
        for added in json["added_tokens"].as_array().into_iter().flatten() {
            if let (Some(id), Some(name)) = (added["id"].as_i64(), added["content"].as_str()) {
                names.insert(id, name.to_owned());
            }
        }
        if vocabulary.len() < 50_000 || names.get(&FIRST_LOC).map(String::as_str) != Some("<loc_0>") {
            bail!("Florence-2's tokenizer is not the one Nuzky expects");
        }
        let byte_char = byte_chars();
        let char_byte = byte_char.iter().enumerate().map(|(b, &c)| (c, b as u8)).collect();
        Ok(Self { vocabulary, names, merges, byte_char, char_byte })
    }

    /// Token ids of `text` between BOS and EOS.
    pub fn encode(&self, text: &str) -> Vec<i64> {
        let mut ids = vec![BOS];
        for piece in pieces(text) {
            let mut word: Vec<String> = piece.bytes().map(|b| self.byte_char[b as usize].to_string()).collect();
            while word.len() > 1 {
                let best = word
                    .windows(2)
                    .enumerate()
                    .filter_map(|(i, pair)| self.merges.get(&(pair[0].clone(), pair[1].clone())).map(|rank| (*rank, i)))
                    .min();
                let Some((_, at)) = best else { break };
                let (a, b) = (word[at].clone(), word[at + 1].clone());
                let mut merged = Vec::with_capacity(word.len());
                let mut i = 0;
                while i < word.len() {
                    if i + 1 < word.len() && word[i] == a && word[i + 1] == b {
                        merged.push(format!("{a}{b}"));
                        i += 2;
                    } else {
                        merged.push(word[i].clone());
                        i += 1;
                    }
                }
                word = merged;
            }
            ids.extend(word.iter().filter_map(|token| self.vocabulary.get(token)));
        }
        ids.push(EOS);
        ids
    }

    pub fn decode(&self, ids: &[i64]) -> Decoded {
        let (mut bytes, mut locs) = (Vec::new(), Vec::new());
        for &id in ids {
            if (FIRST_LOC..FIRST_LOC + LOCS).contains(&id) {
                locs.push(id - FIRST_LOC);
                bytes.push(b' ');
            } else if id >= FIRST_ADDED || (0..=3).contains(&id) {
                bytes.push(b' ');
            } else if let Some(name) = self.names.get(&id) {
                bytes.extend(name.chars().filter_map(|c| self.char_byte.get(&c)));
            }
        }
        Decoded { text: String::from_utf8_lossy(&bytes).into_owned(), locs }
    }
}

/// GPT-2's map of bytes to printable characters: printable Latin-1 stays, the rest moves past U+0100.
fn byte_chars() -> [char; 256] {
    let mut out = ['\0'; 256];
    let mut extra = 0u32;
    for b in 0..=255u32 {
        let printable = (0x21..=0x7E).contains(&b) || (0xA1..=0xAC).contains(&b) || (0xAE..=0xFF).contains(&b);
        out[b as usize] = if printable {
            char::from_u32(b).unwrap_or('\0')
        } else {
            extra += 1;
            char::from_u32(0xFF + extra).unwrap_or('\0')
        };
    }
    out
}

/// GPT-2's pre-tokenizer pattern, `'s|'t|'re|'ve|'m|'ll|'d| ?\p{L}+| ?\p{N}+| ?[^\s\p{L}\p{N}]+|\s+(?!\S)|\s+`,
/// as a scan: Rust's regex has no look-ahead.
fn pieces(text: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let end = |i: usize| chars.get(i).map_or(text.len(), |c| c.0);
    let (n, mut out, mut i) = (chars.len(), Vec::new(), 0);
    let letter = |c: char| c.is_alphabetic();
    let number = |c: char| c.is_numeric();
    let other = |c: char| !c.is_whitespace() && !c.is_alphabetic() && !c.is_numeric();
    while i < n {
        let c = chars[i].1;
        if c == '\'' {
            let rest: String = chars[i + 1..n.min(i + 3)].iter().map(|c| c.1).collect();
            let length = ["re", "ve", "ll"]
                .iter()
                .find(|s| rest.starts_with(**s))
                .map(|_| 3)
                .or_else(|| rest.chars().next().filter(|c| matches!(c, 's' | 't' | 'm' | 'd')).map(|_| 2));
            if let Some(length) = length {
                out.push(&text[chars[i].0..end(i + length)]);
                i += length;
                continue;
            }
        }
        let j = if c == ' ' && i + 1 < n && !chars[i + 1].1.is_whitespace() { i + 1 } else { i };
        let class: Option<fn(char) -> bool> = match chars.get(j).map(|c| c.1) {
            Some(c) if letter(c) => Some(|c| c.is_alphabetic()),
            Some(c) if number(c) => Some(|c| c.is_numeric()),
            Some(c) if other(c) => Some(|c| !c.is_whitespace() && !c.is_alphabetic() && !c.is_numeric()),
            _ => None,
        };
        if let Some(class) = class {
            let mut k = j;
            while k < n && class(chars[k].1) {
                k += 1;
            }
            out.push(&text[chars[i].0..end(k)]);
            i = k;
            continue;
        }
        // Whitespace: all of it at the end, else all but the last character, which goes with the next word.
        let mut k = i;
        while k < n && chars[k].1.is_whitespace() {
            k += 1;
        }
        let stop = if k < n && k - i > 1 { k - 1 } else { k };
        out.push(&text[chars[i].0..end(stop)]);
        i = stop;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_split_as_gpt2_splits_them() {
        assert_eq!(
            pieces("Locate the red car in the image."),
            ["Locate", " the", " red", " car", " in", " the", " image", "."]
        );
        assert_eq!(
            pieces("Locate  dogs\tand\ncats  in"),
            ["Locate", " ", " dogs", "\t", "and", "\n", "cats", " ", " in"]
        );
        assert_eq!(pieces("kůň's 3D-printed   mug!!"), ["kůň", "'s", " 3", "D", "-", "printed", "  ", " mug", "!!"]);
        assert_eq!(pieces("end  "), ["end", "  "]);
        assert!(pieces("").is_empty());
    }

    #[test]
    fn bytes_map_to_printable_characters_both_ways() {
        let chars = byte_chars();
        assert_eq!((chars[b' ' as usize], chars[b'\n' as usize], chars[b'A' as usize]), ('Ġ', 'Ċ', 'A'));
        let unique: std::collections::HashSet<_> = chars.iter().collect();
        assert_eq!(unique.len(), 256);
    }

    #[test]
    fn repeats_of_three_tokens_are_forbidden() {
        let mut logits = vec![0.0; 10];
        forbid_repeats(&[1, 2, 3, 1, 2], &mut logits);
        assert_eq!(logits[3], f32::NEG_INFINITY);
        assert!(logits.iter().filter(|l| l.is_infinite()).count() == 1);
        let mut short = vec![0.0; 10];
        forbid_repeats(&[1], &mut short);
        assert!(short.iter().all(|l| l.is_finite()));
    }

    #[test]
    fn coordinates_become_regions_and_empty_ones_drop() {
        let decoded = Decoded { text: String::new(), locs: vec![303, 613, 694, 998, 10, 10, 10, 20, 1, 2] };
        let regions = decoded.regions();
        assert_eq!(regions.len(), 1, "{regions:?}");
        let [x, y, w, h] = regions[0];
        assert!(
            (x - 0.3035).abs() < 1e-6
                && (y - 0.6135).abs() < 1e-6
                && (w - 0.391).abs() < 1e-6
                && (h - 0.385).abs() < 1e-6
        );
    }
}
