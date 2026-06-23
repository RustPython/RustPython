use core::ops::{Deref, DerefMut, Index, IndexMut};

use crate::{IndexMap, IndexSet, error::InternalError};
use malachite_bigint::BigInt;
use num_complex::Complex;
use num_traits::{ToPrimitive, Zero};
use rustpython_wtf8::Wtf8Buf;

use rustpython_compiler_core::{
    OneIndexed, SourceLocation,
    bytecode::{
        AnyInstruction, AnyOpcode, CO_FAST_ARG_KW, CO_FAST_ARG_POS, CO_FAST_ARG_VAR, CO_FAST_CELL,
        CO_FAST_FREE, CO_FAST_HIDDEN, CO_FAST_LOCAL, CodeFlags, CodeObject, CodeUnit, CodeUnits,
        ConstantData, InstrDisplayContext, Instruction, IntrinsicFunction1, OpArg, OpArgByte,
        Opcode, PseudoInstruction, PseudoOpcode, PyCodeLocationInfoKind, oparg,
    },
    varint::{write_signed_varint, write_varint},
};

/// Location info for linetable generation (allows line 0 for RESUME)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LineTableLocation {
    line: i32,
    end_line: i32,
    col: i32,
    end_col: i32,
}

#[derive(Clone, Copy)]
struct InstructionLocation {
    location: SourceLocation,
    end_location: SourceLocation,
    lineno_override: Option<i32>,
}

pub(crate) const LINE_ONLY_LOCATION_OVERRIDE: i32 = -4;
pub(crate) const NEXT_LOCATION_OVERRIDE: i32 = -2;
pub(crate) const NO_LOCATION_OVERRIDE: i32 = -1;

const MAX_INT_SIZE: u64 = 128;
const MAX_COLLECTION_SIZE: usize = 256;
const DEFAULT_CODE_SIZE: usize = 128;
const DEFAULT_LNOTAB_SIZE: usize = 16;
const DEFAULT_CNOTAB_SIZE: usize = 32;
const DEFAULT_BLOCK_SIZE: usize = 16;
const INITIAL_INSTR_SEQUENCE_SIZE: usize = 100;
const INITIAL_INSTR_SEQUENCE_LABELS_MAP_SIZE: usize = 10;
const MAX_REAL_OPCODE: u16 = 254;
const MAX_OPCODE: u16 = 511;
const MAX_TOTAL_ITEMS: isize = 1024;
const MAX_STR_SIZE: usize = 4096;
const MIN_CONST_SEQUENCE_SIZE: usize = 3;
const STACK_USE_GUIDELINE: usize = 30;

/// pycore_opcode_utils.h IS_WITHIN_OPCODE_RANGE
fn is_within_opcode_range(opcode: AnyOpcode) -> bool {
    match opcode {
        AnyOpcode::Real(opcode) => u16::from(opcode.as_u8()) <= MAX_REAL_OPCODE,
        AnyOpcode::Pseudo(opcode) => opcode.as_u16() <= MAX_OPCODE,
    }
}

#[derive(Clone, Debug, Default)]
pub struct ConstantPool {
    constants: Vec<ConstantData>,
}

impl ConstantPool {
    fn constant_contains_nan(constant: &ConstantData) -> bool {
        match constant {
            ConstantData::Float { value } => value.is_nan(),
            ConstantData::Complex { value } => value.re.is_nan() || value.im.is_nan(),
            ConstantData::Tuple { elements } | ConstantData::Frozenset { elements } => {
                elements.iter().any(Self::constant_contains_nan)
            }
            ConstantData::Slice { elements } => elements.iter().any(Self::constant_contains_nan),
            _ => false,
        }
    }

    pub fn insert_full(&mut self, constant: ConstantData) -> (usize, bool) {
        // CPython's _PyCode_ConstantKey() keeps NaN-bearing constants distinct
        // because Python-level NaN keys do not compare equal.
        if !Self::constant_contains_nan(&constant)
            && let Some(idx) = self
                .constants
                .iter()
                .position(|existing| existing == &constant)
        {
            return (idx, false);
        }
        let idx = self.constants.len();
        self.constants.push(constant);
        (idx, true)
    }

    fn try_insert_full(&mut self, constant: ConstantData) -> crate::InternalResult<(usize, bool)> {
        // CPython's _PyCode_ConstantKey() keeps NaN-bearing constants distinct
        // because Python-level NaN keys do not compare equal.
        if !Self::constant_contains_nan(&constant)
            && let Some(idx) = self
                .constants
                .iter()
                .position(|existing| existing == &constant)
        {
            return Ok((idx, false));
        }
        self.constants
            .try_reserve_exact(1)
            .map_err(|_| InternalError::MalformedControlFlowGraph)?;
        let idx = self.constants.len();
        self.constants.push(constant);
        Ok((idx, true))
    }

    pub fn insert(&mut self, constant: ConstantData) -> bool {
        self.insert_full(constant).1
    }

    #[must_use]
    pub fn get_index(&self, idx: usize) -> Option<&ConstantData> {
        self.constants.get(idx)
    }

    pub fn iter(&self) -> core::slice::Iter<'_, ConstantData> {
        self.constants.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.constants.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.constants.is_empty()
    }

    pub fn clear(&mut self) {
        self.constants.clear();
    }
}

impl Index<usize> for ConstantPool {
    type Output = ConstantData;

    fn index(&self, idx: usize) -> &Self::Output {
        &self.constants[idx]
    }
}

impl IntoIterator for ConstantPool {
    type Item = ConstantData;
    type IntoIter = alloc::vec::IntoIter<ConstantData>;

    fn into_iter(self) -> Self::IntoIter {
        self.constants.into_iter()
    }
}

/// Metadata for a code unit
// = _PyCompile_CodeUnitMetadata
#[derive(Clone, Debug)]
pub struct CodeUnitMetadata {
    pub name: String,                        // u_name (obj_name)
    pub qualname: Option<String>,            // u_qualname
    pub consts: ConstantPool,                // u_consts
    pub names: IndexSet<String>,             // u_names
    pub varnames: IndexSet<String>,          // u_varnames
    pub cellvars: IndexSet<String>,          // u_cellvars
    pub freevars: IndexSet<String>,          // u_freevars
    pub fast_hidden: IndexMap<String, bool>, // u_fast_hidden
    pub fast_hidden_final: IndexSet<String>, // final CO_FAST_HIDDEN names
    pub argcount: u32,                       // u_argcount
    pub posonlyargcount: u32,                // u_posonlyargcount
    pub kwonlyargcount: u32,                 // u_kwonlyargcount
    pub firstlineno: OneIndexed,             // u_firstlineno
}
// use rustpython_parser_core::source_code::{LineNumber, SourceLocation};

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct BlockIdx(u32);

impl BlockIdx {
    pub const NULL: Self = Self::new(u32::MAX);

    /// Creates a new instance of [`BlockIdx`] from a [`u32`].
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the inner [`u32`] value.
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// Returns the inner value as a [`usize`].
    #[must_use]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }

    /// Returns the inner value as a [`usize`].
    #[must_use]
    pub const fn idx(self) -> usize {
        self.as_usize()
    }
}

impl From<BlockIdx> for u32 {
    fn from(block_idx: BlockIdx) -> Self {
        block_idx.as_u32()
    }
}

