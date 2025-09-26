//! Implement python as a virtual machine with bytecode. This module
//! implements bytecode structure.

use crate::{OneIndexed, SourceLocation};
use bitflags::bitflags;
use itertools::Itertools;
use malachite_bigint::BigInt;
use num_complex::Complex64;
use rustpython_wtf8::{Wtf8, Wtf8Buf};
use std::{collections::BTreeSet, fmt, hash, marker::PhantomData, mem};

pub use crate::instruction::{Instruction, NameIdx};

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq)]
#[repr(i8)]
#[allow(clippy::cast_possible_wrap)]
pub enum ConversionFlag {
    /// No conversion
    None = -1, // CPython uses -1
    /// Converts by calling `str(<value>)`.
    Str = b's' as i8,
    /// Converts by calling `ascii(<value>)`.
    Ascii = b'a' as i8,
    /// Converts by calling `repr(<value>)`.
    Repr = b'r' as i8,
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

/// CPython 3.11+ linetable location info codes
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PyCodeLocationInfoKind {
    // Short forms are 0 to 9
    Short0 = 0,
    Short1 = 1,
    Short2 = 2,
    Short3 = 3,
    Short4 = 4,
    Short5 = 5,
    Short6 = 6,
    Short7 = 7,
    Short8 = 8,
    Short9 = 9,
    // One line forms are 10 to 12
    OneLine0 = 10,
    OneLine1 = 11,
    OneLine2 = 12,
    NoColumns = 13,
    Long = 14,
    None = 15,
}

impl PyCodeLocationInfoKind {
    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Short0),
            1 => Some(Self::Short1),
            2 => Some(Self::Short2),
            3 => Some(Self::Short3),
            4 => Some(Self::Short4),
            5 => Some(Self::Short5),
            6 => Some(Self::Short6),
            7 => Some(Self::Short7),
            8 => Some(Self::Short8),
            9 => Some(Self::Short9),
            10 => Some(Self::OneLine0),
            11 => Some(Self::OneLine1),
            12 => Some(Self::OneLine2),
            13 => Some(Self::NoColumns),
            14 => Some(Self::Long),
            15 => Some(Self::None),
            _ => Option::None,
        }
    }

    pub fn is_short(&self) -> bool {
        (*self as u8) <= 9
    }

    pub fn short_column_group(&self) -> Option<u8> {
        if self.is_short() {
            Some(*self as u8)
        } else {
            Option::None
        }
    }

    pub fn one_line_delta(&self) -> Option<i32> {
        match self {
            Self::OneLine0 => Some(0),
            Self::OneLine1 => Some(1),
            Self::OneLine2 => Some(2),
            _ => Option::None,
        }
    }
}

pub trait Constant: Sized {
    type Name: AsRef<str>;

    /// Transforms the given Constant to a BorrowedConstant
    fn borrow_constant(&self) -> BorrowedConstant<'_, Self>;
}

impl Constant for ConstantData {
    type Name = String;

    fn borrow_constant(&self) -> BorrowedConstant<'_, Self> {
        use BorrowedConstant::*;

        match self {
            Self::Integer { value } => Integer { value },
            Self::Float { value } => Float { value: *value },
            Self::Complex { value } => Complex { value: *value },
            Self::Boolean { value } => Boolean { value: *value },
            Self::Str { value } => Str { value },
            Self::Bytes { value } => Bytes { value },
            Self::Code { code } => Code { code },
            Self::Tuple { elements } => Tuple { elements },
            Self::None => None,
            Self::Ellipsis => Ellipsis,
        }
    }
}

/// A Constant Bag
pub trait ConstantBag: Sized + std::marker::Copy {
    type Constant: Constant;

    fn make_constant<C: Constant>(&self, constant: BorrowedConstant<'_, C>) -> Self::Constant;

    fn make_int(&self, value: BigInt) -> Self::Constant;

    fn make_tuple(&self, elements: impl Iterator<Item = Self::Constant>) -> Self::Constant;

    fn make_code(&self, code: CodeObject<Self::Constant>) -> Self::Constant;

    fn make_name(&self, name: &str) -> <Self::Constant as Constant>::Name;
}

pub trait AsBag {
    type Bag: ConstantBag;

