use core::{fmt, marker::PhantomData};

use crate::marshal::MarshalError;

use super::{OpArg, OpArgByte, OpArgType, oparg};

macro_rules! define_opcodes {
    (
        #[repr($typ:ident)]
        $opcode_vis:vis enum $opcode_name:ident;

        $(#[$instr_meta:meta])*
        $instr_vis:vis enum $instr_name:ident {
            $(
                $(#[$op_meta:meta])*
                    $op_name:ident $({ $arg_name:ident: Arg<$arg_type:ty> $(,)? })? = $op_id:expr
            ),* $(,)?
        }
    ) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        $opcode_vis enum $opcode_name {
            $($op_name),*
        }

        impl $opcode_name {
            #[doc = concat!("Converts this opcode to [`", stringify!($instr_name), "`].")]
            #[must_use]
            $opcode_vis const fn as_instruction(&self) -> $instr_name {
                match self {
                    $(
                        Self::$op_name => $instr_name::$op_name $({ $arg_name: Arg::marker() })?,
                    )*
                }
            }

            /// Map a specialized or instrumented opcode back to its adaptive (base) variant.
            #[must_use]
            $opcode_vis const fn deoptimize(self) -> Self {
                match self.deopt() {
                    Some(v) => v,
                    None => {
                        // Instrumented opcodes map back to their base
                        match self.to_base() {
                            Some(v) => v,
                            None => self,
                        }
                    }
                }
            }

            // NOTE: Keep private. Will be exposed under `try_from_u8/try_from_u16`.
            pub(super) const fn try_from_numeric(value: $typ) -> Result<Self, $crate::marshal::MarshalError> {
                match value {
                    $($op_id => Ok(Self::$op_name),)*
                    _ => Err($crate::marshal::MarshalError::InvalidBytecode),
                }
            }

            // NOTE: Keep private. Will be exposed under `as_u8/as_u16`.
            #[must_use]
            pub(super) const fn as_numeric(self) -> $typ {
                match self {
                    $(Self::$op_name => $op_id,)*
                }
            }
        }

        impl From<$opcode_name> for $instr_name {
            fn from(opcode: $opcode_name) -> Self {
                opcode.as_instruction()
            }
        }


        impl TryFrom<$typ> for $opcode_name {
            type Error = $crate::marshal::MarshalError;

            fn try_from(value: $typ) -> Result<Self, Self::Error> {
                Self::try_from_numeric(value)
            }
        }

        impl From<$opcode_name> for $typ {
            fn from(opcode: $opcode_name) -> Self {
                opcode.as_numeric()
            }
        }

        #[derive(Clone, Copy, Debug)]
        #[repr($typ)] // TODO: Remove this repr
        $instr_vis enum $instr_name {
            $(
                $(#[$op_meta])*
                $op_name $({ $arg_name: Arg<$arg_type> })? = $op_id // TODO: Don't assign value
            ),*
        }

        impl $instr_name {
            #[doc = concat!("Get the corresponding [`", stringify!($opcode_name), "`].")]
            #[must_use]
            $instr_vis const fn as_opcode(&self) -> $opcode_name {
                match self {
                    $(
                        Self::$op_name $({ $arg_name: _ })? => $opcode_name::$op_name,
                    )*
                }
            }

            #[must_use]
            $instr_vis const fn label_arg(&self) -> Option<Arg<oparg::Label>> {
                //define_opcodes!(@label_arm Self::$op_name $({ $arg_name } : $arg_type)?)
                define_opcodes!(@match self, Self, [$($op_name $({ $arg_name : $arg_type })?),*])
            }

            #[must_use]
            pub const fn to_base(self) -> Option<Self> {
                if let Some(op) = self.as_opcode().to_base() {
                    Some(op.as_instruction())
                } else {
                    None
                }
            }

            #[must_use]
            pub const fn to_instrumented(self) -> Option<Self> {
                if let Some(op) = self.as_opcode().to_instrumented() {
                    Some(op.as_instruction())
                } else {
                    None
                }
            }

            /// Returns `true` if this is any instrumented opcode.
            #[must_use]
            $instr_vis const fn is_instrumented(&self) -> bool {
                self.as_opcode().is_instrumented()
            }

            #[must_use]
            $instr_vis const fn is_unconditional_jump(&self) -> bool {
                self.as_opcode().is_unconditional_jump()
            }

            #[must_use]
            $instr_vis const fn is_block_push(&self) -> bool {
                self.as_opcode().is_block_push()
            }

            #[must_use]
            $instr_vis const fn is_scope_exit(&self) -> bool {
                self.as_opcode().is_scope_exit()
            }

            #[must_use]
            $instr_vis const fn is_terminator(&self) -> bool {
                self.as_opcode().is_terminator()
            }

            #[must_use]
            $instr_vis const fn is_no_fallthrough(&self) -> bool {
                self.as_opcode().is_no_fallthrough()
            }

            #[must_use]
            $instr_vis const fn has_target(&self) -> bool {
                self.as_opcode().has_target()
            }

            #[must_use]
            $instr_vis const fn has_jump(&self) -> bool {
                self.as_opcode().has_jump()
            }

            #[must_use]
            $instr_vis const fn has_arg(&self) -> bool {
                self.as_opcode().has_arg()
            }

            #[must_use]
            $instr_vis const fn has_const(&self) -> bool {
                self.as_opcode().has_const()
            }

            #[must_use]
            $instr_vis const fn has_eval_break(&self) -> bool {
                self.as_opcode().has_eval_break()
            }

            #[must_use]
            $instr_vis const fn is_assembler(&self) -> bool {
                self.as_opcode().is_assembler()
            }

            #[must_use]
            $instr_vis const fn cache_entries(&self) -> usize{
                self.as_opcode().cache_entries()
            }

            /// Map a specialized or instrumented opcode back to its adaptive (base) variant.
            #[must_use]
            $instr_vis const fn deoptimize(&self) -> Self {
                self.as_opcode().deoptimize().as_instruction()
            }

            #[must_use]
            $instr_vis fn stack_effect_jump(&self, oparg: u32) -> i32 {
                self.as_opcode().stack_effect_jump(oparg)
            }

            #[must_use]
            $instr_vis fn stack_effect_info(&self, oparg: u32) -> StackEffect {
                self.as_opcode().stack_effect_info(oparg)
            }

            #[must_use]
            $instr_vis fn stack_effect(&self, oparg: u32) -> i32 {
                self.as_opcode().stack_effect(oparg)
            }
        }

        impl From<$instr_name> for $opcode_name {
            fn from(instr: $instr_name) -> Self {
                instr.as_opcode()
            }
        }

        impl TryFrom<$typ> for $instr_name {
            type Error = $crate::marshal::MarshalError;

            fn try_from(value: $typ) -> Result<Self, Self::Error> {
                $opcode_name::try_from_numeric(value).map(Into::into)
            }
        }

        impl From<$instr_name> for $typ {
            fn from(instr: $instr_name) -> Self {
                instr.as_opcode().into()
            }
        }
    };

    // Base case: empty list
    (@match $self:expr, $name:ident, []) => {
        None
    };

    // Label field variant (with trailing variants)
    (@match $self:expr, $name:ident, [$variant:ident { $field:ident : Label } , $($rest:tt)*]) => {
        match $self {
            $name::$variant { $field } => Some(*$field),
            other => define_opcodes!(@match other, $name, [$($rest)*]),
        }
    };

    // Label field variant (last in list)
    (@match $self:expr, $name:ident, [$variant:ident { $field:ident : Label }]) => {
        match $self {
            $name::$variant { $field } => Some(*$field),
            other => define_opcodes!(@match other, $name, []),
        }
    };

    // Non-Label field variant (with trailing variants)
    (@match $self:expr, $name:ident, [$variant:ident { $field:ident : $type:ty } , $($rest:tt)*]) => {
        match $self {
            $name::$variant { .. } => None,
            other => define_opcodes!(@match other, $name, [$($rest)*]),
        }
    };

    // Non-Label field variant (last in list)
    (@match $self:expr, $name:ident, [$variant:ident { $field:ident : $type:ty }]) => {
        match $self {
            $name::$variant { .. } => None,
            _ => define_opcodes!(@match _, $name, []),
        }
    };

    // Unit variant (with trailing variants)
    (@match $self:expr, $name:ident, [$variant:ident , $($rest:tt)*]) => {
        match $self {
            $name::$variant => None,
            other => define_opcodes!(@match other, $name, [$($rest)*]),
        }
    };

    // Unit variant (last in list)
    (@match $self:expr, $name:ident, [$variant:ident]) => {
        match $self {
            $name::$variant => None,
            _ => define_opcodes!(@match _, $name, []),
        }
    };
}

define_opcodes!(
    #[repr(u8)]
    pub enum Opcode;

    pub enum Instruction {
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
        ExitInitCheck = 11,
        FormatSimple = 12,
        FormatWithSpec = 13,
        GetAiter = 14,
        GetAnext = 15,
        GetIter = 16,
        Reserved = 17,
        GetLen = 18,
        GetYieldFromIter = 19,
        InterpreterExit = 20,
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
        BinaryOp {
            op: Arg<oparg::BinaryOperator>,
        } = 44,
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
            argc: Arg<oparg::BuildSliceArgCount>,
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
            func: Arg<oparg::IntrinsicFunction1>,
        } = 53,
        CallIntrinsic2 {
            func: Arg<oparg::IntrinsicFunction2>,
        } = 54,
        CallKw {
            argc: Arg<u32>,
        } = 55,
        CompareOp {
            opname: Arg<oparg::ComparisonOperator>,
        } = 56,
        ContainsOp {
            invert: Arg<oparg::Invert>,
        } = 57,
        ConvertValue {
            oparg: Arg<oparg::ConvertValueOparg>,
        } = 58,
        Copy {
            i: Arg<u32>,
        } = 59,
        CopyFreeVars {
            n: Arg<u32>,
        } = 60,
        DeleteAttr {
            namei: Arg<oparg::NameIdx>,
        } = 61,
        DeleteDeref {
            i: Arg<oparg::VarNum>,
        } = 62,
        DeleteFast {
            var_num: Arg<oparg::VarNum>,
        } = 63,
        DeleteGlobal {
            namei: Arg<oparg::NameIdx>,
        } = 64,
        DeleteName {
            namei: Arg<oparg::NameIdx>,
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
            delta: Arg<oparg::Label>,
        } = 70,
        GetAwaitable {
            r#where: Arg<u32>,
        } = 71,
        ImportFrom {
            namei: Arg<oparg::NameIdx>,
        } = 72,
        ImportName {
            namei: Arg<oparg::NameIdx>,
        } = 73,
        IsOp {
            invert: Arg<oparg::Invert>,
        } = 74,
        JumpBackward {
            delta: Arg<oparg::Label>,
        } = 75,
        JumpBackwardNoInterrupt {
            delta: Arg<oparg::Label>,
        } = 76,
        JumpForward {
            delta: Arg<oparg::Label>,
        } = 77,
        ListAppend {
            i: Arg<u32>,
        } = 78,
        ListExtend {
            i: Arg<u32>,
        } = 79,
        LoadAttr {
            namei: Arg<oparg::LoadAttr>,
        } = 80,
        LoadCommonConstant {
            idx: Arg<oparg::CommonConstant>,
        } = 81,
        LoadConst {
            consti: Arg<oparg::ConstIdx>,
        } = 82,
        LoadDeref {
            i: Arg<oparg::VarNum>,
        } = 83,
        LoadFast {
            var_num: Arg<oparg::VarNum>,
        } = 84,
        LoadFastAndClear {
            var_num: Arg<oparg::VarNum>,
        } = 85,
        LoadFastBorrow {
            var_num: Arg<oparg::VarNum>,
        } = 86,
        LoadFastBorrowLoadFastBorrow {
            var_nums: Arg<oparg::VarNums>,
        } = 87,
        LoadFastCheck {
            var_num: Arg<oparg::VarNum>,
        } = 88,
        LoadFastLoadFast {
            var_nums: Arg<oparg::VarNums>,
        } = 89,
        LoadFromDictOrDeref {
            i: Arg<oparg::VarNum>,
        } = 90,
        LoadFromDictOrGlobals {
            i: Arg<oparg::NameIdx>,
        } = 91,
        LoadGlobal {
            namei: Arg<oparg::NameIdx>,
        } = 92,
        LoadName {
            namei: Arg<oparg::NameIdx>,
        } = 93,
        LoadSmallInt {
            i: Arg<u32>,
        } = 94,
        LoadSpecial {
            method: Arg<oparg::SpecialMethod>,
        } = 95,
        LoadSuperAttr {
            namei: Arg<oparg::LoadSuperAttr>,
        } = 96,
        MakeCell {
            i: Arg<oparg::VarNum>,
        } = 97,
        MapAdd {
            i: Arg<u32>,
        } = 98,
        MatchClass {
            count: Arg<u32>,
        } = 99,
        PopJumpIfFalse {
            delta: Arg<oparg::Label>,
        } = 100,
        PopJumpIfNone {
            delta: Arg<oparg::Label>,
        } = 101,
        PopJumpIfNotNone {
            delta: Arg<oparg::Label>,
        } = 102,
        PopJumpIfTrue {
            delta: Arg<oparg::Label>,
        } = 103,
        RaiseVarargs {
            argc: Arg<oparg::RaiseKind>,
        } = 104,
        Reraise {
            depth: Arg<u32>,
        } = 105,
        Send {
            delta: Arg<oparg::Label>,
        } = 106,
        SetAdd {
            i: Arg<u32>,
        } = 107,
        SetFunctionAttribute {
            flag: Arg<oparg::MakeFunctionFlag>,
        } = 108,
        SetUpdate {
            i: Arg<u32>,
        } = 109,
        StoreAttr {
            namei: Arg<oparg::NameIdx>,
        } = 110,
        StoreDeref {
            i: Arg<oparg::VarNum>,
        } = 111,
        StoreFast {
            var_num: Arg<oparg::VarNum>,
        } = 112,
        StoreFastLoadFast {
            var_nums: Arg<oparg::VarNums>,
        } = 113,
        StoreFastStoreFast {
            var_nums: Arg<oparg::VarNums>,
        } = 114,
        StoreGlobal {
            namei: Arg<oparg::NameIdx>,
        } = 115,
        StoreName {
            namei: Arg<oparg::NameIdx>,
        } = 116,
        Swap {
            i: Arg<u32>,
        } = 117,
        UnpackEx {
            counts: Arg<oparg::UnpackExArgs>,
        } = 118,
        UnpackSequence {
            count: Arg<u32>,
        } = 119,
        YieldValue {
            arg: Arg<u32>,
        } = 120,
        Resume {
            context: Arg<oparg::ResumeContext>,
        } = 128,
        BinaryOpAddFloat = 129,
        BinaryOpAddInt = 130,
        BinaryOpAddUnicode = 131,
        BinaryOpExtend = 132,
        BinaryOpMultiplyFloat = 133,
        BinaryOpMultiplyInt = 134,
        BinaryOpSubscrDict = 135,
        BinaryOpSubscrGetitem = 136,
        BinaryOpSubscrListInt = 137,
        BinaryOpSubscrListSlice = 138,
        BinaryOpSubscrStrInt = 139,
        BinaryOpSubscrTupleInt = 140,
        BinaryOpSubtractFloat = 141,
        BinaryOpSubtractInt = 142,
        CallAllocAndEnterInit = 143,
        CallBoundMethodExactArgs = 144,
        CallBoundMethodGeneral = 145,
        CallBuiltinClass = 146,
        CallBuiltinFast = 147,
        CallBuiltinFastWithKeywords = 148,
        CallBuiltinO = 149,
        CallIsinstance = 150,
        CallKwBoundMethod = 151,
        CallKwNonPy = 152,
        CallKwPy = 153,
        CallLen = 154,
        CallListAppend = 155,
        CallMethodDescriptorFast = 156,
        CallMethodDescriptorFastWithKeywords = 157,
        CallMethodDescriptorNoargs = 158,
        CallMethodDescriptorO = 159,
        CallNonPyGeneral = 160,
        CallPyExactArgs = 161,
        CallPyGeneral = 162,
        CallStr1 = 163,
        CallTuple1 = 164,
        CallType1 = 165,
        CompareOpFloat = 166,
        CompareOpInt = 167,
        CompareOpStr = 168,
        ContainsOpDict = 169,
        ContainsOpSet = 170,
        ForIterGen = 171,
        ForIterList = 172,
        ForIterRange = 173,
        ForIterTuple = 174,
        JumpBackwardJit = 175,
        JumpBackwardNoJit = 176,
        LoadAttrClass = 177,
        LoadAttrClassWithMetaclassCheck = 178,
        LoadAttrGetattributeOverridden = 179,
        LoadAttrInstanceValue = 180,
        LoadAttrMethodLazyDict = 181,
        LoadAttrMethodNoDict = 182,
        LoadAttrMethodWithValues = 183,
        LoadAttrModule = 184,
        LoadAttrNondescriptorNoDict = 185,
        LoadAttrNondescriptorWithValues = 186,
        LoadAttrProperty = 187,
        LoadAttrSlot = 188,
        LoadAttrWithHint = 189,
        LoadConstImmortal = 190,
        LoadConstMortal = 191,
        LoadGlobalBuiltin = 192,
        LoadGlobalModule = 193,
        LoadSuperAttrAttr = 194,
        LoadSuperAttrMethod = 195,
        ResumeCheck = 196,
        SendGen = 197,
        StoreAttrInstanceValue = 198,
        StoreAttrSlot = 199,
        StoreAttrWithHint = 200,
        StoreSubscrDict = 201,
        StoreSubscrListInt = 202,
        ToBoolAlwaysTrue = 203,
        ToBoolBool = 204,
        ToBoolInt = 205,
        ToBoolList = 206,
        ToBoolNone = 207,
        ToBoolStr = 208,
        UnpackSequenceList = 209,
        UnpackSequenceTuple = 210,
        UnpackSequenceTwoTuple = 211,
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
        EnterExecutor = 255,
    }
);

define_opcodes!(
    #[repr(u16)]
    pub enum PseudoOpcode;

    pub enum PseudoInstruction {
        AnnotationsPlaceholder = 256,
        Jump { delta: Arg<oparg::Label> } = 257,
        JumpIfFalse { delta: Arg<oparg::Label> } = 258,
        JumpIfTrue { delta: Arg<oparg::Label> } = 259,
        JumpNoInterrupt { delta: Arg<oparg::Label> } = 260,
        LoadClosure { i: Arg<oparg::NameIdx> } = 261,
        PopBlock = 262,
        SetupCleanup { delta: Arg<oparg::Label> } = 263,
        SetupFinally { delta: Arg<oparg::Label> } = 264,
        SetupWith { delta: Arg<oparg::Label> } = 265,
        StoreFastMaybeNull { var_num: Arg<oparg::NameIdx> } = 266,
    }
);

impl Opcode {
    #[must_use]
    pub const fn is_unconditional_jump(&self) -> bool {
        matches!(
            self,
            Self::JumpForward | Self::JumpBackward | Self::JumpBackwardNoInterrupt
        )
    }

    /// CPython's `IS_ASSEMBLER_OPCODE`.
    #[must_use]
    pub const fn is_assembler(&self) -> bool {
        matches!(
            self,
            Self::JumpForward | Self::JumpBackward | Self::JumpBackwardNoInterrupt
        )
    }

    #[must_use]
    pub const fn is_scope_exit(&self) -> bool {
        matches!(self, Self::ReturnValue | Self::RaiseVarargs | Self::Reraise)
    }

    /// CPython's `IS_TERMINATOR_OPCODE`.
    #[must_use]
    pub const fn is_terminator(&self) -> bool {
        self.has_jump() || self.is_scope_exit()
    }

    /// CPython's `IS_SCOPE_EXIT_OPCODE || IS_UNCONDITIONAL_JUMP_OPCODE`.
    #[must_use]
    pub const fn is_no_fallthrough(&self) -> bool {
        self.is_scope_exit() || self.is_unconditional_jump()
    }

    /// CPython's `HAS_TARGET`.
    #[must_use]
    pub const fn has_target(&self) -> bool {
        self.has_jump() || self.is_block_push()
    }

    #[must_use]
    pub const fn is_block_push(&self) -> bool {
        false
    }

    /// Stack effect of [`Self::stack_effect_info`].
    #[must_use]
    pub fn stack_effect(&self, oparg: u32) -> i32 {
        self.stack_effect_info(oparg).effect()
    }

    /// Stack effect when the instruction takes its branch (jump=true).
    ///
    /// CPython equivalent: `stack_effect(opcode, oparg, jump=True)`.
    /// Current opcode metadata has the same real-opcode stack effect
    /// for jump and fallthrough stack-depth calculation.
    #[must_use]
    pub fn stack_effect_jump(&self, oparg: u32) -> i32 {
        self.stack_effect(oparg)
    }
}

impl PseudoOpcode {
    #[must_use]
    pub const fn is_block_push(&self) -> bool {
        matches!(
            self,
            Self::SetupCleanup | Self::SetupFinally | Self::SetupWith
        )
    }

    #[must_use]
    pub const fn is_scope_exit(&self) -> bool {
        false
    }

    #[must_use]
    pub const fn is_unconditional_jump(&self) -> bool {
        matches!(self, Self::Jump | Self::JumpNoInterrupt)
    }

    #[must_use]
    pub const fn is_assembler(&self) -> bool {
        false
    }

    /// CPython's `IS_TERMINATOR_OPCODE`.
    #[must_use]
    pub const fn is_terminator(&self) -> bool {
        self.has_jump()
    }

    /// CPython's `IS_SCOPE_EXIT_OPCODE || IS_UNCONDITIONAL_JUMP_OPCODE`.
    #[must_use]
    pub const fn is_no_fallthrough(&self) -> bool {
        self.is_unconditional_jump()
    }

    /// CPython's `HAS_TARGET`.
    #[must_use]
    pub const fn has_target(&self) -> bool {
        self.has_jump() || self.is_block_push()
    }

    /// flowgraph.c get_stack_effects block-push non-jump case.
    #[must_use]
    pub fn stack_effect(&self, oparg: u32) -> i32 {
        if self.is_block_push() {
            0
        } else {
            self.stack_effect_info(oparg).effect()
        }
    }

    /// Handler entry effect for SETUP_* pseudo ops.
    ///
    /// Fallthrough effect is 0 (NOPs), but when the branch is taken the
    /// handler block starts with extra values on the stack:
    ///   SETUP_FINALLY:  +1  (exc)
    ///   SETUP_CLEANUP:  +2  (lasti + exc)
    ///   SETUP_WITH:     +1  (pops __enter__ result, pushes lasti + exc)
    #[must_use]
    pub fn stack_effect_jump(&self, oparg: u32) -> i32 {
        match self {
            Self::SetupFinally | Self::SetupWith => 1,
            Self::SetupCleanup => 2,
            _ => self.stack_effect(oparg),
        }
    }
}

macro_rules! either_real_pseudo {
    // Const
    (
        $(#[$meta:meta])*
        $vis:vis const fn $name:ident(&self $(, $arg:ident : $arg_ty:ty)*) -> $ret:ty
    ) => {
        $(#[$meta])*
        $vis const fn $name(&self $(, $arg: $arg_ty)*) -> $ret {
            match self {
                Self::Real(v) => v.$name($($arg),*),
                Self::Pseudo(v) => v.$name($($arg),*),
            }
        }
    };

    // Not const
    (
        $(#[$meta:meta])*
        $vis:vis fn $name:ident(&self $(, $arg:ident : $arg_ty:ty)*) -> $ret:ty
    ) => {
        $(#[$meta])*
        $vis fn $name(&self $(, $arg: $arg_ty)*) -> $ret {
            match self {
                Self::Real(v) => v.$name($($arg),*),
                Self::Pseudo(v) => v.$name($($arg),*),
            }
        }
    };
}

#[derive(Clone, Copy, Debug)]
pub enum AnyInstruction {
    Real(Instruction),
    Pseudo(PseudoInstruction),
}

impl AnyInstruction {
    either_real_pseudo!(
        #[must_use]
        pub const fn is_unconditional_jump(&self) -> bool
    );

    either_real_pseudo!(
        #[must_use]
        pub const fn is_scope_exit(&self) -> bool
    );

    either_real_pseudo!(
        #[must_use]
        pub const fn is_terminator(&self) -> bool
    );

    either_real_pseudo!(
        #[must_use]
        pub const fn is_no_fallthrough(&self) -> bool
    );

    either_real_pseudo!(
        #[must_use]
        pub const fn has_target(&self) -> bool
    );

    either_real_pseudo!(
        #[must_use]
        pub const fn has_jump(&self) -> bool
    );

    either_real_pseudo!(
        #[must_use]
        pub const fn has_arg(&self) -> bool
    );

    either_real_pseudo!(
        #[must_use]
        pub const fn has_const(&self) -> bool
    );

    either_real_pseudo!(
        #[must_use]
        pub const fn has_eval_break(&self) -> bool
    );

    either_real_pseudo!(
        #[must_use]
        pub const fn is_assembler(&self) -> bool
    );

    either_real_pseudo!(
        #[must_use]
        pub fn stack_effect(&self, oparg: u32) -> i32
    );

    either_real_pseudo!(
        #[must_use]
        pub fn stack_effect_jump(&self, oparg: u32) -> i32
    );

    either_real_pseudo!(
        #[must_use]
        pub fn stack_effect_info(&self, oparg: u32) -> StackEffect
    );
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

impl From<Opcode> for AnyInstruction {
    fn from(value: Opcode) -> Self {
        Self::Real(value.into())
    }
}

impl From<PseudoOpcode> for AnyInstruction {
    fn from(value: PseudoOpcode) -> Self {
        Self::Pseudo(value.into())
    }
}

impl From<AnyOpcode> for AnyInstruction {
    fn from(value: AnyOpcode) -> Self {
        match value {
            AnyOpcode::Real(op) => op.into(),
            AnyOpcode::Pseudo(op) => op.into(),
        }
    }
}

impl AnyInstruction {
    /// Inner value of [`Self::Real`].
    #[must_use]
    pub const fn real(self) -> Option<Instruction> {
        match self {
            Self::Real(ins) => Some(ins),
            _ => None,
        }
    }

    /// Inner value of [`Self::Pseudo`].
    #[must_use]
    pub const fn pseudo(self) -> Option<PseudoInstruction> {
        match self {
            Self::Pseudo(ins) => Some(ins),
            _ => None,
        }
    }

    /// Get [`Self::Real`] as [`Opcode`].
    #[must_use]
    pub const fn real_opcode(self) -> Option<Opcode> {
        match self.real() {
            Some(ins) => Some(ins.as_opcode()),
            _ => None,
        }
    }

    /// Get [`Self::Pseudo`] as [`PseudoOpcode`].
    #[must_use]
    pub const fn pseudo_opcode(self) -> Option<PseudoOpcode> {
        match self.pseudo() {
            Some(ins) => Some(ins.as_opcode()),
            _ => None,
        }
    }

    /// Same as [`Self::real`] but panics if wasn't called on [`Self::Real`].
    ///
    /// # Panics
    ///
    /// If was called on something else other than [`Self::Real`].
    #[must_use]
    pub const fn expect_real(self) -> Instruction {
        self.real()
            .expect("Expected AnyInstruction::Real, found AnyInstruction::Pseudo")
    }

    /// Same as [`Self::pseudo`] but panics if wasn't called on [`Self::Pseudo`].
    ///
    /// # Panics
    ///
    /// If was called on something else other than [`Self::Pseudo`].
    #[must_use]
    pub const fn expect_pseudo(self) -> PseudoInstruction {
        self.pseudo()
            .expect("Expected AnyInstruction::Pseudo, found AnyInstruction::Real")
    }

    /// Returns true if this is a [`PseudoInstruction::PopBlock`].
    #[must_use]
    pub const fn is_pop_block(self) -> bool {
        matches!(self, Self::Pseudo(PseudoInstruction::PopBlock))
    }

    /// See [`PseudoInstruction::is_block_push`].
    #[must_use]
    pub const fn is_block_push(self) -> bool {
        matches!(self, Self::Pseudo(p) if p.is_block_push())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnyOpcode {
    Real(Opcode),
    Pseudo(PseudoOpcode),
}

impl From<Opcode> for AnyOpcode {
    fn from(value: Opcode) -> Self {
        Self::Real(value)
    }
}

impl From<PseudoOpcode> for AnyOpcode {
    fn from(value: PseudoOpcode) -> Self {
        Self::Pseudo(value)
    }
}

impl TryFrom<u8> for AnyOpcode {
    type Error = MarshalError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(Opcode::try_from(value)?.into())
    }
}

impl TryFrom<u16> for AnyOpcode {
    type Error = MarshalError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match u8::try_from(value) {
            Ok(v) => v.try_into(),
            Err(_) => Ok(PseudoOpcode::try_from(value)?.into()),
        }
    }
}

impl From<AnyInstruction> for AnyOpcode {
    fn from(value: AnyInstruction) -> Self {
        match value {
            AnyInstruction::Real(instr) => Self::Real(instr.into()),
            AnyInstruction::Pseudo(instr) => Self::Pseudo(instr.into()),
        }
    }
}

impl AnyOpcode {
    /// Gets the inner value of [`Self::Real`].
    #[must_use]
    pub const fn real(self) -> Option<Opcode> {
        match self {
            Self::Real(op) => Some(op),
            _ => None,
        }
    }

    /// Gets the inner value of [`Self::Pseudo`].
    #[must_use]
    pub const fn pseudo(self) -> Option<PseudoOpcode> {
        match self {
            Self::Pseudo(op) => Some(op),
            _ => None,
        }
    }

    /// Same as [`Self::real`] but panics if wasn't called on [`Self::Real`].
    ///
    /// # Panics
    ///
    /// If was called on something else other than [`Self::Real`].
    #[must_use]
    pub const fn expect_real(self) -> Opcode {
        self.real()
            .expect("Expected AnyOpcode::Real, found AnyOpcode::Pseudo")
    }

    /// Same as [`Self::pseudo`] but panics if wasn't called on [`Self::Pseudo`].
    ///
    /// # Panics
    ///
    /// If was called on something else other than [`Self::Pseudo`].
    #[must_use]
    pub const fn expect_pseudo(self) -> PseudoOpcode {
        self.pseudo()
            .expect("Expected AnyOpcode::Pseudo, found AnyOpcode::Real")
    }

    either_real_pseudo!(
        #[must_use]
        pub const fn has_arg(&self) -> bool
    );

    either_real_pseudo!(
        #[must_use]
        pub const fn has_jump(&self) -> bool
    );

    either_real_pseudo!(
        #[must_use]
        pub const fn has_free(&self) -> bool
    );

    either_real_pseudo!(
        #[must_use]
        pub const fn has_local(&self) -> bool
    );

    either_real_pseudo!(
        #[must_use]
        pub const fn has_name(&self) -> bool
    );

    either_real_pseudo!(
        #[must_use]
        pub const fn has_const(&self) -> bool
    );

    either_real_pseudo!(
        #[must_use]
        pub const fn is_instrumented(&self) -> bool
    );

    either_real_pseudo!(
        #[must_use]
        pub const fn is_block_push(&self) -> bool
    );

    either_real_pseudo!(
        #[must_use]
        pub fn stack_effect_jump(&self, oparg: u32) -> i32
    );

    either_real_pseudo!(
        #[must_use]
        pub fn stack_effect(&self, oparg: u32) -> i32
    );

    #[must_use]
    pub const fn deopt(&self) -> Option<Self> {
        match self {
            Self::Real(opcode) => {
                if let Some(op) = opcode.deopt() {
                    Some(Self::Real(op))
                } else {
                    None
                }
            }
            Self::Pseudo(opcode) => {
                if let Some(op) = opcode.deopt() {
                    Some(Self::Pseudo(op))
                } else {
                    None
                }
            }
        }
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
    #[must_use]
    pub const fn new(pushed: u32, popped: u32) -> Self {
        Self { pushed, popped }
    }

    /// Get the calculated stack effect as [`i32`].
    #[must_use]
    pub fn effect(self) -> i32 {
        self.into()
    }

    /// Get the pushed count.
    #[must_use]
    pub const fn pushed(self) -> u32 {
        self.pushed
    }

    /// Get the popped count.
    #[must_use]
    pub const fn popped(self) -> u32 {
        self.popped
    }
}

impl From<StackEffect> for i32 {
    fn from(effect: StackEffect) -> Self {
        (effect.pushed() as Self) - (effect.popped() as Self)
    }
}

#[derive(Copy, Clone)]
pub struct Arg<T: OpArgType>(PhantomData<T>);

impl<T: OpArgType> Arg<T> {
    #[inline]
    #[must_use]
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
    #[must_use]
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
    #[must_use]
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

// TODO: Can probably remove these asserts and remove the `repr($typ)` from the macro. but this
// breaks the VM:/
const _: () = assert!(core::mem::size_of::<Instruction>() == 1);
const _: () = assert!(core::mem::size_of::<PseudoInstruction>() == 2);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_break_flags_match_cpython_jump_metadata() {
        assert!(Opcode::JumpBackward.has_eval_break());
        assert!(!Opcode::JumpBackwardNoInterrupt.has_eval_break());
        assert!(!Opcode::JumpForward.has_eval_break());

        assert!(PseudoOpcode::Jump.has_eval_break());
        assert!(!PseudoOpcode::JumpIfFalse.has_eval_break());
        assert!(!PseudoOpcode::JumpIfTrue.has_eval_break());
        assert!(!PseudoOpcode::JumpNoInterrupt.has_eval_break());

        assert!(AnyInstruction::from(PseudoOpcode::Jump).has_eval_break());
    }

    #[test]
    fn terminator_flags_match_cpython_opcode_utils() {
        assert!(Opcode::JumpForward.is_terminator());
        assert!(Opcode::PopJumpIfFalse.is_terminator());
        assert!(Opcode::ForIter.is_terminator());
        assert!(Opcode::ReturnValue.is_terminator());
        assert!(!Opcode::Nop.is_terminator());

        assert!(PseudoOpcode::JumpIfTrue.is_terminator());
        assert!(PseudoOpcode::JumpNoInterrupt.is_terminator());
        assert!(!PseudoOpcode::SetupFinally.is_terminator());
        assert!(!PseudoOpcode::SetupWith.is_terminator());
        assert!(!PseudoOpcode::SetupCleanup.is_terminator());
        assert!(!PseudoOpcode::PopBlock.is_terminator());

        assert!(AnyInstruction::from(PseudoOpcode::JumpIfFalse).is_terminator());
    }

    #[test]
    fn assembler_flags_match_cpython_opcode_utils() {
        assert!(Opcode::JumpForward.is_assembler());
        assert!(Opcode::JumpBackward.is_assembler());
        assert!(Opcode::JumpBackwardNoInterrupt.is_assembler());
        assert!(!Opcode::PopJumpIfFalse.is_assembler());
        assert!(!Opcode::Nop.is_assembler());

        assert!(!PseudoOpcode::Jump.is_assembler());
        assert!(!PseudoOpcode::JumpNoInterrupt.is_assembler());
        assert!(!AnyInstruction::from(PseudoOpcode::Jump).is_assembler());
    }

    #[test]
    fn target_flags_match_cpython_opcode_utils() {
        assert!(Opcode::JumpForward.has_target());
        assert!(Opcode::ForIter.has_target());
        assert!(!Opcode::ReturnValue.has_target());
        assert!(!Opcode::Nop.has_target());

        assert!(PseudoOpcode::Jump.has_target());
        assert!(PseudoOpcode::SetupFinally.has_target());
        assert!(PseudoOpcode::SetupWith.has_target());
        assert!(PseudoOpcode::SetupCleanup.has_target());
        assert!(!PseudoOpcode::PopBlock.has_target());

        assert!(AnyInstruction::from(PseudoOpcode::SetupFinally).has_target());
    }

    #[test]
    fn arg_flags_match_cpython_opcode_metadata() {
        assert!(Opcode::LoadConst.has_arg());
        assert!(Opcode::YieldValue.has_arg());
        assert!(!Opcode::Nop.has_arg());
        assert!(!Opcode::ReturnValue.has_arg());

        assert!(PseudoOpcode::Jump.has_arg());
        assert!(PseudoOpcode::JumpIfFalse.has_arg());
        assert!(PseudoOpcode::JumpIfTrue.has_arg());
        assert!(PseudoOpcode::JumpNoInterrupt.has_arg());
        assert!(PseudoOpcode::LoadClosure.has_arg());
        assert!(PseudoOpcode::StoreFastMaybeNull.has_arg());
        assert!(!PseudoOpcode::AnnotationsPlaceholder.has_arg());
        assert!(!PseudoOpcode::PopBlock.has_arg());
    }

    #[test]
    fn const_flags_match_cpython_opcode_metadata() {
        assert!(Opcode::LoadConst.has_const());
        assert!(Opcode::LoadConstImmortal.has_const());
        assert!(Opcode::LoadConstMortal.has_const());
        assert!(!Opcode::LoadSmallInt.has_const());
        assert!(!Opcode::Nop.has_const());

        assert!(!PseudoOpcode::LoadClosure.has_const());
        assert!(!AnyInstruction::from(PseudoOpcode::Jump).has_const());
    }

    #[test]
    fn stack_effects_match_cpython_opcode_metadata() {
        assert_eq!(Opcode::ForIter.stack_effect_info(0).popped(), 1);
        assert_eq!(Opcode::ForIter.stack_effect_info(0).pushed(), 2);
        assert_eq!(Opcode::ForIter.stack_effect(0), 1);
        assert_eq!(Opcode::ForIter.stack_effect_jump(0), 1);

        assert_eq!(Opcode::EndAsyncFor.stack_effect_info(0).popped(), 2);
        assert_eq!(Opcode::EndAsyncFor.stack_effect_info(0).pushed(), 0);
        assert_eq!(Opcode::PopJumpIfFalse.stack_effect(0), -1);
        assert_eq!(Opcode::PopJumpIfFalse.stack_effect_jump(0), -1);

        assert_eq!(PseudoOpcode::SetupFinally.stack_effect_info(0).pushed(), 1);
        assert_eq!(PseudoOpcode::SetupFinally.stack_effect(0), 0);
        assert_eq!(PseudoOpcode::SetupFinally.stack_effect_jump(0), 1);
        assert_eq!(PseudoOpcode::SetupCleanup.stack_effect_info(0).pushed(), 2);
        assert_eq!(PseudoOpcode::SetupCleanup.stack_effect(0), 0);
        assert_eq!(PseudoOpcode::SetupCleanup.stack_effect_jump(0), 2);
    }

    #[test]
    fn no_fallthrough_flags_match_cpython_basicblock_nofallthrough() {
        assert!(Opcode::JumpForward.is_no_fallthrough());
        assert!(Opcode::ReturnValue.is_no_fallthrough());
        assert!(!Opcode::PopJumpIfFalse.is_no_fallthrough());
        assert!(!Opcode::ForIter.is_no_fallthrough());
        assert!(!Opcode::Nop.is_no_fallthrough());

        assert!(PseudoOpcode::Jump.is_no_fallthrough());
        assert!(PseudoOpcode::JumpNoInterrupt.is_no_fallthrough());
        assert!(!PseudoOpcode::JumpIfFalse.is_no_fallthrough());
        assert!(!PseudoOpcode::SetupFinally.is_no_fallthrough());
        assert!(!PseudoOpcode::SetupWith.is_no_fallthrough());

        assert!(AnyInstruction::from(PseudoOpcode::Jump).is_no_fallthrough());
    }
}
