use core::fmt;

use crate::{
    bytecode::{CodeUnit, instructions::Instruction},
    marshal::MarshalError,
};

pub trait OpArgType: Copy + Into<u32> + TryFrom<u32> {}

/// Opcode argument that may be extended by a prior ExtendedArg.
#[derive(Copy, Clone, PartialEq, Eq)]
#[repr(transparent)]
pub struct OpArgByte(u8);

impl OpArgByte {
    pub const NULL: Self = Self::new(0);

    #[must_use]
    pub const fn new(value: u8) -> Self {
        Self(value)
    }

    /// Returns the inner value as a [`u8`].
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    /// Returns the inner value as a [`u32`].
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        self.0 as u32
    }
}

impl From<u8> for OpArgByte {
    fn from(raw: u8) -> Self {
        Self::new(raw)
    }
}

impl From<OpArgByte> for u8 {
    fn from(value: OpArgByte) -> Self {
        value.as_u8()
    }
}

impl fmt::Debug for OpArgByte {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Full 32-bit op_arg, including any possible ExtendedArg extension.
#[derive(Copy, Clone, Debug)]
#[repr(transparent)]
pub struct OpArg(u32);

impl OpArg {
    pub const NULL: Self = Self::new(0);

    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns how many CodeUnits a instruction with this op_arg will be encoded as
    #[inline]
    #[must_use]
    pub const fn instr_size(self) -> usize {
        (self.0 > 0xff) as usize + (self.0 > 0xff_ff) as usize + (self.0 > 0xff_ff_ff) as usize + 1
    }

    /// returns the arg split into any necessary ExtendedArg components (in big-endian order) and
    /// the arg for the real opcode itself
    #[inline(always)]
    pub fn split(self) -> (impl ExactSizeIterator<Item = OpArgByte>, OpArgByte) {
        let mut it = self
            .0
            .to_le_bytes()
            .map(OpArgByte)
            .into_iter()
            .take(self.instr_size());
        let lo = it.next().unwrap();
        (it.rev(), lo)
    }
}

impl From<u32> for OpArg {
    fn from(raw: u32) -> Self {
        Self::new(raw)
    }
}

impl From<OpArg> for u32 {
    fn from(value: OpArg) -> Self {
        value.0
    }
}

#[derive(Default, Copy, Clone)]
#[repr(transparent)]
pub struct OpArgState {
    state: u32,
}

impl OpArgState {
    #[inline(always)]
    pub const fn get(&mut self, ins: CodeUnit) -> (Instruction, OpArg) {
        let arg = self.extend(ins.arg);
        if !matches!(ins.op, Instruction::ExtendedArg) {
            self.reset();
        }
        (ins.op, arg)
    }

    #[inline(always)]
    pub const fn extend(&mut self, arg: OpArgByte) -> OpArg {
        self.state = (self.state << 8) | arg.as_u32();
        OpArg::new(self.state)
    }

