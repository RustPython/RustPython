use core::{fmt, marker::PhantomData, mem};

use crate::{
    bytecode::{
        BorrowedConstant, Constant, InstrDisplayContext, decode_load_attr_arg,
        oparg::{
            BinaryOperator, BuildSliceArgCount, ComparisonOperator, ConvertValueOparg,
            IntrinsicFunction1, IntrinsicFunction2, Invert, Label, MakeFunctionFlags, NameIdx,
            OpArg, OpArgByte, OpArgType, RaiseKind, UnpackExArgs,
        },
    },
    marshal::MarshalError,
};

/// A Single bytecode instruction.
/// Instructions are ordered to match CPython 3.13 opcode numbers exactly.
/// HAVE_ARGUMENT = 44: opcodes 0-43 have no argument, 44+ have arguments.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum Instruction {
    // ==================== No-argument instructions (opcode < 44) ====================
    Cache = 0, // Placeholder
    BeforeAsyncWith = 1,
    BeforeWith = 2,
    BinaryOpInplaceAddUnicode = 3, // Placeholder
    BinarySlice = 4,               // Placeholder
    BinarySubscr = 5,
    CheckEgMatch = 6,
    CheckExcMatch = 7,
    CleanupThrow = 8,
    DeleteSubscr = 9,
    EndAsyncFor = 10,
    EndFor = 11, // Placeholder
    EndSend = 12,
    ExitInitCheck = 13, // Placeholder
    FormatSimple = 14,
    FormatWithSpec = 15,
    GetAIter = 16,
    Reserved = 17,
    GetANext = 18,
    GetIter = 19,
    GetLen = 20,
    GetYieldFromIter = 21,
    InterpreterExit = 22,    // Placeholder
    LoadAssertionError = 23, // Placeholder
    LoadBuildClass = 24,
    LoadLocals = 25, // Placeholder
    MakeFunction = 26,
    MatchKeys = 27,
    MatchMapping = 28,
    MatchSequence = 29,
    Nop = 30,
    PopExcept = 31,
    PopTop = 32,
    PushExcInfo = 33,
    PushNull = 34,        // Placeholder
    ReturnGenerator = 35, // Placeholder
    ReturnValue = 36,
    SetupAnnotations = 37,
    StoreSlice = 38, // Placeholder
    StoreSubscr = 39,
    ToBool = 40,
    UnaryInvert = 41,
    UnaryNegative = 42,
    UnaryNot = 43,
    WithExceptStart = 44,
    // ==================== With-argument instructions (opcode > 44) ====================
    BinaryOp {
        op: Arg<BinaryOperator>,
    } = 45,
    BuildConstKeyMap {
        size: Arg<u32>,
    } = 46, // Placeholder
    BuildList {
        size: Arg<u32>,
    } = 47,
    BuildMap {
        size: Arg<u32>,
    } = 48,
    BuildSet {
        size: Arg<u32>,
    } = 49,
    BuildSlice {
        argc: Arg<BuildSliceArgCount>,
    } = 50,
    BuildString {
        size: Arg<u32>,
    } = 51,
    BuildTuple {
        size: Arg<u32>,
    } = 52,
    Call {
        nargs: Arg<u32>,
    } = 53,
    CallFunctionEx {
        has_kwargs: Arg<bool>,
    } = 54,
    CallIntrinsic1 {
        func: Arg<IntrinsicFunction1>,
    } = 55,
    CallIntrinsic2 {
        func: Arg<IntrinsicFunction2>,
    } = 56,
    CallKw {
        nargs: Arg<u32>,
    } = 57,
    CompareOp {
        op: Arg<ComparisonOperator>,
    } = 58,
    ContainsOp(Arg<Invert>) = 59,
    ConvertValue {
        oparg: Arg<ConvertValueOparg>,
    } = 60,
    Copy {
        index: Arg<u32>,
    } = 61,
    CopyFreeVars {
        count: Arg<u32>,
    } = 62, // Placeholder
    DeleteAttr {
        idx: Arg<NameIdx>,
    } = 63,
    DeleteDeref(Arg<NameIdx>) = 64,
    DeleteFast(Arg<NameIdx>) = 65,
    DeleteGlobal(Arg<NameIdx>) = 66,
    DeleteName(Arg<NameIdx>) = 67,
    DictMerge {
        index: Arg<u32>,
    } = 68, // Placeholder
    DictUpdate {
        index: Arg<u32>,
    } = 69,
    EnterExecutor = 70, // Placeholder
    ExtendedArg = 71,
    ForIter {
        target: Arg<Label>,
    } = 72,
    GetAwaitable = 73, // TODO: Make this instruction to hold an oparg
    ImportFrom {
        idx: Arg<NameIdx>,
    } = 74,
    ImportName {
        idx: Arg<NameIdx>,
    } = 75,
    IsOp(Arg<Invert>) = 76,
    JumpBackward {
        target: Arg<Label>,
    } = 77, // Placeholder
    JumpBackwardNoInterrupt {
        target: Arg<Label>,
    } = 78, // Placeholder
    JumpForward {
        target: Arg<Label>,
    } = 79, // Placeholder
    ListAppend {
        i: Arg<u32>,
    } = 80,
    ListExtend {
        i: Arg<u32>,
    } = 81, // Placeholder
    LoadAttr {
        idx: Arg<NameIdx>,
    } = 82,
    LoadConst {
        idx: Arg<u32>,
    } = 83,
    LoadDeref(Arg<NameIdx>) = 84,
    LoadFast(Arg<NameIdx>) = 85,
    LoadFastAndClear(Arg<NameIdx>) = 86,
    LoadFastCheck(Arg<NameIdx>) = 87, // Placeholder
    LoadFastLoadFast {
        arg: Arg<u32>,
    } = 88, // Placeholder
    LoadFromDictOrDeref(Arg<NameIdx>) = 89,
    LoadFromDictOrGlobals(Arg<NameIdx>) = 90, // Placeholder
    LoadGlobal(Arg<NameIdx>) = 91,
    LoadName(Arg<NameIdx>) = 92,
    LoadSuperAttr {
        arg: Arg<u32>,
    } = 93,
    MakeCell(Arg<NameIdx>) = 94, // Placeholder
    MapAdd {
        i: Arg<u32>,
    } = 95,
    MatchClass(Arg<u32>) = 96,
    PopJumpIfFalse {
        target: Arg<Label>,
    } = 97,
    PopJumpIfNone {
        target: Arg<Label>,
    } = 98, // Placeholder
    PopJumpIfNotNone {
        target: Arg<Label>,
    } = 99, // Placeholder
    PopJumpIfTrue {
        target: Arg<Label>,
    } = 100,
    RaiseVarargs {
        kind: Arg<RaiseKind>,
    } = 101,
    Reraise {
        depth: Arg<u32>,
    } = 102,
    ReturnConst {
        idx: Arg<u32>,
    } = 103,
    Send {
        target: Arg<Label>,
    } = 104,
    SetAdd {
        i: Arg<u32>,
    } = 105,
    SetFunctionAttribute {
        attr: Arg<MakeFunctionFlags>,
    } = 106,
    SetUpdate {
        i: Arg<u32>,
    } = 107, // Placeholder
    StoreAttr {
        idx: Arg<NameIdx>,
    } = 108,
    StoreDeref(Arg<NameIdx>) = 109,
    StoreFast(Arg<NameIdx>) = 110,
    StoreFastLoadFast {
        store_idx: Arg<NameIdx>,
        load_idx: Arg<NameIdx>,
    } = 111,
    StoreFastStoreFast {
        arg: Arg<u32>,
    } = 112, // Placeholder
    StoreGlobal(Arg<NameIdx>) = 113,
    StoreName(Arg<NameIdx>) = 114,
    Swap {
        index: Arg<u32>,
    } = 115,
    UnpackEx {
        args: Arg<UnpackExArgs>,
    } = 116,
    UnpackSequence {
        size: Arg<u32>,
    } = 117,
    YieldValue {
        arg: Arg<u32>,
    } = 118,
    Resume {
        arg: Arg<u32>,
    } = 149,
    // ===== LOAD_SUPER_* Pseudo Opcodes (136-138) =====
    // These are converted to LoadSuperAttr during bytecode finalization.
    // "Zero" variants are for 0-arg super() calls (has_class=false).
    // Non-"Zero" variants are for 2-arg super(cls, self) calls (has_class=true).
    /// 2-arg super(cls, self).method() - has_class=true, load_method=true
    LoadSuperMethod {
        idx: Arg<NameIdx>,
    } = 136, // CPython uses pseudo-op 260
    LoadZeroSuperAttr {
        idx: Arg<NameIdx>,
    } = 137, // CPython uses pseudo-op 261
    LoadZeroSuperMethod {
        idx: Arg<NameIdx>,
    } = 138, // CPython uses pseudo-op 262
    // ==================== RustPython-only instructions (119-135) ====================
    // Ideally, we want to be fully aligned with CPython opcodes, but we still have some leftovers.
    // So we assign random IDs to these opcodes.
    Break {
        target: Arg<Label>,
    } = 119,
    BuildListFromTuples {
        size: Arg<u32>,
    } = 120,
    BuildMapForCall {
        size: Arg<u32>,
    } = 121,
    BuildSetFromTuples {
        size: Arg<u32>,
    } = 122,
    BuildTupleFromIter = 123,
    BuildTupleFromTuples {
        size: Arg<u32>,
    } = 124,
    Continue {
        target: Arg<Label>,
    } = 128,
    JumpIfFalseOrPop {
        target: Arg<Label>,
    } = 129,
    JumpIfTrueOrPop {
        target: Arg<Label>,
    } = 130,
    JumpIfNotExcMatch(Arg<Label>) = 131,
    SetExcInfo = 134,
    Subscript = 135,
    // ===== Pseudo Opcodes (252+) ======
    Jump {
        target: Arg<Label>,
    } = 252, // CPython uses pseudo-op 256
    LoadClosure(Arg<NameIdx>) = 253, // CPython uses pseudo-op 258
    LoadAttrMethod {
        idx: Arg<NameIdx>,
    } = 254, // CPython uses pseudo-op 259
    PopBlock = 255,                  // CPython uses pseudo-op 263
}