impl From<BlockIdx> for usize {
    fn from(block_idx: BlockIdx) -> Self {
        block_idx.as_usize()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct InstructionInfo {
    pub instr: AnyInstruction,
    pub arg: OpArg,
    pub target: BlockIdx,
    pub location: SourceLocation,
    pub end_location: SourceLocation,
    pub except_handler: Option<ExceptHandlerInfo>,
    /// Override line number for linetable (e.g., line 0 for module RESUME)
    pub lineno_override: Option<i32>,
}

/// Exception handler information for an instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExceptHandlerInfo {
    /// Block to jump to when exception occurs
    pub handler_block: BlockIdx,
    /// Whether to push lasti before exception
    pub preserve_lasti: bool,
}

/// flowgraph.c INSTR_SET_OP0
fn instr_set_op0(info: &mut InstructionInfo, instr: AnyInstruction) {
    debug_assert!(!AnyOpcode::from(instr).has_arg());
    info.instr = instr;
    info.arg = OpArg::new(0);
}

/// flowgraph.c INSTR_SET_OP1
fn instr_set_op1(info: &mut InstructionInfo, instr: AnyInstruction, arg: OpArg) {
    debug_assert!(AnyOpcode::from(instr).has_arg());
    info.instr = instr;
    info.arg = arg;
}

/// flowgraph.c INSTR_SET_LOC
fn instr_set_loc(
    info: &mut InstructionInfo,
    location: SourceLocation,
    end_location: SourceLocation,
    lineno_override: Option<i32>,
) {
    info.location = location;
    info.end_location = end_location;
    info.lineno_override = lineno_override;
}

fn instr_location(info: &InstructionInfo) -> InstructionLocation {
    InstructionLocation {
        location: info.location,
        end_location: info.end_location,
        lineno_override: info.lineno_override,
    }
}

fn instr_set_location(info: &mut InstructionInfo, loc: InstructionLocation) {
    instr_set_loc(info, loc.location, loc.end_location, loc.lineno_override);
}

fn no_instruction_location() -> InstructionLocation {
    InstructionLocation {
        location: SourceLocation::default(),
        end_location: SourceLocation::default(),
        lineno_override: Some(NO_LOCATION_OVERRIDE),
    }
}

fn set_to_nop(info: &mut InstructionInfo) {
    instr_set_op0(info, Instruction::Nop.into());
}

fn nop_out_no_location(info: &mut InstructionInfo) {
    set_to_nop(info);
    instr_set_loc(
        info,
        SourceLocation::default(),
        SourceLocation::default(),
        Some(NO_LOCATION_OVERRIDE),
    );
}

fn empty_instruction_info() -> InstructionInfo {
    InstructionInfo {
        instr: Instruction::Nop.into(),
        arg: OpArg::new(0),
        target: BlockIdx::NULL,
        location: SourceLocation::default(),
        end_location: SourceLocation::default(),
        except_handler: None,
        lineno_override: None,
    }
}

/// codegen.c _Py_CArray_EnsureCapacity
fn c_array_ensure_capacity<T>(
    allocated_entries: usize,
    idx: usize,
    initial_num_entries: usize,
) -> crate::InternalResult<usize> {
    if allocated_entries == 0 {
        let new_alloc = if idx >= initial_num_entries {
            idx.checked_add(initial_num_entries)
                .ok_or(InternalError::MalformedControlFlowGraph)?
        } else {
            initial_num_entries
        };
        Ok(new_alloc)
    } else if idx >= allocated_entries {
        let oldsize = allocated_entries
            .checked_mul(core::mem::size_of::<T>())
            .ok_or(InternalError::MalformedControlFlowGraph)?;
        let doubled = allocated_entries
            .checked_mul(2)
            .ok_or(InternalError::MalformedControlFlowGraph)?;
        let new_alloc = if idx >= doubled {
            idx.checked_add(initial_num_entries)
                .ok_or(InternalError::MalformedControlFlowGraph)?
        } else {
            doubled
        };
        let newsize = new_alloc
            .checked_mul(core::mem::size_of::<T>())
            .ok_or(InternalError::MalformedControlFlowGraph)?;
        if oldsize > usize::MAX >> 1 || newsize == 0 {
            return Err(InternalError::MalformedControlFlowGraph);
        }
        Ok(new_alloc)
    } else {
        Ok(allocated_entries)
    }
}

/// flowgraph.c basicblock_next_instr
fn basicblock_next_instr(block: &mut Block) -> crate::InternalResult<usize> {
    let off = block.instruction_used;
    let new_allocation = c_array_ensure_capacity::<InstructionInfo>(
        block.instruction_allocation,
        off + 1,
        DEFAULT_BLOCK_SIZE,
    )?;
    if new_allocation > block.instruction_allocation {
        if new_allocation > block.instructions.len() {
            block
                .instructions
                .try_reserve_exact(new_allocation - block.instructions.len())
                .map_err(|_| InternalError::MalformedControlFlowGraph)?;
            block
                .instructions
                .resize_with(new_allocation, empty_instruction_info);
        }
        block.instruction_allocation = new_allocation;
    }
    debug_assert!(block.instruction_allocation > off);
    block.instruction_used += 1;
    Ok(off)
}

/// flowgraph.c basicblock_last_instr
fn basicblock_last_instr(block: &Block) -> Option<&InstructionInfo> {
    debug_assert!(block.instruction_allocation >= block.instruction_used);
    if block.instruction_used > 0 {
        debug_assert!(!block.instructions.is_empty());
        Some(&block.instructions[block.instruction_used - 1])
    } else {
        None
    }
}

/// flowgraph.c basicblock_last_instr
fn basicblock_last_instr_mut(block: &mut Block) -> Option<&mut InstructionInfo> {
    debug_assert!(block.instruction_allocation >= block.instruction_used);
    if block.instruction_used > 0 {
        debug_assert!(!block.instructions.is_empty());
        Some(&mut block.instructions[block.instruction_used - 1])
    } else {
        None
    }
}

/// flowgraph.c basicblock_addop
fn basicblock_addop(block: &mut Block, mut info: InstructionInfo) -> crate::InternalResult<()> {
    let opcode = AnyOpcode::from(info.instr);
    debug_assert!(is_within_opcode_range(opcode));
    debug_assert!(!info.instr.is_assembler());
    debug_assert!(
        info.instr.has_arg() || info.instr.has_target() || u32::from(info.arg) == 0,
        "CPython basicblock_addop requires OPCODE_HAS_ARG, HAS_TARGET, or oparg == 0"
    );
    debug_assert!(
        u32::from(info.arg) < (1 << 30),
        "CPython basicblock_addop requires 0 <= oparg < (1 << 30)"
    );
    let off = basicblock_next_instr(block)?;
    let except_handler = block.instructions[off].except_handler;
    info.target = BlockIdx::NULL;
    info.except_handler = except_handler;
    block.instructions[off] = info;
    Ok(())
}

/// flowgraph.c basicblock_insert_instruction
fn basicblock_insert_instruction(
    block: &mut Block,
    pos: usize,
    info: InstructionInfo,
) -> crate::InternalResult<()> {
    let old_len = block.instruction_used;
    debug_assert!(pos <= old_len);
    basicblock_next_instr(block)?;
    for i in (pos + 1..=old_len).rev() {
        block.instructions[i] = block.instructions[i - 1];
    }
    block.instructions[pos] = info;
    Ok(())
}

/// flowgraph.c direct `b_iused = 0`
fn basicblock_clear(block: &mut Block) {
    block.instruction_used = 0;
}

/// CPython direct `b_instr[0]` access. Some passes set `b_iused = 0`
/// without clearing the backing array, so an empty basic block can still have
/// a first raw instruction slot.
fn basicblock_raw_first_instr_mut(block: &mut Block) -> &mut InstructionInfo {
    debug_assert!(block.instruction_allocation > 0);
    &mut block.instructions[0]
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InstructionSequenceLabel(i32);

/// flowgraph.c SAME_LABEL
fn same_label(a: InstructionSequenceLabel, b: InstructionSequenceLabel) -> bool {
    a == b
}

/// flowgraph.c IS_LABEL
fn is_label(label: InstructionSequenceLabel) -> bool {
    !same_label(label, InstructionSequenceLabel::NO_LABEL)
}

impl InstructionSequenceLabel {
    pub(crate) const NO_LABEL: Self = Self(-1);

    pub(crate) fn from_index(index: i32) -> Self {
        Self(index)
    }

    pub(crate) fn is_jump_target_label(self) -> bool {
        is_label(self)
    }

    pub(crate) fn idx(self) -> usize {
        debug_assert!(self.0 >= 0);
        self.0 as usize
    }
}

#[derive(Clone, Copy)]
struct InstructionSequenceExceptHandlerInfo {
    h_label: i32,
    start_depth: i32,
    preserve_lasti: i32,
}

const NO_EXCEPTION_HANDLER_LABEL: i32 = -1;
const ZERO_EXCEPTION_HANDLER_INFO: InstructionSequenceExceptHandlerInfo =
    InstructionSequenceExceptHandlerInfo {
        h_label: 0,
        start_depth: 0,
        preserve_lasti: 0,
    };

#[derive(Clone, Copy)]
struct InstructionSequenceEntry {
    info: InstructionInfo,
    except_handler: InstructionSequenceExceptHandlerInfo,
    i_target: i32,
    i_offset: i32,
}

impl InstructionSequenceEntry {
    fn new(info: InstructionInfo, except_handler: InstructionSequenceExceptHandlerInfo) -> Self {
        Self {
            info,
            except_handler,
            i_target: 0,
            i_offset: 0,
        }
    }
}

const INSTRUCTION_SEQUENCE_UNSET_LABEL: i32 = -111;

#[derive(Clone)]
pub(crate) struct InstructionSequence {
    /// CPython `instr_sequence.s_instrs`, including allocated slots beyond `s_used`.
    instrs: Vec<InstructionSequenceEntry>,
    /// CPython `instr_sequence.s_allocated`, the allocated size of `s_instrs`.
    instr_allocation: usize,
    /// CPython `instr_sequence.s_used`, the number of used entries in `s_instrs`.
    instr_used: usize,
    /// CPython `instr_sequence.s_next_free_label`.
    next_free_label: i32,
    label_map: Option<Vec<i32>>,
    label_map_allocation: usize,
    annotations_code: Option<Box<Self>>,
}

impl InstructionSequence {
    pub(crate) fn new() -> Self {
        instruction_sequence_new()
    }
}

/// instruction_sequence.c _PyInstructionSequence_New / inst_seq_create
fn instruction_sequence_new() -> InstructionSequence {
    InstructionSequence {
        instrs: Vec::new(),
        instr_allocation: 0,
        instr_used: 0,
        next_free_label: 0,
        label_map: None,
        label_map_allocation: 0,
        annotations_code: None,
    }
}

/// instruction_sequence.c instr_sequence_next_inst
fn instruction_sequence_next_inst(seq: &mut InstructionSequence) -> crate::InternalResult<usize> {
    debug_assert!(!seq.instrs.is_empty() || seq.instr_used == 0);
    let idx = seq.instr_used;
    let new_allocation = c_array_ensure_capacity::<InstructionSequenceEntry>(
        seq.instr_allocation,
        idx + 1,
        INITIAL_INSTR_SEQUENCE_SIZE,
    )?;
    if new_allocation > seq.instr_allocation {
        if new_allocation > seq.instrs.capacity() {
            seq.instrs
                .try_reserve_exact(new_allocation - seq.instrs.capacity())
                .map_err(|_| InternalError::MalformedControlFlowGraph)?;
        }
        if new_allocation > seq.instrs.len() {
            seq.instrs.resize(
                new_allocation,
                InstructionSequenceEntry::new(
                    InstructionInfo {
                        instr: Instruction::Cache.into(),
                        arg: OpArg::new(0),
                        target: BlockIdx::NULL,
                        location: SourceLocation::default(),
                        end_location: SourceLocation::default(),
                        except_handler: None,
                        lineno_override: None,
                    },
                    ZERO_EXCEPTION_HANDLER_INFO,
                ),
            );
        }
        seq.instr_allocation = new_allocation;
    }
    debug_assert!(seq.instr_allocation > idx);
    seq.instr_used += 1;
    Ok(idx)
}

/// instruction_sequence.c _PyInstructionSequence_NewLabel
fn instruction_sequence_new_label(seq: &mut InstructionSequence) -> InstructionSequenceLabel {
    seq.next_free_label += 1;
    InstructionSequenceLabel(seq.next_free_label)
}

/// instruction_sequence.c _PyInstructionSequence_Addop asserts.
fn instruction_sequence_debug_check_addop(info: &InstructionInfo) {
    let opcode = AnyOpcode::from(info.instr);
    debug_assert!(is_within_opcode_range(opcode));
    debug_assert!(
        opcode.has_arg() || info.instr.has_target() || u32::from(info.arg) == 0,
        "CPython _PyInstructionSequence_Addop requires either OPCODE_HAS_ARG, HAS_TARGET, or oparg == 0"
    );
    debug_assert!(
        u32::from(info.arg) < (1 << 30),
        "CPython _PyInstructionSequence_Addop requires 0 <= oparg < (1 << 30)"
    );
}

/// instruction_sequence.c _PyInstructionSequence_SetAnnotationsCode
fn instruction_sequence_set_annotations_code(
    seq: &mut InstructionSequence,
    annotations_code: Option<Box<InstructionSequence>>,
) {
    debug_assert!(seq.annotations_code.is_none());
    seq.annotations_code = annotations_code;
}

/// instruction_sequence.c _PyInstructionSequence_UseLabel
#[allow(clippy::needless_range_loop)]
fn instruction_sequence_use_label(
    seq: &mut InstructionSequence,
    label: InstructionSequenceLabel,
) -> crate::InternalResult<()> {
    let old_size = seq.label_map_allocation;
    let new_allocation = c_array_ensure_capacity::<i32>(
        seq.label_map_allocation,
        label.idx(),
        INITIAL_INSTR_SEQUENCE_LABELS_MAP_SIZE,
    )?;
    if new_allocation > seq.label_map_allocation {
        if let Some(label_map) = &mut seq.label_map {
            if new_allocation > label_map.capacity() {
                label_map
                    .try_reserve_exact(new_allocation - label_map.capacity())
                    .map_err(|_| InternalError::MalformedControlFlowGraph)?;
            }
        } else {
            let mut label_map = Vec::new();
            label_map
                .try_reserve_exact(new_allocation)
                .map_err(|_| InternalError::MalformedControlFlowGraph)?;
            seq.label_map = Some(label_map);
        }
        seq.label_map_allocation = new_allocation;
    }
    let label_map = seq
        .label_map
        .as_mut()
        .ok_or(InternalError::MalformedControlFlowGraph)?;
    if label_map.len() < seq.label_map_allocation {
        label_map.resize(seq.label_map_allocation, INSTRUCTION_SEQUENCE_UNSET_LABEL);
    }
    for i in old_size..seq.label_map_allocation {
        label_map[i] = INSTRUCTION_SEQUENCE_UNSET_LABEL;
    }
    label_map[label.idx()] = seq.instr_used as i32;
    Ok(())
}

/// instruction_sequence.c _PyInstructionSequence_Addop
fn instruction_sequence_addop(
    seq: &mut InstructionSequence,
    info: InstructionInfo,
) -> crate::InternalResult<&mut InstructionSequenceEntry> {
    instruction_sequence_debug_check_addop(&info);
    let idx = instruction_sequence_next_inst(seq)?;
    let entry = &mut seq.instrs[idx];
    entry.info = info;
    Ok(entry)
}

fn instruction_sequence_last_info_mut(
    seq: &mut InstructionSequence,
) -> Option<&mut InstructionInfo> {
    if seq.instr_used == 0 {
        None
    } else {
        Some(&mut seq.instrs[seq.instr_used - 1].info)
    }
}

/// instruction_sequence.c _PyInstructionSequence_InsertInstruction
#[allow(clippy::needless_range_loop)]
fn instruction_sequence_insert_instruction(
    seq: &mut InstructionSequence,
    pos: usize,
    info: InstructionInfo,
) -> crate::InternalResult<()> {
    debug_assert!(pos <= seq.instr_used);
    let last_idx = instruction_sequence_next_inst(seq)?;
    for i in (pos..last_idx).rev() {
        seq.instrs[i + 1] = seq.instrs[i];
    }
    seq.instrs[pos].info = info;
    if let Some(label_map) = &mut seq.label_map {
        let pos = pos as i32;
        for lbl in 0..seq.label_map_allocation {
            if label_map[lbl] >= pos {
                label_map[lbl] += 1;
            }
        }
    }
    Ok(())
}

/// instruction_sequence.c _PyInstructionSequence_ApplyLabelMap
#[allow(clippy::needless_range_loop, clippy::unnecessary_wraps)]
fn instruction_sequence_apply_label_map(
    instrs: &mut InstructionSequence,
) -> crate::InternalResult<()> {
    {
        let Some(label_map) = instrs.label_map.as_ref() else {
            return Ok(());
        };
        for i in 0..instrs.instr_used {
            let entry = &mut instrs.instrs[i];
            if entry.info.instr.has_target() {
                let label = u32::from(entry.info.arg) as usize;
                debug_assert!(label < instrs.label_map_allocation);
                let target = label_map[label];
                debug_assert!(target >= 0);
                entry.info.arg = OpArg::new(target as u32);
            }
            let handler = &mut entry.except_handler;
            if handler.h_label >= 0 {
                let label = handler.h_label as usize;
                debug_assert!(label < instrs.label_map_allocation);
                handler.h_label = label_map[label];
            }
        }
    }
    instrs.label_map = None;
    instrs.label_map_allocation = 0;
    Ok(())
}

/// assemble.c instr_size
fn instr_size(instr: &InstructionInfo) -> usize {
    let opcode = instr.instr.expect_real();
    let oparg = u32::from(instr.arg) as i32;
    debug_assert!(
        instr.instr.has_arg() || oparg == 0,
        "CPython assemble.c instr_size requires OPCODE_HAS_ARG or oparg == 0"
    );
    let extended_args =
        (0xFF_FFFF < oparg) as usize + (0xFF_FF < oparg) as usize + (0xFF < oparg) as usize;
    let caches = opcode.cache_entries();
    extended_args + 1 + caches
}

/// pycore_opcode_metadata.h is_pseudo_target
const fn is_pseudo_target(pseudo: PseudoOpcode, target: Opcode) -> bool {
    match pseudo {
        PseudoOpcode::LoadClosure => matches!(target, Opcode::LoadFast),
        PseudoOpcode::StoreFastMaybeNull => matches!(target, Opcode::StoreFast),
        PseudoOpcode::AnnotationsPlaceholder
        | PseudoOpcode::SetupFinally
        | PseudoOpcode::SetupCleanup
        | PseudoOpcode::SetupWith
        | PseudoOpcode::PopBlock => matches!(target, Opcode::Nop),
        PseudoOpcode::Jump => matches!(target, Opcode::JumpForward | Opcode::JumpBackward),
        PseudoOpcode::JumpNoInterrupt => {
            matches!(
                target,
                Opcode::JumpForward | Opcode::JumpBackwardNoInterrupt
            )
        }
        PseudoOpcode::JumpIfFalse => {
            matches!(
                target,
                Opcode::Copy | Opcode::ToBool | Opcode::PopJumpIfFalse
            )
        }
        PseudoOpcode::JumpIfTrue => {
            matches!(
                target,
                Opcode::Copy | Opcode::ToBool | Opcode::PopJumpIfTrue
            )
        }
    }
}
/// assemble.c resolve_unconditional_jumps
#[allow(clippy::unnecessary_wraps)]
fn resolve_unconditional_jumps(
    instr_sequence: &mut InstructionSequence,
) -> crate::InternalResult<()> {
    for i in 0..instr_sequence.instr_used {
        let instr = &mut instr_sequence.instrs[i].info;
        let is_forward = (u32::from(instr.arg) as i32) > i as i32;
        match instr.instr {
            AnyInstruction::Pseudo(PseudoInstruction::Jump { .. }) => {
                debug_assert!(is_pseudo_target(PseudoOpcode::Jump, Opcode::JumpForward));
                debug_assert!(is_pseudo_target(PseudoOpcode::Jump, Opcode::JumpBackward));

                if is_forward {
                    instr.instr = Opcode::JumpForward.into();
                } else {
                    instr.instr = Opcode::JumpBackward.into();
                }
            }
            AnyInstruction::Pseudo(PseudoInstruction::JumpNoInterrupt { .. }) => {
                debug_assert!(is_pseudo_target(
                    PseudoOpcode::JumpNoInterrupt,
                    Opcode::JumpForward
                ));
                debug_assert!(is_pseudo_target(
                    PseudoOpcode::JumpNoInterrupt,
                    Opcode::JumpBackwardNoInterrupt
                ));
                if is_forward {
                    instr.instr = Opcode::JumpForward.into();
                } else {
                    instr.instr = Opcode::JumpBackwardNoInterrupt.into();
                }
            }
            _ => {
                if instr.instr.has_jump() && matches!(instr.instr, AnyInstruction::Pseudo(_)) {
                    unreachable!("remaining pseudo jump in resolve_unconditional_jumps");
                }
            }
        }
    }
    Ok(())
}

/// assemble.c resolve_jump_offsets
#[allow(clippy::needless_range_loop, clippy::unnecessary_wraps)]
fn resolve_jump_offsets(instr_sequence: &mut InstructionSequence) -> crate::InternalResult<()> {
    // The offset (in code units) of END_SEND from SEND in the yield-from sequence.
    const END_SEND_OFFSET: i32 = 5;
    for i in 0..instr_sequence.instr_used {
        let instr = &mut instr_sequence.instrs[i];
        let opcode = instr.info.instr.expect_real();
        if opcode.has_jump() {
            instr.i_target = u32::from(instr.info.arg) as i32;
        }
    }
    let mut extended_arg_recompile;
    loop {
        let mut totsize = 0i32;
        for i in 0..instr_sequence.instr_used {
            let instr = &mut instr_sequence.instrs[i];
            instr.i_offset = totsize;
            let isize = instr_size(&instr.info);
            totsize += isize as i32;
        }
        extended_arg_recompile = false;
        let mut offset = 0i32;
        for i in 0..instr_sequence.instr_used {
            let isize = instr_size(&instr_sequence.instrs[i].info);
            // Jump offsets are computed relative to the instruction pointer
            // after fetching the jump instruction.
            offset += isize as i32;

            let opcode = instr_sequence.instrs[i].info.instr.expect_real();
            if opcode.has_jump() {
                let target = instr_sequence.instrs[i].i_target;
                let target_offset = instr_sequence.instrs[target as usize].i_offset;
                let info = &mut instr_sequence.instrs[i].info;
                let op = opcode;
                let mut oparg = target_offset;
                info.arg = OpArg::new(oparg as u32);
                if matches!(op, Instruction::EndAsyncFor) {
                    oparg = offset - oparg - END_SEND_OFFSET;
                } else if oparg < offset {
                    debug_assert!(matches!(
                        op.into(),
                        Opcode::JumpBackward | Opcode::JumpBackwardNoInterrupt
                    ));
                    oparg = offset - oparg;
                } else {
                    debug_assert!(!matches!(
                        op.into(),
                        Opcode::JumpBackward | Opcode::JumpBackwardNoInterrupt
                    ));
                    oparg -= offset;
                }
                info.arg = OpArg::new(oparg as u32);
                if instr_size(info) != isize {
                    extended_arg_recompile = true;
                }
            }
        }

        if !extended_arg_recompile {
            break;
        }
    }

    Ok(())
}

struct AssembledCode {
    instructions: Vec<CodeUnit>,
    linetable: Box<[u8]>,
    exceptiontable: Box<[u8]>,
}

struct LocalsPlusInfo {
    cellvars: Box<[String]>,
    kinds: Box<[u8]>,
}

/// assemble.c same_location
fn same_location(a: LineTableLocation, b: LineTableLocation) -> bool {
    a.line == b.line && a.end_line == b.end_line && a.col == b.col && a.end_col == b.end_col
}

fn instruction_linetable_location(info: &InstructionInfo) -> LineTableLocation {
    match info.lineno_override {
        Some(NO_LOCATION_OVERRIDE) => LineTableLocation {
            line: NO_LOCATION_OVERRIDE,
            end_line: NO_LOCATION_OVERRIDE,
            col: NO_LOCATION_OVERRIDE,
            end_col: NO_LOCATION_OVERRIDE,
        },
        Some(LINE_ONLY_LOCATION_OVERRIDE) => LineTableLocation {
            line: info.location.line.get() as i32,
            end_line: info.end_location.line.get() as i32,
            col: -1,
            end_col: -1,
        },
        Some(NEXT_LOCATION_OVERRIDE) => next_linetable_location(),
        Some(lineno) => LineTableLocation {
            line: lineno,
            end_line: info.end_location.line.get() as i32,
            col: info.location.character_offset.to_zero_indexed() as i32,
            end_col: info.end_location.character_offset.to_zero_indexed() as i32,
        },
        None => LineTableLocation {
            line: info.location.line.get() as i32,
            end_line: info.end_location.line.get() as i32,
            col: info.location.character_offset.to_zero_indexed() as i32,
            end_col: info.end_location.character_offset.to_zero_indexed() as i32,
        },
    }
}

/// assemble.c write_instr
fn write_instr(instructions: &mut Vec<CodeUnit>, info: &InstructionInfo, ilen: usize) {
    let opcode = info.instr.expect_real();
    let oparg = u32::from(info.arg) as i32;
    debug_assert!(
        info.instr.has_arg() || oparg == 0,
        "CPython assemble.c write_instr requires OPCODE_HAS_ARG or oparg == 0"
    );
    let caches = opcode.cache_entries();
    let non_cache_units = ilen - caches;
    match non_cache_units {
        1..=4 => {}
        _ => unreachable!("CPython write_instr expects 1 to 4 non-cache code units"),
    }
    if non_cache_units >= 4 {
        instructions.push(CodeUnit::new(
            Instruction::ExtendedArg,
            OpArgByte::new(((oparg >> 24) & 0xff) as u8),
        ));
    }
    if non_cache_units >= 3 {
        instructions.push(CodeUnit::new(
            Instruction::ExtendedArg,
            OpArgByte::new(((oparg >> 16) & 0xff) as u8),
        ));
    }
    if non_cache_units >= 2 {
        instructions.push(CodeUnit::new(
            Instruction::ExtendedArg,
            OpArgByte::new(((oparg >> 8) & 0xff) as u8),
        ));
    }
    instructions.push(CodeUnit::new(opcode, OpArgByte::new((oparg & 0xff) as u8)));
    for _ in 0..caches {
        instructions.push(CodeUnit::new(Instruction::Cache, OpArgByte::new(0)));
    }
}

/// assemble.c assemble_emit_instr
fn assemble_emit_instr(
    instructions: &mut Vec<CodeUnit>,
    info: &mut InstructionInfo,
) -> crate::InternalResult<()> {
    let size = instr_size(info);
    let required = instructions
        .len()
        .checked_add(size)
        .ok_or(InternalError::MalformedControlFlowGraph)?;
    if required >= instructions.capacity() {
        vec_try_resize_to_double_capacity(instructions)?;
    }
    write_instr(instructions, info, size);
    Ok(())
}

/// assemble.c assemble_location_info
#[allow(clippy::needless_range_loop)]
fn assemble_location_info(
    instr_sequence: &mut InstructionSequence,
    first_line: i32,
    debug_ranges: bool,
) -> crate::InternalResult<Box<[u8]>> {
    for i in (0..instr_sequence.instr_used).rev() {
        let loc = instruction_linetable_location(&instr_sequence.instrs[i].info);
        if same_location(loc, next_linetable_location()) {
            if instr_sequence.instrs[i]
                .info
                .instr
                .expect_real()
                .is_terminator()
            {
                instr_sequence.instrs[i].info.lineno_override = Some(NO_LOCATION_OVERRIDE);
            } else {
                debug_assert!(i < instr_sequence.instr_used - 1);
                let next = instr_sequence.instrs[i + 1].info;
                instr_set_loc(
                    &mut instr_sequence.instrs[i].info,
                    next.location,
                    next.end_location,
                    next.lineno_override,
                );
            }
        }
    }

    let mut linetable = Vec::new();
    vec_try_reserve_exact(&mut linetable, DEFAULT_CNOTAB_SIZE)?;
    let mut prev_line = first_line;
    let mut loc = no_linetable_location();
    let mut size = 0;
    for i in 0..instr_sequence.instr_used {
        let entry = &instr_sequence.instrs[i];
        let instr_loc = instruction_linetable_location(&entry.info);
        if !same_location(loc, instr_loc) {
            assemble_emit_location(&mut linetable, loc, size, &mut prev_line, debug_ranges)?;
            loc = instr_loc;
            size = 0;
        }
        size += instr_size(&entry.info);
    }
    assemble_emit_location(&mut linetable, loc, size, &mut prev_line, debug_ranges)?;
    Ok(linetable.into_boxed_slice())
}

/// assemble.c assemble_emit
fn assemble_emit(
    instr_sequence: &mut InstructionSequence,
    first_line: i32,
    debug_ranges: bool,
) -> crate::InternalResult<AssembledCode> {
    let mut instructions = Vec::new();
    vec_try_reserve_exact(
        &mut instructions,
        DEFAULT_CODE_SIZE / core::mem::size_of::<CodeUnit>(),
    )?;

    for i in 0..instr_sequence.instr_used {
        let instr = &mut instr_sequence.instrs[i].info;
        assemble_emit_instr(&mut instructions, instr)?;
    }

    let linetable = assemble_location_info(instr_sequence, first_line, debug_ranges)?;

    let exceptiontable =
        assemble_exception_table(&instr_sequence.instrs[..instr_sequence.instr_used])?;

    Ok(AssembledCode {
        instructions,
        linetable,
        exceptiontable,
    })
}

/// assemble.c compute_localsplus_info
fn compute_localsplus_info(
    umd: &CodeUnitMetadata,
    nlocalsplus: usize,
    flags: CodeFlags,
) -> crate::InternalResult<LocalsPlusInfo> {
    let nlocals = umd.varnames.len();
    let ncells = umd.cellvars.len();
    let nfrees = umd.freevars.len();
    let mut localspluskinds = Vec::new();
    vec_try_reserve_exact(&mut localspluskinds, nlocalsplus)?;
    localspluskinds.resize(nlocalsplus, 0);
    let mut cellvars = Vec::new();
    vec_try_reserve_exact(&mut cellvars, ncells)?;

    let argvarkinds = [
        (umd.posonlyargcount as usize, CO_FAST_ARG_POS),
        (umd.argcount as usize, CO_FAST_ARG_POS | CO_FAST_ARG_KW),
        (umd.kwonlyargcount as usize, CO_FAST_ARG_KW),
        (
            usize::from(flags.contains(CodeFlags::VARARGS)),
            CO_FAST_ARG_VAR | CO_FAST_ARG_POS,
        ),
        (
            usize::from(flags.contains(CodeFlags::VARKEYWORDS)),
            CO_FAST_ARG_VAR | CO_FAST_ARG_KW,
        ),
        (usize::MAX, 0),
    ];
    let mut pos = 0usize;
    let mut max = 0usize;
    for (count, argkind) in argvarkinds {
        max = if count == usize::MAX {
            usize::MAX
        } else {
            max + count
        };
        while pos < max && pos < nlocals {
            let name = umd
                .varnames
                .get_index(pos)
                .expect("varname index is in range")
                .as_str();
            let mut kind = CO_FAST_LOCAL | argkind;
            if umd.fast_hidden.get(name).copied().unwrap_or(false)
                || umd.fast_hidden_final.contains(name)
            {
                kind |= CO_FAST_HIDDEN;
            }
            if umd.cellvars.contains(name) {
                kind |= CO_FAST_CELL;
                cellvars.push(name.to_owned());
            }
            localspluskinds[pos] = kind;
            pos += 1;
        }
    }

    let mut numdropped = 0usize;
    let mut cellvar_offset = -1i32;
    for i in 0..ncells {
        let name = umd
            .cellvars
            .get_index(i)
            .expect("cellvar index is in range")
            .as_str();
        if umd.varnames.contains(name) {
            numdropped += 1;
            continue;
        }
        let offset = i + nlocals - numdropped;
        debug_assert!(offset < nlocalsplus);
        cellvars.push(name.to_owned());
        localspluskinds[offset] = CO_FAST_CELL;
        cellvar_offset = offset as i32;
    }

    for i in 0..nfrees {
        let offset = ncells + i + nlocals - numdropped;
        debug_assert!(offset < nlocalsplus);
        debug_assert!((offset as i32) > cellvar_offset);
        localspluskinds[offset] = CO_FAST_FREE;
    }

    debug_assert_eq!(
        nlocalsplus,
        nlocals + ncells - numdropped + nfrees,
        "CPython prepare_localsplus() result must match assemble.c localsplus sizing"
    );
    debug_assert_eq!(cellvars.len(), ncells);
    Ok(LocalsPlusInfo {
        cellvars: cellvars.into_boxed_slice(),
        kinds: localspluskinds.into_boxed_slice(),
    })
}

#[derive(Debug, Clone)]
pub struct Block {
    /// CPython `basicblock.b_list`, allocation-order list distinct from CFG `b_next`.
    allocation_next: BlockIdx,
    /// CPython `basicblock.b_label` used by translate_jump_labels_to_targets.
    cpython_label: InstructionSequenceLabel,
    /// CPython `basicblock.b_ialloc`, the allocated size of `b_instr`.
    instruction_allocation: usize,
    /// Exception stack at start of block, used by label_exception_targets (b_exceptstack)
    except_stack: Option<CfgExceptStack>,
    /// CPython `basicblock.b_instr`, including allocated slots beyond `b_iused`.
    pub instructions: Vec<InstructionInfo>,
    pub next: BlockIdx,
    /// CPython `basicblock.b_iused`, the number of used entries in `b_instr`.
    instruction_used: usize,
    /// Potentially uninitialized locals mask for local-check analysis (b_unsafe_locals_mask)
    unsafe_locals_mask: u64,
    /// Number of incoming CFG edges from reachable blocks (b_predecessors)
    predecessors: i32,
    /// Stack depth at block entry, set by stack depth analysis
    pub start_depth: i32,
    /// Whether to preserve lasti for this handler block (b_preserve_lasti)
    pub preserve_lasti: bool,
    /// Temporary traversal mark used by CFG passes (b_visited)
    visited: bool,
    /// Whether this block is an exception handler target (b_except_handler)
    pub except_handler: bool,
    /// Whether this block is only reachable via exception table (b_cold)
    pub cold: bool,
    /// Definitely reachable outside exception-only paths (b_warm)
    warm: bool,
}

impl Default for Block {
    fn default() -> Self {
        Self {
            allocation_next: BlockIdx::NULL,
            cpython_label: InstructionSequenceLabel::NO_LABEL,
            instruction_allocation: 0,
            except_stack: None,
            instructions: Vec::new(),
            next: BlockIdx::NULL,
            instruction_used: 0,
            unsafe_locals_mask: 0,
            predecessors: 0,
            start_depth: START_DEPTH_UNSET,
            preserve_lasti: false,
            visited: false,
            except_handler: false,
            cold: false,
            warm: false,
        }
    }
}

impl Block {
    pub(crate) fn used_instructions(&self) -> &[InstructionInfo] {
        &self.instructions[..self.instruction_used]
    }

    #[must_use]
    pub(crate) const fn is_empty(&self) -> bool {
        self.instruction_used == 0
    }
}

#[derive(Clone, Debug, Default)]
pub struct Blocks(Vec<Block>);

// Vec like methods
impl Blocks {
    pub fn try_reserve(
        &mut self,
        additional: usize,
    ) -> Result<(), alloc::collections::TryReserveError> {
        self.0.try_reserve(additional)
    }

    pub fn push(&mut self, value: Block) {
        self.0.push(value)
    }
}

// CPython functions

impl Blocks {
    /// # See also
    /// [CPython's remove_unreachable](https://github.com/python/cpython/blob/v3.14.6/Python/flowgraph.c#L995-L1041)
    pub fn remove_unreachable(&mut self) -> crate::InternalResult<()> {
        let mut block_idx = BlockIdx(0);
        while block_idx != BlockIdx::NULL {
            self[block_idx].predecessors = 0;
            block_idx = self[block_idx].next;
        }

        let mut stack = self.make_cfg_traversal_stack()?;
        self[0].predecessors = 1;
        stack.push(BlockIdx(0));
        self[0].visited = true;
        while let Some(current) = stack.pop() {
            let idx = current.idx();
            let next = self[idx].next;
            if next != BlockIdx::NULL && bb_has_fallthrough(&self[idx]) {
                if !self[next].visited {
                    debug_assert_eq!(self[next].predecessors, 0);
                    stack.push(next);
                    self[next].visited = true;
                }
                self[next].predecessors += 1;
            }

            let instr_count = self[idx].instruction_used;
            for i in 0..instr_count {
                let instr = self[idx].instructions[i];
                if is_jump(&instr) || is_block_push(&instr) {
                    let target = instr.target;
                    debug_assert!(target != BlockIdx::NULL);
                    let target_idx = target.idx();
                    if !self[target_idx].visited {
                        stack.push(target);
                        self[target_idx].visited = true;
                    }
                    self[target_idx].predecessors += 1;
                }
            }
        }

        block_idx = BlockIdx(0);
        while block_idx != BlockIdx::NULL {
            let next = self[block_idx].next;
            if self[block_idx].predecessors == 0 {
                let block = &mut self[block_idx];
                basicblock_clear(block);
                block.except_handler = false;
            }
            block_idx = next;
        }
        Ok(())
    }

    /// flowgraph.c basicblock_append_instructions
    fn basicblock_append_block_instructions(
        &mut self,
        to: BlockIdx,
        from: BlockIdx,
    ) -> crate::InternalResult<()> {
        debug_assert_ne!(to, from);

        let from_len = self[from].instruction_used;
        for i in 0..from_len {
            let info = self[from].instructions[i];
            let off = basicblock_next_instr(&mut self[to])?;
            self[to].instructions[off] = info;
        }

        Ok(())
    }

    /// flowgraph.c copy_basicblock
    fn copy_basicblock(&mut self, block_idx: BlockIdx) -> crate::InternalResult<BlockIdx> {
        debug_assert!(bb_no_fallthrough(&self[block_idx]));

        let result = blocks_new_block(self)?;
        self.basicblock_append_block_instructions(result, block_idx)?;
        Ok(result)
    }

    fn duplicate_exits_without_lineno(&mut self) -> crate::InternalResult<()> {
        let mut next_lbl = get_max_label(self) + 1;

        let entryblock = BlockIdx(0);
        let mut b = entryblock;
        while b != BlockIdx::NULL {
            let Some(last) = basicblock_last_instr(&self[b]).copied() else {
                b = self[b].next;
                continue;
            };

            if is_jump(&last) {
                debug_assert!(last.target != BlockIdx::NULL);

                let target = next_nonempty_block(self, last.target);

                debug_assert!(target != BlockIdx::NULL);

                if is_exit_or_eval_check_without_lineno(&self[target])
                    && self[target].predecessors > 1
                {
                    let new_target = self.copy_basicblock(target)?;
                    instr_set_location(
                        &mut self[new_target].instructions[0],
                        instr_location(&last),
                    );
                    let last_mut = basicblock_last_instr_mut(&mut self[b]).unwrap();
                    last_mut.target = new_target;
                    self[target].predecessors -= 1;
                    self[new_target].predecessors = 1;
                    self[new_target].next = self[target].next;
                    self[new_target].cpython_label = InstructionSequenceLabel(next_lbl);
                    next_lbl += 1;
                    self[target].next = new_target;
                }
            }
            b = self[b].next;
        }

        b = entryblock;
        while b != BlockIdx::NULL {
            let next = self[b].next;
            if bb_has_fallthrough(&self[b])
                && next != BlockIdx::NULL
                && self[b].instruction_used != 0
                && is_exit_or_eval_check_without_lineno(&self[next])
            {
                let last = *basicblock_last_instr(&self[b]).expect("block has instructions");
                instr_set_location(&mut self[next].instructions[0], instr_location(&last));
            }
            b = self[b].next;
        }

        Ok(())
    }

    fn resolve_line_numbers(&mut self, _firstlineno: OneIndexed) -> crate::InternalResult<()> {
        self.duplicate_exits_without_lineno()?;
        self.propagate_line_numbers();
        Ok(())
    }

    /// flowgraph.c optimize_basic_block
    fn optimize_basic_block(
        &mut self,
        metadata: &mut CodeUnitMetadata,
        block_idx: BlockIdx,
    ) -> crate::InternalResult<()> {
        let mut nop = InstructionInfo {
            instr: Instruction::Nop.into(),
            arg: OpArg::NULL,
            target: BlockIdx::NULL,
            location: SourceLocation::default(),
            end_location: SourceLocation::default(),
            except_handler: None,
            lineno_override: None,
        };
        instr_set_op0(&mut nop, Instruction::Nop.into());
        let mut i = 0;
        while i < self[block_idx].instruction_used {
            let inst = self[block_idx].instructions[i];
            debug_assert!(!inst.instr.is_assembler());
            let target = if inst.instr.has_target() {
                let target = inst.target;
                debug_assert!(target != BlockIdx::NULL);
                debug_assert!(self[target.idx()].instruction_used != 0);
                debug_assert!(!self[target.idx()].instructions[0].instr.is_assembler());
                self[target.idx()].instructions[0]
            } else {
                nop
            };

            let nextop = self[block_idx]
                .instructions
                .get(i + 1)
                .and_then(|next| next.instr.real());

            match inst.instr {
                AnyInstruction::Real(Instruction::BuildTuple { .. }) => {
                    let oparg = u32::from(inst.arg);
                    if matches!(nextop, Some(Instruction::UnpackSequence { .. }))
                        && u32::from(self[block_idx].instructions[i + 1].arg) == oparg
                    {
                        match oparg {
                            1 => {
                                set_to_nop(&mut self[block_idx].instructions[i]);
                                set_to_nop(&mut self[block_idx].instructions[i + 1]);
                                i += 1;
                                continue;
                            }
                            2 | 3 => {
                                set_to_nop(&mut self[block_idx].instructions[i]);
                                self[block_idx].instructions[i + 1].instr = Opcode::Swap.into();
                                i += 1;
                                continue;
                            }
                            _ => {}
                        }
                    }
                    fold_tuple_of_constants(metadata, &mut self[block_idx], i)?;
                }
                AnyInstruction::Real(
                    Instruction::BuildList { .. } | Instruction::BuildSet { .. },
                ) => {
                    optimize_lists_and_sets(metadata, &mut self[block_idx], i, nextop)?;
                }
                AnyInstruction::Real(
                    Instruction::PopJumpIfNotNone { .. } | Instruction::PopJumpIfNone { .. },
                ) if matches!(target.instr.into(), AnyOpcode::Pseudo(PseudoOpcode::Jump))
                    && jump_thread(self, block_idx, i, &target, inst.instr)? =>
                {
                    continue;
                }
                AnyInstruction::Real(Instruction::PopJumpIfFalse { .. })
                    if matches!(target.instr.into(), AnyOpcode::Pseudo(PseudoOpcode::Jump))
                        && jump_thread(self, block_idx, i, &target, inst.instr)? =>
                {
                    continue;
                }
                AnyInstruction::Real(Instruction::PopJumpIfTrue { .. })
                    if matches!(target.instr.into(), AnyOpcode::Pseudo(PseudoOpcode::Jump))
                        && jump_thread(self, block_idx, i, &target, inst.instr)? =>
                {
                    continue;
                }
                AnyInstruction::Pseudo(
                    pseudo @ (PseudoInstruction::JumpIfFalse { .. }
                    | PseudoInstruction::JumpIfTrue { .. }),
                ) => {
                    let opcode = pseudo.into();
                    match target.instr.pseudo().map(Into::into) {
                        Some(PseudoOpcode::Jump)
                            if jump_thread(self, block_idx, i, &target, opcode)? =>
                        {
                            continue;
                        }
                        Some(PseudoOpcode::JumpIfFalse)
                            if matches!(
                                opcode,
                                AnyInstruction::Pseudo(PseudoInstruction::JumpIfFalse { .. })
                            ) && jump_thread(self, block_idx, i, &target, opcode)? =>
                        {
                            continue;
                        }
                        Some(PseudoOpcode::JumpIfTrue)
                            if matches!(
                                opcode,
                                AnyInstruction::Pseudo(PseudoInstruction::JumpIfTrue { .. })
                            ) && jump_thread(self, block_idx, i, &target, opcode)? =>
                        {
                            continue;
                        }
                        Some(PseudoOpcode::JumpIfFalse | PseudoOpcode::JumpIfTrue) => {
                            let next = self[inst.target.idx()].next;
                            debug_assert!(next != BlockIdx::NULL);
                            debug_assert!(next != inst.target);
                            self[block_idx].instructions[i].target = next;
                            continue;
                        }
                        _ => {}
                    }
                }
                AnyInstruction::Pseudo(
                    PseudoInstruction::Jump { .. } | PseudoInstruction::JumpNoInterrupt { .. },
                ) => match target.instr.into() {
                    AnyOpcode::Pseudo(PseudoOpcode::Jump)
                        if jump_thread(self, block_idx, i, &target, PseudoOpcode::Jump.into())? =>
                    {
                        continue;
                    }
                    AnyOpcode::Pseudo(PseudoOpcode::JumpNoInterrupt)
                        if jump_thread(self, block_idx, i, &target, inst.instr)? =>
                    {
                        continue;
                    }
                    _ => {}
                },
                // CPython leaves FOR_ITER jump threading disabled.
                AnyInstruction::Real(Instruction::ForIter { .. }) => {}
                AnyInstruction::Real(Instruction::StoreFast { .. })
                    if matches!(nextop, Some(Instruction::StoreFast { .. }))
                        && u32::from(inst.arg)
                            == u32::from(self[block_idx].instructions[i + 1].arg)
                        && instruction_lineno(&self[block_idx].instructions[i])
                            == instruction_lineno(&self[block_idx].instructions[i + 1]) =>
                {
                    self[block_idx].instructions[i].instr = Instruction::PopTop.into();
                    self[block_idx].instructions[i].arg = OpArg::NULL;
                }
                AnyInstruction::Real(Instruction::Swap { .. }) if u32::from(inst.arg) == 1 => {
                    set_to_nop(&mut self[block_idx].instructions[i]);
                }
                AnyInstruction::Real(Instruction::LoadGlobal { .. })
                    if matches!(nextop, Some(Instruction::PushNull))
                        && (u32::from(inst.arg) & 1) == 0 =>
                {
                    instr_set_op1(
                        &mut self[block_idx].instructions[i],
                        inst.instr,
                        OpArg::new(u32::from(inst.arg) | 1),
                    );
                    set_to_nop(&mut self[block_idx].instructions[i + 1]);
                }
                AnyInstruction::Real(Instruction::CompareOp { .. })
                    if matches!(nextop, Some(Instruction::ToBool)) =>
                {
                    set_to_nop(&mut self[block_idx].instructions[i]);
                    instr_set_op1(
                        &mut self[block_idx].instructions[i + 1],
                        inst.instr,
                        OpArg::new(u32::from(inst.arg) | oparg::COMPARE_OP_BOOL_MASK),
                    );
                    i += 1;
                    continue;
                }
                AnyInstruction::Real(Instruction::ContainsOp { .. } | Instruction::IsOp { .. })
                    if matches!(nextop, Some(Instruction::ToBool)) =>
                {
                    set_to_nop(&mut self[block_idx].instructions[i]);
                    instr_set_op1(
                        &mut self[block_idx].instructions[i + 1],
                        inst.instr,
                        inst.arg,
                    );
                    i += 1;
                    continue;
                }
                AnyInstruction::Real(Instruction::ContainsOp { .. } | Instruction::IsOp { .. })
                    if matches!(nextop, Some(Instruction::UnaryNot)) =>
                {
                    set_to_nop(&mut self[block_idx].instructions[i]);
                    let inverted = u32::from(inst.arg) ^ 1;
                    debug_assert!(inverted == 0 || inverted == 1);
                    instr_set_op1(
                        &mut self[block_idx].instructions[i + 1],
                        inst.instr,
                        OpArg::new(inverted),
                    );
                    i += 1;
                    continue;
                }
                AnyInstruction::Real(Instruction::ToBool)
                    if matches!(nextop, Some(Instruction::ToBool)) =>
                {
                    set_to_nop(&mut self[block_idx].instructions[i]);
                    i += 1;
                    continue;
                }
                AnyInstruction::Real(Instruction::UnaryNot) => {
                    if matches!(nextop, Some(Instruction::ToBool)) {
                        set_to_nop(&mut self[block_idx].instructions[i]);
                        instr_set_op0(&mut self[block_idx].instructions[i + 1], inst.instr);
                        i += 1;
                        continue;
                    }
                    if matches!(nextop, Some(Instruction::UnaryNot)) {
                        set_to_nop(&mut self[block_idx].instructions[i]);
                        set_to_nop(&mut self[block_idx].instructions[i + 1]);
                        i += 1;
                        continue;
                    }
                    fold_const_unaryop(metadata, &mut self[block_idx], i)?;
                }
                AnyInstruction::Real(Instruction::UnaryInvert | Instruction::UnaryNegative) => {
                    fold_const_unaryop(metadata, &mut self[block_idx], i)?;
                }
                AnyInstruction::Real(Instruction::CallIntrinsic1 { func }) => {
                    match func.get(inst.arg) {
                        IntrinsicFunction1::ListToTuple => {
                            if matches!(nextop, Some(Instruction::GetIter)) {
                                set_to_nop(&mut self[block_idx].instructions[i]);
                            } else {
                                fold_constant_intrinsic_list_to_tuple(
                                    metadata,
                                    &mut self[block_idx],
                                    i,
                                )?;
                            }
                        }
                        IntrinsicFunction1::UnaryPositive => {
                            fold_const_unaryop(metadata, &mut self[block_idx], i)?;
                        }
                        _ => {}
                    }
                }
                AnyInstruction::Real(Instruction::BinaryOp { .. }) => {
                    fold_const_binop(metadata, &mut self[block_idx], i)?;
                }
                _ => {}
            }

            i += 1;
        }
        apply_static_swaps_block(&mut self[block_idx])?;
        Ok(())
    }

    /// flowgraph.c _PyCfg_ToInstructionSequence
    fn cfg_to_instruction_sequence(
        &mut self,
        instr_sequence: &mut InstructionSequence,
    ) -> crate::InternalResult<()> {
        let mut label_id = 0;
        let mut block_idx = BlockIdx(0);
        while block_idx != BlockIdx::NULL {
            self[block_idx].cpython_label = InstructionSequenceLabel::from_index(label_id);
            label_id += 1;
            block_idx = self[block_idx].next;
        }

        block_idx = BlockIdx(0);
        while block_idx != BlockIdx::NULL {
            let block_label = self[block_idx].cpython_label;
            debug_assert!(is_label(block_label));
            instruction_sequence_use_label(instr_sequence, block_label)?;

            let instr_count = self[block_idx].instruction_used;
            for i in 0..instr_count {
                if self[block_idx].instructions[i].instr.has_target() {
                    let target_block = self[block_idx].instructions[i].target;
                    debug_assert!(target_block != BlockIdx::NULL);
                    let lbl = self[target_block].cpython_label;
                    debug_assert!(is_label(lbl));
                    self[block_idx].instructions[i].arg = OpArg::new(lbl.0 as u32);
                }

                let mut info = self[block_idx].instructions[i];
                info.target = BlockIdx::NULL;
                let except_handler = info.except_handler.take();
                let entry = instruction_sequence_addop(instr_sequence, info)?;
                let hi = &mut entry.except_handler;
                if let Some(handler) = except_handler {
                    debug_assert!(handler.handler_block != BlockIdx::NULL);
                    let lbl = self[handler.handler_block].cpython_label;
                    debug_assert!(is_label(lbl));
                    let start_depth = self[handler.handler_block].start_depth;
                    debug_assert!(start_depth >= 0);
                    hi.h_label = lbl.0;
                    hi.start_depth = start_depth;
                    hi.preserve_lasti = i32::from(handler.preserve_lasti);
                } else {
                    hi.h_label = NO_EXCEPTION_HANDLER_LABEL;
                }
            }
            block_idx = self[block_idx].next;
        }

        instruction_sequence_apply_label_map(instr_sequence)?;
        Ok(())
    }

    fn optimize_load_fast(&mut self) -> crate::InternalResult<()> {
        let mut max_instrs = 0;
        let mut current = BlockIdx(0);
        while current != BlockIdx::NULL {
            max_instrs = max_instrs.max(self[current].instruction_used);
            current = self[current].next;
        }

        let mut instr_flags = Vec::new();
        instr_flags
            .try_reserve_exact(max_instrs)
            .map_err(|_| InternalError::MalformedControlFlowGraph)?;
        instr_flags.resize(max_instrs, 0u8);
        let mut refs = RefStack {
            refs: Vec::new(),
            size: 0,
            capacity: 0,
        };
        let mut worklist = self.make_cfg_traversal_stack()?;
        worklist.push(BlockIdx(0));
        self[0].start_depth = 0;
        self[0].visited = true;
        while let Some(block_idx) = worklist.pop() {
            let instr_count = self[block_idx].instruction_used;
            instr_flags[..instr_count].fill(0);
            debug_assert!(self[block_idx].start_depth >= 0);
            let start_depth = self[block_idx].start_depth as usize;
            ref_stack_clear(&mut refs);
            for _ in 0..start_depth {
                push_ref(&mut refs, DUMMY_INSTR, NOT_LOCAL)?;
            }

            for i in 0..instr_count {
                let info = self[block_idx].instructions[i];
                let instr = info.instr;
                let arg_u32 = u32::from(info.arg);
                debug_assert!(!matches!(instr.real(), Some(Instruction::ExtendedArg)));

                match instr {
                    AnyInstruction::Real(Instruction::DeleteFast { var_num }) => {
                        kill_local(
                            &mut instr_flags,
                            &refs,
                            local_as_ref_local(usize::from(var_num.get(info.arg))),
                        );
                    }
                    AnyInstruction::Real(Instruction::LoadFast { var_num }) => {
                        push_ref(
                            &mut refs,
                            i as isize,
                            local_as_ref_local(usize::from(var_num.get(info.arg))),
                        )?;
                    }
                    AnyInstruction::Real(Instruction::LoadFastAndClear { var_num }) => {
                        let local = local_as_ref_local(usize::from(var_num.get(info.arg)));
                        kill_local(&mut instr_flags, &refs, local);
                        push_ref(&mut refs, i as isize, local)?;
                    }
                    AnyInstruction::Real(Instruction::LoadFastLoadFast { .. }) => {
                        let local1 = (arg_u32 >> 4) as isize;
                        let local2 = (arg_u32 & 15) as isize;
                        push_ref(&mut refs, i as isize, local1)?;
                        push_ref(&mut refs, i as isize, local2)?;
                    }
                    AnyInstruction::Real(Instruction::StoreFast { var_num }) => {
                        let r = ref_stack_pop(&mut refs);
                        store_local(
                            &mut instr_flags,
                            &refs,
                            local_as_ref_local(usize::from(var_num.get(info.arg))),
                            r,
                        );
                    }
                    AnyInstruction::Real(Instruction::StoreFastLoadFast { .. }) => {
                        let r = ref_stack_pop(&mut refs);
                        store_local(&mut instr_flags, &refs, (arg_u32 >> 4) as isize, r);
                        push_ref(&mut refs, i as isize, (arg_u32 & 15) as isize)?;
                    }
                    AnyInstruction::Real(Instruction::StoreFastStoreFast { .. }) => {
                        let r1 = ref_stack_pop(&mut refs);
                        store_local(&mut instr_flags, &refs, (arg_u32 >> 4) as isize, r1);
                        let r2 = ref_stack_pop(&mut refs);
                        store_local(&mut instr_flags, &refs, (arg_u32 & 15) as isize, r2);
                    }
                    AnyInstruction::Real(Instruction::Copy { i: _ }) => {
                        let depth = arg_u32 as usize;
                        assert!(depth > 0);
                        assert!(refs.size >= depth);
                        let r = ref_stack_at(&refs, refs.size - depth);
                        push_ref(&mut refs, r.instr, r.local)?;
                    }
                    AnyInstruction::Real(Instruction::Swap { i: _ }) => {
                        let depth = arg_u32 as usize;
                        assert!(depth >= 2);
                        assert!(refs.size >= depth);
                        ref_stack_swap_top(&mut refs, depth);
                    }
                    AnyInstruction::Real(
                        Instruction::FormatSimple
                        | Instruction::GetAnext
                        | Instruction::GetLen
                        | Instruction::GetYieldFromIter
                        | Instruction::ImportFrom { .. }
                        | Instruction::MatchKeys
                        | Instruction::MatchMapping
                        | Instruction::MatchSequence
                        | Instruction::WithExceptStart,
                    ) => {
                        let effect = instr.stack_effect_info(arg_u32);
                        let net_pushed = effect.pushed() as isize - effect.popped() as isize;
                        debug_assert!(net_pushed >= 0);
                        // CPython optimize_load_fast() shadows the outer
                        // instruction index in this produced-value loop.
                        for produced in 0..net_pushed {
                            push_ref(&mut refs, produced, NOT_LOCAL)?;
                        }
                    }
                    AnyInstruction::Real(
                        Instruction::DictMerge { .. }
                        | Instruction::DictUpdate { .. }
                        | Instruction::ListAppend { .. }
                        | Instruction::ListExtend { .. }
                        | Instruction::MapAdd { .. }
                        | Instruction::Reraise { .. }
                        | Instruction::SetAdd { .. }
                        | Instruction::SetUpdate { .. },
                    ) => {
                        let effect = instr.stack_effect_info(arg_u32);
                        let net_popped = effect.popped() as isize - effect.pushed() as isize;
                        debug_assert!(net_popped > 0);
                        for _ in 0..net_popped {
                            let _ = ref_stack_pop(&mut refs);
                        }
                    }
                    AnyInstruction::Real(
                        Instruction::EndSend | Instruction::SetFunctionAttribute { .. },
                    ) => {
                        let effect = instr.stack_effect_info(arg_u32);
                        debug_assert_eq!(effect.popped(), 2);
                        debug_assert_eq!(effect.pushed(), 1);
                        let tos = ref_stack_pop(&mut refs);
                        let _ = ref_stack_pop(&mut refs);
                        push_ref(&mut refs, tos.instr, tos.local)?;
                    }
                    AnyInstruction::Real(Instruction::CheckExcMatch) => {
                        let _ = ref_stack_pop(&mut refs);
                        push_ref(&mut refs, i as isize, NOT_LOCAL)?;
                    }
                    AnyInstruction::Real(Instruction::ForIter { .. }) => {
                        let target = info.target;
                        debug_assert!(target != BlockIdx::NULL);
                        load_fast_push_block(&mut worklist, self, target, refs.size + 1);
                        push_ref(&mut refs, i as isize, NOT_LOCAL)?;
                    }
                    AnyInstruction::Real(
                        Instruction::LoadAttr { .. } | Instruction::LoadSuperAttr { .. },
                    ) => {
                        let self_ref = ref_stack_pop(&mut refs);
                        if matches!(instr.real(), Some(Instruction::LoadSuperAttr { .. })) {
                            let _ = ref_stack_pop(&mut refs);
                            let _ = ref_stack_pop(&mut refs);
                        }
                        push_ref(&mut refs, i as isize, NOT_LOCAL)?;
                        if arg_u32 & 1 != 0 {
                            push_ref(&mut refs, self_ref.instr, self_ref.local)?;
                        }
                    }
                    AnyInstruction::Real(
                        Instruction::LoadSpecial { .. } | Instruction::PushExcInfo,
                    ) => {
                        let tos = ref_stack_pop(&mut refs);
                        push_ref(&mut refs, i as isize, NOT_LOCAL)?;
                        push_ref(&mut refs, tos.instr, tos.local)?;
                    }
                    AnyInstruction::Real(Instruction::Send { .. }) => {
                        let target = info.target;
                        debug_assert!(target != BlockIdx::NULL);
                        load_fast_push_block(&mut worklist, self, target, refs.size);
                        let _ = ref_stack_pop(&mut refs);
                        push_ref(&mut refs, i as isize, NOT_LOCAL)?;
                    }
                    _ => {
                        let effect = instr.stack_effect_info(arg_u32);
                        let num_popped = effect.popped() as usize;
                        let num_pushed = effect.pushed() as usize;
                        let target = info.target;
                        if instr.has_target() {
                            debug_assert!(target != BlockIdx::NULL);
                            debug_assert!(refs.size >= num_popped);
                            let target_depth = refs.size - num_popped + num_pushed;
                            load_fast_push_block(&mut worklist, self, target, target_depth);
                        }
                        if !is_block_push(&info) {
                            for _ in 0..num_popped {
                                let _ = ref_stack_pop(&mut refs);
                            }
                            for _ in 0..num_pushed {
                                push_ref(&mut refs, i as isize, NOT_LOCAL)?;
                            }
                        }
                    }
                }
            }

            let fallthrough = self[block_idx].next;
            let term = basicblock_last_instr(&self[block_idx]).copied();
            if let Some(term) = term
                && fallthrough != BlockIdx::NULL
                && !term.instr.is_unconditional_jump()
                && !term.instr.is_scope_exit()
            {
                debug_assert!(bb_has_fallthrough(&self[block_idx]));
                load_fast_push_block(&mut worklist, self, fallthrough, refs.size);
            }

            for i in 0..refs.size {
                let r = ref_stack_at(&refs, i);
                if r.instr != DUMMY_INSTR {
                    instr_flags[r.instr as usize] |= LoadFastInstrFlag::RefUnconsumed as u8;
                }
            }

            let block = &mut self[block_idx];
            let iused = block.instruction_used;
            let mut i = 0;
            while i < iused {
                let info = &mut block.instructions[i];
                if instr_flags[i] != 0 {
                    i += 1;
                    continue;
                }

                match info.instr.real_opcode() {
                    Some(Opcode::LoadFast) => {
                        info.instr = Opcode::LoadFastBorrow.into();
                    }
                    Some(Opcode::LoadFastLoadFast) => {
                        info.instr = Opcode::LoadFastBorrowLoadFastBorrow.into();
                    }
                    _ => {}
                }
                i += 1;
            }
        }

        Ok(())
    }

    fn propagate_line_numbers(&mut self) {
        let mut current = BlockIdx(0);
        while current != BlockIdx::NULL {
            let Some(last) = basicblock_last_instr(&self[current]).copied() else {
                current = self[current].next;
                continue;
            };

            let mut prev_location = no_instruction_location();
            for i in 0..self[current].instruction_used {
                if instruction_is_no_location(&self[current].instructions[i]) {
                    instr_set_location(&mut self[current].instructions[i], prev_location);
                } else {
                    prev_location = instr_location(&self[current].instructions[i]);
                }
            }

            let next = self[current].next;
            if bb_has_fallthrough(&self[current]) {
                debug_assert!(next != BlockIdx::NULL);
                if next != BlockIdx::NULL
                    && self[next].predecessors == 1
                    && self[next].instruction_used != 0
                    && instruction_is_no_location(&self[next].instructions[0])
                {
                    instr_set_location(&mut self[next].instructions[0], prev_location);
                }
            }

            if is_jump(&last) {
                let target = last.target;
                debug_assert!(target != BlockIdx::NULL);
                if self[target].predecessors == 1 {
                    let instr = basicblock_raw_first_instr_mut(&mut self[target]);
                    if instruction_is_no_location(instr) {
                        instr_set_location(instr, prev_location);
                    }
                }
            }
            current = self[current].next;
        }
    }

    /// flowgraph.c remove_redundant_nops_and_pairs
    fn remove_redundant_nops_and_pairs(&mut self) -> crate::InternalResult<()> {
        let mut done = false;

        while !done {
            done = true;
            let mut instr: Option<(BlockIdx, usize)> = None;
            let mut block_idx = BlockIdx::new(0);

            while block_idx != BlockIdx::NULL {
                basicblock_remove_redundant_nops(self, block_idx)?;
                if is_label(self[block_idx].cpython_label) {
                    instr = None;
                }

                let len = self[block_idx].instruction_used;
                for instr_idx in 0..len {
                    let prev_instr = instr;
                    instr = Some((block_idx, instr_idx));
                    let instr_info = self[block_idx].instructions[instr_idx];
                    let mut prev_opcode = None;
                    let prev_oparg = if let Some((prev_block, prev_instr_idx)) = prev_instr {
                        let prev_info = self[prev_block].instructions[prev_instr_idx];
                        prev_opcode = prev_info.instr.real_opcode();
                        match prev_info.instr.real() {
                            Some(Instruction::Copy { i }) => i.get(prev_info.arg),
                            _ => u32::from(prev_info.arg),
                        }
                    } else {
                        0
                    };

                    let opcode = instr_info.instr.real_opcode();
                    let is_redundant_pair = matches!(opcode, Some(Opcode::PopTop))
                        && (matches!(prev_opcode, Some(Opcode::LoadConst | Opcode::LoadSmallInt))
                            || (prev_oparg == 1 && matches!(prev_opcode, Some(Opcode::Copy))));

                    if is_redundant_pair {
                        let (prev_block, prev_instr_idx) =
                            prev_instr.expect("redundant pair has previous");
                        set_to_nop(&mut self[prev_block].instructions[prev_instr_idx]);
                        set_to_nop(&mut self[block_idx].instructions[instr_idx]);
                        done = false;
                    }
                }

                let instr_is_jump = instr.is_some_and(|(instr_block, instr_idx)| {
                    is_jump(&self[instr_block].instructions[instr_idx])
                });

                let block = &self[block_idx];
                if instr_is_jump || !bb_has_fallthrough(block) {
                    instr = None;
                }
                block_idx = block.next;
            }
        }
        Ok(())
    }

    /// flowgraph.c calculate_stackdepth
    fn calculate_stackdepth(&mut self) -> crate::InternalResult<u32> {
        let mut current = BlockIdx(0);
        while current != BlockIdx::NULL {
            self[current.idx()].start_depth = START_DEPTH_UNSET;
            current = self[current.idx()].next;
        }
        let mut stack = self.make_cfg_traversal_stack()?;
        let mut maxdepth = 0i32;
        stackdepth_push(&mut stack, self, BlockIdx(0), 0)?;
        while let Some(block_idx) = stack.pop() {
            let mut depth = self[block_idx].start_depth;
            debug_assert!(depth >= 0);
            let mut next = self[block_idx].next;
            let instr_count = self[block_idx].instruction_used;
            for i in 0..instr_count {
                let ins = self[block_idx].instructions[i];
                let instr = &ins.instr;
                let effects = get_stack_effects(*instr, ins.arg, 0)?;
                let new_depth = depth + effects.net;
                if new_depth < 0 {
                    return Err(InternalError::StackUnderflow);
                }
                maxdepth = maxdepth.max(depth);
                if instr.has_target() && !matches!(instr.real(), Some(Instruction::EndAsyncFor)) {
                    debug_assert!(ins.target != BlockIdx::NULL);
                    let effects = get_stack_effects(*instr, ins.arg, 1)?;
                    let target_depth = depth + effects.net;
                    debug_assert!(target_depth >= 0);
                    maxdepth = maxdepth.max(depth);
                    stackdepth_push(&mut stack, self, ins.target, target_depth)?;
                }
                depth = new_depth;
                debug_assert!(!instr.is_assembler());
                if instr.is_unconditional_jump() || instr.is_scope_exit() {
                    next = BlockIdx::NULL;
                    break;
                }
            }

            if next != BlockIdx::NULL {
                debug_assert!(bb_has_fallthrough(&self[block_idx]));
                stackdepth_push(&mut stack, self, next, depth)?;
            }
        }

        let stackdepth = maxdepth;
        Ok(stackdepth as u32)
    }

    /// flowgraph.c make_cfg_traversal_stack
    fn make_cfg_traversal_stack(&mut self) -> crate::InternalResult<CfgTraversalStack> {
        debug_assert!(!self.is_empty());

        let mut nblocks = 0;
        let mut current = BlockIdx(0);
        while current != BlockIdx::NULL {
            self[current].visited = false;
            nblocks += 1;
            current = self[current].next;
        }
        debug_assert!(nblocks > 0);
        let mut stack = Vec::new();
        stack
            .try_reserve_exact(nblocks)
            .map_err(|_| InternalError::MalformedControlFlowGraph)?;
        stack.resize(nblocks, BlockIdx::NULL);
        let stack = CfgTraversalStack { stack, sp: 0 };
        debug_assert_eq!(stack.capacity(), nblocks);
        Ok(stack)
    }

    /// flowgraph.c normalize_jumps
    fn normalize_jumps(&mut self) -> crate::InternalResult<()> {
        let mut current = BlockIdx(0);
        while current != BlockIdx::NULL {
            self[current].visited = false;
            current = self[current].next;
        }

        let mut current = BlockIdx(0);
        while current != BlockIdx::NULL {
            self[current].visited = true;
            normalize_jumps_in_block(self, current)?;
            current = self[current].next;
        }

        Ok(())
    }

    /// flowgraph.c remove_unused_consts
    #[allow(clippy::needless_range_loop)]
    fn remove_unused_consts(&mut self, consts: &mut ConstantPool) -> crate::InternalResult<()> {
        let nconsts = consts.len();
        if nconsts == 0 {
            return Ok(());
        }

        let mut index_map = Vec::new();
        index_map
            .try_reserve_exact(nconsts)
            .map_err(|_| InternalError::MalformedControlFlowGraph)?;
        index_map.resize(nconsts, 0isize);
        for i in 1..nconsts {
            index_map[i] = -1;
        }
        // The first constant may be docstring; keep it always.
        index_map[0] = 0;

        // Mark used consts.
        let mut block_idx = BlockIdx(0);
        while block_idx != BlockIdx::NULL {
            let block = &self[block_idx];
            for i in 0..block.instruction_used {
                let instr = &block.instructions[i];
                if instr.instr.has_const() {
                    let index = u32::from(instr.arg) as usize;
                    debug_assert!(index < nconsts);
                    index_map[index] = index as isize;
                }
            }
            block_idx = block.next;
        }

        // Now index_map[i] == i if consts[i] is used, -1 otherwise.
        // Condense consts.
        let mut n_used_consts = 0;
        for i in 0..nconsts {
            if index_map[i] != -1 {
                debug_assert_eq!(index_map[i], i as isize);
                index_map[n_used_consts] = index_map[i];
                n_used_consts += 1;
            }
        }

        if n_used_consts == nconsts {
            return Ok(());
        }

        // Move all used consts to the beginning of the consts list.
        debug_assert!(n_used_consts < nconsts);
        for i in 0..n_used_consts {
            let old_index = index_map[i] as usize;
            debug_assert!(i <= old_index && old_index < nconsts);
            if i != old_index {
                let value = consts.constants[old_index].clone();
                consts.constants[i] = value;
            }
        }

        // Truncate the consts list at its new size.
        consts.constants.truncate(n_used_consts);

        // Adjust const indices in the bytecode.
        let mut reverse_index_map = Vec::new();
        reverse_index_map
            .try_reserve_exact(nconsts)
            .map_err(|_| InternalError::MalformedControlFlowGraph)?;
        reverse_index_map.resize(nconsts, 0isize);
        for i in 0..nconsts {
            reverse_index_map[i] = -1;
        }
        for i in 0..n_used_consts {
            let old_index = index_map[i];
            debug_assert!(old_index != -1);
            let old_index = old_index as usize;
            debug_assert_eq!(reverse_index_map[old_index], -1);
            reverse_index_map[old_index] = i as isize;
        }

        block_idx = BlockIdx(0);
        while block_idx != BlockIdx::NULL {
            let next_block = self[block_idx.idx()].next;
            let block = &mut self[block_idx];
            for i in 0..block.instruction_used {
                let instr = &mut block.instructions[i];
                if instr.instr.has_const() {
                    let index = u32::from(instr.arg) as usize;
                    debug_assert!(reverse_index_map[index] >= 0);
                    debug_assert!(reverse_index_map[index] < n_used_consts as isize);
                    instr.arg = OpArg::new(reverse_index_map[index] as u32);
                }
            }
            block_idx = next_block;
        }
        Ok(())
    }

    /// flowgraph.c insert_superinstructions
    fn insert_superinstructions(&mut self) -> crate::InternalResult<usize> {
        let mut block_idx = BlockIdx(0);
        while block_idx != BlockIdx::NULL {
            let next_block = self[block_idx].next;
            let block = &mut self[block_idx];
            for i in 0..block.instruction_used {
                let nextop = (i + 1 < block.instruction_used)
                    .then(|| block.instructions[i + 1].instr.real_opcode())
                    .flatten();

                match (block.instructions[i].instr.real_opcode(), nextop) {
                    (Some(Opcode::LoadFast), _) => {
                        if matches!(nextop, Some(Opcode::LoadFast)) {
                            let (inst1, rest) = block.instructions[i..].split_at_mut(1);
                            make_super_instruction(
                                &mut inst1[0],
                                &mut rest[0],
                                Opcode::LoadFastLoadFast.into(),
                            );
                        }
                    }

                    (Some(Opcode::StoreFast), Some(Opcode::LoadFast)) => {
                        let (inst1, rest) = block.instructions[i..].split_at_mut(1);
                        make_super_instruction(
                            &mut inst1[0],
                            &mut rest[0],
                            Opcode::StoreFastLoadFast.into(),
                        );
                    }

                    (Some(Opcode::StoreFast), Some(Opcode::StoreFast)) => {
                        let (inst1, rest) = block.instructions[i..].split_at_mut(1);
                        make_super_instruction(
                            &mut inst1[0],
                            &mut rest[0],
                            Opcode::StoreFastStoreFast.into(),
                        );
                    }

                    (_, _) => {}
                }
            }

            block_idx = next_block;
        }

        let res = remove_redundant_nops(self)?;

        #[cfg(debug_assertions)]
        assert!(no_redundant_nops(self));

        Ok(res)
    }
}

impl From<Vec<Block>> for Blocks {
    fn from(value: Vec<Block>) -> Self {
        Self(value)
    }
}

impl From<Box<[Block]>> for Blocks {
    fn from(value: Box<[Block]>) -> Self {
        Self(value.into())
    }
}

impl From<&[Block]> for Blocks {
    fn from(value: &[Block]) -> Self {
        Self(value.to_vec())
    }
}

impl From<&mut [Block]> for Blocks {
    fn from(value: &mut [Block]) -> Self {
        Self(value.to_vec())
    }
}

impl<const N: usize> From<[Block; N]> for Blocks {
    fn from(value: [Block; N]) -> Self {
        Self(value.into())
    }
}

impl<const N: usize> From<&[Block; N]> for Blocks {
    fn from(value: &[Block; N]) -> Self {
        Self(value.to_vec())
    }
}

impl Deref for Blocks {
    type Target = [Block];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Blocks {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Index<usize> for Blocks {
    type Output = Block;

    fn index(&self, idx: usize) -> &Self::Output {
        &self.0[idx]
    }
}

impl IndexMut<usize> for Blocks {
    fn index_mut(&mut self, idx: usize) -> &mut Self::Output {
        &mut self.0[idx]
    }
}

impl Index<BlockIdx> for Blocks {
    type Output = Block;

    fn index(&self, block_idx: BlockIdx) -> &Self::Output {
        &self.0[block_idx.as_usize()]
    }
}

impl IndexMut<BlockIdx> for Blocks {
    fn index_mut(&mut self, block_idx: BlockIdx) -> &mut Self::Output {
        &mut self.0[block_idx.as_usize()]
    }
}

pub(crate) const START_DEPTH_UNSET: i32 = i32::MIN;
const CO_MAXBLOCKS: usize = 20;

/// flowgraph.c struct _PyCfgExceptStack
#[derive(Clone, Debug)]
struct CfgExceptStack {
    handlers: [BlockIdx; CO_MAXBLOCKS + 2],
    depth: usize,
}

/// flowgraph.c `basicblock **stack`
#[derive(Clone, Debug)]
struct CfgTraversalStack {
    stack: Vec<BlockIdx>,
    sp: usize,
}

impl CfgTraversalStack {
    fn push(&mut self, block: BlockIdx) {
        debug_assert!(self.sp < self.stack.len());
        self.stack[self.sp] = block;
        self.sp += 1;
    }

    fn pop(&mut self) -> Option<BlockIdx> {
        if self.sp == 0 {
            return None;
        }
        self.sp -= 1;
        Some(self.stack[self.sp])
    }

    fn capacity(&self) -> usize {
        self.stack.len()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct InstructionSequenceLabelMap {
    block_labels: Vec<InstructionSequenceLabel>,
    /// Codegen-side shadow of CPython's instruction-sequence label map.
    ///
    /// `_PyInstructionSequence_UseLabel()` can map multiple labels to the same
    /// instruction offset before `_PyCfg_FromInstructionSequence()` materializes
    /// CFG blocks. The codegen CFG path keeps the same aliasing by resolving
    /// those labels to the block that owns the shared offset.
    cpython_block_by_label: Vec<BlockIdx>,
}

fn instruction_sequence_label_map_register_label(
    map: &mut InstructionSequenceLabelMap,
    label: InstructionSequenceLabel,
) -> crate::InternalResult<()> {
    debug_assert!(is_label(label));
    let old_size = map.cpython_block_by_label.len();
    let new_allocation = c_array_ensure_capacity::<i32>(
        old_size,
        label.idx(),
        INITIAL_INSTR_SEQUENCE_LABELS_MAP_SIZE,
    )?;
    if new_allocation > old_size {
        if new_allocation > map.cpython_block_by_label.capacity() {
            map.cpython_block_by_label
                .try_reserve_exact(new_allocation - map.cpython_block_by_label.capacity())
                .map_err(|_| InternalError::MalformedControlFlowGraph)?;
        }
        map.cpython_block_by_label
            .resize(new_allocation, BlockIdx::NULL);
        for i in old_size..map.cpython_block_by_label.len() {
            map.cpython_block_by_label[i] = BlockIdx::NULL;
        }
    }
    debug_assert!(map.cpython_block_by_label.len() > label.idx());
    Ok(())
}

fn instruction_sequence_label_map_ensure_label_for_block(
    map: &mut InstructionSequenceLabelMap,
    seq: &mut InstructionSequence,
    block: BlockIdx,
) -> crate::InternalResult<InstructionSequenceLabel> {
    debug_assert_ne!(block, BlockIdx::NULL);
    let block_label = map.block_labels[block.idx()];
    if is_label(block_label) {
        return Ok(block_label);
    }
    let label = instruction_sequence_new_label(seq);
    debug_assert_eq!(label.0, seq.next_free_label);
    instruction_sequence_label_map_register_label(map, label)?;
    map.cpython_block_by_label[label.idx()] = block;
    map.block_labels[block.idx()] = label;
    Ok(label)
}

fn instruction_sequence_label_map_label_for_block(
    map: &InstructionSequenceLabelMap,
    block: BlockIdx,
) -> InstructionSequenceLabel {
    debug_assert_ne!(block, BlockIdx::NULL);
    map.block_labels
        .get(block.idx())
        .copied()
        .unwrap_or(InstructionSequenceLabel::NO_LABEL)
}

fn instruction_sequence_label_map_block_for_label(
    map: &InstructionSequenceLabelMap,
    label: InstructionSequenceLabel,
) -> Option<BlockIdx> {
    if !is_label(label) {
        return None;
    }
    map.cpython_block_by_label
        .get(label.idx())
        .copied()
        .filter(|&block| block != BlockIdx::NULL)
}

fn instruction_sequence_label_map_resolve_label(
    map: &InstructionSequenceLabelMap,
    block: BlockIdx,
) -> BlockIdx {
    if block == BlockIdx::NULL {
        return BlockIdx::NULL;
    }
    let label = instruction_sequence_label_map_label_for_block(map, block);
    if !is_label(label) {
        return block;
    }
    instruction_sequence_label_map_block_for_label(map, label).unwrap_or_else(|| {
        debug_assert!(
            false,
            "CPython instruction-sequence label must map to a codegen CFG block"
        );
        BlockIdx::NULL
    })
}

fn instruction_sequence_label_map_resolve_label_to_block(
    map: &InstructionSequenceLabelMap,
    label: InstructionSequenceLabel,
) -> BlockIdx {
    if !is_label(label) {
        return BlockIdx::NULL;
    }
    instruction_sequence_label_map_block_for_label(map, label).unwrap_or_else(|| {
        debug_assert!(
            false,
            "CPython instruction-sequence label must map to a codegen CFG block"
        );
        BlockIdx::NULL
    })
}

fn instruction_sequence_label_oparg(label: InstructionSequenceLabel) -> OpArg {
    debug_assert!(is_label(label));
    OpArg::new(label.idx() as u32)
}

fn instruction_sequence_label_map_use_label_at_block(
    map: &mut InstructionSequenceLabelMap,
    seq: &mut InstructionSequence,
    from: BlockIdx,
    to: BlockIdx,
) -> crate::InternalResult<()> {
    if from == BlockIdx::NULL || from == to {
        return Ok(());
    }
    let from_label = instruction_sequence_label_map_ensure_label_for_block(map, seq, from)?;
    debug_assert!(map.cpython_block_by_label.len() > from_label.idx());
    let to_block = instruction_sequence_label_map_resolve_label(map, to);
    if to_block == BlockIdx::NULL {
        debug_assert!(
            false,
            "CPython label target must map to a codegen CFG block"
        );
        return Ok(());
    }
    map.cpython_block_by_label[from_label.idx()] = to_block;
    Ok(())
}

fn instruction_sequence_label_map_push_unlabeled_block(
    map: &mut InstructionSequenceLabelMap,
) -> crate::InternalResult<()> {
    map.block_labels
        .try_reserve(1)
        .map_err(|_| InternalError::MalformedControlFlowGraph)?;
    map.block_labels.push(InstructionSequenceLabel::NO_LABEL);
    Ok(())
}

fn instruction_sequence_label_map_push_unmapped_label(
    map: &mut InstructionSequenceLabelMap,
    seq: &mut InstructionSequence,
) -> crate::InternalResult<()> {
    let label = instruction_sequence_new_label(seq);
    debug_assert_eq!(label.0, seq.next_free_label);
    instruction_sequence_label_map_register_label(map, label)?;
    let block = BlockIdx(
        map.block_labels
            .len()
            .to_u32()
            .ok_or(InternalError::MalformedControlFlowGraph)?,
    );
    map.cpython_block_by_label[label.idx()] = block;
    map.block_labels
        .try_reserve(1)
        .map_err(|_| InternalError::MalformedControlFlowGraph)?;
    map.block_labels.push(label);
    Ok(())
}

impl InstructionSequenceLabelMap {
    pub(crate) fn new() -> Self {
        Self {
            block_labels: vec![InstructionSequenceLabel::NO_LABEL],
            cpython_block_by_label: Vec::new(),
        }
    }
}

pub struct CodeInfo {
    pub flags: CodeFlags,
    pub source_path: String,
    pub private: Option<String>, // For private name mangling, mostly for class

    pub blocks: Blocks,
    pub current_block: BlockIdx,
    pub(crate) instr_sequence: InstructionSequence,
    pub(crate) instr_sequence_label_map: InstructionSequenceLabelMap,
    pub(crate) annotations_instr_sequence: Option<InstructionSequence>,

    pub metadata: CodeUnitMetadata,

    // For class scopes: attributes accessed via self.X
    pub static_attributes: Option<IndexSet<String>>,

    // True if compiling an inlined comprehension
    pub in_inlined_comp: bool,

    // Block stack for tracking nested control structures
    pub fblock: Vec<crate::compile::FBlockInfo>,

    // Reference to the symbol table for this scope
    pub symbol_table_index: usize,
    // CPython compile.c uses PyList_GET_SIZE(u->u_ste->ste_varnames)
    // when calling flowgraph.c _PyCfg_OptimizeCodeUnit().
    pub nparams: usize,

    // PEP 649: Track nesting depth inside conditional blocks (if/for/while/etc.)
    // u_in_conditional_block
    pub in_conditional_block: u32,

    // PEP 649: Next index for conditional annotation tracking
    // u_next_conditional_annotation_index
    pub next_conditional_annotation_index: u32,
}

impl CodeInfo {
    pub(crate) fn addop_to_instr_sequence(
        &mut self,
        mut info: InstructionInfo,
    ) -> crate::InternalResult<()> {
        if info.instr.has_target() && info.target != BlockIdx::NULL {
            let label = instruction_sequence_label_map_ensure_label_for_block(
                &mut self.instr_sequence_label_map,
                &mut self.instr_sequence,
                info.target,
            )?;
            info.arg = instruction_sequence_label_oparg(label);
            info.target = BlockIdx::NULL;
        }
        instruction_sequence_addop(&mut self.instr_sequence, info)?;
        Ok(())
    }

    pub(crate) fn addop_to_instr_sequence_with_target_label(
        &mut self,
        mut info: InstructionInfo,
        target_label: InstructionSequenceLabel,
    ) -> crate::InternalResult<()> {
        if !info.instr.has_target() {
            return Err(InternalError::MalformedControlFlowGraph);
        }
        info.arg = instruction_sequence_label_oparg(target_label);
        info.target = BlockIdx::NULL;
        instruction_sequence_addop(&mut self.instr_sequence, info)?;
        Ok(())
    }

    pub(crate) fn addop_to_current_block(
        &mut self,
        info: InstructionInfo,
    ) -> crate::InternalResult<()> {
        basicblock_addop(&mut self.blocks[self.current_block.idx()], info)
    }

    pub(crate) fn last_current_block_instr_mut(&mut self) -> Option<&mut InstructionInfo> {
        basicblock_last_instr_mut(&mut self.blocks[self.current_block.idx()])
    }

    pub(crate) fn set_last_instr_sequence_lineno_override(&mut self, lineno_override: i32) {
        if let Some(last) = instruction_sequence_last_info_mut(&mut self.instr_sequence) {
            last.lineno_override = Some(lineno_override);
        }
    }

    pub(crate) fn use_instr_sequence_label(
        &mut self,
        block: BlockIdx,
    ) -> crate::InternalResult<()> {
        let label = instruction_sequence_label_map_ensure_label_for_block(
            &mut self.instr_sequence_label_map,
            &mut self.instr_sequence,
            block,
        )?;
        instruction_sequence_use_label(&mut self.instr_sequence, label)
    }

    pub(crate) fn new_instr_sequence_label(&mut self) -> InstructionSequenceLabel {
        instruction_sequence_new_label(&mut self.instr_sequence)
    }

    pub(crate) fn use_raw_instr_sequence_label(
        &mut self,
        label: InstructionSequenceLabel,
    ) -> crate::InternalResult<()> {
        instruction_sequence_use_label(&mut self.instr_sequence, label)
    }

    pub(crate) fn mark_cpython_cfg_label(&mut self, block: BlockIdx) -> crate::InternalResult<()> {
        let label = instruction_sequence_label_map_ensure_label_for_block(
            &mut self.instr_sequence_label_map,
            &mut self.instr_sequence,
            block,
        )?;
        self.blocks[block.idx()].cpython_label = label;
        Ok(())
    }

    pub(crate) fn resolve_instr_sequence_label(&self, block: BlockIdx) -> BlockIdx {
        instruction_sequence_label_map_resolve_label(&self.instr_sequence_label_map, block)
    }

    pub(crate) fn block_for_instr_sequence_label(
        &self,
        label: InstructionSequenceLabel,
    ) -> BlockIdx {
        instruction_sequence_label_map_resolve_label_to_block(&self.instr_sequence_label_map, label)
    }

    pub(crate) fn use_instr_sequence_label_at_block(
        &mut self,
        from: BlockIdx,
        to: BlockIdx,
    ) -> crate::InternalResult<()> {
        instruction_sequence_label_map_use_label_at_block(
            &mut self.instr_sequence_label_map,
            &mut self.instr_sequence,
            from,
            to,
        )
    }

    pub(crate) fn instr_sequence_label_for_block(
        &mut self,
        block: BlockIdx,
    ) -> crate::InternalResult<InstructionSequenceLabel> {
        if block == BlockIdx::NULL {
            Ok(InstructionSequenceLabel::NO_LABEL)
        } else {
            instruction_sequence_label_map_ensure_label_for_block(
                &mut self.instr_sequence_label_map,
                &mut self.instr_sequence,
                block,
            )
        }
    }

    pub(crate) fn insert_start_setup_cleanup(
        &mut self,
        handler_block: BlockIdx,
    ) -> crate::InternalResult<()> {
        let handler_label = instruction_sequence_label_map_ensure_label_for_block(
            &mut self.instr_sequence_label_map,
            &mut self.instr_sequence,
            handler_block,
        )?;
        instruction_sequence_insert_instruction(
            &mut self.instr_sequence,
            0,
            InstructionInfo {
                instr: PseudoOpcode::SetupCleanup.into(),
                arg: instruction_sequence_label_oparg(handler_label),
                target: BlockIdx::NULL,
                location: SourceLocation::default(),
                end_location: SourceLocation::default(),
                except_handler: None,
                lineno_override: Some(NO_LOCATION_OVERRIDE),
            },
        )
    }

    pub(crate) fn push_unmapped_instr_sequence_label(&mut self) -> crate::InternalResult<()> {
        instruction_sequence_label_map_push_unmapped_label(
            &mut self.instr_sequence_label_map,
            &mut self.instr_sequence,
        )
    }

    pub(crate) fn push_unlabeled_instr_sequence_block(&mut self) -> crate::InternalResult<()> {
        instruction_sequence_label_map_push_unlabeled_block(&mut self.instr_sequence_label_map)
    }

    fn take_recorded_instr_sequence(&mut self) -> crate::InternalResult<InstructionSequence> {
        let mut instr_sequence =
            core::mem::replace(&mut self.instr_sequence, instruction_sequence_new());
        if let Some(mut annotations_instr_sequence) = self.annotations_instr_sequence.take() {
            instruction_sequence_apply_label_map(&mut annotations_instr_sequence)?;
            instruction_sequence_set_annotations_code(
                &mut instr_sequence,
                Some(Box::new(annotations_instr_sequence)),
            );
        }
        Ok(instr_sequence)
    }

    fn prepare_cfg_from_codegen(&mut self) -> crate::InternalResult<InstructionSequence> {
        // CPython compile.c optimize_and_assemble_code_unit passes
        // u_instr_sequence directly into flowgraph.c _PyCfg_FromInstructionSequence().
        self.take_recorded_instr_sequence()
    }
}

fn optimize_code_unit(
    metadata: &mut CodeUnitMetadata,
    blocks: &mut Blocks,
    instr_sequence: InstructionSequence,
    nlocals: usize,
    nparams: usize,
) -> crate::InternalResult<()> {
    // Phase 1: _PyCfg_OptimizeCodeUnit (flowgraph.c)
    *blocks = cfg_from_instruction_sequence(instr_sequence)?;
    translate_jump_labels_to_targets(blocks)?;
    mark_except_handlers(blocks)?;
    label_exception_targets(blocks)?;
    optimize_cfg(metadata, blocks, metadata.firstlineno)?;
    blocks.remove_unused_consts(&mut metadata.consts)?;
    add_checks_for_loads_of_uninitialized_variables(blocks, nlocals, nparams)?;
    // CPython inserts superinstructions in _PyCfg_OptimizeCodeUnit, before
    // later jump normalization / block reordering can create adjacencies
    // that never exist at this stage in flowgraph.c.
    blocks.insert_superinstructions()?;
    push_cold_blocks_to_end(blocks)?;
    // CPython resolves line numbers again after cold-block extraction.
    blocks.resolve_line_numbers(metadata.firstlineno)?;
    Ok(())
}

fn optimize_cfg(
    metadata: &mut CodeUnitMetadata,
    blocks: &mut Blocks,
    firstlineno: OneIndexed,
) -> crate::InternalResult<()> {
    // flowgraph.c optimize_cfg
    // CPython optimize_cfg() starts with check_cfg() and raises
    // SystemError if a jump or scope exit is not the last instruction in
    // its block.
    check_cfg(blocks)?;
    inline_small_or_no_lineno_blocks(blocks)?;
    // CPython does not re-run instruction-sequence label-map/CFG conversion
    // after this point. Unreferenced label blocks left by jump inlining
    // remain block boundaries and can preserve line-marker NOPs.
    blocks.remove_unreachable()?;
    // CPython optimize_cfg resolves line numbers before local checks and
    // superinstruction insertion, so fusion decisions see propagated
    // source locations.
    blocks.resolve_line_numbers(firstlineno)?;
    // CPython optimize_cfg() runs optimize_load_const() and then
    // optimize_basic_block() after line numbers are resolved.
    optimize_load_const(metadata, blocks)?;
    let mut block_idx = BlockIdx(0);
    while block_idx != BlockIdx::NULL {
        let next_block = blocks[block_idx].next;
        blocks.optimize_basic_block(metadata, block_idx)?;
        block_idx = next_block;
    }
    blocks.remove_redundant_nops_and_pairs()?;
    // CPython optimize_cfg() removes newly-unreachable blocks and
    // redundant NOP/jump chains before _PyCfg_OptimizeCodeUnit() prunes
    // unused constants.
    blocks.remove_unreachable()?;
    remove_redundant_nops_and_jumps(blocks)?;
    #[cfg(debug_assertions)]
    assert!(no_redundant_jumps(blocks));
    Ok(())
}

fn optimized_cfg_to_instruction_sequence(
    metadata: &CodeUnitMetadata,
    flags: CodeFlags,
    blocks: &mut Blocks,
) -> crate::InternalResult<(u32, usize, InstructionSequence)> {
    // Phase 2: _PyCfg_OptimizedCfgToInstructionSequence (flowgraph.c)
    convert_pseudo_conditional_jumps(blocks)?;
    let max_stackdepth = blocks.calculate_stackdepth()?;
    debug_assert!(!is_generator(flags) || max_stackdepth != 0);
    let nlocalsplus = prepare_localsplus(metadata, blocks, flags)?;
    // Match CPython order: pseudo ops are lowered after stackdepth and
    // localsplus preparation, before normalize_jumps.
    convert_pseudo_ops(blocks)?;
    blocks.normalize_jumps()?;
    #[cfg(debug_assertions)]
    assert!(no_redundant_jumps(blocks));
    // optimize_load_fast: after normalize_jumps
    blocks.optimize_load_fast()?;

    let mut instr_sequence = instruction_sequence_new();
    blocks.cfg_to_instruction_sequence(&mut instr_sequence)?;
    Ok((max_stackdepth, nlocalsplus, instr_sequence))
}

impl CodeInfo {
    pub fn finalize_code(
        mut self,
        opts: &crate::compile::CompileOpts,
    ) -> crate::InternalResult<CodeObject> {
        let instr_sequence = self.prepare_cfg_from_codegen()?;
        let nlocals = self.metadata.varnames.len();
        let nparams = self.nparams;
        optimize_code_unit(
            &mut self.metadata,
            &mut self.blocks,
            instr_sequence,
            nlocals,
            nparams,
        )?;
        let (max_stackdepth, nlocalsplus, mut instr_sequence) =
            optimized_cfg_to_instruction_sequence(&self.metadata, self.flags, &mut self.blocks)?;
        let localsplusinfo = compute_localsplus_info(&self.metadata, nlocalsplus, self.flags)?;

        let Self {
            flags,
            source_path,
            private: _, // private is only used during compilation

            blocks: _,
            current_block: _,
            instr_sequence: _,
            instr_sequence_label_map: _,
            annotations_instr_sequence: _,
            metadata,
            static_attributes: _,
            in_inlined_comp: _,
            fblock: _,
            symbol_table_index: _,
            nparams: _,
            in_conditional_block: _,
            next_conditional_annotation_index: _,
        } = self;

        let CodeUnitMetadata {
            name: obj_name,
            qualname,
            consts: constants,
            names: name_cache,
            varnames: varname_cache,
            cellvars: _,
            freevars: freevar_cache,
            fast_hidden: _,
            fast_hidden_final: _,
            argcount: arg_count,
            posonlyargcount: posonlyarg_count,
            kwonlyargcount: kwonlyarg_count,
            firstlineno: first_line_number,
        } = metadata;

        resolve_unconditional_jumps(&mut instr_sequence)?;
        resolve_jump_offsets(&mut instr_sequence)?;
        let assembled = assemble_emit(
            &mut instr_sequence,
            first_line_number.get() as i32,
            opts.debug_ranges,
        )?;
        let locations = rustpython_compiler_core::marshal::linetable_to_locations(
            &assembled.linetable,
            first_line_number.get() as i32,
            assembled.instructions.len(),
        );

        Ok(CodeObject {
            flags,
            posonlyarg_count,
            arg_count,
            kwonlyarg_count,
            source_path,
            first_line_number: Some(first_line_number),
            obj_name: obj_name.clone(),
            qualname: qualname.unwrap_or(obj_name),

            max_stackdepth,
            instructions: CodeUnits::from(assembled.instructions),
            locations,
            constants: constants.into_iter().collect(),
            names: name_cache.into_iter().collect(),
            varnames: varname_cache.into_iter().collect(),
            cellvars: localsplusinfo.cellvars,
            freevars: freevar_cache.into_iter().collect(),
            localspluskinds: localsplusinfo.kinds,
            linetable: assembled.linetable,
            exceptiontable: assembled.exceptiontable,
        })
    }
}

/// flowgraph.c IS_GENERATOR
fn is_generator(flags: CodeFlags) -> bool {
    flags.intersects(CodeFlags::GENERATOR | CodeFlags::COROUTINE | CodeFlags::ASYNC_GENERATOR)
}

/// flowgraph.c insert_prefix_instructions
fn insert_prefix_instructions(
    metadata: &CodeUnitMetadata,
    blocks: &mut Blocks,
    cellfixedoffsets: &[i32],
    nfreevars: usize,
    flags: CodeFlags,
) -> crate::InternalResult<()> {
    debug_assert!(!blocks.is_empty());
    let entry = &mut blocks[0];
    let ncellvars = metadata.cellvars.len();
    let firstlineno = metadata.firstlineno;
    debug_assert!(firstlineno.get() > 0);

    if is_generator(flags) {
        let location = SourceLocation {
            line: firstlineno,
            character_offset: OneIndexed::MIN,
        };
        basicblock_insert_instruction(
            entry,
            0,
            InstructionInfo {
                instr: Instruction::ReturnGenerator.into(),
                arg: OpArg::new(0),
                target: BlockIdx::NULL,
                location,
                end_location: location,
                except_handler: None,
                lineno_override: Some(LINE_ONLY_LOCATION_OVERRIDE),
            },
        )?;
        basicblock_insert_instruction(
            entry,
            1,
            InstructionInfo {
                instr: Instruction::PopTop.into(),
                arg: OpArg::new(0),
                target: BlockIdx::NULL,
                location,
                end_location: location,
                except_handler: None,
                lineno_override: Some(LINE_ONLY_LOCATION_OVERRIDE),
            },
        )?;
    }

    if ncellvars > 0 {
        let nvars = metadata.varnames.len() + ncellvars;
        let mut sorted = Vec::new();
        vec_try_reserve_exact(&mut sorted, nvars)?;
        sorted.resize(nvars, 0i32);
        for i in 0..ncellvars {
            sorted[cellfixedoffsets[i] as usize] = i as i32 + 1;
        }
        let mut ncellsused = 0;
        let mut i = 0;
        while ncellsused < ncellvars {
            let oldindex = sorted[i] - 1;
            i += 1;
            if oldindex == -1 {
                continue;
            }
            basicblock_insert_instruction(
                entry,
                ncellsused,
                InstructionInfo {
                    instr: Opcode::MakeCell.into(),
                    arg: OpArg::new(oldindex as u32),
                    target: BlockIdx::NULL,
                    location: SourceLocation::default(),
                    end_location: SourceLocation::default(),
                    except_handler: None,
                    lineno_override: Some(NO_LOCATION_OVERRIDE),
                },
            )?;
            ncellsused += 1;
        }
    }

    if nfreevars > 0 {
        basicblock_insert_instruction(
            entry,
            0,
            InstructionInfo {
                instr: Opcode::CopyFreeVars.into(),
                arg: OpArg::new(nfreevars as u32),
                target: BlockIdx::NULL,
                location: SourceLocation::default(),
                end_location: SourceLocation::default(),
                except_handler: None,
                lineno_override: Some(NO_LOCATION_OVERRIDE),
            },
        )?;
    }
    Ok(())
}

/// flowgraph.c prepare_localsplus
fn prepare_localsplus(
    metadata: &CodeUnitMetadata,
    blocks: &mut Blocks,
    flags: CodeFlags,
) -> crate::InternalResult<usize> {
    let nlocals = metadata.varnames.len();
    let ncellvars = metadata.cellvars.len();
    let nfreevars = metadata.freevars.len();
    let int_max = i32::MAX as usize;
    debug_assert!(nlocals < int_max);
    debug_assert!(ncellvars < int_max);
    debug_assert!(nfreevars < int_max);
    debug_assert!(int_max - nlocals - ncellvars > 0);
    debug_assert!(int_max - nlocals - ncellvars - nfreevars > 0);
    let mut nlocalsplus = nlocals + ncellvars + nfreevars;
    let mut cellfixedoffsets = build_cellfixedoffsets(metadata)?;

    // This must be called before fix_cell_offsets().
    insert_prefix_instructions(metadata, blocks, &cellfixedoffsets, nfreevars, flags)?;

    let numdropped = fix_cell_offsets(metadata, blocks, &mut cellfixedoffsets);
    nlocalsplus -= numdropped;
    Ok(nlocalsplus)
}

/// flowgraph.c eval_const_unaryop
fn eval_const_unaryop(
    operand: &ConstantData,
    op: Instruction,
    intrinsic: Option<oparg::IntrinsicFunction1>,
) -> Option<ConstantData> {
    match (operand, op, intrinsic) {
        (ConstantData::Integer { value }, Instruction::UnaryNegative, None) => {
            Some(ConstantData::Integer { value: -value })
        }
        (ConstantData::Float { value }, Instruction::UnaryNegative, None) => {
            Some(ConstantData::Float { value: -value })
        }
        (ConstantData::Complex { value }, Instruction::UnaryNegative, None) => {
            Some(ConstantData::Complex { value: -value })
        }
        (ConstantData::Boolean { value }, Instruction::UnaryNegative, None) => {
            Some(ConstantData::Integer {
                value: BigInt::from(-i32::from(*value)),
            })
        }
        (ConstantData::Integer { value }, Instruction::UnaryInvert, None) => {
            Some(ConstantData::Integer { value: !value })
        }
        (ConstantData::Boolean { .. }, Instruction::UnaryInvert, None) => None,
        (_, Instruction::UnaryNot, None) => Some(ConstantData::Boolean {
            value: !operand.truthiness(),
        }),
        (
            ConstantData::Integer { value },
            Instruction::CallIntrinsic1 { .. },
            Some(oparg::IntrinsicFunction1::UnaryPositive),
        ) => Some(ConstantData::Integer {
            value: value.clone(),
        }),
        (
            ConstantData::Float { value },
            Instruction::CallIntrinsic1 { .. },
            Some(oparg::IntrinsicFunction1::UnaryPositive),
        ) => Some(ConstantData::Float { value: *value }),
        (
            ConstantData::Boolean { value },
            Instruction::CallIntrinsic1 { .. },
            Some(oparg::IntrinsicFunction1::UnaryPositive),
        ) => Some(ConstantData::Integer {
            value: BigInt::from(i32::from(*value)),
        }),
        (
            ConstantData::Complex { value },
            Instruction::CallIntrinsic1 { .. },
            Some(oparg::IntrinsicFunction1::UnaryPositive),
        ) => Some(ConstantData::Complex { value: *value }),
        _ => None,
    }
}

fn load_const_truthiness(
    instr: Instruction,
    arg: OpArg,
    metadata: &CodeUnitMetadata,
) -> Option<bool> {
    match instr {
        Instruction::LoadConst { consti } => {
            let constant = &metadata.consts[consti.get(arg).as_usize()];
            Some(constant.truthiness())
        }
        Instruction::LoadSmallInt { i } => Some(i.get(arg) != 0),
        _ => None,
    }
}

/// flowgraph.c add_const
fn add_const(
    metadata: &mut CodeUnitMetadata,
    constant: ConstantData,
) -> crate::InternalResult<usize> {
    Ok(metadata.consts.try_insert_full(constant)?.0)
}

fn instr_make_load_const(
    metadata: &mut CodeUnitMetadata,
    instr: &mut InstructionInfo,
    constant: ConstantData,
) -> crate::InternalResult<()> {
    if maybe_instr_make_load_smallint(instr, &constant) {
        return Ok(());
    }

    let const_idx = add_const(metadata, constant)?;
    instr_set_op1(
        instr,
        Opcode::LoadConst.into(),
        OpArg::new(const_idx as u32),
    );
    Ok(())
}

/// flowgraph.c fold_const_unaryop
fn fold_const_unaryop(
    metadata: &mut CodeUnitMetadata,
    block: &mut Block,
    i: usize,
) -> crate::InternalResult<bool> {
    let instr = &block.instructions[i];
    let (op, intrinsic) = match instr.instr.real() {
        Some(Instruction::UnaryNegative) => (Instruction::UnaryNegative, None),
        Some(Instruction::UnaryInvert) => (Instruction::UnaryInvert, None),
        Some(Instruction::UnaryNot) => (Instruction::UnaryNot, None),
        Some(Instruction::CallIntrinsic1 { func })
            if matches!(
                func.get(instr.arg),
                oparg::IntrinsicFunction1::UnaryPositive
            ) =>
        {
            (Opcode::CallIntrinsic1.into(), Some(func.get(instr.arg)))
        }
        _ => return Ok(false),
    };
    let Some(operand_index) = (if let Some(start) = i.checked_sub(1) {
        get_const_loading_instrs(block, start, 1)?
    } else {
        None
    })
    .and_then(|indices| indices.into_iter().next()) else {
        return Ok(false);
    };
    let operand = get_const_value(metadata, &block.instructions[operand_index]);
    let Some(operand) = operand else {
        return Ok(false);
    };
    let Some(folded_const) = eval_const_unaryop(&operand, op, intrinsic) else {
        return Ok(false);
    };
    nop_out(block, &[operand_index]);
    instr_make_load_const(metadata, &mut block.instructions[i], folded_const)?;
    Ok(true)
}

/// flowgraph.c get_const_loading_instrs
fn get_const_loading_instrs(
    block: &Block,
    mut start: usize,
    size: usize,
) -> crate::InternalResult<Option<Vec<usize>>> {
    let mut indices = Vec::new();
    indices
        .try_reserve_exact(size)
        .map_err(|_| InternalError::MalformedControlFlowGraph)?;
    loop {
        if start >= block.instruction_used {
            return Ok(None);
        }
        let instr = &block.instructions[start];
        if !matches!(instr.instr.real(), Some(Instruction::Nop)) {
            if !loads_const(instr) {
                return Ok(None);
            }
            indices.push(start);
            if indices.len() == size {
                break;
            }
        }
        let Some(prev) = start.checked_sub(1) else {
            return Ok(None);
        };
        start = prev;
    }
    indices.reverse();
    Ok(Some(indices))
}

/// flowgraph.c nop_out
fn nop_out(block: &mut Block, instrs: &[usize]) {
    for &i in instrs {
        nop_out_no_location(&mut block.instructions[i]);
    }
}

/// flowgraph.c fold_const_binop
fn fold_const_binop(
    metadata: &mut CodeUnitMetadata,
    block: &mut Block,
    i: usize,
) -> crate::InternalResult<bool> {
    use oparg::BinaryOperator as BinOp;

    let Some(Opcode::BinaryOp) = block.instructions[i].instr.real_opcode() else {
        return Ok(false);
    };

    let Some(operand_indices) = (if let Some(start) = i.checked_sub(1) {
        get_const_loading_instrs(block, start, 2)?
    } else {
        None
    }) else {
        return Ok(false);
    };

    let op_raw = u32::from(block.instructions[i].arg);
    let Ok(op) = BinOp::try_from(op_raw) else {
        return Ok(false);
    };

    let left = get_const_value(metadata, &block.instructions[operand_indices[0]]);
    let right = get_const_value(metadata, &block.instructions[operand_indices[1]]);
    let (Some(left_val), Some(right_val)) = (left, right) else {
        return Ok(false);
    };

    let Some(result_const) = eval_const_binop(&left_val, &right_val, op) else {
        return Ok(false);
    };

    nop_out(block, &operand_indices);
    instr_make_load_const(metadata, &mut block.instructions[i], result_const)?;
    Ok(true)
}

/// flowgraph.c loads_const
fn loads_const(info: &InstructionInfo) -> bool {
    info.instr.has_const() || matches!(info.instr.real_opcode(), Some(Opcode::LoadSmallInt))
}

/// flowgraph.c get_const_value
fn get_const_value(metadata: &CodeUnitMetadata, info: &InstructionInfo) -> Option<ConstantData> {
    match info.instr.real_opcode() {
        Some(Opcode::LoadSmallInt) => {
            let v = u32::from(info.arg) as i32;
            Some(ConstantData::Integer {
                value: BigInt::from(v),
            })
        }
        _ if info.instr.has_const() => {
            let idx = u32::from(info.arg) as usize;
            metadata.consts.get_index(idx).cloned()
        }
        _ => None,
    }
}

/// flowgraph.c const_folding_check_complexity
fn const_folding_check_complexity(obj: &ConstantData, mut limit: isize) -> Option<isize> {
    if let ConstantData::Tuple { elements } = obj {
        limit -= isize::try_from(elements.len()).ok()?;
        if limit < 0 {
            return None;
        }
        for element in elements {
            limit = const_folding_check_complexity(element, limit)?;
        }
    }
    Some(limit)
}

fn repeat_wtf8(value: &Wtf8Buf, n: usize) -> Option<Wtf8Buf> {
    let mut result = Wtf8Buf::new();
    result.try_reserve_exact(value.len().checked_mul(n)?).ok()?;
    for _ in 0..n {
        result.push_wtf8(value);
    }
    Some(result)
}

fn checked_repeat_count(n: &BigInt, item_size: usize) -> Option<usize> {
    let n = n.to_isize()?;
    if item_size != 0 && (n < 0 || n as usize > MAX_STR_SIZE / item_size) {
        return None;
    }
    Some(n.max(0) as usize)
}

/// flowgraph.c const_folding_safe_multiply
fn const_folding_safe_multiply(left: &ConstantData, right: &ConstantData) -> Option<ConstantData> {
    match (left, right) {
        (ConstantData::Integer { value: l }, ConstantData::Integer { value: r }) => {
            if !l.is_zero() && !r.is_zero() && l.bits() + r.bits() > MAX_INT_SIZE {
                return None;
            }
            Some(ConstantData::Integer { value: l * r })
        }
        (ConstantData::Float { value: l }, ConstantData::Float { value: r }) => {
            Some(ConstantData::Float { value: l * r })
        }
        (ConstantData::Str { value: s }, ConstantData::Integer { value: n }) => {
            let n = checked_repeat_count(n, s.code_points().count())?;
            Some(ConstantData::Str {
                value: repeat_wtf8(s, n)?,
            })
        }
        (ConstantData::Integer { .. }, ConstantData::Str { .. }) => {
            const_folding_safe_multiply(right, left)
        }
        (ConstantData::Bytes { value: b }, ConstantData::Integer { value: n }) => {
            let n = checked_repeat_count(n, b.len())?;
            let mut value = Vec::new();
            value.try_reserve_exact(b.len().checked_mul(n)?).ok()?;
            for _ in 0..n {
                value.extend_from_slice(b);
            }
            Some(ConstantData::Bytes { value })
        }
        (ConstantData::Integer { .. }, ConstantData::Bytes { .. }) => {
            const_folding_safe_multiply(right, left)
        }
        (ConstantData::Tuple { elements }, ConstantData::Integer { value: n }) => {
            let n = n.to_usize()?;
            if n != 0 && !elements.is_empty() {
                if n > MAX_COLLECTION_SIZE / elements.len() {
                    return None;
                }
                const_folding_check_complexity(
                    &ConstantData::Tuple {
                        elements: elements.clone(),
                    },
                    MAX_TOTAL_ITEMS / isize::try_from(n).ok()?,
                )?;
            }
            let mut result = Vec::new();
            result
                .try_reserve_exact(elements.len().checked_mul(n)?)
                .ok()?;
            for _ in 0..n {
                result.extend(elements.iter().cloned());
            }
            Some(ConstantData::Tuple { elements: result })
        }
        (ConstantData::Integer { .. }, ConstantData::Tuple { .. }) => {
            const_folding_safe_multiply(right, left)
        }
        _ => None,
    }
}

/// flowgraph.c const_folding_safe_power
fn const_folding_safe_power(left: &ConstantData, right: &ConstantData) -> Option<ConstantData> {
    match (left, right) {
        (ConstantData::Integer { value: l }, ConstantData::Integer { value: r }) => {
            if r < &BigInt::from(0) {
                if l.is_zero() {
                    return None;
                }
                let base = l.to_f64()?;
                if !base.is_finite() {
                    return None;
                }
                let result = if let Some(exp) = r.to_i32() {
                    base.powi(exp)
                } else {
                    base.powf(r.to_f64()?)
                };
                if !result.is_finite() {
                    return None;
                }
                return Some(ConstantData::Float { value: result });
            }
            let exp: u64 = r.try_into().ok()?;
            let exp_usize = usize::try_from(exp).ok()?;
            if !l.is_zero() && exp > 0 && l.bits() > MAX_INT_SIZE / exp {
                return None;
            }
            Some(ConstantData::Integer {
                value: num_traits::pow::pow(l.clone(), exp_usize),
            })
        }
        (ConstantData::Float { value: l }, ConstantData::Float { value: r }) => {
            let result = l.powf(*r);
            result
                .is_finite()
                .then_some(ConstantData::Float { value: result })
        }
        _ => None,
    }
}

/// flowgraph.c const_folding_safe_lshift
fn const_folding_safe_lshift(left: &ConstantData, right: &ConstantData) -> Option<ConstantData> {
    let (ConstantData::Integer { value: l }, ConstantData::Integer { value: r }) = (left, right)
    else {
        return None;
    };
    let shift: u64 = r.try_into().ok()?;
    let shift_usize = usize::try_from(shift).ok()?;
    if shift > MAX_INT_SIZE || (!l.is_zero() && l.bits() > MAX_INT_SIZE - shift) {
        return None;
    }
    Some(ConstantData::Integer {
        value: l << shift_usize,
    })
}

/// flowgraph.c const_folding_safe_mod
fn const_folding_safe_mod(left: &ConstantData, right: &ConstantData) -> Option<ConstantData> {
    if matches!(left, ConstantData::Str { .. } | ConstantData::Bytes { .. }) {
        return None;
    }

    match (left, right) {
        (ConstantData::Integer { value: l }, ConstantData::Integer { value: r }) => {
            if r.is_zero() {
                return None;
            }
            let rem = l.clone() % r.clone();
            let value = if !rem.is_zero() && (rem < BigInt::from(0)) != (*r < BigInt::from(0)) {
                rem + r
            } else {
                rem
            };
            Some(ConstantData::Integer { value })
        }
        (ConstantData::Float { value: l }, ConstantData::Float { value: r }) => {
            let (_, modulo) = float_div_mod(*l, *r)?;
            Some(ConstantData::Float { value: modulo })
        }
        _ => None,
    }
}

fn float_div_mod(left: f64, right: f64) -> Option<(f64, f64)> {
    if right == 0.0 {
        return None;
    }

    let mut modulo = left % right;
    let div = (left - modulo) / right;
    let floordiv = if modulo != 0.0 {
        let div = if (right < 0.0) != (modulo < 0.0) {
            modulo += right;
            div - 1.0
        } else {
            div
        };
        let mut floordiv = div.floor();
        if div - floordiv > 0.5 {
            floordiv += 1.0;
        }
        floordiv
    } else {
        modulo = 0.0f64.copysign(right);
        0.0f64.copysign(left / right)
    };

    Some((floordiv, modulo))
}

/// flowgraph.c eval_const_binop complex result construction
fn eval_const_complex_const(value: Complex<f64>) -> Option<ConstantData> {
    (value.re.is_finite() && value.im.is_finite()).then_some(ConstantData::Complex { value })
}

/// flowgraph.c eval_const_binop complex operations
fn eval_const_complex_binop(
    left: Complex<f64>,
    right: Complex<f64>,
    op: oparg::BinaryOperator,
) -> Option<ConstantData> {
    use oparg::BinaryOperator as BinOp;

    let value = match op {
        BinOp::Add => left + right,
        BinOp::Subtract => {
            let re = left.re - right.re;
            // Preserve CPython's signed-zero behavior for real-zero
            // minus zero-complex expressions such as `0 - 0j`.
            let im = if left.re == 0.0
                && left.im == 0.0
                && right.re == 0.0
                && right.im == 0.0
                && !right.im.is_sign_negative()
            {
                -0.0
            } else {
                left.im - right.im
            };
            Complex::new(re, im)
        }
        BinOp::Multiply => left * right,
        BinOp::TrueDivide => {
            if right == Complex::new(0.0, 0.0) {
                return None;
            }
            left / right
        }
        BinOp::Power => {
            if left == Complex::new(0.0, 0.0) {
                if right.im != 0.0 || right.re < 0.0 {
                    return None;
                }

                return eval_const_complex_const(if right.re == 0.0 {
                    Complex::new(1.0, 0.0)
                } else {
                    Complex::new(0.0, 0.0)
                });
            }

            if right.im == 0.0
                && right.re.fract() == 0.0
                && right.re >= f64::from(i32::MIN)
                && right.re <= f64::from(i32::MAX)
            {
                left.powi(right.re as i32)
            } else {
                left.powc(right)
            }
        }
        _ => return None,
    };
    eval_const_complex_const(value)
}

/// flowgraph.c eval_const_binop subscript index conversion
fn constant_as_index(value: &ConstantData) -> Option<i64> {
    match value {
        ConstantData::Integer { value } => value.to_i64().or_else(|| {
            if value < &BigInt::from(0) {
                Some(i64::MIN)
            } else {
                Some(i64::MAX)
            }
        }),
        ConstantData::Boolean { value } => Some(i64::from(*value)),
        _ => None,
    }
}

/// flowgraph.c eval_const_binop subscript slice bound conversion
fn slice_bound(value: &ConstantData) -> Option<Option<i64>> {
    match value {
        ConstantData::None => Some(None),
        _ => constant_as_index(value).map(Some),
    }
}

/// flowgraph.c eval_const_binop subscript slice index adjustment
fn adjusted_slice_indices(len: usize, slice: &[ConstantData; 3]) -> Option<Vec<usize>> {
    let len = i64::try_from(len).ok()?;
    let start = slice_bound(&slice[0])?;
    let stop = slice_bound(&slice[1])?;
    let step = slice_bound(&slice[2])?.unwrap_or(1);
    if step == 0 || step == i64::MIN {
        return None;
    }

    let step_is_negative = step < 0;
    let lower = if step_is_negative { -1 } else { 0 };
    let upper = if step_is_negative { len - 1 } else { len };
    let adjust = |value: Option<i64>, default: i64| {
        let mut value = value.unwrap_or(default);
        if value < 0 {
            value = value.saturating_add(len);
            if value < 0 {
                value = lower;
            }
        } else if value >= len {
            value = upper;
        }
        value
    };
    let start = adjust(start, if step_is_negative { upper } else { lower });
    let stop = adjust(stop, if step_is_negative { lower } else { upper });

    let mut index = i128::from(start);
    let stop = i128::from(stop);
    let step = i128::from(step);
    let slice_len = if step > 0 {
        if index < stop {
            usize::try_from((stop - index - 1) / step + 1).ok()?
        } else {
            0
        }
    } else if index > stop {
        usize::try_from((index - stop - 1) / -step + 1).ok()?
    } else {
        0
    };
    let mut indices = Vec::new();
    indices.try_reserve_exact(slice_len).ok()?;
    if step > 0 {
        while index < stop {
            indices.push(usize::try_from(index).ok()?);
            index += step;
        }
    } else {
        while index > stop {
            indices.push(usize::try_from(index).ok()?);
            index += step;
        }
    }
    Some(indices)
}

/// flowgraph.c eval_const_binop subscript index adjustment
fn adjusted_const_index(len: usize, index: &ConstantData) -> Option<usize> {
    let len = i64::try_from(len).ok()?;
    let index = constant_as_index(index)?;
    let index = if index < 0 {
        index.saturating_add(len)
    } else {
        index
    };
    if index < 0 || index >= len {
        return None;
    }
    usize::try_from(index).ok()
}

/// flowgraph.c eval_const_binop NB_SUBSCR
fn eval_const_subscript(container: &ConstantData, index: &ConstantData) -> Option<ConstantData> {
    match (container, index) {
        (
            ConstantData::Str { value },
            ConstantData::Integer { .. } | ConstantData::Boolean { .. },
        ) => {
            let string = value.to_string();
            if string.contains(char::REPLACEMENT_CHARACTER) {
                return None;
            }
            let mut chars = Vec::new();
            chars.try_reserve_exact(string.chars().count()).ok()?;
            chars.extend(string.chars());
            let index = adjusted_const_index(chars.len(), index)?;
            Some(ConstantData::Str {
                value: chars[index].to_string().into(),
            })
        }
        (ConstantData::Str { value }, ConstantData::Slice { elements }) => {
            let string = value.to_string();
            if string.contains(char::REPLACEMENT_CHARACTER) {
                return None;
            }
            let mut chars = Vec::new();
            chars.try_reserve_exact(string.chars().count()).ok()?;
            chars.extend(string.chars());
            let indices = adjusted_slice_indices(chars.len(), elements)?;
            let capacity = indices.iter().try_fold(0usize, |capacity, &index| {
                capacity.checked_add(chars[index].len_utf8())
            })?;
            let mut result = String::new();
            result.try_reserve_exact(capacity).ok()?;
            for index in indices {
                result.push(chars[index]);
            }
            Some(ConstantData::Str {
                value: result.into(),
            })
        }
        (
            ConstantData::Bytes { value },
            ConstantData::Integer { .. } | ConstantData::Boolean { .. },
        ) => {
            let index = adjusted_const_index(value.len(), index)?;
            Some(ConstantData::Integer {
                value: BigInt::from(value[index]),
            })
        }
        (ConstantData::Bytes { value }, ConstantData::Slice { elements }) => {
            let indices = adjusted_slice_indices(value.len(), elements)?;
            let mut result = Vec::new();
            result.try_reserve_exact(indices.len()).ok()?;
            for index in indices {
                result.push(value[index]);
            }
            Some(ConstantData::Bytes { value: result })
        }
        (
            ConstantData::Tuple { elements },
            ConstantData::Integer { .. } | ConstantData::Boolean { .. },
        ) => {
            let index = adjusted_const_index(elements.len(), index)?;
            Some(elements[index].clone())
        }
        (ConstantData::Tuple { elements }, ConstantData::Slice { elements: slice }) => {
            let indices = adjusted_slice_indices(elements.len(), slice)?;
            let mut result = Vec::new();
            result.try_reserve_exact(indices.len()).ok()?;
            for index in indices {
                result.push(elements[index].clone());
            }
            Some(ConstantData::Tuple { elements: result })
        }
        _ => None,
    }
}

/// flowgraph.c eval_const_binop bool/int coercion
fn constant_as_int(value: &ConstantData) -> Option<(BigInt, bool)> {
    match value {
        ConstantData::Boolean { value } => Some((BigInt::from(u8::from(*value)), true)),
        ConstantData::Integer { value } => Some((value.clone(), false)),
        _ => None,
    }
}

/// flowgraph.c eval_const_binop
fn eval_const_binop(
    left: &ConstantData,
    right: &ConstantData,
    op: oparg::BinaryOperator,
) -> Option<ConstantData> {
    use oparg::BinaryOperator as BinOp;

    if matches!(op, BinOp::Subscr) {
        return eval_const_subscript(left, right);
    }

    if let (Some((left_int, left_is_bool)), Some((right_int, right_is_bool))) =
        (constant_as_int(left), constant_as_int(right))
        && (left_is_bool || right_is_bool)
    {
        if left_is_bool && right_is_bool {
            match op {
                BinOp::And => {
                    return Some(ConstantData::Boolean {
                        value: !left_int.is_zero() & !right_int.is_zero(),
                    });
                }
                BinOp::Or => {
                    return Some(ConstantData::Boolean {
                        value: !left_int.is_zero() | !right_int.is_zero(),
                    });
                }
                BinOp::Xor => {
                    return Some(ConstantData::Boolean {
                        value: !left_int.is_zero() ^ !right_int.is_zero(),
                    });
                }
                _ => {}
            }
        }

        return eval_const_binop(
            &ConstantData::Integer { value: left_int },
            &ConstantData::Integer { value: right_int },
            op,
        );
    }

    match (left, right) {
        (ConstantData::Integer { value: l }, ConstantData::Integer { value: r }) => {
            let result = match op {
                BinOp::Add => l + r,
                BinOp::Subtract => l - r,
                BinOp::Multiply => {
                    return const_folding_safe_multiply(left, right);
                }
                BinOp::TrueDivide => {
                    if r.is_zero() {
                        return None;
                    }
                    let l_f = l.to_f64()?;
                    let r_f = r.to_f64()?;
                    let result = l_f / r_f;
                    if !result.is_finite() {
                        return None;
                    }
                    return Some(ConstantData::Float { value: result });
                }
                BinOp::FloorDivide => {
                    if r.is_zero() {
                        return None;
                    }
                    // Python floor division: round towards negative infinity
                    let (q, rem) = (l.clone() / r.clone(), l.clone() % r.clone());
                    if !rem.is_zero() && (rem < BigInt::from(0)) != (*r < BigInt::from(0)) {
                        q - 1
                    } else {
                        q
                    }
                }
                BinOp::Remainder => return const_folding_safe_mod(left, right),
                BinOp::Power => return const_folding_safe_power(left, right),
                BinOp::Lshift => return const_folding_safe_lshift(left, right),
                BinOp::Rshift => {
                    let shift: u32 = r.try_into().ok()?;
                    l >> (shift as usize)
                }
                BinOp::And => l & r,
                BinOp::Or => l | r,
                BinOp::Xor => l ^ r,
                _ => return None,
            };
            Some(ConstantData::Integer { value: result })
        }
        (ConstantData::Float { value: l }, ConstantData::Float { value: r }) => {
            let result = match op {
                BinOp::Add => l + r,
                BinOp::Subtract => l - r,
                BinOp::Multiply => return const_folding_safe_multiply(left, right),
                BinOp::TrueDivide => {
                    if *r == 0.0 {
                        return None;
                    }
                    l / r
                }
                BinOp::FloorDivide => {
                    let (floordiv, _) = float_div_mod(*l, *r)?;
                    floordiv
                }
                BinOp::Remainder => return const_folding_safe_mod(left, right),
                BinOp::Power => return const_folding_safe_power(left, right),
                _ => return None,
            };
            if matches!(op, BinOp::Power) && !result.is_finite() {
                return None;
            }
            Some(ConstantData::Float { value: result })
        }
        // Int op Float or Float op Int → Float
        (ConstantData::Integer { value: l }, ConstantData::Float { value: r }) => {
            let l_f = l.to_f64()?;
            eval_const_binop(
                &ConstantData::Float { value: l_f },
                &ConstantData::Float { value: *r },
                op,
            )
        }
        (ConstantData::Float { value: l }, ConstantData::Integer { value: r }) => {
            let r_f = r.to_f64()?;
            eval_const_binop(
                &ConstantData::Float { value: *l },
                &ConstantData::Float { value: r_f },
                op,
            )
        }
        (ConstantData::Integer { value: l }, ConstantData::Complex { value: r }) => {
            eval_const_complex_binop(Complex::new(l.to_f64()?, 0.0), *r, op)
        }
        (ConstantData::Complex { value: l }, ConstantData::Integer { value: r }) => {
            eval_const_complex_binop(*l, Complex::new(r.to_f64()?, 0.0), op)
        }
        (ConstantData::Float { value: l }, ConstantData::Complex { value: r }) => {
            eval_const_complex_binop(Complex::new(*l, 0.0), *r, op)
        }
        (ConstantData::Complex { value: l }, ConstantData::Float { value: r }) => {
            eval_const_complex_binop(*l, Complex::new(*r, 0.0), op)
        }
        (ConstantData::Complex { value: l }, ConstantData::Complex { value: r }) => {
            eval_const_complex_binop(*l, *r, op)
        }
        // String concatenation and repetition
        (ConstantData::Str { value: l }, ConstantData::Str { value: r })
            if matches!(op, BinOp::Add) =>
        {
            let mut result = Wtf8Buf::new();
            result
                .try_reserve_exact(l.len().checked_add(r.len())?)
                .ok()?;
            result.push_wtf8(l);
            result.push_wtf8(r);
            Some(ConstantData::Str { value: result })
        }
        (ConstantData::Str { .. }, ConstantData::Integer { .. })
            if matches!(op, BinOp::Multiply) =>
        {
            const_folding_safe_multiply(left, right)
        }
        (ConstantData::Tuple { elements: l }, ConstantData::Tuple { elements: r })
            if matches!(op, BinOp::Add) =>
        {
            let mut result = Vec::new();
            result
                .try_reserve_exact(l.len().checked_add(r.len())?)
                .ok()?;
            result.extend(l.iter().cloned());
            result.extend(r.iter().cloned());
            Some(ConstantData::Tuple { elements: result })
        }
        (ConstantData::Tuple { .. }, ConstantData::Integer { .. })
            if matches!(op, BinOp::Multiply) =>
        {
            const_folding_safe_multiply(left, right)
        }
        (ConstantData::Integer { .. }, ConstantData::Tuple { .. })
            if matches!(op, BinOp::Multiply) =>
        {
            const_folding_safe_multiply(left, right)
        }
        (ConstantData::Integer { .. }, ConstantData::Str { .. })
            if matches!(op, BinOp::Multiply) =>
        {
            const_folding_safe_multiply(left, right)
        }
        (ConstantData::Bytes { value: l }, ConstantData::Bytes { value: r })
            if matches!(op, BinOp::Add) =>
        {
            let mut result = Vec::new();
            result
                .try_reserve_exact(l.len().checked_add(r.len())?)
                .ok()?;
            result.extend_from_slice(l);
            result.extend_from_slice(r);
            Some(ConstantData::Bytes { value: result })
        }
        (ConstantData::Bytes { .. }, ConstantData::Integer { .. })
            if matches!(op, BinOp::Multiply) =>
        {
            const_folding_safe_multiply(left, right)
        }
        (ConstantData::Integer { .. }, ConstantData::Bytes { .. })
            if matches!(op, BinOp::Multiply) =>
        {
            const_folding_safe_multiply(left, right)
        }
        _ => None,
    }
}

/// flowgraph.c fold_tuple_of_constants
fn fold_tuple_of_constants(
    metadata: &mut CodeUnitMetadata,
    block: &mut Block,
    i: usize,
) -> crate::InternalResult<bool> {
    let Some(Opcode::BuildTuple) = block.instructions[i].instr.real_opcode() else {
        return Ok(false);
    };

    let tuple_size = u32::from(block.instructions[i].arg) as usize;
    if tuple_size > STACK_USE_GUIDELINE {
        return Ok(false);
    }

    let Some(operand_indices) = (if tuple_size == 0 {
        Some(Vec::new())
    } else if let Some(start) = i.checked_sub(1) {
        get_const_loading_instrs(block, start, tuple_size)?
    } else {
        None
    }) else {
        return Ok(false);
    };

    let mut elements = Vec::new();
    elements
        .try_reserve_exact(tuple_size)
        .map_err(|_| InternalError::MalformedControlFlowGraph)?;
    for &j in &operand_indices {
        let Some(element) = get_const_value(metadata, &block.instructions[j]) else {
            return Ok(false);
        };
        elements.push(element);
    }

    nop_out(block, &operand_indices);
    instr_make_load_const(
        metadata,
        &mut block.instructions[i],
        ConstantData::Tuple { elements },
    )?;
    Ok(true)
}

fn fold_constant_intrinsic_list_to_tuple(
    metadata: &mut CodeUnitMetadata,
    block: &mut Block,
    i: usize,
) -> crate::InternalResult<bool> {
    let Some(Instruction::CallIntrinsic1 { func }) = block.instructions[i].instr.real() else {
        return Ok(false);
    };
    if func.get(block.instructions[i].arg) != IntrinsicFunction1::ListToTuple {
        return Ok(false);
    }

    let mut consts_found = 0usize;
    let mut expect_append = true;
    let mut pos = i;
    while let Some(prev) = pos.checked_sub(1) {
        pos = prev;
        let instr = &block.instructions[pos];
        if matches!(instr.instr.real(), Some(Instruction::Nop)) {
            continue;
        }

        if matches!(instr.instr.real(), Some(Instruction::BuildList { .. }))
            && u32::from(instr.arg) == 0
        {
            if !expect_append {
                return Ok(false);
            }

            let mut elements = Vec::new();
            elements
                .try_reserve_exact(consts_found)
                .map_err(|_| InternalError::MalformedControlFlowGraph)?;
            for idx in (pos..i).rev() {
                if matches!(block.instructions[idx].instr.real(), Some(Instruction::Nop)) {
                    continue;
                }
                if loads_const(&block.instructions[idx]) {
                    let Some(value) = get_const_value(metadata, &block.instructions[idx]) else {
                        return Ok(false);
                    };
                    elements.push(value);
                }
                nop_out_no_location(&mut block.instructions[idx]);
            }
            debug_assert_eq!(elements.len(), consts_found);
            elements.reverse();
            instr_make_load_const(
                metadata,
                &mut block.instructions[i],
                ConstantData::Tuple { elements },
            )?;
            return Ok(true);
        }

        if expect_append {
            if !matches!(instr.instr.real(), Some(Instruction::ListAppend { .. }))
                || u32::from(instr.arg) != 1
            {
                return Ok(false);
            }
        } else {
            if !loads_const(instr) {
                return Ok(false);
            }
            consts_found += 1;
        }
        expect_append = !expect_append;
    }

    Ok(false)
}

/// Port of CPython's flowgraph.c optimize_lists_and_sets().
fn optimize_lists_and_sets(
    metadata: &mut CodeUnitMetadata,
    block: &mut Block,
    i: usize,
    nextop: Option<Instruction>,
) -> crate::InternalResult<bool> {
    let Some(instr) = block.instructions[i].instr.real() else {
        return Ok(false);
    };
    let is_list = matches!(instr, Instruction::BuildList { .. });
    let is_set = matches!(instr, Instruction::BuildSet { .. });
    if !is_list && !is_set {
        return Ok(false);
    }

    let contains_or_iter = matches!(
        nextop,
        Some(Instruction::GetIter | Instruction::ContainsOp { .. })
    );
    let seq_size = u32::from(block.instructions[i].arg) as usize;
    if seq_size > STACK_USE_GUIDELINE || (seq_size < MIN_CONST_SEQUENCE_SIZE && !contains_or_iter) {
        return Ok(false);
    }

    let Some(operand_indices) = (if seq_size == 0 {
        Some(Vec::new())
    } else if let Some(start) = i.checked_sub(1) {
        get_const_loading_instrs(block, start, seq_size)?
    } else {
        None
    }) else {
        if contains_or_iter && is_list {
            let arg = block.instructions[i].arg;
            instr_set_op1(&mut block.instructions[i], Opcode::BuildTuple.into(), arg);
            return Ok(true);
        }
        return Ok(false);
    };

    let mut elements = Vec::new();
    elements
        .try_reserve_exact(seq_size)
        .map_err(|_| InternalError::MalformedControlFlowGraph)?;
    for &j in &operand_indices {
        let Some(element) = get_const_value(metadata, &block.instructions[j]) else {
            return Ok(false);
        };
        elements.push(element);
    }

    let const_data = if is_list {
        ConstantData::Tuple { elements }
    } else {
        ConstantData::Frozenset { elements }
    };
    let const_idx = add_const(metadata, const_data)?;

    if !contains_or_iter {
        debug_assert!(i >= 2);
        let folded_loc = block.instructions[i].location;
        let end_loc = block.instructions[i].end_location;

        nop_out(block, &operand_indices);

        let build_instr = if is_list {
            Opcode::BuildList
        } else {
            Opcode::BuildSet
        }
        .into();
        instr_set_op1(&mut block.instructions[i - 2], build_instr, OpArg::new(0));
        block.instructions[i - 2].location = folded_loc;
        block.instructions[i - 2].end_location = end_loc;
        block.instructions[i - 2].lineno_override = None;

        instr_set_op1(
            &mut block.instructions[i - 1],
            Opcode::LoadConst.into(),
            OpArg::new(const_idx as u32),
        );

        let extend_instr = if is_list {
            Opcode::ListExtend
        } else {
            Opcode::SetUpdate
        };
        instr_set_op1(
            &mut block.instructions[i],
            extend_instr.into(),
            OpArg::new(1),
        );
        return Ok(true);
    }

    nop_out(block, &operand_indices);

    instr_set_op1(
        &mut block.instructions[i],
        Opcode::LoadConst.into(),
        OpArg::new(const_idx as u32),
    );
    Ok(true)
}

/// flowgraph.c VISITED
const VISITED: i32 = -1;

/// flowgraph.c SWAPPABLE
fn is_swappable(instr: AnyInstruction) -> bool {
    matches!(
        instr.into(),
        AnyOpcode::Real(Opcode::StoreFast | Opcode::PopTop)
            | AnyOpcode::Pseudo(PseudoOpcode::StoreFastMaybeNull)
    )
}

/// flowgraph.c STORES_TO
fn stores_to(info: &InstructionInfo) -> i32 {
    match info.instr.into() {
        AnyOpcode::Real(Opcode::StoreFast)
        | AnyOpcode::Pseudo(PseudoOpcode::StoreFastMaybeNull) => u32::from(info.arg) as i32,
        _ => -1,
    }
}

/// flowgraph.c next_swappable_instruction
fn next_swappable_instruction(block: &Block, mut i: usize, lineno: i32) -> Option<usize> {
    loop {
        i += 1;
        if i >= block.instruction_used {
            return None;
        }

        let info = &block.instructions[i];
        let info_lineno = instruction_lineno(info);

        if lineno >= 0 && info_lineno != lineno {
            return None;
        }

        if matches!(info.instr, AnyInstruction::Real(Instruction::Nop)) {
            continue;
        }

        if is_swappable(info.instr) {
            return Some(i);
        }

        return None;
    }
}

/// flowgraph.c swaptimize
fn swaptimize(block: &mut Block, ix: &mut usize) -> crate::InternalResult<()> {
    debug_assert!(matches!(
        block.instructions[*ix].instr.real_opcode(),
        Some(Opcode::Swap)
    ));
    let mut depth = u32::from(block.instructions[*ix].arg) as usize;
    let mut len = 1usize;
    let mut more = false;
    let limit = block.instruction_used - *ix;
    while len < limit {
        match block.instructions[*ix + len].instr.real_opcode() {
            Some(Opcode::Swap) => {
                depth = depth.max(u32::from(block.instructions[*ix + len].arg) as usize);
                more = true;
                len += 1;
            }
            Some(Opcode::Nop) => {
                len += 1;
            }
            _ => break,
        }
    }

    if !more {
        return Ok(());
    }

    let mut stack = Vec::new();
    stack
        .try_reserve_exact(depth)
        .map_err(|_| InternalError::MalformedControlFlowGraph)?;
    stack.resize(depth, 0);
    let mut i = 0;
    while i < depth {
        stack[i] = i as i32;
        i += 1;
    }

    i = 0;
    while i < len {
        let info = &block.instructions[*ix + i];
        if matches!(info.instr.real_opcode(), Some(Opcode::Swap)) {
            let oparg = u32::from(info.arg) as usize;
            stack.swap(0, oparg - 1);
        }
        i += 1;
    }

    let mut current = len as isize - 1;
    for i in 0..depth {
        if stack[i] == VISITED || stack[i] == i as i32 {
            continue;
        }
        let mut j = i;
        loop {
            if j != 0 {
                debug_assert!(current >= 0);
                let out = &mut block.instructions[*ix + current as usize];
                out.instr = Opcode::Swap.into();
                out.arg = OpArg::new((j + 1) as u32);
                current -= 1;
            }
            if stack[j] == VISITED {
                debug_assert_eq!(j, i);
                break;
            }
            let next_j = stack[j] as usize;
            stack[j] = VISITED;
            j = next_j;
        }
    }

    while current >= 0 {
        set_to_nop(&mut block.instructions[*ix + current as usize]);
        current -= 1;
    }
    *ix += len - 1;
    Ok(())
}

/// flowgraph.c apply_static_swaps
fn apply_static_swaps(block: &mut Block, mut i: isize) {
    while i >= 0 {
        let idx = i as usize;
        debug_assert!(idx < block.instruction_used);
        let swap_arg = match block.instructions[idx].instr.real_opcode() {
            Some(Opcode::Swap) => u32::from(block.instructions[idx].arg),
            Some(Opcode::Nop | Opcode::PopTop | Opcode::StoreFast) => {
                i -= 1;
                continue;
            }
            _ if matches!(
                block.instructions[idx].instr.pseudo_opcode(),
                Some(PseudoOpcode::StoreFastMaybeNull)
            ) =>
            {
                i -= 1;
                continue;
            }
            _ => return,
        };

        let Some(j) = next_swappable_instruction(block, idx, -1) else {
            return;
        };
        let lineno = instruction_lineno(&block.instructions[j]);
        let mut k = j;
        for _ in 1..swap_arg {
            let Some(next) = next_swappable_instruction(block, k, lineno) else {
                return;
            };
            k = next;
        }

        let store_j = stores_to(&block.instructions[j]);
        let store_k = stores_to(&block.instructions[k]);
        if store_j >= 0 || store_k >= 0 {
            if store_j == store_k {
                return;
            }
            let mut idx = j + 1;
            while idx < k {
                let store_idx = stores_to(&block.instructions[idx]);
                if store_idx >= 0 && (store_idx == store_j || store_idx == store_k) {
                    return;
                }
                idx += 1;
            }
        }

        set_to_nop(&mut block.instructions[idx]);
        block.instructions.swap(j, k);
        i -= 1;
    }
}

/// flowgraph.c optimize_basic_block swap pass
fn apply_static_swaps_block(block: &mut Block) -> crate::InternalResult<()> {
    let mut i = 0;
    while i < block.instruction_used {
        if matches!(
            block.instructions[i].instr.real_opcode(),
            Some(Opcode::Swap)
        ) {
            swaptimize(block, &mut i)?;
            apply_static_swaps(block, i as isize);
        }
        i += 1;
    }
    Ok(())
}

/// flowgraph.c maybe_instr_make_load_smallint
fn maybe_instr_make_load_smallint(instr: &mut InstructionInfo, constant: &ConstantData) -> bool {
    if let ConstantData::Integer { value } = constant
        && let Some(small) = value.to_i32().filter(|v| (0..=255).contains(v))
    {
        instr_set_op1(instr, Opcode::LoadSmallInt.into(), OpArg::new(small as u32));
        return true;
    }
    false
}

/// flowgraph.c basicblock_optimize_load_const
fn basicblock_optimize_load_const(
    metadata: &mut CodeUnitMetadata,
    block: &mut Block,
) -> crate::InternalResult<()> {
    let mut i = 0;
    let mut effective_opcode = None;
    let mut effective_oparg = OpArg::new(0);
    while i < block.instruction_used {
        if matches!(
            block.instructions[i].instr.real(),
            Some(Instruction::LoadConst { .. })
        ) && let Some(constant) = get_const_value(metadata, &block.instructions[i])
        {
            maybe_instr_make_load_smallint(&mut block.instructions[i], &constant);
        }

        let curr = block.instructions[i];
        let curr_arg = curr.arg;

        // Only combine if the source is a real instruction.
        let Some(curr_instr) = curr.instr.real() else {
            i += 1;
            continue;
        };

        let is_copy_of_load_const = matches!(
            (effective_opcode, curr_instr),
            (Some(Instruction::LoadConst { .. }), Instruction::Copy { i }) if i.get(curr_arg) == 1
        );
        if !is_copy_of_load_const {
            effective_opcode = Some(curr_instr);
            effective_oparg = curr_arg;
        }
        let Some(const_instr) = effective_opcode else {
            i += 1;
            continue;
        };
        let const_arg = effective_oparg;

        if i + 1 >= block.instruction_used {
            i += 1;
            continue;
        }

        let next = block.instructions[i + 1];
        let next_arg = next.arg;

        if let Some(is_true) = load_const_truthiness(const_instr, const_arg, metadata) {
            let const_jump = match (next.instr.real_opcode(), next.instr.pseudo_opcode()) {
                (_, Some(PseudoOpcode::JumpIfTrue)) => Some((true, false)),
                (_, Some(PseudoOpcode::JumpIfFalse)) => Some((false, false)),
                (Some(Opcode::PopJumpIfTrue), _) => Some((true, true)),
                (Some(Opcode::PopJumpIfFalse), _) => Some((false, true)),
                _ => None,
            };
            if let Some((jump_if_true, pops_condition)) = const_jump {
                if pops_condition {
                    set_to_nop(&mut block.instructions[i]);
                }
                if is_true == jump_if_true {
                    block.instructions[i + 1].instr = PseudoOpcode::Jump.into();
                } else {
                    set_to_nop(&mut block.instructions[i + 1]);
                }
                i += 1;
                continue;
            }
        }

        // The remaining combinations require both instructions to be real.
        let Some(next_instr) = next.instr.real() else {
            i += 1;
            continue;
        };

        if let Instruction::LoadConst { consti } = const_instr {
            let constant = &metadata.consts[consti.get(const_arg).as_usize()];
            if matches!(constant, ConstantData::None)
                && let Instruction::IsOp { invert } = next_instr
            {
                let mut jump_idx = i + 2;
                if jump_idx >= block.instruction_used {
                    i += 1;
                    continue;
                }

                if matches!(
                    block.instructions[jump_idx].instr.real(),
                    Some(Instruction::ToBool)
                ) {
                    set_to_nop(&mut block.instructions[jump_idx]);
                    jump_idx += 1;
                    if jump_idx >= block.instruction_used {
                        i += 1;
                        continue;
                    }
                }

                let Some(jump_instr) = block.instructions[jump_idx].instr.real() else {
                    i += 1;
                    continue;
                };

                let mut invert = matches!(
                    invert.get(next_arg),
                    rustpython_compiler_core::bytecode::Invert::Yes
                );
                match jump_instr {
                    Instruction::PopJumpIfFalse { .. } => {
                        invert = !invert;
                    }
                    Instruction::PopJumpIfTrue { .. } => {}
                    _ => {
                        i += 1;
                        continue;
                    }
                };

                set_to_nop(&mut block.instructions[i]);
                set_to_nop(&mut block.instructions[i + 1]);
                block.instructions[jump_idx].instr = if invert {
                    Opcode::PopJumpIfNotNone
                } else {
                    Opcode::PopJumpIfNone
                }
                .into();
                i = jump_idx;
                continue;
            }
        }

        if matches!(
            const_instr,
            Instruction::LoadConst { .. } | Instruction::LoadSmallInt { .. }
        ) && matches!(next_instr, Instruction::ToBool)
            && let Some(value) = load_const_truthiness(const_instr, const_arg, metadata)
        {
            let const_idx = add_const(metadata, ConstantData::Boolean { value })?;
            set_to_nop(&mut block.instructions[i]);
            instr_set_op1(
                &mut block.instructions[i + 1],
                Opcode::LoadConst.into(),
                OpArg::new(const_idx as u32),
            );
            i += 1;
            continue;
        }

        i += 1;
    }
    Ok(())
}

/// flowgraph.c optimize_load_const
fn optimize_load_const(
    metadata: &mut CodeUnitMetadata,
    blocks: &mut Blocks,
) -> crate::InternalResult<()> {
    let mut block_idx = BlockIdx(0);
    while block_idx != BlockIdx::NULL {
        let next_block = blocks[block_idx.idx()].next;
        let block = &mut blocks[block_idx];
        basicblock_optimize_load_const(metadata, block)?;
        block_idx = next_block;
    }
    Ok(())
}

#[cfg(test)]
impl CodeInfo {
    fn debug_block_dump(&self) -> String {
        let mut out = String::new();
        let mut block_idx = BlockIdx(0);
        while block_idx != BlockIdx::NULL {
            use core::fmt::Write;
            let block = &self.blocks[block_idx.idx()];
            let block_return = if basicblock_returns(block) {
                " return"
            } else {
                ""
            };
            let _ = writeln!(
                out,
                "block {} next={} cold={} except={} preserve_lasti={} start_depth={}{}",
                u32::from(block_idx),
                if block.next == BlockIdx::NULL {
                    String::from("NULL")
                } else {
                    u32::from(block.next).to_string()
                },
                block.cold,
                block.except_handler,
                block.preserve_lasti,
                if block.start_depth < 0 {
                    String::from("None")
                } else {
                    block.start_depth.to_string()
                },
                block_return,
            );
            for info in &block.instructions[..block.instruction_used] {
                let lineno = instruction_lineno(info);
                let _ = writeln!(
                    out,
                    "  [disp={}:{} raw={}:{}-{}:{} override={:?}] {:?} arg={} target={}",
                    lineno,
                    info.location.character_offset.get(),
                    info.location.line.get(),
                    info.location.character_offset.get(),
                    info.end_location.line.get(),
                    info.end_location.character_offset.get(),
                    info.lineno_override,
                    info.instr,
                    u32::from(info.arg),
                    if info.target == BlockIdx::NULL {
                        String::from("NULL")
                    } else {
                        u32::from(info.target).to_string()
                    }
                );
            }
            block_idx = block.next;
        }
        out
    }

    pub(crate) fn debug_late_cfg_trace(mut self) -> crate::InternalResult<Vec<(String, String)>> {
        let mut trace = Vec::new();
        trace.push(("initial".to_owned(), self.debug_block_dump()));

        let instr_sequence = self.prepare_cfg_from_codegen()?;
        self.blocks = cfg_from_instruction_sequence(instr_sequence)?;
        trace.push((
            "after_cfg_from_instruction_sequence".to_owned(),
            self.debug_block_dump(),
        ));
        translate_jump_labels_to_targets(&mut self.blocks)?;
        mark_except_handlers(&mut self.blocks)?;
        label_exception_targets(&mut self.blocks)?;
        check_cfg(&self.blocks)?;
        inline_small_or_no_lineno_blocks(&mut self.blocks)?;
        trace.push((
            "after_inline_small_or_no_lineno_blocks".to_owned(),
            self.debug_block_dump(),
        ));
        self.blocks.remove_unreachable()?;
        self.blocks
            .resolve_line_numbers(self.metadata.firstlineno)?;
        optimize_load_const(&mut self.metadata, &mut self.blocks)?;
        trace.push((
            "after_optimize_load_const".to_owned(),
            self.debug_block_dump(),
        ));
        let mut block_idx = BlockIdx(0);
        while block_idx != BlockIdx::NULL {
            let next_block = self.blocks[block_idx].next;
            self.blocks
                .optimize_basic_block(&mut self.metadata, block_idx)?;
            block_idx = next_block;
        }
        trace.push((
            "after_optimize_basic_block".to_owned(),
            self.debug_block_dump(),
        ));
        self.blocks.remove_redundant_nops_and_pairs()?;
        self.blocks.remove_unreachable()?;
        remove_redundant_nops_and_jumps(&mut self.blocks)?;
        #[cfg(debug_assertions)]
        assert!(no_redundant_jumps(&self.blocks));
        self.blocks
            .remove_unused_consts(&mut self.metadata.consts)?;
        trace.push((
            "after_optimize_cfg_cleanup".to_owned(),
            self.debug_block_dump(),
        ));
        let nlocals = self.metadata.varnames.len();
        let nparams = self.nparams;
        add_checks_for_loads_of_uninitialized_variables(&mut self.blocks, nlocals, nparams)?;
        self.blocks.insert_superinstructions()?;
        push_cold_blocks_to_end(&mut self.blocks)?;
        trace.push((
            "after_push_cold_before_chain_reorder".to_owned(),
            self.debug_block_dump(),
        ));
        self.blocks
            .resolve_line_numbers(self.metadata.firstlineno)?;
        trace.push((
            "after_push_cold_resolve_line_numbers".to_owned(),
            self.debug_block_dump(),
        ));

        trace.push((
            "after_push_cold_blocks_to_end".to_owned(),
            self.debug_block_dump(),
        ));

        convert_pseudo_conditional_jumps(&mut self.blocks)?;
        trace.push((
            "after_convert_pseudo_conditional_jumps".to_owned(),
            self.debug_block_dump(),
        ));

        let _max_stackdepth = self.blocks.calculate_stackdepth()?;
        let _nlocalsplus = prepare_localsplus(&self.metadata, &mut self.blocks, self.flags)?;
        convert_pseudo_ops(&mut self.blocks)?;
        trace.push((
            "after_convert_pseudo_ops".to_owned(),
            self.debug_block_dump(),
        ));

        self.blocks.normalize_jumps()?;
        #[cfg(debug_assertions)]
        assert!(no_redundant_jumps(&self.blocks));
        trace.push(("after_normalize_jumps".to_owned(), self.debug_block_dump()));
        self.blocks.optimize_load_fast()?;
        trace.push((
            "after_optimize_load_fast".to_owned(),
            self.debug_block_dump(),
        ));

        Ok(trace)
    }
}

impl InstrDisplayContext for CodeInfo {
    type Constant = ConstantData;

    fn get_constant(&self, consti: oparg::ConstIdx) -> &ConstantData {
        &self.metadata.consts[consti.as_usize()]
    }

    fn get_name(&self, i: usize) -> &str {
        self.metadata.names[i].as_ref()
    }

    fn get_varname(&self, var_num: oparg::VarNum) -> &str {
        self.metadata.varnames[var_num.as_usize()].as_ref()
    }

    fn get_localsplus_name(&self, var_num: oparg::VarNum) -> &str {
        let idx = var_num.as_usize();
        let nlocals = self.metadata.varnames.len();
        if idx < nlocals {
            self.metadata.varnames[idx].as_ref()
        } else {
            let cell_idx = idx - nlocals;
            self.metadata
                .cellvars
                .get_index(cell_idx)
                .unwrap_or_else(|| &self.metadata.freevars[cell_idx - self.metadata.cellvars.len()])
                .as_ref()
        }
    }
}

const NOT_LOCAL: isize = -1;
const DUMMY_INSTR: isize = -1;

/// flowgraph.c make_super_instruction
fn make_super_instruction(
    inst1: &mut InstructionInfo,
    inst2: &mut InstructionInfo,
    super_op: AnyInstruction,
) {
    let line1 = instruction_lineno(inst1);
    let line2 = instruction_lineno(inst2);
    if line1 >= 0 && line2 >= 0 && line1 != line2 {
        return;
    }
    let arg1 = u32::from(inst1.arg);
    let arg2 = u32::from(inst2.arg);
    if arg1 >= 16 || arg2 >= 16 {
        return;
    }
    instr_set_op1(inst1, super_op, OpArg::new((arg1 << 4) | arg2));
    set_to_nop(inst2);
}

/// flowgraph.c LoadFastInstrFlag
#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(u8)]
enum LoadFastInstrFlag {
    SupportKilled = 1,
    StoredAsLocal = 2,
    RefUnconsumed = 4,
}

/// flowgraph.c ref
#[derive(Clone, Copy)]
struct Ref {
    instr: isize,
    local: isize,
}

/// flowgraph.c ref_stack
struct RefStack {
    refs: Vec<Ref>,
    size: usize,
    capacity: usize,
}

/// flowgraph.c ref_stack_push
fn ref_stack_push(stack: &mut RefStack, r: Ref) -> crate::InternalResult<()> {
    debug_assert_eq!(stack.refs.len(), stack.capacity);
    if stack.size == stack.capacity {
        let doubled = stack.capacity * 2;
        let new_cap = 32.max(doubled);
        stack
            .refs
            .try_reserve_exact(new_cap - stack.capacity)
            .map_err(|_| InternalError::MalformedControlFlowGraph)?;
        stack.refs.resize(new_cap, Ref { instr: 0, local: 0 });
        stack.capacity = new_cap;
    }
    stack.refs[stack.size] = r;
    stack.size += 1;
    Ok(())
}

/// flowgraph.c ref_stack_pop
fn ref_stack_pop(stack: &mut RefStack) -> Ref {
    assert!(stack.size > 0);
    stack.size -= 1;
    stack.refs[stack.size]
}

/// flowgraph.c ref_stack_swap_top
fn ref_stack_swap_top(stack: &mut RefStack, off: usize) {
    assert!(off >= 2 && stack.size >= off);
    let top = stack.size - 1;
    let other = stack.size - off;
    stack.refs.swap(top, other);
}

/// flowgraph.c ref_stack_at
fn ref_stack_at(stack: &RefStack, idx: usize) -> Ref {
    assert!(idx < stack.size);
    stack.refs[idx]
}

/// flowgraph.c ref_stack_clear
fn ref_stack_clear(stack: &mut RefStack) {
    stack.size = 0;
}

/// flowgraph.c optimize_load_fast PUSH_REF
fn push_ref(stack: &mut RefStack, instr: isize, local: isize) -> crate::InternalResult<()> {
    ref_stack_push(stack, Ref { instr, local })
}

/// flowgraph.c kill_local
fn kill_local(instr_flags: &mut [u8], refs: &RefStack, local: isize) {
    for i in 0..refs.size {
        let r = ref_stack_at(refs, i);
        if r.local != local {
            continue;
        }
        debug_assert!(r.instr >= 0);
        instr_flags[r.instr as usize] |= LoadFastInstrFlag::SupportKilled as u8;
    }
}

/// flowgraph.c store_local
fn store_local(instr_flags: &mut [u8], refs: &RefStack, local: isize, r: Ref) {
    kill_local(instr_flags, refs, local);
    if r.instr != DUMMY_INSTR {
        instr_flags[r.instr as usize] |= LoadFastInstrFlag::StoredAsLocal as u8;
    }
}

fn local_as_ref_local(local: usize) -> isize {
    local as isize
}

/// flowgraph.c load_fast_push_block
fn load_fast_push_block(
    worklist: &mut CfgTraversalStack,
    blocks: &mut Blocks,
    target: BlockIdx,
    start_depth: usize,
) {
    debug_assert!(target != BlockIdx::NULL);
    debug_assert!(blocks[target].start_depth >= 0);
    debug_assert_eq!(blocks[target].start_depth as usize, start_depth,);
    if !blocks[target].visited {
        blocks[target].visited = true;
        worklist.push(target);
    }
}

fn stackdepth_push(
    stack: &mut CfgTraversalStack,
    blocks: &mut Blocks,
    target: BlockIdx,
    depth: i32,
) -> crate::InternalResult<()> {
    let idx = target.idx();
    let block_depth = &mut blocks[idx].start_depth;
    if !(*block_depth < 0 || *block_depth == depth) {
        return Err(InternalError::InconsistentStackDepth);
    }
    if *block_depth < depth && *block_depth < 100 {
        debug_assert!(*block_depth < 0);
        *block_depth = depth;
        stack.push(target);
    }
    Ok(())
}

/// flowgraph.c stack_effects
#[derive(Clone, Copy, Eq, PartialEq)]
struct StackEffects {
    net: i32,
}

/// flowgraph.c get_stack_effects
#[allow(clippy::unnecessary_wraps)]
fn get_stack_effects(
    instr: AnyInstruction,
    oparg: OpArg,
    jump: i32,
) -> crate::InternalResult<StackEffects> {
    if instr
        .real()
        .is_some_and(|op| op.as_opcode().deopt().is_some())
    {
        return Err(InternalError::InvalidStackEffect);
    }
    let oparg = u32::from(oparg);
    let net = if instr.is_block_push() && jump == 0 {
        0
    } else if jump != 0 {
        instr.stack_effect_jump(oparg)
    } else {
        instr.stack_effect(oparg)
    };
    Ok(StackEffects { net })
}

fn vec_try_reserve_exact<T>(vec: &mut Vec<T>, additional: usize) -> crate::InternalResult<()> {
    vec.try_reserve_exact(additional)
        .map_err(|_| InternalError::MalformedControlFlowGraph)
}

fn vec_try_resize_to_double_capacity<T>(vec: &mut Vec<T>) -> crate::InternalResult<()> {
    let capacity = vec.capacity();
    debug_assert!(capacity > 0);
    let len = capacity
        .checked_mul(core::mem::size_of::<T>())
        .ok_or(InternalError::MalformedControlFlowGraph)?;
    if capacity == 0 || len > usize::MAX / 2 {
        return Err(InternalError::MalformedControlFlowGraph);
    }
    let new_capacity = capacity * 2;
    let additional = new_capacity
        .checked_sub(vec.len())
        .ok_or(InternalError::MalformedControlFlowGraph)?;
    vec_try_reserve_exact(vec, additional)
}

/// assemble.c write_location_first_byte
fn write_location_first_byte(linetable: &mut Vec<u8>, code: u8, length: usize) {
    linetable.extend(write_location_entry_start(code, length));
}

/// pycore_code.h write_location_entry_start
fn write_location_entry_start(code: u8, length: usize) -> [u8; 1] {
    debug_assert!(length > 0 && length <= 8);
    debug_assert_eq!(code & 15, code);
    [0x80 | (code << 3) | ((length - 1) as u8)]
}

/// assemble.c write_location_byte
fn write_location_byte(linetable: &mut Vec<u8>, value: u8) {
    linetable.push(value);
}

/// assemble.c write_location_varint
fn write_location_varint(linetable: &mut Vec<u8>, value: u32) {
    write_varint(linetable, value);
}

/// assemble.c write_location_signed_varint
fn write_location_signed_varint(linetable: &mut Vec<u8>, value: i32) {
    write_signed_varint(linetable, value);
}

/// assemble.c write_location_info_short_form
fn write_location_info_short_form(
    linetable: &mut Vec<u8>,
    length: usize,
    column: i32,
    end_column: i32,
) {
    debug_assert!(length > 0 && length <= 8);
    debug_assert!(column < 80);
    debug_assert!(end_column >= column);
    debug_assert!(end_column - column < 16);
    let column_low_bits = column & 7;
    let column_group = column >> 3;
    let code = PyCodeLocationInfoKind::Short0 as u8 + column_group as u8;
    write_location_first_byte(linetable, code, length);
    write_location_byte(
        linetable,
        ((column_low_bits as u8) << 4) | ((end_column - column) as u8),
    );
}

/// assemble.c write_location_info_oneline_form
fn write_location_info_oneline_form(
    linetable: &mut Vec<u8>,
    length: usize,
    line_delta: i32,
    column: i32,
    end_column: i32,
) {
    debug_assert!(length > 0 && length <= 8);
    debug_assert!((0..3).contains(&line_delta));
    debug_assert!(column < 128);
    debug_assert!(end_column < 128);
    let code = PyCodeLocationInfoKind::OneLine0 as u8 + line_delta as u8;
    write_location_first_byte(linetable, code, length);
    write_location_byte(linetable, column as u8);
    write_location_byte(linetable, end_column as u8);
}

/// assemble.c write_location_info_long_form
fn write_location_info_long_form(
    linetable: &mut Vec<u8>,
    loc: LineTableLocation,
    length: usize,
    line_delta: i32,
) {
    debug_assert!(length > 0 && length <= 8);
    write_location_first_byte(linetable, PyCodeLocationInfoKind::Long as u8, length);
    write_location_signed_varint(linetable, line_delta);
    debug_assert!(loc.end_line >= loc.line);
    write_location_varint(linetable, (loc.end_line - loc.line) as u32);
    write_location_varint(
        linetable,
        if loc.col < 0 { 0 } else { (loc.col as u32) + 1 },
    );
    write_location_varint(
        linetable,
        if loc.end_col < 0 {
            0
        } else {
            (loc.end_col as u32) + 1
        },
    );
}

/// assemble.c write_location_info_none
fn write_location_info_none(linetable: &mut Vec<u8>, length: usize) {
    write_location_first_byte(linetable, PyCodeLocationInfoKind::None as u8, length);
}

/// assemble.c write_location_info_no_column
fn write_location_info_no_column(linetable: &mut Vec<u8>, length: usize, line_delta: i32) {
    write_location_first_byte(linetable, PyCodeLocationInfoKind::NoColumns as u8, length);
    write_location_signed_varint(linetable, line_delta);
}

/// assemble.c write_location_info_entry
fn write_location_info_entry(
    linetable: &mut Vec<u8>,
    loc: LineTableLocation,
    length: usize,
    prev_line: &mut i32,
    debug_ranges: bool,
) -> crate::InternalResult<()> {
    const THEORETICAL_MAX_ENTRY_SIZE: usize = 25;
    if linetable
        .len()
        .checked_add(THEORETICAL_MAX_ENTRY_SIZE)
        .ok_or(InternalError::MalformedControlFlowGraph)?
        >= linetable.capacity()
    {
        debug_assert!(linetable.capacity() > THEORETICAL_MAX_ENTRY_SIZE);
        vec_try_resize_to_double_capacity(linetable)?;
    }
    if loc.line == NO_LOCATION_OVERRIDE {
        write_location_info_none(linetable, length);
        return Ok(());
    }

    let line_delta = loc.line - *prev_line;
    let column = loc.col;
    let end_column = loc.end_col;
    if !debug_ranges
        || ((column < 0 || end_column < 0) && (loc.end_line == loc.line || loc.end_line < 0))
    {
        write_location_info_no_column(linetable, length, line_delta);
        *prev_line = loc.line;
        return Ok(());
    }

    if loc.end_line == loc.line {
        if line_delta == 0 && column < 80 && end_column - column < 16 && end_column >= column {
            write_location_info_short_form(linetable, length, column, end_column);
            return Ok(());
        }
        if (0..3).contains(&line_delta) && column < 128 && end_column < 128 {
            write_location_info_oneline_form(linetable, length, line_delta, column, end_column);
            *prev_line = loc.line;
            return Ok(());
        }
    }

    write_location_info_long_form(linetable, loc, length, line_delta);
    *prev_line = loc.line;
    Ok(())
}

/// assemble.c assemble_emit_location
fn assemble_emit_location(
    linetable: &mut Vec<u8>,
    loc: LineTableLocation,
    mut size: usize,
    prev_line: &mut i32,
    debug_ranges: bool,
) -> crate::InternalResult<()> {
    if size == 0 {
        return Ok(());
    }
    while size > 8 {
        write_location_info_entry(linetable, loc, 8, prev_line, debug_ranges)?;
        size -= 8;
    }
    write_location_info_entry(linetable, loc, size, prev_line, debug_ranges)
}

fn no_linetable_location() -> LineTableLocation {
    LineTableLocation {
        line: NO_LOCATION_OVERRIDE,
        end_line: NO_LOCATION_OVERRIDE,
        col: NO_LOCATION_OVERRIDE,
        end_col: NO_LOCATION_OVERRIDE,
    }
}

fn next_linetable_location() -> LineTableLocation {
    LineTableLocation {
        line: NEXT_LOCATION_OVERRIDE,
        end_line: NEXT_LOCATION_OVERRIDE,
        col: NEXT_LOCATION_OVERRIDE,
        end_col: NEXT_LOCATION_OVERRIDE,
    }
}

/// assemble.c assemble_emit_exception_table_item
fn assemble_emit_exception_table_item(table: &mut Vec<u8>, value: i32, mut msb: u8) {
    debug_assert!((msb | 128) == 128);
    debug_assert!((0..(1 << 30)).contains(&value));
    let value = value as u32;
    const CONTINUATION_BIT: u8 = 64;
    if value >= 1 << 24 {
        table.push(((value >> 24) as u8) | CONTINUATION_BIT | msb);
        msb = 0;
    }
    if value >= 1 << 18 {
        table.push((((value >> 18) & 0x3f) as u8) | CONTINUATION_BIT | msb);
        msb = 0;
    }
    if value >= 1 << 12 {
        table.push((((value >> 12) & 0x3f) as u8) | CONTINUATION_BIT | msb);
        msb = 0;
    }
    if value >= 1 << 6 {
        table.push((((value >> 6) & 0x3f) as u8) | CONTINUATION_BIT | msb);
        msb = 0;
    }
    table.push(((value & 0x3f) as u8) | msb);
}

/// assemble.c assemble_emit_exception_table_entry
fn assemble_emit_exception_table_entry(
    table: &mut Vec<u8>,
    start: i32,
    end: i32,
    handler_offset: i32,
    handler: InstructionSequenceExceptHandlerInfo,
) -> crate::InternalResult<()> {
    const MAX_SIZE_OF_ENTRY: usize = 20;
    if table
        .len()
        .checked_add(MAX_SIZE_OF_ENTRY)
        .ok_or(InternalError::MalformedControlFlowGraph)?
        >= table.capacity()
    {
        vec_try_resize_to_double_capacity(table)?;
    }
    let size = end - start;
    debug_assert!(end > start);
    let target = handler_offset;
    let mut depth = handler.start_depth - 1;
    if handler.preserve_lasti > 0 {
        depth -= 1;
    }
    debug_assert!(depth >= 0);
    let depth_lasti = (depth << 1) | handler.preserve_lasti;
    assemble_emit_exception_table_item(table, start, 1 << 7);
    assemble_emit_exception_table_item(table, size, 0);
    assemble_emit_exception_table_item(table, target, 0);
    assemble_emit_exception_table_item(table, depth_lasti, 0);
    Ok(())
}

/// assemble.c assemble_exception_table
fn assemble_exception_table(
    instrs: &[InstructionSequenceEntry],
) -> crate::InternalResult<Box<[u8]>> {
    let mut table = Vec::new();
    vec_try_reserve_exact(&mut table, DEFAULT_LNOTAB_SIZE)?;
    let mut handler = InstructionSequenceExceptHandlerInfo {
        h_label: NO_EXCEPTION_HANDLER_LABEL,
        start_depth: -1,
        preserve_lasti: -1,
    };
    let mut start = -1;
    let mut ioffset = 0i32;

    for i in 0..instrs.len() {
        let instr = &instrs[i];
        if instr.except_handler.h_label != handler.h_label {
            if handler.h_label >= 0 {
                let handler_offset = instrs[handler.h_label as usize].i_offset;
                assemble_emit_exception_table_entry(
                    &mut table,
                    start,
                    ioffset,
                    handler_offset,
                    handler,
                )?;
            }
            start = ioffset;
            handler = instr.except_handler;
        }
        ioffset += instr_size(&instr.info) as i32;
    }

    if handler.h_label >= 0 {
        let handler_offset = instrs[handler.h_label as usize].i_offset;
        assemble_emit_exception_table_entry(&mut table, start, ioffset, handler_offset, handler)?;
    }

    Ok(table.into_boxed_slice())
}

/// Mark exception handler target blocks.
/// flowgraph.c mark_except_handlers
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn mark_except_handlers(blocks: &mut Blocks) -> crate::InternalResult<()> {
    #[cfg(debug_assertions)]
    {
        let mut block_idx = BlockIdx(0);
        while block_idx != BlockIdx::NULL {
            assert!(!blocks[block_idx].except_handler);
            block_idx = blocks[block_idx].next;
        }
    }

    let mut block_idx = BlockIdx(0);
    while block_idx != BlockIdx::NULL {
        let next = blocks[block_idx].next;
        let instr_count = blocks[block_idx].instruction_used;
        for i in 0..instr_count {
            let instr = blocks[block_idx].instructions[i];
            if is_block_push(&instr) {
                debug_assert!(instr.target != BlockIdx::NULL);
                blocks[instr.target].except_handler = true;
            }
        }
        block_idx = next;
    }
    Ok(())
}

/// flowgraph.c mark_cold (two-pass to match CPython).
///
/// Phase 1 (mark_warm): propagate "warm" from entry via fall-through and
/// jump targets. CPython asserts while visiting warm blocks that they are not
/// exception handlers.
///
/// Phase 2 (mark_cold): propagate "cold" from except_handler blocks via
/// forward edges. Blocks reached only via runtime exception dispatch are
/// marked cold and pushed to the end by push_cold_blocks_to_end.
///
/// Blocks reached by neither phase remain `cold=false`. They are typically
/// empty unreachable placeholders left by remove_unreachable; they stay in
/// their original chain position (e.g. between entry and the post-try
/// continuation for a nested try/except whose inner_end was emptied by
/// optimize_cfg). This matches CPython's behavior and is necessary for
/// optimize_load_fast to terminate fall-through at those placeholders.
/// flowgraph.c mark_warm
fn mark_warm(blocks: &mut Blocks) -> crate::InternalResult<()> {
    let mut stack = blocks.make_cfg_traversal_stack()?;
    stack.push(BlockIdx(0));
    blocks[0].visited = true;
    while let Some(block_idx) = stack.pop() {
        let idx = block_idx.idx();
        debug_assert!(!blocks[idx].except_handler);
        blocks[idx].warm = true;

        let next = blocks[idx].next;
        if next != BlockIdx::NULL && bb_has_fallthrough(&blocks[idx]) && !blocks[next].visited {
            stack.push(next);
            blocks[next.idx()].visited = true;
        }

        let instr_count = blocks[idx].instruction_used;
        for i in 0..instr_count {
            let instr = blocks[idx].instructions[i];
            if is_jump(&instr) {
                let target = instr.target;
                debug_assert!(target != BlockIdx::NULL);
                if !blocks[target.idx()].visited {
                    stack.push(target);
                    blocks[target.idx()].visited = true;
                }
            }
        }
    }
    Ok(())
}

fn mark_cold(blocks: &mut Blocks) -> crate::InternalResult<()> {
    let mut block_idx = BlockIdx(0);
    while block_idx != BlockIdx::NULL {
        let block = &mut blocks[block_idx];
        debug_assert!(!block.cold);
        debug_assert!(!block.warm);
        block_idx = block.next;
    }

    mark_warm(blocks)?;

    let mut cold_stack = blocks.make_cfg_traversal_stack()?;
    block_idx = BlockIdx(0);
    while block_idx != BlockIdx::NULL {
        let i = block_idx.idx();
        let next = blocks[i].next;
        let block = &blocks[i];
        if block.except_handler {
            debug_assert!(!block.warm);
            cold_stack.push(block_idx);
            blocks[i].visited = true;
        }
        block_idx = next;
    }
    while let Some(block_idx) = cold_stack.pop() {
        let idx = block_idx.idx();
        blocks[idx].cold = true;
        let next = blocks[idx].next;
        if next != BlockIdx::NULL && bb_has_fallthrough(&blocks[idx]) {
            let next_idx = next.idx();
            if !blocks[next_idx].warm && !blocks[next_idx].visited {
                cold_stack.push(next);
                blocks[next_idx].visited = true;
            }
        }

        let instr_count = blocks[idx].instruction_used;
        for i in 0..instr_count {
            let instr = blocks[idx].instructions[i];
            if is_jump(&instr) {
                debug_assert_eq!(i, instr_count - 1);
                let target = instr.target;
                debug_assert!(target != BlockIdx::NULL);
                if !blocks[target.idx()].warm && !blocks[target.idx()].visited {
                    cold_stack.push(target);
                    blocks[target.idx()].visited = true;
                }
            }
        }
    }
    Ok(())
}

/// flowgraph.c push_cold_blocks_to_end
fn push_cold_blocks_to_end(blocks: &mut Blocks) -> crate::InternalResult<()> {
    if blocks[0].next == BlockIdx::NULL {
        return Ok(());
    }

    mark_cold(blocks)?;
    let mut next_label = get_max_label(blocks) + 1;

    // If a cold block falls through to a warm block, add an explicit jump
    let mut block_idx = BlockIdx(0);
    while block_idx != BlockIdx::NULL {
        let next = blocks[block_idx].next;
        if blocks[block_idx].cold
            && bb_has_fallthrough(&blocks[block_idx])
            && next != BlockIdx::NULL
            && blocks[next].warm
        {
            let explicit_jump = blocks_new_block(blocks)?;
            if !is_label(blocks[next].cpython_label) {
                blocks[next].cpython_label = InstructionSequenceLabel::from_index(next_label);
                next_label += 1;
            }
            let jump_label = blocks[next].cpython_label;
            debug_assert!(is_label(jump_label));
            basicblock_addop(
                &mut blocks[explicit_jump],
                InstructionInfo {
                    instr: PseudoOpcode::JumpNoInterrupt.into(),
                    arg: instruction_sequence_label_oparg(jump_label),
                    target: BlockIdx::NULL,
                    location: SourceLocation::default(),
                    end_location: SourceLocation::default(),
                    except_handler: None,
                    lineno_override: Some(NO_LOCATION_OVERRIDE),
                },
            )?;
            blocks[explicit_jump].cold = true;
            blocks[explicit_jump].next = next;
            blocks[explicit_jump].predecessors = 1;
            blocks[block_idx].next = explicit_jump;
            let target = blocks[explicit_jump].next;
            let last = basicblock_last_instr_mut(&mut blocks[explicit_jump])
                .expect("missing explicit jump");
            last.target = target;
        }
        block_idx = blocks[block_idx].next;
    }

    assert!(!blocks[0].cold);
    let mut cold_blocks: BlockIdx = BlockIdx::NULL;
    let mut cold_blocks_tail: BlockIdx = BlockIdx::NULL;
    let mut block_idx = BlockIdx(0);

    while blocks[block_idx].next != BlockIdx::NULL {
        debug_assert!(!blocks[block_idx].cold);
        while blocks[block_idx].next != BlockIdx::NULL && !blocks[blocks[block_idx].next].cold {
            block_idx = blocks[block_idx].next;
        }
        if blocks[block_idx].next == BlockIdx::NULL {
            break;
        }

        debug_assert!(!blocks[block_idx].cold);
        debug_assert!(blocks[blocks[block_idx].next].cold);

        let mut block_end = blocks[block_idx].next;
        while blocks[block_end].next != BlockIdx::NULL && blocks[blocks[block_end].next].cold {
            block_end = blocks[block_end].next;
        }

        debug_assert!(blocks[block_end].cold);
        debug_assert!(
            blocks[block_end].next == BlockIdx::NULL || !blocks[blocks[block_end].next].cold
        );

        if cold_blocks == BlockIdx::NULL {
            cold_blocks = blocks[block_idx].next;
        } else {
            blocks[cold_blocks_tail].next = blocks[block_idx].next;
        }

        cold_blocks_tail = block_end;
        blocks[block_idx].next = blocks[block_end].next;
        blocks[block_end].next = BlockIdx::NULL;
    }

    debug_assert!(blocks[block_idx].next == BlockIdx::NULL);
    blocks[block_idx].next = cold_blocks;

    if cold_blocks != BlockIdx::NULL {
        remove_redundant_nops_and_jumps(blocks)?;
    }
    Ok(())
}

/// flowgraph.c check_cfg
fn check_cfg(blocks: &Blocks) -> crate::InternalResult<()> {
    let mut block_idx = BlockIdx(0);
    while block_idx != BlockIdx::NULL {
        let block = &blocks[block_idx];
        for i in 0..block.instruction_used {
            let opcode = block.instructions[i].instr;
            debug_assert!(!opcode.is_assembler());
            if opcode.is_terminator() && i != block.instruction_used - 1 {
                return Err(InternalError::MalformedControlFlowGraph);
            }
        }
        block_idx = block.next;
    }
    Ok(())
}

/// flowgraph.c jump_thread
fn jump_thread(
    blocks: &mut Blocks,
    block_idx: BlockIdx,
    instr_idx: usize,
    target: &InstructionInfo,
    opcode: AnyInstruction,
) -> crate::InternalResult<bool> {
    let bi = block_idx.idx();
    debug_assert!(is_jump(&blocks[bi].instructions[instr_idx]));
    debug_assert!(is_jump(target));
    debug_assert_eq!(instr_idx + 1, blocks[bi].instruction_used);
    debug_assert!(target.target != BlockIdx::NULL);
    if blocks[bi].instructions[instr_idx].target != target.target {
        set_to_nop(&mut blocks[bi].instructions[instr_idx]);
        basicblock_add_jump(blocks, block_idx, opcode, target.target, target)?;
        return Ok(true);
    }
    Ok(false)
}

/// flowgraph.c basicblock_add_jump
fn basicblock_add_jump(
    blocks: &mut Blocks,
    block_idx: BlockIdx,
    instr: AnyInstruction,
    target: BlockIdx,
    loc_source: &InstructionInfo,
) -> crate::InternalResult<()> {
    let bi = block_idx.idx();
    let last = basicblock_last_instr(&blocks[bi]);
    if last.is_some_and(is_jump) {
        return Err(InternalError::MalformedControlFlowGraph);
    }
    debug_assert!(target != BlockIdx::NULL);
    let label = blocks[target.idx()].cpython_label;
    debug_assert!(is_label(label));
    let arg = instruction_sequence_label_oparg(label);
    let block = &mut blocks[bi];
    basicblock_addop(
        block,
        InstructionInfo {
            instr,
            arg,
            target: BlockIdx::NULL,
            location: loc_source.location,
            end_location: loc_source.end_location,
            except_handler: None,
            lineno_override: loc_source.lineno_override,
        },
    )?;
    let last = basicblock_last_instr_mut(block).expect("missing jump");
    debug_assert!(match (last.instr, instr) {
        (AnyInstruction::Real(last), AnyInstruction::Real(opcode)) =>
            last.as_opcode() == opcode.as_opcode(),
        (AnyInstruction::Pseudo(last), AnyInstruction::Pseudo(opcode)) =>
            last.as_opcode() == opcode.as_opcode(),
        _ => false,
    });
    last.target = target;
    Ok(())
}

/// pycore_opcode_utils.h IS_CONDITIONAL_JUMP_OPCODE
fn is_conditional_jump_opcode(instr: AnyInstruction) -> bool {
    matches!(
        instr.real().map(Into::into),
        Some(
            Opcode::PopJumpIfFalse
                | Opcode::PopJumpIfTrue
                | Opcode::PopJumpIfNone
                | Opcode::PopJumpIfNotNone
        )
    )
}

/// flowgraph.c convert_pseudo_conditional_jumps
fn convert_pseudo_conditional_jumps(blocks: &mut Blocks) -> crate::InternalResult<()> {
    let mut block_idx = BlockIdx(0);
    while block_idx != BlockIdx::NULL {
        let next = blocks[block_idx.idx()].next;
        let block = &mut blocks[block_idx.idx()];
        let mut i = 0;
        while i < block.instruction_used {
            let instr = block.instructions[i];
            let opcode = instr.instr;
            if matches!(
                opcode.pseudo_opcode(),
                Some(PseudoOpcode::JumpIfFalse | PseudoOpcode::JumpIfTrue)
            ) {
                debug_assert_eq!(i, block.instruction_used - 1);
                block.instructions[i].instr =
                    if matches!(opcode.pseudo_opcode(), Some(PseudoOpcode::JumpIfFalse)) {
                        Opcode::PopJumpIfFalse
                    } else {
                        Opcode::PopJumpIfTrue
                    }
                    .into();

                let location = instr.location;
                let end_location = instr.end_location;
                let except_handler = instr.except_handler;
                let lineno_override = instr.lineno_override;
                let copy = InstructionInfo {
                    instr: Opcode::Copy.into(),
                    arg: OpArg::new(1),
                    target: BlockIdx::NULL,
                    location,
                    end_location,
                    except_handler,
                    lineno_override,
                };
                basicblock_insert_instruction(block, i, copy)?;
                i += 1;

                let to_bool = InstructionInfo {
                    instr: Opcode::ToBool.into(),
                    arg: OpArg::new(0),
                    target: BlockIdx::NULL,
                    location,
                    end_location,
                    except_handler,
                    lineno_override,
                };
                basicblock_insert_instruction(block, i, to_bool)?;
                i += 1;
            }
            i += 1;
        }
        block_idx = next;
    }
    Ok(())
}

/// flowgraph.c normalize_jumps_in_block
fn normalize_jumps_in_block(blocks: &mut Blocks, block_idx: BlockIdx) -> crate::InternalResult<()> {
    let idx = block_idx.idx();
    let Some(last_ins) = basicblock_last_instr(&blocks[idx]).copied() else {
        return Ok(());
    };
    if !is_conditional_jump_opcode(last_ins.instr) {
        return Ok(());
    }
    debug_assert!(!last_ins.instr.is_assembler());

    debug_assert!(last_ins.target != BlockIdx::NULL);
    let is_forward = !blocks[last_ins.target.idx()].visited;

    if is_forward {
        // Insert NOT_TAKEN after forward conditional jump.
        let not_taken = InstructionInfo {
            instr: Opcode::NotTaken.into(),
            arg: OpArg::new(0),
            target: BlockIdx::NULL,
            location: last_ins.location,
            end_location: last_ins.end_location,
            except_handler: None,
            lineno_override: last_ins.lineno_override,
        };
        basicblock_addop(&mut blocks[idx], not_taken)?;
        return Ok(());
    }

    let reversed_opcode = match last_ins.instr.real_opcode() {
        Some(Opcode::PopJumpIfNotNone) => Opcode::PopJumpIfNone.into(),
        Some(Opcode::PopJumpIfNone) => Opcode::PopJumpIfNotNone.into(),
        Some(Opcode::PopJumpIfFalse) => Opcode::PopJumpIfTrue.into(),
        Some(Opcode::PopJumpIfTrue) => Opcode::PopJumpIfFalse.into(),
        _ => unreachable!("conditional jump has reverse opcode"),
    };

    // Transform 'conditional jump T' to 'reversed_jump b_next' followed by
    // 'jump_backwards T'.
    let loc = last_ins.location;
    let end_loc = last_ins.end_location;

    let target = last_ins.target;
    let backwards_jump_idx = blocks_new_block(blocks)?;
    basicblock_addop(
        &mut blocks[backwards_jump_idx.idx()],
        InstructionInfo {
            instr: Opcode::NotTaken.into(),
            arg: OpArg::new(0),
            target: BlockIdx::NULL,
            location: loc,
            end_location: end_loc,
            except_handler: None,
            lineno_override: last_ins.lineno_override,
        },
    )?;
    basicblock_add_jump(
        blocks,
        backwards_jump_idx,
        PseudoOpcode::Jump.into(),
        target,
        &last_ins,
    )?;
    blocks[backwards_jump_idx.idx()].start_depth = blocks[target.idx()].start_depth;

    let old_next = blocks[idx].next;
    debug_assert!(old_next != BlockIdx::NULL);

    let last_mut = basicblock_last_instr_mut(&mut blocks[idx]).unwrap();
    last_mut.instr = reversed_opcode;
    last_mut.target = old_next;

    blocks[backwards_jump_idx.idx()].cold = blocks[idx].cold;
    blocks[backwards_jump_idx.idx()].next = old_next;
    blocks[idx].next = backwards_jump_idx;
    Ok(())
}

/// flowgraph.c basicblock_inline_small_or_no_lineno_blocks
fn basicblock_inline_small_or_no_lineno_blocks(
    blocks: &mut Blocks,
    block_idx: BlockIdx,
) -> crate::InternalResult<bool> {
    let Some(last) = basicblock_last_instr(&blocks[block_idx]).copied() else {
        return Ok(false);
    };
    if !last.instr.is_unconditional_jump() {
        return Ok(false);
    }

    let target = last.target;
    debug_assert!(target != BlockIdx::NULL);
    let small_exit_block =
        basicblock_exits_scope(&blocks[target]) && blocks[target].instruction_used <= MAX_COPY_SIZE;
    let no_lineno_no_fallthrough =
        basicblock_has_no_lineno(&blocks[target]) && !bb_has_fallthrough(&blocks[target]);
    if small_exit_block || no_lineno_no_fallthrough {
        debug_assert!(is_jump(&last));
        let removed_jump_opcode = last.instr;
        let last = basicblock_last_instr_mut(&mut blocks[block_idx])
            .expect("non-empty block has last instruction");
        set_to_nop(last);
        blocks.basicblock_append_block_instructions(block_idx, target)?;
        if no_lineno_no_fallthrough {
            let last = basicblock_last_instr_mut(&mut blocks[block_idx]).unwrap();
            if last.instr.is_unconditional_jump()
                && matches!(
                    removed_jump_opcode.into(),
                    AnyOpcode::Pseudo(PseudoOpcode::Jump)
                )
            {
                last.instr = PseudoOpcode::Jump.into();
            }
        }
        blocks[target].predecessors -= 1;
        return Ok(true);
    }
    Ok(false)
}

/// flowgraph.c inline_small_or_no_lineno_blocks
fn inline_small_or_no_lineno_blocks(blocks: &mut Blocks) -> crate::InternalResult<bool> {
    loop {
        let mut changes = false;
        let mut current = BlockIdx(0);
        while current != BlockIdx::NULL {
            let next = blocks[current.idx()].next;
            let res = basicblock_inline_small_or_no_lineno_blocks(blocks, current)?;
            if res {
                changes = true;
            }

            current = next;
        }
        if !changes {
            return Ok(changes);
        }
    }
}

/// flowgraph.c basicblock_remove_redundant_nops
#[allow(clippy::unnecessary_wraps)]
fn basicblock_remove_redundant_nops(
    blocks: &mut Blocks,
    block_idx: BlockIdx,
) -> crate::InternalResult<usize> {
    let bi = block_idx.idx();
    let mut dest = 0;
    let mut prev_lineno = -1i32;
    let instr_count = blocks[bi].instruction_used;

    for src in 0..instr_count {
        let instr = blocks[bi].instructions[src];
        let lineno = instruction_lineno(&instr);

        if matches!(instr.instr.real(), Some(Instruction::Nop)) {
            if lineno < 0 {
                continue;
            }
            if prev_lineno == lineno {
                continue;
            }
            if src < instr_count - 1 {
                let next_lineno = instruction_lineno(&blocks[bi].instructions[src + 1]);
                if next_lineno == lineno {
                    continue;
                }
                if next_lineno < 0 {
                    instr_set_loc(
                        &mut blocks[bi].instructions[src + 1],
                        instr.location,
                        instr.end_location,
                        instr.lineno_override,
                    );
                    continue;
                }
            } else {
                let next = next_nonempty_block(blocks, blocks[bi].next);
                if next != BlockIdx::NULL {
                    let mut next_loc = no_linetable_location();
                    let mut next_i = 0;
                    while next_i < blocks[next.idx()].instruction_used {
                        let instr = blocks[next.idx()].instructions[next_i];
                        if matches!(instr.instr.real(), Some(Instruction::Nop))
                            && instruction_lineno(&instr) < 0
                        {
                            next_i += 1;
                            continue;
                        }
                        next_loc = instruction_linetable_location(&instr);
                        break;
                    }
                    if lineno == next_loc.line {
                        continue;
                    }
                }
            }
        }

        if dest != src {
            blocks[bi].instructions[dest] = blocks[bi].instructions[src];
        }
        dest += 1;
        prev_lineno = lineno;
    }

    debug_assert!(dest <= instr_count);
    let num_removed = instr_count - dest;
    blocks[bi].instruction_used = dest;
    Ok(num_removed)
}

/// flowgraph.c remove_redundant_nops
#[allow(clippy::unnecessary_wraps)]
fn remove_redundant_nops(blocks: &mut Blocks) -> crate::InternalResult<usize> {
    let mut changes = 0;
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let next = blocks[current.idx()].next;
        let change = basicblock_remove_redundant_nops(blocks, current)?;
        changes += change;
        current = next;
    }
    Ok(changes)
}

/// flowgraph.c no_redundant_nops
#[cfg(debug_assertions)]
fn no_redundant_nops(blocks: &mut Blocks) -> bool {
    matches!(remove_redundant_nops(blocks), Ok(0))
}

/// flowgraph.c remove_redundant_jumps
fn remove_redundant_jumps(blocks: &mut Blocks) -> crate::InternalResult<usize> {
    let mut changes = 0;
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let block_idx = current.idx();
        let Some(last) = basicblock_last_instr(&blocks[block_idx]).copied() else {
            current = blocks[block_idx].next;
            continue;
        };
        debug_assert!(!last.instr.is_assembler());
        if last.instr.is_unconditional_jump() {
            let jump_target = next_nonempty_block(blocks, last.target);
            if jump_target == BlockIdx::NULL {
                return Err(InternalError::MalformedControlFlowGraph);
            }
            let next = next_nonempty_block(blocks, blocks[block_idx].next);
            if jump_target == next {
                changes += 1;
                let last = basicblock_last_instr_mut(&mut blocks[block_idx]).unwrap();
                set_to_nop(last);
            }
        }
        current = blocks[block_idx].next;
    }
    Ok(changes)
}

/// flowgraph.c no_redundant_jumps
#[cfg(debug_assertions)]
fn no_redundant_jumps(blocks: &Blocks) -> bool {
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let block = &blocks[current.idx()];
        if let Some(last) = basicblock_last_instr(block)
            && last.instr.is_unconditional_jump()
        {
            let next = next_nonempty_block(blocks, block.next);
            let jump_target = next_nonempty_block(blocks, last.target);
            if jump_target == next {
                assert!(next != BlockIdx::NULL);
                if instruction_lineno(last)
                    == instruction_lineno(&blocks[next.idx()].instructions[0])
                {
                    assert_ne!(
                        instruction_lineno(last),
                        instruction_lineno(&blocks[next.idx()].instructions[0]),
                        "redundant jump has same line as fallthrough target"
                    );
                    return false;
                }
            }
        }
        current = block.next;
    }
    true
}

fn remove_redundant_nops_and_jumps(blocks: &mut Blocks) -> crate::InternalResult<()> {
    loop {
        // Convergence is guaranteed because the number of redundant jumps and
        // nops only decreases.
        let removed_nops = remove_redundant_nops(blocks)?;
        let removed_jumps = remove_redundant_jumps(blocks)?;
        if removed_nops + removed_jumps == 0 {
            break;
        }
    }
    Ok(())
}

fn blocks_new_block(blocks: &mut Blocks) -> crate::InternalResult<BlockIdx> {
    blocks
        .try_reserve(1)
        .map_err(|_| InternalError::MalformedControlFlowGraph)?;
    let block_idx = BlockIdx(
        blocks
            .len()
            .to_u32()
            .ok_or(InternalError::MalformedControlFlowGraph)?,
    );
    blocks.push(Block::default());
    Ok(block_idx)
}

/// flowgraph.c struct _PyCfgBuilder
struct CfgBuilder {
    blocks: Blocks,
    entry: BlockIdx,
    block_list: BlockIdx,
    current: BlockIdx,
    current_label: InstructionSequenceLabel,
}

/// flowgraph.c cfg_builder_new_block
fn cfg_builder_new_block(g: &mut CfgBuilder) -> crate::InternalResult<BlockIdx> {
    let block = blocks_new_block(&mut g.blocks)?;
    g.blocks[block.idx()].allocation_next = g.block_list;
    g.blocks[block.idx()].cpython_label = InstructionSequenceLabel::NO_LABEL;
    g.block_list = block;
    Ok(block)
}

/// flowgraph.c cfg_builder_use_next_block
fn cfg_builder_use_next_block(g: &mut CfgBuilder, block: BlockIdx) -> BlockIdx {
    debug_assert!(block != BlockIdx::NULL);
    g.blocks[g.current.idx()].next = block;
    g.current = block;
    block
}

/// flowgraph.c init_cfg_builder
fn init_cfg_builder(g: &mut CfgBuilder) -> crate::InternalResult<()> {
    g.block_list = BlockIdx::NULL;
    let block = cfg_builder_new_block(g)?;
    g.entry = block;
    g.current = block;
    g.current_label = InstructionSequenceLabel::NO_LABEL;
    Ok(())
}

/// flowgraph.c _PyCfgBuilder_New
fn cfg_builder_new() -> crate::InternalResult<CfgBuilder> {
    let mut builder = CfgBuilder {
        blocks: Blocks::default(),
        entry: BlockIdx::NULL,
        block_list: BlockIdx::NULL,
        current: BlockIdx::NULL,
        current_label: InstructionSequenceLabel::NO_LABEL,
    };
    init_cfg_builder(&mut builder)?;
    Ok(builder)
}

/// flowgraph.c cfg_builder_current_block_is_terminated
fn cfg_builder_current_block_is_terminated(g: &mut CfgBuilder) -> bool {
    let block = &mut g.blocks[g.current.idx()];
    let last = basicblock_last_instr(block).copied();
    if last.is_some_and(|last| last.instr.is_terminator()) {
        return true;
    }
    if is_label(g.current_label) {
        if last.is_some() || is_label(block.cpython_label) {
            return true;
        }
        block.cpython_label = g.current_label;
        g.current_label = InstructionSequenceLabel::NO_LABEL;
    }
    false
}

/// flowgraph.c cfg_builder_maybe_start_new_block
fn cfg_builder_maybe_start_new_block(g: &mut CfgBuilder) -> crate::InternalResult<()> {
    if cfg_builder_current_block_is_terminated(g) {
        let block = cfg_builder_new_block(g)?;
        g.blocks[block.idx()].cpython_label = g.current_label;
        g.current_label = InstructionSequenceLabel::NO_LABEL;
        cfg_builder_use_next_block(g, block);
    }
    Ok(())
}

/// flowgraph.c _PyCfgBuilder_UseLabel
fn cfg_builder_use_label(
    g: &mut CfgBuilder,
    label_id: InstructionSequenceLabel,
) -> crate::InternalResult<()> {
    g.current_label = label_id;
    cfg_builder_maybe_start_new_block(g)
}

/// flowgraph.c _PyCfgBuilder_Addop
fn cfg_builder_addop(g: &mut CfgBuilder, info: InstructionInfo) -> crate::InternalResult<()> {
    cfg_builder_maybe_start_new_block(g)?;
    basicblock_addop(&mut g.blocks[g.current], info)
}

/// flowgraph.c cfg_builder_check
fn cfg_builder_check(g: &CfgBuilder) -> bool {
    debug_assert!(g.entry != BlockIdx::NULL);
    debug_assert!(g.blocks[g.entry.idx()].instruction_used != 0);
    let mut block = g.block_list;
    while block != BlockIdx::NULL {
        debug_assert!(block.idx() < g.blocks.len());
        let block_ref = &g.blocks[block.idx()];
        let has_instr_array = block_ref.instruction_allocation > 0;
        if has_instr_array {
            debug_assert!(block_ref.instruction_allocation > 0);
            debug_assert_eq!(
                block_ref.instructions.len(),
                block_ref.instruction_allocation
            );
            debug_assert!(block_ref.instruction_allocation >= block_ref.instruction_used);
        } else {
            debug_assert_eq!(block_ref.instruction_used, 0);
            debug_assert_eq!(block_ref.instruction_allocation, 0);
        }
        block = block_ref.allocation_next;
    }
    true
}

/// flowgraph.c _PyCfgBuilder_CheckSize
fn cfg_builder_check_size(g: &CfgBuilder) -> crate::InternalResult<()> {
    debug_assert!(g.entry != BlockIdx::NULL);
    debug_assert!(g.block_list != BlockIdx::NULL);
    debug_assert!(g.current != BlockIdx::NULL);
    let mut nblocks = 0usize;
    let mut block = g.block_list;
    while block != BlockIdx::NULL {
        debug_assert!(block.idx() < g.blocks.len());
        nblocks += 1;
        block = g.blocks[block.idx()].allocation_next;
    }
    debug_assert_eq!(nblocks, g.blocks.len());
    if nblocks > usize::MAX / core::mem::size_of::<usize>() {
        return Err(InternalError::MalformedControlFlowGraph);
    }
    Ok(())
}

/// flowgraph.c translate_jump_labels_to_targets
fn translate_jump_labels_to_targets(blocks: &mut Blocks) -> crate::InternalResult<()> {
    let max_label = get_max_label(blocks);
    let label_count = (max_label + 1) as usize;
    if label_count > usize::MAX / core::mem::size_of::<usize>() {
        return Err(InternalError::MalformedControlFlowGraph);
    }
    let mut label_to_block = Vec::new();
    vec_try_reserve_exact(&mut label_to_block, label_count)?;
    label_to_block.resize(label_count, BlockIdx::NULL);

    let mut block_idx = BlockIdx(0);
    while block_idx != BlockIdx::NULL {
        let block = &blocks[block_idx];
        if is_label(block.cpython_label) {
            let label_id = block.cpython_label;
            debug_assert!(label_id.0 <= max_label);
            label_to_block[label_id.idx()] = block_idx;
        }
        block_idx = block.next;
    }

    block_idx = BlockIdx(0);
    while block_idx != BlockIdx::NULL {
        let next = blocks[block_idx].next;
        for i in 0..blocks[block_idx].instruction_used {
            let info = &mut blocks[block_idx].instructions[i];
            debug_assert_eq!(info.target, BlockIdx::NULL);
            if info.instr.has_target() {
                let lbl = u32::from(info.arg) as i32;
                debug_assert!(lbl >= 0 && lbl <= max_label);
                let target = label_to_block[lbl as usize];
                debug_assert!(target != BlockIdx::NULL);
                info.target = target;
                debug_assert_eq!(blocks[target].cpython_label, InstructionSequenceLabel(lbl));
            }
        }
        block_idx = next;
    }
    Ok(())
}

/// flowgraph.c _PyCfg_FromInstructionSequence
fn cfg_from_instruction_sequence(
    mut instr_sequence: InstructionSequence,
) -> crate::InternalResult<Blocks> {
    instruction_sequence_apply_label_map(&mut instr_sequence)?;
    let mut builder = cfg_builder_new()?;

    for i in 0..instr_sequence.instr_used {
        instr_sequence.instrs[i].i_target = 0;
    }
    for i in 0..instr_sequence.instr_used {
        if instr_sequence.instrs[i].info.instr.has_target() {
            let target_offset = u32::from(instr_sequence.instrs[i].info.arg) as usize;
            debug_assert!(target_offset < instr_sequence.instr_used);
            instr_sequence.instrs[target_offset].i_target = 1;
        }
    }
    let InstructionSequence {
        instrs,
        instr_used,
        label_map,
        label_map_allocation,
        annotations_code,
        ..
    } = instr_sequence;
    debug_assert!(label_map.is_none());
    debug_assert_eq!(label_map_allocation, 0);

    let mut offset = 0i32;

    let mut i = 0;
    while i < instr_used {
        let mut entry = instrs[i];
        if matches!(
            entry.info.instr.pseudo(),
            Some(PseudoInstruction::AnnotationsPlaceholder)
        ) {
            if let Some(annotations_code) = &annotations_code {
                debug_assert!(annotations_code.label_map.is_none());
                debug_assert_eq!(annotations_code.label_map_allocation, 0);
                for j in 0..annotations_code.instr_used {
                    let ann_entry = annotations_code.instrs[j];
                    debug_assert!(!ann_entry.info.instr.has_target());
                    let mut info = ann_entry.info;
                    info.target = BlockIdx::NULL;
                    cfg_builder_addop(&mut builder, info)?;
                }
                offset += annotations_code.instr_used as i32 - 1;
            } else {
                offset -= 1;
            }
            i += 1;
            continue;
        }

        if entry.i_target != 0 {
            let label_id = i as i32 + offset;
            let label = InstructionSequenceLabel(label_id);
            cfg_builder_use_label(&mut builder, label)?;
        }

        let opcode = entry.info.instr;
        let mut oparg = entry.info.arg;
        if opcode.has_target() {
            let target_offset = u32::from(oparg) as i32 + offset;
            debug_assert!(target_offset >= 0);
            oparg = OpArg::new(target_offset as u32);
        }
        entry.info.instr = opcode;
        entry.info.arg = oparg;
        entry.info.target = BlockIdx::NULL;
        cfg_builder_addop(&mut builder, entry.info)?;
        i += 1;
    }

    cfg_builder_check_size(&builder)?;
    debug_assert!(cfg_builder_check(&builder));
    Ok(builder.blocks)
}

/// flowgraph.c maybe_push
fn maybe_push(
    blocks: &mut Blocks,
    worklist: &mut CfgTraversalStack,
    block: BlockIdx,
    unsafe_mask: u64,
) {
    debug_assert!(block != BlockIdx::NULL);

    let idx = block.idx();
    let both = blocks[idx].unsafe_locals_mask | unsafe_mask;
    if blocks[idx].unsafe_locals_mask != both {
        blocks[idx].unsafe_locals_mask = both;
        if !blocks[idx].visited {
            worklist.push(block);
            blocks[idx].visited = true;
        }
    }
}

/// flowgraph.c scan_block_for_locals
fn scan_block_for_locals(
    blocks: &mut Blocks,
    block_idx: BlockIdx,
    worklist: &mut CfgTraversalStack,
) {
    let idx = block_idx.idx();
    let mut unsafe_mask = blocks[idx].unsafe_locals_mask;
    let instr_count = blocks[idx].instruction_used;

    for i in 0..instr_count {
        let (instr, arg, except_handler) = {
            let info = &blocks[idx].instructions[i];
            (
                info.instr,
                info.arg,
                info.except_handler.map(|eh| eh.handler_block),
            )
        };
        debug_assert!(!matches!(instr.real(), Some(Instruction::ExtendedArg)));

        if let Some(handler_block) = except_handler {
            maybe_push(blocks, worklist, handler_block, unsafe_mask);
        }

        let oparg = u32::from(arg) as usize;
        if oparg >= LOCAL_UNSAFE_MASK_BITS {
            continue;
        }

        let bit = 1u64 << oparg;
        match instr {
            AnyInstruction::Real(
                Instruction::DeleteFast { .. } | Instruction::LoadFastAndClear { .. },
            )
            | AnyInstruction::Pseudo(PseudoInstruction::StoreFastMaybeNull { .. }) => {
                unsafe_mask |= bit;
            }
            AnyInstruction::Real(Instruction::StoreFast { .. }) => {
                unsafe_mask &= !bit;
            }
            AnyInstruction::Real(Instruction::LoadFastCheck { .. }) => {
                // If this doesn't raise, then the local is defined.
                unsafe_mask &= !bit;
            }
            AnyInstruction::Real(Instruction::LoadFast { .. }) => {
                if unsafe_mask & bit != 0 {
                    blocks[idx].instructions[i].instr = Opcode::LoadFastCheck.into();
                }
                unsafe_mask &= !bit;
            }
            _ => {}
        }
    }

    let next = blocks[idx].next;
    if next != BlockIdx::NULL && bb_has_fallthrough(&blocks[idx]) {
        maybe_push(blocks, worklist, next, unsafe_mask);
    }

    let last = basicblock_last_instr(&blocks[idx]).copied();
    if let Some(last) = last
        && is_jump(&last)
    {
        let target = last.target;
        debug_assert!(target != BlockIdx::NULL);
        maybe_push(blocks, worklist, target, unsafe_mask);
    }
}

/// flowgraph.c fast_scan_many_locals
fn fast_scan_many_locals(blocks: &mut Blocks, nlocals: usize) -> crate::InternalResult<()> {
    debug_assert!(nlocals > LOCAL_UNSAFE_MASK_BITS);
    let mut states = Vec::new();
    states
        .try_reserve_exact(nlocals - LOCAL_UNSAFE_MASK_BITS)
        .map_err(|_| InternalError::MalformedControlFlowGraph)?;
    states.resize(nlocals - LOCAL_UNSAFE_MASK_BITS, 0usize);
    let mut blocknum = 0usize;
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        blocknum += 1;
        for i in 0..blocks[current.idx()].instruction_used {
            let info = &mut blocks[current.idx()].instructions[i];
            debug_assert!(!matches!(info.instr.real(), Some(Instruction::ExtendedArg)));
            let arg = u32::from(info.arg) as usize;
            if arg < LOCAL_UNSAFE_MASK_BITS {
                continue;
            }
            debug_assert!(arg >= LOCAL_UNSAFE_MASK_BITS);
            match info.instr {
                AnyInstruction::Real(
                    Instruction::DeleteFast { .. } | Instruction::LoadFastAndClear { .. },
                )
                | AnyInstruction::Pseudo(PseudoInstruction::StoreFastMaybeNull { .. }) => {
                    debug_assert!(arg < nlocals);
                    states[arg - LOCAL_UNSAFE_MASK_BITS] = blocknum - 1;
                }
                AnyInstruction::Real(Instruction::StoreFast { .. }) => {
                    debug_assert!(arg < nlocals);
                    states[arg - LOCAL_UNSAFE_MASK_BITS] = blocknum;
                }
                AnyInstruction::Real(Instruction::LoadFast { .. }) => {
                    debug_assert!(arg < nlocals);
                    if states[arg - LOCAL_UNSAFE_MASK_BITS] != blocknum {
                        info.instr = Opcode::LoadFastCheck.into();
                    }
                    states[arg - LOCAL_UNSAFE_MASK_BITS] = blocknum;
                }
                _ => {}
            }
        }
        current = blocks[current.idx()].next;
    }
    Ok(())
}

/// flowgraph.c add_checks_for_loads_of_uninitialized_variables
fn add_checks_for_loads_of_uninitialized_variables(
    blocks: &mut Blocks,
    mut nlocals: usize,
    nparams: usize,
) -> crate::InternalResult<()> {
    if nlocals == 0 {
        return Ok(());
    }

    if nlocals > LOCAL_UNSAFE_MASK_BITS {
        fast_scan_many_locals(blocks, nlocals)?;
        nlocals = LOCAL_UNSAFE_MASK_BITS;
    }

    let mut worklist = blocks.make_cfg_traversal_stack()?;
    let mut start_mask = 0u64;
    for i in nparams..nlocals {
        start_mask |= 1u64 << i;
    }
    maybe_push(blocks, &mut worklist, BlockIdx(0), start_mask);

    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        scan_block_for_locals(blocks, current, &mut worklist);
        current = blocks[current.idx()].next;
    }

    while let Some(block_idx) = worklist.pop() {
        blocks[block_idx.idx()].visited = false;
        scan_block_for_locals(blocks, block_idx, &mut worklist);
    }
    Ok(())
}

/// Follow chain of empty blocks to find first non-empty block.
fn next_nonempty_block(blocks: &Blocks, mut idx: BlockIdx) -> BlockIdx {
    while idx != BlockIdx::NULL && blocks[idx].instruction_used == 0 {
        idx = blocks[idx].next;
    }
    idx
}

fn instruction_lineno(instr: &InstructionInfo) -> i32 {
    match instr.lineno_override {
        Some(LINE_ONLY_LOCATION_OVERRIDE) | None => instr.location.line.get() as i32,
        Some(lineno) => lineno,
    }
}

fn instruction_is_no_location(instr: &InstructionInfo) -> bool {
    instruction_lineno(instr) == NO_LOCATION_OVERRIDE
}

/// flowgraph.c basicblock_nofallthrough
fn basicblock_nofallthrough(block: &Block) -> bool {
    let last = basicblock_last_instr(block);
    last.is_some_and(|last| last.instr.is_scope_exit() || last.instr.is_unconditional_jump())
}

/// flowgraph.c BB_NO_FALLTHROUGH
fn bb_no_fallthrough(block: &Block) -> bool {
    basicblock_nofallthrough(block)
}

/// flowgraph.c BB_HAS_FALLTHROUGH
fn bb_has_fallthrough(block: &Block) -> bool {
    !bb_no_fallthrough(block)
}

/// flowgraph.c add_checks_for_loads_of_uninitialized_variables uses uint64_t masks.
const LOCAL_UNSAFE_MASK_BITS: usize = 64;

/// flowgraph.c MAX_COPY_SIZE
const MAX_COPY_SIZE: usize = 4;

/// flowgraph.c is_jump
fn is_jump(instr: &InstructionInfo) -> bool {
    instr.instr.has_jump()
}

/// flowgraph.c is_block_push
fn is_block_push(instr: &InstructionInfo) -> bool {
    instr.instr.is_block_push()
}

/// flowgraph.c basicblock_returns
#[cfg(test)]
fn basicblock_returns(block: &Block) -> bool {
    let last = basicblock_last_instr(block);
    if let Some(last) = last {
        matches!(last.instr.real(), Some(Instruction::ReturnValue))
    } else {
        false
    }
}

/// flowgraph.c basicblock_exits_scope
fn basicblock_exits_scope(block: &Block) -> bool {
    let last = basicblock_last_instr(block);
    last.is_some_and(|last| last.instr.is_scope_exit())
}

/// flowgraph.c is_exit_or_eval_check_without_lineno
fn is_exit_or_eval_check_without_lineno(block: &Block) -> bool {
    if basicblock_exits_scope(block) || basicblock_has_eval_break(block) {
        basicblock_has_no_lineno(block)
    } else {
        false
    }
}

/// flowgraph.c basicblock_has_eval_break
fn basicblock_has_eval_break(block: &Block) -> bool {
    let mut i = 0;
    while i < block.instruction_used {
        if block.instructions[i].instr.has_eval_break() {
            return true;
        }
        i += 1;
    }
    false
}

/// flowgraph.c basicblock_has_no_lineno
fn basicblock_has_no_lineno(block: &Block) -> bool {
    let mut i = 0;
    while i < block.instruction_used {
        if instruction_lineno(&block.instructions[i]) >= 0 {
            return false;
        }
        i += 1;
    }
    true
}

/// flowgraph.c get_max_label
fn get_max_label(blocks: &Blocks) -> i32 {
    let mut lbl = -1;
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let cpython_label = blocks[current.idx()].cpython_label;
        lbl = lbl.max(cpython_label.0);
        current = blocks[current.idx()].next;
    }
    lbl
}

