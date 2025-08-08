//! Implement python as a virtual machine with bytecode. This module
//! implements bytecode structure.

use crate::{OneIndexed, SourceLocation};
use bitflags::bitflags;
use itertools::Itertools;
use malachite_bigint::BigInt;
use num_complex::Complex64;
use rustpython_wtf8::{Wtf8, Wtf8Buf};
use std::{collections::BTreeSet, fmt, hash, marker::PhantomData, mem};

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
pub trait ConstantBag: Sized + Copy {
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
    pub posonlyarg_count: u32,
    // Number of positional-only arguments
    pub arg_count: u32,
    pub kwonlyarg_count: u32,
    pub source_path: C::Name,
    pub first_line_number: Option<OneIndexed>,
    pub max_stackdepth: u32,
    pub obj_name: C::Name,
    // Name of the object that created this code object
    pub qualname: C::Name,
    // Qualified name of the object (like CPython's co_qualname)
    pub cell2arg: Option<Box<[i32]>>,
    pub constants: Box<[C]>,
    pub names: Box<[C::Name]>,
    pub varnames: Box<[C::Name]>,
    pub cellvars: Box<[C::Name]>,
    pub freevars: Box<[C::Name]>,
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

#[derive(Default, Copy, Clone)]
#[repr(transparent)]
pub struct OpArgState {
    state: u32,
}

impl OpArgState {
    #[inline(always)]
    pub fn get(&mut self, ins: CodeUnit) -> (Instruction, OpArg) {
        let arg = self.extend(ins.arg);
        if ins.op != Instruction::ExtendedArg {
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

pub trait OpArgType: Copy {
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

pub type NameIdx = u32;

/// A Single bytecode instruction.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum Instruction {
    Nop,
    /// Importing by name
    ImportName {
        idx: Arg<NameIdx>,
    },
    /// Importing without name
    ImportNameless,
    /// from ... import ...
    ImportFrom {
        idx: Arg<NameIdx>,
    },
    LoadFast(Arg<NameIdx>),
    LoadNameAny(Arg<NameIdx>),
    LoadGlobal(Arg<NameIdx>),
    LoadDeref(Arg<NameIdx>),
    LoadClassDeref(Arg<NameIdx>),
    StoreFast(Arg<NameIdx>),
    StoreLocal(Arg<NameIdx>),
    StoreGlobal(Arg<NameIdx>),
    StoreDeref(Arg<NameIdx>),
    DeleteFast(Arg<NameIdx>),
    DeleteLocal(Arg<NameIdx>),
    DeleteGlobal(Arg<NameIdx>),
    DeleteDeref(Arg<NameIdx>),
    LoadClosure(Arg<NameIdx>),
    Subscript,
    StoreSubscript,
    DeleteSubscript,
    StoreAttr {
        idx: Arg<NameIdx>,
    },
    DeleteAttr {
        idx: Arg<NameIdx>,
    },
    LoadConst {
        /// index into constants vec
        idx: Arg<u32>,
    },
    UnaryOperation {
        op: Arg<UnaryOperator>,
    },
    BinaryOperation {
        op: Arg<BinaryOperator>,
    },
    BinaryOperationInplace {
        op: Arg<BinaryOperator>,
    },
    BinarySubscript,
    LoadAttr {
        idx: Arg<NameIdx>,
    },
    TestOperation {
        op: Arg<TestOperator>,
    },
    CompareOperation {
        op: Arg<ComparisonOperator>,
    },
    CopyItem {
        index: Arg<u32>,
    },
    Pop,
    Swap {
        index: Arg<u32>,
    },
    // ToBool,
    Rotate2,
    Rotate3,
    Duplicate,
    Duplicate2,
    GetIter,
    GetLen,
    CallIntrinsic1 {
        func: Arg<IntrinsicFunction1>,
    },
    CallIntrinsic2 {
        func: Arg<IntrinsicFunction2>,
    },
    Continue {
        target: Arg<Label>,
    },
    Break {
        target: Arg<Label>,
    },
    Jump {
        target: Arg<Label>,
    },
    /// Pop the top of the stack, and jump if this value is true.
    JumpIfTrue {
        target: Arg<Label>,
    },
    /// Pop the top of the stack, and jump if this value is false.
    JumpIfFalse {
        target: Arg<Label>,
    },
    /// Peek at the top of the stack, and jump if this value is true.
    /// Otherwise, pop top of stack.
    JumpIfTrueOrPop {
        target: Arg<Label>,
    },
    /// Peek at the top of the stack, and jump if this value is false.
    /// Otherwise, pop top of stack.
    JumpIfFalseOrPop {
        target: Arg<Label>,
    },
    MakeFunction,
    SetFunctionAttribute {
        attr: Arg<MakeFunctionFlags>,
    },
    CallFunctionPositional {
        nargs: Arg<u32>,
    },
    CallFunctionKeyword {
        nargs: Arg<u32>,
    },
    CallFunctionEx {
        has_kwargs: Arg<bool>,
    },
    LoadMethod {
        idx: Arg<NameIdx>,
    },
    CallMethodPositional {
        nargs: Arg<u32>,
    },
    CallMethodKeyword {
        nargs: Arg<u32>,
    },
    CallMethodEx {
        has_kwargs: Arg<bool>,
    },
    ForIter {
        target: Arg<Label>,
    },
    ReturnValue,
    ReturnConst {
        idx: Arg<u32>,
    },
    YieldValue,
    YieldFrom,

    /// Resume execution (e.g., at function start, after yield, etc.)
    Resume {
        arg: Arg<u32>,
    },

    SetupAnnotation,
    SetupLoop,

    /// Setup a finally handler, which will be called whenever one of this events occurs:
    /// - the block is popped
    /// - the function returns
    /// - an exception is returned
    SetupFinally {
        handler: Arg<Label>,
    },

    /// Enter a finally block, without returning, excepting, just because we are there.
    EnterFinally,

    /// Marker bytecode for the end of a finally sequence.
    /// When this bytecode is executed, the eval loop does one of those things:
    /// - Continue at a certain bytecode position
    /// - Propagate the exception
    /// - Return from a function
    /// - Do nothing at all, just continue
    EndFinally,

    SetupExcept {
        handler: Arg<Label>,
    },
    SetupWith {
        end: Arg<Label>,
    },
    WithCleanupStart,
    WithCleanupFinish,
    PopBlock,
    Raise {
        kind: Arg<RaiseKind>,
    },
    BuildString {
        size: Arg<u32>,
    },
    BuildTuple {
        size: Arg<u32>,
    },
    BuildTupleFromTuples {
        size: Arg<u32>,
    },
    BuildTupleFromIter,
    BuildList {
        size: Arg<u32>,
    },
    BuildListFromTuples {
        size: Arg<u32>,
    },
    BuildSet {
        size: Arg<u32>,
    },
    BuildSetFromTuples {
        size: Arg<u32>,
    },
    BuildMap {
        size: Arg<u32>,
    },
    BuildMapForCall {
        size: Arg<u32>,
    },
    DictUpdate {
        index: Arg<u32>,
    },
    BuildSlice {
        /// whether build a slice with a third step argument
        step: Arg<bool>,
    },
    ListAppend {
        i: Arg<u32>,
    },
    SetAdd {
        i: Arg<u32>,
    },
    MapAdd {
        i: Arg<u32>,
    },

    PrintExpr,
    LoadBuildClass,
    UnpackSequence {
        size: Arg<u32>,
    },
    UnpackEx {
        args: Arg<UnpackExArgs>,
    },
    FormatValue {
        conversion: Arg<ConversionFlag>,
    },
    PopException,
    Reverse {
        amount: Arg<u32>,
    },
    GetAwaitable,
    BeforeAsyncWith,
    SetupAsyncWith {
        end: Arg<Label>,
    },
    GetAIter,
    GetANext,
    EndAsyncFor,
    MatchMapping,
    MatchSequence,
    MatchKeys,
    MatchClass(Arg<u32>),
    ExtendedArg,
    // If you add a new instruction here, be sure to keep LAST_INSTRUCTION updated
}

// This must be kept up to date to avoid marshaling errors
const LAST_INSTRUCTION: Instruction = Instruction::ExtendedArg;

const _: () = assert!(mem::size_of::<Instruction>() == 1);

impl From<Instruction> for u8 {
    #[inline]
    fn from(ins: Instruction) -> Self {
        // SAFETY: there's no padding bits
        unsafe { std::mem::transmute::<Instruction, Self>(ins) }
    }
}

impl TryFrom<u8> for Instruction {
    type Error = crate::marshal::MarshalError;

    #[inline]
    fn try_from(value: u8) -> Result<Self, crate::marshal::MarshalError> {
        if value <= u8::from(LAST_INSTRUCTION) {
            Ok(unsafe { std::mem::transmute::<u8, Self>(value) })
        } else {
            Err(crate::marshal::MarshalError::InvalidBytecode)
        }
    }
}

#[derive(Copy, Clone)]
#[repr(C)]
pub struct CodeUnit {
    pub op: Instruction,
    pub arg: OpArgByte,
}

const _: () = assert!(mem::size_of::<CodeUnit>() == 2);

impl CodeUnit {
    pub const fn new(op: Instruction, arg: OpArgByte) -> Self {
        Self { op, arg }
    }
}

use self::Instruction::*;

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

impl<C: Constant> Copy for BorrowedConstant<'_, C> {}

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
            if let Some(l) = instruction.label_arg() {
                label_targets.insert(l.get(arg));
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
            instruction.fmt_dis(arg, f, self, expand_code_objects, 21, level)?;
            writeln!(f)?;
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

impl Instruction {
    /// Gets the label stored inside this instruction, if it exists
    #[inline]
    pub const fn label_arg(&self) -> Option<Arg<Label>> {
        match self {
            Jump { target: l }
            | JumpIfTrue { target: l }
            | JumpIfFalse { target: l }
            | JumpIfTrueOrPop { target: l }
            | JumpIfFalseOrPop { target: l }
            | ForIter { target: l }
            | SetupFinally { handler: l }
            | SetupExcept { handler: l }
            | SetupWith { end: l }
            | SetupAsyncWith { end: l }
            | Break { target: l }
            | Continue { target: l } => Some(*l),
            _ => None,
        }
    }

    /// Whether this is an unconditional branching
    ///
    /// # Examples
    ///
    /// ```
    /// use rustpython_compiler_core::bytecode::{Arg, Instruction};
    /// let jump_inst = Instruction::Jump { target: Arg::marker() };
    /// assert!(jump_inst.unconditional_branch())
    /// ```
    pub const fn unconditional_branch(&self) -> bool {
        matches!(
            self,
            Jump { .. }
                | Continue { .. }
                | Break { .. }
                | ReturnValue
                | ReturnConst { .. }
                | Raise { .. }
        )
    }

    /// What effect this instruction has on the stack
    ///
    /// # Examples
    ///
    /// ```
    /// use rustpython_compiler_core::bytecode::{Arg, Instruction, Label, UnaryOperator};
    /// let (target, jump_arg) = Arg::new(Label(0xF));
    /// let jump_instruction = Instruction::Jump { target };
    /// let (op, invert_arg) = Arg::new(UnaryOperator::Invert);
    /// let invert_instruction = Instruction::UnaryOperation { op };
    /// assert_eq!(jump_instruction.stack_effect(jump_arg, true), 0);
    /// assert_eq!(invert_instruction.stack_effect(invert_arg, false), 0);
    /// ```
    ///
    pub fn stack_effect(&self, arg: OpArg, jump: bool) -> i32 {
        match self {
            Nop => 0,
            ImportName { .. } | ImportNameless => -1,
            ImportFrom { .. } => 1,
            LoadFast(_) | LoadNameAny(_) | LoadGlobal(_) | LoadDeref(_) | LoadClassDeref(_) => 1,
            StoreFast(_) | StoreLocal(_) | StoreGlobal(_) | StoreDeref(_) => -1,
            DeleteFast(_) | DeleteLocal(_) | DeleteGlobal(_) | DeleteDeref(_) => 0,
            LoadClosure(_) => 1,
            Subscript => -1,
            StoreSubscript => -3,
            DeleteSubscript => -2,
            LoadAttr { .. } => 0,
            StoreAttr { .. } => -2,
            DeleteAttr { .. } => -1,
            LoadConst { .. } => 1,
            UnaryOperation { .. } => 0,
            BinaryOperation { .. }
            | BinaryOperationInplace { .. }
            | TestOperation { .. }
            | CompareOperation { .. } => -1,
            BinarySubscript => -1,
            CopyItem { .. } => 1,
            Pop => -1,
            Swap { .. } => 0,
            // ToBool => 0,
            Rotate2 | Rotate3 => 0,
            Duplicate => 1,
            Duplicate2 => 2,
            GetIter => 0,
            GetLen => 1,
            CallIntrinsic1 { .. } => 0,  // Takes 1, pushes 1
            CallIntrinsic2 { .. } => -1, // Takes 2, pushes 1
            Continue { .. } => 0,
            Break { .. } => 0,
            Jump { .. } => 0,
            JumpIfTrue { .. } | JumpIfFalse { .. } => -1,
            JumpIfTrueOrPop { .. } | JumpIfFalseOrPop { .. } => {
                if jump {
                    0
                } else {
                    -1
                }
            }
            MakeFunction => {
                // CPython 3.13 style: MakeFunction only pops code object
                -1 + 1 // pop code, push function
            }
            SetFunctionAttribute { .. } => {
                // pops attribute value and function, pushes function back
                -2 + 1
            }
            CallFunctionPositional { nargs } => -(nargs.get(arg) as i32) - 1 + 1,
            CallMethodPositional { nargs } => -(nargs.get(arg) as i32) - 3 + 1,
            CallFunctionKeyword { nargs } => -1 - (nargs.get(arg) as i32) - 1 + 1,
            CallMethodKeyword { nargs } => -1 - (nargs.get(arg) as i32) - 3 + 1,
            CallFunctionEx { has_kwargs } => -1 - (has_kwargs.get(arg) as i32) - 1 + 1,
            CallMethodEx { has_kwargs } => -1 - (has_kwargs.get(arg) as i32) - 3 + 1,
            LoadMethod { .. } => -1 + 3,
            ForIter { .. } => {
                if jump {
                    -1
                } else {
                    1
                }
            }
            ReturnValue => -1,
            ReturnConst { .. } => 0,
            Resume { .. } => 0,
            YieldValue => 0,
            YieldFrom => -1,
            SetupAnnotation | SetupLoop | SetupFinally { .. } | EnterFinally | EndFinally => 0,
            SetupExcept { .. } => jump as i32,
            SetupWith { .. } => (!jump) as i32,
            WithCleanupStart => 0,
            WithCleanupFinish => -1,
            PopBlock => 0,
            Raise { kind } => -(kind.get(arg) as u8 as i32),
            BuildString { size }
            | BuildTuple { size, .. }
            | BuildTupleFromTuples { size, .. }
            | BuildList { size, .. }
            | BuildListFromTuples { size, .. }
            | BuildSet { size, .. }
            | BuildSetFromTuples { size, .. } => -(size.get(arg) as i32) + 1,
            BuildTupleFromIter => 0,
            BuildMap { size } => {
                let nargs = size.get(arg) * 2;
                -(nargs as i32) + 1
            }
            BuildMapForCall { size } => {
                let nargs = size.get(arg);
                -(nargs as i32) + 1
            }
            DictUpdate { .. } => -1,
            BuildSlice { step } => -2 - (step.get(arg) as i32) + 1,
            ListAppend { .. } | SetAdd { .. } => -1,
            MapAdd { .. } => -2,
            PrintExpr => -1,
            LoadBuildClass => 1,
            UnpackSequence { size } => -1 + size.get(arg) as i32,
            UnpackEx { args } => {
                let UnpackExArgs { before, after } = args.get(arg);
                -1 + before as i32 + 1 + after as i32
            }
            FormatValue { .. } => -1,
            PopException => 0,
            Reverse { .. } => 0,
            GetAwaitable => 0,
            BeforeAsyncWith => 1,
            SetupAsyncWith { .. } => {
                if jump {
                    -1
                } else {
                    0
                }
            }
            GetAIter => 0,
            GetANext => 1,
            EndAsyncFor => -2,
            MatchMapping | MatchSequence => 0,
            MatchKeys => -1,
            MatchClass(_) => -2,
            ExtendedArg => 0,
        }
    }

    pub fn display<'a>(
        &'a self,
        arg: OpArg,
        ctx: &'a impl InstrDisplayContext,
    ) -> impl fmt::Display + 'a {
        struct FmtFn<F>(F);
        impl<F: Fn(&mut fmt::Formatter<'_>) -> fmt::Result> fmt::Display for FmtFn<F> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                (self.0)(f)
            }
        }
        FmtFn(move |f: &mut fmt::Formatter<'_>| self.fmt_dis(arg, f, ctx, false, 0, 0))
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
            Nop => w!(Nop),
            ImportName { idx } => w!(ImportName, name = idx),
            ImportNameless => w!(ImportNameless),
            ImportFrom { idx } => w!(ImportFrom, name = idx),
            LoadFast(idx) => w!(LoadFast, varname = idx),
            LoadNameAny(idx) => w!(LoadNameAny, name = idx),
            LoadGlobal(idx) => w!(LoadGlobal, name = idx),
            LoadDeref(idx) => w!(LoadDeref, cell_name = idx),
            LoadClassDeref(idx) => w!(LoadClassDeref, cell_name = idx),
            StoreFast(idx) => w!(StoreFast, varname = idx),
            StoreLocal(idx) => w!(StoreLocal, name = idx),
            StoreGlobal(idx) => w!(StoreGlobal, name = idx),
            StoreDeref(idx) => w!(StoreDeref, cell_name = idx),
            DeleteFast(idx) => w!(DeleteFast, varname = idx),
            DeleteLocal(idx) => w!(DeleteLocal, name = idx),
            DeleteGlobal(idx) => w!(DeleteGlobal, name = idx),
            DeleteDeref(idx) => w!(DeleteDeref, cell_name = idx),
            LoadClosure(i) => w!(LoadClosure, cell_name = i),
            Subscript => w!(Subscript),
            StoreSubscript => w!(StoreSubscript),
            DeleteSubscript => w!(DeleteSubscript),
            StoreAttr { idx } => w!(StoreAttr, name = idx),
            DeleteAttr { idx } => w!(DeleteAttr, name = idx),
            LoadConst { idx } => fmt_const("LoadConst", arg, f, idx),
            UnaryOperation { op } => w!(UnaryOperation, ?op),
            BinaryOperation { op } => w!(BinaryOperation, ?op),
            BinaryOperationInplace { op } => w!(BinaryOperationInplace, ?op),
            BinarySubscript => w!(BinarySubscript),
            LoadAttr { idx } => w!(LoadAttr, name = idx),
            TestOperation { op } => w!(TestOperation, ?op),
            CompareOperation { op } => w!(CompareOperation, ?op),
            CopyItem { index } => w!(CopyItem, index),
            Pop => w!(Pop),
            Swap { index } => w!(Swap, index),
            // ToBool => w!(ToBool),
            Rotate2 => w!(Rotate2),
            Rotate3 => w!(Rotate3),
            Duplicate => w!(Duplicate),
            Duplicate2 => w!(Duplicate2),
            GetIter => w!(GetIter),
            // GET_LEN
            GetLen => w!(GetLen),
            CallIntrinsic1 { func } => w!(CallIntrinsic1, ?func),
            CallIntrinsic2 { func } => w!(CallIntrinsic2, ?func),
            Continue { target } => w!(Continue, target),
            Break { target } => w!(Break, target),
            Jump { target } => w!(Jump, target),
            JumpIfTrue { target } => w!(JumpIfTrue, target),
            JumpIfFalse { target } => w!(JumpIfFalse, target),
            JumpIfTrueOrPop { target } => w!(JumpIfTrueOrPop, target),
            JumpIfFalseOrPop { target } => w!(JumpIfFalseOrPop, target),
            MakeFunction => w!(MakeFunction),
            SetFunctionAttribute { attr } => w!(SetFunctionAttribute, ?attr),
            CallFunctionPositional { nargs } => w!(CallFunctionPositional, nargs),
            CallFunctionKeyword { nargs } => w!(CallFunctionKeyword, nargs),
            CallFunctionEx { has_kwargs } => w!(CallFunctionEx, has_kwargs),
            LoadMethod { idx } => w!(LoadMethod, name = idx),
            CallMethodPositional { nargs } => w!(CallMethodPositional, nargs),
            CallMethodKeyword { nargs } => w!(CallMethodKeyword, nargs),
            CallMethodEx { has_kwargs } => w!(CallMethodEx, has_kwargs),
            ForIter { target } => w!(ForIter, target),
            ReturnValue => w!(ReturnValue),
            ReturnConst { idx } => fmt_const("ReturnConst", arg, f, idx),
            Resume { arg } => w!(Resume, arg),
            YieldValue => w!(YieldValue),
            YieldFrom => w!(YieldFrom),
            SetupAnnotation => w!(SetupAnnotation),
            SetupLoop => w!(SetupLoop),
            SetupExcept { handler } => w!(SetupExcept, handler),
            SetupFinally { handler } => w!(SetupFinally, handler),
            EnterFinally => w!(EnterFinally),
            EndFinally => w!(EndFinally),
            SetupWith { end } => w!(SetupWith, end),
            WithCleanupStart => w!(WithCleanupStart),
            WithCleanupFinish => w!(WithCleanupFinish),
            BeforeAsyncWith => w!(BeforeAsyncWith),
            SetupAsyncWith { end } => w!(SetupAsyncWith, end),
            PopBlock => w!(PopBlock),
            Raise { kind } => w!(Raise, ?kind),
            BuildString { size } => w!(BuildString, size),
            BuildTuple { size } => w!(BuildTuple, size),
            BuildTupleFromTuples { size } => w!(BuildTupleFromTuples, size),
            BuildTupleFromIter => w!(BuildTupleFromIter),
            BuildList { size } => w!(BuildList, size),
            BuildListFromTuples { size } => w!(BuildListFromTuples, size),
            BuildSet { size } => w!(BuildSet, size),
            BuildSetFromTuples { size } => w!(BuildSetFromTuples, size),
            BuildMap { size } => w!(BuildMap, size),
            BuildMapForCall { size } => w!(BuildMapForCall, size),
            DictUpdate { index } => w!(DictUpdate, index),
            BuildSlice { step } => w!(BuildSlice, step),
            ListAppend { i } => w!(ListAppend, i),
            SetAdd { i } => w!(SetAdd, i),
            MapAdd { i } => w!(MapAdd, i),
            PrintExpr => w!(PrintExpr),
            LoadBuildClass => w!(LoadBuildClass),
            UnpackSequence { size } => w!(UnpackSequence, size),
            UnpackEx { args } => w!(UnpackEx, args),
            FormatValue { conversion } => w!(FormatValue, ?conversion),
            PopException => w!(PopException),
            Reverse { amount } => w!(Reverse, amount),
            GetAwaitable => w!(GetAwaitable),
            GetAIter => w!(GetAIter),
            GetANext => w!(GetANext),
            EndAsyncFor => w!(EndAsyncFor),
            MatchMapping => w!(MatchMapping),
            MatchSequence => w!(MatchSequence),
            MatchKeys => w!(MatchKeys),
            MatchClass(arg) => w!(MatchClass, arg),
            ExtendedArg => w!(ExtendedArg, Arg::<u32>::marker()),
        }
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