    #[allow(clippy::wrong_self_convention)]
    fn as_bag(self) -> Self::Bag;
}

impl<Bag: ConstantBag> AsBag for Bag {
    type Bag = Self;

    fn as_bag(self) -> Self {
        self
    }
}

#[derive(Clone, Copy)]
pub struct BasicBag;

impl ConstantBag for BasicBag {
    type Constant = ConstantData;

    fn make_constant<C: Constant>(&self, constant: BorrowedConstant<'_, C>) -> Self::Constant {
        constant.to_owned()
    }

    fn make_int(&self, value: BigInt) -> Self::Constant {
        ConstantData::Integer { value }
    }

    fn make_tuple(&self, elements: impl Iterator<Item = Self::Constant>) -> Self::Constant {
        ConstantData::Tuple {
            elements: elements.collect(),
        }
    }

    fn make_code(&self, code: CodeObject<Self::Constant>) -> Self::Constant {
        ConstantData::Code {
            code: Box::new(code),
        }
    }

    fn make_name(&self, name: &str) -> <Self::Constant as Constant>::Name {
        name.to_owned()
    }
}

/// Primary container of a single code object. Each python function has
/// a code object. Also a module has a code object.
#[derive(Clone)]
pub struct CodeObject<C: Constant = ConstantData> {
    pub instructions: Box<[CodeUnit]>,
    pub locations: Box<[SourceLocation]>,
    pub flags: CodeFlags,
    /// Number of positional-only arguments
    pub posonlyarg_count: u32,
    pub arg_count: u32,
    pub kwonlyarg_count: u32,
    pub source_path: C::Name,
    pub first_line_number: Option<OneIndexed>,
    pub max_stackdepth: u32,
    /// Name of the object that created this code object
    pub obj_name: C::Name,
    /// Qualified name of the object (like CPython's co_qualname)
    pub qualname: C::Name,
    pub cell2arg: Option<Box<[i32]>>,
    pub constants: Box<[C]>,
    pub names: Box<[C::Name]>,
    pub varnames: Box<[C::Name]>,
    pub cellvars: Box<[C::Name]>,
    pub freevars: Box<[C::Name]>,
    /// Line number table (CPython 3.11+ format)
    pub linetable: Box<[u8]>,
    /// Exception handling table
    pub exceptiontable: Box<[u8]>,
}

bitflags! {
    #[derive(Copy, Clone, Debug, PartialEq)]
    pub struct CodeFlags: u16 {
        const NEW_LOCALS = 0x01;
        const IS_GENERATOR = 0x02;
        const IS_COROUTINE = 0x04;
        const HAS_VARARGS = 0x08;
        const HAS_VARKEYWORDS = 0x10;
        const IS_OPTIMIZED = 0x20;
    }
}

impl CodeFlags {
    pub const NAME_MAPPING: &'static [(&'static str, Self)] = &[
        ("GENERATOR", Self::IS_GENERATOR),
        ("COROUTINE", Self::IS_COROUTINE),
        (
            "ASYNC_GENERATOR",
            Self::from_bits_truncate(Self::IS_GENERATOR.bits() | Self::IS_COROUTINE.bits()),
        ),
        ("VARARGS", Self::HAS_VARARGS),
        ("VARKEYWORDS", Self::HAS_VARKEYWORDS),
    ];
}

/// an opcode argument that may be extended by a prior ExtendedArg
#[derive(Copy, Clone, PartialEq, Eq)]
#[repr(transparent)]
pub struct OpArgByte(pub u8);

impl OpArgByte {
    pub const fn null() -> Self {
        Self(0)
    }
}