    #[inline(always)]
    pub const fn reset(&mut self) {
        self.state = 0
    }
}

/// Defines an enum whose variants map to fixed `u8` discriminants,
/// and automatically implements the following traits:
///
/// - [`Display`](std::fmt::Display)
/// - [`From`]`<EnumName> for u8` / `u32`
/// - [`TryFrom`]`<u8> / <u32> for EnumName`
/// - [`OpArgType`]
///
/// Along with the inherent methods `as_u8`, `as_u32`, `try_from_u8`, and `try_from_u32`.
///
/// # Variant syntax
///
/// Each variant is assigned a value using one of two forms:
///
/// | Form | Syntax | `Display` output |
/// |---|---|---|
/// | Numeric | `Variant = 0` | `0` |
/// | Labeled | `Variant = (0, "label")` | `label` |
///
/// # Example
///
/// ```ignore
/// oparg_enum! {
///     #[derive(Debug, Clone, Copy, PartialEq, Eq)]
///     pub enum MyArg {
///         /// No argument.
///         None  = 0,
///         /// A small argument, displayed as "small".
///         Small = (1, "small"),
///         /// A large argument, displayed as "large".
///         Large = (2, "large"),
///     }
/// }
///
/// assert_eq!(MyArg::None.as_u8(), 0);
/// assert_eq!(MyArg::Small.as_u8(), 1);
///
/// assert_eq!(MyArg::try_from_u8(2), Ok(MyArg::Large));
/// assert_eq!(u8::from(MyArg::None), 0u8);
///
/// assert_eq!(MyArg::None.to_string(),  "0");
/// assert_eq!(MyArg::Small.to_string(), "small");
/// assert_eq!(MyArg::Large.to_string(), "large");
///
/// // Format specs are respected
/// assert_eq!(format!("{:>10}", MyArg::Small), "     small");
/// ```
macro_rules! oparg_enum {
    (
        $(#[$enum_meta:meta])*
        $vis:vis enum $name:ident {
            $(
                $(#[$variant_meta:meta])*
                $variant:ident = $value:tt
            ),* $(,)?
        }
    ) => {
        $(#[$enum_meta])*
        $vis enum $name {
            $(
                $(#[$variant_meta])*
                $variant, // Do assign value to variant.
            )*
        }

        impl_oparg_enum!(
            $vis enum $name {
                $(
                    $variant = $value,
                )*
            }
        );
    };
}

macro_rules! impl_oparg_enum {
    (
        $vis:vis enum $name:ident {
            $(
                $variant:ident = $value:tt
            ),* $(,)?
        }
    ) => {
        impl $name {
            /// Returns the oparg as a [`u8`] value.
            #[must_use]
            $vis const fn as_u8(self) -> u8 {
                match self {
                    $(
                        Self::$variant => impl_oparg_enum!(@discriminant $value),
                    )*
                }
            }

            /// Returns the oparg as a [`u32`] value.
            #[must_use]
            $vis const fn as_u32(self) -> u32 {
                self.as_u8() as u32
            }

            $vis const fn try_from_u8(value: u8) -> Result<Self, $crate::marshal::MarshalError> {
                Ok(match value {
                    $(
                        impl_oparg_enum!(@discriminant $value) => Self::$variant,
                    )*
                    _ => return Err($crate::marshal::MarshalError::InvalidBytecode),
                })
            }

            $vis const fn try_from_u32(value: u32) -> Result<Self, $crate::marshal::MarshalError> {
                if value > (u8::MAX as u32) {
                    return Err($crate::marshal::MarshalError::InvalidBytecode);
                }

                // We already validated this is a lossles cast.
                Self::try_from_u8(value as u8)
            }

            /// Iterate over the variants.
            $vis fn iter() -> impl Iterator<Item = Self> {
                [$(Self::$variant),*].iter().copied()
            }
        }

        impl TryFrom<u8> for $name {
            type Error = $crate::marshal::MarshalError;

            fn try_from(value: u8) -> Result<Self, Self::Error> {
                Self::try_from_u8(value)
            }
        }

        impl TryFrom<u32> for $name {
            type Error = $crate::marshal::MarshalError;

            fn try_from(value: u32) -> Result<Self, Self::Error> {
                Self::try_from_u32(value)
            }
        }

        impl From<$name> for u8 {
            fn from(value: $name) -> Self {
                value.as_u8()
            }
        }

        impl From<$name> for u32 {
            fn from(value: $name) -> Self {
                value.as_u32()
            }
        }

        impl ::core::fmt::Display for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                match self {
                    $(
                        Self::$variant => impl_oparg_enum!(@display f, $value),
                    )*
                }
            }
        }

        impl OpArgType for $name {}
    };

    (@discriminant ($num:literal, $str:literal)) => { $num };
    (@discriminant $num:literal) => { $num };
    (@display $f:expr, ($num:literal, $str:literal)) => {
        ::core::fmt::Display::fmt($str, $f)
    };
    (@display $f:expr, $num:literal) => {
        ::core::fmt::Display::fmt(&$num, $f)
    };
}

oparg_enum!(
    /// Oparg values for [`Instruction::ConvertValue`].
    ///
    /// ## See also
    ///
    /// - [CPython FVC_* flags](https://github.com/python/cpython/blob/v3.14.4/Include/ceval.h#L129-L132)
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum ConvertValueOparg {
        /// No conversion.
        ///
        /// ```python
        /// f"{x}"
        /// f"{x:4}"
        /// ```
        // NOTE: We should never reach the display of this.
        // `FVC_NONE` are being handled by `Instruction::FormatSimple`
        None = 0,
        /// Converts by calling `str(<value>)`.
        ///
        /// ```python
        /// f"{x!s}"
        /// f"{x!s:2}"
        /// ```
        Str = (1, "str"),
        /// Converts by calling `repr(<value>)`.
        ///
        /// ```python
        /// f"{x!r}"
        /// f"{x!r:2}"
        /// ```
        Repr = (2, "repr"),
        /// Converts by calling `ascii(<value>)`.
        ///
        /// ```python
        /// f"{x!a}"
        /// f"{x!a:2}"
        /// ```
        Ascii = (3, "ascii"),
    }
);

pub type NameIdx = u32;

impl OpArgType for u32 {}

oparg_enum!(
    /// The kind of Raise that occurred.
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub enum RaiseKind {
        /// Bare `raise` statement with no arguments.
        /// Gets the current exception from VM state (topmost_exception).
        /// Maps to RAISE_VARARGS with oparg=0.
        BareRaise = 0,
        /// `raise exc` - exception is on the stack.
        /// Maps to RAISE_VARARGS with oparg=1.
        Raise = 1,
        /// `raise exc from cause` - exception and cause are on the stack.
        /// Maps to RAISE_VARARGS with oparg=2.
        RaiseCause = 2,
        /// Reraise exception from the stack top.
        /// Used in exception handler cleanup blocks (finally, except).
        /// Gets exception from stack, not from VM state.
        /// Maps to the RERAISE opcode.
        ReraiseFromStack = 3,
    }
);

oparg_enum!(
    /// Intrinsic function for CALL_INTRINSIC_1
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub enum IntrinsicFunction1 {
        Invalid = 0,
        Print = 1,
        /// Import * operation
        ImportStar = 2,
        /// Convert StopIteration to RuntimeError in async context
        StopIterationError = 3,
        AsyncGenWrap = 4,
        UnaryPositive = 5,
        /// Convert list to tuple
        ListToTuple = 6,
        /// Type parameter related
        TypeVar = 7,
        ParamSpec = 8,
        TypeVarTuple = 9,
        /// Generic subscript for PEP 695
        SubscriptGeneric = 10,
        TypeAlias = 11,
    }
);

impl IntrinsicFunction1 {
    /// https://github.com/python/cpython/blob/v3.14.4/Include/internal/pycore_intrinsics.h#L9-L20
    #[must_use]
    pub const fn desc(&self) -> &str {
        match self {
            Self::Invalid => "INTRINSIC_1_INVALID",
            Self::Print => "INTRINSIC_PRINT",
            Self::ImportStar => "INTRINSIC_IMPORT_STAR",
            Self::StopIterationError => "INTRINSIC_STOPITERATION_ERROR",
            Self::AsyncGenWrap => "INTRINSIC_ASYNC_GEN_WRAP",
            Self::UnaryPositive => "INTRINSIC_UNARY_POSITIVE",
            Self::ListToTuple => "INTRINSIC_LIST_TO_TUPLE",
            Self::TypeVar => "INTRINSIC_TYPEVAR",
            Self::ParamSpec => "INTRINSIC_PARAMSPEC",
            Self::TypeVarTuple => "INTRINSIC_TYPEVARTUPLE",
            Self::SubscriptGeneric => "INTRINSIC_SUBSCRIPT_GENERIC",
            Self::TypeAlias => "INTRINSIC_TYPEALIAS",
        }
    }
}

oparg_enum!(
    /// Intrinsic function for CALL_INTRINSIC_2
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub enum IntrinsicFunction2 {
        Invalid = 0,
        PrepReraiseStar = 1,
        TypeVarWithBound = 2,
        TypeVarWithConstraint = 3,
        SetFunctionTypeParams = 4,
        /// Set default value for type parameter (PEP 695)
        SetTypeparamDefault = 5,
    }
);

impl IntrinsicFunction2 {
    /// https://github.com/python/cpython/blob/v3.14.4/Include/internal/pycore_intrinsics.h#L26-L31
    #[must_use]
    pub const fn desc(&self) -> &str {
        match self {
            Self::Invalid => "INTRINSIC_2_INVALID",
            Self::PrepReraiseStar => "INTRINSIC_PREP_RERAISE_STAR",
            Self::TypeVarWithBound => "INTRINSIC_TYPEVAR_WITH_BOUND",
            Self::TypeVarWithConstraint => "INTRINSIC_TYPEVAR_WITH_CONSTRAINTS",
            Self::SetFunctionTypeParams => "INTRINSIC_SET_FUNCTION_TYPE_PARAMS",
            Self::SetTypeparamDefault => "INTRINSIC_SET_TYPEPARAM_DEFAULT",
        }
    }
}

bitflagset::bitflag! {
    /// `SET_FUNCTION_ATTRIBUTE` flags.
    /// Bitmask: Defaults=0x01, KwOnly=0x02, Annotations=0x04,
    /// Closure=0x08, TypeParams=0x10, Annotate=0x20.
    /// Stored as bit position (0-5) by `bitflag!` macro.
    #[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
    #[repr(u8)]
    pub enum MakeFunctionFlag {
        Defaults = 0,
        KwOnlyDefaults = 1,
        Annotations = 2,
        Closure = 3,
        /// PEP 649: __annotate__ function closure (instead of __annotations__ dict)
        Annotate = 4,
        TypeParams = 5,
    }
}

bitflagset::bitflagset! {
    #[derive(Copy, Clone, PartialEq, Eq)]
    pub struct MakeFunctionFlags(u8): MakeFunctionFlag
}

impl TryFrom<u32> for MakeFunctionFlag {
    type Error = MarshalError;

    /// Decode from CPython-compatible power-of-two value
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(Self::Defaults),
            0x02 => Ok(Self::KwOnlyDefaults),
            0x04 => Ok(Self::Annotations),
            0x08 => Ok(Self::Closure),
            0x10 => Ok(Self::Annotate),
            0x20 => Ok(Self::TypeParams),
            _ => Err(MarshalError::InvalidBytecode),
        }
    }
}

impl From<MakeFunctionFlag> for u32 {
    /// Encode as CPython-compatible power-of-two value
    fn from(flag: MakeFunctionFlag) -> Self {
        1u32 << (flag as u32)
    }
}

impl OpArgType for MakeFunctionFlag {}

/// `COMPARE_OP` arg is `(cmp_index << 5) | mask`.
///
/// The low four bits are the CPython comparison mask used by specialized
/// compare opcodes, and bit 4 requests bool-conversion of the compare result.
pub const COMPARE_OP_BOOL_MASK: u32 = 1 << 4;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ComparisonOperator {
    Less,
    LessOrEqual,
    Equal,
    NotEqual,
    Greater,
    GreaterOrEqual,
}

impl TryFrom<u8> for ComparisonOperator {
    type Error = MarshalError;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::try_from(value as u32)
    }
}

