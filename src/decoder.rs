use crate::bitreader::BitReader;
use crate::error::AecError;
use crate::params::{AecFlags, AecParams};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flush {
    /// Like `AEC_NO_FLUSH`: decoding may continue once more input is provided.
    NoFlush,
    /// Like `AEC_FLUSH`: the caller asserts no more input will be provided.
    Flush,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeStatus {
    /// More input is required to make progress.
    NeedInput,
    /// The output buffer was filled; provide more output space to continue.
    NeedOutput,
    /// Finished decoding `output_samples`.
    Finished,
}

/// Streaming AEC decoder (Rust-idiomatic, modeled after libaec's `aec_stream`).
///
/// This type allows chunked input and chunked output:
///
/// - call [`Decoder::push_input`] to append more bytes
/// - call [`Decoder::decode`] to write decoded bytes into a caller buffer
///
/// Notes:
/// - Output is **packed sample bytes** (same as [`decode_into`]).
/// - You must know `output_samples` up front (same as one-shot API).
pub struct Decoder {
    params: AecParams,
    bytes_per_sample: usize,
    id_len: usize,
    preprocess: bool,

    output_samples: usize,
    samples_written: usize,

    // Predictor state (only used with preprocessing enabled).
    predictor_x: Option<i64>,
    sample_index_within_rsi: u64,
    block_index_within_rsi: u32,

    // Input bitstream.
    reader: StreamBitReader,

    // Pending output from a partially-flushed decoded block.
    pending: Vec<u8>,
    pending_pos: usize,

    // Pending repeated coded values (used for zero-run etc.).
    pending_repeat: Option<PendingRepeat>,

    total_in: usize,
    total_out: usize,
}

#[derive(Debug, Clone)]
struct PendingRepeat {
    coded_value: u32,
    remaining: usize,
}

impl Decoder {
    pub fn new(params: AecParams, output_samples: usize) -> Result<Self, AecError> {
        validate_params(params)?;
        let bytes_per_sample = bytes_per_sample(params)?;
        let id_len = id_len(params)?;

        Ok(Self {
            params,
            bytes_per_sample,
            id_len,
            preprocess: params.flags.contains(AecFlags::DATA_PREPROCESS),
            output_samples,
            samples_written: 0,
            predictor_x: None,
            sample_index_within_rsi: 0,
            block_index_within_rsi: 0,
            reader: StreamBitReader::new(),
            pending: Vec::new(),
            pending_pos: 0,
            pending_repeat: None,
            total_in: 0,
            total_out: 0,
        })
    }

    /// Append more bytes to the input buffer.
    pub fn push_input(&mut self, input: &[u8]) {
        self.reader.push(input);
    }

    /// Total number of input bytes consumed so far.
    pub fn total_in(&self) -> usize {
        self.total_in
    }

    /// Total number of output bytes produced so far.
    pub fn total_out(&self) -> usize {
        self.total_out
    }

    /// Bytes currently buffered and available for reading.
    pub fn avail_in(&self) -> usize {
        self.reader.avail_bytes()
    }

    /// Decode into `out` and return (written_bytes, status).
    pub fn decode(&mut self, out: &mut [u8], flush: Flush) -> Result<(usize, DecodeStatus), AecError> {
        if self.samples_written >= self.output_samples {
            return Ok((0, DecodeStatus::Finished));
        }

        let mut written: usize = 0;

        // Fast-path: flush any pending bytes first.
        written += self.flush_pending(out, written);
        if written >= out.len() {
            self.total_out += written;
            return Ok((written, DecodeStatus::NeedOutput));
        }

        // Then flush any pending repeat-run.
        if let Some(status) = self.flush_repeat(out, &mut written)? {
            self.total_out += written;
            return Ok((written, status));
        }

        // Decode blocks/runs until output is full or decoding completes.
        while written < out.len() {
            if self.samples_written >= self.output_samples {
                self.total_out += written;
                return Ok((written, DecodeStatus::Finished));
            }

            // Ensure predictor state is reset at RSI boundary when preprocessing is enabled.
            if self.preprocess && self.block_index_within_rsi == 0 {
                self.predictor_x = None;
            }

            // If we don't have enough input to decode the next unit, request more.
            let snapshot = self.snapshot();
            match self.decode_next_unit() {
                Ok(()) => {
                    // Compaction: count consumed whole bytes.
                    let consumed = self.reader.compact_consumed_bytes();
                    self.total_in += consumed;

                    // Flush any newly produced pending output/repeat.
                    written += self.flush_pending(out, written);
                    if written >= out.len() {
                        self.total_out += written;
                        return Ok((written, DecodeStatus::NeedOutput));
                    }

                    if let Some(status) = self.flush_repeat(out, &mut written)? {
                        self.total_out += written;
                        return Ok((written, status));
                    }

                    // Otherwise, loop and decode more.
                }
                Err(AecError::UnexpectedEof { .. }) | Err(AecError::UnexpectedEofDuringDecode { .. }) => {
                    // Restore state and request more input unless flushing.
                    self.restore(snapshot);
                    self.total_out += written;
                    return match flush {
                        Flush::NoFlush => Ok((written, DecodeStatus::NeedInput)),
                        Flush::Flush => Err(AecError::UnexpectedEofDuringDecode {
                            bit_pos: self.reader.bits_read_total(),
                            samples_written: self.samples_written,
                        }),
                    };
                }
                Err(e) => {
                    self.restore(snapshot);
                    return Err(e);
                }
            }
        }

        self.total_out += written;
        Ok((written, DecodeStatus::NeedOutput))
    }

    fn flush_pending(&mut self, out: &mut [u8], written: usize) -> usize {
        if self.pending_pos >= self.pending.len() {
            self.pending.clear();
            self.pending_pos = 0;
            return 0;
        }

        let available = out.len().saturating_sub(written);
        let remaining = self.pending.len().saturating_sub(self.pending_pos);
        let to_copy = available.min(remaining);

        out[written..written + to_copy]
            .copy_from_slice(&self.pending[self.pending_pos..self.pending_pos + to_copy]);
        self.pending_pos += to_copy;
        to_copy
    }

    fn flush_repeat(&mut self, out: &mut [u8], written: &mut usize) -> Result<Option<DecodeStatus>, AecError> {
        let Some(rep) = self.pending_repeat.as_mut() else {
            return Ok(None);
        };

        while *written < out.len() && rep.remaining > 0 {
            if self.samples_written >= self.output_samples {
                self.pending_repeat = None;
                return Ok(Some(DecodeStatus::Finished));
            }

            // Write exactly one sample (packed bytes).
            let out_start = *written;
            let out_end = out_start + self.bytes_per_sample;
            if out_end > out.len() {
                return Ok(Some(DecodeStatus::NeedOutput));
            }

            // Use the same semantics as emit_coded_value(): preprocessing applies here.
            let mut tmp = OutBuf::new(&mut out[out_start..out_end], self.bytes_per_sample);
            tmp.pos = 0;
            emit_coded_value(
                &mut tmp,
                &mut self.predictor_x,
                self.params,
                self.bytes_per_sample,
                rep.coded_value,
                &mut self.sample_index_within_rsi,
                usize::MAX,
            )?;
            *written += self.bytes_per_sample;
            self.samples_written += 1;
            rep.remaining -= 1;
        }

        if rep.remaining == 0 {
            self.pending_repeat = None;
        }

        if *written >= out.len() {
            return Ok(Some(DecodeStatus::NeedOutput));
        }
        Ok(None)
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            predictor_x: self.predictor_x,
            sample_index_within_rsi: self.sample_index_within_rsi,
            block_index_within_rsi: self.block_index_within_rsi,
            samples_written: self.samples_written,
            reader: self.reader.clone(),
            pending: self.pending.clone(),
            pending_pos: self.pending_pos,
            pending_repeat: self.pending_repeat.clone(),
        }
    }

    fn restore(&mut self, s: Snapshot) {
        self.predictor_x = s.predictor_x;
        self.sample_index_within_rsi = s.sample_index_within_rsi;
        self.block_index_within_rsi = s.block_index_within_rsi;
        self.samples_written = s.samples_written;
        self.reader = s.reader;
        self.pending = s.pending;
        self.pending_pos = s.pending_pos;
        self.pending_repeat = s.pending_repeat;
    }

    fn decode_next_unit(&mut self) -> Result<(), AecError> {
        // Ensure no pending bytes: we decode one unit into pending.
        if self.pending_pos < self.pending.len() {
            return Ok(());
        }

        // Build a small output buffer for a single block.
        let mut block_out: Vec<u8> = vec![0u8; self.bytes_per_sample * (self.params.block_size as usize)];
        let mut out = OutBuf::new(&mut block_out, self.bytes_per_sample);

        // Start-of-RSI predictor reset.
        if self.preprocess && self.block_index_within_rsi == 0 {
            self.predictor_x = None;
        }

        let at_rsi_start = self.preprocess && self.block_index_within_rsi == 0;
        let ref_pending = at_rsi_start;
        let mut reference_sample_consumed = false;

        // Read block option id.
        let id = self.reader.read_bits_u32(self.id_len)?;
        let max_id = (1u32 << self.id_len) - 1;

        // Helper to consume the RSI reference sample.
        let mut consume_reference = |this: &mut Self, out: &mut OutBuf<'_>| -> Result<(), AecError> {
            let ref_raw = this.reader.read_bits_u32(this.params.bits_per_sample as usize)?;
            let ref_val = if this.params.flags.contains(AecFlags::DATA_SIGNED) {
                sign_extend(ref_raw, this.params.bits_per_sample)
            } else {
                ref_raw as i64
            };
            write_sample(out, ref_val, this.params)?;
            this.predictor_x = Some(ref_val);
            reference_sample_consumed = true;
            this.sample_index_within_rsi += 1;
            Ok(())
        };

        let remaining_total_samples = self.output_samples.saturating_sub(self.samples_written);
        let max_samples_this_block = (self.params.block_size as usize).min(remaining_total_samples);

        if id == 0 {
            // Low-entropy family.
            let selector = self.reader.read_bit()?;

            // For low-entropy blocks, selector comes before optional RSI reference.
            if ref_pending {
                consume_reference(self, &mut out)?;
                self.samples_written += 1;
            }

            // Remaining capacity after the optional reference sample.
            let remaining_total_samples = self.output_samples.saturating_sub(self.samples_written);

            let mut remaining_in_block = self.params.block_size as usize;
            if reference_sample_consumed {
                remaining_in_block = remaining_in_block.saturating_sub(1);
            }

            if !selector {
                // Zero-block run: do not materialize huge output; schedule repeats.
                let fs = read_unary_stream(&mut self.reader)?;
                let mut z_blocks = fs + 1;
                const ROS: u32 = 5;
                if z_blocks == ROS {
                    let b = self.block_index_within_rsi;
                    let fill1 = self.params.rsi.saturating_sub(b);
                    let fill2 = 64u32.saturating_sub(b % 64);
                    z_blocks = fill1.min(fill2);
                } else if z_blocks > ROS {
                    z_blocks = z_blocks.saturating_sub(1);
                }

                let mut zeros_samples = (z_blocks as usize)
                    .checked_mul(self.params.block_size as usize)
                    .ok_or(AecError::InvalidInput("zero-run overflow"))?;
                if reference_sample_consumed {
                    zeros_samples = zeros_samples.saturating_sub(1);
                }

                // Limit to remaining total samples (reference already counted in `samples_written`).
                zeros_samples = zeros_samples.min(remaining_total_samples);

                // Emit any already-written reference sample into pending bytes.
                let produced_len = out.len();
                drop(out);
                self.pending = block_out[..produced_len].to_vec();
                self.pending_pos = 0;

                // Schedule coded-value repeats (coded_value = 0).
                if zeros_samples > 0 {
                    self.pending_repeat = Some(PendingRepeat { coded_value: 0, remaining: zeros_samples });
                }

                // Advance block counter by z_blocks.
                self.block_index_within_rsi = self.block_index_within_rsi.saturating_add(z_blocks);
                if self.block_index_within_rsi >= self.params.rsi {
                    self.block_index_within_rsi %= self.params.rsi;
                    if self.params.flags.contains(AecFlags::PAD_RSI) {
                        self.reader.align_to_byte();
                    }
                    self.sample_index_within_rsi = 0;
                }

                // We do not increment samples_written here; repeats are accounted for in flush.
                return Ok(());
            }

            // Second Extension option.
            let mut produced_samples = 0usize;
            while remaining_in_block > 0 && produced_samples < max_samples_this_block.saturating_sub(reference_sample_consumed as usize) {
                let m = read_unary_stream(&mut self.reader)?;
                if m > 90 {
                    return Err(AecError::InvalidInput("Second Extension unary symbol too large"));
                }
                let (a, b) = second_extension_pair(m);

                // Emit up to two values.
                if produced_samples < max_samples_this_block.saturating_sub(reference_sample_consumed as usize) {
                    emit_coded_value(
                        &mut out,
                        &mut self.predictor_x,
                        self.params,
                        self.bytes_per_sample,
                        a,
                        &mut self.sample_index_within_rsi,
                        usize::MAX,
                    )?;
                    produced_samples += 1;
                    self.samples_written += 1;
                }

                if remaining_in_block > 0 {
                    remaining_in_block = remaining_in_block.saturating_sub(1);
                }
                if produced_samples < max_samples_this_block.saturating_sub(reference_sample_consumed as usize) {
                    emit_coded_value(
                        &mut out,
                        &mut self.predictor_x,
                        self.params,
                        self.bytes_per_sample,
                        b,
                        &mut self.sample_index_within_rsi,
                        usize::MAX,
                    )?;
                    produced_samples += 1;
                    self.samples_written += 1;
                }
                if remaining_in_block > 0 {
                    remaining_in_block = remaining_in_block.saturating_sub(1);
                }
            }
        } else if id == max_id {
            // Uncompressed block.
            if ref_pending {
                consume_reference(self, &mut out)?;
                self.samples_written += 1;
            }

            let mut remaining_in_block = self.params.block_size as usize;
            if reference_sample_consumed {
                remaining_in_block = remaining_in_block.saturating_sub(1);
            }

            for _ in 0..remaining_in_block {
                if self.samples_written >= self.output_samples {
                    break;
                }
                let v = self.reader.read_bits_u32(self.params.bits_per_sample as usize)?;
                emit_coded_value(
                    &mut out,
                    &mut self.predictor_x,
                    self.params,
                    self.bytes_per_sample,
                    v,
                    &mut self.sample_index_within_rsi,
                    usize::MAX,
                )?;
                self.samples_written += 1;
            }
        } else {
            // Rice split.
            let k = (id - 1) as usize;
            if ref_pending {
                consume_reference(self, &mut out)?;
                self.samples_written += 1;
            }

            let mut remaining_in_block = self.params.block_size as usize;
            if reference_sample_consumed {
                remaining_in_block = remaining_in_block.saturating_sub(1);
            }
            let n = remaining_in_block.min(self.output_samples.saturating_sub(self.samples_written));
            let mut tmp: Vec<u32> = vec![0u32; n];

            for i in 0..n {
                let q = read_unary_stream(&mut self.reader)?;
                tmp[i] = (q as u32)
                    .checked_shl(k as u32)
                    .ok_or(AecError::InvalidInput("rice shift overflow"))?;
            }
            if k > 0 {
                for i in 0..n {
                    let rem = self.reader.read_bits_u32(k)?;
                    tmp[i] |= rem;
                }
            }
            for v in tmp {
                if self.samples_written >= self.output_samples {
                    break;
                }
                emit_coded_value(
                    &mut out,
                    &mut self.predictor_x,
                    self.params,
                    self.bytes_per_sample,
                    v,
                    &mut self.sample_index_within_rsi,
                    usize::MAX,
                )?;
                self.samples_written += 1;
            }
        }

        // Commit block output.
        let produced_len = out.len();
        drop(out);
        self.pending = block_out[..produced_len].to_vec();
        self.pending_pos = 0;

        // Advance block counter.
        self.block_index_within_rsi = self.block_index_within_rsi.saturating_add(1);
        if self.preprocess && self.block_index_within_rsi >= self.params.rsi {
            self.block_index_within_rsi = 0;
            self.sample_index_within_rsi = 0;
            if self.params.flags.contains(AecFlags::PAD_RSI) {
                self.reader.align_to_byte();
            }
        }

        Ok(())
    }
}

