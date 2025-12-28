# Changelog

All notable changes to this project are documented below.

## [0.1.1] - 2025-12-28

### Fixed

- Corrected streaming decoder RSI reference-sample accounting for low-entropy blocks (prevented double-counting and improved zero-run handling).
- Fixed streaming repeat scheduling so small output chunks work correctly.

### Added

- `Decoder` streaming tests validating byte-for-byte equivalence with one-shot `decode` under multiple input/output chunk patterns.
- `examples/stream_decode_aec_payload.rs` — example demonstrating a libaec-like loop (push input, decode, handle `NeedInput` / `NeedOutput`, and `Flush`).
- `docs/README.md` — English documentation for the crate.

### Packaging

- Bumped crate version to **0.1.1**.
- Excluded `/docs/**` from the crates.io package (docs kept for local use only).

## [0.1.0] - 2025-12-27

- Initial published release: `rust-aec v0.1.0`.
