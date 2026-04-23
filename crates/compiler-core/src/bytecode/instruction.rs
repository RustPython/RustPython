use core::{fmt, marker::PhantomData};

use crate::{
    bytecode::oparg::{
        self, BinaryOperator, BuildSliceArgCount, CommonConstant, ComparisonOperator,
        ConvertValueOparg, IntrinsicFunction1, IntrinsicFunction2, Invert, Label, LoadAttr,
        LoadSuperAttr, MakeFunctionFlag, NameIdx, OpArg, OpArgByte, OpArgType, RaiseKind,
        SpecialMethod, UnpackExArgs,
    },
    marshal::MarshalError,
};

macro_rules! define_opcodes {
    (
        #[repr($typ:ident)]
        $opcode_vis:vis enum $opcode_name:ident;

        $(#[$instr_meta:meta])*
        $instr_vis:vis enum $instr_name:ident {
            $(
                $(#[$op_meta:meta])*
                    $op_name:ident $({ $arg_name:ident: Arg<$arg_type:ty> $(,)? })? = ($op_id:expr, $op_display:literal)
            ),* $(,)?
        }
    ) => {
        #[derive(Clone, Copy, Debug)]
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

            /// Gets the CPython name representation.
            #[must_use]
            $opcode_vis const fn name(&self) -> &str {
                match self {
                    $(Self::$op_name => $op_display,)*
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
                match value {
                    $($op_id => Ok(Self::$op_name),)*
                    _ => Err(Self::Error::InvalidBytecode),
                }
            }
        }

        impl From<$opcode_name> for $typ {
            fn from(opcode: $opcode_name) -> Self {
                match opcode {
                    $($opcode_name::$op_name => $op_id,)*
                }
            }
        }

        impl ::core::fmt::Display for $opcode_name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                write!(f, "{}", self.name())
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
            $instr_vis const fn opcode(&self) -> $opcode_name {
                match self {
                    $(
                        Self::$op_name $({ $arg_name: _ })? => $opcode_name::$op_name,
                    )*
                }
            }

        }

        impl From<$instr_name> for $opcode_name {
            fn from(instr: $instr_name) -> Self {
                instr.opcode()
            }
        }

        impl TryFrom<$typ> for $instr_name {
            type Error = $crate::marshal::MarshalError;

            fn try_from(value: $typ) -> Result<Self, Self::Error> {
                $opcode_name::try_from(value).map(Into::into)
            }
        }

        impl From<$instr_name> for $typ {
            fn from(instr: $instr_name) -> Self {
                instr.opcode().into()
            }
        }
    };
}

define_opcodes!(
    #[repr(u8)]
    pub enum Opcode;

    pub enum Instruction {
        Cache = (0, "CACHE"),
        BinarySlice = (1, "BINARY_SLICE"),
        BuildTemplate = (2, "BUILD_TEMPLATE"),
        BinaryOpInplaceAddUnicode = (3, "BINARY_OP_INPLACE_ADD_UNICODE"),
        CallFunctionEx = (4, "CALL_FUNCTION_EX"),
        CheckEgMatch = (5, "CHECK_EG_MATCH"),
        CheckExcMatch = (6, "CHECK_EXC_MATCH"),
        CleanupThrow = (7, "CLEANUP_THROW"),
        DeleteSubscr = (8, "DELETE_SUBSCR"),
        EndFor = (9, "END_FOR"),
        EndSend = (10, "END_SEND"),
        ExitInitCheck = (11, "EXIT_INIT_CHECK"),
        FormatSimple = (12, "FORMAT_SIMPLE"),
        FormatWithSpec = (13, "FORMAT_WITH_SPEC"),
        GetAIter = (14, "GET_AITER"),
        GetANext = (15, "GET_ANEXT"),
        GetIter = (16, "GET_ITER"),
        Reserved = (17, "RESERVED"),
        GetLen = (18, "GET_LEN"),
        GetYieldFromIter = (19, "GET_YIELD_FROM_ITER"),
        InterpreterExit = (20, "INTERPRETER_EXIT"),
        LoadBuildClass = (21, "LOAD_BUILD_CLASS"),
        LoadLocals = (22, "LOAD_LOCALS"),
        MakeFunction = (23, "MAKE_FUNCTION"),
        MatchKeys = (24, "MATCH_KEYS"),
        MatchMapping = (25, "MATCH_MAPPING"),
        MatchSequence = (26, "MATCH_SEQUENCE"),
        Nop = (27, "NOP"),
        NotTaken = (28, "NOT_TAKEN"),
        PopExcept = (29, "POP_EXCEPT"),
        PopIter = (30, "POP_ITER"),
        PopTop = (31, "POP_TOP"),
        PushExcInfo = (32, "PUSH_EXC_INFO"),
        PushNull = (33, "PUSH_NULL"),
        ReturnGenerator = (34, "RETURN_GENERATOR"),
        ReturnValue = (35, "RETURN_VALUE"),
        SetupAnnotations = (36, "SETUP_ANNOTATIONS"),
        StoreSlice = (37, "STORE_SLICE"),
        StoreSubscr = (38, "STORE_SUBSCR"),
        ToBool = (39, "TO_BOOL"),
        UnaryInvert = (40, "UNARY_INVERT"),
        UnaryNegative = (41, "UNARY_NEGATIVE"),
        UnaryNot = (42, "UNARY_NOT"),
        WithExceptStart = (43, "WITH_EXCEPT_START"),
        BinaryOp {
            op: Arg<BinaryOperator>,
        } = (44, "BINARY_OP"),
        BuildInterpolation {
            format: Arg<u32>,
        } = (45, "BUILD_INTERPOLATION"),
        BuildList {
            count: Arg<u32>,
        } = (46, "BUILD_LIST"),
        BuildMap {
            count: Arg<u32>,
        } = (47, "BUILD_MAP"),
        BuildSet {
            count: Arg<u32>,
        } = (48, "BUILD_SET"),
        BuildSlice {
            argc: Arg<BuildSliceArgCount>,
        } = (49, "BUILD_SLICE"),
        BuildString {
            count: Arg<u32>,
        } = (50, "BUILD_STRING"),
        BuildTuple {
            count: Arg<u32>,
        } = (51, "BUILD_TUPLE"),
        Call {
            argc: Arg<u32>,
        } = (52, "CALL"),
        CallIntrinsic1 {
            func: Arg<IntrinsicFunction1>,
        } = (53, "CALL_INTRINSIC_1"),
        CallIntrinsic2 {
            func: Arg<IntrinsicFunction2>,
        } = (54, "CALL_INTRINSIC_2"),
        CallKw {
            argc: Arg<u32>,
        } = (55, "CALL_KW"),
        CompareOp {
            opname: Arg<ComparisonOperator>,
        } = (56, "COMPARE_OP"),
        ContainsOp {
            invert: Arg<Invert>,
        } = (57, "CONTAINS_OP"),
        ConvertValue {
            oparg: Arg<ConvertValueOparg>,
        } = (58, "CONVERT_VALUE"),
        Copy {
            i: Arg<u32>,
        } = (59, "COPY"),
        CopyFreeVars {
            n: Arg<u32>,
        } = (60, "COPY_FREE_VARS"),
        DeleteAttr {
            namei: Arg<NameIdx>,
        } = (61, "DELETE_ATTR"),
        DeleteDeref {
            i: Arg<oparg::VarNum>,
        } = (62, "DELETE_DEREF"),
        DeleteFast {
            var_num: Arg<oparg::VarNum>,
        } = (63, "DELETE_FAST"),
        DeleteGlobal {
            namei: Arg<NameIdx>,
        } = (64, "DELETE_GLOBAL"),
        DeleteName {
            namei: Arg<NameIdx>,
        } = (65, "DELETE_NAME"),
        DictMerge {
            i: Arg<u32>,
        } = (66, "DICT_MERGE"),
        DictUpdate {
            i: Arg<u32>,
        } = (67, "DICT_UPDATE"),
        EndAsyncFor = (68, "END_ASYNC_FOR"),
        ExtendedArg = (69, "EXTENDED_ARG"),
        ForIter {
            delta: Arg<Label>,
        } = (70, "FOR_ITER"),
        GetAwaitable {
            r#where: Arg<u32>,
        } = (71, "GET_AWAITABLE"),
        ImportFrom {
            namei: Arg<NameIdx>,
        } = (72, "IMPORT_FROM"),
        ImportName {
            namei: Arg<NameIdx>,
        } = (73, "IMPORT_NAME"),
        IsOp {
            invert: Arg<Invert>,
        } = (74, "IS_OP"),
        JumpBackward {
            delta: Arg<Label>,
        } = (75, "JUMP_BACKWARD"),
        JumpBackwardNoInterrupt {
            delta: Arg<Label>,
        } = (76, "JUMP_BACKWARD_NO_INTERRUPT"),
        JumpForward {
            delta: Arg<Label>,
        } = (77, "JUMP_FORWARD"),
        ListAppend {
            i: Arg<u32>,
        } = (78, "LIST_APPEND"),
        ListExtend {
            i: Arg<u32>,
        } = (79, "LIST_EXTEND"),
        LoadAttr {
            namei: Arg<LoadAttr>,
        } = (80, "LOAD_ATTR"),
        LoadCommonConstant {
            idx: Arg<CommonConstant>,
        } = (81, "LOAD_COMMON_CONSTANT"),
        LoadConst {
            consti: Arg<oparg::ConstIdx>,
        } = (82, "LOAD_CONST"),
        LoadDeref {
            i: Arg<oparg::VarNum>,
        } = (83, "LOAD_DEREF"),
        LoadFast {
            var_num: Arg<oparg::VarNum>,
        } = (84, "LOAD_FAST"),
        LoadFastAndClear {
            var_num: Arg<oparg::VarNum>,
        } = (85, "LOAD_FAST_AND_CLEAR"),
        LoadFastBorrow {
            var_num: Arg<oparg::VarNum>,
        } = (86, "LOAD_FAST_BORROW"),
        LoadFastBorrowLoadFastBorrow {
            var_nums: Arg<oparg::VarNums>,
        } = (87, "LOAD_FAST_BORROW_LOAD_FAST_BORROW"),
        LoadFastCheck {
            var_num: Arg<oparg::VarNum>,
        } = (88, "LOAD_FAST_CHECK"),
        LoadFastLoadFast {
            var_nums: Arg<oparg::VarNums>,
        } = (89, "LOAD_FAST_LOAD_FAST"),
        LoadFromDictOrDeref {
            i: Arg<oparg::VarNum>,
        } = (90, "LOAD_FROM_DICT_OR_DEREF"),
        LoadFromDictOrGlobals {
            i: Arg<NameIdx>,
        } = (91, "LOAD_FROM_DICT_OR_GLOBALS"),
        LoadGlobal {
            namei: Arg<NameIdx>,
        } = (92, "LOAD_GLOBAL"),
        LoadName {
            namei: Arg<NameIdx>,
        } = (93, "LOAD_NAME"),
        LoadSmallInt {
            i: Arg<u32>,
        } = (94, "LOAD_SMALL_INT"),
        LoadSpecial {
            method: Arg<SpecialMethod>,
        } = (95, "LOAD_SPECIAL"),
        LoadSuperAttr {
            namei: Arg<LoadSuperAttr>,
        } = (96, "LOAD_SUPER_ATTR"),
        MakeCell {
            i: Arg<oparg::VarNum>,
        } = (97, "MAKE_CELL"),
        MapAdd {
            i: Arg<u32>,
        } = (98, "MAP_ADD"),
        MatchClass {
            count: Arg<u32>,
        } = (99, "MATCH_CLASS"),
        PopJumpIfFalse {
            delta: Arg<Label>,
        } = (100, "POP_JUMP_IF_FALSE"),
        PopJumpIfNone {
            delta: Arg<Label>,
        } = (101, "POP_JUMP_IF_NONE"),
        PopJumpIfNotNone {
            delta: Arg<Label>,
        } = (102, "POP_JUMP_IF_NOT_NONE"),
        PopJumpIfTrue {
            delta: Arg<Label>,
        } = (103, "POP_JUMP_IF_TRUE"),
        RaiseVarargs {
            argc: Arg<RaiseKind>,
        } = (104, "RAISE_VARARGS"),
        Reraise {
            depth: Arg<u32>,
        } = (105, "RERAISE"),
        Send {
            delta: Arg<Label>,
        } = (106, "SEND"),
        SetAdd {
            i: Arg<u32>,
        } = (107, "SET_ADD"),
        SetFunctionAttribute {
            flag: Arg<MakeFunctionFlag>,
        } = (108, "SET_FUNCTION_ATTRIBUTE"),
        SetUpdate {
            i: Arg<u32>,
        } = (109, "SET_UPDATE"),
        StoreAttr {
            namei: Arg<NameIdx>,
        } = (110, "STORE_ATTR"),
        StoreDeref {
            i: Arg<oparg::VarNum>,
        } = (111, "STORE_DEREF"),
        StoreFast {
            var_num: Arg<oparg::VarNum>,
        } = (112, "STORE_FAST"),
        StoreFastLoadFast {
            var_nums: Arg<oparg::VarNums>,
        } = (113, "STORE_FAST_LOAD_FAST"),
        StoreFastStoreFast {
            var_nums: Arg<oparg::VarNums>,
        } = (114, "STORE_FAST_STORE_FAST"),
        StoreGlobal {
            namei: Arg<NameIdx>,
        } = (115, "STORE_GLOBAL"),
        StoreName {
            namei: Arg<NameIdx>,
        } = (116, "STORE_NAME"),
        Swap {
            i: Arg<u32>,
        } = (117, "SWAP"),
        UnpackEx {
            counts: Arg<UnpackExArgs>,
        } = (118, "UNPACK_EX"),
        UnpackSequence {
            count: Arg<u32>,
        } = (119, "UNPACK_SEQUENCE"),
        YieldValue {
            arg: Arg<u32>,
        } = (120, "YIELD_VALUE"),
        Resume {
            context: Arg<oparg::ResumeContext>,
        } = (128, "RESUME"),
        BinaryOpAddFloat = (129, "BINARY_OP_ADD_FLOAT"),
        BinaryOpAddInt = (130, "BINARY_OP_ADD_INT"),
        BinaryOpAddUnicode = (131, "BINARY_OP_ADD_UNICODE"),
        BinaryOpExtend = (132, "BINARY_OP_EXTEND"),
        BinaryOpMultiplyFloat = (133, "BINARY_OP_MULTIPLY_FLOAT"),
        BinaryOpMultiplyInt = (134, "BINARY_OP_MULTIPLY_INT"),
        BinaryOpSubscrDict = (135, "BINARY_OP_SUBSCR_DICT"),
        BinaryOpSubscrGetitem = (136, "BINARY_OP_SUBSCR_GETITEM"),
        BinaryOpSubscrListInt = (137, "BINARY_OP_SUBSCR_LIST_INT"),
        BinaryOpSubscrListSlice = (138, "BINARY_OP_SUBSCR_LIST_SLICE"),
        BinaryOpSubscrStrInt = (139, "BINARY_OP_SUBSCR_STR_INT"),
        BinaryOpSubscrTupleInt = (140, "BINARY_OP_SUBSCR_TUPLE_INT"),
        BinaryOpSubtractFloat = (141, "BINARY_OP_SUBTRACT_FLOAT"),
        BinaryOpSubtractInt = (142, "BINARY_OP_SUBTRACT_INT"),
        CallAllocAndEnterInit = (143, "CALL_ALLOC_AND_ENTER_INIT"),
        CallBoundMethodExactArgs = (144, "CALL_BOUND_METHOD_EXACT_ARGS"),
        CallBoundMethodGeneral = (145, "CALL_BOUND_METHOD_GENERAL"),
        CallBuiltinClass = (146, "CALL_BUILTIN_CLASS"),
        CallBuiltinFast = (147, "CALL_BUILTIN_FAST"),
        CallBuiltinFastWithKeywords = (148, "CALL_BUILTIN_FAST_WITH_KEYWORDS"),
        CallBuiltinO = (149, "CALL_BUILTIN_O"),
        CallIsinstance = (150, "CALL_ISINSTANCE"),
        CallKwBoundMethod = (151, "CALL_KW_BOUND_METHOD"),
        CallKwNonPy = (152, "CALL_KW_NON_PY"),
        CallKwPy = (153, "CALL_KW_PY"),
        CallLen = (154, "CALL_LEN"),
        CallListAppend = (155, "CALL_LIST_APPEND"),
        CallMethodDescriptorFast = (156, "CALL_METHOD_DESCRIPTOR_FAST"),
        CallMethodDescriptorFastWithKeywords = (157, "CALL_METHOD_DESCRIPTOR_FAST_WITH_KEYWORDS"),
        CallMethodDescriptorNoargs = (158, "CALL_METHOD_DESCRIPTOR_NOARGS"),
        CallMethodDescriptorO = (159, "CALL_METHOD_DESCRIPTOR_O"),
        CallNonPyGeneral = (160, "CALL_NON_PY_GENERAL"),
        CallPyExactArgs = (161, "CALL_PY_EXACT_ARGS"),
        CallPyGeneral = (162, "CALL_PY_GENERAL"),
        CallStr1 = (163, "CALL_STR_1"),
        CallTuple1 = (164, "CALL_TUPLE_1"),
        CallType1 = (165, "CALL_TYPE_1"),
        CompareOpFloat = (166, "COMPARE_OP_FLOAT"),
        CompareOpInt = (167, "COMPARE_OP_INT"),
        CompareOpStr = (168, "COMPARE_OP_STR"),
        ContainsOpDict = (169, "CONTAINS_OP_DICT"),
        ContainsOpSet = (170, "CONTAINS_OP_SET"),
        ForIterGen = (171, "FOR_ITER_GEN"),
        ForIterList = (172, "FOR_ITER_LIST"),
        ForIterRange = (173, "FOR_ITER_RANGE"),
        ForIterTuple = (174, "FOR_ITER_TUPLE"),
        JumpBackwardJit = (175, "JUMP_BACKWARD_JIT"),
        JumpBackwardNoJit = (176, "JUMP_BACKWARD_NO_JIT"),
        LoadAttrClass = (177, "LOAD_ATTR_CLASS"),
        LoadAttrClassWithMetaclassCheck = (178, "LOAD_ATTR_CLASS_WITH_METACLASS_CHECK"),
        LoadAttrGetattributeOverridden = (179, "LOAD_ATTR_GETATTRIBUTE_OVERRIDDEN"),
        LoadAttrInstanceValue = (180, "LOAD_ATTR_INSTANCE_VALUE"),
        LoadAttrMethodLazyDict = (181, "LOAD_ATTR_METHOD_LAZY_DICT"),
        LoadAttrMethodNoDict = (182, "LOAD_ATTR_METHOD_NO_DICT"),
        LoadAttrMethodWithValues = (183, "LOAD_ATTR_METHOD_WITH_VALUES"),
        LoadAttrModule = (184, "LOAD_ATTR_MODULE"),
        LoadAttrNondescriptorNoDict = (185, "LOAD_ATTR_NONDESCRIPTOR_NO_DICT"),
        LoadAttrNondescriptorWithValues = (186, "LOAD_ATTR_NONDESCRIPTOR_WITH_VALUES"),
        LoadAttrProperty = (187, "LOAD_ATTR_PROPERTY"),
        LoadAttrSlot = (188, "LOAD_ATTR_SLOT"),
        LoadAttrWithHint = (189, "LOAD_ATTR_WITH_HINT"),
        LoadConstImmortal = (190, "LOAD_CONST_IMMORTAL"),
        LoadConstMortal = (191, "LOAD_CONST_MORTAL"),
        LoadGlobalBuiltin = (192, "LOAD_GLOBAL_BUILTIN"),
        LoadGlobalModule = (193, "LOAD_GLOBAL_MODULE"),
        LoadSuperAttrAttr = (194, "LOAD_SUPER_ATTR_ATTR"),
        LoadSuperAttrMethod = (195, "LOAD_SUPER_ATTR_METHOD"),
        ResumeCheck = (196, "RESUME_CHECK"),
        SendGen = (197, "SEND_GEN"),
        StoreAttrInstanceValue = (198, "STORE_ATTR_INSTANCE_VALUE"),
        StoreAttrSlot = (199, "STORE_ATTR_SLOT"),
        StoreAttrWithHint = (200, "STORE_ATTR_WITH_HINT"),
        StoreSubscrDict = (201, "STORE_SUBSCR_DICT"),
        StoreSubscrListInt = (202, "STORE_SUBSCR_LIST_INT"),
        ToBoolAlwaysTrue = (203, "TO_BOOL_ALWAYS_TRUE"),
        ToBoolBool = (204, "TO_BOOL_BOOL"),
        ToBoolInt = (205, "TO_BOOL_INT"),
        ToBoolList = (206, "TO_BOOL_LIST"),
        ToBoolNone = (207, "TO_BOOL_NONE"),
        ToBoolStr = (208, "TO_BOOL_STR"),
        UnpackSequenceList = (209, "UNPACK_SEQUENCE_LIST"),
        UnpackSequenceTuple = (210, "UNPACK_SEQUENCE_TUPLE"),
        UnpackSequenceTwoTuple = (211, "UNPACK_SEQUENCE_TWO_TUPLE"),
        InstrumentedEndFor = (234, "INSTRUMENTED_END_FOR"),
        InstrumentedPopIter = (235, "INSTRUMENTED_POP_ITER"),
        InstrumentedEndSend = (236, "INSTRUMENTED_END_SEND"),
        InstrumentedForIter = (237, "INSTRUMENTED_FOR_ITER"),
        InstrumentedInstruction = (238, "INSTRUMENTED_INSTRUCTION"),
        InstrumentedJumpForward = (239, "INSTRUMENTED_JUMP_FORWARD"),
        InstrumentedNotTaken = (240, "INSTRUMENTED_NOT_TAKEN"),
        InstrumentedPopJumpIfTrue = (241, "INSTRUMENTED_POP_JUMP_IF_TRUE"),
        InstrumentedPopJumpIfFalse = (242, "INSTRUMENTED_POP_JUMP_IF_FALSE"),
        InstrumentedPopJumpIfNone = (243, "INSTRUMENTED_POP_JUMP_IF_NONE"),
        InstrumentedPopJumpIfNotNone = (244, "INSTRUMENTED_POP_JUMP_IF_NOT_NONE"),
        InstrumentedResume = (245, "INSTRUMENTED_RESUME"),
        InstrumentedReturnValue = (246, "INSTRUMENTED_RETURN_VALUE"),
        InstrumentedYieldValue = (247, "INSTRUMENTED_YIELD_VALUE"),
        InstrumentedEndAsyncFor = (248, "INSTRUMENTED_END_ASYNC_FOR"),
        InstrumentedLoadSuperAttr = (249, "INSTRUMENTED_LOAD_SUPER_ATTR"),
        InstrumentedCall = (250, "INSTRUMENTED_CALL"),
        InstrumentedCallKw = (251, "INSTRUMENTED_CALL_KW"),
        InstrumentedCallFunctionEx = (252, "INSTRUMENTED_CALL_FUNCTION_EX"),
        InstrumentedJumpBackward = (253, "INSTRUMENTED_JUMP_BACKWARD"),
        InstrumentedLine = (254, "INSTRUMENTED_LINE"),
        EnterExecutor = (255, "ENTER_EXECUTOR"),
    }
);

impl Instruction {
    /// Returns `true` if this is any instrumented opcode
    /// (regular INSTRUMENTED_*, INSTRUMENTED_LINE, or INSTRUMENTED_INSTRUCTION).
    pub const fn is_instrumented(self) -> bool {
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
    pub const fn to_base(self) -> Option<Self> {
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
    pub const fn deopt(self) -> Option<Self> {
        let opcode = match self {
            Self::ResumeCheck => Opcode::Resume,
            Self::LoadConstMortal | Self::LoadConstImmortal => Opcode::LoadConst,
            Self::ToBoolAlwaysTrue
            | Self::ToBoolBool
            | Self::ToBoolInt
            | Self::ToBoolList
            | Self::ToBoolNone
            | Self::ToBoolStr => Opcode::ToBool,
            Self::BinaryOpMultiplyInt
            | Self::BinaryOpAddInt
            | Self::BinaryOpSubtractInt
            | Self::BinaryOpMultiplyFloat
            | Self::BinaryOpAddFloat
            | Self::BinaryOpSubtractFloat
            | Self::BinaryOpAddUnicode
            | Self::BinaryOpSubscrListInt
            | Self::BinaryOpSubscrListSlice
            | Self::BinaryOpSubscrTupleInt
            | Self::BinaryOpSubscrStrInt
            | Self::BinaryOpSubscrDict
            | Self::BinaryOpSubscrGetitem
            | Self::BinaryOpExtend
            | Self::BinaryOpInplaceAddUnicode => Opcode::BinaryOp,
            Self::StoreSubscrDict | Self::StoreSubscrListInt => Opcode::StoreSubscr,
            Self::SendGen => Opcode::Send,
            Self::UnpackSequenceTwoTuple | Self::UnpackSequenceTuple | Self::UnpackSequenceList => {
                Opcode::UnpackSequence
            }

            Self::StoreAttrInstanceValue | Self::StoreAttrSlot | Self::StoreAttrWithHint => {
                Opcode::StoreAttr
            }
            Self::LoadGlobalModule | Self::LoadGlobalBuiltin => Opcode::LoadGlobal,
            Self::LoadSuperAttrAttr | Self::LoadSuperAttrMethod => Opcode::LoadSuperAttr,
            Self::LoadAttrInstanceValue
            | Self::LoadAttrModule
            | Self::LoadAttrWithHint
            | Self::LoadAttrSlot
            | Self::LoadAttrClass
            | Self::LoadAttrClassWithMetaclassCheck
            | Self::LoadAttrProperty
            | Self::LoadAttrGetattributeOverridden
            | Self::LoadAttrMethodWithValues
            | Self::LoadAttrMethodNoDict
            | Self::LoadAttrMethodLazyDict
            | Self::LoadAttrNondescriptorWithValues
            | Self::LoadAttrNondescriptorNoDict => Opcode::LoadAttr,
            Self::CompareOpFloat | Self::CompareOpInt | Self::CompareOpStr => Opcode::CompareOp,
            Self::ContainsOpSet | Self::ContainsOpDict => Opcode::ContainsOp,
            Self::JumpBackwardNoJit | Self::JumpBackwardJit => Opcode::JumpBackward,
            Self::ForIterList | Self::ForIterTuple | Self::ForIterRange | Self::ForIterGen => {
                Opcode::ForIter
            }
            Self::CallBoundMethodExactArgs
            | Self::CallPyExactArgs
            | Self::CallType1
            | Self::CallStr1
            | Self::CallTuple1
            | Self::CallBuiltinClass
            | Self::CallBuiltinO
            | Self::CallBuiltinFast
            | Self::CallBuiltinFastWithKeywords
            | Self::CallLen
            | Self::CallIsinstance
            | Self::CallListAppend
            | Self::CallMethodDescriptorO
            | Self::CallMethodDescriptorFastWithKeywords
            | Self::CallMethodDescriptorNoargs
            | Self::CallMethodDescriptorFast
            | Self::CallAllocAndEnterInit
            | Self::CallPyGeneral
            | Self::CallBoundMethodGeneral
            | Self::CallNonPyGeneral => Opcode::Call,
            Self::CallKwBoundMethod | Self::CallKwPy | Self::CallKwNonPy => Opcode::CallKw,
            _ => return None,
        };

        Some(opcode.as_instruction())
    }

    /// Map a specialized or instrumented opcode back to its adaptive (base) variant.
    pub const fn deoptimize(self) -> Self {
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

    /// Number of CACHE code units that follow this instruction.
    ///
    /// Instrumented and specialized opcodes have the same cache entries as their base.
    ///
    /// _PyOpcode_Caches
    pub const fn cache_entries(self) -> usize {
        match self.deoptimize().opcode() {
            Opcode::LoadAttr => 9,
            Opcode::BinaryOp => 5,
            Opcode::LoadGlobal => 4,
            Opcode::StoreAttr => 4,
            Opcode::Call => 3,
            Opcode::CallKw => 3,
            Opcode::ToBool => 3,
            Opcode::CompareOp => 1,
            Opcode::ContainsOp => 1,
            Opcode::ForIter => 1,
            Opcode::JumpBackward => 1,
            Opcode::LoadSuperAttr => 1,
            Opcode::Send => 1,
            Opcode::StoreSubscr => 1,
            Opcode::UnpackSequence => 1,
            Opcode::PopJumpIfTrue => 1,
            Opcode::PopJumpIfFalse => 1,
            Opcode::PopJumpIfNone => 1,
            Opcode::PopJumpIfNotNone => 1,
            _ => 0,
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
            Self::LoadSpecial { .. } => (2, 1),
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
            Self::WithExceptStart => (7, 6),
            Self::YieldValue { .. } => (1, 1),
        };

        debug_assert!((0..=i32::MAX).contains(&pushed));
        debug_assert!((0..=i32::MAX).contains(&popped));

        StackEffect::new(pushed as u32, popped as u32)
    }

    // In CPython 3.14 the metadata-based stack_effect is the same for both
    // fallthrough and branch paths for all real instructions.
    // Only pseudo-instructions (SETUP_*) differ — see PseudoInstruction.
}

define_opcodes!(
    #[repr(u16)]
    pub enum PseudoOpcode;

    pub enum PseudoInstruction {
        AnnotationsPlaceholder = (256, "ANNOTATIONS_PLACEHOLDER"),
        Jump { delta: Arg<Label> } = (257, "JUMP"),
        JumpIfFalse { delta: Arg<Label> } = (258, "JUMP_IF_FALSE"),
        JumpIfTrue { delta: Arg<Label> } = (259, "JUMP_IF_TRUE"),
        JumpNoInterrupt { delta: Arg<Label> } = (260, "JUMP_NO_INTERRUPT"),
        LoadClosure { i: Arg<NameIdx> } = (261, "LOAD_CLOSURE"),
        PopBlock = (262, "POP_BLOCK"),
        SetupCleanup { delta: Arg<Label> } = (263, "SETUP_CLEANUP"),
        SetupFinally { delta: Arg<Label> } = (264, "SETUP_FINALLY"),
        SetupWith { delta: Arg<Label> } = (265, "SETUP_WITH"),
        StoreFastMaybeNull { var_num: Arg<NameIdx> } = (266, "STORE_FAST_MAYBE_NULL"),
    }
);

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

    /// Handler entry effect for SETUP_* pseudo ops.
    ///
    /// Fallthrough effect is 0 (NOPs), but when the branch is taken the
    /// handler block starts with extra values on the stack:
    ///   SETUP_FINALLY:  +1  (exc)
    ///   SETUP_CLEANUP:  +2  (lasti + exc)
    ///   SETUP_WITH:     +1  (pops __enter__ result, pushes lasti + exc)
    fn stack_effect_jump(&self, oparg: u32) -> i32 {
        match self {
            Self::SetupFinally { .. } | Self::SetupWith { .. } => 1,
            Self::SetupCleanup { .. } => 2,
            _ => self.stack_effect(oparg),
        }
    }

    fn is_unconditional_jump(&self) -> bool {
        matches!(self, Self::Jump { .. } | Self::JumpNoInterrupt { .. })
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

    inst_either!(fn stack_effect_jump(&self, oparg: u32) -> i32);

    inst_either!(fn stack_effect_info(&self, oparg: u32) -> StackEffect);
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
            .expect("Expected AnyInstruction::Real, found AnyInstruction::Pseudo")
    }

    /// Same as [`Self::pseudo`] but panics if wasn't called on [`Self::Pseudo`].
    ///
    /// # Panics
    ///
    /// If was called on something else other than [`Self::Pseudo`].
    pub const fn expect_pseudo(self) -> PseudoInstruction {
        self.pseudo()
            .expect("Expected AnyInstruction::Pseudo, found AnyInstruction::Real")
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

#[derive(Clone, Copy, Debug)]
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
    pub const fn real(self) -> Option<Opcode> {
        match self {
            Self::Real(op) => Some(op),
            _ => None,
        }
    }

    /// Gets the inner value of [`Self::Pseudo`].
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
    pub const fn expect_real(self) -> Opcode {
        self.real()
            .expect("Expected AnyOpcode::Real, found AnyOpcode::Pseudo")
    }

    /// Same as [`Self::pseudo`] but panics if wasn't called on [`Self::Pseudo`].
    ///
    /// # Panics
    ///
    /// If was called on something else other than [`Self::Pseudo`].
    pub const fn expect_pseudo(self) -> PseudoOpcode {
        self.pseudo()
            .expect("Expected AnyOpcode::Pseudo, found AnyOpcode::Real")
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

    /// Stack effect when the instruction takes its branch (jump=true).
    ///
    /// CPython equivalent: `stack_effect(opcode, oparg, jump=True)`.
    /// For most instructions this equals the fallthrough effect.
    /// Override for instructions where branch and fallthrough differ
    /// (e.g. `FOR_ITER`: fallthrough = +1, branch = −1).
    fn stack_effect_jump(&self, oparg: u32) -> i32 {
        self.stack_effect(oparg)
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

// TODO: Can probably remove these asserts and remove the `repr($typ)` from the macro. but this
// breaks the VM:/
const _: () = assert!(core::mem::size_of::<Instruction>() == 1);
const _: () = assert!(core::mem::size_of::<PseudoInstruction>() == 2);
