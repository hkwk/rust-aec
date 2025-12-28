use std::path::PathBuf;

use rust_aec::{decode, flags_from_grib2_ccsds_flags, AecParams, DecodeStatus, Decoder, Flush};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).canonicalize().unwrap()
}

fn decode_streaming(payload: &[u8], params: AecParams, output_samples: usize, in_chunk: usize, out_chunk: usize) -> anyhow::Result<Vec<u8>> {
    let mut dec = Decoder::new(params, output_samples)?;

    let mut out = Vec::<u8>::new();
    let mut out_buf = vec![0u8; out_chunk.max(1)];

    let mut cursor = 0usize;
    while cursor < payload.len() {
        let end = (cursor + in_chunk.max(1)).min(payload.len());
        dec.push_input(&payload[cursor..end]);
        cursor = end;

        loop {
            let (n, status) = dec.decode(&mut out_buf, Flush::NoFlush)?;
            out.extend_from_slice(&out_buf[..n]);
            match status {
                DecodeStatus::NeedOutput => continue,
                DecodeStatus::NeedInput => break,
                DecodeStatus::Finished => return Ok(out),
            }
        }
    }

    loop {
        let (n, status) = dec.decode(&mut out_buf, Flush::Flush)?;
        out.extend_from_slice(&out_buf[..n]);
        match status {
            DecodeStatus::NeedOutput => continue,
            DecodeStatus::NeedInput => anyhow::bail!("decoder requested more input during Flush"),
            DecodeStatus::Finished => return Ok(out),
        }
    }
}

#[test]
fn streaming_matches_one_shot_on_oracle_payload() -> anyhow::Result<()> {
    let root = repo_root();
    let payload_path = root.join("aec_payload.bin");

    if !payload_path.exists() {
        eprintln!("skipping streaming test; missing file: {}", payload_path.display());
        return Ok(());
    }

    let payload = std::fs::read(payload_path)?;

    // From ccsds_dump / aec_oracle_dump on data.grib2.
    let bits_per_sample = 12u8;
    let block_size = 32u32;
    let rsi = 128u32;
    let grib_ccsds_flags = 0x0eu8;
    let num_points = 1_038_240usize;

    let params = AecParams::new(
        bits_per_sample,
        block_size,
        rsi,
        flags_from_grib2_ccsds_flags(grib_ccsds_flags),
    );

    let expected = decode(&payload, params, num_points)?;

    // A couple of chunking patterns to exercise NeedInput/NeedOutput paths.
    for (in_chunk, out_chunk) in [(1usize, 7usize), (13usize, 4096usize), (4096usize, 1024usize)] {
        let got = decode_streaming(&payload, params, num_points, in_chunk, out_chunk)?;
        assert_eq!(got.len(), expected.len(), "length mismatch for in_chunk={in_chunk} out_chunk={out_chunk}");
        assert_eq!(got, expected, "content mismatch for in_chunk={in_chunk} out_chunk={out_chunk}");
    }

    Ok(())
}