const _: () = assert!(mem::size_of::<Instruction>() == 1);

impl From<Instruction> for u8 {
    #[inline]
    fn from(ins: Instruction) -> Self {
        // SAFETY: there's no padding bits
        unsafe { core::mem::transmute::<Instruction, Self>(ins) }
    }
}

impl TryFrom<u8> for Instruction {
    type Error = MarshalError;

    #[inline]
    fn try_from(value: u8) -> Result<Self, MarshalError> {
        // CPython-compatible opcodes (0-118)
        let cpython_start = u8::from(Self::Cache);
        let cpython_end = u8::from(Self::YieldValue { arg: Arg::marker() });

        // Resume has a non-contiguous opcode (149)
        let resume_id = u8::from(Self::Resume { arg: Arg::marker() });

        // RustPython-only opcodes (explicit list to avoid gaps like 125-127)
        let custom_ops: &[u8] = &[
            u8::from(Self::Break {
                target: Arg::marker(),
            }),
            u8::from(Self::BuildListFromTuples {
                size: Arg::marker(),
            }),
            u8::from(Self::BuildMapForCall {
                size: Arg::marker(),
            }),
            u8::from(Self::BuildSetFromTuples {
                size: Arg::marker(),
            }),
            u8::from(Self::BuildTupleFromIter),
            u8::from(Self::BuildTupleFromTuples {
                size: Arg::marker(),
            }),
            // 125, 126, 127 are unused
            u8::from(Self::Continue {
                target: Arg::marker(),
            }),
            u8::from(Self::JumpIfFalseOrPop {
                target: Arg::marker(),
            }),
            u8::from(Self::JumpIfTrueOrPop {
                target: Arg::marker(),
            }),
            u8::from(Self::JumpIfNotExcMatch(Arg::marker())),
            u8::from(Self::SetExcInfo),
            u8::from(Self::Subscript),
            // LOAD_SUPER_* pseudo opcodes (136-138)
            u8::from(Self::LoadSuperMethod { idx: Arg::marker() }),
            u8::from(Self::LoadZeroSuperAttr { idx: Arg::marker() }),
            u8::from(Self::LoadZeroSuperMethod { idx: Arg::marker() }),
        ];

        // Pseudo opcodes (252-255)
        let pseudo_start = u8::from(Self::Jump {
            target: Arg::marker(),
        });
        let pseudo_end = u8::from(Self::PopBlock);

        if (cpython_start..=cpython_end).contains(&value)
            || value == resume_id
            || custom_ops.contains(&value)
            || (pseudo_start..=pseudo_end).contains(&value)
        {
            Ok(unsafe { core::mem::transmute::<u8, Self>(value) })
        } else {
            Err(Self::Error::InvalidBytecode)
        }
    }
}

