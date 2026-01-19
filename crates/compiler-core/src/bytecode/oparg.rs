use bitflags::bitflags;

use core::{fmt, num::NonZeroU8};

use crate::bytecode::{CodeUnit, instruction::Instruction};

pub trait OpArgType: Copy {
    fn from_op_arg(x: u32) -> Option<Self>;

    fn to_op_arg(self) -> u32;
}

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

/// Oparg values for [`Instruction::ConvertValue`].
///
/// ## See also
///
/// - [CPython FVC_* flags](https://github.com/python/cpython/blob/8183fa5e3f78ca6ab862de7fb8b14f3d929421e0/Include/ceval.h#L129-L132)
#[repr(u8)]
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq)]
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

impl OpArgType for ConvertValueOparg {
    #[inline]
    fn from_op_arg(x: u32) -> Option<Self> {
        Some(match x {
            // Ruff `ConversionFlag::None` is `-1i8`,
            // when its converted to `u8` its value is `u8::MAX`
            0 | 255 => Self::None,
            1 => Self::Str,
            2 => Self::Repr,
            3 => Self::Ascii,
            _ => return None,
        })
    }

    #[inline]
    fn to_op_arg(self) -> u32 {
        self as u32
    }
}

/// Resume type for the RESUME instruction
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq)]
#[repr(u32)]
pub enum ResumeType {
    AtFuncStart = 0,
    AfterYield = 1,
    AfterYieldFrom = 2,
    AfterAwait = 3,
}

impl OpArgType for u32 {
    #[inline(always)]
    fn from_op_arg(x: u32) -> Option<Self> {
        Some(x)
    }

    #[inline(always)]
    fn to_op_arg(self) -> u32 {
        self
    }
}

impl OpArgType for bool {
    #[inline(always)]
    fn from_op_arg(x: u32) -> Option<Self> {
        Some(x != 0)
    }

    #[inline(always)]
    fn to_op_arg(self) -> u32 {
        self as u32
    }
}

macro_rules! op_arg_enum_impl {
    (enum $name:ident { $($(#[$var_attr:meta])* $var:ident = $value:literal,)* }) => {
        impl OpArgType for $name {
            fn to_op_arg(self) -> u32 {
                self as u32
            }

            fn from_op_arg(x: u32) -> Option<Self> {
                Some(match u8::try_from(x).ok()? {
                    $($value => Self::$var,)*
                    _ => return None,
                })
            }
        }
    };
}

macro_rules! op_arg_enum {
    ($(#[$attr:meta])* $vis:vis enum $name:ident { $($(#[$var_attr:meta])* $var:ident = $value:literal,)* }) => {
        $(#[$attr])*
        $vis enum $name {
            $($(#[$var_attr])* $var = $value,)*
        }

        op_arg_enum_impl!(enum $name {
            $($(#[$var_attr])* $var = $value,)*
        });
    };
}

pub type NameIdx = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
#[repr(transparent)]
pub struct Label(pub u32);

impl OpArgType for Label {
    #[inline(always)]
    fn from_op_arg(x: u32) -> Option<Self> {
        Some(Self(x))
    }

    #[inline(always)]
    fn to_op_arg(self) -> u32 {
        self.0
    }
}

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

op_arg_enum!(
    /// The kind of Raise that occurred.
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    #[repr(u8)]
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

op_arg_enum!(
    /// Intrinsic function for CALL_INTRINSIC_1
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    #[repr(u8)]
    pub enum IntrinsicFunction1 {
        // Invalid = 0,
        Print = 1,
        /// Import * operation
        ImportStar = 2,
        // StopIterationError = 3,
        // AsyncGenWrap = 4,
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

op_arg_enum!(
    /// Intrinsic function for CALL_INTRINSIC_2
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    #[repr(u8)]
    pub enum IntrinsicFunction2 {
        PrepReraiseStar = 1,
        TypeVarWithBound = 2,
        TypeVarWithConstraint = 3,
        SetFunctionTypeParams = 4,
        /// Set default value for type parameter (PEP 695)
        SetTypeparamDefault = 5,
    }
);

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

impl OpArgType for MakeFunctionFlags {
    #[inline(always)]
    fn from_op_arg(x: u32) -> Option<Self> {
        Self::from_bits(x as u8)
    }

    #[inline(always)]
    fn to_op_arg(self) -> u32 {
        self.bits().into()
    }
}

op_arg_enum!(
    /// The possible comparison operators
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    #[repr(u8)]
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
);

op_arg_enum!(
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
        };
        write!(f, "{op}")
    }
}

op_arg_enum!(
    /// Whether or not to invert the operation.
    #[repr(u8)]
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

impl OpArgType for BuildSliceArgCount {
    #[inline(always)]
    fn from_op_arg(x: u32) -> Option<Self> {
        Some(match x {
            2 => Self::Two,
            3 => Self::Three,
            _ => return None,
        })
    }

    #[inline(always)]
    fn to_op_arg(self) -> u32 {
        u32::from(self.argc().get())
    }
}

impl BuildSliceArgCount {
    /// Get the numeric value of `Self`.
    #[must_use]
    pub const fn argc(self) -> NonZeroU8 {
        let inner = match self {
            Self::Two => 2,
            Self::Three => 3,
        };
        // Safety: `inner` can be either 2 or 3.
        unsafe { NonZeroU8::new_unchecked(inner) }
    }
}

#[derive(Copy, Clone)]
pub struct UnpackExArgs {
    pub before: u8,
    pub after: u8,
}

impl OpArgType for UnpackExArgs {
    #[inline(always)]
    fn from_op_arg(x: u32) -> Option<Self> {
        let [before, after, ..] = x.to_le_bytes();
        Some(Self { before, after })
    }

    #[inline(always)]
    fn to_op_arg(self) -> u32 {
        u32::from_le_bytes([self.before, self.after, 0, 0])
    }
}

impl fmt::Display for UnpackExArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "before: {}, after: {}", self.before, self.after)
    }
}