/// flowgraph.c make_except_stack
#[allow(clippy::unnecessary_wraps)]
fn make_except_stack() -> crate::InternalResult<CfgExceptStack> {
    let handlers = [BlockIdx::NULL; CO_MAXBLOCKS + 2];
    debug_assert_eq!(handlers[0], BlockIdx::NULL);
    Ok(CfgExceptStack { handlers, depth: 0 })
}

/// flowgraph.c copy_except_stack
#[allow(clippy::unnecessary_wraps)]
fn copy_except_stack(stack: &CfgExceptStack) -> crate::InternalResult<CfgExceptStack> {
    debug_assert!(stack.depth <= CO_MAXBLOCKS + 1);
    Ok(CfgExceptStack {
        handlers: stack.handlers,
        depth: stack.depth,
    })
}

/// flowgraph.c except_stack_top
fn except_stack_top(stack: &CfgExceptStack, blocks: &Blocks) -> Option<ExceptHandlerInfo> {
    debug_assert!(stack.depth <= CO_MAXBLOCKS + 1);
    let handler_block = stack.handlers[stack.depth];
    if handler_block == BlockIdx::NULL {
        return None;
    }
    Some(ExceptHandlerInfo {
        handler_block,
        preserve_lasti: blocks[handler_block].preserve_lasti,
    })
}