impl TryFrom<u32> for ComparisonOperator {
    type Error = MarshalError;
    /// Decode from `COMPARE_OP` arg: `(cmp_index << 5) | mask`.
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value >> 5 {
            0 => Ok(Self::Less),
            1 => Ok(Self::LessOrEqual),
            2 => Ok(Self::Equal),
            3 => Ok(Self::NotEqual),
            4 => Ok(Self::Greater),
            5 => Ok(Self::GreaterOrEqual),
            _ => Err(MarshalError::InvalidBytecode),
        }
    }
}

impl From<ComparisonOperator> for u8 {
    /// Encode using CPython's comparison mask layout.
    fn from(value: ComparisonOperator) -> Self {
        match value {
            ComparisonOperator::Less => 2,
            ComparisonOperator::LessOrEqual => (1 << 5) | 2 | 8,
            ComparisonOperator::Equal => (2 << 5) | 8,
            ComparisonOperator::NotEqual => (3 << 5) | 1 | 2 | 4,
            ComparisonOperator::Greater => (4 << 5) | 4,
            ComparisonOperator::GreaterOrEqual => (5 << 5) | 4 | 8,
        }
    }
}

impl From<ComparisonOperator> for u32 {
    fn from(value: ComparisonOperator) -> Self {
        Self::from(u8::from(value))
    }
}