impl Instruction {
    /// Gets the label stored inside this instruction, if it exists
    #[inline]
    pub const fn label_arg(&self) -> Option<Arg<Label>> {
        match self {
            Self::Jump { target: l }
            | Self::JumpBackward { target: l }
            | Self::JumpBackwardNoInterrupt { target: l }
            | Self::JumpForward { target: l }
            | Self::JumpIfNotExcMatch(l)
            | Self::PopJumpIfTrue { target: l }
            | Self::PopJumpIfFalse { target: l }
            | Self::JumpIfTrueOrPop { target: l }
            | Self::JumpIfFalseOrPop { target: l }
            | Self::ForIter { target: l }
            | Self::Break { target: l }
            | Self::Continue { target: l }
            | Self::Send { target: l } => Some(*l),
            _ => None,
        }
    }

    /// Whether this is an unconditional branching
    ///
    /// # Examples
    ///
    /// ```
    /// use rustpython_compiler_core::bytecode::{Arg, Instruction};
    /// let jump_inst = Instruction::Jump { target: Arg::marker() };
    /// assert!(jump_inst.unconditional_branch())
    /// ```
    pub const fn unconditional_branch(&self) -> bool {
        matches!(
            self,
            Self::Jump { .. }
                | Self::JumpForward { .. }
                | Self::JumpBackward { .. }
                | Self::JumpBackwardNoInterrupt { .. }
                | Self::Continue { .. }
                | Self::Break { .. }
                | Self::ReturnValue
                | Self::ReturnConst { .. }
                | Self::RaiseVarargs { .. }
                | Self::Reraise { .. }
        )
    }

