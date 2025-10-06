pub use crate::opcodes::{PseudoOpcode, RealOpcode};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Opcode {
    Real(RealOpcode),
    Pseudo(PseudoOpcode),
}

macro_rules! impl_try_from {
    ($struct_name:ident, $($t:ty),+ $(,)?) => {
        $(
            impl TryFrom<$t> for $struct_name {
                type Error = ();

                fn try_from(raw: $t) -> Result<Self, Self::Error> {
                    RealOpcode::try_from(raw)
                        .map(Opcode::Real)
                        .or_else(|_| PseudoOpcode::try_from(raw).map(Opcode::Pseudo))
                }
            }
        )+
    };
}

impl_try_from!(
    Opcode, i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);