#[derive(Clone)]
struct Snapshot {
    predictor_x: Option<i64>,
    sample_index_within_rsi: u64,
    block_index_within_rsi: u32,
    samples_written: usize,
    reader: StreamBitReader,
    pending: Vec<u8>,
    pending_pos: usize,
    pending_repeat: Option<PendingRepeat>,
}

/// Streaming-capable bit reader backed by an internal buffer.
///
/// It allows appending input incrementally and compacting consumed bytes.
#[derive(Debug, Clone)]
struct StreamBitReader {
    buf: Vec<u8>,
    bit_pos: usize,
    total_bytes_dropped: usize,
}

impl StreamBitReader {
    fn new() -> Self {
        Self { buf: Vec::new(), bit_pos: 0, total_bytes_dropped: 0 }
    }

    fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    fn avail_bytes(&self) -> usize {
        self.buf.len().saturating_sub(self.bit_pos / 8)
    }

    fn bits_read_total(&self) -> usize {
        self.total_bytes_dropped * 8 + self.bit_pos
    }

    fn align_to_byte(&mut self) {
        let rem = self.bit_pos % 8;
        if rem != 0 {
            self.bit_pos += 8 - rem;
        }
    }

    fn read_bit(&mut self) -> Result<bool, AecError> {
        Ok(self.read_bits_u32(1)? != 0)
    }