    /// What effect this instruction has on the stack
    ///
    /// # Examples
    ///
    /// ```
    /// use rustpython_compiler_core::bytecode::{Arg, Instruction, Label};
    /// let (target, jump_arg) = Arg::new(Label(0xF));
    /// let jump_instruction = Instruction::Jump { target };
    /// assert_eq!(jump_instruction.stack_effect(jump_arg, true), 0);
    /// ```
    ///
    pub fn stack_effect(&self, arg: OpArg, jump: bool) -> i32 {
        match self {
            Self::Nop => 0,
            Self::ImportName { .. } => -1,
            Self::ImportFrom { .. } => 1,
            Self::LoadFast(_) => 1,
            Self::LoadFastAndClear(_) => 1,
            Self::LoadName(_) => 1,
            Self::LoadGlobal(_) => 1,
            Self::LoadDeref(_) => 1,
            Self::StoreFast(_) => -1,
            Self::StoreName(_) => -1,
            Self::StoreGlobal(_) => -1,
            Self::StoreDeref(_) => -1,
            Self::StoreFastLoadFast { .. } => 0, // pop 1, push 1
            Self::DeleteFast(_) => 0,
            Self::DeleteName(_) => 0,
            Self::DeleteGlobal(_) => 0,
            Self::DeleteDeref(_) => 0,
            Self::LoadFromDictOrDeref(_) => 1,
            Self::LoadClosure(_) => 1,
            Self::Subscript => -1,
            Self::StoreSubscr => -3,
            Self::DeleteSubscr => -2,
            Self::LoadAttr { .. } => 0,
            // LoadAttrMethod: pop obj, push method + self_or_null
            Self::LoadAttrMethod { .. } => 1,
            Self::StoreAttr { .. } => -2,
            Self::DeleteAttr { .. } => -1,
            Self::LoadConst { .. } => 1,
            Self::Reserved => 0,
            Self::BinaryOp { .. } => -1,
            Self::CompareOp { .. } => -1,
            Self::BinarySubscr => -1,
            Self::Copy { .. } => 1,
            Self::PopTop => -1,
            Self::Swap { .. } => 0,
            Self::ToBool => 0,
            Self::GetIter => 0,
            Self::GetLen => 1,
            Self::CallIntrinsic1 { .. } => 0,  // Takes 1, pushes 1
            Self::CallIntrinsic2 { .. } => -1, // Takes 2, pushes 1
            Self::Continue { .. } => 0,
            Self::Break { .. } => 0,
            Self::Jump { .. } => 0,
            Self::PopJumpIfTrue { .. } => -1,
            Self::PopJumpIfFalse { .. } => -1,
            Self::JumpIfTrueOrPop { .. } => {
                if jump {
                    0
                } else {
                    -1
                }
            }
            Self::JumpIfFalseOrPop { .. } => {
                if jump {
                    0
                } else {
                    -1
                }
            }
            Self::MakeFunction => {
                // CPython 3.13 style: MakeFunction only pops code object
                -1 + 1 // pop code, push function
            }
            Self::SetFunctionAttribute { .. } => {
                // pops attribute value and function, pushes function back
                -2 + 1
            }
            // Call: pops nargs + self_or_null + callable, pushes result
            Self::Call { nargs } => -(nargs.get(arg) as i32) - 2 + 1,
            // CallKw: pops kw_names_tuple + nargs + self_or_null + callable, pushes result
            Self::CallKw { nargs } => -1 - (nargs.get(arg) as i32) - 2 + 1,
            // CallFunctionEx: pops kwargs(if any) + args_tuple + self_or_null + callable, pushes result
            Self::CallFunctionEx { has_kwargs } => -1 - (has_kwargs.get(arg) as i32) - 2 + 1,
            Self::CheckEgMatch => 0, // pops 2 (exc, type), pushes 2 (rest, match)
            Self::ConvertValue { .. } => 0,
            Self::FormatSimple => 0,
            Self::FormatWithSpec => -1,
            Self::ForIter { .. } => {
                if jump {
                    -1
                } else {
                    1
                }
            }
            Self::IsOp(_) => -1,
            Self::ContainsOp(_) => -1,
            Self::JumpIfNotExcMatch(_) => -2,
            Self::ReturnValue => -1,
            Self::ReturnConst { .. } => 0,
            Self::Resume { .. } => 0,
            Self::YieldValue { .. } => 0,
            // SEND: (receiver, val) -> (receiver, retval) - no change, both paths keep same depth
            Self::Send { .. } => 0,
            // END_SEND: (receiver, value) -> (value)
            Self::EndSend => -1,
            // CLEANUP_THROW: (sub_iter, last_sent_val, exc) -> (None, value) = 3 pop, 2 push = -1
            Self::CleanupThrow => -1,
            Self::SetExcInfo => 0,
            Self::PushExcInfo => 1,    // [exc] -> [prev_exc, exc]
            Self::CheckExcMatch => 0,  // [exc, type] -> [exc, bool] (pops type, pushes bool)
            Self::Reraise { .. } => 0, // Exception raised, stack effect doesn't matter
            Self::SetupAnnotations => 0,
            Self::BeforeWith => 1, // push __exit__, then replace ctx_mgr with __enter__ result
            Self::WithExceptStart => 1, // push __exit__ result
            Self::PopBlock => 0,
            Self::RaiseVarargs { kind } => {
                // Stack effects for different raise kinds:
                // - Reraise (0): gets from VM state, no stack pop
                // - Raise (1): pops 1 exception
                // - RaiseCause (2): pops 2 (exception + cause)
                // - ReraiseFromStack (3): pops 1 exception from stack
                match kind.get(arg) {
                    RaiseKind::BareRaise => 0,
                    RaiseKind::Raise => -1,
                    RaiseKind::RaiseCause => -2,
                    RaiseKind::ReraiseFromStack => -1,
                }
            }
            Self::BuildString { size } => -(size.get(arg) as i32) + 1,
            Self::BuildTuple { size, .. } => -(size.get(arg) as i32) + 1,
            Self::BuildTupleFromTuples { size, .. } => -(size.get(arg) as i32) + 1,
            Self::BuildList { size, .. } => -(size.get(arg) as i32) + 1,
            Self::BuildListFromTuples { size, .. } => -(size.get(arg) as i32) + 1,
            Self::BuildSet { size, .. } => -(size.get(arg) as i32) + 1,
            Self::BuildSetFromTuples { size, .. } => -(size.get(arg) as i32) + 1,
            Self::BuildTupleFromIter => 0,
            Self::BuildMap { size } => {
                let nargs = size.get(arg) * 2;
                -(nargs as i32) + 1
            }
            Self::BuildMapForCall { size } => {
                let nargs = size.get(arg);
                -(nargs as i32) + 1
            }
            Self::DictUpdate { .. } => -1,
            Self::BuildSlice { argc } => {
                // push 1
                // pops either 2/3
                // Default to Two (2 args) if arg is invalid
                1 - (argc
                    .try_get(arg)
                    .unwrap_or(BuildSliceArgCount::Two)
                    .argc()
                    .get() as i32)
            }
            Self::ListAppend { .. } => -1,
            Self::SetAdd { .. } => -1,
            Self::MapAdd { .. } => -2,
            Self::LoadBuildClass => 1,
            Self::UnpackSequence { size } => -1 + size.get(arg) as i32,
            Self::UnpackEx { args } => {
                let UnpackExArgs { before, after } = args.get(arg);
                -1 + before as i32 + 1 + after as i32
            }
            Self::PopExcept => 0,
            Self::GetAwaitable => 0,
            Self::BeforeAsyncWith => 1,
            Self::GetAIter => 0,
            Self::GetANext => 1,
            Self::EndAsyncFor => -2,  // pops (awaitable, exc) from stack
            Self::MatchMapping => 1,  // Push bool result
            Self::MatchSequence => 1, // Push bool result
            Self::MatchKeys => 1, // Pop 2 (subject, keys), push 3 (subject, keys_or_none, values_or_none)
            Self::MatchClass(_) => -2,
            Self::ExtendedArg => 0,
            Self::UnaryInvert => 0,
            Self::UnaryNegative => 0,
            Self::UnaryNot => 0,
            Self::GetYieldFromIter => 0,
            Self::PushNull => 1, // Push NULL for call protocol
            // LoadSuperAttr: pop [super, class, self], push [attr] or [method, self_or_null]
            // stack_effect depends on load_method flag (bit 0 of oparg)
            Self::LoadSuperAttr { arg: idx } => {
                let (_, load_method, _) = decode_load_super_attr_arg(idx.get(arg));
                if load_method { -3 + 2 } else { -3 + 1 }
            }
            // Pseudo instructions (calculated before conversion)
            Self::LoadSuperMethod { .. } => -3 + 2, // pop 3, push [method, self_or_null]
            Self::LoadZeroSuperAttr { .. } => -3 + 1, // pop 3, push [attr]
            Self::LoadZeroSuperMethod { .. } => -3 + 2, // pop 3, push [method, self_or_null]
            Self::Cache => 0,
            Self::BinarySlice => 0,
            Self::BinaryOpInplaceAddUnicode => 0,
            Self::EndFor => 0,
            Self::ExitInitCheck => 0,
            Self::InterpreterExit => 0,
            Self::LoadAssertionError => 0,
            Self::LoadLocals => 0,
            Self::ReturnGenerator => 0,
            Self::StoreSlice => 0,
            Self::DictMerge { .. } => 0,
            Self::BuildConstKeyMap { .. } => 0,
            Self::CopyFreeVars { .. } => 0,
            Self::EnterExecutor => 0,
            Self::JumpBackwardNoInterrupt { .. } => 0,
            Self::JumpBackward { .. } => 0,
            Self::JumpForward { .. } => 0,
            Self::ListExtend { .. } => 0,
            Self::LoadFastCheck(_) => 0,
            Self::LoadFastLoadFast { .. } => 0,
            Self::LoadFromDictOrGlobals(_) => 0,
            Self::SetUpdate { .. } => 0,
            Self::MakeCell(_) => 0,
            Self::StoreFastStoreFast { .. } => 0,
            Self::PopJumpIfNone { .. } => 0,
            Self::PopJumpIfNotNone { .. } => 0,
        }
    }

