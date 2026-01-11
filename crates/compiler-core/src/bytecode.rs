//! Implement python as a virtual machine with bytecode. This module
//! implements bytecode structure.

use crate::{
    marshal::MarshalError,
    varint::{read_varint, read_varint_with_start, write_varint, write_varint_with_start},
    {OneIndexed, SourceLocation},
};
use alloc::{collections::BTreeSet, fmt, vec::Vec};
use bitflags::bitflags;
use core::{hash, marker::PhantomData, mem, num::NonZeroU8, ops::Deref};
use itertools::Itertools;
use malachite_bigint::BigInt;
use num_complex::Complex64;
use rustpython_wtf8::{Wtf8, Wtf8Buf};

pub use crate::bytecode::instruction::Instruction;

mod instruction;

/// Exception table entry for zero-cost exception handling
/// Format: (start, size, target, depth<<1|lasti)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExceptionTableEntry {
    /// Start instruction offset (inclusive)
    pub start: u32,
    /// End instruction offset (exclusive)
    pub end: u32,
    /// Handler target offset
    pub target: u32,
    /// Stack depth at handler entry
    pub depth: u16,
    /// Whether to push lasti before exception
    pub push_lasti: bool,
}

impl ExceptionTableEntry {
    pub fn new(start: u32, end: u32, target: u32, depth: u16, push_lasti: bool) -> Self {
        Self {
            start,
            end,
            target,
            depth,
            push_lasti,
        }
    }
}

/// Encode exception table entries.
/// Uses 6-bit varint encoding with start marker (MSB) and continuation bit.
pub fn encode_exception_table(entries: &[ExceptionTableEntry]) -> alloc::boxed::Box<[u8]> {
    let mut data = Vec::new();
    for entry in entries {
        let size = entry.end.saturating_sub(entry.start);
        let depth_lasti = ((entry.depth as u32) << 1) | (entry.push_lasti as u32);

        write_varint_with_start(&mut data, entry.start);
        write_varint(&mut data, size);
        write_varint(&mut data, entry.target);
        write_varint(&mut data, depth_lasti);
    }
    data.into_boxed_slice()
}

/// Find exception handler for given instruction offset.
pub fn find_exception_handler(table: &[u8], offset: u32) -> Option<ExceptionTableEntry> {
    let mut pos = 0;
    while pos < table.len() {
        let start = read_varint_with_start(table, &mut pos)?;
        let size = read_varint(table, &mut pos)?;
        let target = read_varint(table, &mut pos)?;
        let depth_lasti = read_varint(table, &mut pos)?;

        let end = start + size;
        let depth = (depth_lasti >> 1) as u16;
        let push_lasti = (depth_lasti & 1) != 0;

        if offset >= start && offset < end {
            return Some(ExceptionTableEntry {
                start,
                end,
                target,
                depth,
                push_lasti,
            });
        }
    }
    None
}

/// Encode LOAD_ATTR oparg: bit 0 = method flag, bits 1+ = name index.
#[inline]
pub const fn encode_load_attr_arg(name_idx: u32, is_method: bool) -> u32 {
    (name_idx << 1) | (is_method as u32)
}

/// Decode LOAD_ATTR oparg: returns (name_idx, is_method).
#[inline]
pub const fn decode_load_attr_arg(oparg: u32) -> (u32, bool) {
    let is_method = (oparg & 1) == 1;
    let name_idx = oparg >> 1;
    (name_idx, is_method)
}

/// Encode LOAD_SUPER_ATTR oparg: bit 0 = load_method, bit 1 = has_class, bits 2+ = name index.
#[inline]
pub const fn encode_load_super_attr_arg(name_idx: u32, load_method: bool, has_class: bool) -> u32 {
    (name_idx << 2) | ((has_class as u32) << 1) | (load_method as u32)
}

/// Decode LOAD_SUPER_ATTR oparg: returns (name_idx, load_method, has_class).
#[inline]
pub const fn decode_load_super_attr_arg(oparg: u32) -> (u32, bool, bool) {
    let load_method = (oparg & 1) == 1;
    let has_class = (oparg & 2) == 2;
    let name_idx = oparg >> 2;
    (name_idx, load_method, has_class)
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
    pub instructions: CodeUnits,
    pub locations: Box<[(SourceLocation, SourceLocation)]>,
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
        const OPTIMIZED = 0x0001;
        const NEWLOCALS = 0x0002;
        const VARARGS = 0x0004;
        const VARKEYWORDS = 0x0008;
        const GENERATOR = 0x0020;
        const COROUTINE = 0x0080;
    }
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
        write!(f, "Arg<{}>", core::any::type_name::<T>())
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

