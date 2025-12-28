//! `rust-aec` is a pure Rust decoder for **CCSDS 121.0-B-3 Adaptive Entropy Coding (AEC)**.
//!
//! Primary goal: support **GRIB2 Data Representation Template 5.0 = 42 (CCSDS/AEC)** without
//! requiring native `libaec`.
//!
//! # Quick start
//!
//! ```
//! use rust_aec::{decode, flags_from_grib2_ccsds_flags, AecParams};
//!
//! // In a real GRIB2 pipeline, `payload` is Section 7 and `num_points` comes from Section 5.
//! // This snippet focuses on API shape and compiles without external files.
//! let payload: Vec<u8> = Vec::new();
//! let num_points: usize = 0;
//!
//! let params = AecParams::new(12, 32, 128, flags_from_grib2_ccsds_flags(0x0e));
//! let decoded = decode(&payload, params, num_points);
//! assert!(decoded.is_ok());
//! ```

pub mod bitreader;
mod decoder;
pub mod error;
pub mod params;

pub use crate::error::AecError;
pub use crate::params::{AecFlags, AecParams};

pub use crate::decoder::{DecodeStatus, Decoder, Flush};

/// Decode an AEC bitstream into packed sample bytes.
///
/// - `input`: CCSDS/AEC payload bitstream.
/// - `params`: bit width, block size, RSI, and flags.
/// - `output_samples`: number of samples expected in the output.
///
/// Returns a `Vec<u8>` of length `output_samples * bytes_per_sample`, where
/// `bytes_per_sample = ceil(bits_per_sample / 8)`.
///
/// Note: When `AecFlags::MSB` is set, samples are written big-endian (MSB-first)
/// per sample; otherwise little-endian.
pub fn decode(input: &[u8], params: AecParams, output_samples: usize) -> Result<Vec<u8>, AecError> {
    decoder::decode(input, params, output_samples)
}

/// Decode an AEC bitstream into a caller-provided output buffer.
///
/// This is useful when you want to reuse an allocation (e.g. decode many tiles/messages)
/// without repeatedly allocating a `Vec<u8>`.
///
/// The `output` buffer length must be exactly `output_samples * bytes_per_sample`, where
/// `bytes_per_sample = ceil(bits_per_sample / 8)` (subject to `AecFlags::DATA_3BYTE` rules).
pub fn decode_into(
    input: &[u8],
    params: AecParams,
    output_samples: usize,
    output: &mut [u8],
) -> Result<(), AecError> {
    decoder::decode_into(input, params, output_samples, output)
}

/// Helper: convert GRIB2 `ccsdsFlags` (template 5.42) to `AecFlags`.
pub fn flags_from_grib2_ccsds_flags(ccsds_flags: u8) -> AecFlags {
    let mut flags = AecFlags::empty();

    if (ccsds_flags & (1 << 0)) != 0 {
        flags |= AecFlags::DATA_SIGNED;
    }
    if (ccsds_flags & (1 << 1)) != 0 {
        flags |= AecFlags::DATA_3BYTE;
    }
    if (ccsds_flags & (1 << 2)) != 0 {
        flags |= AecFlags::MSB;
    }
    if (ccsds_flags & (1 << 3)) != 0 {
        flags |= AecFlags::DATA_PREPROCESS;
    }
    if (ccsds_flags & (1 << 4)) != 0 {
        flags |= AecFlags::RESTRICTED;
    }
    if (ccsds_flags & (1 << 5)) != 0 {
        flags |= AecFlags::PAD_RSI;
    }

    flags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_mapping_smoke() {
        let f = flags_from_grib2_ccsds_flags(0b0011_1011);
        assert!(f.contains(AecFlags::DATA_SIGNED));
        assert!(f.contains(AecFlags::DATA_3BYTE));
        assert!(!f.contains(AecFlags::MSB));
        assert!(f.contains(AecFlags::DATA_PREPROCESS));
        assert!(f.contains(AecFlags::RESTRICTED));
        assert!(f.contains(AecFlags::PAD_RSI));
    }
}