    fn read_bits_u32(&mut self, nbits: usize) -> Result<u32, AecError> {
        if nbits == 0 {
            return Ok(0);
        }
        if nbits > 32 {
            return Err(AecError::InvalidInput("read_bits_u32 supports up to 32 bits"));
        }

        let mut out: u32 = 0;
        for _ in 0..nbits {
            let byte_idx = self.bit_pos / 8;
            let bit_in_byte = self.bit_pos % 8;
            let byte = *self
                .buf
                .get(byte_idx)
                .ok_or(AecError::UnexpectedEof { bit_pos: self.bits_read_total() })?;
            let bit = (byte >> (7 - bit_in_byte)) & 1;
            out = (out << 1) | (bit as u32);
            self.bit_pos += 1;
        }
        Ok(out)
    }

    fn compact_consumed_bytes(&mut self) -> usize {
        let bytes = self.bit_pos / 8;
        if bytes == 0 {
            return 0;
        }
        self.buf.drain(0..bytes);
        self.bit_pos -= bytes * 8;
        self.total_bytes_dropped += bytes;
        bytes
    }
}

fn read_unary_stream(r: &mut StreamBitReader) -> Result<u32, AecError> {
    let mut count: u32 = 0;
    loop {
        let bit = r.read_bit()?;
        if bit {
            return Ok(count);
        }
        count = count.saturating_add(1);
        if count > 1_000_000 {
            return Err(AecError::InvalidInput("unary run too long"));
        }
    }
}

