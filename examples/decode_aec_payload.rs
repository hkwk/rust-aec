use std::path::PathBuf;

use anyhow::Context;
use rust_aec::{decode, flags_from_grib2_ccsds_flags, AecParams};

fn main() -> anyhow::Result<()> {
    // Minimal argument parsing (no clap dependency).
    // Usage:
    //   cargo run -p rust-aec --example decode_aec_payload -- --payload aec_payload.bin --samples 1038240

    let mut payload_path: Option<PathBuf> = None;
    let mut samples: Option<usize> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--payload" => {
                let v = args.next().context("--payload requires a value")?;
                payload_path = Some(PathBuf::from(v));
            }
            "--samples" => {
                let v = args.next().context("--samples requires a value")?;
                samples = Some(v.parse::<usize>().context("--samples must be an integer")?);
            }
            "--help" | "-h" => {
                print_help();
                return Ok(());
            }
            other => {
                anyhow::bail!("unknown argument: {other} (use --help)");
            }
        }
    }

    let payload_path = payload_path.unwrap_or_else(|| PathBuf::from("aec_payload.bin"));
    let samples = samples.unwrap_or(1_038_240);

    // These parameters match the sample `data.grib2` used in the host workspace.
    // For your own data, read them from GRIB2 template 5.42.
    let bits_per_sample: u8 = 12;
    let block_size: u32 = 32;
    let rsi: u32 = 128;
    let ccsds_flags: u8 = 0x0e;

    let payload = std::fs::read(&payload_path)
        .with_context(|| format!("failed to read payload: {}", payload_path.display()))?;

    let params = AecParams::new(
        bits_per_sample,
        block_size,
        rsi,
        flags_from_grib2_ccsds_flags(ccsds_flags),
    );

    let decoded = decode(&payload, params, samples).context("AEC decode failed")?;

    println!("payload: {} ({} bytes)", payload_path.display(), payload.len());
    println!("samples: {samples}");
    println!("decoded bytes: {}", decoded.len());
    println!("first 16 bytes: {:02x?}", &decoded[..decoded.len().min(16)]);

    Ok(())
}

fn print_help() {
    println!("decode_aec_payload (example)");
    println!("");
    println!("Usage:");
    println!("  decode_aec_payload --payload <path> --samples <n>");
    println!("");
    println!("Defaults:");
    println!("  --payload aec_payload.bin");
    println!("  --samples 1038240");
}
