# rust-aec

Pure Rust decoder for **CCSDS 121.0-B-3 Adaptive Entropy Coding (AEC)**, with an initial focus on **GRIB2 Data Representation Template 5.0 = 42 (CCSDS/AEC)**.

This crate was created to avoid native `libaec` build friction (C/CMake/bindgen) on Windows while keeping byte-for-byte compatibility.

Repository: https://github.com/hkwk/rust-aec

## Visual comparison

| rust-aec | libaec |
| --- | --- |
| ![rust-aec](https://cdn.jsdelivr.net/gh/hkwk/blog-photo/2025/01/rust-aec.png) | ![libaec](https://cdn.jsdelivr.net/gh/hkwk/blog-photo/2025/01/libaec.png) |

## Status

- Decoder implemented for the subset needed by GRIB2 template 5.0=42.
- Validated against `libaec` using an “oracle” byte-for-byte comparison on real GRIB2 data (in the upstream workspace that hosts this crate).

## What this crate provides

- `decode(input, params, output_samples) -> Result<Vec<u8>, AecError>`: decode an AEC bitstream into packed sample bytes.
- `AecParams` / `AecFlags`: minimal parameter set aligned with `libaec`’s `aec_stream`.
- `flags_from_grib2_ccsds_flags(ccsds_flags: u8)`: helper for GRIB2 template 5.42.

## Non-goals (for now)

- Full coverage of every possible `libaec` flag combination.
- Streaming / incremental decode API.
- Complete GRIB2 decoding pipeline (bitmap/reduced grids etc.). This crate only decodes the **AEC payload**.

## Usage

### Decode a GRIB2 template 42 payload

You need:

- `payload`: the GRIB2 Section 7 payload (the CCSDS/AEC bitstream)
- `num_points`: number of decoded samples (GRIB2: `section5.num_encoded_points`)
- `bits_per_sample`, `block_size`, `rsi`, `ccsds_flags`: from GRIB2 template 5.42

```rust
use rust_aec::{decode, flags_from_grib2_ccsds_flags, AecParams};

let params = AecParams::new(
    12,                 // bits_per_sample
    32,                 // block_size
    128,                // rsi
    flags_from_grib2_ccsds_flags(0x0e),
);

let decoded: Vec<u8> = decode(&payload, params, num_points)?;
```

`decoded` is a byte vector of length `num_points * bytes_per_sample`, where `bytes_per_sample = ceil(bits_per_sample/8)`.
Byte order is controlled by the `MSB` flag.

### Example program

This crate ships a small example binary:

```powershell
cargo run -p rust-aec --example decode_aec_payload -- --payload aec_payload.bin --samples 1038240
```

## API notes

- When `AecFlags::DATA_PREPROCESS` is set, the output bytes are the **reconstructed sample values** (inverse preprocessing applied).
- When preprocessing is not set, the output bytes represent the raw coded values.

## License

MIT. See `LICENSE`.

## Related

- The host workspace includes a GRIB2 preview integration that can fall back to `rust-aec` for template 42.
