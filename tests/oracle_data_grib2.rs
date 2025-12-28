use std::path::PathBuf;

use rust_aec::{decode, flags_from_grib2_ccsds_flags};
use rust_aec::params::AecParams;

fn repo_root() -> PathBuf {
    // Standalone crate: `CARGO_MANIFEST_DIR` is the repo root.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).canonicalize().unwrap()
}

#[test]
fn oracle_matches_data_grib2_payload() -> anyhow::Result<()> {
    let root = repo_root();
    let payload_path = root.join("aec_payload.bin");
    let oracle_path = root.join("aec_decoded_oracle.bin");

    if !payload_path.exists() || !oracle_path.exists() {
        eprintln!(
            "skipping oracle test; missing files: {} or {}",
            payload_path.display(),
            oracle_path.display()
        );
        return Ok(());
    }

    let payload = std::fs::read(payload_path)?;
    let oracle = std::fs::read(oracle_path)?;

    let num_points = 1_038_240usize;
    assert_eq!(oracle.len(), num_points * 2, "expected 2 bytes/sample for 12-bit samples");

    // From ccsds_dump / aec_oracle_dump on data.grib2.
    let bits_per_sample = 12u8;
    let block_size = 32u32;
    let rsi = 128u32;
    let grib_ccsds_flags = 0x0eu8;

    let params = AecParams::new(bits_per_sample, block_size, rsi, flags_from_grib2_ccsds_flags(grib_ccsds_flags));

    let decoded = decode(&payload, params, num_points)?;

    assert_eq!(decoded.len(), oracle.len());

    if decoded != oracle {
        let mut first = None;
        for (i, (a, b)) in decoded.iter().zip(oracle.iter()).enumerate() {
            if a != b {
                first = Some((i, *a, *b));
                break;
            }
        }

        if let Some((i, got, expected)) = first {
            let start = i.saturating_sub(16);
            let end = (i + 16).min(decoded.len());
            eprintln!("oracle mismatch at byte {i}: got={got} expected={expected}");
            eprintln!("decoded[{start}..{end}] = {:?}", &decoded[start..end]);
            eprintln!("oracle [{start}..{end}] = {:?}", &oracle[start..end]);

            // Also try to interpret as 16-bit samples (big endian) since our oracle file is
            // 2 bytes per sample.
            let sample = i / 2;
            if sample + 1 < num_points {
                let di = sample * 2;
                let oi = sample * 2;
                let d_be = u16::from_be_bytes([decoded[di], decoded[di + 1]]);
                let o_be = u16::from_be_bytes([oracle[oi], oracle[oi + 1]]);
                eprintln!("at sample {sample}: decoded_u16_be={d_be} oracle_u16_be={o_be}");

                if sample > 0 {
                    let pi = (sample - 1) * 2;
                    let prev_be = u16::from_be_bytes([oracle[pi], oracle[pi + 1]]);
                    let delta = (o_be as i32) - (prev_be as i32);
                    let expected_d = if delta >= 0 {
                        (2 * delta) as u32
                    } else {
                        (-2 * delta - 1) as u32
                    };
                    eprintln!(
                        "oracle delta from prev sample: prev_u16_be={} delta={} => expected_d={} (folded)",
                        prev_be,
                        delta,
                        expected_d
                    );
                }

                // Show expected d values around the mismatch (derived from oracle deltas).
                let w0 = sample.saturating_sub(2);
                let w1 = (sample + 3).min(num_points);
                for s in w0..w1 {
                    if s == 0 {
                        continue;
                    }
                    let cur_i = s * 2;
                    let prev_i = (s - 1) * 2;
                    let cur = u16::from_be_bytes([oracle[cur_i], oracle[cur_i + 1]]) as i32;
                    let prev = u16::from_be_bytes([oracle[prev_i], oracle[prev_i + 1]]) as i32;
                    let dlt = cur - prev;
                    let d_expected = if dlt >= 0 { (2 * dlt) as u32 } else { (-2 * dlt - 1) as u32 };
                    eprintln!("expected_d at sample {s}: prev={prev} cur={cur} delta={dlt} d={d_expected}");
                }

                // Trigger targeted tracing in the decoder around this sample.
                unsafe {
                    std::env::set_var("RUST_AEC_TRACE_SAMPLE", sample.to_string());
                }
                let _ = decode(&payload, params, num_points);
            }
        } else {
            eprintln!("oracle mismatch but no differing byte found (unexpected)");
        }

        panic!("decoded output does not match oracle");
    }

    Ok(())
}
