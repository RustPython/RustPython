use std::mem;

use crate::{
    bytecode::{
        Arg, BinaryOperator, BuildSliceArgCount, ComparisonOperator, ConversionFlag,
        IntrinsicFunction1, IntrinsicFunction2, Invert, Label, MakeFunctionFlags, NameIdx,
        RaiseKind, UnaryOperator, UnpackExArgs,
    },
    marshal::MarshalError,
};

/// A Single bytecode instruction.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum Instruction {
    Nop,
    /// Importing by name
    ImportName {
        idx: Arg<NameIdx>,
    },
    /// Importing without name
    ImportNameless,
    /// from ... import ...
    ImportFrom {
        idx: Arg<NameIdx>,
    },
    LoadFast(Arg<NameIdx>),
    LoadNameAny(Arg<NameIdx>),
    LoadGlobal(Arg<NameIdx>),
    LoadDeref(Arg<NameIdx>),
    LoadClassDeref(Arg<NameIdx>),
    StoreFast(Arg<NameIdx>),
    StoreLocal(Arg<NameIdx>),
    StoreGlobal(Arg<NameIdx>),
    StoreDeref(Arg<NameIdx>),
    DeleteFast(Arg<NameIdx>),
    DeleteLocal(Arg<NameIdx>),
    DeleteGlobal(Arg<NameIdx>),
    DeleteDeref(Arg<NameIdx>),
    LoadClosure(Arg<NameIdx>),
    Subscript,
    StoreSubscript,
    DeleteSubscript,
    /// Performs `is` comparison, or `is not` if `invert` is 1.
    IsOp(Arg<Invert>),
    /// Performs `in` comparison, or `not in` if `invert` is 1.
    ContainsOp(Arg<Invert>),
    StoreAttr {
        idx: Arg<NameIdx>,
    },
    DeleteAttr {
        idx: Arg<NameIdx>,
    },
    LoadConst {
        /// index into constants vec
        idx: Arg<u32>,
    },
    UnaryOperation {
        op: Arg<UnaryOperator>,
    },
    BinaryOperation {
        op: Arg<BinaryOperator>,
    },
    BinaryOperationInplace {
        op: Arg<BinaryOperator>,
    },
    BinarySubscript,
    LoadAttr {
        idx: Arg<NameIdx>,
    },
    CompareOperation {
        op: Arg<ComparisonOperator>,
    },
    CopyItem {
        index: Arg<u32>,
    },
    Pop,
    Swap {
        index: Arg<u32>,
    },
    ToBool,
    GetIter,
    GetLen,
    CallIntrinsic1 {
        func: Arg<IntrinsicFunction1>,
    },
    CallIntrinsic2 {
        func: Arg<IntrinsicFunction2>,
    },
    Continue {
        target: Arg<Label>,
    },
    Break {
        target: Arg<Label>,
    },
    /// Performs exception matching for except.
    /// Tests whether the STACK[-2] is an exception matching STACK[-1].
    /// Pops STACK[-1] and pushes the boolean result of the test.
    JumpIfNotExcMatch(Arg<Label>),
    Jump {
        target: Arg<Label>,
    },
    /// Pop the top of the stack, and jump if this value is true.
    PopJumpIfTrue {
        target: Arg<Label>,
    },
    /// Pop the top of the stack, and jump if this value is false.
    PopJumpIfFalse {
        target: Arg<Label>,
    },
    /// Peek at the top of the stack, and jump if this value is true.
    /// Otherwise, pop top of stack.
    JumpIfTrueOrPop {
        target: Arg<Label>,
    },
    /// Peek at the top of the stack, and jump if this value is false.
    /// Otherwise, pop top of stack.
    JumpIfFalseOrPop {
        target: Arg<Label>,
    },
    MakeFunction,
    SetFunctionAttribute {
        attr: Arg<MakeFunctionFlags>,
    },
    CallFunctionPositional {
        nargs: Arg<u32>,
    },
    CallFunctionKeyword {
        nargs: Arg<u32>,
    },
    CallFunctionEx {
        has_kwargs: Arg<bool>,
    },
    LoadMethod {
        idx: Arg<NameIdx>,
    },
    CallMethodPositional {
        nargs: Arg<u32>,
    },
    CallMethodKeyword {
        nargs: Arg<u32>,
    },
    CallMethodEx {
        has_kwargs: Arg<bool>,
    },
    ForIter {
        target: Arg<Label>,
    },
    ReturnValue,
    ReturnConst {
        idx: Arg<u32>,
    },
    YieldValue,
    YieldFrom,

    /// Resume execution (e.g., at function start, after yield, etc.)
    Resume {
        arg: Arg<u32>,
    },

    SetupAnnotation,
    SetupLoop,

    /// Setup a finally handler, which will be called whenever one of this events occurs:
    /// - the block is popped
    /// - the function returns
    /// - an exception is returned
    SetupFinally {
        handler: Arg<Label>,
    },

    /// Enter a finally block, without returning, excepting, just because we are there.
    EnterFinally,

    /// Marker bytecode for the end of a finally sequence.
    /// When this bytecode is executed, the eval loop does one of those things:
    /// - Continue at a certain bytecode position
    /// - Propagate the exception
    /// - Return from a function
    /// - Do nothing at all, just continue
    EndFinally,

    SetupExcept {
        handler: Arg<Label>,
    },
    SetupWith {
        end: Arg<Label>,
    },
    WithCleanupStart,
    WithCleanupFinish,
    PopBlock,
    Raise {
        kind: Arg<RaiseKind>,
    },
    BuildString {
        size: Arg<u32>,
    },
    BuildTuple {
        size: Arg<u32>,
    },
    BuildTupleFromTuples {
        size: Arg<u32>,
    },
    BuildTupleFromIter,
    BuildList {
        size: Arg<u32>,
    },
    BuildListFromTuples {
        size: Arg<u32>,
    },
    BuildSet {
        size: Arg<u32>,
    },
    BuildSetFromTuples {
        size: Arg<u32>,
    },
    BuildMap {
        size: Arg<u32>,
    },
    BuildMapForCall {
        size: Arg<u32>,
    },
    DictUpdate {
        index: Arg<u32>,
    },
    BuildSlice {
        argc: Arg<BuildSliceArgCount>,
    },
    ListAppend {
        i: Arg<u32>,
    },
    SetAdd {
        i: Arg<u32>,
    },
    MapAdd {
        i: Arg<u32>,
    },

    PrintExpr,
    LoadBuildClass,
    UnpackSequence {
        size: Arg<u32>,
    },
    UnpackEx {
        args: Arg<UnpackExArgs>,
    },
    FormatValue {
        conversion: Arg<ConversionFlag>,
    },
    PopException,
    Reverse {
        amount: Arg<u32>,
    },
    GetAwaitable,
    BeforeAsyncWith,
    SetupAsyncWith {
        end: Arg<Label>,
    },
    GetAIter,
    GetANext,
    EndAsyncFor,
    MatchMapping,
    MatchSequence,
    MatchKeys,
    MatchClass(Arg<u32>),
    ExtendedArg,
    // If you add a new instruction here, be sure to keep LAST_INSTRUCTION updated
}

// This must be kept up to date to avoid marshaling errors
const LAST_INSTRUCTION: Instruction = Instruction::ExtendedArg;

const _: () = assert!(mem::size_of::<Instruction>() == 1);

impl From<Instruction> for u8 {
    #[inline]
    fn from(ins: Instruction) -> Self {
        // SAFETY: there's no padding bits
        unsafe { std::mem::transmute::<Instruction, Self>(ins) }
    }
}

impl TryFrom<u8> for Instruction {
    type Error = MarshalError;

    #[inline]
    fn try_from(value: u8) -> Result<Self, MarshalError> {
        if value <= u8::from(LAST_INSTRUCTION) {
            Ok(unsafe { std::mem::transmute::<u8, Self>(value) })
        } else {
            Err(MarshalError::InvalidBytecode)
        }
    }
}
