use crate::output_stream::OutputStream;
use arrayvec::ArrayVec;
use rustpython_bytecode::bytecode::{self, CodeObject, Instruction, Location};

const PEEPHOLE_BUFFER_SIZE: usize = 10;

pub struct PeepholeOptimizer<O: OutputStream> {
    inner: O,
    buffer: ArrayVec<[(Instruction, Location); PEEPHOLE_BUFFER_SIZE]>,
}

impl<O: OutputStream> From<CodeObject> for PeepholeOptimizer<O> {
    fn from(code: CodeObject) -> Self {
        Self::new(code.into())
    }
}
impl<O: OutputStream> From<PeepholeOptimizer<O>> for CodeObject {
    fn from(mut peep: PeepholeOptimizer<O>) -> Self {
        peep.flush();
        peep.inner.into()
    }
}

impl<O: OutputStream> PeepholeOptimizer<O> {
    pub fn new(inner: O) -> Self {
        PeepholeOptimizer {
            inner,
            buffer: ArrayVec::default(),
        }
    }

    fn emit(&mut self, instruction: Instruction, loc: Location) {
        if self.buffer.is_full() {
            let (instr, loc) = self.buffer.remove(0);
            self.inner.emit(instr, loc);
            assert_eq!(self.buffer.len(), PEEPHOLE_BUFFER_SIZE - 1)
        }
        // safe because we just checked that: if full then remove one element from it
        unsafe { self.buffer.push_unchecked((instruction, loc)) };
    }

    fn pop(&mut self) -> (Instruction, Location) {
        self.buffer.pop().unwrap()
    }

    fn optimize(&mut self, instruction: Instruction, loc: Location) {
        match instruction {
            Instruction::BinaryOperation { op, inplace } => {
                let (rhs, rhs_loc) = self.pop();
                let (lhs, lhs_loc) = self.pop();
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
                    ($($arg:tt)*) => {
                        self.emit(lc!($($arg)*), lhs_loc)
                    };
                }
                macro_rules! op {
                    ($op:ident) => {
                        bytecode::BinaryOperator::$op
                    };
                }
                match (op, lhs, rhs) {
                    (op!(Add), lc!(Integer, lhs), lc!(Integer, rhs)) => {
                        emitconst!(Integer, lhs + rhs)
                    }
                    (op!(Subtract), lc!(Integer, lhs), lc!(Integer, rhs)) => {
                        emitconst!(Integer, lhs - rhs)
                    }
                    (op!(Add), lc!(Float, lhs), lc!(Float, rhs)) => emitconst!(Float, lhs + rhs),
                    (op!(Subtract), lc!(Float, lhs), lc!(Float, rhs)) => {
                        emitconst!(Float, lhs - rhs)
                    }
                    (op!(Multiply), lc!(Float, lhs), lc!(Float, rhs)) => {
                        emitconst!(Float, lhs * rhs)
                    }
                    (op!(Divide), lc!(Float, lhs), lc!(Float, rhs)) => emitconst!(Float, lhs / rhs),
                    (op!(Power), lc!(Float, lhs), lc!(Float, rhs)) => {
                        emitconst!(Float, lhs.powf(rhs))
                    }
                    (op!(Add), lc!(String, mut lhs), lc!(String, rhs)) => {
                        lhs.push_str(&rhs);
                        emitconst!(String, lhs);
                    }
                    (op, lhs, rhs) => {
                        self.emit(lhs, lhs_loc);
                        self.emit(rhs, rhs_loc);
                        self.emit(Instruction::BinaryOperation { op, inplace }, loc);
                    }
                }
            }
            other => self.emit(other, loc),
        }
    }

    fn flush(&mut self) {
        for (instruction, location) in self.buffer.drain(..) {
            self.inner.emit(instruction, location);
        }
    }
}

impl<O> OutputStream for PeepholeOptimizer<O>
where
    O: OutputStream,
{
    fn emit(&mut self, instruction: Instruction, location: Location) {
        self.optimize(instruction, location);
    }
    fn set_label(&mut self, label: crate::compile::Label) {
        self.flush();
        self.inner.set_label(label);
    }
    fn mark_generator(&mut self) {
        self.inner.mark_generator()
    }
}
