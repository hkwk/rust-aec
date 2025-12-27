use bitflags::bitflags;

bitflags! {
    /// AEC flags (mirrors `libaec`'s `aec_stream.flags`).
    ///
    /// For GRIB2 template 5.42, a subset of these flags is provided in the
    /// `ccsdsFlags` field; see [`crate::flags_from_grib2_ccsds_flags`].
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct AecFlags: u32 {
        /// Signed samples (two's complement). If not set, samples are unsigned.
        const DATA_SIGNED     = 1 << 0;
        /// Use 3 bytes/sample for 17..=24-bit samples (otherwise 4).
        const DATA_3BYTE      = 1 << 1;
        /// Output samples as MSB-first byte order (big-endian within each sample).
        const MSB            = 1 << 2;
        /// Enable preprocessing (predictor + folding) in the bitstream.
        const DATA_PREPROCESS = 1 << 3;
        /// Restricted ID table for small bit depths.
        const RESTRICTED      = 1 << 4;
        /// Pad each RSI interval to the next byte boundary.
        const PAD_RSI         = 1 << 5;
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AecParams {
    /// Bits per sample.
    ///
    /// For GRIB2 template 5.42: `template42.simple.num_bits`.
    pub bits_per_sample: u8,
    /// Block size.
    ///
    /// For GRIB2 template 5.42: `template42.block_size`.
    pub block_size: u32,
    /// Reference sample interval (RSI).
    ///
    /// For GRIB2 template 5.42: `template42.ref_sample_interval`.
    pub rsi: u32,
    /// Decoder flags.
    pub flags: AecFlags,
}

impl AecParams {
    /// Create a new parameter set.
    pub fn new(bits_per_sample: u8, block_size: u32, rsi: u32, flags: AecFlags) -> Self {
        Self { bits_per_sample, block_size, rsi, flags }
    }
}
