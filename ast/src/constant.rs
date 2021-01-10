use num_bigint::BigInt;

#[derive(Debug, PartialEq)]
pub enum Constant {
    None,
    Bool(bool),
    Str(String),
    Bytes(Vec<u8>),
    Int(BigInt),
    Tuple(Vec<Constant>),
    Float(f64),
    Complex { real: f64, imag: f64 },
    Ellipsis,
}

impl From<String> for Constant {
    fn from(s: String) -> Constant {
        Self::Str(s)
    }
}
impl From<Vec<u8>> for Constant {
    fn from(b: Vec<u8>) -> Constant {
        Self::Bytes(b)
    }
}
impl From<bool> for Constant {
    fn from(b: bool) -> Constant {
        Self::Bool(b)
    }
}

/// Transforms a value prior to formatting it.
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u8)]
pub enum ConversionFlag {
    /// Converts by calling `str(<value>)`.
    Str = b's',
    /// Converts by calling `ascii(<value>)`.
    Ascii = b'a',
    /// Converts by calling `repr(<value>)`.
    Repr = b'r',
}

impl ConversionFlag {
    pub fn try_from_byte(b: u8) -> Option<Self> {
        match b {
            b's' => Some(Self::Str),
            b'a' => Some(Self::Ascii),
            b'r' => Some(Self::Repr),
            _ => None,
        }
    }
}
