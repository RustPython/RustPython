use core::{fmt, marker::PhantomData, mem};

use crate::{
    bytecode::{
        BorrowedConstant, Constant, InstrDisplayContext,
        oparg::{
            BinaryOperator, BuildSliceArgCount, CommonConstant, ComparisonOperator,
            ConvertValueOparg, IntrinsicFunction1, IntrinsicFunction2, Invert, Label, LoadAttr,
            LoadSuperAttr, MakeFunctionFlags, NameIdx, OpArg, OpArgByte, OpArgType, RaiseKind,
            SpecialMethod, StoreFastLoadFast, UnpackExArgs,
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
    Cache = 0,
    BinarySlice = 1,
    BuildTemplate = 2,
    BinaryOpInplaceAddUnicode = 3,
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
    NotTaken = 28,
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
        format: Arg<u32>,
    } = 45,
    BuildList {
        count: Arg<u32>,
    } = 46,
    BuildMap {
        count: Arg<u32>,
    } = 47,
    BuildSet {
        count: Arg<u32>,
    } = 48,
    BuildSlice {
        argc: Arg<BuildSliceArgCount>,
    } = 49,
    BuildString {
        count: Arg<u32>,
    } = 50,
    BuildTuple {
        count: Arg<u32>,
    } = 51,
    Call {
        argc: Arg<u32>,
    } = 52,
    CallIntrinsic1 {
        func: Arg<IntrinsicFunction1>,
    } = 53,
    CallIntrinsic2 {
        func: Arg<IntrinsicFunction2>,
    } = 54,
    CallKw {
        argc: Arg<u32>,
    } = 55,
    CompareOp {
        opname: Arg<ComparisonOperator>,
    } = 56,
    ContainsOp {
        invert: Arg<Invert>,
    } = 57,
    ConvertValue {
        oparg: Arg<ConvertValueOparg>,
    } = 58,
    Copy {
        i: Arg<u32>,
    } = 59,
    CopyFreeVars {
        n: Arg<u32>,
    } = 60,
    DeleteAttr {
        namei: Arg<NameIdx>,
    } = 61,
    DeleteDeref {
        i: Arg<NameIdx>,
    } = 62,
    DeleteFast {
        var_num: Arg<NameIdx>,
    } = 63,
    DeleteGlobal {
        namei: Arg<NameIdx>,
    } = 64,
    DeleteName {
        namei: Arg<NameIdx>,
    } = 65,
    DictMerge {
        i: Arg<u32>,
    } = 66,
    DictUpdate {
        i: Arg<u32>,
    } = 67,
    EndAsyncFor = 68,
    ExtendedArg = 69,
    ForIter {
        delta: Arg<Label>,
    } = 70,
    GetAwaitable {
        r#where: Arg<u32>,
    } = 71,
    ImportFrom {
        namei: Arg<NameIdx>,
    } = 72,
    ImportName {
        namei: Arg<NameIdx>,
    } = 73,
    IsOp {
        invert: Arg<Invert>,
    } = 74,
    JumpBackward {
        delta: Arg<Label>,
    } = 75,
    JumpBackwardNoInterrupt {
        delta: Arg<Label>,
    } = 76, // Placeholder
    JumpForward {
        delta: Arg<Label>,
    } = 77,
    ListAppend {
        i: Arg<u32>,
    } = 78,
    ListExtend {
        i: Arg<u32>,
    } = 79,
    LoadAttr {
        namei: Arg<LoadAttr>,
    } = 80,
    LoadCommonConstant {
        idx: Arg<CommonConstant>,
    } = 81,
    LoadConst {
        consti: Arg<u32>,
    } = 82,
    LoadDeref {
        i: Arg<NameIdx>,
    } = 83,
    LoadFast {
        var_num: Arg<NameIdx>,
    } = 84,
    LoadFastAndClear {
        var_num: Arg<NameIdx>,
    } = 85,
    LoadFastBorrow {
        var_num: Arg<NameIdx>,
    } = 86,
    LoadFastBorrowLoadFastBorrow {
        var_nums: Arg<u32>,
    } = 87,
    LoadFastCheck {
        var_num: Arg<NameIdx>,
    } = 88,
    LoadFastLoadFast {
        var_nums: Arg<u32>,
    } = 89,
    LoadFromDictOrDeref {
        i: Arg<NameIdx>,
    } = 90,
    LoadFromDictOrGlobals {
        i: Arg<NameIdx>,
    } = 91,
    LoadGlobal {
        namei: Arg<NameIdx>,
    } = 92,
    LoadName {
        namei: Arg<NameIdx>,
    } = 93,
    LoadSmallInt {
        i: Arg<u32>,
    } = 94,
    LoadSpecial {
        method: Arg<SpecialMethod>,
    } = 95,
    LoadSuperAttr {
        namei: Arg<LoadSuperAttr>,
    } = 96,
    MakeCell {
        i: Arg<NameIdx>,
    } = 97,
    MapAdd {
        i: Arg<u32>,
    } = 98,
    MatchClass {
        count: Arg<u32>,
    } = 99,
    PopJumpIfFalse {
        delta: Arg<Label>,
    } = 100,
    PopJumpIfNone {
        delta: Arg<Label>,
    } = 101,
    PopJumpIfNotNone {
        delta: Arg<Label>,
    } = 102,
    PopJumpIfTrue {
        delta: Arg<Label>,
    } = 103,
    RaiseVarargs {
        argc: Arg<RaiseKind>,
    } = 104,
    Reraise {
        depth: Arg<u32>,
    } = 105,
    Send {
        delta: Arg<Label>,
    } = 106,
    SetAdd {
        i: Arg<u32>,
    } = 107,
    SetFunctionAttribute {
        flag: Arg<MakeFunctionFlags>,
    } = 108,
    SetUpdate {
        i: Arg<u32>,
    } = 109,
    StoreAttr {
        namei: Arg<NameIdx>,
    } = 110,
    StoreDeref {
        i: Arg<NameIdx>,
    } = 111,
    StoreFast {
        var_num: Arg<NameIdx>,
    } = 112,
    StoreFastLoadFast {
        var_nums: Arg<StoreFastLoadFast>,
    } = 113,
    StoreFastStoreFast {
        var_nums: Arg<u32>,
    } = 114,
    StoreGlobal {
        namei: Arg<NameIdx>,
    } = 115,
    StoreName {
        namei: Arg<NameIdx>,
    } = 116,
    Swap {
        i: Arg<u32>,
    } = 117,
    UnpackEx {
        counts: Arg<UnpackExArgs>,
    } = 118,
    UnpackSequence {
        count: Arg<u32>,
    } = 119,
    YieldValue {
        arg: Arg<u32>,
    } = 120,
    // CPython 3.14 RESUME (128)
    Resume {
        context: Arg<u32>,
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
    InstrumentedEndFor = 234,
    InstrumentedPopIter = 235,
    InstrumentedEndSend = 236,
    InstrumentedForIter = 237,
    InstrumentedInstruction = 238,
    InstrumentedJumpForward = 239,
    InstrumentedNotTaken = 240,
    InstrumentedPopJumpIfTrue = 241,
    InstrumentedPopJumpIfFalse = 242,
    InstrumentedPopJumpIfNone = 243,
    InstrumentedPopJumpIfNotNone = 244,
    InstrumentedResume = 245,
    InstrumentedReturnValue = 246,
    InstrumentedYieldValue = 247,
    InstrumentedEndAsyncFor = 248,
    InstrumentedLoadSuperAttr = 249,
    InstrumentedCall = 250,
    InstrumentedCallKw = 251,
    InstrumentedCallFunctionEx = 252,
    InstrumentedJumpBackward = 253,
    InstrumentedLine = 254,
    EnterExecutor = 255, // Placeholder
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
        let resume_id = u8::from(Self::Resume {
            context: Arg::marker(),
        });
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

impl Instruction {
    /// Returns `true` if this is any instrumented opcode
    /// (regular INSTRUMENTED_*, INSTRUMENTED_LINE, or INSTRUMENTED_INSTRUCTION).
    pub fn is_instrumented(self) -> bool {
        self.to_base().is_some()
            || matches!(self, Self::InstrumentedLine | Self::InstrumentedInstruction)
    }

    /// Map a base opcode to its INSTRUMENTED_* variant.
    /// Returns `None` if this opcode has no instrumented counterpart.
    ///
    /// # Panics (debug)
    /// Panics if called on an already-instrumented opcode.
    pub fn to_instrumented(self) -> Option<Self> {
        debug_assert!(
            !self.is_instrumented(),
            "to_instrumented called on already-instrumented opcode {self:?}"
        );
        Some(match self {
            Self::Resume { .. } => Self::InstrumentedResume,
            Self::ReturnValue => Self::InstrumentedReturnValue,
            Self::YieldValue { .. } => Self::InstrumentedYieldValue,
            Self::Call { .. } => Self::InstrumentedCall,
            Self::CallKw { .. } => Self::InstrumentedCallKw,
            Self::CallFunctionEx => Self::InstrumentedCallFunctionEx,
            Self::LoadSuperAttr { .. } => Self::InstrumentedLoadSuperAttr,
            Self::JumpForward { .. } => Self::InstrumentedJumpForward,
            Self::JumpBackward { .. } => Self::InstrumentedJumpBackward,
            Self::ForIter { .. } => Self::InstrumentedForIter,
            Self::EndFor => Self::InstrumentedEndFor,
            Self::EndSend => Self::InstrumentedEndSend,
            Self::PopJumpIfTrue { .. } => Self::InstrumentedPopJumpIfTrue,
            Self::PopJumpIfFalse { .. } => Self::InstrumentedPopJumpIfFalse,
            Self::PopJumpIfNone { .. } => Self::InstrumentedPopJumpIfNone,
            Self::PopJumpIfNotNone { .. } => Self::InstrumentedPopJumpIfNotNone,
            Self::NotTaken => Self::InstrumentedNotTaken,
            Self::PopIter => Self::InstrumentedPopIter,
            Self::EndAsyncFor => Self::InstrumentedEndAsyncFor,
            _ => return None,
        })
    }

    /// Map an INSTRUMENTED_* opcode back to its base variant.
    /// Returns `None` for non-instrumented opcodes, and also for
    /// `InstrumentedLine` / `InstrumentedInstruction` which are event-layer
    /// placeholders without a fixed base opcode (the real opcode is stored in
    /// `CoMonitoringData`).
    ///
    /// The returned base opcode uses `Arg::marker()` for typed fields —
    /// only the opcode byte matters since `replace_op` preserves the arg byte.
    pub fn to_base(self) -> Option<Self> {
        Some(match self {
            Self::InstrumentedResume => Self::Resume {
                context: Arg::marker(),
            },
            Self::InstrumentedReturnValue => Self::ReturnValue,
            Self::InstrumentedYieldValue => Self::YieldValue { arg: Arg::marker() },
            Self::InstrumentedCall => Self::Call {
                argc: Arg::marker(),
            },
            Self::InstrumentedCallKw => Self::CallKw {
                argc: Arg::marker(),
            },
            Self::InstrumentedCallFunctionEx => Self::CallFunctionEx,
            Self::InstrumentedLoadSuperAttr => Self::LoadSuperAttr {
                namei: Arg::marker(),
            },
            Self::InstrumentedJumpForward => Self::JumpForward {
                delta: Arg::marker(),
            },
            Self::InstrumentedJumpBackward => Self::JumpBackward {
                delta: Arg::marker(),
            },
            Self::InstrumentedForIter => Self::ForIter {
                delta: Arg::marker(),
            },
            Self::InstrumentedEndFor => Self::EndFor,
            Self::InstrumentedEndSend => Self::EndSend,
            Self::InstrumentedPopJumpIfTrue => Self::PopJumpIfTrue {
                delta: Arg::marker(),
            },
            Self::InstrumentedPopJumpIfFalse => Self::PopJumpIfFalse {
                delta: Arg::marker(),
            },
            Self::InstrumentedPopJumpIfNone => Self::PopJumpIfNone {
                delta: Arg::marker(),
            },
            Self::InstrumentedPopJumpIfNotNone => Self::PopJumpIfNotNone {
                delta: Arg::marker(),
            },
            Self::InstrumentedNotTaken => Self::NotTaken,
            Self::InstrumentedPopIter => Self::PopIter,
            Self::InstrumentedEndAsyncFor => Self::EndAsyncFor,
            _ => return None,
        })
    }

    /// Map a specialized opcode back to its adaptive (base) variant.
    /// `_PyOpcode_Deopt`
    pub fn deoptimize(self) -> Self {
        match self {
            // LOAD_ATTR specializations
            Self::LoadAttrClass
            | Self::LoadAttrClassWithMetaclassCheck
            | Self::LoadAttrGetattributeOverridden
            | Self::LoadAttrInstanceValue
            | Self::LoadAttrMethodLazyDict
            | Self::LoadAttrMethodNoDict
            | Self::LoadAttrMethodWithValues
            | Self::LoadAttrModule
            | Self::LoadAttrNondescriptorNoDict
            | Self::LoadAttrNondescriptorWithValues
            | Self::LoadAttrProperty
            | Self::LoadAttrSlot
            | Self::LoadAttrWithHint => Self::LoadAttr {
                namei: Arg::marker(),
            },
            // BINARY_OP specializations
            Self::BinaryOpAddFloat
            | Self::BinaryOpAddInt
            | Self::BinaryOpAddUnicode
            | Self::BinaryOpExtend
            | Self::BinaryOpInplaceAddUnicode
            | Self::BinaryOpMultiplyFloat
            | Self::BinaryOpMultiplyInt
            | Self::BinaryOpSubscrDict
            | Self::BinaryOpSubscrGetitem
            | Self::BinaryOpSubscrListInt
            | Self::BinaryOpSubscrListSlice
            | Self::BinaryOpSubscrStrInt
            | Self::BinaryOpSubscrTupleInt
            | Self::BinaryOpSubtractFloat
            | Self::BinaryOpSubtractInt => Self::BinaryOp { op: Arg::marker() },
            // CALL specializations
            Self::CallAllocAndEnterInit
            | Self::CallBoundMethodExactArgs
            | Self::CallBoundMethodGeneral
            | Self::CallBuiltinClass
            | Self::CallBuiltinFast
            | Self::CallBuiltinFastWithKeywords
            | Self::CallBuiltinO
            | Self::CallIsinstance
            | Self::CallLen
            | Self::CallListAppend
            | Self::CallMethodDescriptorFast
            | Self::CallMethodDescriptorFastWithKeywords
            | Self::CallMethodDescriptorNoargs
            | Self::CallMethodDescriptorO
            | Self::CallNonPyGeneral
            | Self::CallPyExactArgs
            | Self::CallPyGeneral
            | Self::CallStr1
            | Self::CallTuple1
            | Self::CallType1 => Self::Call {
                argc: Arg::marker(),
            },
            // CALL_KW specializations
            Self::CallKwBoundMethod | Self::CallKwNonPy | Self::CallKwPy => Self::CallKw {
                argc: Arg::marker(),
            },
            // TO_BOOL specializations
            Self::ToBoolAlwaysTrue
            | Self::ToBoolBool
            | Self::ToBoolInt
            | Self::ToBoolList
            | Self::ToBoolNone
            | Self::ToBoolStr => Self::ToBool,
            // COMPARE_OP specializations
            Self::CompareOpFloat | Self::CompareOpInt | Self::CompareOpStr => Self::CompareOp {
                opname: Arg::marker(),
            },
            // CONTAINS_OP specializations
            Self::ContainsOpDict | Self::ContainsOpSet => Self::ContainsOp {
                invert: Arg::marker(),
            },
            // FOR_ITER specializations
            Self::ForIterGen | Self::ForIterList | Self::ForIterRange | Self::ForIterTuple => {
                Self::ForIter {
                    delta: Arg::marker(),
                }
            }
            // LOAD_GLOBAL specializations
            Self::LoadGlobalBuiltin | Self::LoadGlobalModule => Self::LoadGlobal {
                namei: Arg::marker(),
            },
            // STORE_ATTR specializations
            Self::StoreAttrInstanceValue | Self::StoreAttrSlot | Self::StoreAttrWithHint => {
                Self::StoreAttr {
                    namei: Arg::marker(),
                }
            }
            // LOAD_SUPER_ATTR specializations
            Self::LoadSuperAttrAttr | Self::LoadSuperAttrMethod => Self::LoadSuperAttr {
                namei: Arg::marker(),
            },
            // STORE_SUBSCR specializations
            Self::StoreSubscrDict | Self::StoreSubscrListInt => Self::StoreSubscr,
            // UNPACK_SEQUENCE specializations
            Self::UnpackSequenceList | Self::UnpackSequenceTuple | Self::UnpackSequenceTwoTuple => {
                Self::UnpackSequence {
                    count: Arg::marker(),
                }
            }
            // SEND specializations
            Self::SendGen => Self::Send {
                delta: Arg::marker(),
            },
            // LOAD_CONST specializations
            Self::LoadConstImmortal | Self::LoadConstMortal => Self::LoadConst {
                consti: Arg::marker(),
            },
            // RESUME specializations
            Self::ResumeCheck => Self::Resume {
                context: Arg::marker(),
            },
            // JUMP_BACKWARD specializations
            Self::JumpBackwardJit | Self::JumpBackwardNoJit => Self::JumpBackward {
                delta: Arg::marker(),
            },
            // Instrumented opcodes map back to their base
            _ => match self.to_base() {
                Some(base) => base,
                None => self,
            },
        }
    }

    /// Number of CACHE code units that follow this instruction.
    /// _PyOpcode_Caches
    pub fn cache_entries(self) -> usize {
        match self {
            // LOAD_ATTR: 9 cache entries
            Self::LoadAttr { .. }
            | Self::LoadAttrClass
            | Self::LoadAttrClassWithMetaclassCheck
            | Self::LoadAttrGetattributeOverridden
            | Self::LoadAttrInstanceValue
            | Self::LoadAttrMethodLazyDict
            | Self::LoadAttrMethodNoDict
            | Self::LoadAttrMethodWithValues
            | Self::LoadAttrModule
            | Self::LoadAttrNondescriptorNoDict
            | Self::LoadAttrNondescriptorWithValues
            | Self::LoadAttrProperty
            | Self::LoadAttrSlot
            | Self::LoadAttrWithHint => 9,

            // BINARY_OP: 5 cache entries
            Self::BinaryOp { .. }
            | Self::BinaryOpAddFloat
            | Self::BinaryOpAddInt
            | Self::BinaryOpAddUnicode
            | Self::BinaryOpExtend
            | Self::BinaryOpInplaceAddUnicode
            | Self::BinaryOpMultiplyFloat
            | Self::BinaryOpMultiplyInt
            | Self::BinaryOpSubscrDict
            | Self::BinaryOpSubscrGetitem
            | Self::BinaryOpSubscrListInt
            | Self::BinaryOpSubscrListSlice
            | Self::BinaryOpSubscrStrInt
            | Self::BinaryOpSubscrTupleInt
            | Self::BinaryOpSubtractFloat
            | Self::BinaryOpSubtractInt => 5,

            // LOAD_GLOBAL / STORE_ATTR: 4 cache entries
            Self::LoadGlobal { .. }
            | Self::LoadGlobalBuiltin
            | Self::LoadGlobalModule
            | Self::StoreAttr { .. }
            | Self::StoreAttrInstanceValue
            | Self::StoreAttrSlot
            | Self::StoreAttrWithHint => 4,

            // CALL / CALL_KW / TO_BOOL: 3 cache entries
            Self::Call { .. }
            | Self::CallAllocAndEnterInit
            | Self::CallBoundMethodExactArgs
            | Self::CallBoundMethodGeneral
            | Self::CallBuiltinClass
            | Self::CallBuiltinFast
            | Self::CallBuiltinFastWithKeywords
            | Self::CallBuiltinO
            | Self::CallIsinstance
            | Self::CallLen
            | Self::CallListAppend
            | Self::CallMethodDescriptorFast
            | Self::CallMethodDescriptorFastWithKeywords
            | Self::CallMethodDescriptorNoargs
            | Self::CallMethodDescriptorO
            | Self::CallNonPyGeneral
            | Self::CallPyExactArgs
            | Self::CallPyGeneral
            | Self::CallStr1
            | Self::CallTuple1
            | Self::CallType1
            | Self::CallKw { .. }
            | Self::CallKwBoundMethod
            | Self::CallKwNonPy
            | Self::CallKwPy
            | Self::ToBool
            | Self::ToBoolAlwaysTrue
            | Self::ToBoolBool
            | Self::ToBoolInt
            | Self::ToBoolList
            | Self::ToBoolNone
            | Self::ToBoolStr => 3,

            // 1 cache entry
            Self::CompareOp { .. }
            | Self::CompareOpFloat
            | Self::CompareOpInt
            | Self::CompareOpStr
            | Self::ContainsOp { .. }
            | Self::ContainsOpDict
            | Self::ContainsOpSet
            | Self::ForIter { .. }
            | Self::ForIterGen
            | Self::ForIterList
            | Self::ForIterRange
            | Self::ForIterTuple
            | Self::JumpBackward { .. }
            | Self::JumpBackwardJit
            | Self::JumpBackwardNoJit
            | Self::LoadSuperAttr { .. }
            | Self::LoadSuperAttrAttr
            | Self::LoadSuperAttrMethod
            | Self::PopJumpIfTrue { .. }
            | Self::PopJumpIfFalse { .. }
            | Self::PopJumpIfNone { .. }
            | Self::PopJumpIfNotNone { .. }
            | Self::Send { .. }
            | Self::SendGen
            | Self::StoreSubscr
            | Self::StoreSubscrDict
            | Self::StoreSubscrListInt
            | Self::UnpackSequence { .. }
            | Self::UnpackSequenceList
            | Self::UnpackSequenceTuple
            | Self::UnpackSequenceTwoTuple => 1,

            // Instrumented opcodes have the same cache entries as their base
            _ => match self.to_base() {
                Some(base) => base.cache_entries(),
                None => 0,
            },
        }
    }
}

impl InstructionMetadata for Instruction {
    #[inline]
    fn label_arg(&self) -> Option<Arg<Label>> {
        match self {
            Self::JumpBackward { delta: l }
            | Self::JumpBackwardNoInterrupt { delta: l }
            | Self::JumpForward { delta: l }
            | Self::PopJumpIfTrue { delta: l }
            | Self::PopJumpIfFalse { delta: l }
            | Self::PopJumpIfNone { delta: l }
            | Self::PopJumpIfNotNone { delta: l }
            | Self::ForIter { delta: l }
            | Self::Send { delta: l } => Some(*l),
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

    fn stack_effect_info(&self, oparg: u32) -> StackEffect {
        // Reason for converting oparg to i32 is because of expressions like `1 + (oparg -1)`
        // that causes underflow errors.
        let oparg = i32::try_from(oparg).expect("oparg does not fit in an `i32`");

        // NOTE: Please don't "simplify" expressions here (i.e. `1 + (oparg - 1)`)
        // as it will be harder to see diff with what CPython auto-generates
        let (pushed, popped) = match self {
            Self::BinaryOp { .. } => (1, 2),
            Self::BinaryOpAddFloat => (1, 2),
            Self::BinaryOpAddInt => (1, 2),
            Self::BinaryOpAddUnicode => (1, 2),
            Self::BinaryOpExtend => (1, 2),
            Self::BinaryOpInplaceAddUnicode => (0, 2),
            Self::BinaryOpMultiplyFloat => (1, 2),
            Self::BinaryOpMultiplyInt => (1, 2),
            Self::BinaryOpSubscrDict => (1, 2),
            Self::BinaryOpSubscrGetitem => (0, 2),
            Self::BinaryOpSubscrListInt => (1, 2),
            Self::BinaryOpSubscrListSlice => (1, 2),
            Self::BinaryOpSubscrStrInt => (1, 2),
            Self::BinaryOpSubscrTupleInt => (1, 2),
            Self::BinaryOpSubtractFloat => (1, 2),
            Self::BinaryOpSubtractInt => (1, 2),
            Self::BinarySlice { .. } => (1, 3),
            Self::BuildInterpolation { .. } => (1, 2 + (oparg & 1)),
            Self::BuildList { .. } => (1, oparg),
            Self::BuildMap { .. } => (1, oparg * 2),
            Self::BuildSet { .. } => (1, oparg),
            Self::BuildSlice { .. } => (1, oparg),
            Self::BuildString { .. } => (1, oparg),
            Self::BuildTemplate { .. } => (1, 2),
            Self::BuildTuple { .. } => (1, oparg),
            Self::Cache => (0, 0),
            Self::Call { .. } => (1, 2 + oparg),
            Self::CallAllocAndEnterInit => (0, 2 + oparg),
            Self::CallBoundMethodExactArgs => (0, 2 + oparg),
            Self::CallBoundMethodGeneral => (0, 2 + oparg),
            Self::CallBuiltinClass => (1, 2 + oparg),
            Self::CallBuiltinFast => (1, 2 + oparg),
            Self::CallBuiltinFastWithKeywords => (1, 2 + oparg),
            Self::CallBuiltinO => (1, 2 + oparg),
            Self::CallFunctionEx => (1, 4),
            Self::CallIntrinsic1 { .. } => (1, 1),
            Self::CallIntrinsic2 { .. } => (1, 2),
            Self::CallIsinstance => (1, 2 + oparg),
            Self::CallKw { .. } => (1, 3 + oparg),
            Self::CallKwBoundMethod => (0, 3 + oparg),
            Self::CallKwNonPy => (1, 3 + oparg),
            Self::CallKwPy => (0, 3 + oparg),
            Self::CallLen => (1, 3),
            Self::CallListAppend => (0, 3),
            Self::CallMethodDescriptorFast => (1, 2 + oparg),
            Self::CallMethodDescriptorFastWithKeywords => (1, 2 + oparg),
            Self::CallMethodDescriptorNoargs => (1, 2 + oparg),
            Self::CallMethodDescriptorO => (1, 2 + oparg),
            Self::CallNonPyGeneral => (1, 2 + oparg),
            Self::CallPyExactArgs => (0, 2 + oparg),
            Self::CallPyGeneral => (0, 2 + oparg),
            Self::CallStr1 => (1, 3),
            Self::CallTuple1 => (1, 3),
            Self::CallType1 => (1, 3),
            Self::CheckEgMatch => (2, 2),
            Self::CheckExcMatch => (2, 2),
            Self::CleanupThrow => (2, 3),
            Self::CompareOp { .. } => (1, 2),
            Self::CompareOpFloat => (1, 2),
            Self::CompareOpInt => (1, 2),
            Self::CompareOpStr => (1, 2),
            Self::ContainsOp { .. } => (1, 2),
            Self::ContainsOpDict => (1, 2),
            Self::ContainsOpSet => (1, 2),
            Self::ConvertValue { .. } => (1, 1),
            Self::Copy { .. } => (2 + (oparg - 1), 1 + (oparg - 1)),
            Self::CopyFreeVars { .. } => (0, 0),
            Self::DeleteAttr { .. } => (0, 1),
            Self::DeleteDeref { .. } => (0, 0),
            Self::DeleteFast { .. } => (0, 0),
            Self::DeleteGlobal { .. } => (0, 0),
            Self::DeleteName { .. } => (0, 0),
            Self::DeleteSubscr => (0, 2),
            Self::DictMerge { .. } => (4 + (oparg - 1), 5 + (oparg - 1)),
            Self::DictUpdate { .. } => (1 + (oparg - 1), 2 + (oparg - 1)),
            Self::EndAsyncFor => (0, 2),
            Self::EndFor => (0, 1),
            Self::EndSend => (1, 2),
            Self::EnterExecutor => (0, 0),
            Self::ExitInitCheck => (0, 1),
            Self::ExtendedArg => (0, 0),
            Self::ForIter { .. } => (2, 1),
            Self::ForIterGen => (1, 1),
            Self::ForIterList => (2, 1),
            Self::ForIterRange => (2, 1),
            Self::ForIterTuple => (2, 1),
            Self::FormatSimple => (1, 1),
            Self::FormatWithSpec => (1, 2),
            Self::GetAIter => (1, 1),
            Self::GetANext => (2, 1),
            Self::GetAwaitable { .. } => (1, 1),
            Self::GetIter => (1, 1),
            Self::GetLen => (2, 1),
            Self::GetYieldFromIter => (1, 1),
            Self::ImportFrom { .. } => (2, 1),
            Self::ImportName { .. } => (1, 2),
            Self::InstrumentedCall => (1, 2 + oparg),
            Self::InstrumentedCallFunctionEx => (1, 4),
            Self::InstrumentedCallKw => (1, 3 + oparg),
            Self::InstrumentedEndAsyncFor => (0, 2),
            Self::InstrumentedEndFor => (1, 2),
            Self::InstrumentedEndSend => (1, 2),
            Self::InstrumentedForIter => (2, 1),
            Self::InstrumentedInstruction => (0, 0),
            Self::InstrumentedJumpBackward => (0, 0),
            Self::InstrumentedJumpForward => (0, 0),
            Self::InstrumentedLine => (0, 0),
            Self::InstrumentedLoadSuperAttr => (1 + (oparg & 1), 3),
            Self::InstrumentedNotTaken => (0, 0),
            Self::InstrumentedPopIter => (0, 1),
            Self::InstrumentedPopJumpIfFalse => (0, 1),
            Self::InstrumentedPopJumpIfNone => (0, 1),
            Self::InstrumentedPopJumpIfNotNone => (0, 1),
            Self::InstrumentedPopJumpIfTrue => (0, 1),
            Self::InstrumentedResume => (0, 0),
            Self::InstrumentedReturnValue => (1, 1),
            Self::InstrumentedYieldValue => (1, 1),
            Self::InterpreterExit => (0, 1),
            Self::IsOp { .. } => (1, 2),
            Self::JumpBackward { .. } => (0, 0),
            Self::JumpBackwardJit => (0, 0),
            Self::JumpBackwardNoInterrupt { .. } => (0, 0),
            Self::JumpBackwardNoJit => (0, 0),
            Self::JumpForward { .. } => (0, 0),
            Self::ListAppend { .. } => (1 + (oparg - 1), 2 + (oparg - 1)),
            Self::ListExtend { .. } => (1 + (oparg - 1), 2 + (oparg - 1)),
            Self::LoadAttr { .. } => (1 + (oparg & 1), 1),
            Self::LoadAttrClass => (1 + (oparg & 1), 1),
            Self::LoadAttrClassWithMetaclassCheck => (1 + (oparg & 1), 1),
            Self::LoadAttrGetattributeOverridden => (1, 1),
            Self::LoadAttrInstanceValue => (1 + (oparg & 1), 1),
            Self::LoadAttrMethodLazyDict => (2, 1),
            Self::LoadAttrMethodNoDict => (2, 1),
            Self::LoadAttrMethodWithValues => (2, 1),
            Self::LoadAttrModule => (1 + (oparg & 1), 1),
            Self::LoadAttrNondescriptorNoDict => (1, 1),
            Self::LoadAttrNondescriptorWithValues => (1, 1),
            Self::LoadAttrProperty => (0, 1),
            Self::LoadAttrSlot => (1 + (oparg & 1), 1),
            Self::LoadAttrWithHint => (1 + (oparg & 1), 1),
            Self::LoadBuildClass => (1, 0),
            Self::LoadCommonConstant { .. } => (1, 0),
            Self::LoadConst { .. } => (1, 0),
            Self::LoadConstImmortal => (1, 0),
            Self::LoadConstMortal => (1, 0),
            Self::LoadDeref { .. } => (1, 0),
            Self::LoadFast { .. } => (1, 0),
            Self::LoadFastAndClear { .. } => (1, 0),
            Self::LoadFastBorrow { .. } => (1, 0),
            Self::LoadFastBorrowLoadFastBorrow { .. } => (2, 0),
            Self::LoadFastCheck { .. } => (1, 0),
            Self::LoadFastLoadFast { .. } => (2, 0),
            Self::LoadFromDictOrDeref { .. } => (1, 1),
            Self::LoadFromDictOrGlobals { .. } => (1, 1),
            Self::LoadGlobal { .. } => (1 + (oparg & 1), 0),
            Self::LoadGlobalBuiltin => (1 + (oparg & 1), 0),
            Self::LoadGlobalModule => (1 + (oparg & 1), 0),
            Self::LoadLocals => (1, 0),
            Self::LoadName { .. } => (1, 0),
            Self::LoadSmallInt { .. } => (1, 0),
            Self::LoadSpecial { .. } => (1, 1),
            Self::LoadSuperAttr { .. } => (1 + (oparg & 1), 3),
            Self::LoadSuperAttrAttr => (1, 3),
            Self::LoadSuperAttrMethod => (2, 3),
            Self::MakeCell { .. } => (0, 0),
            Self::MakeFunction { .. } => (1, 1),
            Self::MapAdd { .. } => (1 + (oparg - 1), 3 + (oparg - 1)),
            Self::MatchClass { .. } => (1, 3),
            Self::MatchKeys { .. } => (3, 2),
            Self::MatchMapping => (2, 1),
            Self::MatchSequence => (2, 1),
            Self::Nop => (0, 0),
            Self::NotTaken => (0, 0),
            Self::PopExcept => (0, 1),
            Self::PopIter => (0, 1),
            Self::PopJumpIfFalse { .. } => (0, 1),
            Self::PopJumpIfNone { .. } => (0, 1),
            Self::PopJumpIfNotNone { .. } => (0, 1),
            Self::PopJumpIfTrue { .. } => (0, 1),
            Self::PopTop => (0, 1),
            Self::PushExcInfo => (2, 1),
            Self::PushNull => (1, 0),
            Self::RaiseVarargs { .. } => (0, oparg),
            Self::Reraise { .. } => (oparg, 1 + oparg),
            Self::Reserved => (0, 0),
            Self::Resume { .. } => (0, 0),
            Self::ResumeCheck => (0, 0),
            Self::ReturnGenerator => (1, 0),
            Self::ReturnValue => (1, 1),
            Self::Send { .. } => (2, 2),
            Self::SendGen => (1, 2),
            Self::SetAdd { .. } => (1 + (oparg - 1), 2 + (oparg - 1)),
            Self::SetFunctionAttribute { .. } => (1, 2),
            Self::SetUpdate { .. } => (1 + (oparg - 1), 2 + (oparg - 1)),
            Self::SetupAnnotations => (0, 0),
            Self::StoreAttr { .. } => (0, 2),
            Self::StoreAttrInstanceValue => (0, 2),
            Self::StoreAttrSlot => (0, 2),
            Self::StoreAttrWithHint => (0, 2),
            Self::StoreDeref { .. } => (0, 1),
            Self::StoreFast { .. } => (0, 1),
            Self::StoreFastLoadFast { .. } => (1, 1),
            Self::StoreFastStoreFast { .. } => (0, 2),
            Self::StoreGlobal { .. } => (0, 1),
            Self::StoreName { .. } => (0, 1),
            Self::StoreSlice => (0, 4),
            Self::StoreSubscr => (0, 3),
            Self::StoreSubscrDict => (0, 3),
            Self::StoreSubscrListInt => (0, 3),
            Self::Swap { .. } => (2 + (oparg - 2), 2 + (oparg - 2)),
            Self::ToBool => (1, 1),
            Self::ToBoolAlwaysTrue => (1, 1),
            Self::ToBoolBool => (1, 1),
            Self::ToBoolInt => (1, 1),
            Self::ToBoolList => (1, 1),
            Self::ToBoolNone => (1, 1),
            Self::ToBoolStr => (1, 1),
            Self::UnaryInvert => (1, 1),
            Self::UnaryNegative => (1, 1),
            Self::UnaryNot => (1, 1),
            Self::UnpackEx { .. } => (1 + (oparg & 0xFF) + (oparg >> 8), 1),
            Self::UnpackSequence { .. } => (oparg, 1),
            Self::UnpackSequenceList => (oparg, 1),
            Self::UnpackSequenceTuple => (oparg, 1),
            Self::UnpackSequenceTwoTuple => (2, 1),
            Self::WithExceptStart => (6, 5),
            Self::YieldValue { .. } => (1, 1),
        };

        debug_assert!((0..=i32::MAX).contains(&pushed));
        debug_assert!((0..=i32::MAX).contains(&popped));

        StackEffect::new(pushed as u32, popped as u32)
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
            Self::BinarySlice => w!(BINARY_SLICE),
            Self::BinaryOp { op } => write!(f, "{:pad$}({})", "BINARY_OP", op.get(arg)),
            Self::BinaryOpInplaceAddUnicode => w!(BINARY_OP_INPLACE_ADD_UNICODE),
            Self::BuildList { count } => w!(BUILD_LIST, count),
            Self::BuildMap { count } => w!(BUILD_MAP, count),
            Self::BuildSet { count } => w!(BUILD_SET, count),
            Self::BuildSlice { argc } => w!(BUILD_SLICE, ?argc),
            Self::BuildString { count } => w!(BUILD_STRING, count),
            Self::BuildTuple { count } => w!(BUILD_TUPLE, count),
            Self::Call { argc } => w!(CALL, argc),
            Self::CallFunctionEx => w!(CALL_FUNCTION_EX),
            Self::CallKw { argc } => w!(CALL_KW, argc),
            Self::CallIntrinsic1 { func } => w!(CALL_INTRINSIC_1, ?func),
            Self::CallIntrinsic2 { func } => w!(CALL_INTRINSIC_2, ?func),
            Self::Cache => w!(CACHE),
            Self::CheckEgMatch => w!(CHECK_EG_MATCH),
            Self::CheckExcMatch => w!(CHECK_EXC_MATCH),
            Self::CleanupThrow => w!(CLEANUP_THROW),
            Self::CompareOp { opname } => w!(COMPARE_OP, ?opname),
            Self::ContainsOp { invert } => w!(CONTAINS_OP, ?invert),
            Self::ConvertValue { oparg } => write!(f, "{:pad$}{}", "CONVERT_VALUE", oparg.get(arg)),
            Self::Copy { i } => w!(COPY, i),
            Self::CopyFreeVars { n } => w!(COPY_FREE_VARS, n),
            Self::DeleteAttr { namei } => w!(DELETE_ATTR, name = namei),
            Self::DeleteDeref { i } => w!(DELETE_DEREF, cell_name = i),
            Self::DeleteFast { var_num } => w!(DELETE_FAST, varname = var_num),
            Self::DeleteGlobal { namei } => w!(DELETE_GLOBAL, name = namei),
            Self::DeleteName { namei } => w!(DELETE_NAME, name = namei),
            Self::DeleteSubscr => w!(DELETE_SUBSCR),
            Self::DictMerge { i } => w!(DICT_MERGE, i),
            Self::DictUpdate { i } => w!(DICT_UPDATE, i),
            Self::EndAsyncFor => w!(END_ASYNC_FOR),
            Self::EndSend => w!(END_SEND),
            Self::ExtendedArg => w!(EXTENDED_ARG, Arg::<u32>::marker()),
            Self::ExitInitCheck => w!(EXIT_INIT_CHECK),
            Self::ForIter { delta } => w!(FOR_ITER, delta),
            Self::FormatSimple => w!(FORMAT_SIMPLE),
            Self::FormatWithSpec => w!(FORMAT_WITH_SPEC),
            Self::GetAIter => w!(GET_AITER),
            Self::GetANext => w!(GET_ANEXT),
            Self::GetAwaitable { r#where } => w!(GET_AWAITABLE, r#where),
            Self::Reserved => w!(RESERVED),
            Self::GetIter => w!(GET_ITER),
            Self::GetLen => w!(GET_LEN),
            Self::ImportFrom { namei } => w!(IMPORT_FROM, name = namei),
            Self::ImportName { namei } => w!(IMPORT_NAME, name = namei),
            Self::InterpreterExit => w!(INTERPRETER_EXIT),
            Self::IsOp { invert } => w!(IS_OP, ?invert),
            Self::JumpBackward { delta } => w!(JUMP_BACKWARD, delta),
            Self::JumpBackwardNoInterrupt { delta } => w!(JUMP_BACKWARD_NO_INTERRUPT, delta),
            Self::JumpForward { delta } => w!(JUMP_FORWARD, delta),
            Self::ListAppend { i } => w!(LIST_APPEND, i),
            Self::ListExtend { i } => w!(LIST_EXTEND, i),
            Self::LoadAttr { namei } => {
                let oparg = namei.get(arg);
                let oparg_u32 = u32::from(oparg);
                let attr_name = name(oparg.name_idx());
                if oparg.is_method() {
                    write!(
                        f,
                        "{:pad$}({}, {}, method=true)",
                        "LOAD_ATTR", oparg_u32, attr_name
                    )
                } else {
                    write!(f, "{:pad$}({}, {})", "LOAD_ATTR", oparg_u32, attr_name)
                }
            }
            Self::LoadBuildClass => w!(LOAD_BUILD_CLASS),
            Self::LoadCommonConstant { idx } => w!(LOAD_COMMON_CONSTANT, ?idx),
            Self::LoadFromDictOrDeref { i } => w!(LOAD_FROM_DICT_OR_DEREF, cell_name = i),
            Self::LoadConst { consti } => fmt_const("LOAD_CONST", arg, f, consti),
            Self::LoadSmallInt { i } => w!(LOAD_SMALL_INT, i),
            Self::LoadDeref { i } => w!(LOAD_DEREF, cell_name = i),
            Self::LoadFast { var_num } => w!(LOAD_FAST, varname = var_num),
            Self::LoadFastAndClear { var_num } => w!(LOAD_FAST_AND_CLEAR, varname = var_num),
            Self::LoadFastBorrow { var_num } => w!(LOAD_FAST_BORROW, varname = var_num),
            Self::LoadFastCheck { var_num } => w!(LOAD_FAST_CHECK, varname = var_num),
            Self::LoadFastLoadFast { var_nums } => {
                let oparg = var_nums.get(arg);
                let idx1 = oparg >> 4;
                let idx2 = oparg & 15;
                let name1 = varname(idx1);
                let name2 = varname(idx2);
                write!(f, "{:pad$}({}, {})", "LOAD_FAST_LOAD_FAST", name1, name2)
            }
            Self::LoadFastBorrowLoadFastBorrow { var_nums } => {
                let oparg = var_nums.get(arg);
                let idx1 = oparg >> 4;
                let idx2 = oparg & 15;
                let name1 = varname(idx1);
                let name2 = varname(idx2);
                write!(
                    f,
                    "{:pad$}({}, {})",
                    "LOAD_FAST_BORROW_LOAD_FAST_BORROW", name1, name2
                )
            }
            Self::LoadFromDictOrGlobals { i } => w!(LOAD_FROM_DICT_OR_GLOBALS, name = i),
            Self::LoadGlobal { namei } => {
                let oparg = namei.get(arg);
                let name_idx = oparg >> 1;
                if (oparg & 1) != 0 {
                    write!(
                        f,
                        "{:pad$}({}, NULL + {})",
                        "LOAD_GLOBAL",
                        oparg,
                        name(name_idx)
                    )
                } else {
                    write!(f, "{:pad$}({}, {})", "LOAD_GLOBAL", oparg, name(name_idx))
                }
            }
            Self::LoadGlobalBuiltin => {
                let oparg = u32::from(arg);
                let name_idx = oparg >> 1;
                if (oparg & 1) != 0 {
                    write!(
                        f,
                        "{:pad$}({}, NULL + {})",
                        "LOAD_GLOBAL_BUILTIN",
                        oparg,
                        name(name_idx)
                    )
                } else {
                    write!(
                        f,
                        "{:pad$}({}, {})",
                        "LOAD_GLOBAL_BUILTIN",
                        oparg,
                        name(name_idx)
                    )
                }
            }
            Self::LoadGlobalModule => {
                let oparg = u32::from(arg);
                let name_idx = oparg >> 1;
                if (oparg & 1) != 0 {
                    write!(
                        f,
                        "{:pad$}({}, NULL + {})",
                        "LOAD_GLOBAL_MODULE",
                        oparg,
                        name(name_idx)
                    )
                } else {
                    write!(
                        f,
                        "{:pad$}({}, {})",
                        "LOAD_GLOBAL_MODULE",
                        oparg,
                        name(name_idx)
                    )
                }
            }
            Self::LoadLocals => w!(LOAD_LOCALS),
            Self::LoadName { namei } => w!(LOAD_NAME, name = namei),
            Self::LoadSpecial { method } => w!(LOAD_SPECIAL, method),
            Self::LoadSuperAttr { namei } => {
                let oparg = namei.get(arg);
                write!(
                    f,
                    "{:pad$}({}, {}, method={}, class={})",
                    "LOAD_SUPER_ATTR",
                    u32::from(oparg),
                    name(oparg.name_idx()),
                    oparg.is_load_method(),
                    oparg.has_class()
                )
            }
            Self::MakeCell { i } => w!(MAKE_CELL, cell_name = i),
            Self::MakeFunction => w!(MAKE_FUNCTION),
            Self::MapAdd { i } => w!(MAP_ADD, i),
            Self::MatchClass { count } => w!(MATCH_CLASS, count),
            Self::MatchKeys => w!(MATCH_KEYS),
            Self::MatchMapping => w!(MATCH_MAPPING),
            Self::MatchSequence => w!(MATCH_SEQUENCE),
            Self::Nop => w!(NOP),
            Self::NotTaken => w!(NOT_TAKEN),
            Self::PopExcept => w!(POP_EXCEPT),
            Self::PopJumpIfFalse { delta } => w!(POP_JUMP_IF_FALSE, delta),
            Self::PopJumpIfNone { delta } => w!(POP_JUMP_IF_NONE, delta),
            Self::PopJumpIfNotNone { delta } => w!(POP_JUMP_IF_NOT_NONE, delta),
            Self::PopJumpIfTrue { delta } => w!(POP_JUMP_IF_TRUE, delta),
            Self::PopTop => w!(POP_TOP),
            Self::EndFor => w!(END_FOR),
            Self::PopIter => w!(POP_ITER),
            Self::PushExcInfo => w!(PUSH_EXC_INFO),
            Self::PushNull => w!(PUSH_NULL),
            Self::RaiseVarargs { argc } => w!(RAISE_VARARGS, ?argc),
            Self::Reraise { depth } => w!(RERAISE, depth),
            Self::Resume { context } => w!(RESUME, context),
            Self::ReturnValue => w!(RETURN_VALUE),
            Self::ReturnGenerator => w!(RETURN_GENERATOR),
            Self::Send { delta } => w!(SEND, delta),
            Self::SetAdd { i } => w!(SET_ADD, i),
            Self::SetFunctionAttribute { flag } => w!(SET_FUNCTION_ATTRIBUTE, ?flag),
            Self::SetupAnnotations => w!(SETUP_ANNOTATIONS),
            Self::SetUpdate { i } => w!(SET_UPDATE, i),
            Self::StoreAttr { namei } => w!(STORE_ATTR, name = namei),
            Self::StoreDeref { i } => w!(STORE_DEREF, cell_name = i),
            Self::StoreFast { var_num } => w!(STORE_FAST, varname = var_num),
            Self::StoreFastLoadFast { var_nums } => {
                let oparg = var_nums.get(arg);
                let store_idx = oparg.store_idx();
                let load_idx = oparg.load_idx();
                write!(f, "STORE_FAST_LOAD_FAST")?;
                write!(f, " ({}, {})", store_idx, load_idx)
            }
            Self::StoreFastStoreFast { var_nums } => {
                let oparg = var_nums.get(arg);
                let idx1 = oparg >> 4;
                let idx2 = oparg & 15;
                write!(
                    f,
                    "{:pad$}({}, {})",
                    "STORE_FAST_STORE_FAST",
                    varname(idx1),
                    varname(idx2)
                )
            }
            Self::StoreGlobal { namei } => w!(STORE_GLOBAL, name = namei),
            Self::StoreName { namei } => w!(STORE_NAME, name = namei),
            Self::StoreSlice => w!(STORE_SLICE),
            Self::StoreSubscr => w!(STORE_SUBSCR),
            Self::Swap { i } => w!(SWAP, i),
            Self::ToBool => w!(TO_BOOL),
            Self::UnpackEx { counts } => w!(UNPACK_EX, counts),
            Self::UnpackSequence { count } => w!(UNPACK_SEQUENCE, count),
            Self::WithExceptStart => w!(WITH_EXCEPT_START),
            Self::UnaryInvert => w!(UNARY_INVERT),
            Self::UnaryNegative => w!(UNARY_NEGATIVE),
            Self::UnaryNot => w!(UNARY_NOT),
            Self::YieldValue { arg } => w!(YIELD_VALUE, arg),
            Self::GetYieldFromIter => w!(GET_YIELD_FROM_ITER),
            Self::BuildTemplate => w!(BUILD_TEMPLATE),
            Self::BuildInterpolation { format } => w!(BUILD_INTERPOLATION, format),
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
    Jump { delta: Arg<Label> } = 257,
    JumpIfFalse { delta: Arg<Label> } = 258,
    JumpIfTrue { delta: Arg<Label> } = 259,
    JumpNoInterrupt { delta: Arg<Label> } = 260,
    LoadClosure { i: Arg<NameIdx> } = 261,
    PopBlock = 262,
    SetupCleanup { delta: Arg<Label> } = 263,
    SetupFinally { delta: Arg<Label> } = 264,
    SetupWith { delta: Arg<Label> } = 265,
    StoreFastMaybeNull { var_num: Arg<NameIdx> } = 266,
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
        let end = u16::from(Self::StoreFastMaybeNull {
            var_num: Arg::marker(),
        });

        if (start..=end).contains(&value) {
            Ok(unsafe { mem::transmute::<u16, Self>(value) })
        } else {
            Err(Self::Error::InvalidBytecode)
        }
    }
}

impl PseudoInstruction {
    /// Returns true if this is a block push pseudo instruction
    /// (SETUP_FINALLY, SETUP_CLEANUP, or SETUP_WITH).
    pub fn is_block_push(&self) -> bool {
        matches!(
            self,
            Self::SetupCleanup { .. } | Self::SetupFinally { .. } | Self::SetupWith { .. }
        )
    }
}

impl InstructionMetadata for PseudoInstruction {
    fn label_arg(&self) -> Option<Arg<Label>> {
        match self {
            Self::Jump { delta: l }
            | Self::JumpIfFalse { delta: l }
            | Self::JumpIfTrue { delta: l }
            | Self::JumpNoInterrupt { delta: l }
            | Self::SetupCleanup { delta: l }
            | Self::SetupFinally { delta: l }
            | Self::SetupWith { delta: l } => Some(*l),
            _ => None,
        }
    }

    fn is_scope_exit(&self) -> bool {
        false
    }

    fn stack_effect_info(&self, _oparg: u32) -> StackEffect {
        // Reason for converting oparg to i32 is because of expressions like `1 + (oparg -1)`
        // that causes underflow errors.
        let _oparg = i32::try_from(_oparg).expect("oparg does not fit in an `i32`");

        // NOTE: Please don't "simplify" expressions here (i.e. `1 + (oparg - 1)`)
        // as it will be harder to see diff with what CPython auto-generates
        let (pushed, popped) = match self {
            Self::AnnotationsPlaceholder => (0, 0),
            Self::Jump { .. } => (0, 0),
            Self::JumpIfFalse { .. } => (1, 1),
            Self::JumpIfTrue { .. } => (1, 1),
            Self::JumpNoInterrupt { .. } => (0, 0),
            Self::LoadClosure { .. } => (1, 0),
            Self::PopBlock => (0, 0),
            // Normal path effect is 0 (these are NOPs on fall-through).
            // Handler entry effects are computed directly in max_stackdepth().
            Self::SetupCleanup { .. } => (0, 0),
            Self::SetupFinally { .. } => (0, 0),
            Self::SetupWith { .. } => (0, 0),
            Self::StoreFastMaybeNull { .. } => (0, 1),
        };

        debug_assert!((0..=i32::MAX).contains(&pushed));
        debug_assert!((0..=i32::MAX).contains(&popped));

        StackEffect::new(pushed as u32, popped as u32)
    }

    fn is_unconditional_jump(&self) -> bool {
        matches!(self, Self::Jump { .. } | Self::JumpNoInterrupt { .. })
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

    inst_either!(fn stack_effect(&self, oparg: u32) -> i32);

    inst_either!(fn stack_effect_info(&self, oparg: u32) -> StackEffect);

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

    /// Returns true if this is a block push pseudo instruction
    /// (SETUP_FINALLY, SETUP_CLEANUP, or SETUP_WITH).
    pub fn is_block_push(&self) -> bool {
        matches!(self, Self::Pseudo(p) if p.is_block_push())
    }

    /// Returns true if this is a POP_BLOCK pseudo instruction.
    pub fn is_pop_block(&self) -> bool {
        matches!(self, Self::Pseudo(PseudoInstruction::PopBlock))
    }
}

/// What effect the instruction has on the stack.
#[derive(Clone, Copy)]
pub struct StackEffect {
    /// How many items the instruction is pushing on the stack.
    pushed: u32,
    /// How many items the instruction is popping from the stack.
    popped: u32,
}

impl StackEffect {
    /// Creates a new [`Self`].
    pub const fn new(pushed: u32, popped: u32) -> Self {
        Self { pushed, popped }
    }

    /// Get the calculated stack effect as [`i32`].
    pub fn effect(self) -> i32 {
        self.into()
    }

    /// Get the pushed count.
    pub const fn pushed(self) -> u32 {
        self.pushed
    }

    /// Get the popped count.
    pub const fn popped(self) -> u32 {
        self.popped
    }
}

impl From<StackEffect> for i32 {
    fn from(effect: StackEffect) -> Self {
        (effect.pushed() as i32) - (effect.popped() as i32)
    }
}

pub trait InstructionMetadata {
    /// Gets the label stored inside this instruction, if it exists.
    fn label_arg(&self) -> Option<Arg<Label>>;

    fn is_scope_exit(&self) -> bool;

    fn is_unconditional_jump(&self) -> bool;

    /// Stack effect info for how many items are pushed/popped from the stack,
    /// for this instruction.
    fn stack_effect_info(&self, oparg: u32) -> StackEffect;

    /// Stack effect of [`Self::stack_effect_info`].
    fn stack_effect(&self, oparg: u32) -> i32 {
        self.stack_effect_info(oparg).effect()
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
        (Self(PhantomData), OpArg::new(arg.into()))
    }

    #[inline]
    pub fn new_single(arg: T) -> (Self, OpArgByte)
    where
        T: Into<u8>,
    {
        (Self(PhantomData), OpArgByte::new(arg.into()))
    }

    #[inline(always)]
    pub fn get(self, arg: OpArg) -> T {
        self.try_get(arg).unwrap()
    }

    #[inline(always)]
    pub fn try_get(self, arg: OpArg) -> Result<T, MarshalError> {
        T::try_from(u32::from(arg)).map_err(|_| MarshalError::InvalidBytecode)
    }

    /// # Safety
    /// T::from_op_arg(self) must succeed
    #[inline(always)]
    pub unsafe fn get_unchecked(self, arg: OpArg) -> T {
        // SAFETY: requirements forwarded from caller
        unsafe { T::try_from(u32::from(arg)).unwrap_unchecked() }
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