struct OutBuf<'a> {
    buf: &'a mut [u8],
    pos: usize,
    bytes_per_sample: usize,
}

impl<'a> OutBuf<'a> {
    fn new(buf: &'a mut [u8], bytes_per_sample: usize) -> Self {
        Self { buf, pos: 0, bytes_per_sample }
    }

    fn len(&self) -> usize {
        self.pos
    }

    fn capacity(&self) -> usize {
        self.buf.len()
    }

    fn samples_written(&self) -> usize {
        self.pos / self.bytes_per_sample
    }
}

pub fn decode(input: &[u8], params: AecParams, output_samples: usize) -> Result<Vec<u8>, AecError> {
    validate_params(params)?;

    let bytes_per_sample = bytes_per_sample(params)?;
    let output_bytes = output_samples
        .checked_mul(bytes_per_sample)
        .ok_or(AecError::InvalidInput("output too large"))?;

    let mut out = vec![0u8; output_bytes];
    decode_into(input, params, output_samples, &mut out)?;
    Ok(out)
}

pub fn decode_into(
    input: &[u8],
    params: AecParams,
    output_samples: usize,
    output: &mut [u8],
) -> Result<(), AecError> {
    validate_params(params)?;

    let trace_sample: Option<usize> = std::env::var("RUST_AEC_TRACE_SAMPLE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok());

    let bytes_per_sample = bytes_per_sample(params)?;
    let output_bytes = output_samples
        .checked_mul(bytes_per_sample)
        .ok_or(AecError::InvalidInput("output too large"))?;

    if output.len() != output_bytes {
        return Err(AecError::InvalidInput("output buffer has wrong length"));
    }

    let mut out = OutBuf::new(output, bytes_per_sample);
    let mut r = BitReader::new(input);

    let id_len = id_len(params)?;

    let preprocess = params.flags.contains(AecFlags::DATA_PREPROCESS);

    let mut sample_index_within_rsi: u64 = 0;
    let mut block_index_within_rsi: u32 = 0;

    // Predictor state (only used with preprocessing enabled).
    let mut predictor_x: Option<i64> = None;

    while out.len() < output_bytes {
        // Start of RSI interval.
        if preprocess && block_index_within_rsi == 0 {
            predictor_x = None;
        }

        let at_rsi_start = preprocess && block_index_within_rsi == 0;
        let ref_pending = at_rsi_start;
        let mut reference_sample_consumed = false;

        let block_start_sample = out.samples_written();

        // Read block option id.
        let id = match r.read_bits_u32(id_len) {
            Ok(v) => v,
            Err(AecError::UnexpectedEof { bit_pos }) => {
                return Err(AecError::UnexpectedEofDuringDecode {
                    bit_pos,
                    samples_written: out.samples_written(),
                });
            }
            Err(e) => return Err(e),
        };

        let max_id = (1u32 << id_len) - 1;

        // How many *coded values* does this block contribute? (set per mode; for split/SE/zero
        // it's typically block_size - ref, but uncompressed reads full block_size raw samples).
        let mut remaining_in_block: usize;

        // Helper: consume the RSI reference sample (when preprocessing is enabled).
        let mut consume_reference = |r: &mut BitReader, out: &mut OutBuf<'_>| -> Result<(), AecError> {
            let ref_raw = match r.read_bits_u32(params.bits_per_sample as usize) {
                Ok(v) => v,
                Err(AecError::UnexpectedEof { bit_pos }) => {
                    return Err(AecError::UnexpectedEofDuringDecode {
                        bit_pos,
                        samples_written: out.samples_written(),
                    });
                }
                Err(e) => return Err(e),
            };
            let ref_val = if params.flags.contains(AecFlags::DATA_SIGNED) {
                sign_extend(ref_raw, params.bits_per_sample)
            } else {
                ref_raw as i64
            };

            write_sample(out, ref_val, params)?;
            predictor_x = Some(ref_val);
            reference_sample_consumed = true;
            sample_index_within_rsi += 1;
            Ok(())
        };

        if id == 0 {
            // Low-entropy family.
            let selector = match r.read_bit() {
                Ok(v) => v,
                Err(AecError::UnexpectedEof { bit_pos }) => {
                    return Err(AecError::UnexpectedEofDuringDecode {
                        bit_pos,
                        samples_written: out.samples_written(),
                    });
                }
                Err(e) => return Err(e),
            };

            if let Some(ts) = trace_sample {
                let block_end = block_start_sample + params.block_size as usize;
                if (block_start_sample..block_end).contains(&ts) {
                    eprintln!(
                        "TRACE sample={ts} rsi_block={block_index_within_rsi} bits={} id=0 mode=LE selector={} block_samples=[{}, {})",
                        r.bits_read(),
                        selector,
                        block_start_sample,
                        block_end
                    );
                }
            }

            // For low-entropy blocks, the selector bit comes BEFORE the optional RSI reference.
            if ref_pending {
                consume_reference(&mut r, &mut out)?;
                if out.len() >= output_bytes {
                    break;
                }
            }

            remaining_in_block = params.block_size as usize;
            if reference_sample_consumed {
                remaining_in_block = remaining_in_block.saturating_sub(1);
            }

            if !selector {
                // Zero-block run.
                let fs = match read_unary(&mut r) {
                    Ok(v) => v,
                    Err(AecError::UnexpectedEof { bit_pos }) => {
                        return Err(AecError::UnexpectedEofDuringDecode {
                            bit_pos,
                            samples_written: out.samples_written(),
                        });
                    }
                    Err(e) => return Err(e),
                };
                let mut z_blocks = fs + 1;

                const ROS: u32 = 5;

                if z_blocks == ROS {
                    // Fill-to-boundary; bounded by RSI.
                    let b = block_index_within_rsi;
                    let fill1 = params.rsi.saturating_sub(b);
                    let fill2 = 64u32.saturating_sub(b % 64);
                    z_blocks = fill1.min(fill2);
                } else if z_blocks > ROS {
                    z_blocks = z_blocks.saturating_sub(1);
                }

                let mut zeros_samples = z_blocks
                    .checked_mul(params.block_size)
                    .ok_or(AecError::InvalidInput("zero-run overflow"))? as usize;

                // If we already emitted the reference sample for the first block, the zero-run
                // covers the whole blocks, but the first sample is already accounted for.
                if reference_sample_consumed {
                    zeros_samples = zeros_samples.saturating_sub(1);
                }

                if let Some(ts) = trace_sample {
                    let total_samples = (z_blocks as usize)
                        .checked_mul(params.block_size as usize)
                        .unwrap_or(usize::MAX);
                    let run_end = block_start_sample.saturating_add(total_samples);
                    if (block_start_sample..run_end).contains(&ts) {
                        eprintln!(
                            "TRACE sample={ts} rsi_block={block_index_within_rsi} bits={} id=0 mode=ZRUN fs={} z_blocks={} run_samples=[{}, {})",
                            r.bits_read(),
                            fs,
                            z_blocks,
                            block_start_sample,
                            run_end
                        );
                    }
                }

                emit_repeated_value(
                    &mut out,
                    &mut predictor_x,
                    params,
                    bytes_per_sample,
                    0,
                    zeros_samples,
                    &mut sample_index_within_rsi,
                    output_bytes,
                )?;

                // Advance block counter by z_blocks.
                // We have already consumed the current block header as part of the run.
                block_index_within_rsi = block_index_within_rsi.saturating_add(z_blocks);
                if block_index_within_rsi >= params.rsi {
                    block_index_within_rsi %= params.rsi;
                    if params.flags.contains(AecFlags::PAD_RSI) {
                        r.align_to_byte();
                    }
                    sample_index_within_rsi = 0;
                }

                continue;
            }

            // Second Extension option.
            emit_second_extension(
                &mut r,
                &mut out,
                &mut predictor_x,
                params,
                bytes_per_sample,
                remaining_in_block,
                reference_sample_consumed,
                &mut sample_index_within_rsi,
                output_bytes,
            )?;
        } else if id == max_id {
            // Uncompressed block.
            if let Some(ts) = trace_sample {
                let block_end = block_start_sample + params.block_size as usize;
                if (block_start_sample..block_end).contains(&ts) {
                    eprintln!(
                        "TRACE sample={ts} rsi_block={block_index_within_rsi} bits={} id={} mode=UNCOMP block_samples=[{}, {})",
                        r.bits_read(),
                        id,
                        block_start_sample,
                        block_end
                    );
                }
            }
            if ref_pending {
                // For uncompressed blocks, the reference sample is the first raw sample.
                consume_reference(&mut r, &mut out)?;
                if out.len() >= output_bytes {
                    break;
                }
                remaining_in_block = params.block_size as usize - 1;
            } else {
                remaining_in_block = params.block_size as usize;
            }

            for _ in 0..remaining_in_block {
                let v = match r.read_bits_u32(params.bits_per_sample as usize) {
                    Ok(v) => v,
                    Err(AecError::UnexpectedEof { bit_pos }) => {
                        return Err(AecError::UnexpectedEofDuringDecode {
                            bit_pos,
                            samples_written: out.samples_written(),
                        });
                    }
                    Err(e) => return Err(e),
                };
                emit_coded_value(
                    &mut out,
                    &mut predictor_x,
                    params,
                    bytes_per_sample,
                    v,
                    &mut sample_index_within_rsi,
                    output_bytes,
                )?;
                if out.len() >= output_bytes {
                    break;
                }
            }
        } else {
            // Rice "split" option: decode all fundamental sequences first, then all k-bit
            // binary parts (this matches libaec's bitstream layout).
            let k = (id - 1) as usize;

            if let Some(ts) = trace_sample {
                let block_end = block_start_sample + params.block_size as usize;
                if (block_start_sample..block_end).contains(&ts) {
                    eprintln!(
                        "TRACE sample={ts} rsi_block={block_index_within_rsi} bits={} id={} mode=SPLIT k={} block_samples=[{}, {})",
                        r.bits_read(),
                        id,
                        k,
                        block_start_sample,
                        block_end
                    );
                }
            }

            if ref_pending {
                consume_reference(&mut r, &mut out)?;
                if out.len() >= output_bytes {
                    break;
                }
            }

            remaining_in_block = params.block_size as usize;
            if reference_sample_consumed {
                remaining_in_block = remaining_in_block.saturating_sub(1);
            }

            let n = remaining_in_block;
            let mut tmp: Vec<u32> = vec![0u32; n];

            // If tracing is enabled and the trace sample falls within the coded portion of this
            // block, record the quotient/remainder at that offset.
            let trace_offset_in_block: Option<usize> = trace_sample.and_then(|ts| {
                let coded_start = out.samples_written();
                if ts >= coded_start && ts < coded_start + n {
                    Some(ts - coded_start)
                } else {
                    None
                }
            });
            let mut trace_q: Option<u32> = None;
            let mut trace_rem: Option<u32> = None;

            for i in 0..n {
                let q = match read_unary(&mut r) {
                    Ok(v) => v,
                    Err(AecError::UnexpectedEof { bit_pos }) => {
                        return Err(AecError::UnexpectedEofDuringDecode {
                            bit_pos,
                            samples_written: out.samples_written(),
                        });
                    }
                    Err(e) => return Err(e),
                };
                if trace_offset_in_block == Some(i) {
                    trace_q = Some(q);
                }
                tmp[i] = (q as u32)
                    .checked_shl(k as u32)
                    .ok_or(AecError::InvalidInput("rice shift overflow"))?;
            }

            if k > 0 {
                for i in 0..n {
                    let rem_bitpos_before = if trace_offset_in_block
                        .map(|off| i + 2 >= off && i <= off + 2)
                        .unwrap_or(false)
                    {
                        Some(r.bits_read())
                    } else {
                        None
                    };

                    let rem = match r.read_bits_u32(k) {
                        Ok(v) => v,
                        Err(AecError::UnexpectedEof { bit_pos }) => {
                            return Err(AecError::UnexpectedEofDuringDecode {
                                bit_pos,
                                samples_written: out.samples_written(),
                            });
                        }
                        Err(e) => return Err(e),
                    };

                    if let (Some(off), Some(bitpos)) = (trace_offset_in_block, rem_bitpos_before) {
                        if i + 2 >= off && i <= off + 2 {
                            eprintln!(
                                "TRACE rem i={} (off={}) bitpos={} bits={:0width$b} rem={}",
                                i,
                                off,
                                bitpos,
                                rem,
                                rem,
                                width = k
                            );
                        }
                    }

                    if trace_offset_in_block == Some(i) {
                        trace_rem = Some(rem);
                    }
                    tmp[i] |= rem;
                }
            }

            if let Some(off) = trace_offset_in_block {
                let d = tmp[off];
                let w_start = off.saturating_sub(2);
                let w_end = (off + 3).min(n);
                let window = tmp[w_start..w_end].to_vec();
                eprintln!(
                    "TRACE split-detail sample={} rsi_block={} id={} k={} off={} q={:?} rem={:?} d={} window[{}..{}]={:?}",
                    trace_sample.unwrap_or(0),
                    block_index_within_rsi,
                    id,
                    k,
                    off,
                    trace_q,
                    trace_rem,
                    d
                    ,
                    w_start,
                    w_end,
                    window
                );
            }

            for v in tmp {
                emit_coded_value(
                    &mut out,
                    &mut predictor_x,
                    params,
                    bytes_per_sample,
                    v,
                    &mut sample_index_within_rsi,
                    output_bytes,
                )?;
                if out.len() >= output_bytes {
                    break;
                }
            }
        }

        // Next block.
        block_index_within_rsi = block_index_within_rsi.saturating_add(1);
        if preprocess && block_index_within_rsi >= params.rsi {
            block_index_within_rsi = 0;
            sample_index_within_rsi = 0;
            if params.flags.contains(AecFlags::PAD_RSI) {
                r.align_to_byte();
            }
        }
    }

    Ok(())
}

