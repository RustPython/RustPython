use core::{fmt, marker::PhantomData, mem};

use crate::{
    bytecode::{
        BorrowedConstant, Constant, InstrDisplayContext,
        oparg::{
            BinaryOperator, BuildSliceArgCount, CommonConstant, ComparisonOperator,
            ConvertValueOparg, IntrinsicFunction1, IntrinsicFunction2, Invert, Label,
            MakeFunctionFlags, NameIdx, OpArg, OpArgByte, OpArgType, RaiseKind, SpecialMethod,
            UnpackExArgs,
        },
    },
    marshal::MarshalError,
};

/// A Single bytecode instruction that are executed by the VM.
///
/// Currently aligned with CPython 3.14.
///
/// ## See also
/// - [CPython opcode IDs](https://github.com/python/cpython/blob/v3.14.2/Include/opcode_ids.h)
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum Instruction {
    // No-argument instructions (opcode < HAVE_ARGUMENT=44)
    Cache = 0, // Placeholder
    BinarySlice = 1,
    BuildTemplate = 2,
    BinaryOpInplaceAddUnicode = 3, // Placeholder
    CallFunctionEx = 4,
    CheckEgMatch = 5,
    CheckExcMatch = 6,
    CleanupThrow = 7,
    DeleteSubscr = 8,
    EndFor = 9,
    EndSend = 10,
    ExitInitCheck = 11, // Placeholder
    FormatSimple = 12,
    FormatWithSpec = 13,
    GetAIter = 14,
    GetANext = 15,
    GetIter = 16,
    Reserved = 17,
    GetLen = 18,
    GetYieldFromIter = 19,
    InterpreterExit = 20, // Placeholder
    LoadBuildClass = 21,
    LoadLocals = 22,
    MakeFunction = 23,
    MatchKeys = 24,
    MatchMapping = 25,
    MatchSequence = 26,
    Nop = 27,
    NotTaken = 28, // Placeholder
    PopExcept = 29,
    PopIter = 30,
    PopTop = 31,
    PushExcInfo = 32,
    PushNull = 33,
    ReturnGenerator = 34,
    ReturnValue = 35,
    SetupAnnotations = 36,
    StoreSlice = 37,
    StoreSubscr = 38,
    ToBool = 39,
    UnaryInvert = 40,
    UnaryNegative = 41,
    UnaryNot = 42,
    WithExceptStart = 43,
    // CPython 3.14 opcodes with arguments (44-120)
    BinaryOp {
        op: Arg<BinaryOperator>,
    } = 44,
    /// Build an Interpolation from value, expression string, and optional format_spec on stack.
    ///
    /// oparg encoding: (conversion << 2) | has_format_spec
    /// - has_format_spec (bit 0): if 1, format_spec is on stack
    /// - conversion (bits 2+): 0=None, 1=Str, 2=Repr, 3=Ascii
    ///
    /// Stack: [value, expression_str, format_spec?] -> [interpolation]
    BuildInterpolation {
        oparg: Arg<u32>,
    } = 45,
    BuildList {
        size: Arg<u32>,
    } = 46,
    BuildMap {
        size: Arg<u32>,
    } = 47,
    BuildSet {
        size: Arg<u32>,
    } = 48,
    BuildSlice {
        argc: Arg<BuildSliceArgCount>,
    } = 49,
    BuildString {
        size: Arg<u32>,
    } = 50,
    BuildTuple {
        size: Arg<u32>,
    } = 51,
    Call {
        nargs: Arg<u32>,
    } = 52,
    CallIntrinsic1 {
        func: Arg<IntrinsicFunction1>,
    } = 53,
    CallIntrinsic2 {
        func: Arg<IntrinsicFunction2>,
    } = 54,
    CallKw {
        nargs: Arg<u32>,
    } = 55,
    CompareOp {
        op: Arg<ComparisonOperator>,
    } = 56,
    ContainsOp(Arg<Invert>) = 57,
    ConvertValue {
        oparg: Arg<ConvertValueOparg>,
    } = 58,
    Copy {
        index: Arg<u32>,
    } = 59,
    CopyFreeVars {
        count: Arg<u32>,
    } = 60,
    DeleteAttr {
        idx: Arg<NameIdx>,
    } = 61,
    DeleteDeref(Arg<NameIdx>) = 62,
    DeleteFast(Arg<NameIdx>) = 63,
    DeleteGlobal(Arg<NameIdx>) = 64,
    DeleteName(Arg<NameIdx>) = 65,
    DictMerge {
        index: Arg<u32>,
    } = 66,
    DictUpdate {
        index: Arg<u32>,
    } = 67,
    EndAsyncFor = 68,
    ExtendedArg = 69,
    ForIter {
        target: Arg<Label>,
    } = 70,
    GetAwaitable = 71, // TODO: Make this instruction to hold an oparg
    ImportFrom {
        idx: Arg<NameIdx>,
    } = 72,
    ImportName {
        idx: Arg<NameIdx>,
    } = 73,
    IsOp(Arg<Invert>) = 74,
    JumpBackward {
        target: Arg<Label>,
    } = 75,
    JumpBackwardNoInterrupt {
        target: Arg<Label>,
    } = 76, // Placeholder
    JumpForward {
        target: Arg<Label>,
    } = 77,
    ListAppend {
        i: Arg<u32>,
    } = 78,
    ListExtend {
        i: Arg<u32>,
    } = 79,
    LoadAttr {
        idx: Arg<NameIdx>,
    } = 80,
    LoadCommonConstant {
        idx: Arg<CommonConstant>,
    } = 81,
    LoadConst {
        idx: Arg<u32>,
    } = 82,
    LoadDeref(Arg<NameIdx>) = 83,
    LoadFast(Arg<NameIdx>) = 84,
    LoadFastAndClear(Arg<NameIdx>) = 85,
    LoadFastBorrow(Arg<NameIdx>) = 86, // Placeholder
    LoadFastBorrowLoadFastBorrow {
        arg: Arg<u32>,
    } = 87, // Placeholder
    LoadFastCheck(Arg<NameIdx>) = 88,
    LoadFastLoadFast {
        arg: Arg<u32>,
    } = 89,
    LoadFromDictOrDeref(Arg<NameIdx>) = 90,
    LoadFromDictOrGlobals(Arg<NameIdx>) = 91,
    LoadGlobal(Arg<NameIdx>) = 92,
    LoadName(Arg<NameIdx>) = 93,
    LoadSmallInt {
        idx: Arg<u32>,
    } = 94,
    LoadSpecial {
        method: Arg<SpecialMethod>,
    } = 95,
    LoadSuperAttr {
        arg: Arg<u32>,
    } = 96,
    MakeCell(Arg<NameIdx>) = 97,
    MapAdd {
        i: Arg<u32>,
    } = 98,
    MatchClass(Arg<u32>) = 99,
    PopJumpIfFalse {
        target: Arg<Label>,
    } = 100,
    PopJumpIfNone {
        target: Arg<Label>,
    } = 101,
    PopJumpIfNotNone {
        target: Arg<Label>,
    } = 102,
    PopJumpIfTrue {
        target: Arg<Label>,
    } = 103,
    RaiseVarargs {
        kind: Arg<RaiseKind>,
    } = 104,
    Reraise {
        depth: Arg<u32>,
    } = 105,
    Send {
        target: Arg<Label>,
    } = 106,
    SetAdd {
        i: Arg<u32>,
    } = 107,
    SetFunctionAttribute {
        attr: Arg<MakeFunctionFlags>,
    } = 108,
    SetUpdate {
        i: Arg<u32>,
    } = 109,
    StoreAttr {
        idx: Arg<NameIdx>,
    } = 110,
    StoreDeref(Arg<NameIdx>) = 111,
    StoreFast(Arg<NameIdx>) = 112,
    StoreFastLoadFast {
        store_idx: Arg<NameIdx>,
        load_idx: Arg<NameIdx>,
    } = 113,
    StoreFastStoreFast {
        arg: Arg<u32>,
    } = 114,
    StoreGlobal(Arg<NameIdx>) = 115,
    StoreName(Arg<NameIdx>) = 116,
    Swap {
        index: Arg<u32>,
    } = 117,
    UnpackEx {
        args: Arg<UnpackExArgs>,
    } = 118,
    UnpackSequence {
        size: Arg<u32>,
    } = 119,
    YieldValue {
        arg: Arg<u32>,
    } = 120,
    // CPython 3.14 RESUME (128)
    Resume {
        arg: Arg<u32>,
    } = 128,
    // CPython 3.14 specialized opcodes (129-211)
    BinaryOpAddFloat = 129,                     // Placeholder
    BinaryOpAddInt = 130,                       // Placeholder
    BinaryOpAddUnicode = 131,                   // Placeholder
    BinaryOpExtend = 132,                       // Placeholder
    BinaryOpMultiplyFloat = 133,                // Placeholder
    BinaryOpMultiplyInt = 134,                  // Placeholder
    BinaryOpSubscrDict = 135,                   // Placeholder
    BinaryOpSubscrGetitem = 136,                // Placeholder
    BinaryOpSubscrListInt = 137,                // Placeholder
    BinaryOpSubscrListSlice = 138,              // Placeholder
    BinaryOpSubscrStrInt = 139,                 // Placeholder
    BinaryOpSubscrTupleInt = 140,               // Placeholder
    BinaryOpSubtractFloat = 141,                // Placeholder
    BinaryOpSubtractInt = 142,                  // Placeholder
    CallAllocAndEnterInit = 143,                // Placeholder
    CallBoundMethodExactArgs = 144,             // Placeholder
    CallBoundMethodGeneral = 145,               // Placeholder
    CallBuiltinClass = 146,                     // Placeholder
    CallBuiltinFast = 147,                      // Placeholder
    CallBuiltinFastWithKeywords = 148,          // Placeholder
    CallBuiltinO = 149,                         // Placeholder
    CallIsinstance = 150,                       // Placeholder
    CallKwBoundMethod = 151,                    // Placeholder
    CallKwNonPy = 152,                          // Placeholder
    CallKwPy = 153,                             // Placeholder
    CallLen = 154,                              // Placeholder
    CallListAppend = 155,                       // Placeholder
    CallMethodDescriptorFast = 156,             // Placeholder
    CallMethodDescriptorFastWithKeywords = 157, // Placeholder
    CallMethodDescriptorNoargs = 158,           // Placeholder
    CallMethodDescriptorO = 159,                // Placeholder
    CallNonPyGeneral = 160,                     // Placeholder
    CallPyExactArgs = 161,                      // Placeholder
    CallPyGeneral = 162,                        // Placeholder
    CallStr1 = 163,                             // Placeholder
    CallTuple1 = 164,                           // Placeholder
    CallType1 = 165,                            // Placeholder
    CompareOpFloat = 166,                       // Placeholder
    CompareOpInt = 167,                         // Placeholder
    CompareOpStr = 168,                         // Placeholder
    ContainsOpDict = 169,                       // Placeholder
    ContainsOpSet = 170,                        // Placeholder
    ForIterGen = 171,                           // Placeholder
    ForIterList = 172,                          // Placeholder
    ForIterRange = 173,                         // Placeholder
    ForIterTuple = 174,                         // Placeholder
    JumpBackwardJit = 175,                      // Placeholder
    JumpBackwardNoJit = 176,                    // Placeholder
    LoadAttrClass = 177,                        // Placeholder
    LoadAttrClassWithMetaclassCheck = 178,      // Placeholder
    LoadAttrGetattributeOverridden = 179,       // Placeholder
    LoadAttrInstanceValue = 180,                // Placeholder
    LoadAttrMethodLazyDict = 181,               // Placeholder
    LoadAttrMethodNoDict = 182,                 // Placeholder
    LoadAttrMethodWithValues = 183,             // Placeholder
    LoadAttrModule = 184,                       // Placeholder
    LoadAttrNondescriptorNoDict = 185,          // Placeholder
    LoadAttrNondescriptorWithValues = 186,      // Placeholder
    LoadAttrProperty = 187,                     // Placeholder
    LoadAttrSlot = 188,                         // Placeholder
    LoadAttrWithHint = 189,                     // Placeholder
    LoadConstImmortal = 190,                    // Placeholder
    LoadConstMortal = 191,                      // Placeholder
    LoadGlobalBuiltin = 192,                    // Placeholder
    LoadGlobalModule = 193,                     // Placeholder
    LoadSuperAttrAttr = 194,                    // Placeholder
    LoadSuperAttrMethod = 195,                  // Placeholder
    ResumeCheck = 196,                          // Placeholder
    SendGen = 197,                              // Placeholder
    StoreAttrInstanceValue = 198,               // Placeholder
    StoreAttrSlot = 199,                        // Placeholder
    StoreAttrWithHint = 200,                    // Placeholder
    StoreSubscrDict = 201,                      // Placeholder
    StoreSubscrListInt = 202,                   // Placeholder
    ToBoolAlwaysTrue = 203,                     // Placeholder
    ToBoolBool = 204,                           // Placeholder
    ToBoolInt = 205,                            // Placeholder
    ToBoolList = 206,                           // Placeholder
    ToBoolNone = 207,                           // Placeholder
    ToBoolStr = 208,                            // Placeholder
    UnpackSequenceList = 209,                   // Placeholder
    UnpackSequenceTuple = 210,                  // Placeholder
    UnpackSequenceTwoTuple = 211,               // Placeholder
    // CPython 3.14 instrumented opcodes (234-254)
    InstrumentedEndFor = 234,           // Placeholder
    InstrumentedPopIter = 235,          // Placeholder
    InstrumentedEndSend = 236,          // Placeholder
    InstrumentedForIter = 237,          // Placeholder
    InstrumentedInstruction = 238,      // Placeholder
    InstrumentedJumpForward = 239,      // Placeholder
    InstrumentedNotTaken = 240,         // Placeholder
    InstrumentedPopJumpIfTrue = 241,    // Placeholder
    InstrumentedPopJumpIfFalse = 242,   // Placeholder
    InstrumentedPopJumpIfNone = 243,    // Placeholder
    InstrumentedPopJumpIfNotNone = 244, // Placeholder
    InstrumentedResume = 245,           // Placeholder
    InstrumentedReturnValue = 246,      // Placeholder
    InstrumentedYieldValue = 247,       // Placeholder
    InstrumentedEndAsyncFor = 248,      // Placeholder
    InstrumentedLoadSuperAttr = 249,    // Placeholder
    InstrumentedCall = 250,             // Placeholder
    InstrumentedCallKw = 251,           // Placeholder
    InstrumentedCallFunctionEx = 252,   // Placeholder
    InstrumentedJumpBackward = 253,     // Placeholder
    InstrumentedLine = 254,             // Placeholder
    EnterExecutor = 255,                // Placeholder
}