    pub fn display<'a>(
        &'a self,
        arg: OpArg,
        ctx: &'a impl InstrDisplayContext,
    ) -> impl fmt::Display + 'a {
        struct FmtFn<F>(F);
        impl<F: Fn(&mut fmt::Formatter<'_>) -> fmt::Result> fmt::Display for FmtFn<F> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                (self.0)(f)
            }
        }
        FmtFn(move |f: &mut fmt::Formatter<'_>| self.fmt_dis(arg, f, ctx, false, 0, 0))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fmt_dis(
        &self,
        arg: OpArg,
        f: &mut fmt::Formatter<'_>,
        ctx: &impl InstrDisplayContext,
        expand_code_objects: bool,
        pad: usize,
        level: usize,
    ) -> fmt::Result {
        macro_rules! w {
            ($variant:ident) => {
                write!(f, stringify!($variant))
            };
            ($variant:ident, $map:ident = $arg_marker:expr) => {{
                let arg = $arg_marker.get(arg);
                write!(f, "{:pad$}({}, {})", stringify!($variant), arg, $map(arg))
            }};
            ($variant:ident, $arg_marker:expr) => {
                write!(f, "{:pad$}({})", stringify!($variant), $arg_marker.get(arg))
            };
            ($variant:ident, ?$arg_marker:expr) => {
                write!(
                    f,
                    "{:pad$}({:?})",
                    stringify!($variant),
                    $arg_marker.get(arg)
                )
            };
        }

        let varname = |i: u32| ctx.get_varname(i as usize);
        let name = |i: u32| ctx.get_name(i as usize);
        let cell_name = |i: u32| ctx.get_cell_name(i as usize);

        let fmt_const =
            |op: &str, arg: OpArg, f: &mut fmt::Formatter<'_>, idx: &Arg<u32>| -> fmt::Result {
                let value = ctx.get_constant(idx.get(arg) as usize);
                match value.borrow_constant() {
                    BorrowedConstant::Code { code } if expand_code_objects => {
                        write!(f, "{op:pad$}({code:?}):")?;
                        code.display_inner(f, true, level + 1)?;
                        Ok(())
                    }
                    c => {
                        write!(f, "{op:pad$}(")?;
                        c.fmt_display(f)?;
                        write!(f, ")")
                    }
                }
            };

        match self {
            Self::BeforeAsyncWith => w!(BEFORE_ASYNC_WITH),
            Self::BeforeWith => w!(BEFORE_WITH),
            Self::BinaryOp { op } => write!(f, "{:pad$}({})", "BINARY_OP", op.get(arg)),
            Self::BinarySubscr => w!(BINARY_SUBSCR),
            Self::Break { target } => w!(BREAK, target),
            Self::BuildList { size } => w!(BUILD_LIST, size),
            Self::BuildListFromTuples { size } => w!(BUILD_LIST_FROM_TUPLES, size),
            Self::BuildMap { size } => w!(BUILD_MAP, size),
            Self::BuildMapForCall { size } => w!(BUILD_MAP_FOR_CALL, size),
            Self::BuildSet { size } => w!(BUILD_SET, size),
            Self::BuildSetFromTuples { size } => w!(BUILD_SET_FROM_TUPLES, size),
            Self::BuildSlice { argc } => w!(BUILD_SLICE, ?argc),
            Self::BuildString { size } => w!(BUILD_STRING, size),
            Self::BuildTuple { size } => w!(BUILD_TUPLE, size),
            Self::BuildTupleFromIter => w!(BUILD_TUPLE_FROM_ITER),
            Self::BuildTupleFromTuples { size } => w!(BUILD_TUPLE_FROM_TUPLES, size),
            Self::Call { nargs } => w!(CALL, nargs),
            Self::CallFunctionEx { has_kwargs } => w!(CALL_FUNCTION_EX, has_kwargs),
            Self::CallKw { nargs } => w!(CALL_KW, nargs),
            Self::CallIntrinsic1 { func } => w!(CALL_INTRINSIC_1, ?func),
            Self::CallIntrinsic2 { func } => w!(CALL_INTRINSIC_2, ?func),
            Self::CheckEgMatch => w!(CHECK_EG_MATCH),
            Self::CheckExcMatch => w!(CHECK_EXC_MATCH),
            Self::CleanupThrow => w!(CLEANUP_THROW),
            Self::CompareOp { op } => w!(COMPARE_OP, ?op),
            Self::ContainsOp(inv) => w!(CONTAINS_OP, ?inv),
            Self::Continue { target } => w!(CONTINUE, target),
            Self::ConvertValue { oparg } => write!(f, "{:pad$}{}", "CONVERT_VALUE", oparg.get(arg)),
            Self::Copy { index } => w!(COPY, index),
            Self::DeleteAttr { idx } => w!(DELETE_ATTR, name = idx),
            Self::DeleteDeref(idx) => w!(DELETE_DEREF, cell_name = idx),
            Self::DeleteFast(idx) => w!(DELETE_FAST, varname = idx),
            Self::DeleteGlobal(idx) => w!(DELETE_GLOBAL, name = idx),
            Self::DeleteName(idx) => w!(DELETE_NAME, name = idx),
            Self::DeleteSubscr => w!(DELETE_SUBSCR),
            Self::DictUpdate { index } => w!(DICT_UPDATE, index),
            Self::EndAsyncFor => w!(END_ASYNC_FOR),
            Self::EndSend => w!(END_SEND),
            Self::ExtendedArg => w!(EXTENDED_ARG, Arg::<u32>::marker()),
            Self::ForIter { target } => w!(FOR_ITER, target),
            Self::FormatSimple => w!(FORMAT_SIMPLE),
            Self::FormatWithSpec => w!(FORMAT_WITH_SPEC),
            Self::GetAIter => w!(GET_AITER),
            Self::GetANext => w!(GET_ANEXT),
            Self::GetAwaitable => w!(GET_AWAITABLE),
            Self::Reserved => w!(RESERVED),
            Self::GetIter => w!(GET_ITER),
            Self::GetLen => w!(GET_LEN),
            Self::ImportFrom { idx } => w!(IMPORT_FROM, name = idx),
            Self::ImportName { idx } => w!(IMPORT_NAME, name = idx),
            Self::IsOp(inv) => w!(IS_OP, ?inv),
            Self::Jump { target } => w!(JUMP, target),
            Self::JumpBackward { target } => w!(JUMP_BACKWARD, target),
            Self::JumpBackwardNoInterrupt { target } => w!(JUMP_BACKWARD_NO_INTERRUPT, target),
            Self::JumpForward { target } => w!(JUMP_FORWARD, target),
            Self::JumpIfFalseOrPop { target } => w!(JUMP_IF_FALSE_OR_POP, target),
            Self::JumpIfNotExcMatch(target) => w!(JUMP_IF_NOT_EXC_MATCH, target),
            Self::JumpIfTrueOrPop { target } => w!(JUMP_IF_TRUE_OR_POP, target),
            Self::ListAppend { i } => w!(LIST_APPEND, i),
            Self::LoadAttr { idx } => {
                let encoded = idx.get(arg);
                let (name_idx, is_method) = decode_load_attr_arg(encoded);
                let attr_name = name(name_idx);
                if is_method {
                    write!(
                        f,
                        "{:pad$}({}, {}, method=true)",
                        "LOAD_ATTR", encoded, attr_name
                    )
                } else {
                    write!(f, "{:pad$}({}, {})", "LOAD_ATTR", encoded, attr_name)
                }
            }
            Self::LoadAttrMethod { idx } => w!(LOAD_ATTR_METHOD, name = idx),
            Self::LoadBuildClass => w!(LOAD_BUILD_CLASS),
            Self::LoadFromDictOrDeref(i) => w!(LOAD_FROM_DICT_OR_DEREF, cell_name = i),
            Self::LoadClosure(i) => w!(LOAD_CLOSURE, cell_name = i),
            Self::LoadConst { idx } => fmt_const("LOAD_CONST", arg, f, idx),
            Self::LoadDeref(idx) => w!(LOAD_DEREF, cell_name = idx),
            Self::LoadFast(idx) => w!(LOAD_FAST, varname = idx),
            Self::LoadFastAndClear(idx) => w!(LOAD_FAST_AND_CLEAR, varname = idx),
            Self::LoadGlobal(idx) => w!(LOAD_GLOBAL, name = idx),
            Self::LoadName(idx) => w!(LOAD_NAME, name = idx),
            Self::LoadSuperAttr { arg: idx } => {
                let encoded = idx.get(arg);
                let (name_idx, load_method, has_class) = decode_load_super_attr_arg(encoded);
                let attr_name = name(name_idx);
                write!(
                    f,
                    "{:pad$}({}, {}, method={}, class={})",
                    "LOAD_SUPER_ATTR", encoded, attr_name, load_method, has_class
                )
            }
            Self::LoadSuperMethod { idx } => w!(LOAD_SUPER_METHOD, name = idx),
            Self::LoadZeroSuperAttr { idx } => w!(LOAD_ZERO_SUPER_ATTR, name = idx),
            Self::LoadZeroSuperMethod { idx } => w!(LOAD_ZERO_SUPER_METHOD, name = idx),
            Self::MakeFunction => w!(MAKE_FUNCTION),
            Self::MapAdd { i } => w!(MAP_ADD, i),
            Self::MatchClass(arg) => w!(MATCH_CLASS, arg),
            Self::MatchKeys => w!(MATCH_KEYS),
            Self::MatchMapping => w!(MATCH_MAPPING),
            Self::MatchSequence => w!(MATCH_SEQUENCE),
            Self::Nop => w!(NOP),
            Self::PopBlock => w!(POP_BLOCK),
            Self::PopExcept => w!(POP_EXCEPT),
            Self::PopJumpIfFalse { target } => w!(POP_JUMP_IF_FALSE, target),
            Self::PopJumpIfTrue { target } => w!(POP_JUMP_IF_TRUE, target),
            Self::PopTop => w!(POP_TOP),
            Self::PushExcInfo => w!(PUSH_EXC_INFO),
            Self::PushNull => w!(PUSH_NULL),
            Self::RaiseVarargs { kind } => w!(RAISE_VARARGS, ?kind),
            Self::Reraise { depth } => w!(RERAISE, depth),
            Self::Resume { arg } => w!(RESUME, arg),
            Self::ReturnConst { idx } => fmt_const("RETURN_CONST", arg, f, idx),
            Self::ReturnValue => w!(RETURN_VALUE),
            Self::Send { target } => w!(SEND, target),
            Self::SetAdd { i } => w!(SET_ADD, i),
            Self::SetExcInfo => w!(SET_EXC_INFO),
            Self::SetFunctionAttribute { attr } => w!(SET_FUNCTION_ATTRIBUTE, ?attr),
            Self::SetupAnnotations => w!(SETUP_ANNOTATIONS),
            Self::StoreAttr { idx } => w!(STORE_ATTR, name = idx),
            Self::StoreDeref(idx) => w!(STORE_DEREF, cell_name = idx),
            Self::StoreFast(idx) => w!(STORE_FAST, varname = idx),
            Self::StoreFastLoadFast {
                store_idx,
                load_idx,
            } => {
                write!(f, "STORE_FAST_LOAD_FAST")?;
                write!(f, " ({}, {})", store_idx.get(arg), load_idx.get(arg))
            }
            Self::StoreGlobal(idx) => w!(STORE_GLOBAL, name = idx),
            Self::StoreName(idx) => w!(STORE_NAME, name = idx),
            Self::StoreSubscr => w!(STORE_SUBSCR),
            Self::Subscript => w!(SUBSCRIPT),
            Self::Swap { index } => w!(SWAP, index),
            Self::ToBool => w!(TO_BOOL),
            Self::UnpackEx { args } => w!(UNPACK_EX, args),
            Self::UnpackSequence { size } => w!(UNPACK_SEQUENCE, size),
            Self::WithExceptStart => w!(WITH_EXCEPT_START),
            Self::UnaryInvert => w!(UNARY_INVERT),
            Self::UnaryNegative => w!(UNARY_NEGATIVE),
            Self::UnaryNot => w!(UNARY_NOT),
            Self::YieldValue { arg } => w!(YIELD_VALUE, arg),
            Self::GetYieldFromIter => w!(GET_YIELD_FROM_ITER),
            _ => w!(RUSTPYTHON_PLACEHOLDER),
        }
    }
}