fn validate_params(params: AecParams) -> Result<(), AecError> {
    if !(1..=32).contains(&params.bits_per_sample) {
        return Err(AecError::InvalidInput("bits_per_sample must be 1..=32"));
    }
    if params.block_size == 0 {
        return Err(AecError::InvalidInput("block_size must be > 0"));
    }
    if params.rsi == 0 {
        return Err(AecError::InvalidInput("rsi must be > 0"));
    }

    // Common AEC block sizes; keep permissive but avoid pathological values.
    if ![8u32, 16, 32, 64].contains(&params.block_size) {
        return Err(AecError::Unsupported("block_size must be one of 8,16,32,64"));
    }

    Ok(())
}

fn bytes_per_sample(params: AecParams) -> Result<usize, AecError> {
    let bps = params.bits_per_sample;

    let b = match bps {
        1..=8 => 1,
        9..=16 => 2,
        17..=24 => {
            if params.flags.contains(AecFlags::DATA_3BYTE) {
                3
            } else {
                4
            }
        }
        25..=32 => 4,
        _ => return Err(AecError::InvalidInput("invalid bits_per_sample")),
    };

    Ok(b)
}

fn id_len(params: AecParams) -> Result<usize, AecError> {
    let bps = params.bits_per_sample;

    let mut id_len = if bps > 16 { 5 } else if bps > 8 { 4 } else { 3 };

    if params.flags.contains(AecFlags::RESTRICTED) && bps <= 4 {
        id_len = if bps <= 2 { 1 } else { 2 };
    }

    Ok(id_len)
}