impl OpArgType for ComparisonOperator {}

impl fmt::Display for ComparisonOperator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let op = match self {
            Self::Less => "<",
            Self::LessOrEqual => "<=",
            Self::Equal => "==",
            Self::NotEqual => "!=",
            Self::Greater => ">",
            Self::GreaterOrEqual => ">=",
        };
        f.write_str(op)
    }
}

oparg_enum!(
    /// The possible Binary operators
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rustpython_compiler_core::bytecode::{Arg, BinaryOperator, Instruction};
    /// let (op, _) = Arg::new(BinaryOperator::Add);
    /// let instruction = Instruction::BinaryOp { op };
    /// ```
    ///
    /// See also:
    /// - [_PyEval_BinaryOps](https://github.com/python/cpython/blob/8183fa5e3f78ca6ab862de7fb8b14f3d929421e0/Python/ceval.c#L316-L343)
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum BinaryOperator {
        /// `+`
        Add = (0, "+"),
        /// `&`
        And = (1, "&"),
        /// `//`
        FloorDivide = (2, "//"),
        /// `<<`
        Lshift = (3, "<<"),
        /// `@`
        MatrixMultiply = (4, "@"),
        /// `*`
        Multiply = (5, "*"),
        /// `%`
        Remainder = (6, "%"),
        /// `|`
        Or = (7, "|"),
        /// `**`
        Power = (8, "**"),
        /// `>>`
        Rshift = (9, ">>"),
        /// `-`
        Subtract = (10, "-"),
        /// `/`
        TrueDivide = (11, "/"),
        /// `^`
        Xor = (12, "^"),
        /// `+=`
        InplaceAdd = (13, "+="),
        /// `&=`
        InplaceAnd = (14, "&="),
        /// `//=`
        InplaceFloorDivide = (15, "//="),
        /// `<<=`
        InplaceLshift = (16, "<<="),
        /// `@=`
        InplaceMatrixMultiply = (17, "@="),
        /// `*=`
        InplaceMultiply = (18, "*="),
        /// `%=`
        InplaceRemainder = (19, "%="),
        /// `|=`
        InplaceOr = (20, "|="),
        /// `**=`
        InplacePower = (21, "**="),
        /// `>>=`
        InplaceRshift = (22, ">>="),
        /// `-=`
        InplaceSubtract = (23, "-="),
        /// `/=`
        InplaceTrueDivide = (24, "/="),
        /// `^=`
        InplaceXor = (25, "^="),
        /// `[]` subscript
        Subscr = (26, "[]"),
    }
);

