use core::fmt;

use crate as rustpython_compiler_core; // Required for newtype_oparg macro
use rustpython_macros::newtype_oparg;

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

/// Oparg values for [`Instruction::ConvertValue`].
///
/// ## See also
///
/// - [CPython FVC_* flags](https://github.com/python/cpython/blob/8183fa5e3f78ca6ab862de7fb8b14f3d929421e0/Include/ceval.h#L129-L132)
#[newtype_oparg]
pub enum ConvertValueOparg {
    /// No conversion.
    ///
    /// ```python
    /// f"{x}"
    /// f"{x:4}"
    /// ```
    #[oparg(display = "")]
    None = 0,
    /// Converts by calling `str(<value>)`.
    ///
    /// ```python
    /// f"{x!s}"
    /// f"{x!s:2}"
    /// ```
    #[oparg(display = "1 (str)")]
    Str = 1,
    /// Converts by calling `repr(<value>)`.
    ///
    /// ```python
    /// f"{x!r}"
    /// f"{x!r:2}"
    /// ```
    #[oparg(display = "2 (repr)")]
    Repr = 2,
    /// Converts by calling `ascii(<value>)`.
    ///
    /// ```python
    /// f"{x!a}"
    /// f"{x!a:2}"
    /// ```
    #[oparg(display = "3 (ascii)")]
    Ascii = 3,
}

/// Resume type for the RESUME instruction
#[newtype_oparg]
pub enum ResumeType {
    AtFuncStart = 0,
    AfterYield = 1,
    AfterYieldFrom = 2,
    AfterAwait = 3,
    #[oparg(catch_all)]
    Other(u32),
}

pub type NameIdx = u32;

impl OpArgType for u32 {}

/// The kind of Raise that occurred.
#[newtype_oparg]
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

#[newtype_oparg]
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

/// Intrinsic function for CALL_INTRINSIC_2
#[newtype_oparg]
pub enum IntrinsicFunction2 {
    PrepReraiseStar = 1,
    TypeVarWithBound = 2,
    TypeVarWithConstraint = 3,
    SetFunctionTypeParams = 4,
    /// Set default value for type parameter (PEP 695)
    SetTypeparamDefault = 5,
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

/// `COMPARE_OP` arg is `(cmp_index << 5) | mask`.  Only the upper
/// 3 bits identify the comparison; the lower 5 bits are an inline
/// cache mask for adaptive specialization.
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
    /// Encode as `cmp_index << 5` (mask bits zero).
    fn from(value: ComparisonOperator) -> Self {
        match value {
            ComparisonOperator::Less => 0,
            ComparisonOperator::LessOrEqual => 1 << 5,
            ComparisonOperator::Equal => 2 << 5,
            ComparisonOperator::NotEqual => 3 << 5,
            ComparisonOperator::Greater => 4 << 5,
            ComparisonOperator::GreaterOrEqual => 5 << 5,
        }
    }
}