/// flowgraph.c push_except_block
fn push_except_block(
    stack: &mut CfgExceptStack,
    setup: InstructionInfo,
    blocks: &mut Blocks,
) -> Option<ExceptHandlerInfo> {
    debug_assert!(is_block_push(&setup));
    let instr = setup.instr;
    let target = setup.target;
    debug_assert!(target != BlockIdx::NULL);
    if matches!(
        instr.pseudo(),
        Some(PseudoInstruction::SetupWith { .. } | PseudoInstruction::SetupCleanup { .. })
    ) {
        blocks[target].preserve_lasti = true;
    }
    debug_assert!(stack.depth <= CO_MAXBLOCKS);
    stack.depth += 1;
    stack.handlers[stack.depth] = target;
    debug_assert!(stack.depth <= CO_MAXBLOCKS + 1);
    except_stack_top(stack, blocks)
}

/// flowgraph.c pop_except_block
fn pop_except_block(stack: &mut CfgExceptStack, blocks: &Blocks) -> Option<ExceptHandlerInfo> {
    debug_assert!(stack.depth > 0);
    stack.depth -= 1;
    debug_assert!(stack.depth <= CO_MAXBLOCKS);
    except_stack_top(stack, blocks)
}

pub(crate) fn label_exception_targets(blocks: &mut Blocks) -> crate::InternalResult<()> {
    let mut todo = blocks.make_cfg_traversal_stack()?;

    todo.push(BlockIdx(0));
    blocks[0].visited = true;
    blocks[0].except_stack = Some(make_except_stack()?);

    while let Some(block_idx) = todo.pop() {
        let bi = block_idx.idx();
        debug_assert!(blocks[bi].visited);
        let mut stack = Some(
            blocks[bi]
                .except_stack
                .take()
                .expect("visited exception block has an except stack"),
        );
        let mut handler = except_stack_top(stack.as_ref().expect("active exception stack"), blocks);
        let mut last_yield_except_depth: i32 = -1;
        let mut stack_transferred = false;

        let instr_count = blocks[bi].instruction_used;
        for i in 0..instr_count {
            let info = blocks[bi].instructions[i];
            let instr = info.instr;
            let target = info.target;
            let arg = info.arg;

            if is_block_push(&info) {
                debug_assert!(target != BlockIdx::NULL);
                if !blocks[target].visited {
                    blocks[target].except_stack = Some(copy_except_stack(
                        stack.as_ref().expect("active exception stack"),
                    )?);
                    todo.push(target);
                    blocks[target].visited = true;
                }
                handler = push_except_block(
                    stack.as_mut().expect("active exception stack"),
                    info,
                    blocks,
                );
            } else if instr.is_pop_block() {
                handler = pop_except_block(stack.as_mut().expect("active exception stack"), blocks);
                set_to_nop(&mut blocks[bi].instructions[i]);
            } else if is_jump(&blocks[bi].instructions[i]) {
                blocks[bi].instructions[i].except_handler = handler;
                debug_assert_eq!(i, instr_count - 1);

                // CPython label_exception_targets(): copy the except stack
                // when this block can also fall through, otherwise transfer it
                // to the jump target.
                debug_assert!(target != BlockIdx::NULL);
                if !blocks[target].visited {
                    if bb_has_fallthrough(&blocks[bi]) {
                        blocks[target].except_stack = Some(copy_except_stack(
                            stack.as_ref().expect("active exception stack"),
                        )?);
                    } else {
                        blocks[target].except_stack = stack.take();
                        stack_transferred = true;
                        todo.push(target);
                        blocks[target].visited = true;
                        break;
                    }
                    todo.push(target);
                    blocks[target].visited = true;
                }
            } else if matches!(instr.real(), Some(Instruction::YieldValue { .. })) {
                blocks[bi].instructions[i].except_handler = handler;
                last_yield_except_depth =
                    stack.as_ref().expect("active exception stack").depth as i32;
            } else if let Some(Instruction::Resume { context: _ }) = instr.real() {
                blocks[bi].instructions[i].except_handler = handler;
                let resume_arg = u32::from(arg);
                if resume_arg != u32::from(oparg::ResumeLocation::AtFuncStart) {
                    debug_assert!(last_yield_except_depth >= 0);
                    if last_yield_except_depth == 1 {
                        blocks[bi].instructions[i].arg =
                            OpArg::new(resume_arg | oparg::ResumeContext::DEPTH1_MASK);
                    }
                    last_yield_except_depth = -1;
                }
            } else {
                blocks[bi].instructions[i].except_handler = handler;
            }
        }

        let next = blocks[bi].next;
        if !stack_transferred && bb_has_fallthrough(&blocks[bi]) {
            debug_assert!(next != BlockIdx::NULL);
            if next != BlockIdx::NULL && !blocks[next].visited {
                blocks[next].except_stack = stack.take();
                todo.push(next);
                blocks[next].visited = true;
            }
        }
    }
    #[cfg(debug_assertions)]
    {
        let mut block_idx = BlockIdx(0);
        while block_idx != BlockIdx::NULL {
            let block = &blocks[block_idx];
            debug_assert!(block.except_stack.is_none());
            block_idx = block.next;
        }
    }
    Ok(())
}

