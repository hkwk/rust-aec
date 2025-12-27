use core::fmt;

#[derive(Debug, Clone)]
pub enum AecError {
    InvalidInput(&'static str),
    Unsupported(&'static str),
    NotImplemented(&'static str),
    UnexpectedEof { bit_pos: usize },
    UnexpectedEofDuringDecode { bit_pos: usize, samples_written: usize },
}

impl fmt::Display for AecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AecError::InvalidInput(s) => write!(f, "invalid input: {s}"),
            AecError::Unsupported(s) => write!(f, "unsupported: {s}"),
            AecError::NotImplemented(s) => write!(f, "not implemented: {s}"),
            AecError::UnexpectedEof { bit_pos } => write!(f, "unexpected end of input at bit {bit_pos}"),
            AecError::UnexpectedEofDuringDecode { bit_pos, samples_written } => {
                write!(f, "unexpected end of input at bit {bit_pos} (wrote {samples_written} samples)")
            }
        }
    }
}

impl std::error::Error for AecError {}