const _: () = assert!(mem::size_of::<Instruction>() == 1);

impl From<Instruction> for u8 {
    #[inline]
    fn from(ins: Instruction) -> Self {
        // SAFETY: there's no padding bits
        unsafe { mem::transmute::<Instruction, Self>(ins) }
    }
}

impl TryFrom<u8> for Instruction {
    type Error = MarshalError;

    #[inline]
    fn try_from(value: u8) -> Result<Self, MarshalError> {
        // CPython-compatible opcodes (0-120)
        let cpython_start = u8::from(Self::Cache);
        let cpython_end = u8::from(Self::YieldValue { arg: Arg::marker() });

        // Resume has a non-contiguous opcode (128)
        let resume_id = u8::from(Self::Resume { arg: Arg::marker() });
        let enter_executor_id = u8::from(Self::EnterExecutor);

        let specialized_start = u8::from(Self::BinaryOpAddFloat);
        let specialized_end = u8::from(Self::UnpackSequenceTwoTuple);

        let instrumented_start = u8::from(Self::InstrumentedEndFor);
        let instrumented_end = u8::from(Self::InstrumentedLine);

        // No RustPython-only opcodes anymore - all opcodes match CPython 3.14
        let custom_ops: &[u8] = &[];

        if (cpython_start..=cpython_end).contains(&value)
            || value == resume_id
            || value == enter_executor_id
            || custom_ops.contains(&value)
            || (specialized_start..=specialized_end).contains(&value)
            || (instrumented_start..=instrumented_end).contains(&value)
        {
            Ok(unsafe { mem::transmute::<u8, Self>(value) })
        } else {
            Err(Self::Error::InvalidBytecode)
        }
    }
}

