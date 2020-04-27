use rustpython_bytecode::bytecode::{self, Instruction};

use super::{InstructionMetadata, OptimizationBuffer};

macro_rules! metas {
    [$($metas:expr),*$(,)?] => {
        InstructionMetadata::from(vec![$($metas),*])
    };
}
macro_rules! lc {
    ($name:ident {$($field:tt)*}) => {
        Instruction::LoadConst {
            value: bytecode::Constant::$name {$($field)*},
        }
    };
    ($name:ident, $($value:tt)*) => {
        lc!($name { value: $($value)* })
    };
}
macro_rules! emitconst {
    ($buf:expr, [$($metas:expr),*$(,)?], $($arg:tt)*) => {
        $buf.emit(
            lc!($($arg)*),
            metas![$($metas),*],
        )
    };
}

pub fn operator(buf: &mut impl OptimizationBuffer) {
    let (instruction, meta) = buf.pop();
    if let Instruction::BinaryOperation { op, inplace } = instruction {
        let (rhs, rhs_meta) = buf.pop();
        let (lhs, lhs_meta) = buf.pop();
        macro_rules! op {
            ($op:ident) => {
                bytecode::BinaryOperator::$op
            };
        }
        match (op, lhs, rhs) {
            (op!(Add), lc!(Integer, lhs), lc!(Integer, rhs)) => {
                emitconst!(buf, [lhs_meta, rhs_meta], Integer, lhs + rhs)
            }
            (op!(Subtract), lc!(Integer, lhs), lc!(Integer, rhs)) => {
                emitconst!(buf, [lhs_meta, rhs_meta], Integer, lhs - rhs)
            }
            (op!(Add), lc!(Float, lhs), lc!(Float, rhs)) => {
                emitconst!(buf, [lhs_meta, rhs_meta], Float, lhs + rhs)
            }
            (op!(Subtract), lc!(Float, lhs), lc!(Float, rhs)) => {
                emitconst!(buf, [lhs_meta, rhs_meta], Float, lhs - rhs)
            }
            (op!(Multiply), lc!(Float, lhs), lc!(Float, rhs)) => {
                emitconst!(buf, [lhs_meta, rhs_meta], Float, lhs * rhs)
            }
            (op!(Divide), lc!(Float, lhs), lc!(Float, rhs)) if rhs != 0.0 => {
                emitconst!(buf, [lhs_meta, rhs_meta], Float, lhs / rhs)
            }
            (op!(Power), lc!(Float, lhs), lc!(Float, rhs)) => {
                emitconst!(buf, [lhs_meta, rhs_meta], Float, lhs.powf(rhs))
            }
            (op!(Add), lc!(String, mut lhs), lc!(String, rhs)) => {
                lhs.push_str(&rhs);
                emitconst!(buf, [lhs_meta, rhs_meta], String, lhs);
            }
            (op, lhs, rhs) => {
                buf.emit(lhs, lhs_meta);
                buf.emit(rhs, rhs_meta);
                buf.emit(Instruction::BinaryOperation { op, inplace }, meta);
            }
        }
    } else {
        buf.emit(instruction, meta)
    }
}

// TODO: make a version of this that doesn't miscompile `a, b = (1, 2) if True else (3, 4)`
// pub fn unpack(buf: &mut impl OptimizationBuffer) {
//     let (instruction, meta) = buf.pop();
//     if let Instruction::UnpackSequence { size } = instruction {
//         let (arg, arg_meta) = buf.pop();
//         match arg {
//             Instruction::BuildTuple {
//                 size: tup_size,
//                 unpack,
//             } if !unpack && tup_size == size => {
//                 buf.emit(
//                     Instruction::Reverse { amount: size },
//                     vec![arg_meta, meta].into(),
//                 );
//             }
//             arg => {
//                 buf.emit(arg, arg_meta);
//                 buf.emit(instruction, meta);
//             }
//         }
//     } else {
//         buf.emit(instruction, meta)
//     }
// }
