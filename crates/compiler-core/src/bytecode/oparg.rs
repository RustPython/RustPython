use bitflags::bitflags;
use num_enum::{IntoPrimitive, TryFromPrimitive};

use core::fmt;

use crate::{
    bytecode::{CodeUnit, instruction::Instruction},
    marshal::MarshalError,
};

pub trait OpArgType: Copy + Into<u32> + TryFrom<u32> {}

/// Opcode argument that may be extended by a prior ExtendedArg.
#[derive(Copy, Clone, PartialEq, Eq)]
#[repr(transparent)]
pub struct OpArgByte(pub u8);

impl OpArgByte {
    pub const fn null() -> Self {
        Self(0)
    }
}

impl From<u8> for OpArgByte {
    fn from(raw: u8) -> Self {
        Self(raw)
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
pub struct OpArg(pub u32);

impl OpArg {
    pub const fn null() -> Self {
        Self(0)
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
        Self(raw)
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
        OpArg(self.state)
    }

    #[inline(always)]
    pub const fn reset(&mut self) {
        self.state = 0
    }
}

macro_rules! impl_oparg_enum_traits {
    ($name:ty) => {
        impl From<$name> for u32 {
            fn from(value: $name) -> Self {
                Self::from(u8::from(value))
            }
        }

        impl TryFrom<u32> for $name {
            type Error = $crate::marshal::MarshalError;

            fn try_from(value: u32) -> Result<Self, Self::Error> {
                u8::try_from(value)
                    .map_err(|_| Self::Error::InvalidBytecode)
                    .map(TryInto::try_into)?
            }
        }
    };
}

/// Oparg values for [`Instruction::ConvertValue`].
///
/// ## See also
///
/// - [CPython FVC_* flags](https://github.com/python/cpython/blob/8183fa5e3f78ca6ab862de7fb8b14f3d929421e0/Include/ceval.h#L129-L132)
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Hash, IntoPrimitive, PartialEq, TryFromPrimitive)]
#[num_enum(error_type(name = MarshalError, constructor = new_invalid_bytecode))]
pub enum ConvertValueOparg {
    /// No conversion.
    ///
    /// ```python
    /// f"{x}"
    /// f"{x:4}"
    /// ```
    // Ruff `ConversionFlag::None` is `-1i8`, when its converted to `u8` its value is `u8::MAX`.
    #[num_enum(alternatives = [255])]
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

impl_oparg_enum_traits!(ConvertValueOparg);

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

impl OpArgType for ConvertValueOparg {}

/// Resume type for the RESUME instruction
#[repr(u8)]
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, IntoPrimitive, TryFromPrimitive)]
#[num_enum(error_type(name = MarshalError, constructor = new_invalid_bytecode))]
pub enum ResumeType {
    AtFuncStart = 0,
    AfterYield = 1,
    AfterYieldFrom = 2,
    AfterAwait = 3,
}

pub type NameIdx = u32;

impl OpArgType for u32 {}
//impl OpArgType for bool {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
#[repr(transparent)]
pub struct Label(pub u32);

impl Label {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }
}

impl From<u32> for Label {
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

impl From<Label> for u32 {
    fn from(value: Label) -> Self {
        value.0
    }
}

impl OpArgType for Label {}

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The kind of Raise that occurred.
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, TryFromPrimitive, IntoPrimitive, Eq)]
#[num_enum(error_type(name = MarshalError, constructor = new_invalid_bytecode))]
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

impl_oparg_enum_traits!(RaiseKind);
impl OpArgType for RaiseKind {}

/// Intrinsic function for CALL_INTRINSIC_1
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, IntoPrimitive, TryFromPrimitive)]
#[num_enum(error_type(name = MarshalError, constructor = new_invalid_bytecode))]
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

impl_oparg_enum_traits!(IntrinsicFunction1);
impl OpArgType for IntrinsicFunction1 {}

/// Intrinsic function for CALL_INTRINSIC_2
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, TryFromPrimitive, IntoPrimitive)]
#[num_enum(error_type(name = MarshalError, constructor = new_invalid_bytecode))]
pub enum IntrinsicFunction2 {
    PrepReraiseStar = 1,
    TypeVarWithBound = 2,
    TypeVarWithConstraint = 3,
    SetFunctionTypeParams = 4,
    /// Set default value for type parameter (PEP 695)
    SetTypeparamDefault = 5,
}

impl_oparg_enum_traits!(IntrinsicFunction2);
impl OpArgType for IntrinsicFunction2 {}

bitflags! {
    #[derive(Copy, Clone, Debug, PartialEq)]
    pub struct MakeFunctionFlags: u8 {
        const CLOSURE = 0x01;
        const ANNOTATIONS = 0x02;
        const KW_ONLY_DEFAULTS = 0x04;
        const DEFAULTS = 0x08;
        const TYPE_PARAMS = 0x10;
        /// PEP 649: __annotate__ function closure (instead of __annotations__ dict)
        const ANNOTATE = 0x20;
    }
}

impl TryFrom<u32> for MakeFunctionFlags {
    type Error = MarshalError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::from_bits(value as u8).ok_or(Self::Error::InvalidBytecode)
    }
}

impl From<MakeFunctionFlags> for u32 {
    fn from(value: MakeFunctionFlags) -> Self {
        value.bits().into()
    }
}

