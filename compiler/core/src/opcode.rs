use crate::marshal::MarshalError;
pub use crate::opcodes::{PseudoOpcode, RealOpcode};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Opcode {
    Real(RealOpcode),
    Pseudo(PseudoOpcode),
}

impl TryFrom<u16> for Opcode {
    type Error = MarshalError;

    fn try_from(raw: u16) -> Result<Self, Self::Error> {
        // Try first pseudo opcode. If not, fallback to real opcode.
        PseudoOpcode::try_from(raw)
            .map(Opcode::Pseudo)
            .or_else(|_| {
                Self::try_from(u8::try_from(raw).map_err(|_| Self::Error::InvalidBytecode)?)
            })
    }
}

impl TryFrom<u8> for Opcode {
    type Error = MarshalError;

    fn try_from(raw: u8) -> Result<Self, Self::Error> {
        // u8 can never be a pseduo.
        RealOpcode::try_from(raw).map(Opcode::Real)
    }
}

macro_rules! impl_try_from {
    ($struct_name:ident, $($t:ty),+ $(,)?) => {
        $(
            impl TryFrom<$t> for $struct_name {
                type Error = MarshalError;

                fn try_from(raw: $t) -> Result<Self, Self::Error> {
                    Self::try_from(u16::try_from(raw).map_err(|_| Self::Error::InvalidBytecode)?)
                }
            }
        )+
    };
}

impl_try_from!(
    Opcode, i8, i16, i32, i64, i128, isize, u32, u64, u128, usize
);
