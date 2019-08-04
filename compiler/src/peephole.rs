use crate::compile::Label;
use crate::output_stream::OutputStream;
use arrayvec::ArrayVec;
use rustpython_bytecode::bytecode::{self, CodeObject, Instruction, Location};

const PEEPHOLE_BUFFER_SIZE: usize = 20;

struct InstructionMetadata {
    loc: Location,
    labels: Vec<Label>,
}

impl InstructionMetadata {
    fn from_multiple(metas: Vec<Self>) -> Self {
        debug_assert!(!metas.is_empty(), "`metas` must not be empty");
        InstructionMetadata {
            loc: metas[0].loc.clone(),
            labels: metas
                .into_iter()
                .flat_map(|meta| meta.labels.into_iter())
                .collect(),
        }
    }
}

pub struct PeepholeOptimizer<O: OutputStream> {
    inner: O,
    buffer: ArrayVec<[(Instruction, InstructionMetadata); PEEPHOLE_BUFFER_SIZE]>,
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

    fn inner_emit(inner: &mut O, instruction: Instruction, meta: InstructionMetadata) {
        inner.emit(instruction, meta.loc);
        for label in meta.labels {
            inner.set_label(label);
        }
    }

    fn emit(&mut self, instruction: Instruction, meta: InstructionMetadata) {
        if self.buffer.is_full() {
            let (instr, meta) = self.buffer.remove(0);
            Self::inner_emit(&mut self.inner, instr, meta);
        }
        // safe because we just checked that: if full then remove one element from it
        unsafe { self.buffer.push_unchecked((instruction, meta)) };
    }

    fn pop(&mut self) -> (Instruction, InstructionMetadata) {
        self.buffer.pop().unwrap()
    }

    fn optimize(&mut self, instruction: Instruction, meta: InstructionMetadata) {
        match instruction {
            Instruction::BinaryOperation { op, inplace } => {
                let (rhs, rhs_meta) = self.pop();
                let (lhs, lhs_meta) = self.pop();
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
                    ([$($metas:expr),*], $($arg:tt)*) => {
                        self.emit(
                            lc!($($arg)*),
                            InstructionMetadata::from_multiple(vec![$($metas),*]),
                        )
                    };
                }
                macro_rules! op {
                    ($op:ident) => {
                        bytecode::BinaryOperator::$op
                    };
                }
                match (op, lhs, rhs) {
                    (op!(Add), lc!(Integer, lhs), lc!(Integer, rhs)) => {
                        emitconst!([lhs_meta, rhs_meta], Integer, lhs + rhs)
                    }
                    (op!(Subtract), lc!(Integer, lhs), lc!(Integer, rhs)) => {
                        emitconst!([lhs_meta, rhs_meta], Integer, lhs - rhs)
                    }
                    (op!(Add), lc!(Float, lhs), lc!(Float, rhs)) => {
                        emitconst!([lhs_meta, rhs_meta], Float, lhs + rhs)
                    }
                    (op!(Subtract), lc!(Float, lhs), lc!(Float, rhs)) => {
                        emitconst!([lhs_meta, rhs_meta], Float, lhs - rhs)
                    }
                    (op!(Multiply), lc!(Float, lhs), lc!(Float, rhs)) => {
                        emitconst!([lhs_meta, rhs_meta], Float, lhs * rhs)
                    }
                    (op!(Divide), lc!(Float, lhs), lc!(Float, rhs)) => {
                        emitconst!([lhs_meta, rhs_meta], Float, lhs / rhs)
                    }
                    (op!(Power), lc!(Float, lhs), lc!(Float, rhs)) => {
                        emitconst!([lhs_meta, rhs_meta], Float, lhs.powf(rhs))
                    }
                    (op!(Add), lc!(String, mut lhs), lc!(String, rhs)) => {
                        lhs.push_str(&rhs);
                        emitconst!([lhs_meta, rhs_meta], String, lhs);
                    }
                    (op, lhs, rhs) => {
                        self.emit(lhs, lhs_meta);
                        self.emit(rhs, rhs_meta);
                        self.emit(Instruction::BinaryOperation { op, inplace }, meta);
                    }
                }
            }
            other => self.emit(other, meta),
        }
    }

    fn flush(&mut self) {
        for (instruction, meta) in self.buffer.drain(..) {
            Self::inner_emit(&mut self.inner, instruction, meta);
        }
    }
}

impl<O> OutputStream for PeepholeOptimizer<O>
where
    O: OutputStream,
{
    fn emit(&mut self, instruction: Instruction, loc: Location) {
        self.optimize(
            instruction,
            InstructionMetadata {
                loc,
                labels: Vec::new(),
            },
        );
    }
    fn set_label(&mut self, label: crate::compile::Label) {
        if let Some(instr) = self.buffer.last_mut() {
            instr.1.labels.push(label)
        }
    }
    fn mark_generator(&mut self) {
        self.inner.mark_generator()
    }
}