fn read_unary(r: &mut BitReader<'_>) -> Result<u32, AecError> {
    let mut count: u32 = 0;
    loop {
        let bit = r.read_bit()?;
        if bit {
            return Ok(count);
        }
        count = count.saturating_add(1);
        // Safety guard against pathological/corrupt inputs.
        // Valid streams can have unary lengths larger than 90 (Second Extension is the main
        // mode that constrains it to <= 90), so we only cap at a very large value.
        if count > 1_000_000 {
            return Err(AecError::InvalidInput("unary run too long"));
        }
    }
}

fn emit_coded_value(
    out: &mut OutBuf<'_>,
    predictor_x: &mut Option<i64>,
    params: AecParams,
    _bytes_per_sample: usize,
    v: u32,
    sample_index_within_rsi: &mut u64,
    output_bytes: usize,
) -> Result<(), AecError> {
    if out.len() >= output_bytes {
        return Ok(());
    }

    if params.flags.contains(AecFlags::DATA_PREPROCESS) {
        let x_prev = predictor_x.ok_or(AecError::InvalidInput("missing reference sample"))?;
        let x_next = inverse_preprocess_step(x_prev, v, params);
        write_sample(out, x_next, params)?;
        *predictor_x = Some(x_next);
        *sample_index_within_rsi += 1;
        return Ok(());
    }

    // No preprocessing: v is the sample value (raw n-bit field).
    write_sample(out, v as i64, params)?;
    *sample_index_within_rsi += 1;
    Ok(())
}

