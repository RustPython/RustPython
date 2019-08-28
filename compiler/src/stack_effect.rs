use rustpython_bytecode::bytecode::{CallType, Instruction};

/// Determine the effect on the stack of the given instruction.
///
/// This function must match what is executed in frame.rs
/// The return value is the amount of stack change created by the
/// given opcode.
pub fn stack_effect(instruction: &Instruction) -> isize {
    use Instruction::*;

    match instruction {
        ImportStar => -1,
        Import { .. } => 1,
        ImportFrom { .. } => 1,
        PopException => 0,
        Jump { .. } => 0,
        JumpIfFalse { .. } => -1,
        JumpIfTrue { .. } => -1,
        JumpIfFalseOrPop { .. } => -1,
        JumpIfTrueOrPop { .. } => -1,
        Break => 0,
        Continue => 0,
        PopBlock => 0,
        GetIter => 0,
        ForIter { .. } => 1,
        CallFunction { typ } => match typ {
            CallType::Positional(amount) => -(*amount as isize),
            CallType::Keyword(amount) => -1 - (*amount as isize),
            CallType::Ex(has_kwargs) => {
                if *has_kwargs {
                    -2
                } else {
                    -1
                }
            }
        },
        UnaryOperation { .. } => 0,
        BinaryOperation { .. } => -1,
        Pop => -1,
        Duplicate => 1,
        Rotate { .. } => 0,
        Reverse { .. } => 0,
        Subscript { .. } => -1,
        StoreSubscript { .. } => -3,
        DeleteSubscript { .. } => -2,
        LoadAttr { .. } => 0,
        StoreAttr { .. } => -2,
        DeleteAttr { .. } => -1,
        LoadName { .. } => 1,
        StoreName { .. } => -1,
        DeleteName { .. } => 0,
        LoadConst { .. } => 1,
        ReturnValue { .. } => -1,
        YieldValue { .. } => 0,
        YieldFrom { .. } => 0,
        CompareOperation { .. } => -1,
        BuildList { size, .. }
        | BuildSet { size, .. }
        | BuildTuple { size, .. }
        | BuildSlice { size, .. }
        | BuildString { size } => 1 - (*size as isize),
        BuildMap { size, unpack } => {
            let size = *size as isize;
            if *unpack {
                1 - size
            } else {
                1 - size * 2
            }
        }
        ListAppend { .. } => -1,
        SetAdd { .. } => -1,
        MapAdd { .. } => -2,
        Unpack => {
            unimplemented!("we cannot know the effect of this instruction on the stack :(");
        }
        UnpackEx { before, after } => -1 + (*before as isize) + (*after as isize) + 1,
        UnpackSequence { size } => -1 + (*size as isize),
        SetupLoop { .. } => 0,
        SetupWith { .. } => 0,
        CleanupWith { .. } => 0,
        SetupExcept { .. } => 0,
        SetupFinally { .. } => 0,
        EnterFinally { .. } => 0,
        EndFinally { .. } => 0,
        LoadBuildClass => 1,
        MakeFunction { .. } => 0,
        Raise { argc } => -(*argc as isize),
        PrintExpr => -1,
        FormatValue { .. } => 0,
    }
}
