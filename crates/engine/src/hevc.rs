//! What an HEVC stream's picture parameter sets say about decoding one picture on several threads.

/// Whether every picture parameter set in `extradata` (an `hvcC` record or Annex B NAL units) splits pictures into
/// rows that decode in parallel: wavefront parallel processing, which Apple's encoder and x265 turn on. FFmpeg
/// decodes a picture of several tiles on one thread, so tiles count as no. False when there is no parameter set to
/// read.
pub(crate) fn parallel_rows(extradata: &[u8]) -> bool {
    let units =
        if extradata.len() > 22 && extradata[0] == 1 { hvcc_units(extradata) } else { annex_b_units(extradata) };
    let sets: Vec<Option<bool>> = units
        .into_iter()
        .filter(|nal| nal.first().is_some_and(|byte| byte >> 1 & 0x3f == PPS))
        .map(pps_parallel)
        .collect();
    !sets.is_empty() && sets.iter().all(|parallel| *parallel == Some(true))
}

const PPS: u8 = 34;

/// The NAL units of an `hvcC` record (ISO/IEC 14496-15): a 22 byte header, then arrays of length-prefixed units.
fn hvcc_units(data: &[u8]) -> Vec<&[u8]> {
    let mut units = Vec::new();
    let mut at = 23;
    for _ in 0..data[22] {
        let Some(&[_, high, low]) = data.get(at..at + 3) else { break };
        at += 3;
        for _ in 0..u16::from_be_bytes([high, low]) {
            let Some(&[high, low]) = data.get(at..at + 2) else { return units };
            let end = at + 2 + usize::from(u16::from_be_bytes([high, low]));
            let Some(unit) = data.get(at + 2..end) else { return units };
            units.push(unit);
            at = end;
        }
    }
    units
}

/// NAL units after `00 00 01` start codes; a unit's trailing zeros, the first byte of a 4 byte start code, do
/// not matter here.
fn annex_b_units(data: &[u8]) -> Vec<&[u8]> {
    let starts: Vec<usize> =
        (0..data.len().saturating_sub(2)).filter(|&i| data[i..i + 3] == [0, 0, 1]).map(|i| i + 3).collect();
    let ends = starts.iter().skip(1).map(|&start| start - 3).chain([data.len()]);
    starts.iter().zip(ends).map(|(&start, end)| &data[start..end]).collect()
}

/// `entropy_coding_sync_enabled_flag && !tiles_enabled_flag` of a PPS NAL unit (H.265, 7.3.2.3.1), or `None`
/// when it ends before them.
fn pps_parallel(nal: &[u8]) -> Option<bool> {
    let mut bits = Bits::new(nal.get(2..)?);
    bits.ue()?; // pps_pic_parameter_set_id
    bits.ue()?; // pps_seq_parameter_set_id
    bits.skip(1 + 1 + 3 + 1 + 1)?; // dependent slices, output flag, extra slice header bits, sign hiding, cabac init
    bits.ue()?; // num_ref_idx_l0_default_active_minus1
    bits.ue()?; // num_ref_idx_l1_default_active_minus1
    bits.ue()?; // init_qp_minus26
    bits.skip(2)?; // constrained intra prediction, transform skip
    if bits.flag()? {
        bits.ue()?; // diff_cu_qp_delta_depth
    }
    bits.ue()?; // pps_cb_qp_offset
    bits.ue()?; // pps_cr_qp_offset
    bits.skip(4)?; // slice chroma QP offsets, weighted prediction, weighted biprediction, transquant bypass
    let tiles = bits.flag()?;
    let wavefronts = bits.flag()?;
    Some(wavefronts && !tiles)
}

/// Reads the bits of a NAL unit's payload, without the emulation prevention bytes (0x000003 → 0x0000).
struct Bits<'a> {
    data: &'a [u8],
    byte: usize,
    bit: u8,
    zeros: u8,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, byte: 0, bit: 0, zeros: 0 }
    }

    fn flag(&mut self) -> Option<bool> {
        if self.bit == 0 {
            // A 3 after two zero bytes is there only so the payload never looks like a start code.
            if self.zeros >= 2 && self.data.get(self.byte) == Some(&3) {
                self.byte += 1;
                self.zeros = 0;
            }
            let byte = *self.data.get(self.byte)?;
            self.zeros = if byte == 0 { self.zeros + 1 } else { 0 };
        }
        let value = self.data[self.byte] >> (7 - self.bit) & 1 == 1;
        self.bit = (self.bit + 1) % 8;
        if self.bit == 0 {
            self.byte += 1;
        }
        Some(value)
    }

    fn skip(&mut self, count: u32) -> Option<()> {
        (0..count).try_for_each(|_| self.flag().map(drop))
    }

    /// An unsigned Exp-Golomb code; signed ones take the same bits.
    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while !self.flag()? {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        let mut value = 0u64;
        for _ in 0..zeros {
            value = value << 1 | u64::from(self.flag()?);
        }
        u32::try_from((1u64 << zeros) - 1 + value).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `hvcC` of 360p HEVC from Apple's VideoToolbox encoder (`hevc_videotoolbox`): WPP on.
    const APPLE: &str = "010160000000b000000000005af000fcfdf8f800000f03a00001001840010c01ffff01600000\
        0300b0000003000003005a170240a10001002a420101016000000300b0000003000003005aa005020171f2e205ee45914bff\
        2e7f13fac05a8101010040a2000100074401c072f45364";
    /// PPS NAL units of x265 with its default WPP and with `wpp=0`.
    const X265_WPP: &str = "4401c172b46240";
    const X265_NO_WPP: &str = "4401c172b42240";
    /// The same with `tiles_enabled_flag` set, the bit before the wavefront flag.
    const X265_WPP_TILES: &str = "4401c172b4e240";
    const X265_TILES: &str = "4401c172b4a240";

    fn bytes(hex: &str) -> Vec<u8> {
        (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect()
    }

    fn hvcc(pps: &[u8]) -> Vec<u8> {
        let mut record = bytes(APPLE)[..22].to_vec();
        record.extend([1, 0x80 | PPS, 0, 1, 0, pps.len() as u8]);
        record.extend(pps);
        record
    }

    #[test]
    fn reads_wavefronts_from_the_picture_parameter_sets() {
        assert!(parallel_rows(&bytes(APPLE)));
        assert!(parallel_rows(&hvcc(&bytes(X265_WPP))));
        assert!(!parallel_rows(&hvcc(&bytes(X265_NO_WPP))));
        let annex_b = |pps: &str| [vec![0, 0, 0, 1], bytes(pps)].concat();
        assert!(parallel_rows(&annex_b(X265_WPP)));
        assert!(!parallel_rows(&annex_b(X265_NO_WPP)));
        // One PPS without them is enough to keep decoding a picture per thread.
        assert!(!parallel_rows(&[annex_b(X265_WPP), annex_b(X265_NO_WPP)].concat()));
        // Tiles, with or without wavefronts: FFmpeg would decode each picture on one thread.
        assert!(!parallel_rows(&hvcc(&bytes(X265_WPP_TILES))));
        assert!(!parallel_rows(&hvcc(&bytes(X265_TILES))));
    }

    #[test]
    fn says_no_without_a_readable_parameter_set() {
        assert!(!parallel_rows(&[]));
        assert!(!parallel_rows(&bytes(APPLE)[..100]));
        assert!(!parallel_rows(&hvcc(&bytes(X265_WPP)[..4])));
        assert!(!parallel_rows(&[0, 0, 1, 0x44, 0x01, 0, 0, 3, 0]));
    }
}