impl fmt::Debug for OpArgByte {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// a full 32-bit op_arg, including any possible ExtendedArg extension
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
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
        if !matches!(ins.op, Instruction::ExtendedArg(_)) {
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

pub trait OpArgType: std::marker::Copy {
    fn from_op_arg(x: u32) -> Option<Self>;

    fn to_op_arg(self) -> u32;
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
        write!(f, "Arg<{}>", std::any::type_name::<T>())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
#[repr(transparent)]
// XXX: if you add a new instruction that stores a Label, make sure to add it in
// Instruction::label_arg
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

impl OpArgType for ConversionFlag {
    #[inline]
    fn from_op_arg(x: u32) -> Option<Self> {
        match x as u8 {
            b's' => Some(Self::Str),
            b'a' => Some(Self::Ascii),
            b'r' => Some(Self::Repr),
            std::u8::MAX => Some(Self::None),
            _ => None,
        }
    }

    #[inline]
    fn to_op_arg(self) -> u32 {
        self as i8 as u8 as u32
    }
}

op_arg_enum!(
    /// The kind of Raise that occurred.
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    #[repr(u8)]
    pub enum RaiseKind {
        Reraise = 0,
        Raise = 1,
        RaiseCause = 2,
    }
);

op_arg_enum!(
    /// Intrinsic function for CALL_INTRINSIC_1
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    #[repr(u8)]
    pub enum IntrinsicFunction1 {
        // Invalid = 0,
        // Print = 1,
        /// Import * operation
        ImportStar = 2,
        // StopIterationError = 3,
        // AsyncGenWrap = 4,
        // UnaryPositive = 5,
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
        // PrepReraiseS tar = 1,
        TypeVarWithBound = 2,
        TypeVarWithConstraint = 3,
        SetFunctionTypeParams = 4,
        /// Set default value for type parameter (PEP 695)
        SetTypeparamDefault = 5,
    }
);

#[derive(Copy, Clone)]
#[repr(C)]
pub struct CodeUnit {
    pub op: Instruction,
    pub arg: OpArgByte,
}

// TODO: Uncomment this
// const _: () = assert!(mem::size_of::<CodeUnit>() == 2);

impl CodeUnit {
    pub const fn new(op: Instruction, arg: OpArgByte) -> Self {
        Self { op, arg }
    }
}

bitflags! {
    #[derive(Copy, Clone, Debug, PartialEq)]
    pub struct MakeFunctionFlags: u8 {
        const CLOSURE = 0x01;
        const ANNOTATIONS = 0x02;
        const KW_ONLY_DEFAULTS = 0x04;
        const DEFAULTS = 0x08;
        const TYPE_PARAMS = 0x10;
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

/// A Constant (which usually encapsulates data within it)
///
/// # Examples
/// ```
/// use rustpython_compiler_core::bytecode::ConstantData;
/// let a = ConstantData::Float {value: 120f64};
/// let b = ConstantData::Boolean {value: false};
/// assert_ne!(a, b);
/// ```
#[derive(Debug, Clone)]
pub enum ConstantData {
    Tuple { elements: Vec<ConstantData> },
    Integer { value: BigInt },
    Float { value: f64 },
    Complex { value: Complex64 },
    Boolean { value: bool },
    Str { value: Wtf8Buf },
    Bytes { value: Vec<u8> },
    Code { code: Box<CodeObject> },
    None,
    Ellipsis,
}

impl PartialEq for ConstantData {
    fn eq(&self, other: &Self) -> bool {
        use ConstantData::*;

        match (self, other) {
            (Integer { value: a }, Integer { value: b }) => a == b,
            // we want to compare floats *by actual value* - if we have the *exact same* float
            // already in a constant cache, we want to use that
            (Float { value: a }, Float { value: b }) => a.to_bits() == b.to_bits(),
            (Complex { value: a }, Complex { value: b }) => {
                a.re.to_bits() == b.re.to_bits() && a.im.to_bits() == b.im.to_bits()
            }
            (Boolean { value: a }, Boolean { value: b }) => a == b,
            (Str { value: a }, Str { value: b }) => a == b,
            (Bytes { value: a }, Bytes { value: b }) => a == b,
            (Code { code: a }, Code { code: b }) => std::ptr::eq(a.as_ref(), b.as_ref()),
            (Tuple { elements: a }, Tuple { elements: b }) => a == b,
            (None, None) => true,
            (Ellipsis, Ellipsis) => true,
            _ => false,
        }
    }
}

impl Eq for ConstantData {}

impl hash::Hash for ConstantData {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        use ConstantData::*;

        mem::discriminant(self).hash(state);
        match self {
            Integer { value } => value.hash(state),
            Float { value } => value.to_bits().hash(state),
            Complex { value } => {
                value.re.to_bits().hash(state);
                value.im.to_bits().hash(state);
            }
            Boolean { value } => value.hash(state),
            Str { value } => value.hash(state),
            Bytes { value } => value.hash(state),
            Code { code } => std::ptr::hash(code.as_ref(), state),
            Tuple { elements } => elements.hash(state),
            None => {}
            Ellipsis => {}
        }
    }
}

/// A borrowed Constant
pub enum BorrowedConstant<'a, C: Constant> {
    Integer { value: &'a BigInt },
    Float { value: f64 },
    Complex { value: Complex64 },
    Boolean { value: bool },
    Str { value: &'a Wtf8 },
    Bytes { value: &'a [u8] },
    Code { code: &'a CodeObject<C> },
    Tuple { elements: &'a [C] },
    None,
    Ellipsis,
}

impl<C: Constant> std::marker::Copy for BorrowedConstant<'_, C> {}

impl<C: Constant> Clone for BorrowedConstant<'_, C> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<C: Constant> BorrowedConstant<'_, C> {
    pub fn fmt_display(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BorrowedConstant::Integer { value } => write!(f, "{value}"),
            BorrowedConstant::Float { value } => write!(f, "{value}"),
            BorrowedConstant::Complex { value } => write!(f, "{value}"),
            BorrowedConstant::Boolean { value } => {
                write!(f, "{}", if *value { "True" } else { "False" })
            }
            BorrowedConstant::Str { value } => write!(f, "{value:?}"),
            BorrowedConstant::Bytes { value } => write!(f, r#"b"{}""#, value.escape_ascii()),
            BorrowedConstant::Code { code } => write!(f, "{code:?}"),
            BorrowedConstant::Tuple { elements } => {
                write!(f, "(")?;
                let mut first = true;
                for c in *elements {
                    if first {
                        first = false
                    } else {
                        write!(f, ", ")?;
                    }
                    c.borrow_constant().fmt_display(f)?;
                }
                write!(f, ")")
            }
            BorrowedConstant::None => write!(f, "None"),
            BorrowedConstant::Ellipsis => write!(f, "..."),
        }
    }

    pub fn to_owned(self) -> ConstantData {
        use ConstantData::*;

        match self {
            BorrowedConstant::Integer { value } => Integer {
                value: value.clone(),
            },
            BorrowedConstant::Float { value } => Float { value },
            BorrowedConstant::Complex { value } => Complex { value },
            BorrowedConstant::Boolean { value } => Boolean { value },
            BorrowedConstant::Str { value } => Str {
                value: value.to_owned(),
            },
            BorrowedConstant::Bytes { value } => Bytes {
                value: value.to_owned(),
            },
            BorrowedConstant::Code { code } => Code {
                code: Box::new(code.map_clone_bag(&BasicBag)),
            },
            BorrowedConstant::Tuple { elements } => Tuple {
                elements: elements
                    .iter()
                    .map(|c| c.borrow_constant().to_owned())
                    .collect(),
            },
            BorrowedConstant::None => None,
            BorrowedConstant::Ellipsis => Ellipsis,
        }
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
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    #[repr(u8)]
    pub enum TestOperator {
        In = 0,
        NotIn = 1,
        Is = 2,
        IsNot = 3,
        /// two exceptions that match?
        ExceptionMatch = 4,
    }
);

op_arg_enum!(
    /// The possible Binary operators
    /// # Examples
    ///
    /// ```ignore
    /// use rustpython_compiler_core::Instruction::BinaryOperation;
    /// use rustpython_compiler_core::BinaryOperator::Add;
    /// let op = BinaryOperation {op: Add};
    /// ```
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    #[repr(u8)]
    pub enum BinaryOperator {
        Power = 0,
        Multiply = 1,
        MatrixMultiply = 2,
        Divide = 3,
        FloorDivide = 4,
        Modulo = 5,
        Add = 6,
        Subtract = 7,
        Lshift = 8,
        Rshift = 9,
        And = 10,
        Xor = 11,
        Or = 12,
    }
);

op_arg_enum!(
    /// The possible unary operators
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    #[repr(u8)]
    pub enum UnaryOperator {
        Not = 0,
        Invert = 1,
        Minus = 2,
        Plus = 3,
    }
);

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

/*
Maintain a stack of blocks on the VM.
pub enum BlockType {
    Loop,
    Except,
}
*/

/// Argument structure
pub struct Arguments<'a, N: AsRef<str>> {
    pub posonlyargs: &'a [N],
    pub args: &'a [N],
    pub vararg: Option<&'a N>,
    pub kwonlyargs: &'a [N],
    pub varkwarg: Option<&'a N>,
}

impl<N: AsRef<str>> fmt::Debug for Arguments<'_, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        macro_rules! fmt_slice {
            ($x:expr) => {
                format_args!("[{}]", $x.iter().map(AsRef::as_ref).format(", "))
            };
        }
        f.debug_struct("Arguments")
            .field("posonlyargs", &fmt_slice!(self.posonlyargs))
            .field("args", &fmt_slice!(self.posonlyargs))
            .field("vararg", &self.vararg.map(N::as_ref))
            .field("kwonlyargs", &fmt_slice!(self.kwonlyargs))
            .field("varkwarg", &self.varkwarg.map(N::as_ref))
            .finish()
    }
}

impl<C: Constant> CodeObject<C> {
    /// Get all arguments of the code object
    /// like inspect.getargs
    pub fn arg_names(&self) -> Arguments<'_, C::Name> {
        let nargs = self.arg_count as usize;
        let nkwargs = self.kwonlyarg_count as usize;
        let mut varargs_pos = nargs + nkwargs;
        let posonlyargs = &self.varnames[..self.posonlyarg_count as usize];
        let args = &self.varnames[..nargs];
        let kwonlyargs = &self.varnames[nargs..varargs_pos];

        let vararg = if self.flags.contains(CodeFlags::HAS_VARARGS) {
            let vararg = &self.varnames[varargs_pos];
            varargs_pos += 1;
            Some(vararg)
        } else {
            None
        };
        let varkwarg = if self.flags.contains(CodeFlags::HAS_VARKEYWORDS) {
            Some(&self.varnames[varargs_pos])
        } else {
            None
        };

        Arguments {
            posonlyargs,
            args,
            vararg,
            kwonlyargs,
            varkwarg,
        }
    }

    /// Return the labels targeted by the instructions of this CodeObject
    pub fn label_targets(&self) -> BTreeSet<Label> {
        let mut label_targets = BTreeSet::new();
        let mut arg_state = OpArgState::default();
        for instruction in &*self.instructions {
            let (instruction, arg) = arg_state.get(*instruction);
            match instruction {
                // TODO: Put more instructions here
                Instruction::Jump(l) => {
                    let label = Label(l.get(arg));
                    label_targets.insert(label);
                }
                _ => {}
            }
        }
        label_targets
    }

    fn display_inner(
        &self,
        f: &mut fmt::Formatter<'_>,
        expand_code_objects: bool,
        level: usize,
    ) -> fmt::Result {
        let label_targets = self.label_targets();
        let line_digits = (3).max(self.locations.last().unwrap().row.to_string().len());
        let offset_digits = (4).max(self.instructions.len().to_string().len());
        let mut last_line = OneIndexed::MAX;
        let mut arg_state = OpArgState::default();
        for (offset, &instruction) in self.instructions.iter().enumerate() {
            let (instruction, arg) = arg_state.get(instruction);
            // optional line number
            let line = self.locations[offset].row;
            if line != last_line {
                if last_line != OneIndexed::MAX {
                    writeln!(f)?;
                }
                last_line = line;
                write!(f, "{line:line_digits$}")?;
            } else {
                for _ in 0..line_digits {
                    write!(f, " ")?;
                }
            }
            write!(f, " ")?;

            // level indent
            for _ in 0..level {
                write!(f, "    ")?;
            }

            // arrow and offset
            let arrow = if label_targets.contains(&Label(offset as u32)) {
                ">>"
            } else {
                "  "
            };
            write!(f, "{arrow} {offset:offset_digits$} ")?;

            // instruction
            /*
            instruction.fmt_dis(arg, f, self, expand_code_objects, 21, level)?;
            writeln!(f)?;
            */
        }
        Ok(())
    }

    /// Recursively display this CodeObject
    pub fn display_expand_code_objects(&self) -> impl fmt::Display + '_ {
        struct Display<'a, C: Constant>(&'a CodeObject<C>);
        impl<C: Constant> fmt::Display for Display<'_, C> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.display_inner(f, true, 1)
            }
        }
        Display(self)
    }