impl InstructionMetadata for Instruction {
    #[inline]
    fn label_arg(&self) -> Option<Arg<Label>> {
        match self {
            Self::JumpBackward { target: l }
            | Self::JumpBackwardNoInterrupt { target: l }
            | Self::JumpForward { target: l }
            | Self::PopJumpIfTrue { target: l }
            | Self::PopJumpIfFalse { target: l }
            | Self::PopJumpIfNone { target: l }
            | Self::PopJumpIfNotNone { target: l }
            | Self::ForIter { target: l }
            | Self::Send { target: l } => Some(*l),
            _ => None,
        }
    }

    fn is_unconditional_jump(&self) -> bool {
        matches!(
            self,
            Self::JumpForward { .. }
                | Self::JumpBackward { .. }
                | Self::JumpBackwardNoInterrupt { .. }
        )
    }

    fn is_scope_exit(&self) -> bool {
        matches!(
            self,
            Self::ReturnValue | Self::RaiseVarargs { .. } | Self::Reraise { .. }
        )
    }

    fn stack_effect(&self, arg: OpArg) -> i32 {
        match self {
            Self::Nop => 0,
            Self::NotTaken => 0,
            Self::ImportName { .. } => -1,
            Self::ImportFrom { .. } => 1,
            Self::LoadFast(_) => 1,
            Self::LoadFastBorrow(_) => 1,
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
            Self::LoadFromDictOrDeref(_) => 0, // (dict -- value)
            Self::StoreSubscr => -3,
            Self::DeleteSubscr => -2,
            Self::LoadAttr { idx } => {
                // Stack effect depends on method flag in encoded oparg
                // method=false: pop obj, push attr → effect = 0
                // method=true: pop obj, push (method, self_or_null) → effect = +1
                let (_, is_method) = decode_load_attr_arg(idx.get(arg));
                if is_method { 1 } else { 0 }
            }
            Self::StoreAttr { .. } => -2,
            Self::DeleteAttr { .. } => -1,
            Self::LoadCommonConstant { .. } => 1,
            Self::LoadConst { .. } => 1,
            Self::LoadSmallInt { .. } => 1,
            Self::LoadSpecial { .. } => 0,
            Self::Reserved => 0,
            Self::BinaryOp { .. } => -1,
            Self::CompareOp { .. } => -1,
            Self::Copy { .. } => 1,
            Self::PopTop => -1,
            Self::Swap { .. } => 0,
            Self::ToBool => 0,
            Self::GetIter => 0,
            Self::GetLen => 1,
            Self::CallIntrinsic1 { .. } => 0,  // Takes 1, pushes 1
            Self::CallIntrinsic2 { .. } => -1, // Takes 2, pushes 1
            Self::PopJumpIfTrue { .. } => -1,
            Self::PopJumpIfFalse { .. } => -1,
            Self::MakeFunction => {
                // CPython 3.14 style: MakeFunction only pops code object
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
            // CallFunctionEx: always pops kwargs_or_null + args_tuple + self_or_null + callable, pushes result
            Self::CallFunctionEx => -4 + 1,
            Self::CheckEgMatch => 0, // pops 2 (exc, type), pushes 2 (rest, match)
            Self::ConvertValue { .. } => 0,
            Self::FormatSimple => 0,
            Self::FormatWithSpec => -1,
            Self::ForIter { .. } => 1, // push next value
            Self::IsOp(_) => -1,
            Self::ContainsOp(_) => -1,
            Self::ReturnValue => -1,
            Self::Resume { .. } => 0,
            Self::YieldValue { .. } => 0,
            // SEND: (receiver, val) -> (receiver, retval) - no change, both paths keep same depth
            Self::Send { .. } => 0,
            // END_SEND: (receiver, value) -> (value)
            Self::EndSend => -1,
            // CLEANUP_THROW: (sub_iter, last_sent_val, exc) -> (None, value) = 3 pop, 2 push = -1
            Self::CleanupThrow => -1,
            Self::PushExcInfo => 1,    // [exc] -> [prev_exc, exc]
            Self::CheckExcMatch => 0,  // [exc, type] -> [exc, bool] (pops type, pushes bool)
            Self::Reraise { .. } => 0, // Exception raised, stack effect doesn't matter
            Self::SetupAnnotations => 0,
            Self::WithExceptStart => 1, // push __exit__ result
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
            Self::BuildList { size, .. } => -(size.get(arg) as i32) + 1,
            Self::BuildSet { size, .. } => -(size.get(arg) as i32) + 1,
            Self::BuildMap { size } => {
                let nargs = size.get(arg) * 2;
                -(nargs as i32) + 1
            }
            Self::DictUpdate { .. } => -1,
            Self::DictMerge { .. } => -1,
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
            Self::ListExtend { .. } => -1,
            Self::SetAdd { .. } => -1,
            Self::SetUpdate { .. } => -1,
            Self::MapAdd { .. } => -2,
            Self::LoadBuildClass => 1,
            Self::UnpackSequence { size } => -1 + size.get(arg) as i32,
            Self::UnpackEx { args } => {
                let UnpackExArgs { before, after } = args.get(arg);
                -1 + before as i32 + 1 + after as i32
            }
            Self::PopExcept => -1,
            Self::PopIter => -1,
            Self::GetAwaitable => 0,
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
            Self::Cache => 0,
            Self::BinarySlice => -2, // (container, start, stop -- res)
            Self::BinaryOpInplaceAddUnicode => 0,
            Self::EndFor => -1,        // pop next value at end of loop iteration
            Self::ExitInitCheck => -1, // (should_be_none -- )
            Self::InterpreterExit => 0,
            Self::LoadLocals => 1,      // ( -- locals)
            Self::ReturnGenerator => 1, // pushes None for POP_TOP to consume
            Self::StoreSlice => -4,     // (v, container, start, stop -- )
            Self::CopyFreeVars { .. } => 0,
            Self::EnterExecutor => 0,
            Self::JumpBackwardNoInterrupt { .. } => 0,
            Self::JumpBackward { .. } => 0,
            Self::JumpForward { .. } => 0,
            Self::LoadFastCheck(_) => 1,
            Self::LoadFastLoadFast { .. } => 2,
            Self::LoadFastBorrowLoadFastBorrow { .. } => 2,
            Self::LoadFromDictOrGlobals(_) => 0,
            Self::MakeCell(_) => 0,
            Self::StoreFastStoreFast { .. } => -2, // pops 2 values
            Self::PopJumpIfNone { .. } => -1,      // (value -- )
            Self::PopJumpIfNotNone { .. } => -1,   // (value -- )
            Self::BinaryOpAddFloat => 0,
            Self::BinaryOpAddInt => 0,
            Self::BinaryOpAddUnicode => 0,
            Self::BinaryOpExtend => 0,
            Self::BinaryOpMultiplyFloat => 0,
            Self::BinaryOpMultiplyInt => 0,
            Self::BinaryOpSubtractFloat => 0,
            Self::BinaryOpSubtractInt => 0,
            Self::BinaryOpSubscrDict => 0,
            Self::BinaryOpSubscrGetitem => 0,
            Self::BinaryOpSubscrListInt => 0,
            Self::BinaryOpSubscrListSlice => 0,
            Self::BinaryOpSubscrStrInt => 0,
            Self::BinaryOpSubscrTupleInt => 0,
            Self::CallAllocAndEnterInit => 0,
            Self::CallBoundMethodExactArgs => 0,
            Self::CallBoundMethodGeneral => 0,
            Self::CallBuiltinClass => 0,
            Self::CallBuiltinFast => 0,
            Self::CallBuiltinFastWithKeywords => 0,
            Self::CallBuiltinO => 0,
            Self::CallIsinstance => 0,
            Self::CallKwBoundMethod => 0,
            Self::CallKwNonPy => 0,
            Self::CallKwPy => 0,
            Self::CallLen => 0,
            Self::CallListAppend => 0,
            Self::CallMethodDescriptorFast => 0,
            Self::CallMethodDescriptorFastWithKeywords => 0,
            Self::CallMethodDescriptorNoargs => 0,
            Self::CallMethodDescriptorO => 0,
            Self::CallNonPyGeneral => 0,
            Self::CallPyExactArgs => 0,
            Self::CallPyGeneral => 0,
            Self::CallStr1 => 0,
            Self::CallTuple1 => 0,
            Self::CallType1 => 0,
            Self::CompareOpFloat => 0,
            Self::CompareOpInt => 0,
            Self::CompareOpStr => 0,
            Self::ContainsOpDict => 0,
            Self::ContainsOpSet => 0,
            Self::ForIterGen => 0,
            Self::ForIterList => 0,
            Self::ForIterRange => 0,
            Self::ForIterTuple => 0,
            Self::JumpBackwardJit => 0,
            Self::JumpBackwardNoJit => 0,
            Self::LoadAttrClass => 0,
            Self::LoadAttrClassWithMetaclassCheck => 0,
            Self::LoadAttrGetattributeOverridden => 0,
            Self::LoadAttrInstanceValue => 0,
            Self::LoadAttrMethodLazyDict => 0,
            Self::LoadAttrMethodNoDict => 0,
            Self::LoadAttrMethodWithValues => 0,
            Self::LoadAttrModule => 0,
            Self::LoadAttrNondescriptorNoDict => 0,
            Self::LoadAttrNondescriptorWithValues => 0,
            Self::LoadAttrProperty => 0,
            Self::LoadAttrSlot => 0,
            Self::LoadAttrWithHint => 0,
            Self::LoadConstImmortal => 0,
            Self::LoadConstMortal => 0,
            Self::LoadGlobalBuiltin => 0,
            Self::LoadGlobalModule => 0,
            Self::LoadSuperAttrAttr => 0,
            Self::LoadSuperAttrMethod => 0,
            Self::ResumeCheck => 0,
            Self::SendGen => 0,
            Self::StoreAttrInstanceValue => 0,
            Self::StoreAttrSlot => 0,
            Self::StoreAttrWithHint => 0,
            Self::StoreSubscrDict => 0,
            Self::StoreSubscrListInt => 0,
            Self::ToBoolAlwaysTrue => 0,
            Self::ToBoolBool => 0,
            Self::ToBoolInt => 0,
            Self::ToBoolList => 0,
            Self::ToBoolNone => 0,
            Self::ToBoolStr => 0,
            Self::UnpackSequenceList => 0,
            Self::UnpackSequenceTuple => 0,
            Self::UnpackSequenceTwoTuple => 0,
            Self::InstrumentedEndFor => 0,
            Self::InstrumentedPopIter => -1,
            Self::InstrumentedEndSend => 0,
            Self::InstrumentedForIter => 0,
            Self::InstrumentedInstruction => 0,
            Self::InstrumentedJumpForward => 0,
            Self::InstrumentedNotTaken => 0,
            Self::InstrumentedJumpBackward => 0,
            Self::InstrumentedPopJumpIfTrue => 0,
            Self::InstrumentedPopJumpIfFalse => 0,
            Self::InstrumentedPopJumpIfNone => 0,
            Self::InstrumentedPopJumpIfNotNone => 0,
            Self::InstrumentedResume => 0,
            Self::InstrumentedReturnValue => 0,
            Self::InstrumentedYieldValue => 0,
            Self::InstrumentedEndAsyncFor => -2,
            Self::InstrumentedLoadSuperAttr => 0,
            Self::InstrumentedCall => 0,
            Self::InstrumentedCallKw => 0,
            Self::InstrumentedCallFunctionEx => 0,
            Self::InstrumentedLine => 0,
            // BuildTemplate: pops [strings_tuple, interpolations_tuple], pushes [template]
            Self::BuildTemplate => -1,
            // BuildInterpolation: pops [value, expr_str, format_spec?], pushes [interpolation]
            // has_format_spec is bit 0 of oparg
            Self::BuildInterpolation { oparg } => {
                let has_format_spec = oparg.get(arg) & 1 != 0;
                if has_format_spec { -2 } else { -1 }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn fmt_dis(
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
            Self::BinaryOp { op } => write!(f, "{:pad$}({})", "BINARY_OP", op.get(arg)),
            Self::BuildList { size } => w!(BUILD_LIST, size),
            Self::BuildMap { size } => w!(BUILD_MAP, size),
            Self::BuildSet { size } => w!(BUILD_SET, size),
            Self::BuildSlice { argc } => w!(BUILD_SLICE, ?argc),
            Self::BuildString { size } => w!(BUILD_STRING, size),
            Self::BuildTuple { size } => w!(BUILD_TUPLE, size),
            Self::Call { nargs } => w!(CALL, nargs),
            Self::CallFunctionEx => w!(CALL_FUNCTION_EX),
            Self::CallKw { nargs } => w!(CALL_KW, nargs),
            Self::CallIntrinsic1 { func } => w!(CALL_INTRINSIC_1, ?func),
            Self::CallIntrinsic2 { func } => w!(CALL_INTRINSIC_2, ?func),
            Self::CheckEgMatch => w!(CHECK_EG_MATCH),
            Self::CheckExcMatch => w!(CHECK_EXC_MATCH),
            Self::CleanupThrow => w!(CLEANUP_THROW),
            Self::CompareOp { op } => w!(COMPARE_OP, ?op),
            Self::ContainsOp(inv) => w!(CONTAINS_OP, ?inv),
            Self::ConvertValue { oparg } => write!(f, "{:pad$}{}", "CONVERT_VALUE", oparg.get(arg)),
            Self::Copy { index } => w!(COPY, index),
            Self::DeleteAttr { idx } => w!(DELETE_ATTR, name = idx),
            Self::DeleteDeref(idx) => w!(DELETE_DEREF, cell_name = idx),
            Self::DeleteFast(idx) => w!(DELETE_FAST, varname = idx),
            Self::DeleteGlobal(idx) => w!(DELETE_GLOBAL, name = idx),
            Self::DeleteName(idx) => w!(DELETE_NAME, name = idx),
            Self::DeleteSubscr => w!(DELETE_SUBSCR),
            Self::DictMerge { index } => w!(DICT_MERGE, index),
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
            Self::JumpBackward { target } => w!(JUMP_BACKWARD, target),
            Self::JumpBackwardNoInterrupt { target } => w!(JUMP_BACKWARD_NO_INTERRUPT, target),
            Self::JumpForward { target } => w!(JUMP_FORWARD, target),
            Self::ListAppend { i } => w!(LIST_APPEND, i),
            Self::ListExtend { i } => w!(LIST_EXTEND, i),
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
            Self::LoadBuildClass => w!(LOAD_BUILD_CLASS),
            Self::LoadFromDictOrDeref(i) => w!(LOAD_FROM_DICT_OR_DEREF, cell_name = i),
            Self::LoadConst { idx } => fmt_const("LOAD_CONST", arg, f, idx),
            Self::LoadSmallInt { idx } => w!(LOAD_SMALL_INT, idx),
            Self::LoadDeref(idx) => w!(LOAD_DEREF, cell_name = idx),
            Self::LoadFast(idx) => w!(LOAD_FAST, varname = idx),
            Self::LoadFastAndClear(idx) => w!(LOAD_FAST_AND_CLEAR, varname = idx),
            Self::LoadGlobal(idx) => w!(LOAD_GLOBAL, name = idx),
            Self::LoadName(idx) => w!(LOAD_NAME, name = idx),
            Self::LoadSpecial { method } => w!(LOAD_SPECIAL, method),
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
            Self::MakeFunction => w!(MAKE_FUNCTION),
            Self::MapAdd { i } => w!(MAP_ADD, i),
            Self::MatchClass(arg) => w!(MATCH_CLASS, arg),
            Self::MatchKeys => w!(MATCH_KEYS),
            Self::MatchMapping => w!(MATCH_MAPPING),
            Self::MatchSequence => w!(MATCH_SEQUENCE),
            Self::Nop => w!(NOP),
            Self::PopExcept => w!(POP_EXCEPT),
            Self::PopJumpIfFalse { target } => w!(POP_JUMP_IF_FALSE, target),
            Self::PopJumpIfTrue { target } => w!(POP_JUMP_IF_TRUE, target),
            Self::PopTop => w!(POP_TOP),
            Self::EndFor => w!(END_FOR),
            Self::PopIter => w!(POP_ITER),
            Self::PushExcInfo => w!(PUSH_EXC_INFO),
            Self::PushNull => w!(PUSH_NULL),
            Self::RaiseVarargs { kind } => w!(RAISE_VARARGS, ?kind),
            Self::Reraise { depth } => w!(RERAISE, depth),
            Self::Resume { arg } => w!(RESUME, arg),
            Self::ReturnValue => w!(RETURN_VALUE),
            Self::Send { target } => w!(SEND, target),
            Self::SetAdd { i } => w!(SET_ADD, i),
            Self::SetFunctionAttribute { attr } => w!(SET_FUNCTION_ATTRIBUTE, ?attr),
            Self::SetupAnnotations => w!(SETUP_ANNOTATIONS),
            Self::SetUpdate { i } => w!(SET_UPDATE, i),
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
            Self::BuildTemplate => w!(BUILD_TEMPLATE),
            Self::BuildInterpolation { oparg } => w!(BUILD_INTERPOLATION, oparg),
            _ => w!(RUSTPYTHON_PLACEHOLDER),
        }
    }
}

/// Instructions used by the compiler. They are not executed by the VM.
///
/// CPython 3.14.2 aligned (256-266).
#[derive(Clone, Copy, Debug)]
#[repr(u16)]
pub enum PseudoInstruction {
    // CPython 3.14.2 pseudo instructions (256-266)
    AnnotationsPlaceholder = 256,
    Jump { target: Arg<Label> } = 257,
    JumpIfFalse { target: Arg<Label> } = 258,
    JumpIfTrue { target: Arg<Label> } = 259,
    JumpNoInterrupt { target: Arg<Label> } = 260,
    LoadClosure(Arg<NameIdx>) = 261,
    PopBlock = 262,
    SetupCleanup = 263,
    SetupFinally = 264,
    SetupWith = 265,
    StoreFastMaybeNull(Arg<NameIdx>) = 266,
}

const _: () = assert!(mem::size_of::<PseudoInstruction>() == 2);

impl From<PseudoInstruction> for u16 {
    #[inline]
    fn from(ins: PseudoInstruction) -> Self {
        // SAFETY: there's no padding bits
        unsafe { mem::transmute::<PseudoInstruction, Self>(ins) }
    }
}

impl TryFrom<u16> for PseudoInstruction {
    type Error = MarshalError;

    #[inline]
    fn try_from(value: u16) -> Result<Self, MarshalError> {
        let start = u16::from(Self::AnnotationsPlaceholder);
        let end = u16::from(Self::StoreFastMaybeNull(Arg::marker()));

        if (start..=end).contains(&value) {
            Ok(unsafe { mem::transmute::<u16, Self>(value) })
        } else {
            Err(Self::Error::InvalidBytecode)
        }
    }
}

impl InstructionMetadata for PseudoInstruction {
    fn label_arg(&self) -> Option<Arg<Label>> {
        match self {
            Self::Jump { target: l }
            | Self::JumpIfFalse { target: l }
            | Self::JumpIfTrue { target: l }
            | Self::JumpNoInterrupt { target: l } => Some(*l),
            _ => None,
        }
    }

    fn is_scope_exit(&self) -> bool {
        false
    }

    fn is_unconditional_jump(&self) -> bool {
        matches!(self, Self::Jump { .. } | Self::JumpNoInterrupt { .. })
    }

    fn stack_effect(&self, _arg: OpArg) -> i32 {
        match self {
            Self::AnnotationsPlaceholder => 0,
            Self::Jump { .. } => 0,
            Self::JumpIfFalse { .. } => 0, // peek, don't pop: COPY + TO_BOOL + POP_JUMP_IF_FALSE
            Self::JumpIfTrue { .. } => 0,  // peek, don't pop: COPY + TO_BOOL + POP_JUMP_IF_TRUE
            Self::JumpNoInterrupt { .. } => 0,
            Self::LoadClosure(_) => 1,
            Self::PopBlock => 0,
            Self::SetupCleanup => 0,
            Self::SetupFinally => 0,
            Self::SetupWith => 0,
            Self::StoreFastMaybeNull(_) => -1,
        }
    }

    fn fmt_dis(
        &self,
        _arg: OpArg,
        _f: &mut fmt::Formatter<'_>,
        _ctx: &impl InstrDisplayContext,
        _expand_code_objects: bool,
        _pad: usize,
        _level: usize,
    ) -> fmt::Result {
        unimplemented!()
    }
}

#[derive(Clone, Copy, Debug)]
pub enum AnyInstruction {
    Real(Instruction),
    Pseudo(PseudoInstruction),
}

impl From<Instruction> for AnyInstruction {
    fn from(value: Instruction) -> Self {
        Self::Real(value)
    }
}

impl From<PseudoInstruction> for AnyInstruction {
    fn from(value: PseudoInstruction) -> Self {
        Self::Pseudo(value)
    }
}

impl TryFrom<u8> for AnyInstruction {
    type Error = MarshalError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(Instruction::try_from(value)?.into())
    }
}

impl TryFrom<u16> for AnyInstruction {
    type Error = MarshalError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match u8::try_from(value) {
            Ok(v) => v.try_into(),
            Err(_) => Ok(PseudoInstruction::try_from(value)?.into()),
        }
    }
}

macro_rules! inst_either {
    (fn $name:ident ( &self $(, $arg:ident : $arg_ty:ty )* ) -> $ret:ty ) => {
        fn $name(&self $(, $arg : $arg_ty )* ) -> $ret {
            match self {
                Self::Real(op) => op.$name($($arg),*),
                Self::Pseudo(op) => op.$name($($arg),*),
            }
        }
    };
}

impl InstructionMetadata for AnyInstruction {
    inst_either!(fn label_arg(&self) -> Option<Arg<Label>>);

