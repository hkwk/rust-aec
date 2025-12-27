use crate::error::AecError;

/// MSB-first bit reader over a byte slice.
#[derive(Debug, Clone)]
pub struct BitReader<'a> {
    data: &'a [u8],
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }

    pub fn bits_read(&self) -> usize {
        self.bit_pos
    }

    pub fn align_to_byte(&mut self) {
        let rem = self.bit_pos % 8;
        if rem != 0 {
            self.bit_pos += 8 - rem;
        }
    }

    pub fn read_bit(&mut self) -> Result<bool, AecError> {
        Ok(self.read_bits_u32(1)? != 0)
    }

    pub fn read_bits_u32(&mut self, nbits: usize) -> Result<u32, AecError> {
        if nbits == 0 {
            return Ok(0);
        }
        if nbits > 32 {
            return Err(AecError::InvalidInput("read_bits_u32 supports up to 32 bits"));
        }

        let mut out: u32 = 0;
        for _ in 0..nbits {
            let byte_idx = self.bit_pos / 8;
            let bit_in_byte = self.bit_pos % 8;
            let byte = *self
                .data
                .get(byte_idx)
                .ok_or(AecError::UnexpectedEof { bit_pos: self.bit_pos })?;
            let bit = (byte >> (7 - bit_in_byte)) & 1;
            out = (out << 1) | (bit as u32);
            self.bit_pos += 1;
        }
        Ok(out)
    }
}

/// LSB-first bit reader over a byte slice.
///
/// This is primarily for compatibility testing: CCSDS/AEC is typically MSB-first,
/// but some producers/containers can flip intra-byte bit order.
#[derive(Debug, Clone)]
pub struct BitReaderLsb<'a> {
    data: &'a [u8],
    bit_pos: usize,
}

impl<'a> BitReaderLsb<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }

    pub fn bits_read(&self) -> usize {
        self.bit_pos
    }

    pub fn align_to_byte(&mut self) {
        let rem = self.bit_pos % 8;
        if rem != 0 {
            self.bit_pos += 8 - rem;
        }
    }

    pub fn read_bit(&mut self) -> Result<bool, AecError> {
        Ok(self.read_bits_u32(1)? != 0)
    }

    pub fn read_bits_u32(&mut self, nbits: usize) -> Result<u32, AecError> {
        if nbits == 0 {
            return Ok(0);
        }
        if nbits > 32 {
            return Err(AecError::InvalidInput("read_bits_u32 supports up to 32 bits"));
        }

        let mut out: u32 = 0;
        for _ in 0..nbits {
            let byte_idx = self.bit_pos / 8;
            let bit_in_byte = self.bit_pos % 8;
            let byte = *self
                .data
                .get(byte_idx)
                .ok_or(AecError::UnexpectedEof { bit_pos: self.bit_pos })?;
            let bit = (byte >> bit_in_byte) & 1;
            out = (out << 1) | (bit as u32);
            self.bit_pos += 1;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_bits_across_bytes() -> anyhow::Result<()> {
        let data = [0b1010_1100u8, 0b0101_0001u8];
        let mut r = BitReader::new(&data);

        assert_eq!(r.read_bits_u32(4)?, 0b1010);
        assert_eq!(r.read_bits_u32(4)?, 0b1100);
        assert_eq!(r.read_bits_u32(3)?, 0b010);
        assert_eq!(r.read_bits_u32(5)?, 0b10001);

        Ok(())
    }

    #[test]
    fn align_to_byte() -> anyhow::Result<()> {
        let data = [0xffu8, 0x12u8];
        let mut r = BitReader::new(&data);
        assert_eq!(r.read_bits_u32(1)?, 1);
        r.align_to_byte();
        assert_eq!(r.read_bits_u32(8)?, 0x12);
        Ok(())
    }
}