/// Convert remaining pseudo ops to real instructions or NOP.
/// flowgraph.c convert_pseudo_ops
pub(crate) fn convert_pseudo_ops(blocks: &mut Blocks) -> crate::InternalResult<()> {
    let mut block_idx = BlockIdx(0);
    while block_idx != BlockIdx::NULL {
        let next = blocks[block_idx.idx()].next;
        let block = &mut blocks[block_idx.idx()];
        for i in 0..block.instruction_used {
            let info = &mut block.instructions[i];
            if is_block_push(info) {
                set_to_nop(info);
            } else if matches!(
                info.instr.pseudo(),
                Some(PseudoInstruction::LoadClosure { .. })
            ) {
                debug_assert!(is_pseudo_target(
                    PseudoOpcode::LoadClosure,
                    Opcode::LoadFast
                ));
                info.instr = Opcode::LoadFast.into();
            } else if matches!(
                info.instr.pseudo(),
                Some(PseudoInstruction::StoreFastMaybeNull { .. })
            ) {
                debug_assert!(is_pseudo_target(
                    PseudoOpcode::StoreFastMaybeNull,
                    Opcode::StoreFast
                ));
                info.instr = Opcode::StoreFast.into();
            }
        }
        block_idx = next;
    }
    // CPython flowgraph.c::convert_pseudo_ops() finishes by calling
    // remove_redundant_nops_and_jumps().
    remove_redundant_nops_and_jumps(blocks)
}