fn emit_repeated_value(
    out: &mut OutBuf<'_>,
    predictor_x: &mut Option<i64>,
    params: AecParams,
    bytes_per_sample: usize,
    v: u32,
    count: usize,
    sample_index_within_rsi: &mut u64,
    output_bytes: usize,
) -> Result<(), AecError> {
    for _ in 0..count {
        if out.len() >= output_bytes {
            break;
        }
        emit_coded_value(
            out,
            predictor_x,
            params,
            bytes_per_sample,
            v,
            sample_index_within_rsi,
            output_bytes,
        )?;
    }
    Ok(())
}

fn emit_second_extension(
    r: &mut BitReader<'_>,
    out: &mut OutBuf<'_>,
    predictor_x: &mut Option<i64>,
    params: AecParams,
    bytes_per_sample: usize,
    mut remaining_in_block: usize,
    reference_sample_consumed: bool,
    sample_index_within_rsi: &mut u64,
    output_bytes: usize,
) -> Result<(), AecError> {
    // Second Extension yields pairs (a,b) aligned to even sample indices.
    // If we started at an odd sample index because sample 0 was the reference,
    // emit only the second element from the first symbol.
    let mut need_odd_first = reference_sample_consumed;

    while remaining_in_block > 0 && out.len() < output_bytes {
        let m = read_unary(r)?;
        if m > 90 {
            return Err(AecError::InvalidInput("Second Extension unary symbol too large"));
        }

        let (a, b) = second_extension_pair(m);

        if need_odd_first {
            // Only emit the odd-index element.
            emit_coded_value(
                out,
                predictor_x,
                params,
                bytes_per_sample,
                b,
                sample_index_within_rsi,
                output_bytes,
            )?;
            remaining_in_block = remaining_in_block.saturating_sub(1);
            need_odd_first = false;
            continue;
        }

        // Emit a (even index)
        emit_coded_value(
            out,
            predictor_x,
            params,
            bytes_per_sample,
            a,
            sample_index_within_rsi,
            output_bytes,
        )?;
        remaining_in_block = remaining_in_block.saturating_sub(1);
        if remaining_in_block == 0 || out.len() >= output_bytes {
            break;
        }

        // Emit b (odd index)
        emit_coded_value(
            out,
            predictor_x,
            params,
            bytes_per_sample,
            b,
            sample_index_within_rsi,
            output_bytes,
        )?;
        remaining_in_block = remaining_in_block.saturating_sub(1);
    }

    Ok(())
}