    inst_either!(fn is_unconditional_jump(&self) -> bool);

    inst_either!(fn is_scope_exit(&self) -> bool);

    inst_either!(fn stack_effect(&self, arg: OpArg) -> i32);

    inst_either!(fn fmt_dis(
        &self,
        arg: OpArg,
        f: &mut fmt::Formatter<'_>,
        ctx: &impl InstrDisplayContext,
        expand_code_objects: bool,
        pad: usize,
        level: usize
    ) -> fmt::Result);
}

impl AnyInstruction {
    /// Gets the inner value of [`Self::Real`].
    pub const fn real(self) -> Option<Instruction> {
        match self {
            Self::Real(ins) => Some(ins),
            _ => None,
        }
    }

    /// Gets the inner value of [`Self::Pseudo`].
    pub const fn pseudo(self) -> Option<PseudoInstruction> {
        match self {
            Self::Pseudo(ins) => Some(ins),
            _ => None,
        }
    }

    /// Same as [`Self::real`] but panics if wasn't called on [`Self::Real`].
    ///
    /// # Panics
    ///
    /// If was called on something else other than [`Self::Real`].
    pub const fn expect_real(self) -> Instruction {
        self.real()
            .expect("Expected Instruction::Real, found Instruction::Pseudo")
    }

    /// Same as [`Self::pseudo`] but panics if wasn't called on [`Self::Pseudo`].
    ///
    /// # Panics
    ///
    /// If was called on something else other than [`Self::Pseudo`].
    pub const fn expect_pseudo(self) -> PseudoInstruction {
        self.pseudo()
            .expect("Expected Instruction::Pseudo, found Instruction::Real")
    }
}

pub trait InstructionMetadata {
    /// Gets the label stored inside this instruction, if it exists.
    fn label_arg(&self) -> Option<Arg<Label>>;

