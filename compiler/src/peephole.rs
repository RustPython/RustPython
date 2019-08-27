use crate::output_stream::OutputStream;
use arrayvec::ArrayVec;
use rustpython_bytecode::bytecode::{self, CodeObject, Instruction, Label, Location};

const PEEPHOLE_BUFFER_SIZE: usize = 20;

pub struct InstructionMetadata {
    loc: Location,
    labels: Vec<Label>,
}

impl From<Vec<InstructionMetadata>> for InstructionMetadata {
    fn from(metas: Vec<Self>) -> Self {
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
impl From<Location> for InstructionMetadata {
    fn from(loc: Location) -> Self {
        InstructionMetadata {
            loc,
            labels: Vec::new(),
        }
    }
}

pub(crate) struct PeepholeOptimizer<O: OutputStream> {
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

    fn push(&mut self, instruction: Instruction, meta: InstructionMetadata) {
        if self.buffer.is_full() {
            let (instr, meta) = self.buffer.remove(0);
            Self::inner_emit(&mut self.inner, instr, meta);
        }
        self.buffer.push((instruction, meta));
    }

    fn pop(&mut self) -> (Instruction, InstructionMetadata) {
        self.buffer
            .pop()
            .expect("Failed to pop instruction from PeepholeOptimizer buffer")
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
        self.push(instruction, loc.into());
        optimize(self);
    }
    fn set_label(&mut self, label: Label) {
        if let Some(instr) = self.buffer.last_mut() {
            instr.1.labels.push(label)
        }
    }
    fn mark_generator(&mut self) {
        self.inner.mark_generator()
    }
}

impl<O: OutputStream> OptimizationBuffer for PeepholeOptimizer<O> {
    fn emit(&mut self, instruction: Instruction, meta: InstructionMetadata) {
        self.push(instruction, meta);
    }
    fn pop(&mut self) -> (Instruction, InstructionMetadata) {
        self.pop()
    }
}

// OPTIMIZATION

pub trait OptimizationBuffer {
    fn emit(&mut self, instruction: Instruction, meta: InstructionMetadata);
    fn pop(&mut self) -> (Instruction, InstructionMetadata);
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
    ($buf:expr, [$($metas:expr),*], $($arg:tt)*) => {
        $buf.emit(
            lc!($($arg)*),
            InstructionMetadata::from(vec![$($metas),*]),
        )
    };
}

pub fn optimize(buf: &mut impl OptimizationBuffer) {
    optimize_operator(buf);
    optimize_unpack(buf);
}

fn optimize_operator(buf: &mut impl OptimizationBuffer) {
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
            (op!(Divide), lc!(Float, lhs), lc!(Float, rhs)) => {
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

fn optimize_unpack(buf: &mut impl OptimizationBuffer) {
    let (instruction, meta) = buf.pop();
    if let Instruction::UnpackSequence { size } = instruction {
        let (arg, arg_meta) = buf.pop();
        match arg {
            Instruction::BuildTuple {
                size: tup_size,
                unpack,
            } if !unpack && tup_size == size => {
                buf.emit(
                    Instruction::Reverse { amount: size },
                    vec![arg_meta, meta].into(),
                );
            }
            arg => {
                buf.emit(arg, arg_meta);
                buf.emit(instruction, meta);
            }
        }
    } else {
        buf.emit(instruction, meta)
    }
}