    /// Map this CodeObject to one that holds a Bag::Constant
    pub fn map_bag<Bag: ConstantBag>(self, bag: Bag) -> CodeObject<Bag::Constant> {
        let map_names = |names: Box<[C::Name]>| {
            names
                .into_vec()
                .into_iter()
                .map(|x| bag.make_name(x.as_ref()))
                .collect::<Box<[_]>>()
        };
        CodeObject {
            constants: self
                .constants
                .into_vec()
                .into_iter()
                .map(|x| bag.make_constant(x.borrow_constant()))
                .collect(),
            names: map_names(self.names),
            varnames: map_names(self.varnames),
            cellvars: map_names(self.cellvars),
            freevars: map_names(self.freevars),
            source_path: bag.make_name(self.source_path.as_ref()),
            obj_name: bag.make_name(self.obj_name.as_ref()),
            qualname: bag.make_name(self.qualname.as_ref()),

            instructions: self.instructions,
            locations: self.locations,
            flags: self.flags,
            posonlyarg_count: self.posonlyarg_count,
            arg_count: self.arg_count,
            kwonlyarg_count: self.kwonlyarg_count,
            first_line_number: self.first_line_number,
            max_stackdepth: self.max_stackdepth,
            cell2arg: self.cell2arg,
            linetable: self.linetable,
            exceptiontable: self.exceptiontable,
        }
    }

