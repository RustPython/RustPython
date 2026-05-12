//! Implement python as a virtual machine with bytecode. This module
//! implements bytecode structure.

use crate::{
    marshal::MarshalError,
    varint::{read_varint, read_varint_with_start, write_varint_be, write_varint_with_start},
    {OneIndexed, SourceLocation},
};
use alloc::{borrow::ToOwned, boxed::Box, collections::BTreeSet, fmt, string::String, vec::Vec};
use bitflags::bitflags;
use core::{
    cell::UnsafeCell,
    hash, mem,
    ops::{Deref, Index, IndexMut},
    sync::atomic::{AtomicU8, AtomicU16, AtomicUsize, Ordering},
};
use itertools::Itertools;
use malachite_bigint::BigInt;
use num_complex::Complex64;
use rustpython_wtf8::{Wtf8, Wtf8Buf};

pub use crate::bytecode::{
    instruction::{AnyInstruction, AnyOpcode, Arg, StackEffect},
    instructions::{Instruction, Opcode, PseudoInstruction, PseudoOpcode},
    oparg::{
        BinaryOperator, BuildSliceArgCount, CommonConstant, ComparisonOperator, ConvertValueOparg,
        IntrinsicFunction1, IntrinsicFunction2, Invert, Label, LoadAttr, LoadSuperAttr,
        MakeFunctionFlag, MakeFunctionFlags, NameIdx, OpArg, OpArgByte, OpArgState, OpArgType,
        RaiseKind, SpecialMethod, UnpackExArgs,
    },
};

mod instruction;
mod instructions;
pub mod oparg;