/// flowgraph.c build_cellfixedoffsets
#[allow(clippy::needless_range_loop)]
pub(crate) fn build_cellfixedoffsets(
    metadata: &CodeUnitMetadata,
) -> crate::InternalResult<Vec<i32>> {
    let nlocals = metadata.varnames.len();
    let ncellvars = metadata.cellvars.len();
    let nfreevars = metadata.freevars.len();
    let noffsets = ncellvars + nfreevars;
    let mut fixed = Vec::new();
    vec_try_reserve_exact(&mut fixed, noffsets)?;
    fixed.resize(noffsets, 0);
    for i in 0..noffsets {
        fixed[i] = (nlocals + i) as i32;
    }
    for oldindex in 0..ncellvars {
        let varname = metadata
            .cellvars
            .get_index(oldindex)
            .expect("cellvar index is in range");
        if let Some(varindex) = metadata.varnames.get_index_of(varname) {
            let argoffset = varindex as i32;
            fixed[oldindex] = argoffset;
        }
    }
    Ok(fixed)
}

/// flowgraph.c fix_cell_offsets
#[allow(clippy::needless_range_loop)]
pub(crate) fn fix_cell_offsets(
    metadata: &CodeUnitMetadata,
    blocks: &mut Blocks,
    cellfixedoffsets: &mut [i32],
) -> usize {
    let nlocals = metadata.varnames.len();
    let ncellvars = metadata.cellvars.len();
    let nfreevars = metadata.freevars.len();
    let noffsets = ncellvars + nfreevars;
    debug_assert_eq!(cellfixedoffsets.len(), noffsets);

    let mut numdropped = 0usize;
    for i in 0..noffsets {
        if cellfixedoffsets[i] == (i + nlocals) as i32 {
            cellfixedoffsets[i] -= numdropped as i32;
        } else {
            numdropped += 1;
        }
    }

    let mut block_idx = BlockIdx(0);
    while block_idx != BlockIdx::NULL {
        let next = blocks[block_idx.idx()].next;
        let block = &mut blocks[block_idx.idx()];
        for i in 0..block.instruction_used {
            let inst = &mut block.instructions[i];
            debug_assert!(
                !matches!(inst.instr.real(), Some(Instruction::ExtendedArg)),
                "fix_cell_offsets is called before extended args are generated"
            );
            let oldoffset = u32::from(inst.arg) as i32;
            match inst.instr {
                AnyInstruction::Real(
                    Instruction::MakeCell { .. }
                    | Instruction::LoadDeref { .. }
                    | Instruction::StoreDeref { .. }
                    | Instruction::DeleteDeref { .. }
                    | Instruction::LoadFromDictOrDeref { .. },
                )
                | AnyInstruction::Pseudo(PseudoInstruction::LoadClosure { .. }) => {
                    debug_assert!(oldoffset >= 0);
                    debug_assert!(oldoffset < noffsets as i32);
                    let fixed_offset = cellfixedoffsets[oldoffset as usize];
                    debug_assert!(fixed_offset >= 0);
                    inst.arg = OpArg::new(fixed_offset as u32);
                }
                _ => {}
            }
        }
        block_idx = next;
    }
    numdropped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_location(line: u32) -> SourceLocation {
        SourceLocation {
            line: OneIndexed::new(line as usize).expect("valid line number"),
            character_offset: OneIndexed::MIN,
        }
    }

    fn test_instr(instr: Instruction, line: u32) -> InstructionInfo {
        InstructionInfo {
            instr: instr.into(),
            arg: OpArg::new(0),
            target: BlockIdx::NULL,
            location: test_location(line),
            end_location: test_location(line),
            except_handler: None,
            lineno_override: None,
        }
    }

    fn test_jump(target: BlockIdx, line: u32) -> InstructionInfo {
        let mut instr = test_instr(Instruction::Nop, line);
        instr.instr = PseudoOpcode::Jump.into();
        instr.target = target;
        instr
    }

    fn test_cond_jump(target: BlockIdx, line: u32) -> InstructionInfo {
        let mut instr = test_instr(Instruction::Nop, line);
        instr.instr = PseudoOpcode::JumpIfFalse.into();
        instr.target = target;
        instr
    }

    fn test_block_push(block: &mut Block, info: InstructionInfo) {
        let off = basicblock_next_instr(block).expect("test block instruction slot");
        block.instructions[off] = info;
    }

    fn test_code_info(block: Block) -> CodeInfo {
        CodeInfo {
            flags: CodeFlags::empty(),
            source_path: "source_path".to_owned(),
            private: None,
            blocks: Blocks::from([block]),
            current_block: BlockIdx::new(0),
            instr_sequence: instruction_sequence_new(),
            instr_sequence_label_map: InstructionSequenceLabelMap::new(),
            annotations_instr_sequence: None,
            metadata: CodeUnitMetadata {
                name: "<module>".to_owned(),
                qualname: Some("<module>".to_owned()),
                consts: Default::default(),
                names: IndexSet::default(),
                varnames: IndexSet::default(),
                cellvars: IndexSet::default(),
                freevars: IndexSet::default(),
                fast_hidden: IndexMap::default(),
                fast_hidden_final: IndexSet::default(),
                argcount: 0,
                posonlyargcount: 0,
                kwonlyargcount: 0,
                firstlineno: OneIndexed::MIN,
            },
            static_attributes: None,
            in_inlined_comp: false,
            fblock: Vec::new(),
            symbol_table_index: 0,
            nparams: 0,
            in_conditional_block: 0,
            next_conditional_annotation_index: 0,
        }
    }

    #[test]
    fn get_stack_effects_rejects_cpython_deopt_opcodes() {
        match get_stack_effects(Instruction::BinaryOpAddInt.into(), OpArg::new(0), 0) {
            Err(InternalError::InvalidStackEffect) => {}
            Err(err) => panic!("unexpected stack-effect error: {err}"),
            Ok(_) => panic!("CPython get_stack_effects rejects specialized deopt opcodes"),
        }
    }

    #[test]
    fn instruction_sequence_label_shadow_preserves_cpython_offset_aliases() {
        let mut seq = instruction_sequence_new();
        let mut labels = InstructionSequenceLabelMap::new();
        instruction_sequence_label_map_push_unmapped_label(&mut labels, &mut seq).unwrap();
        instruction_sequence_label_map_push_unmapped_label(&mut labels, &mut seq).unwrap();
        assert_eq!(
            labels.cpython_block_by_label.len(),
            INITIAL_INSTR_SEQUENCE_LABELS_MAP_SIZE
        );

        let first = BlockIdx::new(1);
        let second = BlockIdx::new(2);
        assert_ne!(
            instruction_sequence_label_map_label_for_block(&labels, first),
            instruction_sequence_label_map_label_for_block(&labels, second)
        );

        // CPython `_PyInstructionSequence_UseLabel()` can map consecutive
        // labels to the same instruction offset. The codegen CFG shadow must
        // resolve the later block label to the block owning that shared offset.
        instruction_sequence_label_map_use_label_at_block(&mut labels, &mut seq, second, first)
            .unwrap();
        assert_eq!(
            instruction_sequence_label_map_resolve_label(&labels, first),
            first
        );
        assert_eq!(
            instruction_sequence_label_map_resolve_label(&labels, second),
            first
        );
    }

    #[test]
    fn except_stack_tracks_cpython_depth_and_handler_slots() {
        let mut stack = make_except_stack().unwrap();
        assert_eq!(stack.depth, 0);
        assert_eq!(stack.handlers.len(), CO_MAXBLOCKS + 2);
        assert_eq!(stack.handlers[0], BlockIdx::NULL);

        let mut blocks = Blocks::from([Block::default(), Block::default()]);
        assert!(except_stack_top(&stack, &blocks).is_none());

        let setup = InstructionInfo {
            instr: PseudoOpcode::SetupWith.into(),
            arg: OpArg::new(0),
            target: BlockIdx::new(1),
            location: SourceLocation::default(),
            end_location: SourceLocation::default(),
            except_handler: None,
            lineno_override: None,
        };
        let handler = push_except_block(&mut stack, setup, &mut blocks).unwrap();
        assert_eq!(stack.depth, 1);
        assert_eq!(stack.handlers[1], BlockIdx::new(1));
        assert_eq!(handler.handler_block, BlockIdx::new(1));
        assert!(handler.preserve_lasti);
        assert!(blocks[1].preserve_lasti);

        let copy = copy_except_stack(&stack).unwrap();
        assert_eq!(copy.depth, stack.depth);
        assert_eq!(copy.handlers, stack.handlers);

        assert!(pop_except_block(&mut stack, &blocks).is_none());
        assert_eq!(stack.depth, 0);
    }

    #[test]
    fn ref_stack_tracks_cpython_size_and_allocated_refs() {
        let mut stack = RefStack {
            refs: Vec::new(),
            size: 0,
            capacity: 0,
        };
        ref_stack_push(&mut stack, Ref { instr: 7, local: 3 }).unwrap();
        assert_eq!(stack.size, 1);
        assert_eq!(stack.capacity, 32);
        assert_eq!(stack.refs.len(), 32);
        assert_eq!(ref_stack_at(&stack, 0).instr, 7);
        assert_eq!(ref_stack_at(&stack, 0).local, 3);

        ref_stack_clear(&mut stack);
        assert_eq!(stack.size, 0);
        assert_eq!(stack.capacity, 32);
        assert_eq!(stack.refs.len(), 32);

        ref_stack_push(
            &mut stack,
            Ref {
                instr: DUMMY_INSTR,
                local: NOT_LOCAL,
            },
        )
        .unwrap();
        assert_eq!(stack.size, 1);
        assert_eq!(ref_stack_pop(&mut stack).instr, DUMMY_INSTR);
        assert_eq!(stack.size, 0);
    }

    #[test]
    fn cfg_traversal_stack_resets_visited_and_allocates_for_blocks() {
        let mut blocks = Blocks::from([Block::default(), Block::default()]);
        blocks[0].next = BlockIdx::new(1);
        blocks[0].visited = true;
        blocks[1].visited = true;

        let mut stack = blocks.make_cfg_traversal_stack().unwrap();
        assert!(!blocks[0].visited);
        assert!(!blocks[1].visited);
        assert!(stack.capacity() >= 2);
        assert_eq!(stack.pop(), None);

        stack.push(BlockIdx::new(1));
        stack.push(BlockIdx::new(0));
        assert_eq!(stack.pop(), Some(BlockIdx::new(0)));
        assert_eq!(stack.pop(), Some(BlockIdx::new(1)));
        assert_eq!(stack.pop(), None);
    }

    #[test]
    fn instruction_sequence_insert_preserves_cpython_slot_metadata() {
        let handler = InstructionSequenceExceptHandlerInfo {
            h_label: 7,
            start_depth: 3,
            preserve_lasti: 1,
        };
        let mut seq = instruction_sequence_new();
        let entry = instruction_sequence_addop(&mut seq, test_instr(Instruction::Nop, 11)).unwrap();
        entry.except_handler = handler;
        entry.i_target = 1;
        entry.i_offset = 42;

        instruction_sequence_insert_instruction(&mut seq, 0, test_instr(Instruction::PopTop, 12))
            .unwrap();

        // CPython `_PyInstructionSequence_InsertInstruction()` shifts the
        // backing instruction slots, then overwrites only opcode/oparg/loc.
        let inserted = &seq.instrs[0];
        assert!(matches!(
            inserted.info.instr.real(),
            Some(Instruction::PopTop)
        ));
        assert_eq!(inserted.except_handler.h_label, handler.h_label);
        assert_eq!(inserted.except_handler.start_depth, handler.start_depth);
        assert_eq!(
            inserted.except_handler.preserve_lasti,
            handler.preserve_lasti
        );
        assert_eq!(inserted.i_target, 1);
        assert_eq!(inserted.i_offset, 42);
    }

    #[test]
    fn instruction_sequence_tracks_cpython_c_array_allocation() {
        let mut seq = instruction_sequence_new();
        for i in 0..99 {
            instruction_sequence_addop(&mut seq, test_instr(Instruction::Nop, 10 + i)).unwrap();
        }
        assert_eq!(seq.instr_allocation, INITIAL_INSTR_SEQUENCE_SIZE);
        assert_eq!(seq.instrs.len(), seq.instr_allocation);
        assert_eq!(seq.instr_used, 99);

        // CPython calls `_Py_CArray_EnsureCapacity(s_used + 1)`, so the 100th
        // instruction expands a 100-slot array to 200 before returning offset 99.
        instruction_sequence_addop(&mut seq, test_instr(Instruction::Nop, 109)).unwrap();
        assert_eq!(seq.instr_allocation, INITIAL_INSTR_SEQUENCE_SIZE * 2);
        assert_eq!(seq.instrs.len(), seq.instr_allocation);
        assert_eq!(seq.instr_used, 100);
    }

    #[test]
    fn instruction_sequence_label_map_tracks_cpython_c_array_allocation() {
        let mut seq = instruction_sequence_new();
        instruction_sequence_use_label(&mut seq, InstructionSequenceLabel::from_index(1)).unwrap();
        assert_eq!(
            seq.label_map_allocation,
            INITIAL_INSTR_SEQUENCE_LABELS_MAP_SIZE
        );
        assert_eq!(
            seq.label_map.as_ref().expect("label map allocated").len(),
            INITIAL_INSTR_SEQUENCE_LABELS_MAP_SIZE
        );

        // CPython passes the label id itself to `_Py_CArray_EnsureCapacity()`.
        // Label 10 therefore expands the initial 10-slot map to 20.
        instruction_sequence_use_label(&mut seq, InstructionSequenceLabel::from_index(10)).unwrap();
        assert_eq!(
            seq.label_map_allocation,
            INITIAL_INSTR_SEQUENCE_LABELS_MAP_SIZE * 2
        );
    }

    #[test]
    fn basicblock_addop_reuses_cpython_spare_except_handler_slot() {
        let handler = ExceptHandlerInfo {
            handler_block: BlockIdx::new(7),
            preserve_lasti: true,
        };
        let mut block = Block::default();
        let mut stale = test_instr(Instruction::Nop, 11);
        stale.except_handler = Some(handler);
        test_block_push(&mut block, stale);
        basicblock_clear(&mut block);

        basicblock_addop(&mut block, test_instr(Instruction::PopTop, 12))
            .expect("basicblock_addop succeeds");

        // CPython `basicblock_addop()` writes opcode/oparg/target/location into
        // the reused `b_instr[b_iused]` slot, but does not clear `i_except`.
        assert_eq!(block.instruction_used, 1);
        assert_eq!(block.instructions[0].except_handler, Some(handler));
        assert_eq!(block.instructions[0].target, BlockIdx::NULL);
    }

    #[test]
    fn basicblock_next_instr_tracks_cpython_c_array_allocation() {
        let mut block = Block::default();
        for i in 0..15 {
            basicblock_addop(&mut block, test_instr(Instruction::PopTop, 10 + i))
                .expect("basicblock_addop succeeds");
        }
        assert_eq!(block.instruction_allocation, DEFAULT_BLOCK_SIZE);

        // CPython calls `_Py_CArray_EnsureCapacity(b_iused + 1)`, so the 16th
        // instruction expands a 16-slot array to 32 before returning offset 15.
        basicblock_addop(&mut block, test_instr(Instruction::PopTop, 25))
            .expect("basicblock_addop succeeds");
        assert_eq!(block.instruction_allocation, DEFAULT_BLOCK_SIZE * 2);
    }

    #[test]
    fn basicblock_insert_instruction_consumes_spare_without_inheriting_except_handler() {
        let handler = ExceptHandlerInfo {
            handler_block: BlockIdx::new(9),
            preserve_lasti: false,
        };
        let mut block = Block::default();
        test_block_push(&mut block, test_instr(Instruction::Nop, 21));
        let mut stale = test_instr(Instruction::Nop, 22);
        stale.except_handler = Some(handler);
        test_block_push(&mut block, stale);
        block.instruction_used = 1;

        basicblock_insert_instruction(&mut block, 0, test_instr(Instruction::PopTop, 23))
            .expect("basicblock_insert_instruction succeeds");

        // CPython `basicblock_insert_instruction()` also obtains a slot with
        // `basicblock_next_instr()`, then overwrites the inserted position with
        // the provided instruction copy, including its `i_except` value.
        assert_eq!(block.instruction_used, 2);
        assert_eq!(block.instructions[0].except_handler, None);
    }

    #[test]
    fn basicblock_clear_preserves_cpython_spare_slots() {
        let handler = ExceptHandlerInfo {
            handler_block: BlockIdx::new(3),
            preserve_lasti: true,
        };
        let mut block = Block::default();
        let mut stale = test_instr(Instruction::PopTop, 31);
        stale.except_handler = Some(handler);
        test_block_push(&mut block, stale);

        basicblock_clear(&mut block);
        basicblock_addop(&mut block, test_instr(Instruction::Nop, 32))
            .expect("basicblock_addop succeeds");

        // CPython `remove_unreachable()` sets `b_iused = 0` without clearing the
        // backing `b_instr` slot. A later `basicblock_addop()` reuses that slot
        // and does not overwrite `i_except`.
        assert_eq!(block.instruction_used, 1);
        assert_eq!(block.instructions[0].except_handler, Some(handler));
    }

    #[test]
    fn basicblock_clear_reuses_cpython_spare_slots_in_offset_order() {
        let mut block = Block::default();
        for i in 0..3 {
            let mut stale = test_instr(Instruction::Nop, 35 + i);
            stale.except_handler = Some(ExceptHandlerInfo {
                handler_block: BlockIdx::new(i + 1),
                preserve_lasti: false,
            });
            test_block_push(&mut block, stale);
        }

        basicblock_clear(&mut block);
        for i in 0..3 {
            basicblock_addop(&mut block, test_instr(Instruction::PopTop, 38 + i))
                .expect("basicblock_addop succeeds");
        }

        let handlers = block
            .used_instructions()
            .iter()
            .map(|instr| {
                instr
                    .except_handler
                    .expect("reused CPython slot")
                    .handler_block
            })
            .collect::<Vec<_>>();
        assert_eq!(
            handlers,
            [BlockIdx::new(1), BlockIdx::new(2), BlockIdx::new(3)]
        );
    }

    #[test]
    fn basicblock_append_instructions_overwrites_cpython_spare_slot() {
        let handler = ExceptHandlerInfo {
            handler_block: BlockIdx::new(5),
            preserve_lasti: false,
        };
        let mut blocks = Blocks::from([Block::default(), Block::default()]);
        let mut stale = test_instr(Instruction::Nop, 41);
        stale.except_handler = Some(handler);
        test_block_push(&mut blocks[0], stale);
        basicblock_clear(&mut blocks[0]);

        test_block_push(&mut blocks[1], test_instr(Instruction::PopTop, 42));
        blocks
            .basicblock_append_block_instructions(BlockIdx::new(0), BlockIdx::new(1))
            .expect("basicblock_append_block_instructions succeeds");

        // CPython `basicblock_append_instructions()` obtains a slot with
        // `basicblock_next_instr()`, then overwrites it with the copied
        // instruction, including `i_except`.
        assert_eq!(blocks[0].instruction_used, 1);
        assert_eq!(blocks[0].instructions[0].except_handler, None);
    }

    #[test]
    fn instr_set_op0_nop_preserves_cpython_stale_target() {
        let mut info = test_jump(BlockIdx::new(1), 50);
        set_to_nop(&mut info);

        assert_eq!(info.target, BlockIdx::new(1));

        let mut blocks = Blocks::from([Block::default(), Block::default()]);
        test_block_push(&mut blocks[0], info);
        blocks[0].next = BlockIdx::new(1);

        let mut instr_sequence = instruction_sequence_new();
        blocks
            .cfg_to_instruction_sequence(&mut instr_sequence)
            .expect("non-target NOP should ignore stale CPython i_target");
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "target_block != BlockIdx::NULL")]
    fn cfg_to_instruction_sequence_requires_target_for_target_opcodes() {
        let mut block = Block::default();
        test_block_push(&mut block, test_jump(BlockIdx::NULL, 51));
        let mut blocks = Blocks::from([block]);

        let mut instr_sequence = instruction_sequence_new();
        let _ = blocks.cfg_to_instruction_sequence(&mut instr_sequence);
    }

    #[test]
    fn static_swaps_respect_cpython_no_location_line_boundary() {
        let mut block = Block::default();
        let mut swap = test_instr(Opcode::Swap.into(), 60);
        swap.arg = OpArg::new(2);
        let mut store = test_instr(Opcode::StoreFast.into(), 60);
        store.arg = OpArg::new(0);
        let mut pop = test_instr(Instruction::PopTop, 60);
        pop.lineno_override = Some(NO_LOCATION_OVERRIDE);
        for info in [swap, store, pop] {
            test_block_push(&mut block, info);
        }

        apply_static_swaps_block(&mut block).expect("apply_static_swaps_block succeeds");

        // CPython `next_swappable_instruction()` compares `i_loc.lineno`
        // directly, so a following NO_LOCATION swaperand does not match the
        // first swaperand's positive line number.
        assert!(matches!(
            block.instructions[0].instr.real(),
            Some(Instruction::Swap { .. })
        ));
        assert!(matches!(
            block.instructions[1].instr.real(),
            Some(Instruction::StoreFast { .. })
        ));
        assert!(matches!(
            block.instructions[2].instr.real(),
            Some(Instruction::PopTop)
        ));

        let mut block = Block::default();
        let mut swap = test_instr(Opcode::Swap.into(), 70);
        swap.arg = OpArg::new(2);
        let mut store = test_instr(Opcode::StoreFast.into(), 70);
        store.arg = OpArg::new(0);
        store.lineno_override = Some(NO_LOCATION_OVERRIDE);
        let pop = test_instr(Instruction::PopTop, 71);
        for info in [swap, store, pop] {
            test_block_push(&mut block, info);
        }

        apply_static_swaps_block(&mut block).expect("apply_static_swaps_block succeeds");

        // Conversely, when the first swaperand has NO_LOCATION, CPython passes
        // `-1` as the line filter and does not enforce a boundary.
        assert!(matches!(
            block.instructions[0].instr.real_opcode(),
            Some(Opcode::Nop)
        ));
        assert!(matches!(
            block.instructions[1].instr.real_opcode(),
            Some(Opcode::PopTop)
        ));
        assert!(matches!(
            block.instructions[2].instr.real_opcode(),
            Some(Opcode::StoreFast)
        ));
    }

    #[test]
    fn optimize_load_const_tracks_cpython_copy_of_load_const() {
        let mut block = Block::default();
        test_block_push(&mut block, test_instr(Opcode::LoadConst.into(), 80));
        let mut copy = test_instr(Opcode::Copy.into(), 80);
        copy.arg = OpArg::new(1);
        test_block_push(&mut block, copy);
        test_block_push(&mut block, test_instr(Instruction::ToBool, 80));

        let mut code = test_code_info(block);
        let (const_idx, _) = code.metadata.consts.insert_full(ConstantData::Tuple {
            elements: vec![ConstantData::Integer {
                value: BigInt::from(1),
            }],
        });
        code.blocks[0].instructions[0].arg = OpArg::new(const_idx as u32);

        optimize_load_const(&mut code.metadata, &mut code.blocks)
            .expect("optimize_load_const succeeds");

        // CPython `basicblock_optimize_load_const()` keeps the previous
        // LOAD_CONST as the effective opcode for a following `COPY 1`, so the
        // COPY is NOPed and TO_BOOL becomes LOAD_CONST True.
        assert!(matches!(
            code.blocks[0].instructions[0].instr.real(),
            Some(Instruction::LoadConst { .. })
        ));
        assert!(matches!(
            code.blocks[0].instructions[1].instr.real(),
            Some(Instruction::Nop)
        ));
        let load_bool = &code.blocks[0].instructions[2];
        assert!(matches!(
            load_bool.instr.real(),
            Some(Instruction::LoadConst { .. })
        ));
        assert_eq!(
            code.metadata.consts[u32::from(load_bool.arg) as usize],
            ConstantData::Boolean { value: true }
        );
    }

    #[test]
    fn optimize_load_fast_records_no_input_opcode_ref_at_cpython_produced_index() {
        let mut block = Block::default();
        test_block_push(&mut block, test_instr(Opcode::LoadFast.into(), 10));
        test_block_push(&mut block, test_instr(Instruction::GetLen, 10));
        let mut swap = test_instr(Opcode::Swap.into(), 10);
        swap.arg = OpArg::new(2);
        test_block_push(&mut block, swap);
        test_block_push(&mut block, test_instr(Instruction::PopTop, 10));

        let mut code = test_code_info(block);
        code.blocks
            .optimize_load_fast()
            .expect("optimize_load_fast succeeds");

        // CPython `optimize_load_fast()` shadows the outer instruction index in
        // the produced-value loop for GET_LEN, so the produced ref is recorded
        // with index 0 here. The original LOAD_FAST is therefore not considered
        // the consumed producer.
        assert!(matches!(
            code.blocks[0].instructions[0].instr.real(),
            Some(Instruction::LoadFast { .. })
        ));
    }

    #[test]
    fn constant_sequence_loads_use_cpython_opcode_has_const_metadata() {
        let mut metadata = CodeUnitMetadata {
            name: "<module>".to_owned(),
            qualname: Some("<module>".to_owned()),
            consts: Default::default(),
            names: IndexSet::default(),
            varnames: IndexSet::default(),
            cellvars: IndexSet::default(),
            freevars: IndexSet::default(),
            fast_hidden: IndexMap::default(),
            fast_hidden_final: IndexSet::default(),
            argcount: 0,
            posonlyargcount: 0,
            kwonlyargcount: 0,
            firstlineno: OneIndexed::MIN,
        };
        let (left, _) = metadata
            .consts
            .insert_full(ConstantData::Str { value: "a".into() });
        let (right, _) = metadata
            .consts
            .insert_full(ConstantData::Str { value: "b".into() });

        let mut immortal = test_instr(Instruction::Nop, 90);
        immortal.instr = Opcode::LoadConstImmortal.into();
        immortal.arg = OpArg::new(left as u32);
        let mut mortal = test_instr(Instruction::Nop, 90);
        mortal.instr = Opcode::LoadConstMortal.into();
        mortal.arg = OpArg::new(right as u32);
        let mut build = test_instr(Opcode::BuildTuple.into(), 90);
        build.arg = OpArg::new(2);
        let mut block = Block::default();
        for info in [immortal, mortal, build] {
            test_block_push(&mut block, info);
        }

        assert!(
            fold_tuple_of_constants(&mut metadata, &mut block, 2)
                .expect("fold_tuple_of_constants succeeds")
        );

        // CPython `loads_const()` accepts every `OPCODE_HAS_CONST` opcode, not
        // just canonical LOAD_CONST, so LOAD_CONST_IMMORTAL/MORTAL participate
        // in constant-sequence folding.
        assert!(matches!(
            block.instructions[0].instr.real(),
            Some(Instruction::Nop)
        ));
        assert!(matches!(
            block.instructions[1].instr.real(),
            Some(Instruction::Nop)
        ));
        let folded = &block.instructions[2];
        assert!(matches!(
            folded.instr.real(),
            Some(Instruction::LoadConst { .. })
        ));
        assert!(matches!(
            &metadata.consts[u32::from(folded.arg) as usize],
            ConstantData::Tuple { elements } if elements.len() == 2
        ));
    }

    #[test]
    fn resolve_line_numbers_duplicates_exit_blocks_like_cpython() {
        let exit = BlockIdx::new(2);
        let mut blocks = Blocks::from([Block::default(), Block::default(), Block::default()]);
        blocks[0].cpython_label = InstructionSequenceLabel::from_index(0);
        blocks[1].cpython_label = InstructionSequenceLabel::from_index(1);
        blocks[2].cpython_label = InstructionSequenceLabel::from_index(2);
        blocks[0].next = BlockIdx::new(1);
        test_block_push(&mut blocks[0], test_cond_jump(exit, 10));
        blocks[1].next = exit;
        test_block_push(&mut blocks[1], test_jump(exit, 20));
        test_block_push(&mut blocks[2], test_instr(Instruction::ReturnValue, 30));
        blocks[2].instructions[0].lineno_override = Some(NO_LOCATION_OVERRIDE);

        blocks
            .remove_unreachable()
            .expect("remove_unreachable succeeds");
        blocks
            .resolve_line_numbers(OneIndexed::MIN)
            .expect("resolve_line_numbers succeeds");

        // CPython `duplicate_exits_without_lineno()` copies a shared exit block
        // reached by jumps so each copy can inherit its sole predecessor's line.
        let duplicate = blocks[0].instructions[0].target;
        assert_ne!(duplicate, exit);
        assert_eq!(
            blocks[duplicate].cpython_label,
            InstructionSequenceLabel::from_index(3)
        );
        assert_eq!(instruction_lineno(&blocks[duplicate].instructions[0]), 10);
        assert_eq!(blocks[1].instructions[0].target, exit);
        assert_eq!(instruction_lineno(&blocks[exit].instructions[0]), 20);
    }

    #[test]
    fn propagate_line_numbers_treats_next_location_like_cpython() {
        let mut block = Block::default();
        test_block_push(&mut block, test_instr(Instruction::Nop, 10));
        test_block_push(&mut block, test_instr(Instruction::PopTop, 20));
        block.instructions[1].lineno_override = Some(NEXT_LOCATION_OVERRIDE);
        test_block_push(&mut block, test_instr(Instruction::ReturnValue, 30));
        block.instructions[2].lineno_override = Some(NO_LOCATION_OVERRIDE);
        let mut blocks = Blocks::from([block]);

        blocks
            .remove_unreachable()
            .expect("remove_unreachable succeeds");
        blocks.propagate_line_numbers();

        // CPython `propagate_line_numbers()` only copies over NO_LOCATION
        // (`lineno == NO_LOCATION`). `NEXT_LOCATION` (`lineno == -2`) becomes the
        // current previous location and is copied to following NO_LOCATION
        // instructions for assemble.c to resolve later.
        assert_eq!(
            blocks[0].instructions[1].lineno_override,
            Some(NEXT_LOCATION_OVERRIDE)
        );
        assert_eq!(
            blocks[0].instructions[2].lineno_override,
            Some(NEXT_LOCATION_OVERRIDE)
        );
    }

    #[test]
    fn propagate_line_numbers_updates_empty_jump_target_raw_slot_like_cpython() {
        let mut blocks = Blocks::from([Block::default(), Block::default(), Block::default()]);
        blocks[0].next = BlockIdx::new(2);
        test_block_push(&mut blocks[0], test_cond_jump(BlockIdx::new(1), 10));
        test_block_push(&mut blocks[1], test_instr(Instruction::Nop, 20));
        blocks[1].instructions[0].lineno_override = Some(NO_LOCATION_OVERRIDE);
        basicblock_clear(&mut blocks[1]);
        test_block_push(&mut blocks[2], test_instr(Instruction::ReturnValue, 30));

        blocks
            .remove_unreachable()
            .expect("remove_unreachable succeeds");
        blocks.propagate_line_numbers();

        // CPython `propagate_line_numbers()` directly reads `target->b_instr[0]`
        // for jump targets without checking `b_iused`. If
        // `remove_redundant_nops()` emptied the target, that writes the stale
        // backing slot rather than an active instruction.
        assert_eq!(instruction_lineno(&blocks[1].instructions[0]), 10);
    }

    #[test]
    fn basicblock_has_no_lineno_treats_next_location_like_cpython() {
        let mut block = Block::default();
        test_block_push(&mut block, test_instr(Instruction::Nop, 10));
        block.instructions[0].lineno_override = Some(NEXT_LOCATION_OVERRIDE);

        // CPython `basicblock_has_no_lineno()` treats every negative lineno as
        // no line number, including `NEXT_LOCATION` (`lineno == -2`).
        assert!(basicblock_has_no_lineno(&block));

        test_block_push(&mut block, test_instr(Instruction::PopTop, 11));
        assert!(!basicblock_has_no_lineno(&block));
    }

    #[test]
    fn jump_threading_rechecks_new_jump_like_cpython() {
        let mut blocks = Blocks::from([
            Block::default(),
            Block::default(),
            Block::default(),
            Block::default(),
        ]);
        for (i, block) in blocks.iter_mut().enumerate() {
            block.cpython_label = InstructionSequenceLabel::from_index(i as i32);
        }
        blocks[0].next = BlockIdx::new(1);
        blocks[1].next = BlockIdx::new(2);
        blocks[2].next = BlockIdx::new(3);
        test_block_push(&mut blocks[0], test_jump(BlockIdx::new(1), 10));
        test_block_push(&mut blocks[1], test_jump(BlockIdx::new(2), 20));
        test_block_push(&mut blocks[2], test_jump(BlockIdx::new(3), 30));
        test_block_push(&mut blocks[3], test_instr(Instruction::ReturnValue, 40));

        let mut metadata = test_code_info(Block::default()).metadata;
        blocks
            .optimize_basic_block(&mut metadata, BlockIdx::new(0))
            .expect("valid jump chain");

        // CPython `optimize_basic_block()` continues after `jump_thread()`, so
        // the appended jump is immediately checked against the next jump target.
        let threaded = basicblock_last_instr(&blocks[0]).expect("threaded jump");
        assert!(matches!(
            threaded.instr.pseudo(),
            Some(PseudoInstruction::Jump { .. })
        ));
        assert_eq!(threaded.target, BlockIdx::new(3));
        assert_eq!(u32::from(threaded.arg), 3);
    }
}