fn second_extension_pair(m: u32) -> (u32, u32) {
    // Enumerate sums s = 0..=12, then k = 0..=s, mapping m -> (s-k, k).
    let mut idx: u32 = 0;
    for s in 0u32..=12 {
        for k in 0u32..=s {
            if idx == m {
                return (s - k, k);
            }
            idx += 1;
        }
    }

    // m is validated by caller; fallback is harmless.
    (0, 0)
}

fn inverse_preprocess_step(x_prev: i64, d: u32, params: AecParams) -> i64 {
    let n = params.bits_per_sample;

    // Match libaec inverse preprocessing exactly (see vendor/libaec.../src/decode.c).
    // The coded value `d` is mapped to a signed delta using the LSB as sign, but the
    // application of that delta is bounded; if it would cross the selected boundary,
    // a reflection mapping is used instead.
    let delta: i64 = ((d >> 1) as i64) ^ (!(((d & 1) as i64) - 1));
    let half_d: i64 = ((d >> 1) + (d & 1)) as i64;

    if params.flags.contains(AecFlags::DATA_SIGNED) {
        // signed_max matches libaec state->xmax for signed data.
        let signed_max: i64 = (1i64 << (n - 1)) - 1;
        let data = x_prev;

        if data < 0 {
            if half_d <= signed_max + data + 1 {
                data + delta
            } else {
                (d as i64) - signed_max - 1
            }
        } else {
            if half_d <= signed_max - data {
                data + delta
            } else {
                signed_max - (d as i64)
            }
        }
    } else {
        let unsigned_max: u64 = (1u64 << n) - 1;
        let data_u: u64 = x_prev as u64;

        // med is a single bit (the MSB) for unsigned samples.
        let med: u64 = unsigned_max / 2 + 1;
        let mask: u64 = if (data_u & med) != 0 { unsigned_max } else { 0 };

        if (half_d as u64) <= (mask ^ data_u) {
            (x_prev + delta) as i64
        } else {
            (mask ^ (d as u64)) as i64
        }
    }
}

fn write_sample(out: &mut OutBuf<'_>, value: i64, params: AecParams) -> Result<(), AecError> {
    let n = params.bits_per_sample as u32;
    let mask: u64 = if n == 32 { u64::MAX } else { (1u64 << n) - 1 };

    let raw_u = if params.flags.contains(AecFlags::DATA_SIGNED) {
        (value as i64 as u64) & mask
    } else {
        (value.max(0) as u64) & mask
    };

    let bytes_per_sample = out.bytes_per_sample;
    if out.pos.checked_add(bytes_per_sample).ok_or(AecError::InvalidInput("output too large"))? > out.capacity() {
        return Err(AecError::InvalidInput("output buffer too small"));
    }

    let msb = params.flags.contains(AecFlags::MSB);
    if msb {
        for i in (0..bytes_per_sample).rev() {
            out.buf[out.pos] = ((raw_u >> (i * 8)) & 0xff) as u8;
            out.pos += 1;
        }
    } else {
        for i in 0..bytes_per_sample {
            out.buf[out.pos] = ((raw_u >> (i * 8)) & 0xff) as u8;
            out.pos += 1;
        }
    }

    Ok(())
}

fn sign_extend(raw: u32, bits: u8) -> i64 {
    if bits == 32 {
        return (raw as i32) as i64;
    }
    let shift = 32 - bits as u32;
    (((raw << shift) as i32) >> shift) as i64
}