/// Exception table entry for zero-cost exception handling
/// Format: (start, size, target, depth<<1|lasti)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    #[must_use]
    pub const fn new(start: u32, end: u32, target: u32, depth: u16, push_lasti: bool) -> Self {
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
#[must_use]
pub fn encode_exception_table(entries: &[ExceptionTableEntry]) -> alloc::boxed::Box<[u8]> {
    let mut data = Vec::new();
    for entry in entries {
        let size = entry.end.saturating_sub(entry.start);
        let depth_lasti = ((entry.depth as u32) << 1) | (entry.push_lasti as u32);

        write_varint_with_start(&mut data, entry.start);
        write_varint_be(&mut data, size);
        write_varint_be(&mut data, entry.target);
        write_varint_be(&mut data, depth_lasti);
    }
    data.into_boxed_slice()
}

/// Find exception handler for given instruction offset.
#[must_use]
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

/// Decode all exception table entries.
#[must_use]
pub fn decode_exception_table(table: &[u8]) -> Vec<ExceptionTableEntry> {
    let mut entries = Vec::new();
    let mut pos = 0;
    while pos < table.len() {
        let Some(start) = read_varint_with_start(table, &mut pos) else {
            break;
        };
        let Some(size) = read_varint(table, &mut pos) else {
            break;
        };
        let Some(target) = read_varint(table, &mut pos) else {
            break;
        };
        let Some(depth_lasti) = read_varint(table, &mut pos) else {
            break;
        };
        let Some(end) = start.checked_add(size) else {
            break;
        };
        entries.push(ExceptionTableEntry {
            start,
            end,
            target,
            depth: (depth_lasti >> 1) as u16,
            push_lasti: (depth_lasti & 1) != 0,
        });
    }
    entries
}

/// Parse linetable to build a boolean mask indicating which code units
/// have NO_LOCATION (line == -1). Returns a Vec<bool> of length `num_units`.
#[must_use]
pub fn build_no_location_mask(linetable: &[u8], num_units: usize) -> Vec<bool> {
    let mut mask = Vec::new();
    mask.resize(num_units, false);
    let mut pos = 0;
    let mut unit_idx = 0;

    while pos < linetable.len() && unit_idx < num_units {
        let header = linetable[pos];
        pos += 1;
        let code = (header >> 3) & 0xf;
        let length = ((header & 7) + 1) as usize;

        let is_no_location = code == PyCodeLocationInfoKind::None as u8;

        // Skip payload bytes based on location kind
        match code {
            0..=9 => pos += 1,   // Short forms: 1 byte payload
            10..=12 => pos += 2, // OneLine forms: 2 bytes payload
            13 => {
                // NoColumns: signed varint (line delta)
                while pos < linetable.len() {
                    let b = linetable[pos];
                    pos += 1;
                    if b & 0x40 == 0 {
                        break;
                    }
                }
            }
            14 => {
                // Long form: signed varint (line delta) + 3 unsigned varints
                // line_delta
                while pos < linetable.len() {
                    let b = linetable[pos];
                    pos += 1;
                    if b & 0x40 == 0 {
                        break;
                    }
                }
                // end_line_delta, col+1, end_col+1
                for _ in 0..3 {
                    while pos < linetable.len() {
                        let b = linetable[pos];
                        pos += 1;
                        if b & 0x40 == 0 {
                            break;
                        }
                    }
                }
            }
            15 => {} // None: no payload
            _ => {}
        }

        for _ in 0..length {
            if unit_idx < num_units {
                mask[unit_idx] = is_no_location;
                unit_idx += 1;
            }
        }
    }

    mask
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
    #[must_use]
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

    #[must_use]
    pub fn is_short(&self) -> bool {
        (*self as u8) <= 9
    }

    #[must_use]
    pub fn short_column_group(&self) -> Option<u8> {
        if self.is_short() {
            Some(*self as u8)
        } else {
            Option::None
        }
    }

    #[must_use]
    pub fn one_line_delta(&self) -> Option<i32> {
        match self {
            Self::OneLine0 => Some(0),
            Self::OneLine1 => Some(1),
            Self::OneLine2 => Some(2),
            _ => Option::None,
        }
    }
}

pub trait Constant: Sized + Clone {
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
            Self::Slice { elements } => Slice { elements },
            Self::Frozenset { elements } => Frozenset { elements },
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

#[derive(Clone)]
pub struct Constants<C: Constant>(Box<[C]>);

impl<C: Constant> Deref for Constants<C> {
    type Target = [C];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<C: Constant> Index<oparg::ConstIdx> for Constants<C> {
    type Output = C;

    fn index(&self, consti: oparg::ConstIdx) -> &Self::Output {
        &self.0[consti.as_usize()]
    }
}

impl<C: Constant> FromIterator<C> for Constants<C> {
    fn from_iter<T: IntoIterator<Item = C>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

// TODO: Newtype "CodeObject.varnames". Make sure only `oparg:VarNum` can be used as index
impl<T> Index<oparg::VarNum> for [T] {
    type Output = T;

    fn index(&self, var_num: oparg::VarNum) -> &Self::Output {
        &self[var_num.as_usize()]
    }
}

// TODO: Newtype "CodeObject.varnames". Make sure only `oparg:VarNum` can be used as index
impl<T> IndexMut<oparg::VarNum> for [T] {
    fn index_mut(&mut self, var_num: oparg::VarNum) -> &mut Self::Output {
        &mut self[var_num.as_usize()]
    }
}

/// Per-slot kind flags for localsplus (co_localspluskinds).
pub const CO_FAST_HIDDEN: u8 = 0x10;
pub const CO_FAST_LOCAL: u8 = 0x20;
pub const CO_FAST_CELL: u8 = 0x40;
pub const CO_FAST_FREE: u8 = 0x80;

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
    pub constants: Constants<C>,
    pub names: Box<[C::Name]>,
    pub varnames: Box<[C::Name]>,
    pub cellvars: Box<[C::Name]>,
    pub freevars: Box<[C::Name]>,
    /// Per-slot kind flags: CO_FAST_LOCAL, CO_FAST_CELL, CO_FAST_FREE, CO_FAST_HIDDEN.
    /// Length = nlocalsplus (nlocals + ncells + nfrees).
    pub localspluskinds: Box<[u8]>,
    /// Line number table (CPython 3.11+ format)
    pub linetable: Box<[u8]>,
    /// Exception handling table
    pub exceptiontable: Box<[u8]>,
}

bitflags! {
    #[derive(Copy, Clone, Debug, PartialEq)]
    pub struct CodeFlags: u32 {
        const OPTIMIZED = 0x0001;
        const NEWLOCALS = 0x0002;
        const VARARGS = 0x0004;
        const VARKEYWORDS = 0x0008;
        const NESTED = 0x0010;
        const GENERATOR = 0x0020;
        const COROUTINE = 0x0080;
        const ITERABLE_COROUTINE = 0x0100;
        /// If a code object represents a function and has a docstring,
        /// this bit is set and the first item in co_consts is the docstring.
        const HAS_DOCSTRING = 0x4000000;
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct CodeUnit {
    pub op: Instruction,
    pub arg: OpArgByte,
}

const _: () = assert!(mem::size_of::<CodeUnit>() == 2);

/// Adaptive specialization: number of executions before attempting specialization.
///
/// Matches CPython's `_Py_BackoffCounter` encoding.
pub const ADAPTIVE_WARMUP_VALUE: u16 = adaptive_counter_bits(1, 1);
/// Adaptive specialization: cooldown counter after a successful specialization.
///
/// Value/backoff = (52, 0), matching CPython's ADAPTIVE_COOLDOWN bits.
pub const ADAPTIVE_COOLDOWN_VALUE: u16 = adaptive_counter_bits(52, 0);
/// Initial JUMP_BACKWARD counter bits (value/backoff = 4095/12).
pub const JUMP_BACKWARD_INITIAL_VALUE: u16 = adaptive_counter_bits(4095, 12);

const BACKOFF_BITS: u16 = 4;
const MAX_BACKOFF: u16 = 12;
const UNREACHABLE_BACKOFF: u16 = 15;

/// Encode an adaptive counter as `(value << 4) | backoff`.
#[must_use]
pub const fn adaptive_counter_bits(value: u16, backoff: u16) -> u16 {
    (value << BACKOFF_BITS) | backoff
}

/// True when the adaptive counter should trigger specialization.
#[inline]
#[must_use]
pub const fn adaptive_counter_triggers(counter: u16) -> bool {
    counter < UNREACHABLE_BACKOFF
}

/// Decrement adaptive counter by one countdown step.
#[inline]
#[must_use]
pub const fn advance_adaptive_counter(counter: u16) -> u16 {
    counter.wrapping_sub(1 << BACKOFF_BITS)
}

/// Reset adaptive counter with exponential backoff.
#[inline]
#[must_use]
pub const fn adaptive_counter_backoff(counter: u16) -> u16 {
    let backoff = counter & ((1 << BACKOFF_BITS) - 1);
    if backoff < MAX_BACKOFF {
        adaptive_counter_bits((1 << (backoff + 1)) - 1, backoff + 1)
    } else {
        adaptive_counter_bits((1 << MAX_BACKOFF) - 1, MAX_BACKOFF)
    }
}

impl CodeUnit {
    #[must_use]
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

pub struct CodeUnits {
    units: UnsafeCell<Box<[CodeUnit]>>,
    adaptive_counters: Box<[AtomicU16]>,
    /// Pointer-sized cache entries for descriptor pointers.
    /// Single atomic load/store prevents torn reads when multiple threads
    /// specialize the same instruction concurrently.
    pointer_cache: Box<[AtomicUsize]>,
}

// SAFETY: All cache operations use atomic read/write instructions.
// - replace_op / compare_exchange_op: AtomicU8 store/CAS (Release)
// - cache read/write: AtomicU16 load/store (Relaxed)
// - adaptive counter: AtomicU16 load/store (Relaxed)
// Ordering is established by:
// - replace_op (Release) ↔ dispatch loop read_op (Acquire) for cache data visibility
// - tp_version_tag (Acquire) for descriptor pointer validity
unsafe impl Sync for CodeUnits {}

impl Clone for CodeUnits {
    fn clone(&self) -> Self {
        // SAFETY: No concurrent mutation during clone — cloning is only done
        // during code object construction or marshaling, not while instrumented.
        let units = unsafe { &*self.units.get() }.clone();
        let adaptive_counters = self
            .adaptive_counters
            .iter()
            .map(|c| AtomicU16::new(c.load(Ordering::Relaxed)))
            .collect();
        let pointer_cache = self
            .pointer_cache
            .iter()
            .map(|c| AtomicUsize::new(c.load(Ordering::Relaxed)))
            .collect();
        Self {
            units: UnsafeCell::new(units),
            adaptive_counters,
            pointer_cache,
        }
    }
}

impl fmt::Debug for CodeUnits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // SAFETY: Debug formatting doesn't race with replace_op
        let inner = unsafe { &*self.units.get() };
        f.debug_tuple("CodeUnits").field(inner).finish()
    }
}

impl TryFrom<&[u8]> for CodeUnits {
    type Error = MarshalError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        if !value.len().is_multiple_of(2) {
            return Err(Self::Error::InvalidBytecode);
        }

        let units = value
            .chunks_exact(2)
            .map(CodeUnit::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(units.into())
    }
}

impl<const N: usize> From<[CodeUnit; N]> for CodeUnits {
    fn from(value: [CodeUnit; N]) -> Self {
        Self::from(Vec::from(value))
    }
}

impl From<Vec<CodeUnit>> for CodeUnits {
    fn from(value: Vec<CodeUnit>) -> Self {
        let units = value.into_boxed_slice();
        let len = units.len();
        let adaptive_counters = (0..len)
            .map(|_| AtomicU16::new(0))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let pointer_cache = (0..len)
            .map(|_| AtomicUsize::new(0))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            units: UnsafeCell::new(units),
            adaptive_counters,
            pointer_cache,
        }
    }
}

impl FromIterator<CodeUnit> for CodeUnits {
    fn from_iter<T: IntoIterator<Item = CodeUnit>>(iter: T) -> Self {
        Self::from(iter.into_iter().collect::<Vec<_>>())
    }
}

impl Deref for CodeUnits {
    type Target = [CodeUnit];

    fn deref(&self) -> &Self::Target {
        // SAFETY: Shared references to the slice are valid even while replace_op
        // may update individual opcode bytes — readers tolerate stale opcodes
        // (they will re-read on the next iteration).
        unsafe { &*self.units.get() }
    }
}

impl CodeUnits {
    /// Disable adaptive specialization by setting all counters to unreachable.
    /// Used for CPython-compiled bytecode where specialization may not be safe.
    pub fn disable_specialization(&self) {
        for counter in self.adaptive_counters.iter() {
            counter.store(UNREACHABLE_BACKOFF, Ordering::Relaxed);
        }
    }

    /// Replace the opcode at `index` in-place without changing the arg byte.
    /// Uses atomic Release store to ensure prior cache writes are visible
    /// to threads that subsequently read the new opcode with Acquire.
    ///
    /// # Safety
    /// - `index` must be in bounds.
    /// - `new_op` must have the same arg semantics as the original opcode.
    pub unsafe fn replace_op(&self, index: usize, new_op: Instruction) {
        let units = unsafe { &*self.units.get() };
        let ptr = units.as_ptr().wrapping_add(index) as *const AtomicU8;
        unsafe { &*ptr }.store(new_op.into(), Ordering::Release);
    }

    /// Atomically replace opcode only if it still matches `expected`.
    /// Returns true on success. Uses Release ordering on success.
    ///
    /// # Safety
    /// - `index` must be in bounds.
    pub unsafe fn compare_exchange_op(
        &self,
        index: usize,
        expected: Instruction,
        new_op: Instruction,
    ) -> bool {
        let units = unsafe { &*self.units.get() };
        let ptr = units.as_ptr().wrapping_add(index) as *const AtomicU8;
        unsafe { &*ptr }
            .compare_exchange(
                expected.into(),
                new_op.into(),
                Ordering::Release,
                Ordering::Relaxed,
            )
            .is_ok()
    }

    /// Atomically read the opcode at `index` with Acquire ordering.
    /// Pairs with `replace_op` (Release) to ensure cache data visibility.
    pub fn read_op(&self, index: usize) -> Instruction {
        let units = unsafe { &*self.units.get() };
        let ptr = units.as_ptr().wrapping_add(index) as *const AtomicU8;
        let byte = unsafe { &*ptr }.load(Ordering::Acquire);
        // SAFETY: Only valid Instruction values are stored via replace_op/compare_exchange_op.
        unsafe { mem::transmute::<u8, Instruction>(byte) }
    }

    /// Atomically read the arg byte at `index` with Relaxed ordering.
    pub fn read_arg(&self, index: usize) -> OpArgByte {
        let units = unsafe { &*self.units.get() };
        let ptr = units.as_ptr().wrapping_add(index) as *const u8;
        let arg_ptr = unsafe { ptr.add(1) } as *const AtomicU8;
        OpArgByte::from(unsafe { &*arg_ptr }.load(Ordering::Relaxed))
    }

    /// Write a u16 value into a CACHE code unit at `index`.
    /// Each CodeUnit is 2 bytes (#[repr(C)]: op u8 + arg u8), so one u16 fits exactly.
    /// Uses Relaxed atomic store; ordering is provided by replace_op (Release).
    ///
    /// # Safety
    /// - `index` must be in bounds and point to a CACHE entry.
    pub unsafe fn write_cache_u16(&self, index: usize, value: u16) {
        let units = unsafe { &*self.units.get() };
        let ptr = units.as_ptr().wrapping_add(index) as *const AtomicU16;
        unsafe { &*ptr }.store(value, Ordering::Relaxed);
    }

    /// Read a u16 value from a CACHE code unit at `index`.
    /// Uses Relaxed atomic load; ordering is provided by read_op (Acquire).
    ///
    /// # Panics
    /// Panics if `index` is out of bounds.
    pub fn read_cache_u16(&self, index: usize) -> u16 {
        let units = unsafe { &*self.units.get() };
        assert!(index < units.len(), "read_cache_u16: index out of bounds");
        let ptr = units.as_ptr().wrapping_add(index) as *const AtomicU16;
        unsafe { &*ptr }.load(Ordering::Relaxed)
    }

    /// Write a u32 value across two consecutive CACHE code units starting at `index`.
    ///
    /// # Safety
    /// Same requirements as `write_cache_u16`.
    pub unsafe fn write_cache_u32(&self, index: usize, value: u32) {
        unsafe {
            self.write_cache_u16(index, value as u16);
            self.write_cache_u16(index + 1, (value >> 16) as u16);
        }
    }

    /// Read a u32 value from two consecutive CACHE code units starting at `index`.
    ///
    /// # Panics
    /// Panics if `index + 1` is out of bounds.
    pub fn read_cache_u32(&self, index: usize) -> u32 {
        let lo = self.read_cache_u16(index) as u32;
        let hi = self.read_cache_u16(index + 1) as u32;
        lo | (hi << 16)
    }

    /// Store a pointer-sized value atomically in the pointer cache at `index`.
    ///
    /// Uses a single `AtomicUsize` store to prevent torn writes when
    /// multiple threads specialize the same instruction concurrently.
    ///
    /// # Safety
    /// - `index` must be in bounds.
    /// - `value` must be `0` or a valid `*const PyObject` encoded as `usize`.
    /// - Callers must follow the cache invalidation/upgrade protocol:
    ///   invalidate the version guard before writing and publish the new
    ///   version after writing.
    pub unsafe fn write_cache_ptr(&self, index: usize, value: usize) {
        self.pointer_cache[index].store(value, Ordering::Relaxed);
    }

    /// Load a pointer-sized value atomically from the pointer cache at `index`.
    ///
    /// Uses a single `AtomicUsize` load to prevent torn reads.
    ///
    /// # Panics
    /// Panics if `index` is out of bounds.
    pub fn read_cache_ptr(&self, index: usize) -> usize {
        self.pointer_cache[index].load(Ordering::Relaxed)
    }

    /// Read adaptive counter bits for instruction at `index`.
    /// Uses Relaxed atomic load.
    pub fn read_adaptive_counter(&self, index: usize) -> u16 {
        self.adaptive_counters[index].load(Ordering::Relaxed)
    }

    /// Write adaptive counter bits for instruction at `index`.
    /// Uses Relaxed atomic store.
    ///
    /// # Safety
    /// - `index` must be in bounds.
    pub unsafe fn write_adaptive_counter(&self, index: usize, value: u16) {
        self.adaptive_counters[index].store(value, Ordering::Relaxed);
    }

    /// Produce a clean copy of the bytecode suitable for serialization
    /// (marshal) and `co_code`. Specialized opcodes are mapped back to their
    /// base variants via `deoptimize()` and all CACHE entries are zeroed.
    pub fn original_bytes(&self) -> Vec<u8> {
        let len = self.len();
        let mut out = Vec::with_capacity(len * 2);
        let mut i = 0;
        while i < len {
            let op = self.read_op(i).deoptimize();
            let arg = self.read_arg(i);
            let caches = op.cache_entries();
            out.push(u8::from(op));
            out.push(u8::from(arg));
            // Zero-fill all CACHE entries (counter + cached data)
            for _ in 0..caches {
                i += 1;
                out.push(0); // op = Cache = 0
                out.push(0); // arg = 0
            }
            i += 1;
        }
        out
    }

    /// Initialize adaptive warmup counters for all cacheable instructions.
    /// Called lazily at RESUME (first execution of a code object).
    /// Counters are stored out-of-line to preserve `op = Instruction::Cache`.
    /// All writes are atomic (Relaxed) to avoid data races with concurrent readers.
    pub fn quicken(&self) {
        let len = self.len();
        let mut i = 0;
        while i < len {
            let op = self.read_op(i);
            let caches = op.cache_entries();
            if caches > 0 {
                // Don't write adaptive counter for instrumented opcodes;
                // specialization is skipped while monitoring is active.
                if !op.is_instrumented() {
                    let cache_base = i + 1;
                    if cache_base < len {
                        let initial_counter = if matches!(op, Instruction::JumpBackward { .. }) {
                            JUMP_BACKWARD_INITIAL_VALUE
                        } else {
                            ADAPTIVE_WARMUP_VALUE
                        };
                        unsafe {
                            self.write_adaptive_counter(cache_base, initial_counter);
                        }
                    }
                }
                i += 1 + caches;
            } else {
                i += 1;
            }
        }
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
    Tuple {
        elements: Vec<ConstantData>,
    },
    Integer {
        value: BigInt,
    },
    Float {
        value: f64,
    },
    Complex {
        value: Complex64,
    },
    Boolean {
        value: bool,
    },
    Str {
        value: Wtf8Buf,
    },
    Bytes {
        value: Vec<u8>,
    },
    Code {
        code: Box<CodeObject>,
    },
    /// Constant slice(start, stop, step)
    Slice {
        elements: Box<[ConstantData; 3]>,
    },
    Frozenset {
        elements: Vec<ConstantData>,
    },
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
            (Slice { elements: a }, Slice { elements: b }) => a == b,
            (Frozenset { elements: a }, Frozenset { elements: b }) => a == b,
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
            Slice { elements } => elements.hash(state),
            Frozenset { elements } => elements.hash(state),
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
    Slice { elements: &'a [C; 3] },
    Frozenset { elements: &'a [C] },
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
            BorrowedConstant::Slice { elements } => {
                write!(f, "slice(")?;
                elements[0].borrow_constant().fmt_display(f)?;
                write!(f, ", ")?;
                elements[1].borrow_constant().fmt_display(f)?;
                write!(f, ", ")?;
                elements[2].borrow_constant().fmt_display(f)?;
                write!(f, ")")
            }
            BorrowedConstant::Frozenset { elements } => {
                write!(f, "frozenset({{")?;
                let mut first = true;
                for c in *elements {
                    if first {
                        first = false
                    } else {
                        write!(f, ", ")?;
                    }
                    c.borrow_constant().fmt_display(f)?;
                }
                write!(f, "}})")
            }
            BorrowedConstant::None => write!(f, "None"),
            BorrowedConstant::Ellipsis => write!(f, "..."),
        }
    }

    #[must_use]
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
            BorrowedConstant::Slice { elements } => Slice {
                elements: Box::new(elements.each_ref().map(|c| c.borrow_constant().to_owned())),
            },
            BorrowedConstant::Frozenset { elements } => Frozenset {
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

    /// Map this CodeObject to one that holds a Bag::Constant
    pub fn map_bag<Bag: ConstantBag>(self, bag: Bag) -> CodeObject<Bag::Constant> {
        let map_names = |names: Box<[C::Name]>| {
            names
                .iter()
                .map(|x| bag.make_name(x.as_ref()))
                .collect::<Box<[_]>>()
        };
        CodeObject {
            constants: self
                .constants
                .iter()
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
            localspluskinds: self.localspluskinds,
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
            localspluskinds: self.localspluskinds.clone(),
            linetable: self.linetable.clone(),
            exceptiontable: self.exceptiontable.clone(),
        }
    }
}

pub trait InstrDisplayContext {
    type Constant: Constant;

    fn get_constant(&self, consti: oparg::ConstIdx) -> &Self::Constant;

    fn get_name(&self, i: usize) -> &str;

    fn get_varname(&self, var_num: oparg::VarNum) -> &str;

    /// Get name for a localsplus index (used by DEREF instructions).
    fn get_localsplus_name(&self, var_num: oparg::VarNum) -> &str;
}

impl<C: Constant> InstrDisplayContext for CodeObject<C> {
    type Constant = C;

    fn get_constant(&self, consti: oparg::ConstIdx) -> &C {
        &self.constants[consti]
    }

    fn get_name(&self, i: usize) -> &str {
        self.names[i].as_ref()
    }

    fn get_varname(&self, var_num: oparg::VarNum) -> &str {
        self.varnames[var_num].as_ref()
    }

    fn get_localsplus_name(&self, var_num: oparg::VarNum) -> &str {
        let idx = var_num.as_usize();
        let nlocals = self.varnames.len();
        if idx < nlocals {
            self.varnames[idx].as_ref()
        } else {
            let cell_idx = idx - nlocals;
            self.cellvars
                .get(cell_idx)
                .unwrap_or_else(|| &self.freevars[cell_idx - self.cellvars.len()])
                .as_ref()
        }
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
    use alloc::{vec, vec::Vec};

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