#[derive(Copy, Clone)]
pub struct Arg<T: OpArgType>(PhantomData<T>);

impl<T: OpArgType> Arg<T> {
    #[inline]
    pub const fn marker() -> Self {
        Self(PhantomData)
    }

    #[inline]
    pub fn new(arg: T) -> (Self, OpArg) {
        (Self(PhantomData), OpArg(arg.to_op_arg()))
    }

    #[inline]
    pub fn new_single(arg: T) -> (Self, OpArgByte)
    where
        T: Into<u8>,
    {
        (Self(PhantomData), OpArgByte(arg.into()))
    }

    #[inline(always)]
    pub fn get(self, arg: OpArg) -> T {
        self.try_get(arg).unwrap()
    }

    #[inline(always)]
    pub fn try_get(self, arg: OpArg) -> Option<T> {
        T::from_op_arg(arg.0)
    }

    /// # Safety
    /// T::from_op_arg(self) must succeed
    #[inline(always)]
    pub unsafe fn get_unchecked(self, arg: OpArg) -> T {
        // SAFETY: requirements forwarded from caller
        unsafe { T::from_op_arg(arg.0).unwrap_unchecked() }
    }
}

impl<T: OpArgType> PartialEq for Arg<T> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<T: OpArgType> Eq for Arg<T> {}

impl<T: OpArgType> fmt::Debug for Arg<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Arg<{}>", core::any::type_name::<T>())
    }
}

/// Encode LOAD_SUPER_ATTR oparg: bit 0 = load_method, bit 1 = has_class, bits 2+ = name index.
#[inline]
pub const fn encode_load_super_attr_arg(name_idx: u32, load_method: bool, has_class: bool) -> u32 {
    (name_idx << 2) | ((has_class as u32) << 1) | (load_method as u32)
}

/// Decode LOAD_SUPER_ATTR oparg: returns (name_idx, load_method, has_class).
#[inline]
pub const fn decode_load_super_attr_arg(oparg: u32) -> (u32, bool, bool) {
    let load_method = (oparg & 1) == 1;
    let has_class = (oparg & 2) == 2;
    let name_idx = oparg >> 2;
    (name_idx, load_method, has_class)
}
