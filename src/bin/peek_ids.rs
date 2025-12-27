use rust_aec::bitreader::BitReader;
use rust_aec::flags_from_grib2_ccsds_flags;
use rust_aec::params::{AecFlags, AecParams};

fn bytes_per_sample(params: AecParams) -> usize {
    match params.bits_per_sample {
        1..=8 => 1,
        9..=16 => 2,
        17..=24 => if params.flags.contains(AecFlags::DATA_3BYTE) { 3 } else { 4 },
        _ => 4,
    }
}

fn id_len(params: AecParams) -> usize {
    let bps = params.bits_per_sample;
    let mut id_len = if bps > 16 { 5 } else if bps > 8 { 4 } else { 3 };
    if params.flags.contains(AecFlags::RESTRICTED) && bps <= 4 {
        id_len = if bps <= 2 { 1 } else { 2 };
    }
    id_len
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let payload = std::fs::read("..\\..\\..\\aec_payload.bin")
        .or_else(|_| std::fs::read("aec_payload.bin"))?;

    let bits_per_sample = 12u8;
    let block_size = 32u32;
    let rsi = 128u32;
    let grib_ccsds_flags = 0x0eu8;

    let params = AecParams::new(bits_per_sample, block_size, rsi, flags_from_grib2_ccsds_flags(grib_ccsds_flags));

    println!("payload bytes: {}", payload.len());
    println!("bps={bits_per_sample} block={block_size} rsi={rsi} bytes/sample={} id_len={}", bytes_per_sample(params), id_len(params));

    let mut r = BitReader::new(&payload);
    let id_len = id_len(params);
    let max_id = (1u32 << id_len) - 1;

    for i in 0..50 {
        let id = r.read_bits_u32(id_len)?;
        let mut note = "";
        if id == 0 {
            let sel = r.read_bit()?;
            note = if sel { "low:SE" } else { "low:ZRUN" };
        } else if id == max_id {
            note = "UNCOMP";
            // skip raw samples for this block (assume no reference for peek)
            let _ = r.read_bits_u32((bits_per_sample as usize) * 2)?; // peek just advances a bit
        }
        println!("#{i:02} id={id} {note} (bit_pos={})", r.bits_read());
    }

    Ok(())
}