    fn is_scope_exit(&self) -> bool;

    fn is_unconditional_jump(&self) -> bool;

    /// What effect this instruction has on the stack
    ///
    /// # Examples
    ///
    /// ```
    /// use rustpython_compiler_core::bytecode::{Arg, Instruction, Label, InstructionMetadata};
    /// let (target, jump_arg) = Arg::new(Label(0xF));
    /// let jump_instruction = Instruction::JumpForward { target };
    /// assert_eq!(jump_instruction.stack_effect(jump_arg), 0);
    /// ```
    fn stack_effect(&self, arg: OpArg) -> i32;

    #[allow(clippy::too_many_arguments)]
    fn fmt_dis(
        &self,
        arg: OpArg,
        f: &mut fmt::Formatter<'_>,
        ctx: &impl InstrDisplayContext,
        expand_code_objects: bool,
        pad: usize,
        level: usize,
    ) -> fmt::Result;

    fn display(&self, arg: OpArg, ctx: &impl InstrDisplayContext) -> impl fmt::Display {
        fmt::from_fn(move |f| self.fmt_dis(arg, f, ctx, false, 0, 0))
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

/// Encode LOAD_ATTR oparg: bit 0 = method flag, bits 1+ = name index.
#[inline]
pub const fn encode_load_attr_arg(name_idx: u32, is_method: bool) -> u32 {
    (name_idx << 1) | (is_method as u32)
}

/// Decode LOAD_ATTR oparg: returns (name_idx, is_method).
#[inline]
pub const fn decode_load_attr_arg(oparg: u32) -> (u32, bool) {
    let is_method = (oparg & 1) == 1;
    let name_idx = oparg >> 1;
    (name_idx, is_method)
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