impl From<ComparisonOperator> for u32 {
    fn from(value: ComparisonOperator) -> Self {
        Self::from(u8::from(value))
    }
}

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
#[newtype_oparg]
pub enum BinaryOperator {
    /// `+`
    #[oparg(display = "+")]
    Add = 0,
    /// `&`
    #[oparg(display = "&")]
    And = 1,
    /// `//`
    #[oparg(display = "//")]
    FloorDivide = 2,
    /// `<<`
    #[oparg(display = "<<")]
    Lshift = 3,
    /// `@`
    #[oparg(display = "@")]
    MatrixMultiply = 4,
    /// `*`
    #[oparg(display = "*")]
    Multiply = 5,
    /// `%`
    #[oparg(display = "%")]
    Remainder = 6,
    /// `|`
    #[oparg(display = "|")]
    Or = 7,
    /// `**`
    #[oparg(display = "**")]
    Power = 8,
    /// `>>`
    #[oparg(display = ">>")]
    Rshift = 9,
    /// `-`
    #[oparg(display = "-")]
    Subtract = 10,
    /// `/`
    #[oparg(display = "/")]
    TrueDivide = 11,
    /// `^`
    #[oparg(display = "^")]
    Xor = 12,
    /// `+=`
    #[oparg(display = "+=")]
    InplaceAdd = 13,
    /// `&=`
    #[oparg(display = "&=")]
    InplaceAnd = 14,
    /// `//=`
    #[oparg(display = "//=")]
    InplaceFloorDivide = 15,
    /// `<<=`
    #[oparg(display = "<<=")]
    InplaceLshift = 16,
    /// `@=`
    #[oparg(display = "@=")]
    InplaceMatrixMultiply = 17,
    /// `*=`
    #[oparg(display = "*=")]
    InplaceMultiply = 18,
    /// `%=`
    #[oparg(display = "%=")]
    InplaceRemainder = 19,
    /// `|=`
    #[oparg(display = "|=")]
    InplaceOr = 20,
    /// `**=`
    #[oparg(display = "**=")]
    InplacePower = 21,
    /// `>>=`
    #[oparg(display = ">>=")]
    InplaceRshift = 22,
    /// `-=`
    #[oparg(display = "-=")]
    InplaceSubtract = 23,
    /// `/=`
    #[oparg(display = "/=")]
    InplaceTrueDivide = 24,
    /// `^=`
    #[oparg(display = "^=")]
    InplaceXor = 25,
    /// `[]` subscript
    #[oparg(display = "[]")]
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

/// Whether or not to invert the operation.
#[newtype_oparg]
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

/// Special method for LOAD_SPECIAL opcode (context managers).
#[newtype_oparg]
pub enum SpecialMethod {
    /// `__enter__` for sync context manager
    #[oparg(display = "__enter__")]
    Enter = 0,
    /// `__exit__` for sync context manager
    #[oparg(display = "__exit__")]
    Exit = 1,
    /// `__aenter__` for async context manager
    #[oparg(display = "__aenter__")]
    AEnter = 2,
    /// `__aexit__` for async context manager
    #[oparg(display = "__aexit__")]
    AExit = 3,
}

/// Common constants for LOAD_COMMON_CONSTANT opcode.
/// pycore_opcode_utils.h CONSTANT_*
#[newtype_oparg]
pub enum CommonConstant {
    /// `AssertionError` exception type
    #[oparg(display = "AssertionError")]
    AssertionError = 0,
    /// `NotImplementedError` exception type
    #[oparg(display = "NotImplementedError")]
    NotImplementedError = 1,
    /// Built-in `tuple` type
    #[oparg(display = "tuple")]
    BuiltinTuple = 2,
    /// Built-in `all` function
    #[oparg(display = "all")]
    BuiltinAll = 3,
    /// Built-in `any` function
    #[oparg(display = "any")]
    BuiltinAny = 4,
}

#[newtype_oparg]
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

#[newtype_oparg]
pub struct ConstIdx;

#[newtype_oparg]
pub struct VarNum;

#[newtype_oparg]
pub struct VarNums;

#[newtype_oparg]
pub struct LoadAttr;

#[newtype_oparg]
pub struct LoadSuperAttr;

#[newtype_oparg]
pub struct Label;

impl VarNums {
    #[must_use]
    pub const fn idx_1(self) -> VarNum {
        VarNum::new(self.0 >> 4)
    }

    #[must_use]
    pub const fn idx_2(self) -> VarNum {
        VarNum::new(self.0 & 15)
    }

    #[must_use]
    pub const fn indexes(self) -> (VarNum, VarNum) {
        (self.idx_1(), self.idx_2())
    }
}

impl LoadAttr {
    #[must_use]
    pub fn builder() -> LoadAttrBuilder {
        LoadAttrBuilder::default()
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

#[derive(Clone, Copy, Default)]
pub struct LoadAttrBuilder {
    name_idx: u32,
    is_method: bool,
}

impl LoadAttrBuilder {
    #[must_use]
    pub const fn build(self) -> LoadAttr {
        let value = (self.name_idx << 1) | (self.is_method as u32);
        LoadAttr::new(value)
    }

    #[must_use]
    pub const fn name_idx(mut self, value: u32) -> Self {
        self.name_idx = value;
        self
    }

    #[must_use]
    pub const fn is_method(mut self, value: bool) -> Self {
        self.is_method = value;
        self
    }
}

impl LoadSuperAttr {
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