impl BinaryOperator {
    /// Get the "inplace" version of the operator.
    /// This has no effect if `self` is already an "inplace" operator.
    ///
    /// # Example
    /// ```rust
    /// use rustpython_compiler_core::bytecode::BinaryOperator;
    ///
    /// assert_eq!(BinaryOperator::Power.as_inplace(), BinaryOperator::InplacePower);
    ///
    /// assert_eq!(BinaryOperator::InplaceSubtract.as_inplace(), BinaryOperator::InplaceSubtract);
    /// ```
    #[must_use]
    pub const fn as_inplace(self) -> Self {
        match self {
            Self::Add => Self::InplaceAdd,
            Self::And => Self::InplaceAnd,
            Self::FloorDivide => Self::InplaceFloorDivide,
            Self::Lshift => Self::InplaceLshift,
            Self::MatrixMultiply => Self::InplaceMatrixMultiply,
            Self::Multiply => Self::InplaceMultiply,
            Self::Remainder => Self::InplaceRemainder,
            Self::Or => Self::InplaceOr,
            Self::Power => Self::InplacePower,
            Self::Rshift => Self::InplaceRshift,
            Self::Subtract => Self::InplaceSubtract,
            Self::TrueDivide => Self::InplaceTrueDivide,
            Self::Xor => Self::InplaceXor,
            _ => self,
        }
    }