pub type NameIdx = u32;

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

impl TryFrom<&[u8]> for CodeUnit {
    type Error = MarshalError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        match value.len() {
            2 => Ok(Self::new(value[0].try_into()?, value[1].into())),
            _ => Err(Self::Error::InvalidBytecode),
        }
    }
}

#[derive(Clone)]
pub struct CodeUnits(Box<[CodeUnit]>);

impl TryFrom<&[u8]> for CodeUnits {
    type Error = MarshalError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        if !value.len().is_multiple_of(2) {
            return Err(Self::Error::InvalidBytecode);
        }

        value.chunks_exact(2).map(CodeUnit::try_from).collect()
    }
}

impl<const N: usize> From<[CodeUnit; N]> for CodeUnits {
    fn from(value: [CodeUnit; N]) -> Self {
        Self(Box::from(value))
    }
}

impl From<Vec<CodeUnit>> for CodeUnits {
    fn from(value: Vec<CodeUnit>) -> Self {
        Self(value.into_boxed_slice())
    }
}

impl FromIterator<CodeUnit> for CodeUnits {
    fn from_iter<T: IntoIterator<Item = CodeUnit>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl Deref for CodeUnits {
    type Target = [CodeUnit];

    fn deref(&self) -> &Self::Target {
        &self.0
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
            (Code { code: a }, Code { code: b }) => core::ptr::eq(a.as_ref(), b.as_ref()),
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
            Code { code } => core::ptr::hash(code.as_ref(), state),
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

        let vararg = if self.flags.contains(CodeFlags::VARARGS) {
            let vararg = &self.varnames[varargs_pos];
            varargs_pos += 1;
            Some(vararg)
        } else {
            None
        };
        let varkwarg = if self.flags.contains(CodeFlags::VARKEYWORDS) {
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
        let line_digits = (3).max(self.locations.last().unwrap().0.line.digits().get());
        let offset_digits = (4).max(1 + self.instructions.len().ilog10() as usize);
        let mut last_line = OneIndexed::MAX;
        let mut arg_state = OpArgState::default();
        for (offset, &instruction) in self.instructions.iter().enumerate() {
            let (instruction, arg) = arg_state.get(instruction);
            // optional line number
            let line = self.locations[offset].0.line;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exception_table_encode_decode() {
        let entries = vec![
            ExceptionTableEntry::new(0, 10, 20, 2, false),
            ExceptionTableEntry::new(15, 25, 30, 1, true),
        ];

        let encoded = encode_exception_table(&entries);

        // Find handler at offset 5 (in range [0, 10))
        let handler = find_exception_handler(&encoded, 5);
        assert!(handler.is_some());
        let handler = handler.unwrap();
        assert_eq!(handler.start, 0);
        assert_eq!(handler.end, 10);
        assert_eq!(handler.target, 20);
        assert_eq!(handler.depth, 2);
        assert!(!handler.push_lasti);

        // Find handler at offset 20 (in range [15, 25))
        let handler = find_exception_handler(&encoded, 20);
        assert!(handler.is_some());
        let handler = handler.unwrap();
        assert_eq!(handler.start, 15);
        assert_eq!(handler.end, 25);
        assert_eq!(handler.target, 30);
        assert_eq!(handler.depth, 1);
        assert!(handler.push_lasti);

        // No handler at offset 12 (not in any range)
        let handler = find_exception_handler(&encoded, 12);
        assert!(handler.is_none());

        // No handler at offset 30 (past all ranges)
        let handler = find_exception_handler(&encoded, 30);
        assert!(handler.is_none());
    }

    #[test]
    fn test_exception_table_empty() {
        let entries: Vec<ExceptionTableEntry> = vec![];
        let encoded = encode_exception_table(&entries);
        assert!(encoded.is_empty());
        assert!(find_exception_handler(&encoded, 0).is_none());
    }

    #[test]
    fn test_exception_table_single_entry() {
        let entries = vec![ExceptionTableEntry::new(5, 15, 100, 3, true)];
        let encoded = encode_exception_table(&entries);

        // Inside range
        let handler = find_exception_handler(&encoded, 10);
        assert!(handler.is_some());
        let handler = handler.unwrap();
        assert_eq!(handler.target, 100);
        assert_eq!(handler.depth, 3);
        assert!(handler.push_lasti);

        // At start boundary (inclusive)
        assert!(find_exception_handler(&encoded, 5).is_some());

        // At end boundary (exclusive)
        assert!(find_exception_handler(&encoded, 15).is_none());
    }
}
