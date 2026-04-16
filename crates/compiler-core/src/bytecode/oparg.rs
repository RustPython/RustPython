use core::fmt;

use crate::{
    bytecode::{CodeUnit, instruction::Instruction},
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
}

impl From<u8> for OpArgByte {
    fn from(raw: u8) -> Self {
        Self::new(raw)
    }
}

impl From<OpArgByte> for u8 {
    fn from(value: OpArgByte) -> Self {
        value.0
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
    pub fn get(&mut self, ins: CodeUnit) -> (Instruction, OpArg) {
        let arg = self.extend(ins.arg);
        if !matches!(ins.op, Instruction::ExtendedArg) {
            self.reset();
        }
        (ins.op, arg)
    }

    #[inline(always)]
    pub fn extend(&mut self, arg: OpArgByte) -> OpArg {
        self.state = (self.state << 8) | u32::from(arg.0);
        self.state.into()
    }

    #[inline(always)]
    pub const fn reset(&mut self) {
        self.state = 0
    }
}

/// Helper macro for defining oparg enums in an optimal way.
///
/// Will generate the following:
///
/// - Enum which variant's aren't assigned any value (for optimizations).
/// - impl [`TryFrom<u8>`]
/// - impl [`TryFrom<u32>`]
/// - impl [`Into<u8>`]
/// - impl [`Into<u32>`]
/// - impl [`OpArgType`]
///
/// # Examples
///
/// ```ignore
/// oparg_enum!(
///     /// Oparg for the `X` opcode.
///     #[derive(Clone, Copy)]
///     pub enum MyOpArg {
///         /// Some doc.
///         Foo = 4,
///         Bar = 8,
///         Baz = 15,
///         Qux = 16
///     }
/// );
/// ```
macro_rules! oparg_enum {
    (
        $(#[$enum_meta:meta])*
        $vis:vis enum $name:ident {
            $(
                $(#[$variant_meta:meta])*
                $variant:ident = $value:literal
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
                $variant:ident = $value:literal
            ),* $(,)?
        }
    ) => {
        impl $name {
            /// Returns the oparg as a [`u8`] value.
            #[must_use]
            $vis const fn as_u8(self) -> u8 {
                match self {
                    $(
                        Self::$variant => $value,
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
                        $value => Self::$variant,
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

        impl OpArgType for $name {}
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
        None = 0,
        /// Converts by calling `str(<value>)`.
        ///
        /// ```python
        /// f"{x!s}"
        /// f"{x!s:2}"
        /// ```
        Str = 1,
        /// Converts by calling `repr(<value>)`.
        ///
        /// ```python
        /// f"{x!r}"
        /// f"{x!r:2}"
        /// ```
        Repr = 2,
        /// Converts by calling `ascii(<value>)`.
        ///
        /// ```python
        /// f"{x!a}"
        /// f"{x!a:2}"
        /// ```
        Ascii = 3,
    }
);

impl fmt::Display for ConvertValueOparg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let out = match self {
            Self::Str => "1 (str)",
            Self::Repr => "2 (repr)",
            Self::Ascii => "3 (ascii)",
            // We should never reach this. `FVC_NONE` are being handled by `Instruction::FormatSimple`
            Self::None => "",
        };

        write!(f, "{out}")
    }
}

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
        // Invalid = 0,
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

oparg_enum!(
    /// Intrinsic function for CALL_INTRINSIC_2
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub enum IntrinsicFunction2 {
        PrepReraiseStar = 1,
        TypeVarWithBound = 2,
        TypeVarWithConstraint = 3,
        SetFunctionTypeParams = 4,
        /// Set default value for type parameter (PEP 695)
        SetTypeparamDefault = 5,
    }
);

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
        Add = 0,
        /// `&`
        And = 1,
        /// `//`
        FloorDivide = 2,
        /// `<<`
        Lshift = 3,
        /// `@`
        MatrixMultiply = 4,
        /// `*`
        Multiply = 5,
        /// `%`
        Remainder = 6,
        /// `|`
        Or = 7,
        /// `**`
        Power = 8,
        /// `>>`
        Rshift = 9,
        /// `-`
        Subtract = 10,
        /// `/`
        TrueDivide = 11,
        /// `^`
        Xor = 12,
        /// `+=`
        InplaceAdd = 13,
        /// `&=`
        InplaceAnd = 14,
        /// `//=`
        InplaceFloorDivide = 15,
        /// `<<=`
        InplaceLshift = 16,
        /// `@=`
        InplaceMatrixMultiply = 17,
        /// `*=`
        InplaceMultiply = 18,
        /// `%=`
        InplaceRemainder = 19,
        /// `|=`
        InplaceOr = 20,
        /// `**=`
        InplacePower = 21,
        /// `>>=`
        InplaceRshift = 22,
        /// `-=`
        InplaceSubtract = 23,
        /// `/=`
        InplaceTrueDivide = 24,
        /// `^=`
        InplaceXor = 25,
        /// `[]` subscript
        Subscr = 26,
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
}

impl fmt::Display for BinaryOperator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let op = match self {
            Self::Add => "+",
            Self::And => "&",
            Self::FloorDivide => "//",
            Self::Lshift => "<<",
            Self::MatrixMultiply => "@",
            Self::Multiply => "*",
            Self::Remainder => "%",
            Self::Or => "|",
            Self::Power => "**",
            Self::Rshift => ">>",
            Self::Subtract => "-",
            Self::TrueDivide => "/",
            Self::Xor => "^",
            Self::InplaceAdd => "+=",
            Self::InplaceAnd => "&=",
            Self::InplaceFloorDivide => "//=",
            Self::InplaceLshift => "<<=",
            Self::InplaceMatrixMultiply => "@=",
            Self::InplaceMultiply => "*=",
            Self::InplaceRemainder => "%=",
            Self::InplaceOr => "|=",
            Self::InplacePower => "**=",
            Self::InplaceRshift => ">>=",
            Self::InplaceSubtract => "-=",
            Self::InplaceTrueDivide => "/=",
            Self::InplaceXor => "^=",
            Self::Subscr => "[]",
        };
        write!(f, "{op}")
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
        Enter = 0,
        /// `__exit__` for sync context manager
        Exit = 1,
        /// `__aenter__` for async context manager
        AEnter = 2,
        /// `__aexit__` for async context manager
        AExit = 3,
    }
);

impl fmt::Display for SpecialMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let method_name = match self {
            Self::Enter => "__enter__",
            Self::Exit => "__exit__",
            Self::AEnter => "__aenter__",
            Self::AExit => "__aexit__",
        };
        write!(f, "{method_name}")
    }
}

oparg_enum!(
    /// Common constants for LOAD_COMMON_CONSTANT opcode.
    /// pycore_opcode_utils.h CONSTANT_*
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum CommonConstant {
        /// `AssertionError` exception type
        AssertionError = 0,
        /// `NotImplementedError` exception type
        NotImplementedError = 1,
        /// Built-in `tuple` type
        BuiltinTuple = 2,
        /// Built-in `all` function
        BuiltinAll = 3,
        /// Built-in `any` function
        BuiltinAny = 4,
        /// Built-in `list` type
        BuiltinList = 5,
        /// Built-in `set` type
        BuiltinSet = 6,
    }
);

impl fmt::Display for CommonConstant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::AssertionError => "AssertionError",
            Self::NotImplementedError => "NotImplementedError",
            Self::BuiltinTuple => "tuple",
            Self::BuiltinAll => "all",
            Self::BuiltinAny => "any",
            Self::BuiltinList => "list",
            Self::BuiltinSet => "set",
        };
        write!(f, "{name}")
    }
}

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