    /// https://github.com/python/cpython/blob/v3.14.4/Include/opcode.h#L10-L36
    #[must_use]
    pub const fn desc(&self) -> &str {
        match self {
            Self::Add => "NB_ADD",
            Self::And => "NB_AND",
            Self::FloorDivide => "NB_FLOOR_DIVIDE",
            Self::Lshift => "NB_LSHIFT",
            Self::MatrixMultiply => "NB_MATRIX_MULTIPLY",
            Self::Multiply => "NB_MULTIPLY",
            Self::Remainder => "NB_REMAINDER",
            Self::Or => "NB_OR",
            Self::Power => "NB_POWER",
            Self::Rshift => "NB_RSHIFT",
            Self::Subtract => "NB_SUBTRACT",
            Self::TrueDivide => "NB_TRUE_DIVIDE",
            Self::Xor => "NB_XOR",
            Self::InplaceAdd => "NB_INPLACE_ADD",
            Self::InplaceAnd => "NB_INPLACE_AND",
            Self::InplaceFloorDivide => "NB_INPLACE_FLOOR_DIVIDE",
            Self::InplaceLshift => "NB_INPLACE_LSHIFT",
            Self::InplaceMatrixMultiply => "NB_INPLACE_MATRIX_MULTIPLY",
            Self::InplaceMultiply => "NB_INPLACE_MULTIPLY",
            Self::InplaceRemainder => "NB_INPLACE_REMAINDER",
            Self::InplaceOr => "NB_INPLACE_OR",
            Self::InplacePower => "NB_INPLACE_POWER",
            Self::InplaceRshift => "NB_INPLACE_RSHIFT",
            Self::InplaceSubtract => "NB_INPLACE_SUBTRACT",
            Self::InplaceTrueDivide => "NB_INPLACE_TRUE_DIVIDE",
            Self::InplaceXor => "NB_INPLACE_XOR",
            Self::Subscr => "NB_SUBSCR",
        }
    }
}

oparg_enum!(
    /// Whether or not to invert the operation.
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum Invert {
        /// ```py
        /// foo is bar
        /// x in lst
        /// ```
        No = 0,
        /// ```py
        /// foo is not bar
        /// x not in lst
        /// ```
        Yes = 1,
    }
);

oparg_enum!(
    /// Special method for LOAD_SPECIAL opcode (context managers).
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum SpecialMethod {
        /// `__enter__` for sync context manager
        Enter = (0, "__enter__"),
        /// `__exit__` for sync context manager
        Exit = (1, "__exit__"),
        /// `__aenter__` for async context manager
        AEnter = (2, "__aenter__"),
        /// `__aexit__` for async context manager
        AExit = (3, "__aexit__"),
    }
);

oparg_enum!(
    /// Common constants for LOAD_COMMON_CONSTANT opcode.
    /// pycore_opcode_utils.h CONSTANT_*
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum CommonConstant {
        /// `AssertionError` exception type
        AssertionError = (0, "AssertionError"),
        /// `NotImplementedError` exception type
        NotImplementedError = (1, "NotImplementedError"),
        /// Built-in `tuple` type
        BuiltinTuple = (2, "tuple"),
        /// Built-in `all` function
        BuiltinAll = (3, "all"),
        /// Built-in `any` function
        BuiltinAny = (4, "any"),
        /// Built-in `list` type
        BuiltinList = (5, "list"),
        /// Built-in `set` type
        BuiltinSet = (6, "set"),
    }
);