impl OpArgType for MakeFunctionFlags {}

/// The possible comparison operators.
#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, TryFromPrimitive, IntoPrimitive)]
#[num_enum(error_type(name = MarshalError, constructor = new_invalid_bytecode))]
pub enum ComparisonOperator {
    // be intentional with bits so that we can do eval_ord with just a bitwise and
    // bits: | Equal | Greater | Less |
    Less = 0b001,
    Greater = 0b010,
    NotEqual = 0b011,
    Equal = 0b100,
    LessOrEqual = 0b101,
    GreaterOrEqual = 0b110,
}

impl_oparg_enum_traits!(ComparisonOperator);
impl OpArgType for ComparisonOperator {}

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
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromPrimitive, IntoPrimitive)]
#[num_enum(error_type(name = MarshalError, constructor = new_invalid_bytecode))]
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

impl_oparg_enum_traits!(BinaryOperator);
impl OpArgType for BinaryOperator {}

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

/// Whether or not to invert the operation.
#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, TryFromPrimitive, IntoPrimitive)]
#[num_enum(error_type(name = MarshalError, constructor = new_invalid_bytecode))]
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

impl_oparg_enum_traits!(Invert);
impl OpArgType for Invert {}

/// Special method for LOAD_SPECIAL opcode (context managers).
#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, TryFromPrimitive, IntoPrimitive)]
#[num_enum(error_type(name = MarshalError, constructor = new_invalid_bytecode))]
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

impl_oparg_enum_traits!(SpecialMethod);
impl OpArgType for SpecialMethod {}

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

/// Common constants for LOAD_COMMON_CONSTANT opcode.
/// pycore_opcode_utils.h CONSTANT_*
#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, TryFromPrimitive, IntoPrimitive)]
#[num_enum(error_type(name = MarshalError, constructor = new_invalid_bytecode))]
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
}

impl_oparg_enum_traits!(CommonConstant);
impl OpArgType for CommonConstant {}

impl fmt::Display for CommonConstant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::AssertionError => "AssertionError",
            Self::NotImplementedError => "NotImplementedError",
            Self::BuiltinTuple => "tuple",
            Self::BuiltinAll => "all",
            Self::BuiltinAny => "any",
        };
        write!(f, "{name}")
    }
}

/// Specifies if a slice is built with either 2 or 3 arguments.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuildSliceArgCount {
    /// ```py
    /// x[5:10]
    /// ```
    Two,
    /// ```py
    /// x[5:10:2]
    /// ```
    Three,
}

impl TryFrom<u8> for BuildSliceArgCount {
    type Error = MarshalError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(match value {
            2 => Self::Two,
            3 => Self::Three,
            _ => return Err(Self::Error::InvalidBytecode),
        })
    }
}

impl TryFrom<u32> for BuildSliceArgCount {
    type Error = MarshalError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        u8::try_from(value)
            .map_err(|_| Self::Error::InvalidBytecode)
            .map(TryInto::try_into)?
    }
}

impl From<BuildSliceArgCount> for u32 {
    fn from(value: BuildSliceArgCount) -> Self {
        match value {
            BuildSliceArgCount::Two => 2,
            BuildSliceArgCount::Three => 3,
        }
    }
}

impl OpArgType for BuildSliceArgCount {}

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

#[derive(Clone, Copy)]
pub struct LoadSuperAttr(u32);

impl LoadSuperAttr {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub fn builder() -> LoadSuperAttrBuilder {
        LoadSuperAttrBuilder::default()
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

impl OpArgType for LoadSuperAttr {}

impl From<u32> for LoadSuperAttr {
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

impl From<LoadSuperAttr> for u32 {
    fn from(value: LoadSuperAttr) -> Self {
        value.0
    }
}

#[derive(Clone, Copy, Default)]
pub struct LoadSuperAttrBuilder {
    name_idx: u32,
    is_load_method: bool,
    has_class: bool,
}

impl LoadSuperAttrBuilder {
    #[must_use]
    pub const fn build(self) -> LoadSuperAttr {
        let value =
            (self.name_idx << 2) | ((self.has_class as u32) << 1) | (self.is_load_method as u32);
        LoadSuperAttr::new(value)
    }

    #[must_use]
    pub const fn name_idx(mut self, value: u32) -> Self {
        self.name_idx = value;
        self
    }

    #[must_use]
    pub const fn is_load_method(mut self, value: bool) -> Self {
        self.is_load_method = value;
        self
    }

    #[must_use]
    pub const fn has_class(mut self, value: bool) -> Self {
        self.has_class = value;
        self
    }
}

impl From<LoadSuperAttrBuilder> for LoadSuperAttr {
    fn from(builder: LoadSuperAttrBuilder) -> Self {
        builder.build()
    }
}

/// Helper function for `num_enum` derive macro.
///
/// # Examples
///
/// ```ignore
/// use num_enum::TryFromPrimitive;
///
/// use rustpython_compiler_core::marshal::MarshalError;
///
/// #[repr(u8)]
/// #[derive(TryFromPrimitive)]
/// #[num_enum(error_type(name = MarshalError, constructor = new_invalid_bytecode))]
/// enum Foo {
///   A = 1,
///   B = 2
/// }
fn new_invalid_bytecode<T>(_: T) -> MarshalError {
    MarshalError::InvalidBytecode
}
