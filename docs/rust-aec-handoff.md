# rust-aec v0.1.0 Handoff Notes (for new repo / GPT agent)

> Purpose: copy this file into a new folder/new repo to help a GPT agent quickly understand the current state, how to build/verify, publishing checklist, and next-version TODOs.

## 1) Current Status (Done)

- `rust-aec` is a **pure Rust** CCSDS **121.0-B-3** Adaptive Entropy Coding (AEC) decoder, primarily targeting **GRIB2 Data Representation Template 5.0 = 42**.
- **Byte-for-byte correctness vs libaec**: verified using an oracle approach on real GRIB2 data; output matches **libaec** exactly (tests pass).
- **crates.io readiness**: `README.md`, MIT `LICENSE`, crate metadata (`repository/homepage/docs.rs`, etc.) are prepared and `cargo package` dry-run succeeds.

## 2) Key Files (should exist in the new repo)

- Crate metadata:
  - `Cargo.toml`
- Documentation:
  - `README.md` (English; includes comparison images)
  - `LICENSE` (MIT)
- Core source:
  - `src/lib.rs` (public API: `decode`, `decode_into`, `flags_from_grib2_ccsds_flags`)
  - `src/decoder.rs` (core decoder state machine + bitstream parsing + inverse preprocessing aligned to libaec)
  - `src/bitreader.rs` (MSB-first bit reader)
  - `src/params.rs` (`AecParams`/`AecFlags`)
  - `src/error.rs` (`AecError`)
- Example program:
  - `examples/decode_aec_payload.rs` (minimal: decode payload → output byte stream)
- Tests:
  - `tests/oracle_data_grib2.rs`
    - Oracle test **skips** when oracle files are missing (so CI/crates packaging is not blocked by large binary test data).

- Optional vendored reference sources (repo-only):
  - `vendor/` may contain third-party sources used for local reference and oracle comparisons (e.g. `libaec`).
  - `vendor/` is **excluded from crates.io packages** (see `Cargo.toml` `exclude`).

> Note: In the previous monorepo, there was additional Chinese documentation (e.g. `docs/rust-aec.md`). In a standalone repo, you can optionally move it here or condense it into README “Implementation notes”.

## 3) Public API (v0.1.0)

- `decode(input, params, output_samples) -> Result<Vec<u8>, AecError>`
- `decode_into(input, params, output_samples, output: &mut [u8]) -> Result<(), AecError>`
  - Improvement vs “always allocate a Vec”: callers can reuse a buffer to reduce allocations.
  - Still **one-shot**: requires `output_samples` ahead of time and writes a fixed-length output buffer.
- `flags_from_grib2_ccsds_flags(ccsds_flags: u8) -> AecFlags`
- Parameters:
  - `AecParams { bits_per_sample: u8, block_size: u32, rsi: u32, flags: AecFlags }`

## 4) Build & Verification Log (Windows)

Commands validated (run at crate root):

- Unit tests + (optional) oracle test + doctests:
  - `cargo test`
- Packaging check (crates.io dry-run):
  - `cargo package`

Key point: `cargo package` success indicates the publishable file set + metadata + README/License are acceptable for crates.io.

## 5) Oracle Test (how it works)

- Oracle idea:
  - Extract GRIB2 (template 42) **Section 7** AEC payload
  - Decode with **libaec** to generate oracle bytes
  - Decode with **rust-aec** and compare outputs **byte-for-byte**
- Current strategy:
  - `tests/oracle_data_grib2.rs` will **skip** when oracle files are missing → avoids CI failures and avoids shipping large binary data.
- Reproducing oracle data in a new repo:
  - Requires a small tool/script to extract payload and decode with libaec.
  - Recommendation: keep the oracle generator under an `xtask/` or an optional bin.
  - Do **not** make crates.io users depend on native libaec by default.

## 6) Publishing Checklist (crates.io)

Cargo.toml fields to confirm:

- `license = "MIT"`
- `repository = "https://github.com/hkwk/rust-aec"`
- `homepage = "https://github.com/hkwk/rust-aec"`
- `documentation = "https://docs.rs/rust-aec"`
- `readme = "README.md"`
- `rust-version = "1.85"` (adjust if you want a lower MSRV)

Repository files:

- Root `LICENSE` exists (MIT text)
- README images are external links (crates.io-friendly)

Release steps:

- `cargo publish --dry-run`
- `cargo login <token>`
- `cargo publish`

## 7) TODO for Next Version (proposed)

Prioritized:

1. **Streaming / incremental output**
   - Current: `decode`/`decode_into` are one-shot.
   - TODO: add a `Decoder` type that can output per-block or per-RSI chunk (reduces peak memory; helps “decode → scale/render” pipelines).

2. **Robustness / security**
   - TODO: fuzz the bitstream parser (e.g. `cargo-fuzz`)
   - TODO: expand malformed-input tests.

3. **Performance**
   - TODO: add Criterion benchmarks across `bits_per_sample`, `block_size`, `rsi`, and typical mode distributions.

4. **Coverage**
   - TODO: broaden supported flag/parameter combinations gradually; feature-gate experimental paths.

5. **Tooling**
   - TODO: provide a pure-Rust helper tool to extract AEC payload from GRIB2 (no libaec dependency) for debugging/repro.

6. **Docs**
   - TODO: clearer explanation of how output bytes map to numeric samples (endianness/signedness/bytes-per-sample)
   - TODO: link the AEC output to GRIB2 simple packing scaling workflow.

## 8) Integration note (from the previous monorepo)

- The main GUI app switched to crates.io dependency:
  - `rust-aec = { version = "0.1.0", optional = true }`
- Verified compilation on Windows:
  - `cargo check -q --features grib-ccsds-rust`