oparg_enum!(
    /// Specifies if a slice is built with either 2 or 3 arguments.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum BuildSliceArgCount {
        /// ```py
        /// x[5:10]
        /// ```
        Two = 2,
        /// ```py
        /// x[5:10:2]
        /// ```
        Three = 3,
    }
);

#[derive(Copy, Clone)]
pub struct UnpackExArgs {
    pub before: u8,
    pub after: u8,
}

impl From<u32> for UnpackExArgs {
    fn from(value: u32) -> Self {
        let [before, after, ..] = value.to_le_bytes();
        Self { before, after }
    }
}

impl From<UnpackExArgs> for u32 {
    fn from(value: UnpackExArgs) -> Self {
        Self::from_le_bytes([value.before, value.after, 0, 0])
    }
}

impl OpArgType for UnpackExArgs {}

impl fmt::Display for UnpackExArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "before: {}, after: {}", self.before, self.after)
    }
}

macro_rules! newtype_oparg {
    (
      $(#[$oparg_meta:meta])*
      $vis:vis struct $name:ident(u32)
    ) => {
        $(#[$oparg_meta])*
        $vis struct $name(u32);

        impl $name {
            #[doc = concat!("Creates a new [`", stringify!($name), "`] instance.")]
            #[must_use]
            pub const fn from_u32(value: u32) -> Self {
                Self(value)
            }

            /// Returns the oparg as a [`u32`] value.
            #[must_use]
            pub const fn as_u32(self) -> u32 {
                self.0
            }

            /// Returns the oparg as a [`usize`] value.
            #[must_use]
            pub const fn as_usize(self) -> usize {
              self.0 as usize
            }
        }

        impl From<u32> for $name {
            fn from(value: u32) -> Self {
                Self::from_u32(value)
            }
        }

        impl From<$name> for u32 {
            fn from(value: $name) -> Self {
                value.as_u32()
            }
        }

        impl From<$name> for usize {
            fn from(value: $name) -> Self {
                value.as_usize()
            }
        }

        impl ::core::fmt::Display for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                self.0.fmt(f)
            }
        }

        impl OpArgType for $name {}
    }
}

newtype_oparg!(
    #[derive(Clone, Copy)]
    #[repr(transparent)]
    pub struct ConstIdx(u32)
);

newtype_oparg!(
    #[derive(Clone, Copy)]
    #[repr(transparent)]
    pub struct VarNum(u32)
);

newtype_oparg!(
    #[derive(Clone, Copy)]
    #[repr(transparent)]
    pub struct VarNums(u32)
);

newtype_oparg!(
    #[derive(Clone, Copy)]
    #[repr(transparent)]
    pub struct LoadAttr(u32)
);

newtype_oparg!(
    #[derive(Clone, Copy)]
    #[repr(transparent)]
    pub struct LoadSuperAttr(u32)
);

newtype_oparg!(
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
    #[repr(transparent)]
    pub struct Label(u32)
);

newtype_oparg!(
    /// Context for [`Instruction::Resume`].
    ///
    /// The oparg consists of two parts:
    /// 1. [`ResumeContext::location`]: Indicates where the instruction occurs.
    /// 2. [`ResumeContext::is_exception_depth1`]: Is the instruction is at except-depth 1.
    #[derive(Clone, Copy)]
    #[repr(transparent)]
    pub struct ResumeContext(u32)
);

impl ResumeContext {
    /// [CPython `RESUME_OPARG_LOCATION_MASK`](https://github.com/python/cpython/blob/v3.14.3/Include/internal/pycore_opcode_utils.h#L84)
    pub const LOCATION_MASK: u32 = 0x3;