    /// Same as `map_bag` but clones `self`
    pub fn map_clone_bag<Bag: ConstantBag>(&self, bag: &Bag) -> CodeObject<Bag::Constant> {
        let map_names =
            |names: &[C::Name]| names.iter().map(|x| bag.make_name(x.as_ref())).collect();
        CodeObject {
            constants: self
                .constants
                .iter()
                .map(|x| bag.make_constant(x.borrow_constant()))
                .collect(),
            names: map_names(&self.names),
            varnames: map_names(&self.varnames),
            cellvars: map_names(&self.cellvars),
            freevars: map_names(&self.freevars),
            source_path: bag.make_name(self.source_path.as_ref()),
            obj_name: bag.make_name(self.obj_name.as_ref()),
            qualname: bag.make_name(self.qualname.as_ref()),

            instructions: self.instructions.clone(),
            locations: self.locations.clone(),
            flags: self.flags,
            posonlyarg_count: self.posonlyarg_count,
            arg_count: self.arg_count,
            kwonlyarg_count: self.kwonlyarg_count,
            first_line_number: self.first_line_number,
            max_stackdepth: self.max_stackdepth,
            cell2arg: self.cell2arg.clone(),
            linetable: self.linetable.clone(),
            exceptiontable: self.exceptiontable.clone(),
        }
    }
}

