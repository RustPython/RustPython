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
        #[repr($typ)]
        $opcode_vis enum $opcode_name {
            $($op_name = $op_id),*
        }

        impl $opcode_name {
            #[doc = concat!("Converts this opcode to [`", stringify!($instr_name), "`].")]
            #[must_use]
            #[inline]
            $opcode_vis const fn as_instruction(&self) -> $instr_name {
                // SAFETY: `$opcode_name` and `$instr_name` are both `#[repr($typ)]`
                // enums sharing identical explicit discriminants, and every
                // `$instr_name` payload field is the zero-sized `Arg<T>` marker
                // (see the `size_of` assertion near its definition), so both
                // enums have the same one-`$typ`-wide representation: just the
                // discriminant. Converting a live `$opcode_name` value therefore
                // yields the `$instr_name` variant with the matching discriminant.
                unsafe { core::mem::transmute(*self) }
            }

            /// Map a specialized or instrumented opcode back to its adaptive (base) variant.
            #[must_use]
            #[inline]
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
            // Kept as a match rather than a range check + transmute: `$op_id`
            // values are not contiguous (specialized/instrumented opcodes leave
            // gaps), so validity can't be expressed as a simple bound.
            pub(super) const fn try_from_numeric(value: $typ) -> Result<Self, $crate::marshal::MarshalError> {
                match value {
                    $($op_id => Ok(Self::$op_name),)*
                    _ => Err($crate::marshal::MarshalError::InvalidBytecode),
                }
            }

            // NOTE: Keep private. Will be exposed under `as_u8/as_u16`.
            #[must_use]
            #[inline]
            pub(super) const fn as_numeric(self) -> $typ {
                // `$opcode_name` is `#[repr($typ)]` with an explicit `$op_id`
                // discriminant on every variant, so this is a plain identity cast.
                self as $typ
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

        // Every `$instr_name` payload field is the zero-sized `Arg<T>` marker, so
        // (combined with the `#[repr($typ)]` above) each variant's representation
        // is exactly its `$typ` discriminant with no padding. `as_opcode` and
        // `$opcode_name::as_instruction` rely on this to convert via
        // `mem::transmute` instead of a per-variant match.
        const _: () = assert!(core::mem::size_of::<$instr_name>() == core::mem::size_of::<$typ>());

        impl $instr_name {
            #[doc = concat!("Get the corresponding [`", stringify!($opcode_name), "`].")]
            #[must_use]
            #[inline]
            $instr_vis const fn as_opcode(&self) -> $opcode_name {
                // SAFETY: symmetric to `$opcode_name::as_instruction` above:
                // `*self`'s representation is exactly its `$typ` discriminant
                // (checked by the `size_of` assertion above this impl), and that
                // discriminant is always a valid `$opcode_name` discriminant
                // because both enums share the same explicit `$op_id` list.
                unsafe { core::mem::transmute(*self) }
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

// Shared instructions use CPython 3.15 opcode IDs. RustPython-only instructions
// occupy unused slots 219–221 and retain their existing serialization behavior.
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
        GetIter {
            mode: Arg<u32>,
        } = 70,
        Reserved = 17,
        GetLen = 16,
        InterpreterExit = 18,
        LoadBuildClass = 19,
        LoadLocals = 20,
        MakeFunction = 21,
        MatchKeys = 22,
        MatchMapping = 23,
        MatchSequence = 24,
        Nop = 25,
        NotTaken = 26,
        PopExcept = 27,
        PopIter = 28,
        PopTop = 29,
        PushExcInfo = 30,
        PushNull = 31,
        ReturnGenerator = 32,
        ReturnValue = 33,
        SetupAnnotations = 34,
        StoreSlice = 35,
        StoreSubscr = 36,
        ToBool = 37,
        UnaryInvert = 38,
        UnaryNegative = 39,
        UnaryNot = 40,
        WithExceptStart = 41,
        BinaryOp {
            op: Arg<oparg::BinaryOperator>,
        } = 42,
        BuildInterpolation {
            format: Arg<u32>,
        } = 43,
        BuildList {
            count: Arg<u32>,
        } = 44,
        BuildMap {
            count: Arg<u32>,
        } = 45,
        BuildSet {
            count: Arg<u32>,
        } = 46,
        BuildSlice {
            argc: Arg<oparg::BuildSliceArgCount>,
        } = 47,
        BuildString {
            count: Arg<u32>,
        } = 48,
        BuildTuple {
            count: Arg<u32>,
        } = 49,
        Call {
            argc: Arg<u32>,
        } = 50,
        CallIntrinsic1 {
            func: Arg<oparg::IntrinsicFunction1>,
        } = 51,
        CallIntrinsic2 {
            func: Arg<oparg::IntrinsicFunction2>,
        } = 52,
        CallKw {
            argc: Arg<u32>,
        } = 53,
        CompareOp {
            opname: Arg<oparg::ComparisonOperator>,
        } = 54,
        ContainsOp {
            invert: Arg<oparg::Invert>,
        } = 55,
        ConvertValue {
            oparg: Arg<oparg::ConvertValueOparg>,
        } = 56,
        Copy {
            i: Arg<u32>,
        } = 57,
        CopyFreeVars {
            n: Arg<u32>,
        } = 58,
        DeleteAttr {
            namei: Arg<oparg::NameIdx>,
        } = 59,
        DeleteDeref {
            i: Arg<oparg::VarNum>,
        } = 60,
        DeleteFast {
            var_num: Arg<oparg::VarNum>,
        } = 61,
        DeleteGlobal {
            namei: Arg<oparg::NameIdx>,
        } = 62,
        DeleteName {
            namei: Arg<oparg::NameIdx>,
        } = 63,
        DictMerge {
            i: Arg<u32>,
        } = 64,
        DictUpdate {
            i: Arg<u32>,
        } = 65,
        EndAsyncFor = 66,
        ExtendedArg = 67,
        ForIter {
            delta: Arg<oparg::Label>,
        } = 68,
        GetAwaitable {
            r#where: Arg<u32>,
        } = 69,
        ImportFrom {
            namei: Arg<oparg::NameIdx>,
        } = 71,
        ImportName {
            // (co_names index << 2) | policy: 0 eligible, 1 lazy, 2 forced eager.
            namei: Arg<oparg::NameIdx>,
        } = 72,
        IsOp {
            invert: Arg<oparg::Invert>,
        } = 73,
        JumpBackward {
            delta: Arg<oparg::Label>,
        } = 74,
        JumpBackwardNoInterrupt {
            delta: Arg<oparg::Label>,
        } = 75,
        JumpForward {
            delta: Arg<oparg::Label>,
        } = 76,
        ListAppend {
            i: Arg<u32>,
        } = 77,
        ListExtend {
            i: Arg<u32>,
        } = 78,
        LoadAttr {
            namei: Arg<oparg::LoadAttr>,
        } = 79,
        LoadCommonConstant {
            idx: Arg<oparg::CommonConstant>,
        } = 80,
        LoadConst {
            consti: Arg<oparg::ConstIdx>,
        } = 81,
        LoadDeref {
            i: Arg<oparg::VarNum>,
        } = 82,
        LoadFast {
            var_num: Arg<oparg::VarNum>,
        } = 83,
        LoadFastAndClear {
            var_num: Arg<oparg::VarNum>,
        } = 84,
        LoadFastBorrow {
            var_num: Arg<oparg::VarNum>,
        } = 85,
        LoadFastBorrowLoadFastBorrow {
            var_nums: Arg<oparg::VarNums>,
        } = 86,
        LoadFastCheck {
            var_num: Arg<oparg::VarNum>,
        } = 87,
        LoadFastLoadFast {
            var_nums: Arg<oparg::VarNums>,
        } = 88,
        LoadFromDictOrDeref {
            i: Arg<oparg::VarNum>,
        } = 89,
        LoadFromDictOrGlobals {
            i: Arg<oparg::NameIdx>,
        } = 90,
        LoadGlobal {
            namei: Arg<oparg::NameIdx>,
        } = 91,
        LoadName {
            namei: Arg<oparg::NameIdx>,
        } = 92,
        LoadSmallInt {
            i: Arg<u32>,
        } = 93,
        LoadSpecial {
            method: Arg<oparg::SpecialMethod>,
        } = 94,
        LoadSuperAttr {
            namei: Arg<oparg::LoadSuperAttr>,
        } = 95,
        MakeCell {
            i: Arg<oparg::VarNum>,
        } = 96,
        MapAdd {
            i: Arg<u32>,
        } = 97,
        MatchClass {
            count: Arg<u32>,
        } = 98,
        PopJumpIfFalse {
            delta: Arg<oparg::Label>,
        } = 99,
        PopJumpIfNone {
            delta: Arg<oparg::Label>,
        } = 100,
        PopJumpIfNotNone {
            delta: Arg<oparg::Label>,
        } = 101,
        PopJumpIfTrue {
            delta: Arg<oparg::Label>,
        } = 102,
        RaiseVarargs {
            argc: Arg<oparg::RaiseKind>,
        } = 103,
        Reraise {
            depth: Arg<u32>,
        } = 104,
        Send {
            delta: Arg<oparg::Label>,
        } = 105,
        SetAdd {
            i: Arg<u32>,
        } = 106,
        SetFunctionAttribute {
            flag: Arg<oparg::MakeFunctionFlag>,
        } = 107,
        SetUpdate {
            i: Arg<u32>,
        } = 108,
        StoreAttr {
            namei: Arg<oparg::NameIdx>,
        } = 109,
        StoreDeref {
            i: Arg<oparg::VarNum>,
        } = 110,
        StoreFast {
            var_num: Arg<oparg::VarNum>,
        } = 111,
        StoreFastLoadFast {
            var_nums: Arg<oparg::VarNums>,
        } = 112,
        StoreFastStoreFast {
            var_nums: Arg<oparg::VarNums>,
        } = 113,
        StoreGlobal {
            namei: Arg<oparg::NameIdx>,
        } = 114,
        StoreName {
            namei: Arg<oparg::NameIdx>,
        } = 115,
        Swap {
            i: Arg<u32>,
        } = 116,
        UnpackEx {
            counts: Arg<oparg::UnpackExArgs>,
        } = 117,
        UnpackSequence {
            count: Arg<u32>,
        } = 118,
        YieldValue {
            arg: Arg<u32>,
        } = 119,
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
        BinaryOpSubtractFloat = 142,
        BinaryOpSubtractInt = 143,
        CallAllocAndEnterInit = 144,
        CallBoundMethodExactArgs = 145,
        CallBoundMethodGeneral = 146,
        CallBuiltinClass = 147,
        CallBuiltinFast = 148,
        CallBuiltinFastWithKeywords = 149,
        CallBuiltinO = 150,
        CallIsinstance = 153,
        CallKwBoundMethod = 154,
        CallKwNonPy = 155,
        CallKwPy = 156,
        CallLen = 157,
        CallListAppend = 158,
        CallMethodDescriptorFast = 159,
        CallMethodDescriptorFastWithKeywords = 160,
        CallMethodDescriptorNoargs = 161,
        CallMethodDescriptorO = 162,
        CallNonPyGeneral = 163,
        CallPyExactArgs = 164,
        CallPyGeneral = 165,
        CallStr1 = 166,
        CallTuple1 = 167,
        CallType1 = 168,
        CompareOpFloat = 169,
        CompareOpInt = 170,
        CompareOpStr = 171,
        ContainsOpDict = 172,
        ContainsOpSet = 173,
        ForIterGen = 174,
        ForIterList = 175,
        ForIterRange = 176,
        ForIterTuple = 177,
        JumpBackwardJit = 181,
        JumpBackwardNoJit = 182,
        LoadAttrClass = 183,
        LoadAttrClassWithMetaclassCheck = 184,
        LoadAttrGetattributeOverridden = 185,
        LoadAttrInstanceValue = 186,
        LoadAttrMethodLazyDict = 187,
        LoadAttrMethodNoDict = 188,
        LoadAttrMethodWithValues = 189,
        LoadAttrModule = 190,
        LoadAttrNondescriptorNoDict = 191,
        LoadAttrNondescriptorWithValues = 192,
        LoadAttrProperty = 193,
        LoadAttrSlot = 194,
        LoadAttrWithHint = 195,
        LoadGlobalBuiltin = 196,
        LoadGlobalModule = 197,
        LoadSuperAttrAttr = 198,
        LoadSuperAttrMethod = 199,
        ResumeCheck = 200,
        SendGen = 203,
        StoreAttrInstanceValue = 205,
        StoreAttrSlot = 206,
        StoreAttrWithHint = 207,
        StoreSubscrDict = 208,
        StoreSubscrListInt = 209,
        ToBoolAlwaysTrue = 210,
        ToBoolBool = 211,
        ToBoolInt = 212,
        ToBoolList = 213,
        ToBoolNone = 214,
        ToBoolStr = 215,
        UnpackSequenceList = 216,
        UnpackSequenceTuple = 217,
        UnpackSequenceTwoTuple = 218,
        GetYieldFromIter = 219,
        LoadConstImmortal = 220,
        LoadConstMortal = 221,
        InstrumentedEndFor = 233,
        InstrumentedPopIter = 234,
        InstrumentedEndSend = 235,
        InstrumentedForIter = 236,
        InstrumentedInstruction = 237,
        InstrumentedJumpForward = 238,
        InstrumentedNotTaken = 239,
        InstrumentedPopJumpIfTrue = 240,
        InstrumentedPopJumpIfFalse = 241,
        InstrumentedPopJumpIfNone = 242,
        InstrumentedPopJumpIfNotNone = 243,
        InstrumentedResume = 244,
        InstrumentedReturnValue = 245,
        InstrumentedYieldValue = 246,
        InstrumentedEndAsyncFor = 247,
        InstrumentedLoadSuperAttr = 248,
        InstrumentedCall = 249,
        InstrumentedCallKw = 250,
        InstrumentedCallFunctionEx = 251,
        InstrumentedJumpBackward = 252,
        InstrumentedLine = 253,
        EnterExecutor = 254,
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
        assert!(PseudoOpcode::SetupCleanup.has_arg());
        assert!(PseudoOpcode::SetupFinally.has_arg());
        assert!(PseudoOpcode::SetupWith.has_arg());
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
        assert_eq!(Opcode::ForIter.stack_effect_info(0).popped(), 2);
        assert_eq!(Opcode::ForIter.stack_effect_info(0).pushed(), 3);
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

    /// Snapshot of the chained `match` implementations that `Opcode::deopt`
    /// and `Opcode::cache_entries` used before they were rewritten as table
    /// lookups. Exists only to pin the observable behavior of the table
    /// lookups against the logic they replaced.
    mod reference {
        use super::Opcode;

        pub(super) const fn deopt(op: Opcode) -> Option<Opcode> {
            Some(match op {
                Opcode::ResumeCheck => Opcode::Resume,
                Opcode::LoadConstMortal | Opcode::LoadConstImmortal => Opcode::LoadConst,
                Opcode::ToBoolAlwaysTrue
                | Opcode::ToBoolBool
                | Opcode::ToBoolInt
                | Opcode::ToBoolList
                | Opcode::ToBoolNone
                | Opcode::ToBoolStr => Opcode::ToBool,
                Opcode::BinaryOpMultiplyInt
                | Opcode::BinaryOpAddInt
                | Opcode::BinaryOpSubtractInt
                | Opcode::BinaryOpMultiplyFloat
                | Opcode::BinaryOpAddFloat
                | Opcode::BinaryOpSubtractFloat
                | Opcode::BinaryOpAddUnicode
                | Opcode::BinaryOpSubscrListInt
                | Opcode::BinaryOpSubscrListSlice
                | Opcode::BinaryOpSubscrTupleInt
                | Opcode::BinaryOpSubscrStrInt
                | Opcode::BinaryOpSubscrDict
                | Opcode::BinaryOpSubscrGetitem
                | Opcode::BinaryOpExtend
                | Opcode::BinaryOpInplaceAddUnicode => Opcode::BinaryOp,
                Opcode::StoreSubscrDict | Opcode::StoreSubscrListInt => Opcode::StoreSubscr,
                Opcode::SendGen => Opcode::Send,
                Opcode::UnpackSequenceTwoTuple
                | Opcode::UnpackSequenceTuple
                | Opcode::UnpackSequenceList => Opcode::UnpackSequence,
                Opcode::StoreAttrInstanceValue
                | Opcode::StoreAttrSlot
                | Opcode::StoreAttrWithHint => Opcode::StoreAttr,
                Opcode::LoadGlobalModule | Opcode::LoadGlobalBuiltin => Opcode::LoadGlobal,
                Opcode::LoadSuperAttrAttr | Opcode::LoadSuperAttrMethod => Opcode::LoadSuperAttr,
                Opcode::LoadAttrInstanceValue
                | Opcode::LoadAttrModule
                | Opcode::LoadAttrWithHint
                | Opcode::LoadAttrSlot
                | Opcode::LoadAttrClass
                | Opcode::LoadAttrClassWithMetaclassCheck
                | Opcode::LoadAttrProperty
                | Opcode::LoadAttrGetattributeOverridden
                | Opcode::LoadAttrMethodWithValues
                | Opcode::LoadAttrMethodNoDict
                | Opcode::LoadAttrMethodLazyDict
                | Opcode::LoadAttrNondescriptorWithValues
                | Opcode::LoadAttrNondescriptorNoDict => Opcode::LoadAttr,
                Opcode::CompareOpFloat | Opcode::CompareOpInt | Opcode::CompareOpStr => {
                    Opcode::CompareOp
                }
                Opcode::ContainsOpSet | Opcode::ContainsOpDict => Opcode::ContainsOp,
                Opcode::JumpBackwardNoJit | Opcode::JumpBackwardJit => Opcode::JumpBackward,
                Opcode::ForIterList
                | Opcode::ForIterTuple
                | Opcode::ForIterRange
                | Opcode::ForIterGen => Opcode::ForIter,
                Opcode::CallBoundMethodExactArgs
                | Opcode::CallPyExactArgs
                | Opcode::CallType1
                | Opcode::CallStr1
                | Opcode::CallTuple1
                | Opcode::CallBuiltinClass
                | Opcode::CallBuiltinO
                | Opcode::CallBuiltinFast
                | Opcode::CallBuiltinFastWithKeywords
                | Opcode::CallLen
                | Opcode::CallIsinstance
                | Opcode::CallListAppend
                | Opcode::CallMethodDescriptorO
                | Opcode::CallMethodDescriptorFastWithKeywords
                | Opcode::CallMethodDescriptorNoargs
                | Opcode::CallMethodDescriptorFast
                | Opcode::CallAllocAndEnterInit
                | Opcode::CallPyGeneral
                | Opcode::CallBoundMethodGeneral
                | Opcode::CallNonPyGeneral => Opcode::Call,
                Opcode::CallKwBoundMethod | Opcode::CallKwPy | Opcode::CallKwNonPy => {
                    Opcode::CallKw
                }
                _ => return None,
            })
        }

        pub(super) const fn deoptimize(op: Opcode) -> Opcode {
            match deopt(op) {
                Some(v) => v,
                None => match op.to_base() {
                    Some(v) => v,
                    None => op,
                },
            }
        }

        pub(super) const fn cache_entries(op: Opcode) -> usize {
            match deoptimize(op) {
                Opcode::Resume | Opcode::GetIter | Opcode::CallFunctionEx => 1,
                Opcode::StoreSubscr => 1,
                Opcode::ToBool => 3,
                Opcode::BinaryOp => 5,
                Opcode::Call => 3,
                Opcode::CallKw => 3,
                Opcode::CompareOp => 1,
                Opcode::ContainsOp => 1,
                Opcode::ForIter => 1,
                Opcode::JumpBackward => 1,
                Opcode::LoadAttr => 9,
                Opcode::LoadGlobal => 4,
                Opcode::LoadSuperAttr => 1,
                Opcode::PopJumpIfFalse => 1,
                Opcode::PopJumpIfNone => 1,
                Opcode::PopJumpIfNotNone => 1,
                Opcode::PopJumpIfTrue => 1,
                Opcode::Send => 1,
                Opcode::StoreAttr => 4,
                Opcode::UnpackSequence => 1,
                _ => 0,
            }
        }
    }

    #[test]
    fn cache_entries_and_deopt_tables_match_reference_impl() {
        let mut checked = 0;
        for byte in 0u8..=255 {
            let Ok(op) = Opcode::try_from_u8(byte) else {
                continue;
            };

            assert_eq!(
                op.deopt(),
                reference::deopt(op),
                "deopt() mismatch for {op:?}"
            );
            assert_eq!(
                op.cache_entries(),
                reference::cache_entries(op),
                "cache_entries() mismatch for {op:?}"
            );
            checked += 1;
        }

        // Sanity check that the loop actually exercised opcodes rather than
        // silently skipping all of them.
        assert!(checked > 200);
    }

    /// `Opcode::as_numeric`, `Opcode::as_instruction` and
    /// `Instruction::as_opcode` used to be per-variant matches; they are now
    /// an identity cast and two `mem::transmute`s respectively. `byte` (an
    /// input independent of any of those three functions) together with the
    /// untouched `try_from_u8`/`TryFrom<u8>` conversions serve as the
    /// reference: every opcode reachable from a byte must convert back to
    /// that exact byte and round-trip through `Instruction`.
    #[test]
    fn opcode_instruction_numeric_conversions_match_try_from_numeric() {
        let mut checked = 0;
        for byte in 0u8..=255 {
            let Ok(op) = Opcode::try_from_u8(byte) else {
                continue;
            };

            assert_eq!(op.as_numeric(), byte, "as_numeric() mismatch for {op:?}");

            let instr = op.as_instruction();
            assert_eq!(
                instr.as_opcode(),
                op,
                "as_instruction()/as_opcode() round trip mismatch for {op:?}"
            );

            let instr_via_try_from = Instruction::try_from(byte).unwrap();
            assert_eq!(
                instr_via_try_from.as_opcode(),
                op,
                "Instruction::try_from({byte}) mismatch"
            );

            checked += 1;
        }

        assert!(checked > 200);
    }

    /// Same as [`opcode_instruction_numeric_conversions_match_try_from_numeric`]
    /// but for the `u16`-discriminant pseudo-opcode instantiation of
    /// `define_opcodes!`.
    #[test]
    fn pseudo_opcode_instruction_numeric_conversions_match_try_from_numeric() {
        let mut checked = 0;
        for value in 0u16..=u16::MAX {
            let Ok(op) = PseudoOpcode::try_from_u16(value) else {
                continue;
            };

            assert_eq!(op.as_numeric(), value, "as_numeric() mismatch for {op:?}");

            let instr = op.as_instruction();
            assert_eq!(
                instr.as_opcode(),
                op,
                "as_instruction()/as_opcode() round trip mismatch for {op:?}"
            );

            let instr_via_try_from = PseudoInstruction::try_from(value).unwrap();
            assert_eq!(
                instr_via_try_from.as_opcode(),
                op,
                "PseudoInstruction::try_from({value}) mismatch"
            );

            checked += 1;
        }

        // All 11 `PseudoInstruction` variants should have been exercised.
        assert_eq!(checked, 11);
    }
}