    /// [CPython `RESUME_OPARG_DEPTH1_MASK`](https://github.com/python/cpython/blob/v3.14.3/Include/internal/pycore_opcode_utils.h#L85)
    pub const DEPTH1_MASK: u32 = 0x4;

    #[must_use]
    pub const fn new(location: ResumeLocation, is_exception_depth1: bool) -> Self {
        let value = if is_exception_depth1 {
            Self::DEPTH1_MASK
        } else {
            0
        };

        Self::from_u32(location.as_u32() | value)
    }

    /// Resume location is determined by [`Self::LOCATION_MASK`].
    #[must_use]
    pub fn location(&self) -> ResumeLocation {
        // SAFETY: The mask should return a value that is in range.
        unsafe { ResumeLocation::try_from(self.as_u32() & Self::LOCATION_MASK).unwrap_unchecked() }
    }

    /// True if the bit at [`Self::DEPTH1_MASK`] is on.
    #[must_use]
    pub const fn is_exception_depth1(&self) -> bool {
        (self.as_u32() & Self::DEPTH1_MASK) != 0
    }
}

#[derive(Copy, Clone)]
pub enum ResumeLocation {
    /// At the start of a function, which is neither a generator, coroutine nor an async generator.
    AtFuncStart,
    /// After a `yield` expression.
    AfterYield,
    /// After a `yield from` expression.
    AfterYieldFrom,
    /// After an `await` expression.
    AfterAwait,
}

impl From<ResumeLocation> for ResumeContext {
    fn from(location: ResumeLocation) -> Self {
        Self::new(location, false)
    }
}

impl TryFrom<u32> for ResumeLocation {
    type Error = MarshalError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Ok(match value {
            0 => Self::AtFuncStart,
            1 => Self::AfterYield,
            2 => Self::AfterYieldFrom,
            3 => Self::AfterAwait,
            _ => return Err(Self::Error::InvalidBytecode),
        })
    }
}

impl ResumeLocation {
    #[must_use]
    pub const fn as_u8(&self) -> u8 {
        match self {
            Self::AtFuncStart => 0,
            Self::AfterYield => 1,
            Self::AfterYieldFrom => 2,
            Self::AfterAwait => 3,
        }
    }

    #[must_use]
    pub const fn as_u32(&self) -> u32 {
        self.as_u8() as u32
    }
}

impl From<ResumeLocation> for u8 {
    fn from(location: ResumeLocation) -> Self {
        location.as_u8()
    }
}

impl From<ResumeLocation> for u32 {
    fn from(location: ResumeLocation) -> Self {
        location.as_u32()
    }
}

impl VarNums {
    #[must_use]
    pub const fn idx_1(self) -> VarNum {
        VarNum::from_u32(self.0 >> 4)
    }

    #[must_use]
    pub const fn idx_2(self) -> VarNum {
        VarNum::from_u32(self.0 & 15)
    }

    #[must_use]
    pub const fn indexes(self) -> (VarNum, VarNum) {
        (self.idx_1(), self.idx_2())
    }
}

impl LoadAttr {
    #[must_use]
    pub const fn new(name_idx: u32, is_method: bool) -> Self {
        Self::from_u32((name_idx << 1) | (is_method as u32))
    }

    #[must_use]
    pub const fn name_idx(self) -> u32 {
        self.0 >> 1
    }

    #[must_use]
    pub const fn is_method(self) -> bool {
        (self.0 & 1) == 1
    }
}

impl LoadSuperAttr {
    #[must_use]
    pub const fn new(name_idx: u32, is_load_method: bool, has_class: bool) -> Self {
        Self::from_u32((name_idx << 2) | (is_load_method as u32) | ((has_class as u32) << 1))
    }

    #[must_use]
    pub const fn name_idx(self) -> u32 {
        self.0 >> 2
    }

    #[must_use]
    pub const fn is_load_method(self) -> bool {
        (self.0 & 1) == 1
    }

    #[must_use]
    pub const fn has_class(self) -> bool {
        (self.0 & 2) == 2
    }
}
