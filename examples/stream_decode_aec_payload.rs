use std::path::PathBuf;

use anyhow::Context;
use rust_aec::{flags_from_grib2_ccsds_flags, AecParams, DecodeStatus, Decoder, Flush};

fn main() -> anyhow::Result<()> {
    // Minimal argument parsing (no clap dependency).
    // Usage:
    //   cargo run -p rust-aec --example stream_decode_aec_payload -- --payload aec_payload.bin --samples 1038240

    let mut payload_path: Option<PathBuf> = None;
    let mut samples: Option<usize> = None;
    let mut in_chunk: usize = 4096;
    let mut out_chunk: usize = 16 * 1024;

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
            "--in-chunk" => {
                let v = args.next().context("--in-chunk requires a value")?;
                in_chunk = v.parse::<usize>().context("--in-chunk must be an integer")?;
            }
            "--out-chunk" => {
                let v = args.next().context("--out-chunk requires a value")?;
                out_chunk = v.parse::<usize>().context("--out-chunk must be an integer")?;
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

    let mut dec = Decoder::new(params, samples).context("decoder init failed")?;

    let mut decoded: Vec<u8> = Vec::new();
    let mut out_buf = vec![0u8; out_chunk.max(1)];

    // Feed input in chunks.
    let mut cursor = 0usize;
    while cursor < payload.len() {
        let end = (cursor + in_chunk.max(1)).min(payload.len());
        dec.push_input(&payload[cursor..end]);
        cursor = end;

        loop {
            let (n, status) = dec.decode(&mut out_buf, Flush::NoFlush)?;
            decoded.extend_from_slice(&out_buf[..n]);
            match status {
                DecodeStatus::NeedOutput => continue,
                DecodeStatus::NeedInput => break,
                DecodeStatus::Finished => {
                    print_summary(&payload_path, &payload, samples, &decoded, &dec);
                    return Ok(());
                }
            }
        }
    }

    // Flush phase: assert no more input will arrive.
    loop {
        let (n, status) = dec.decode(&mut out_buf, Flush::Flush)?;
        decoded.extend_from_slice(&out_buf[..n]);
        match status {
            DecodeStatus::NeedOutput => continue,
            DecodeStatus::NeedInput => anyhow::bail!("decoder requested more input during Flush"),
            DecodeStatus::Finished => break,
        }
    }

    print_summary(&payload_path, &payload, samples, &decoded, &dec);
    Ok(())
}

fn print_summary(payload_path: &PathBuf, payload: &[u8], samples: usize, decoded: &[u8], dec: &Decoder) {
    println!("payload: {} ({} bytes)", payload_path.display(), payload.len());
    println!("samples: {samples}");
    println!("decoded bytes: {}", decoded.len());
    println!("total_in: {}", dec.total_in());
    println!("total_out: {}", dec.total_out());
    println!("first 16 bytes: {:02x?}", &decoded[..decoded.len().min(16)]);
}

fn print_help() {
    println!("stream_decode_aec_payload (example)");
    println!("");
    println!("Usage:");
    println!("  stream_decode_aec_payload --payload <path> --samples <n> [--in-chunk <n>] [--out-chunk <n>]");
    println!("");
    println!("Defaults:");
    println!("  --payload aec_payload.bin");
    println!("  --samples 1038240");
    println!("  --in-chunk 4096");
    println!("  --out-chunk 16384");
}