impl<C: Constant> fmt::Display for CodeObject<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.display_inner(f, false, 1)?;
        for constant in &*self.constants {
            if let BorrowedConstant::Code { code } = constant.borrow_constant() {
                writeln!(f, "\nDisassembly of {code:?}")?;
                code.fmt(f)?;
            }
        }
        Ok(())
    }
}

pub trait InstrDisplayContext {
    type Constant: Constant;

    fn get_constant(&self, i: usize) -> &Self::Constant;

    fn get_name(&self, i: usize) -> &str;

    fn get_varname(&self, i: usize) -> &str;

    fn get_cell_name(&self, i: usize) -> &str;
}

impl<C: Constant> InstrDisplayContext for CodeObject<C> {
    type Constant = C;

    fn get_constant(&self, i: usize) -> &C {
        &self.constants[i]
    }

    fn get_name(&self, i: usize) -> &str {
        self.names[i].as_ref()
    }

    fn get_varname(&self, i: usize) -> &str {
        self.varnames[i].as_ref()
    }

    fn get_cell_name(&self, i: usize) -> &str {
        self.cellvars
            .get(i)
            .unwrap_or_else(|| &self.freevars[i - self.cellvars.len()])
            .as_ref()
    }
}

impl fmt::Display for ConstantData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.borrow_constant().fmt_display(f)
    }
}

impl<C: Constant> fmt::Debug for CodeObject<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<code object {} at ??? file {:?}, line {}>",
            self.obj_name.as_ref(),
            self.source_path.as_ref(),
            self.first_line_number.map_or(-1, |x| x.get() as i32)
        )
    }
}
