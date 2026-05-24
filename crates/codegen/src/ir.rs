use core::ops;

use crate::{IndexMap, IndexSet, error::InternalError};
use malachite_bigint::BigInt;
use num_complex::Complex;
use num_traits::{ToPrimitive, Zero};
use rustpython_wtf8::Wtf8Buf;

use rustpython_compiler_core::{
    OneIndexed, SourceLocation,
    bytecode::{
        AnyInstruction, AnyOpcode, Arg, CO_FAST_CELL, CO_FAST_FREE, CO_FAST_HIDDEN, CO_FAST_LOCAL,
        CodeFlags, CodeObject, CodeUnit, CodeUnits, ConstantData, ExceptionTableEntry,
        InstrDisplayContext, Instruction, IntrinsicFunction1, OpArg, Opcode, PseudoInstruction,
        PseudoOpcode, PyCodeLocationInfoKind, encode_exception_table, oparg,
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

pub(crate) const LINE_ONLY_LOCATION_OVERRIDE: i32 = -4;
pub(crate) const NEXT_LOCATION_OVERRIDE: i32 = -2;

const MAX_INT_SIZE_BITS: u64 = 128;
const MAX_COLLECTION_SIZE: usize = 256;
const MAX_TOTAL_ITEMS: isize = 1024;
const MAX_STR_SIZE: usize = 4096;
const MIN_CONST_SEQUENCE_SIZE: usize = 3;
const STACK_USE_GUIDELINE: usize = 30;

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

impl ops::Index<usize> for ConstantPool {
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

    /// Returns the inner value as a [`usize`].
    #[must_use]
    pub const fn idx(self) -> usize {
        self.0 as usize
    }
}

impl From<BlockIdx> for u32 {
    fn from(block_idx: BlockIdx) -> Self {
        block_idx.0
    }
}

impl ops::Index<BlockIdx> for [Block] {
    type Output = Block;

    fn index(&self, idx: BlockIdx) -> &Block {
        &self[idx.idx()]
    }
}

impl ops::IndexMut<BlockIdx> for [Block] {
    fn index_mut(&mut self, idx: BlockIdx) -> &mut Block {
        &mut self[idx.idx()]
    }
}

impl ops::Index<BlockIdx> for Vec<Block> {
    type Output = Block;

    fn index(&self, idx: BlockIdx) -> &Block {
        &self[idx.idx()]
    }
}

impl ops::IndexMut<BlockIdx> for Vec<Block> {
    fn index_mut(&mut self, idx: BlockIdx) -> &mut Block {
        &mut self[idx.idx()]
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
    /// Number of CACHE code units emitted after this instruction
    pub cache_entries: u32,
}

/// Exception handler information for an instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExceptHandlerInfo {
    /// Block to jump to when exception occurs
    pub handler_block: BlockIdx,
    /// Stack depth at handler entry
    pub stack_depth: u32,
    /// Whether to push lasti before exception
    pub preserve_lasti: bool,
}

fn set_to_nop(info: &mut InstructionInfo) {
    info.instr = Instruction::Nop.into();
    info.arg = OpArg::new(0);
    info.target = BlockIdx::NULL;
    info.cache_entries = 0;
}

fn nop_out_no_location(info: &mut InstructionInfo) {
    set_to_nop(info);
    info.lineno_override = Some(-1);
}

/// flowgraph.c basicblock_addop
fn basicblock_addop(block: &mut Block, mut info: InstructionInfo) {
    if let Some(stale) = block.cpython_spare_instr_slots.first().copied() {
        block.cpython_spare_instr_slots.remove(0);
        info.except_handler = stale.except_handler;
    }
    info.target = BlockIdx::NULL;
    block.instructions.push(info);
}

/// flowgraph.c basicblock_add_jump
fn basicblock_add_jump_op(
    block: &mut Block,
    info: InstructionInfo,
    target: BlockIdx,
) -> crate::InternalResult<()> {
    if block
        .instructions
        .last()
        .is_some_and(|last| last.instr.has_jump())
    {
        return Err(InternalError::MalformedControlFlowGraph);
    }
    basicblock_addop(block, info);
    block.instructions.last_mut().expect("missing jump").target = target;
    Ok(())
}

/// flowgraph.c basicblock_insert_instruction
fn basicblock_insert_instruction(block: &mut Block, pos: usize, info: InstructionInfo) {
    if !block.cpython_spare_instr_slots.is_empty() {
        block.cpython_spare_instr_slots.remove(0);
    }
    block.instructions.insert(pos, info);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InstructionSequenceLabel(usize);

impl InstructionSequenceLabel {
    pub(crate) fn idx(self) -> usize {
        self.0
    }
}

#[derive(Clone, Copy)]
struct InstructionSequenceExceptHandlerInfo {
    target_label: Option<InstructionSequenceLabel>,
    target_offset: Option<usize>,
    stack_depth: u32,
    preserve_lasti: bool,
}

#[derive(Clone, Copy)]
struct InstructionSequenceEntry {
    info: InstructionInfo,
    target_label: Option<InstructionSequenceLabel>,
    target_offset: Option<usize>,
    except_handler: Option<InstructionSequenceExceptHandlerInfo>,
    is_target: bool,
}

impl InstructionSequenceEntry {
    fn new(
        info: InstructionInfo,
        target_label: Option<InstructionSequenceLabel>,
        except_handler: Option<InstructionSequenceExceptHandlerInfo>,
    ) -> Self {
        Self {
            info,
            target_label,
            target_offset: None,
            except_handler,
            is_target: false,
        }
    }
}

const INSTRUCTION_SEQUENCE_UNSET_LABEL: isize = -111;

#[derive(Clone)]
enum InstructionSequenceLabelOffsets {
    Active(Vec<isize>),
    Applied,
}

#[derive(Clone)]
pub(crate) struct InstructionSequence {
    instrs: Vec<InstructionSequenceEntry>,
    label_map: InstructionSequenceLabelOffsets,
    annotations_code: Option<Box<Self>>,
}

impl InstructionSequence {
    pub(crate) fn new() -> Self {
        Self {
            instrs: Vec::new(),
            label_map: InstructionSequenceLabelOffsets::Active(Vec::new()),
            annotations_code: None,
        }
    }

    /// instruction_sequence.c _PyInstructionSequence_Addop asserts.
    fn debug_check_addop(info: &InstructionInfo) {
        let opcode = AnyOpcode::from(info.instr);
        debug_assert!(
            opcode.has_arg() || info.instr.has_target() || u32::from(info.arg) == 0,
            "CPython _PyInstructionSequence_Addop requires either OPCODE_HAS_ARG, HAS_TARGET, or oparg == 0"
        );
        debug_assert!(
            u32::from(info.arg) < (1 << 30),
            "CPython _PyInstructionSequence_Addop requires 0 <= oparg < (1 << 30)"
        );
    }

    fn set_annotations_code(&mut self, annotations_code: Option<Box<Self>>) {
        debug_assert!(self.annotations_code.is_none());
        self.annotations_code = annotations_code;
    }

    fn use_label(&mut self, label: InstructionSequenceLabel) {
        let InstructionSequenceLabelOffsets::Active(label_map) = &mut self.label_map else {
            panic!("instruction sequence label map already applied");
        };
        let old_len = label_map.len();
        if label_map.len() <= label.idx() {
            label_map.resize(label.idx() + 1, INSTRUCTION_SEQUENCE_UNSET_LABEL);
        }
        for slot in &mut label_map[old_len..] {
            *slot = INSTRUCTION_SEQUENCE_UNSET_LABEL;
        }
        label_map[label.idx()] = self.instrs.len() as isize;
    }

    fn addop(
        &mut self,
        info: InstructionInfo,
        target_label: Option<InstructionSequenceLabel>,
        except_handler: Option<InstructionSequenceExceptHandlerInfo>,
    ) {
        Self::debug_check_addop(&info);
        self.instrs.push(InstructionSequenceEntry::new(
            info,
            target_label,
            except_handler,
        ));
    }

    fn debug_check_pop_preserves_cpython_label_map(&self) {
        let Some((popped_index, entry)) = self.instrs.len().checked_sub(1).and_then(|index| {
            self.instrs.last().map(|entry| {
                (
                    isize::try_from(index).expect("too many instructions"),
                    entry,
                )
            })
        }) else {
            return;
        };
        debug_assert!(
            !entry.info.instr.has_target()
                && entry.info.target == BlockIdx::NULL
                && entry.target_label.is_none()
                && entry.target_offset.is_none()
                && entry.except_handler.is_none(),
            "RustPython-only instruction-sequence pop must not remove CPython label/target state"
        );
        let InstructionSequenceLabelOffsets::Active(label_map) = &self.label_map else {
            debug_assert!(false, "cannot pop after CPython label map application");
            return;
        };
        debug_assert!(
            label_map.iter().all(|&target| target < popped_index),
            "RustPython-only instruction-sequence pop must not change CPython label offsets"
        );
    }

    fn pop(&mut self) -> Option<InstructionInfo> {
        self.debug_check_pop_preserves_cpython_label_map();
        self.instrs.pop().map(|entry| entry.info)
    }

    fn last_info_mut(&mut self) -> Option<&mut InstructionInfo> {
        self.instrs.last_mut().map(|entry| &mut entry.info)
    }

    fn insert_instruction(
        &mut self,
        pos: usize,
        info: InstructionInfo,
        target_label: Option<InstructionSequenceLabel>,
    ) {
        debug_assert!(pos <= self.instrs.len());
        Self::debug_check_addop(&info);
        self.instrs
            .insert(pos, InstructionSequenceEntry::new(info, target_label, None));
        if let InstructionSequenceLabelOffsets::Active(label_map) = &mut self.label_map {
            for target in label_map.iter_mut() {
                if *target >= pos as isize {
                    *target += 1;
                }
            }
        }
    }

    fn apply_label_map(&mut self) -> crate::InternalResult<()> {
        let label_map = match core::mem::replace(
            &mut self.label_map,
            InstructionSequenceLabelOffsets::Applied,
        ) {
            InstructionSequenceLabelOffsets::Active(label_map) => label_map,
            InstructionSequenceLabelOffsets::Applied => return Ok(()),
        };
        let resolve_label = |label: InstructionSequenceLabel| -> crate::InternalResult<usize> {
            label_map
                .get(label.idx())
                .copied()
                .filter(|target_index| *target_index >= 0)
                .and_then(|target_index| usize::try_from(target_index).ok())
                .ok_or(InternalError::MalformedControlFlowGraph)
        };
        for entry in &mut self.instrs {
            if entry.info.instr.has_target() {
                let label_id = entry
                    .target_label
                    .take()
                    .ok_or(InternalError::MalformedControlFlowGraph)?;
                let target_index = resolve_label(label_id)?;
                entry.info.arg = OpArg::new(
                    target_index
                        .to_u32()
                        .ok_or(InternalError::MalformedControlFlowGraph)?,
                );
                entry.target_offset = Some(target_index);
            } else if entry.target_label.take().is_some() {
                return Err(InternalError::MalformedControlFlowGraph);
            }
            if let Some(handler) = &mut entry.except_handler
                && let Some(label_id) = handler.target_label.take()
            {
                handler.target_offset = Some(resolve_label(label_id)?);
            }
        }
        Ok(())
    }

    fn mark_targets(&mut self) -> crate::InternalResult<()> {
        for entry in &mut self.instrs {
            entry.is_target = false;
        }
        let targets: Vec<usize> = self
            .instrs
            .iter()
            .filter_map(|entry| entry.target_offset)
            .collect();
        for target_offset in targets {
            let target = self
                .instrs
                .get_mut(target_offset)
                .ok_or(InternalError::MalformedControlFlowGraph)?;
            target.is_target = true;
        }
        Ok(())
    }
}

/// flowgraph.c _PyCfg_ToInstructionSequence
fn cfg_to_instruction_sequence(
    blocks: &mut [Block],
    block_order: &[BlockIdx],
) -> crate::InternalResult<InstructionSequence> {
    for block in blocks.iter_mut() {
        block.cpython_label_id = None;
    }

    for (label_id, block_idx) in block_order.iter().copied().enumerate() {
        blocks[block_idx.idx()].cpython_label_id = Some(InstructionSequenceLabel(label_id));
    }

    let mut block_to_label = vec![None; blocks.len()];
    for block_idx in block_order.iter().copied() {
        block_to_label[block_idx.idx()] = blocks[block_idx.idx()].cpython_label_id;
    }

    let mut instr_sequence = InstructionSequence::new();
    for block_idx in block_order.iter().copied() {
        let block_label = blocks[block_idx.idx()]
            .cpython_label_id
            .ok_or(InternalError::MalformedControlFlowGraph)?;
        instr_sequence.use_label(block_label);

        for info in &blocks[block_idx.idx()].instructions {
            let mut info = *info;
            let target_label = if info.target != BlockIdx::NULL {
                if !info.instr.has_target() {
                    return Err(InternalError::MalformedControlFlowGraph);
                }
                let label_id = block_to_label
                    .get(info.target.idx())
                    .copied()
                    .flatten()
                    .ok_or(InternalError::MalformedControlFlowGraph)?;
                info.arg = OpArg::new(
                    label_id
                        .idx()
                        .to_u32()
                        .ok_or(InternalError::MalformedControlFlowGraph)?,
                );
                info.target = BlockIdx::NULL;
                Some(label_id)
            } else {
                None
            };

            let except_handler = if let Some(handler) = info.except_handler.take() {
                let label_id = block_to_label
                    .get(handler.handler_block.idx())
                    .copied()
                    .flatten()
                    .ok_or(InternalError::MalformedControlFlowGraph)?;
                Some(InstructionSequenceExceptHandlerInfo {
                    target_label: Some(label_id),
                    target_offset: None,
                    stack_depth: handler.stack_depth,
                    preserve_lasti: handler.preserve_lasti,
                })
            } else {
                None
            };

            instr_sequence.addop(info, target_label, except_handler);
        }
    }

    instr_sequence.apply_label_map()?;
    Ok(instr_sequence)
}

// spell-checker:ignore petgraph
// TODO: look into using petgraph for handling blocks and stuff? it's heavier than this, but it
// might enable more analysis/optimizations
#[derive(Debug, Clone)]
pub struct Block {
    pub instructions: Vec<InstructionInfo>,
    pub next: BlockIdx,
    // Post-codegen analysis fields (set by label_exception_targets)
    /// Whether this block is an exception handler target (b_except_handler)
    pub except_handler: bool,
    /// Whether to preserve lasti for this handler block (b_preserve_lasti)
    pub preserve_lasti: bool,
    /// Stack depth at block entry, set by stack depth analysis
    pub start_depth: Option<u32>,
    /// Whether this block is only reachable via exception table (b_cold)
    pub cold: bool,
    /// CPython `basicblock.b_label` used by translate_jump_labels_to_targets.
    cpython_label_id: Option<InstructionSequenceLabel>,
    /// CPython keeps `b_instr` allocated beyond `b_iused`. Instructions removed
    /// by compaction remain in those spare slots until `basicblock_next_instr()`
    /// reuses them.
    cpython_spare_instr_slots: Vec<InstructionInfo>,
}

impl Default for Block {
    fn default() -> Self {
        Self {
            instructions: Vec::new(),
            next: BlockIdx::NULL,
            except_handler: false,
            preserve_lasti: false,
            start_depth: None,
            cold: false,
            cpython_label_id: None,
            cpython_spare_instr_slots: Vec::new(),
        }
    }
}

impl Block {
    pub(crate) fn has_cpython_cfg_label(&self) -> bool {
        self.cpython_label_id.is_some()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct InstructionSequenceLabelMap {
    next_free_label: usize,
    block_labels: Vec<InstructionSequenceLabel>,
    /// Direct-CFG shadow for CPython labels that map to the same instruction
    /// offset in `_PyInstructionSequence_UseLabel()`.
    direct_block_by_label: Vec<Option<BlockIdx>>,
}

impl InstructionSequenceLabelMap {
    pub(crate) fn new() -> Self {
        Self {
            next_free_label: 0,
            block_labels: vec![InstructionSequenceLabel(0)],
            direct_block_by_label: vec![Some(BlockIdx::new(0))],
        }
    }

    /// instruction_sequence.c _PyInstructionSequence_NewLabel
    fn new_label(&mut self) -> InstructionSequenceLabel {
        self.next_free_label += 1;
        let label = InstructionSequenceLabel(self.next_free_label);
        if self.direct_block_by_label.len() <= label.idx() {
            self.direct_block_by_label.resize(label.idx() + 1, None);
        }
        label
    }

    pub(crate) fn push_unmapped_label(&mut self) {
        let label = self.new_label();
        let block = BlockIdx(
            self.block_labels
                .len()
                .to_u32()
                .expect("too many direct-CFG blocks"),
        );
        self.direct_block_by_label[label.idx()] = Some(block);
        self.block_labels.push(label);
    }

    fn label_for_block(&self, block: BlockIdx) -> InstructionSequenceLabel {
        debug_assert_ne!(block, BlockIdx::NULL);
        self.block_labels
            .get(block.idx())
            .copied()
            .expect("basic block must have an instruction-sequence label")
    }

    fn block_for_label(&self, label: InstructionSequenceLabel) -> Option<BlockIdx> {
        self.direct_block_by_label
            .get(label.idx())
            .copied()
            .flatten()
    }

    pub(crate) fn resolve_label(&self, block: BlockIdx) -> BlockIdx {
        if block == BlockIdx::NULL {
            return BlockIdx::NULL;
        }
        self.block_for_label(self.label_for_block(block))
            .unwrap_or_else(|| {
                debug_assert!(false, "CPython label must map to a direct-CFG block");
                BlockIdx::NULL
            })
    }

    pub(crate) fn resolve_label_to_block(
        &self,
        label: Option<InstructionSequenceLabel>,
    ) -> BlockIdx {
        let Some(label) = label else {
            return BlockIdx::NULL;
        };
        self.block_for_label(label).unwrap_or_else(|| {
            debug_assert!(false, "CPython label must map to a direct-CFG block");
            BlockIdx::NULL
        })
    }

    pub(crate) fn use_label_at_block(&mut self, from: BlockIdx, to: BlockIdx) {
        if from == BlockIdx::NULL || from == to {
            return;
        }
        let from_label = self.label_for_block(from);
        let to_block = self.resolve_label(to);
        if to_block == BlockIdx::NULL {
            debug_assert!(false, "CPython label target must map to a direct-CFG block");
            return;
        }
        self.direct_block_by_label[from_label.idx()] = Some(to_block);
    }

    fn debug_check_for_blocks(&self, blocks_len: usize) {
        debug_assert_eq!(
            self.block_labels.len(),
            blocks_len,
            "every direct-CFG block must have a CPython instruction-sequence label"
        );
        debug_assert_eq!(self.block_labels[0], InstructionSequenceLabel(0));
        debug_assert!(self.next_free_label + 1 >= self.block_labels.len());
        let mut seen_labels = vec![false; self.next_free_label + 1];
        for &label in &self.block_labels {
            debug_assert!(
                label.idx() <= self.next_free_label,
                "direct-CFG block labels must come from _PyInstructionSequence_NewLabel()"
            );
            debug_assert!(
                !seen_labels[label.idx()],
                "direct-CFG blocks must not share a CPython instruction-sequence label"
            );
            seen_labels[label.idx()] = true;
        }
        for &block in &self.direct_block_by_label {
            if let Some(block) = block {
                debug_assert!(
                    block.idx() < blocks_len,
                    "CPython label must map to an existing direct-CFG block"
                );
            }
        }
        for &label in &self.block_labels {
            debug_assert!(
                self.block_for_label(label)
                    .is_some_and(|block| block.idx() < blocks_len),
                "direct-CFG block label must map to a direct-CFG block"
            );
        }
    }

    fn debug_check_label_blocks_match_instruction_sequence(
        &self,
        instr_sequence: &InstructionSequence,
    ) {
        let InstructionSequenceLabelOffsets::Active(label_map) = &instr_sequence.label_map else {
            debug_assert!(
                false,
                "direct-CFG label map must be checked before CPython label-map application"
            );
            return;
        };
        for (label_idx, block) in self.direct_block_by_label.iter().copied().enumerate() {
            let Some(block) = block else {
                continue;
            };
            let Some(&label_offset) = label_map.get(label_idx) else {
                continue;
            };
            if label_offset < 0 {
                continue;
            }
            let Some(block_label) = self.block_labels.get(block.idx()).copied() else {
                debug_assert!(
                    false,
                    "CPython label must map to an existing direct-CFG block"
                );
                continue;
            };
            let Some(&block_offset) = label_map.get(block_label.idx()) else {
                continue;
            };
            if block_offset < 0 {
                continue;
            }
            debug_assert!(
                label_offset == block_offset,
                "direct-CFG labels may share a block only when CPython maps them to the same instruction offset"
            );
        }
    }
}

pub struct CodeInfo {
    pub flags: CodeFlags,
    pub source_path: String,
    pub private: Option<String>, // For private name mangling, mostly for class

    pub blocks: Vec<Block>,
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
        let target_label = if info.instr.has_target() && info.target != BlockIdx::NULL {
            let label = self.instr_sequence_label_map.label_for_block(info.target);
            info.arg = OpArg::new(
                label
                    .idx()
                    .to_u32()
                    .ok_or(InternalError::MalformedControlFlowGraph)?,
            );
            info.target = BlockIdx::NULL;
            Some(label)
        } else {
            None
        };
        self.instr_sequence.addop(info, target_label, None);
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
        info.arg = OpArg::new(
            target_label
                .idx()
                .to_u32()
                .ok_or(InternalError::MalformedControlFlowGraph)?,
        );
        info.target = BlockIdx::NULL;
        self.instr_sequence.addop(info, Some(target_label), None);
        Ok(())
    }

    pub(crate) fn pop_instr_sequence(&mut self) -> Option<InstructionInfo> {
        self.instr_sequence.pop()
    }

    pub(crate) fn set_last_instr_sequence_lineno_override(&mut self, lineno_override: i32) {
        if let Some(last) = self.instr_sequence.last_info_mut() {
            last.lineno_override = Some(lineno_override);
        }
    }

    pub(crate) fn use_instr_sequence_label(&mut self, block: BlockIdx) {
        let label = self.instr_sequence_label_map.label_for_block(block);
        self.instr_sequence.use_label(label);
    }

    pub(crate) fn mark_cpython_cfg_label(&mut self, block: BlockIdx) {
        let label = self.instr_sequence_label_map.label_for_block(block);
        self.blocks[block.idx()].cpython_label_id = Some(label);
    }

    pub(crate) fn resolve_instr_sequence_label(&self, block: BlockIdx) -> BlockIdx {
        self.instr_sequence_label_map.resolve_label(block)
    }

    pub(crate) fn block_for_instr_sequence_label(
        &self,
        label: Option<InstructionSequenceLabel>,
    ) -> BlockIdx {
        self.instr_sequence_label_map.resolve_label_to_block(label)
    }

    pub(crate) fn use_instr_sequence_label_at_block(&mut self, from: BlockIdx, to: BlockIdx) {
        self.instr_sequence_label_map.use_label_at_block(from, to);
    }

    pub(crate) fn instr_sequence_label_for_block(
        &self,
        block: BlockIdx,
    ) -> Option<InstructionSequenceLabel> {
        if block == BlockIdx::NULL {
            None
        } else {
            Some(self.instr_sequence_label_map.label_for_block(block))
        }
    }

    pub(crate) fn insert_start_setup_cleanup(&mut self, handler_block: BlockIdx) {
        let handler_label = self.instr_sequence_label_map.label_for_block(handler_block);
        self.instr_sequence.insert_instruction(
            0,
            InstructionInfo {
                instr: PseudoInstruction::SetupCleanup {
                    delta: Arg::marker(),
                }
                .into(),
                arg: OpArg::new(handler_label.idx().to_u32().expect("too many labels")),
                target: BlockIdx::NULL,
                location: SourceLocation::default(),
                end_location: SourceLocation::default(),
                except_handler: None,
                lineno_override: Some(-1),
                cache_entries: 0,
            },
            Some(handler_label),
        );
    }

    fn take_recorded_instr_sequence(&mut self) -> crate::InternalResult<InstructionSequence> {
        let mut instr_sequence =
            core::mem::replace(&mut self.instr_sequence, InstructionSequence::new());
        if let Some(mut annotations_instr_sequence) = self.annotations_instr_sequence.take() {
            annotations_instr_sequence.apply_label_map()?;
            instr_sequence.set_annotations_code(Some(Box::new(annotations_instr_sequence)));
        }
        Ok(instr_sequence)
    }

    /// flowgraph.c cfg_builder_check
    fn debug_check_recorded_cfg_builder(&self) {
        debug_assert!(!self.blocks.is_empty());
        debug_assert!(!self.blocks[0].instructions.is_empty());
        debug_assert!(!self.instr_sequence.instrs.is_empty());
        debug_assert!(self.current_block.idx() < self.blocks.len());
        self.instr_sequence_label_map
            .debug_check_for_blocks(self.blocks.len());
        self.instr_sequence_label_map
            .debug_check_label_blocks_match_instruction_sequence(&self.instr_sequence);
        for block in &self.blocks {
            if block.next != BlockIdx::NULL {
                debug_assert!(block.next.idx() < self.blocks.len());
            }
            for instr in &block.instructions {
                debug_assert!(!instr.instr.is_assembler());
                if instr.target != BlockIdx::NULL {
                    debug_assert!(instr.target.idx() < self.blocks.len());
                }
            }
        }
    }

    fn prepare_cfg_from_codegen(&mut self) -> crate::InternalResult<InstructionSequence> {
        // CPython compile.c optimize_and_assemble_code_unit passes
        // u_instr_sequence directly into flowgraph.c _PyCfg_FromInstructionSequence().
        self.debug_check_recorded_cfg_builder();
        self.take_recorded_instr_sequence()
    }

    fn optimize_code_unit(
        &mut self,
        instr_sequence: InstructionSequence,
    ) -> crate::InternalResult<()> {
        // Phase 1: _PyCfg_OptimizeCodeUnit (flowgraph.c)
        self.blocks = cfg_from_instruction_sequence(instr_sequence)?;
        translate_jump_labels_to_targets(&mut self.blocks)?;
        mark_except_handlers(&mut self.blocks)?;
        label_exception_targets(&mut self.blocks)?;
        // CPython optimize_cfg() starts with check_cfg() and raises
        // SystemError if a jump or scope exit is not the last instruction in
        // its block.
        check_cfg(&self.blocks)?;
        inline_small_or_no_lineno_blocks(&mut self.blocks);
        // CPython does not re-run instruction-sequence label-map/CFG conversion
        // after this point. Unreferenced label blocks left by jump inlining
        // remain block boundaries and can preserve line-marker NOPs.
        self.remove_unreachable_blocks();
        // CPython optimize_cfg resolves line numbers before local checks and
        // superinstruction insertion, so fusion decisions see propagated
        // source locations.
        resolve_line_numbers(&mut self.blocks);
        // CPython optimize_cfg() runs optimize_load_const() and then
        // optimize_basic_block() after line numbers are resolved.
        self.convert_to_load_small_int();
        self.peephole_optimize();
        self.convert_to_load_small_int();
        self.optimize_basic_blocks()?;
        self.remove_redundant_nops_and_pairs();
        // CPython optimize_cfg() removes newly-unreachable blocks and
        // redundant NOP/jump chains before _PyCfg_OptimizeCodeUnit() prunes
        // unused constants.
        self.remove_unreachable_blocks();
        remove_redundant_nops_and_jumps(&mut self.blocks)?;
        debug_assert!(no_redundant_jumps(&self.blocks));
        self.remove_unused_consts();
        self.add_checks_for_loads_of_uninitialized_variables();
        // CPython inserts superinstructions in _PyCfg_OptimizeCodeUnit, before
        // later jump normalization / block reordering can create adjacencies
        // that never exist at this stage in flowgraph.c.
        self.insert_superinstructions();
        push_cold_blocks_to_end(&mut self.blocks)?;
        // CPython resolves line numbers again after cold-block extraction.
        resolve_line_numbers(&mut self.blocks);
        Ok(())
    }

    fn optimized_cfg_to_instruction_sequence(&mut self) -> crate::InternalResult<(u32, usize)> {
        // Phase 2: _PyCfg_OptimizedCfgToInstructionSequence (flowgraph.c)
        convert_pseudo_conditional_jumps(&mut self.blocks);
        let max_stackdepth = self.max_stackdepth()?;
        debug_assert!(
            !self.flags.intersects(
                CodeFlags::GENERATOR | CodeFlags::COROUTINE | CodeFlags::ASYNC_GENERATOR
            ) || max_stackdepth != 0
        );
        let nlocalsplus = self.prepare_localsplus();
        // Match CPython order: pseudo ops are lowered after stackdepth and
        // localsplus preparation, before normalize_jumps.
        convert_pseudo_ops(&mut self.blocks)?;
        normalize_jumps(&mut self.blocks)?;
        debug_assert!(no_redundant_jumps(&self.blocks));
        // optimize_load_fast: after normalize_jumps
        self.optimize_load_fast_borrow();

        Ok((max_stackdepth, nlocalsplus))
    }

    pub fn finalize_code(
        mut self,
        opts: &crate::compile::CompileOpts,
    ) -> crate::InternalResult<CodeObject> {
        let instr_sequence = self.prepare_cfg_from_codegen()?;
        self.optimize_code_unit(instr_sequence)?;
        let (max_stackdepth, nlocalsplus) = self.optimized_cfg_to_instruction_sequence()?;

        let Self {
            flags,
            source_path,
            private: _, // private is only used during compilation

            mut blocks,
            current_block: _,
            instr_sequence: _,
            instr_sequence_label_map: _,
            annotations_instr_sequence: _,
            metadata,
            static_attributes: _,
            in_inlined_comp: _,
            fblock: _,
            symbol_table_index: _,
            in_conditional_block: _,
            next_conditional_annotation_index: _,
        } = self;

        let CodeUnitMetadata {
            name: obj_name,
            qualname,
            consts: constants,
            names: name_cache,
            varnames: varname_cache,
            cellvars: cellvar_cache,
            freevars: freevar_cache,
            fast_hidden,
            fast_hidden_final,
            argcount: arg_count,
            posonlyargcount: posonlyarg_count,
            kwonlyargcount: kwonlyarg_count,
            firstlineno: first_line_number,
        } = metadata;

        let mut instructions = Vec::new();
        let mut locations = Vec::new();
        let mut linetable_locations: Vec<LineTableLocation> = Vec::new();

        // Rebuild and adjust cellfixedoffsets for localsplus metadata. Pseudo
        // lowering and deref operand fixups already ran in
        // optimized_cfg_to_instruction_sequence().
        let mut cellfixedoffsets =
            build_cellfixedoffsets(&varname_cache, &cellvar_cache, &freevar_cache);
        let numdropped = fix_cellfixedoffsets(varname_cache.len(), &mut cellfixedoffsets);

        // Pre-compute cache_entries for real (non-pseudo) instructions
        for block in &mut blocks {
            for instr in &mut block.instructions {
                if let AnyInstruction::Real(op) = instr.instr {
                    instr.cache_entries = op.cache_entries() as u32;
                }
            }
        }

        let block_order = layout_block_order(&blocks);
        let mut instr_sequence = cfg_to_instruction_sequence(&mut blocks, &block_order)?;
        let mut instruction_offsets = vec![0u32; instr_sequence.instrs.len()];
        let mut end_offset;
        // The offset (in code units) of END_SEND from SEND in the yield-from sequence.
        const END_SEND_OFFSET: u32 = 5;
        loop {
            let mut num_instructions = 0;
            for (idx, instr) in instr_sequence.instrs.iter().enumerate() {
                instruction_offsets[idx] = num_instructions as u32;
                num_instructions += instr.info.arg.instr_size() + instr.info.cache_entries as usize;
            }
            end_offset = num_instructions as u32;

            instructions.reserve_exact(num_instructions);
            locations.reserve_exact(num_instructions);

            let mut recompile = false;
            for (current_instr_index, entry) in instr_sequence.instrs.iter_mut().enumerate() {
                // Track current instruction offset for jump offset resolution.
                let current_offset = instruction_offsets[current_instr_index];
                {
                    let info = &mut entry.info;
                    let old_arg_size = info.arg.instr_size();
                    let old_cache_entries = info.cache_entries;
                    // Keep offsets fixed within this pass: changes in jump
                    // arg/cache sizes only take effect in the next iteration.
                    let offset_after = current_offset + old_arg_size as u32 + old_cache_entries;
                    let op = match info.instr {
                        AnyInstruction::Pseudo(
                            PseudoInstruction::Jump { .. }
                            | PseudoInstruction::JumpNoInterrupt { .. },
                        ) if entry.target_offset.is_none() => {
                            return Err(InternalError::MalformedControlFlowGraph);
                        }
                        // CPython assemble.c::resolve_unconditional_jumps()
                        // resolves pseudo JUMP/JUMP_NO_INTERRUPT after label-map
                        // application, using instruction indexes rather than CFG
                        // block order or byte offsets.
                        AnyInstruction::Pseudo(PseudoInstruction::Jump { .. }) => {
                            if entry.target_offset.expect("missing jump target")
                                > current_instr_index
                            {
                                Instruction::JumpForward {
                                    delta: Arg::marker(),
                                }
                            } else {
                                Instruction::JumpBackward {
                                    delta: Arg::marker(),
                                }
                            }
                        }
                        AnyInstruction::Pseudo(PseudoInstruction::JumpNoInterrupt { .. }) => {
                            if entry.target_offset.expect("missing jump target")
                                > current_instr_index
                            {
                                Instruction::JumpForward {
                                    delta: Arg::marker(),
                                }
                            } else {
                                Instruction::JumpBackwardNoInterrupt {
                                    delta: Arg::marker(),
                                }
                            }
                        }
                        _ => info.instr.expect_real(),
                    };

                    if let Some(target_index) = entry.target_offset {
                        let target_offset = instruction_offsets[target_index];
                        // CPython assemble.c::resolve_jump_offsets() only
                        // converts label/instruction indexes to bytecode
                        // offsets here. Direction selection for pseudo
                        // unconditional jumps has already happened in
                        // resolve_unconditional_jumps(), and redundant-jump
                        // removal belongs to flowgraph.c optimization.
                        info.instr = op.into();
                        let updated_cache = op.cache_entries() as u32;
                        recompile |= updated_cache != old_cache_entries;
                        info.cache_entries = updated_cache;
                        let new_arg = if matches!(op, Instruction::EndAsyncFor) {
                            let arg = offset_after
                                .checked_sub(target_offset + END_SEND_OFFSET)
                                .expect("END_ASYNC_FOR target must be before instruction");
                            OpArg::new(arg)
                        } else if matches!(
                            op.into(),
                            Opcode::JumpBackward | Opcode::JumpBackwardNoInterrupt
                        ) {
                            let arg = offset_after
                                .checked_sub(target_offset)
                                .expect("backward jump target must be before instruction");
                            OpArg::new(arg)
                        } else {
                            let arg = target_offset
                                .checked_sub(offset_after)
                                .expect("forward jump target must be after instruction");
                            OpArg::new(arg)
                        };
                        recompile |= new_arg.instr_size() != old_arg_size;
                        info.arg = new_arg;
                    }

                    let cache_count = info.cache_entries as usize;
                    let (extras, lo_arg) = info.arg.split();
                    let loc_pair = (info.location, info.end_location);
                    locations.extend(core::iter::repeat_n(
                        loc_pair,
                        info.arg.instr_size() + cache_count,
                    ));
                    // Collect linetable locations with lineno_override support
                    let lt_loc = match info.lineno_override {
                        Some(-1) => LineTableLocation {
                            line: -1,
                            end_line: -1,
                            col: -1,
                            end_col: -1,
                        },
                        Some(LINE_ONLY_LOCATION_OVERRIDE) => LineTableLocation {
                            line: info.location.line.get() as i32,
                            end_line: info.end_location.line.get() as i32,
                            col: -1,
                            end_col: -1,
                        },
                        Some(NEXT_LOCATION_OVERRIDE) => LineTableLocation {
                            line: NEXT_LOCATION_OVERRIDE,
                            end_line: NEXT_LOCATION_OVERRIDE,
                            col: NEXT_LOCATION_OVERRIDE,
                            end_col: NEXT_LOCATION_OVERRIDE,
                        },
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
                    };
                    linetable_locations.extend(core::iter::repeat_n(lt_loc, info.arg.instr_size()));
                    // CACHE entries inherit parent instruction's location
                    if cache_count > 0 {
                        linetable_locations.extend(core::iter::repeat_n(lt_loc, cache_count));
                    }
                    instructions.extend(
                        extras
                            .map(|byte| CodeUnit::new(Instruction::ExtendedArg, byte))
                            .chain([CodeUnit { op, arg: lo_arg }]),
                    );
                    // Emit CACHE code units after the instruction (all zeroed)
                    if cache_count > 0 {
                        instructions.extend(core::iter::repeat_n(
                            CodeUnit::new(Instruction::Cache, 0.into()),
                            cache_count,
                        ));
                    }
                }
            }

            if !recompile {
                break;
            }

            instructions.clear();
            locations.clear();
            linetable_locations.clear();
        }

        // CPython assemble.c::assemble_location_info() resolves NEXT_LOCATION
        // after final instruction sizing, scanning backward through the
        // instruction sequence. Non-terminators inherit the following
        // instruction's location; terminators become NO_LOCATION.
        resolve_next_locations(&instructions, &mut linetable_locations);

        // Generate linetable from linetable_locations (supports line 0 for RESUME)
        let linetable = generate_linetable(
            &linetable_locations,
            first_line_number.get() as i32,
            opts.debug_ranges,
        );
        let locations = rustpython_compiler_core::marshal::linetable_to_locations(
            &linetable,
            first_line_number.get() as i32,
            instructions.len(),
        );

        // Generate exception table before moving source_path
        let exceptiontable =
            generate_exception_table(&instr_sequence.instrs, &instruction_offsets, end_offset);

        // CPython builds u_cellvars in dictbytype() order, but the public
        // co_cellvars tuple follows localsplus order from assemble.c:
        // cell locals already present in varnames first, then remaining cells.
        let final_cellvars = varname_cache
            .iter()
            .filter(|name| cellvar_cache.contains(name.as_str()))
            .chain(
                cellvar_cache
                    .iter()
                    .filter(|name| !varname_cache.contains(name.as_str())),
            )
            .cloned()
            .collect::<Vec<_>>();

        // Build localspluskinds with cell-local merging
        let nlocals = varname_cache.len();
        let ncells = cellvar_cache.len();
        let nfrees = freevar_cache.len();
        debug_assert_eq!(
            nlocalsplus,
            nlocals + ncells - numdropped + nfrees,
            "CPython prepare_localsplus() result must match assemble.c localsplus sizing"
        );
        let mut localspluskinds = vec![0u8; nlocalsplus];
        // Mark locals
        for kind in localspluskinds.iter_mut().take(nlocals) {
            *kind = CO_FAST_LOCAL;
        }
        // Mark cells (merged and non-merged)
        for (i, cellvar) in cellvar_cache.iter().enumerate() {
            let idx = cellfixedoffsets[i] as usize;
            if varname_cache.contains(cellvar.as_str()) {
                localspluskinds[idx] |= CO_FAST_CELL; // merged: LOCAL | CELL
            } else {
                localspluskinds[idx] = CO_FAST_CELL;
            }
        }
        // Mark frees
        for i in 0..nfrees {
            let idx = cellfixedoffsets[ncells + i] as usize;
            localspluskinds[idx] = CO_FAST_FREE;
        }
        // Apply CO_FAST_HIDDEN for inlined comprehension variables
        for (name, &hidden) in &fast_hidden {
            if (hidden || fast_hidden_final.contains(name))
                && let Some(idx) = varname_cache.get_index_of(name.as_str())
            {
                localspluskinds[idx] |= CO_FAST_HIDDEN;
            }
        }

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
            instructions: CodeUnits::from(instructions),
            locations,
            constants: constants.into_iter().collect(),
            names: name_cache.into_iter().collect(),
            varnames: varname_cache.into_iter().collect(),
            cellvars: final_cellvars.into_boxed_slice(),
            freevars: freevar_cache.into_iter().collect(),
            localspluskinds: localspluskinds.into_boxed_slice(),
            linetable,
            exceptiontable,
        })
    }

    /// flowgraph.c insert_prefix_instructions
    fn insert_prefix_instructions(&mut self, cellfixedoffsets: &[u32]) {
        let Some(entry) = self.blocks.first_mut() else {
            return;
        };
        let ncells = self.metadata.cellvars.len();
        let nfrees = self.metadata.freevars.len();
        let firstlineno = self.metadata.firstlineno;

        if self
            .flags
            .intersects(CodeFlags::GENERATOR | CodeFlags::COROUTINE | CodeFlags::ASYNC_GENERATOR)
        {
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
                    cache_entries: 0,
                },
            );
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
                    cache_entries: 0,
                },
            );
        }

        let mut sorted = vec![None; self.metadata.varnames.len() + ncells];
        for (oldindex, fixed) in cellfixedoffsets.iter().copied().take(ncells).enumerate() {
            sorted[fixed as usize] = Some(oldindex + 1);
        }
        for (ncellsused, oldindex) in sorted.into_iter().flatten().enumerate() {
            basicblock_insert_instruction(
                entry,
                ncellsused,
                InstructionInfo {
                    instr: Instruction::MakeCell { i: Arg::marker() }.into(),
                    arg: OpArg::new((oldindex - 1) as u32),
                    target: BlockIdx::NULL,
                    location: SourceLocation::default(),
                    end_location: SourceLocation::default(),
                    except_handler: None,
                    lineno_override: Some(-1),
                    cache_entries: 0,
                },
            );
        }

        if nfrees > 0 {
            basicblock_insert_instruction(
                entry,
                0,
                InstructionInfo {
                    instr: Instruction::CopyFreeVars { n: Arg::marker() }.into(),
                    arg: OpArg::new(nfrees as u32),
                    target: BlockIdx::NULL,
                    location: SourceLocation::default(),
                    end_location: SourceLocation::default(),
                    except_handler: None,
                    lineno_override: Some(-1),
                    cache_entries: 0,
                },
            );
        }
    }

    /// flowgraph.c prepare_localsplus
    fn prepare_localsplus(&mut self) -> usize {
        let nlocals = self.metadata.varnames.len();
        let ncells = self.metadata.cellvars.len();
        let nfrees = self.metadata.freevars.len();
        let mut nlocalsplus = nlocals + ncells + nfrees;
        let mut cellfixedoffsets = build_cellfixedoffsets(
            &self.metadata.varnames,
            &self.metadata.cellvars,
            &self.metadata.freevars,
        );

        // This must be called before fix_cell_offsets().
        self.insert_prefix_instructions(&cellfixedoffsets);

        let numdropped = fix_cell_offsets(&mut self.blocks, nlocals, &mut cellfixedoffsets);
        nlocalsplus -= numdropped;
        nlocalsplus
    }

    /// flowgraph.c remove_unreachable
    fn remove_unreachable_blocks(&mut self) {
        let mut reachable = vec![false; self.blocks.len()];
        reachable[0] = true;
        let mut stack = vec![BlockIdx(0)];
        while let Some(block_idx) = stack.pop() {
            let idx = block_idx.idx();
            let block = &self.blocks[idx];
            let next = block.next;
            if next != BlockIdx::NULL && block_has_fallthrough(block) && !reachable[next.idx()] {
                reachable[next.idx()] = true;
                stack.push(next);
            }
            for ins in &block.instructions {
                if (is_jump_instruction(ins) || ins.instr.is_block_push())
                    && ins.target != BlockIdx::NULL
                    && !reachable[ins.target.idx()]
                {
                    reachable[ins.target.idx()] = true;
                    stack.push(ins.target);
                }
            }
        }

        for block_idx in self.block_next_order() {
            let i = block_idx.idx();
            let is_reachable = reachable[i];
            if !is_reachable {
                let block = &mut self.blocks[i];
                block.instructions.clear();
                block.except_handler = false;
            }
        }
    }

    fn eval_unary_constant(
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

    fn instr_make_load_const(
        metadata: &mut CodeUnitMetadata,
        instr: &mut InstructionInfo,
        constant: ConstantData,
    ) {
        if let ConstantData::Integer { value } = &constant
            && let Some(small) = value.to_i32().filter(|v| (0..=255).contains(v))
        {
            instr.instr = Opcode::LoadSmallInt.into();
            instr.arg = OpArg::new(small as u32);
            return;
        }

        let (const_idx, _) = metadata.consts.insert_full(constant);
        instr.instr = Instruction::LoadConst {
            consti: Arg::marker(),
        }
        .into();
        instr.arg = OpArg::new(const_idx as u32);
    }

    /// Try to fold a single unary instruction at position `i` in `block`.
    /// Returns true if folded. Mirrors CPython fold_const_unaryop().
    fn fold_unary_constant_at(
        metadata: &mut CodeUnitMetadata,
        block: &mut Block,
        i: usize,
    ) -> bool {
        let instr = &block.instructions[i];
        let (op, intrinsic) = match instr.instr.real() {
            Some(Instruction::UnaryNegative) => (Instruction::UnaryNegative, None),
            Some(Instruction::UnaryInvert) => (Instruction::UnaryInvert, None),
            Some(Instruction::CallIntrinsic1 { func })
                if matches!(
                    func.get(instr.arg),
                    oparg::IntrinsicFunction1::UnaryPositive
                ) =>
            {
                (
                    Instruction::CallIntrinsic1 {
                        func: Arg::marker(),
                    },
                    Some(func.get(instr.arg)),
                )
            }
            _ => return false,
        };
        let Some(operand_index) = i
            .checked_sub(1)
            .and_then(|start| Self::get_const_loading_instr_indices(block, start, 1))
            .and_then(|indices| indices.into_iter().next())
        else {
            return false;
        };
        let operand = Self::get_const_value_from(metadata, &block.instructions[operand_index]);
        let Some(operand) = operand else {
            return false;
        };
        let Some(folded_const) = Self::eval_unary_constant(&operand, op, intrinsic) else {
            return false;
        };
        nop_out_no_location(&mut block.instructions[operand_index]);
        let mut prev = operand_index;
        while let Some(idx) = prev.checked_sub(1) {
            if !matches!(block.instructions[idx].instr.real(), Some(Instruction::Nop)) {
                break;
            }
            block.instructions[idx].location = block.instructions[i].location;
            block.instructions[idx].end_location = block.instructions[i].end_location;
            prev = idx;
        }
        Self::instr_make_load_const(metadata, &mut block.instructions[i], folded_const);
        true
    }

    fn get_const_loading_instr_indices(
        block: &Block,
        mut start: usize,
        size: usize,
    ) -> Option<Vec<usize>> {
        let mut indices = Vec::with_capacity(size);
        loop {
            let instr = block.instructions.get(start)?;
            if !matches!(instr.instr.real(), Some(Instruction::Nop)) {
                Self::get_const_value_from_dummy(instr)?;
                indices.push(start);
                if indices.len() == size {
                    break;
                }
            }
            start = start.checked_sub(1)?;
        }
        indices.reverse();
        Some(indices)
    }

    fn get_const_sequence(
        metadata: &CodeUnitMetadata,
        block: &Block,
        build_index: usize,
        size: usize,
    ) -> Option<(Vec<usize>, Vec<ConstantData>)> {
        if size == 0 {
            return Some((Vec::new(), Vec::new()));
        }

        let operand_indices = build_index
            .checked_sub(1)
            .and_then(|start| Self::get_const_loading_instr_indices(block, start, size))?;
        let mut elements = Vec::with_capacity(size);

        for &j in &operand_indices {
            elements.push(Self::get_const_value_from(
                metadata,
                &block.instructions[j],
            )?);
        }

        Some((operand_indices, elements))
    }

    fn block_next_order(&self) -> Vec<BlockIdx> {
        let mut order = Vec::new();
        let mut current = BlockIdx(0);
        while current != BlockIdx::NULL {
            order.push(current);
            current = self.blocks[current.idx()].next;
        }
        order
    }

    /// Try to fold a single BINARY_OP instruction at position `i` in `block`.
    /// Returns true if folded. Mirrors CPython fold_const_binop().
    fn fold_binop_constant_at(
        metadata: &mut CodeUnitMetadata,
        block: &mut Block,
        i: usize,
    ) -> bool {
        use oparg::BinaryOperator as BinOp;

        let Some(Instruction::BinaryOp { .. }) = block.instructions[i].instr.real() else {
            return false;
        };
        let Some(operand_indices) = i
            .checked_sub(1)
            .and_then(|start| Self::get_const_loading_instr_indices(block, start, 2))
        else {
            return false;
        };
        let op_raw = u32::from(block.instructions[i].arg);
        let Ok(op) = BinOp::try_from(op_raw) else {
            return false;
        };
        let left = Self::get_const_value_from(metadata, &block.instructions[operand_indices[0]]);
        let right = Self::get_const_value_from(metadata, &block.instructions[operand_indices[1]]);
        let (Some(left_val), Some(right_val)) = (left, right) else {
            return false;
        };
        let Some(result_const) = Self::eval_binop(&left_val, &right_val, op) else {
            return false;
        };
        for &idx in &operand_indices {
            nop_out_no_location(&mut block.instructions[idx]);
        }
        Self::instr_make_load_const(metadata, &mut block.instructions[i], result_const);
        true
    }

    fn get_const_value_from_dummy(info: &InstructionInfo) -> Option<()> {
        match info.instr.real() {
            Some(Instruction::LoadConst { .. } | Instruction::LoadSmallInt { .. }) => Some(()),
            _ => None,
        }
    }

    fn get_const_value_from(
        metadata: &CodeUnitMetadata,
        info: &InstructionInfo,
    ) -> Option<ConstantData> {
        match info.instr.real() {
            Some(Instruction::LoadConst { .. }) => {
                let idx = u32::from(info.arg) as usize;
                metadata.consts.get_index(idx).cloned()
            }
            Some(Instruction::LoadSmallInt { .. }) => {
                let v = u32::from(info.arg) as i32;
                Some(ConstantData::Integer {
                    value: BigInt::from(v),
                })
            }
            _ => None,
        }
    }

    fn const_folding_check_complexity(obj: &ConstantData, mut limit: isize) -> Option<isize> {
        if let ConstantData::Tuple { elements } = obj {
            limit -= isize::try_from(elements.len()).ok()?;
            if limit < 0 {
                return None;
            }
            for element in elements {
                limit = Self::const_folding_check_complexity(element, limit)?;
            }
        }
        Some(limit)
    }

    fn eval_binop(
        left: &ConstantData,
        right: &ConstantData,
        op: oparg::BinaryOperator,
    ) -> Option<ConstantData> {
        use oparg::BinaryOperator as BinOp;

        fn repeat_wtf8(value: &Wtf8Buf, n: usize) -> Wtf8Buf {
            let mut result = Wtf8Buf::with_capacity(value.len().saturating_mul(n));
            for _ in 0..n {
                result.push_wtf8(value);
            }
            result
        }

        fn checked_repeat_count(n: &BigInt, item_size: usize) -> Option<usize> {
            let n = n.to_isize()?;
            if item_size != 0 && (n < 0 || n as usize > MAX_STR_SIZE / item_size) {
                return None;
            }
            Some(n.max(0) as usize)
        }

        fn eval_complex_binop(
            left: Complex<f64>,
            right: Complex<f64>,
            op: BinOp,
        ) -> Option<ConstantData> {
            fn complex_const(value: Complex<f64>) -> Option<ConstantData> {
                (value.re.is_finite() && value.im.is_finite())
                    .then_some(ConstantData::Complex { value })
            }

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

                        return complex_const(if right.re == 0.0 {
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
            complex_const(value)
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

        fn slice_bound(value: &ConstantData) -> Option<Option<i64>> {
            match value {
                ConstantData::None => Some(None),
                _ => constant_as_index(value).map(Some),
            }
        }

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

            let mut indices = Vec::new();
            let mut index = i128::from(start);
            let stop = i128::from(stop);
            let step = i128::from(step);
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

        fn eval_const_subscript(
            container: &ConstantData,
            index: &ConstantData,
        ) -> Option<ConstantData> {
            match (container, index) {
                (
                    ConstantData::Str { value },
                    ConstantData::Integer { .. } | ConstantData::Boolean { .. },
                ) => {
                    let string = value.to_string();
                    if string.contains(char::REPLACEMENT_CHARACTER) {
                        return None;
                    }
                    let chars = string.chars().collect::<Vec<_>>();
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
                    let chars = string.chars().collect::<Vec<_>>();
                    let mut result = String::new();
                    for index in adjusted_slice_indices(chars.len(), elements)? {
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
                    let mut result = Vec::new();
                    for index in adjusted_slice_indices(value.len(), elements)? {
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
                    let elements = adjusted_slice_indices(elements.len(), slice)?
                        .into_iter()
                        .map(|index| elements[index].clone())
                        .collect();
                    Some(ConstantData::Tuple { elements })
                }
                _ => None,
            }
        }

        if matches!(op, BinOp::Subscr) {
            return eval_const_subscript(left, right);
        }

        fn constant_as_int(value: &ConstantData) -> Option<(BigInt, bool)> {
            match value {
                ConstantData::Boolean { value } => Some((BigInt::from(u8::from(*value)), true)),
                ConstantData::Integer { value } => Some((value.clone(), false)),
                _ => None,
            }
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

            return Self::eval_binop(
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
                        if !l.is_zero() && !r.is_zero() && l.bits() + r.bits() > MAX_INT_SIZE_BITS {
                            return None;
                        }
                        l * r
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
                    BinOp::Remainder => {
                        if r.is_zero() {
                            return None;
                        }
                        // Python modulo: result has same sign as divisor
                        let rem = l.clone() % r.clone();
                        if !rem.is_zero() && (rem < BigInt::from(0)) != (*r < BigInt::from(0)) {
                            rem + r
                        } else {
                            rem
                        }
                    }
                    BinOp::Power => {
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
                        if !l.is_zero() && exp > 0 && l.bits() > MAX_INT_SIZE_BITS / exp {
                            return None;
                        }
                        num_traits::pow::pow(l.clone(), exp_usize)
                    }
                    BinOp::Lshift => {
                        let shift: u64 = r.try_into().ok()?;
                        let shift_usize = usize::try_from(shift).ok()?;
                        if shift > MAX_INT_SIZE_BITS
                            || (!l.is_zero() && l.bits() > MAX_INT_SIZE_BITS - shift)
                        {
                            return None;
                        }
                        l << shift_usize
                    }
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
                    BinOp::Multiply => l * r,
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
                    BinOp::Remainder => {
                        let (_, modulo) = float_div_mod(*l, *r)?;
                        modulo
                    }
                    BinOp::Power => l.powf(*r),
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
                Self::eval_binop(
                    &ConstantData::Float { value: l_f },
                    &ConstantData::Float { value: *r },
                    op,
                )
            }
            (ConstantData::Float { value: l }, ConstantData::Integer { value: r }) => {
                let r_f = r.to_f64()?;
                Self::eval_binop(
                    &ConstantData::Float { value: *l },
                    &ConstantData::Float { value: r_f },
                    op,
                )
            }
            (ConstantData::Integer { value: l }, ConstantData::Complex { value: r }) => {
                eval_complex_binop(Complex::new(l.to_f64()?, 0.0), *r, op)
            }
            (ConstantData::Complex { value: l }, ConstantData::Integer { value: r }) => {
                eval_complex_binop(*l, Complex::new(r.to_f64()?, 0.0), op)
            }
            (ConstantData::Float { value: l }, ConstantData::Complex { value: r }) => {
                eval_complex_binop(Complex::new(*l, 0.0), *r, op)
            }
            (ConstantData::Complex { value: l }, ConstantData::Float { value: r }) => {
                eval_complex_binop(*l, Complex::new(*r, 0.0), op)
            }
            (ConstantData::Complex { value: l }, ConstantData::Complex { value: r }) => {
                eval_complex_binop(*l, *r, op)
            }
            // String concatenation and repetition
            (ConstantData::Str { value: l }, ConstantData::Str { value: r })
                if matches!(op, BinOp::Add) =>
            {
                let mut result = l.clone();
                result.push_wtf8(r);
                Some(ConstantData::Str { value: result })
            }
            (ConstantData::Str { value: s }, ConstantData::Integer { value: n })
                if matches!(op, BinOp::Multiply) =>
            {
                let n = checked_repeat_count(n, s.code_points().count())?;
                let result = repeat_wtf8(s, n);
                Some(ConstantData::Str { value: result })
            }
            (ConstantData::Tuple { elements: l }, ConstantData::Tuple { elements: r })
                if matches!(op, BinOp::Add) =>
            {
                let mut result = l.clone();
                result.extend(r.iter().cloned());
                Some(ConstantData::Tuple { elements: result })
            }
            (ConstantData::Tuple { elements }, ConstantData::Integer { value: n })
                if matches!(op, BinOp::Multiply) =>
            {
                let n = n.to_usize()?;
                if n != 0 && !elements.is_empty() {
                    if n > MAX_COLLECTION_SIZE / elements.len() {
                        return None;
                    }
                    Self::const_folding_check_complexity(
                        &ConstantData::Tuple {
                            elements: elements.clone(),
                        },
                        MAX_TOTAL_ITEMS / isize::try_from(n).ok()?,
                    )?;
                }
                let mut result = Vec::with_capacity(elements.len() * n);
                for _ in 0..n {
                    result.extend(elements.iter().cloned());
                }
                Some(ConstantData::Tuple { elements: result })
            }
            (ConstantData::Integer { value: n }, ConstantData::Tuple { elements })
                if matches!(op, BinOp::Multiply) =>
            {
                let n = n.to_usize()?;
                if n != 0 && !elements.is_empty() {
                    if n > MAX_COLLECTION_SIZE / elements.len() {
                        return None;
                    }
                    Self::const_folding_check_complexity(
                        &ConstantData::Tuple {
                            elements: elements.clone(),
                        },
                        MAX_TOTAL_ITEMS / isize::try_from(n).ok()?,
                    )?;
                }
                let mut result = Vec::with_capacity(elements.len() * n);
                for _ in 0..n {
                    result.extend(elements.iter().cloned());
                }
                Some(ConstantData::Tuple { elements: result })
            }
            (ConstantData::Integer { value: n }, ConstantData::Str { value: s })
                if matches!(op, BinOp::Multiply) =>
            {
                let n = checked_repeat_count(n, s.code_points().count())?;
                let result = repeat_wtf8(s, n);
                Some(ConstantData::Str { value: result })
            }
            (ConstantData::Bytes { value: l }, ConstantData::Bytes { value: r })
                if matches!(op, BinOp::Add) =>
            {
                let mut result = l.clone();
                result.extend_from_slice(r);
                Some(ConstantData::Bytes { value: result })
            }
            (ConstantData::Bytes { value: b }, ConstantData::Integer { value: n })
                if matches!(op, BinOp::Multiply) =>
            {
                let n = checked_repeat_count(n, b.len())?;
                Some(ConstantData::Bytes { value: b.repeat(n) })
            }
            (ConstantData::Integer { value: n }, ConstantData::Bytes { value: b })
                if matches!(op, BinOp::Multiply) =>
            {
                let n = checked_repeat_count(n, b.len())?;
                Some(ConstantData::Bytes { value: b.repeat(n) })
            }
            _ => None,
        }
    }

    fn fold_tuple_constant_at(
        metadata: &mut CodeUnitMetadata,
        block: &mut Block,
        i: usize,
    ) -> bool {
        let Some(Instruction::BuildTuple { .. }) = block.instructions[i].instr.real() else {
            return false;
        };

        let tuple_size = u32::from(block.instructions[i].arg) as usize;
        if tuple_size <= 3
            && block
                .instructions
                .get(i + 1)
                .and_then(|next| next.instr.real())
                .is_some_and(|next| {
                    matches!(
                        next,
                        Instruction::UnpackSequence { .. }
                            if usize::try_from(u32::from(block.instructions[i + 1].arg)).ok()
                                == Some(tuple_size)
                    )
                })
        {
            return false;
        }
        if tuple_size == 0 {
            let (const_idx, _) = metadata.consts.insert_full(ConstantData::Tuple {
                elements: Vec::new(),
            });
            block.instructions[i].instr = Opcode::LoadConst.into();
            block.instructions[i].arg = OpArg::new(const_idx as u32);
            return true;
        }

        let Some((operand_indices, elements)) =
            Self::get_const_sequence(metadata, block, i, tuple_size)
        else {
            return false;
        };

        let (const_idx, _) = metadata
            .consts
            .insert_full(ConstantData::Tuple { elements });

        for &j in &operand_indices {
            nop_out_no_location(&mut block.instructions[j]);
        }

        block.instructions[i].instr = Opcode::LoadConst.into();
        block.instructions[i].arg = OpArg::new(const_idx as u32);
        true
    }

    fn fold_constant_intrinsic_list_to_tuple_at(
        metadata: &mut CodeUnitMetadata,
        block: &mut Block,
        i: usize,
    ) -> bool {
        let Some(Instruction::CallIntrinsic1 { func }) = block.instructions[i].instr.real() else {
            return false;
        };
        if func.get(block.instructions[i].arg) != IntrinsicFunction1::ListToTuple {
            return false;
        }
        if block
            .instructions
            .get(i + 1)
            .and_then(|instr| instr.instr.real())
            .is_some_and(|instr| matches!(instr, Instruction::GetIter))
        {
            return false;
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
                    return false;
                }

                let mut elements = Vec::with_capacity(consts_found);
                let mut expect_load = true;
                for idx in pos + 1..i {
                    let instr = &block.instructions[idx];
                    if matches!(instr.instr.real(), Some(Instruction::Nop)) {
                        continue;
                    }
                    if expect_load {
                        let Some(value) = Self::get_const_value_from(metadata, instr) else {
                            return false;
                        };
                        elements.push(value);
                    } else if !matches!(instr.instr.real(), Some(Instruction::ListAppend { .. }))
                        || u32::from(instr.arg) != 1
                    {
                        return false;
                    }
                    expect_load = !expect_load;
                }
                if !expect_load || elements.len() != consts_found {
                    return false;
                }

                let (const_idx, _) = metadata
                    .consts
                    .insert_full(ConstantData::Tuple { elements });
                for idx in pos..i {
                    nop_out_no_location(&mut block.instructions[idx]);
                }
                block.instructions[i].instr = Instruction::LoadConst {
                    consti: Arg::marker(),
                }
                .into();
                block.instructions[i].arg = OpArg::new(const_idx as u32);
                return true;
            }

            if expect_append {
                if !matches!(instr.instr.real(), Some(Instruction::ListAppend { .. }))
                    || u32::from(instr.arg) != 1
                {
                    return false;
                }
            } else {
                if Self::get_const_value_from_dummy(instr).is_none() {
                    return false;
                }
                consts_found += 1;
            }
            expect_append = !expect_append;
        }

        false
    }

    fn fold_list_constant_at(metadata: &mut CodeUnitMetadata, block: &mut Block, i: usize) -> bool {
        let Some(Instruction::BuildList { .. }) = block.instructions[i].instr.real() else {
            return false;
        };

        let list_size = u32::from(block.instructions[i].arg) as usize;
        if list_size == 0 || list_size > STACK_USE_GUIDELINE {
            return false;
        }

        let Some((operand_indices, elements)) =
            Self::get_const_sequence(metadata, block, i, list_size)
        else {
            return false;
        };
        if list_size < MIN_CONST_SEQUENCE_SIZE {
            return false;
        }

        let (const_idx, _) = metadata
            .consts
            .insert_full(ConstantData::Tuple { elements });

        let folded_loc = block.instructions[i].location;
        let end_loc = block.instructions[i].end_location;
        let eh = block.instructions[i].except_handler;

        let build_idx = operand_indices[0];
        let const_idx_slot = operand_indices[1];

        block.instructions[build_idx].instr = Instruction::BuildList {
            count: Arg::marker(),
        }
        .into();
        block.instructions[build_idx].arg = OpArg::new(0);
        block.instructions[build_idx].location = folded_loc;
        block.instructions[build_idx].end_location = end_loc;
        block.instructions[build_idx].except_handler = eh;

        block.instructions[const_idx_slot].instr = Instruction::LoadConst {
            consti: Arg::marker(),
        }
        .into();
        block.instructions[const_idx_slot].arg = OpArg::new(const_idx as u32);
        block.instructions[const_idx_slot].location = folded_loc;
        block.instructions[const_idx_slot].end_location = end_loc;
        block.instructions[const_idx_slot].except_handler = eh;

        for &j in &operand_indices[2..] {
            set_to_nop(&mut block.instructions[j]);
            block.instructions[j].location = folded_loc;
        }

        block.instructions[i].instr = Opcode::ListExtend.into();
        block.instructions[i].arg = OpArg::new(1);
        true
    }

    /// Port of CPython's flowgraph.c optimize_lists_and_sets().
    fn optimize_lists_and_sets_at(
        metadata: &mut CodeUnitMetadata,
        block: &mut Block,
        i: usize,
        nextop: Option<Instruction>,
    ) -> bool {
        let Some(instr) = block.instructions[i].instr.real() else {
            return false;
        };
        let is_list = matches!(instr, Instruction::BuildList { .. });
        let is_set = matches!(instr, Instruction::BuildSet { .. });
        if !is_list && !is_set {
            return false;
        }

        let contains_or_iter = matches!(
            nextop,
            Some(Instruction::GetIter | Instruction::ContainsOp { .. })
        );
        let seq_size = u32::from(block.instructions[i].arg) as usize;
        if seq_size > STACK_USE_GUIDELINE
            || (seq_size < MIN_CONST_SEQUENCE_SIZE && !contains_or_iter)
        {
            return false;
        }

        let Some((operand_indices, elements)) =
            Self::get_const_sequence(metadata, block, i, seq_size)
        else {
            if contains_or_iter && is_list {
                block.instructions[i].instr = Opcode::BuildTuple.into();
                return true;
            }
            return false;
        };

        if !contains_or_iter {
            return if is_list {
                Self::fold_list_constant_at(metadata, block, i)
            } else {
                Self::fold_set_constant_at(metadata, block, i)
            };
        }

        let const_data = if is_list {
            ConstantData::Tuple { elements }
        } else {
            ConstantData::Frozenset { elements }
        };
        let (const_idx, _) = metadata.consts.insert_full(const_data);
        let folded_loc = block.instructions[i].location;
        let end_loc = block.instructions[i].end_location;
        let eh = block.instructions[i].except_handler;

        for &j in &operand_indices {
            set_to_nop(&mut block.instructions[j]);
            block.instructions[j].location = folded_loc;
            block.instructions[j].end_location = end_loc;
        }

        block.instructions[i].instr = Opcode::LoadConst.into();
        block.instructions[i].arg = OpArg::new(const_idx as u32);
        block.instructions[i].location = folded_loc;
        block.instructions[i].end_location = end_loc;
        block.instructions[i].except_handler = eh;
        true
    }

    fn fold_set_constant_at(metadata: &mut CodeUnitMetadata, block: &mut Block, i: usize) -> bool {
        let Some(Instruction::BuildSet { .. }) = block.instructions[i].instr.real() else {
            return false;
        };

        let set_size = u32::from(block.instructions[i].arg) as usize;
        if !(3..=STACK_USE_GUIDELINE).contains(&set_size) {
            return false;
        }

        let Some((operand_indices, elements)) =
            Self::get_const_sequence(metadata, block, i, set_size)
        else {
            return false;
        };
        let (const_idx, _) = metadata
            .consts
            .insert_full(ConstantData::Frozenset { elements });

        let folded_loc = block.instructions[i].location;
        let end_loc = block.instructions[i].end_location;
        let eh = block.instructions[i].except_handler;

        let build_idx = operand_indices[0];
        let const_idx_slot = operand_indices[1];

        block.instructions[build_idx].instr = Instruction::BuildSet {
            count: Arg::marker(),
        }
        .into();
        block.instructions[build_idx].arg = OpArg::new(0);
        block.instructions[build_idx].location = folded_loc;
        block.instructions[build_idx].end_location = end_loc;
        block.instructions[build_idx].except_handler = eh;

        block.instructions[const_idx_slot].instr = Instruction::LoadConst {
            consti: Arg::marker(),
        }
        .into();
        block.instructions[const_idx_slot].arg = OpArg::new(const_idx as u32);
        block.instructions[const_idx_slot].location = folded_loc;
        block.instructions[const_idx_slot].end_location = end_loc;
        block.instructions[const_idx_slot].except_handler = eh;

        for &j in &operand_indices[2..] {
            set_to_nop(&mut block.instructions[j]);
            block.instructions[j].location = folded_loc;
        }

        block.instructions[i].instr = Opcode::SetUpdate.into();
        block.instructions[i].arg = OpArg::new(1);
        true
    }

    /// apply_static_swaps: eliminate SWAPs by reordering target stores/pops.
    ///
    /// Ported from CPython Python/flowgraph.c::apply_static_swaps.
    /// For each SWAP N, find the 1st and N-th swappable instructions after
    /// it. If both are STORE_FAST/POP_TOP and safe to swap, exchange them
    /// in the bytecode and replace SWAP with NOP.
    ///
    /// Safety: abort if the two stores write the same variable, or if any
    /// intervening swappable stores to one of the same variables. Do not
    /// cross line-number boundaries (user-visible name bindings).
    fn apply_static_swaps_block(block: &mut Block) {
        const VISITED: i32 = -1;

        /// Instruction classes that are safe to reorder around SWAP.
        fn is_swappable(instr: &AnyInstruction) -> bool {
            matches!(
                (*instr).into(),
                AnyOpcode::Real(Opcode::StoreFast | Opcode::PopTop)
                    | AnyOpcode::Pseudo(PseudoOpcode::StoreFastMaybeNull)
            )
        }

        /// Variable index that a STORE_FAST writes to, or None.
        fn stores_to(info: &InstructionInfo) -> Option<u32> {
            match info.instr.into() {
                AnyOpcode::Real(Opcode::StoreFast) => Some(u32::from(info.arg)),
                AnyOpcode::Pseudo(PseudoOpcode::StoreFastMaybeNull) => Some(u32::from(info.arg)),
                _ => None,
            }
        }

        /// Next swappable index after `i` in `instructions`, skipping NOPs.
        /// Returns None if a non-NOP non-swappable instruction blocks, or
        /// if `lineno >= 0` and a different lineno is encountered.
        fn next_swappable(
            instructions: &[InstructionInfo],
            mut i: usize,
            lineno: i32,
        ) -> Option<usize> {
            loop {
                i += 1;
                if i >= instructions.len() {
                    return None;
                }
                let info = &instructions[i];
                let info_lineno = info.location.line.get() as i32;
                if lineno >= 0 && info_lineno > 0 && info_lineno != lineno {
                    return None;
                }
                if matches!(info.instr, AnyInstruction::Real(Instruction::Nop)) {
                    continue;
                }
                if is_swappable(&info.instr) {
                    return Some(i);
                }
                return None;
            }
        }

        fn optimize_swap_block(instructions: &mut [InstructionInfo]) {
            let mut i = 0usize;
            while i < instructions.len() {
                let AnyInstruction::Real(Instruction::Swap { .. }) = instructions[i].instr else {
                    i += 1;
                    continue;
                };

                let mut len = 0usize;
                let mut depth = 0usize;
                let mut more = false;
                while i + len < instructions.len() {
                    let info = &instructions[i + len];
                    match info.instr.real() {
                        Some(Instruction::Swap { .. }) => {
                            let oparg = u32::from(info.arg) as usize;
                            depth = depth.max(oparg);
                            more |= len > 0;
                            len += 1;
                        }
                        Some(Instruction::Nop) => {
                            len += 1;
                        }
                        _ => break,
                    }
                }

                if !more {
                    i += len.max(1);
                    continue;
                }

                let mut stack: Vec<i32> = (0..depth as i32).collect();
                for info in &instructions[i..i + len] {
                    if matches!(info.instr.real(), Some(Instruction::Swap { .. })) {
                        let oparg = u32::from(info.arg) as usize;
                        stack.swap(0, oparg - 1);
                    }
                }

                let mut current = len as isize - 1;
                for slot in 0..depth {
                    if stack[slot] == VISITED || stack[slot] == slot as i32 {
                        continue;
                    }
                    let mut j = slot;
                    loop {
                        if j != 0 {
                            let out = &mut instructions[i + current as usize];
                            out.instr = Opcode::Swap.into();
                            out.arg = OpArg::new((j + 1) as u32);
                            out.target = BlockIdx::NULL;
                            current -= 1;
                        }
                        if stack[j] == VISITED {
                            debug_assert_eq!(j, slot);
                            break;
                        }
                        let next_j = stack[j] as usize;
                        stack[j] = VISITED;
                        j = next_j;
                    }
                }
                while current >= 0 {
                    set_to_nop(&mut instructions[i + current as usize]);
                    current -= 1;
                }
                i += len;
            }
        }

        fn apply_from(instructions: &mut [InstructionInfo], mut i: isize) {
            while i >= 0 {
                let idx = i as usize;
                let swap_arg = match instructions[idx].instr.real() {
                    Some(Instruction::Swap { .. }) => u32::from(instructions[idx].arg),
                    Some(
                        Instruction::Nop | Instruction::PopTop | Instruction::StoreFast { .. },
                    ) => {
                        i -= 1;
                        continue;
                    }
                    _ if matches!(
                        instructions[idx].instr.pseudo(),
                        Some(PseudoInstruction::StoreFastMaybeNull { .. })
                    ) =>
                    {
                        i -= 1;
                        continue;
                    }
                    _ => return,
                };

                if swap_arg < 2 {
                    return;
                }

                let Some(j) = next_swappable(instructions, idx, -1) else {
                    return;
                };
                let lineno = instructions[j].location.line.get() as i32;
                let mut k = j;
                for _ in 1..swap_arg {
                    let Some(next) = next_swappable(instructions, k, lineno) else {
                        return;
                    };
                    k = next;
                }

                let store_j = stores_to(&instructions[j]);
                let store_k = stores_to(&instructions[k]);
                if store_j.is_some() || store_k.is_some() {
                    if store_j == store_k {
                        return;
                    }
                    let conflict = instructions[(j + 1)..k].iter().any(|info| {
                        if let Some(store_idx) = stores_to(info) {
                            Some(store_idx) == store_j || Some(store_idx) == store_k
                        } else {
                            false
                        }
                    });
                    if conflict {
                        return;
                    }
                }

                instructions[idx].instr = Opcode::Nop.into();
                instructions[idx].arg = OpArg::new(0);
                instructions.swap(j, k);
                i -= 1;
            }
        }

        optimize_swap_block(&mut block.instructions);
        let len = block.instructions.len();
        for i in 0..len {
            if matches!(
                block.instructions[i].instr.real(),
                Some(Instruction::Swap { .. })
            ) {
                apply_from(&mut block.instructions, i as isize);
            }
        }
    }

    /// Peephole optimization: combine consecutive instructions into super-instructions
    fn peephole_optimize(&mut self) {
        let const_truthiness =
            |instr: Instruction, arg: OpArg, metadata: &CodeUnitMetadata| match instr {
                Instruction::LoadConst { consti } => {
                    let constant = &metadata.consts[consti.get(arg).as_usize()];
                    match constant {
                        ConstantData::Tuple { .. } => None,
                        ConstantData::Integer { value } => Some(!value.is_zero()),
                        ConstantData::Float { value } => Some(*value != 0.0),
                        ConstantData::Complex { value } => Some(value.re != 0.0 || value.im != 0.0),
                        ConstantData::Boolean { value } => Some(*value),
                        ConstantData::Str { value } => Some(!value.is_empty()),
                        ConstantData::Bytes { value } => Some(!value.is_empty()),
                        ConstantData::Code { .. } => Some(true),
                        ConstantData::Slice { .. } => Some(true),
                        ConstantData::Frozenset { elements } => Some(!elements.is_empty()),
                        ConstantData::None => Some(false),
                        ConstantData::Ellipsis => Some(true),
                    }
                }
                Instruction::LoadSmallInt { i } => Some(i.get(arg) != 0),
                _ => None,
            };
        for block_idx in self.block_next_order() {
            let block = &mut self.blocks[block_idx];
            let mut i = 0;
            while i + 1 < block.instructions.len() {
                let curr = &block.instructions[i];
                let next = &block.instructions[i + 1];
                let curr_arg = curr.arg;
                let next_arg = next.arg;

                // Only combine if the source is a real instruction.
                let Some(curr_instr) = curr.instr.real() else {
                    i += 1;
                    continue;
                };

                if let Some(is_true) = const_truthiness(curr_instr, curr.arg, &self.metadata) {
                    let jump_if_true = match next.instr.pseudo() {
                        Some(PseudoInstruction::JumpIfTrue { .. }) => Some(true),
                        Some(PseudoInstruction::JumpIfFalse { .. }) => Some(false),
                        _ => None,
                    };
                    if let Some(jump_if_true) = jump_if_true {
                        // CPython flowgraph.c::basicblock_optimize_load_const()
                        // folds LOAD_CONST/LOAD_SMALL_INT followed by
                        // JUMP_IF_TRUE/FALSE. Unlike POP_JUMP_IF_*, these
                        // pseudo jumps do not consume the condition, so keep
                        // the constant for the following POP_TOP pair removal.
                        if is_true == jump_if_true {
                            block.instructions[i + 1].instr = PseudoInstruction::Jump {
                                delta: Arg::marker(),
                            }
                            .into();
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

                if let Some(is_true) = const_truthiness(curr_instr, curr.arg, &self.metadata) {
                    let jump_if_true = match next_instr {
                        Instruction::PopJumpIfTrue { .. } => Some(true),
                        Instruction::PopJumpIfFalse { .. } => Some(false),
                        _ => None,
                    };
                    if let Some(jump_if_true) = jump_if_true {
                        let target = match next_instr {
                            Instruction::PopJumpIfTrue { delta }
                            | Instruction::PopJumpIfFalse { delta } => delta.get(next.arg),
                            _ => unreachable!(),
                        };
                        set_to_nop(&mut block.instructions[i]);
                        if is_true == jump_if_true {
                            block.instructions[i + 1].instr = PseudoInstruction::Jump {
                                delta: Arg::marker(),
                            }
                            .into();
                            block.instructions[i + 1].arg = OpArg::new(u32::from(target));
                        } else {
                            set_to_nop(&mut block.instructions[i + 1]);
                        }
                        i += 1;
                        continue;
                    }
                }

                if let Instruction::LoadConst { consti } = curr_instr {
                    let constant = &self.metadata.consts[consti.get(curr_arg).as_usize()];
                    if matches!(constant, ConstantData::None)
                        && let Instruction::IsOp { invert } = next_instr
                    {
                        let mut jump_idx = i + 2;
                        if jump_idx >= block.instructions.len() {
                            i += 1;
                            continue;
                        }

                        if matches!(
                            block.instructions[jump_idx].instr.real(),
                            Some(Instruction::ToBool)
                        ) {
                            set_to_nop(&mut block.instructions[jump_idx]);
                            jump_idx += 1;
                            if jump_idx >= block.instructions.len() {
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
                        let delta = match jump_instr {
                            Instruction::PopJumpIfFalse { delta } => {
                                invert = !invert;
                                delta.get(block.instructions[jump_idx].arg)
                            }
                            Instruction::PopJumpIfTrue { delta } => {
                                delta.get(block.instructions[jump_idx].arg)
                            }
                            _ => {
                                i += 1;
                                continue;
                            }
                        };

                        set_to_nop(&mut block.instructions[i]);
                        set_to_nop(&mut block.instructions[i + 1]);
                        block.instructions[jump_idx].instr = if invert {
                            Instruction::PopJumpIfNotNone {
                                delta: Arg::marker(),
                            }
                        } else {
                            Instruction::PopJumpIfNone {
                                delta: Arg::marker(),
                            }
                        }
                        .into();
                        block.instructions[jump_idx].arg = OpArg::new(u32::from(delta));
                        i = jump_idx;
                        continue;
                    }
                }

                if matches!(
                    curr_instr,
                    Instruction::LoadConst { .. } | Instruction::LoadSmallInt { .. }
                ) && matches!(next_instr, Instruction::ToBool)
                    && let Some(value) = const_truthiness(curr_instr, curr.arg, &self.metadata)
                {
                    let (const_idx, _) = self
                        .metadata
                        .consts
                        .insert_full(ConstantData::Boolean { value });
                    set_to_nop(&mut block.instructions[i]);
                    block.instructions[i + 1].instr = Instruction::LoadConst {
                        consti: Arg::marker(),
                    }
                    .into();
                    block.instructions[i + 1].arg = OpArg::new(const_idx as u32);
                    i += 1;
                    continue;
                }

                if let (Instruction::LoadConst { consti }, Instruction::UnaryNot) =
                    (curr_instr, next_instr)
                {
                    let constant = &self.metadata.consts[consti.get(curr.arg).as_usize()];
                    if let ConstantData::Boolean { value } = constant {
                        let (const_idx, _) = self
                            .metadata
                            .consts
                            .insert_full(ConstantData::Boolean { value: !value });
                        set_to_nop(&mut block.instructions[i]);
                        block.instructions[i + 1].instr = Instruction::LoadConst {
                            consti: Arg::marker(),
                        }
                        .into();
                        block.instructions[i + 1].arg = OpArg::new(const_idx as u32);
                        i += 1;
                        continue;
                    }
                }

                i += 1;
            }
        }
    }

    /// flowgraph.c optimize_basic_block
    fn optimize_basic_blocks(&mut self) -> crate::InternalResult<()> {
        for block_idx in self.block_next_order() {
            {
                let metadata = &mut self.metadata;
                let block = &mut self.blocks[block_idx];
                let mut i = 0;
                while i < block.instructions.len() {
                    let inst = block.instructions[i];
                    let Some(opcode) = inst.instr.real() else {
                        i += 1;
                        continue;
                    };
                    let nextop = block
                        .instructions
                        .get(i + 1)
                        .and_then(|next| next.instr.real());

                    match opcode {
                        Instruction::BuildTuple { .. } => {
                            let oparg = u32::from(inst.arg);
                            if matches!(nextop, Some(Instruction::UnpackSequence { .. }))
                                && u32::from(block.instructions[i + 1].arg) == oparg
                            {
                                match oparg {
                                    1 => {
                                        set_to_nop(&mut block.instructions[i]);
                                        set_to_nop(&mut block.instructions[i + 1]);
                                        i += 1;
                                        continue;
                                    }
                                    2 | 3 => {
                                        set_to_nop(&mut block.instructions[i]);
                                        block.instructions[i + 1].instr =
                                            Instruction::Swap { i: Arg::marker() }.into();
                                        block.instructions[i + 1].arg = OpArg::new(oparg);
                                        i += 1;
                                        continue;
                                    }
                                    _ => {}
                                }
                            }
                            Self::fold_tuple_constant_at(metadata, block, i);
                        }
                        Instruction::BuildList { .. } | Instruction::BuildSet { .. } => {
                            Self::optimize_lists_and_sets_at(metadata, block, i, nextop);
                        }
                        Instruction::StoreFast { .. }
                            if matches!(nextop, Some(Instruction::StoreFast { .. }))
                                && u32::from(inst.arg)
                                    == u32::from(block.instructions[i + 1].arg)
                                && instruction_lineno(&block.instructions[i])
                                    == instruction_lineno(&block.instructions[i + 1]) =>
                        {
                            block.instructions[i].instr = Instruction::PopTop.into();
                            block.instructions[i].arg = OpArg::NULL;
                            block.instructions[i].target = BlockIdx::NULL;
                        }
                        Instruction::Swap { .. } if u32::from(inst.arg) == 1 => {
                            set_to_nop(&mut block.instructions[i]);
                        }
                        Instruction::LoadGlobal { .. }
                            if matches!(nextop, Some(Instruction::PushNull))
                                && (u32::from(inst.arg) & 1) == 0 =>
                        {
                            block.instructions[i].arg = OpArg::new(u32::from(inst.arg) | 1);
                            set_to_nop(&mut block.instructions[i + 1]);
                        }
                        Instruction::CompareOp { .. }
                            if matches!(nextop, Some(Instruction::ToBool)) =>
                        {
                            set_to_nop(&mut block.instructions[i]);
                            block.instructions[i + 1].instr = Instruction::CompareOp {
                                opname: Arg::marker(),
                            }
                            .into();
                            block.instructions[i + 1].arg =
                                OpArg::new(u32::from(inst.arg) | oparg::COMPARE_OP_BOOL_MASK);
                            i += 1;
                            continue;
                        }
                        Instruction::ContainsOp { .. } | Instruction::IsOp { .. }
                            if matches!(nextop, Some(Instruction::ToBool)) =>
                        {
                            set_to_nop(&mut block.instructions[i]);
                            block.instructions[i + 1].instr = opcode.into();
                            block.instructions[i + 1].arg = inst.arg;
                            i += 1;
                            continue;
                        }
                        Instruction::ContainsOp { .. } | Instruction::IsOp { .. }
                            if matches!(nextop, Some(Instruction::UnaryNot)) =>
                        {
                            set_to_nop(&mut block.instructions[i]);
                            block.instructions[i + 1].instr = opcode.into();
                            block.instructions[i + 1].arg = OpArg::new(u32::from(inst.arg) ^ 1);
                            i += 1;
                            continue;
                        }
                        Instruction::ToBool if matches!(nextop, Some(Instruction::ToBool)) => {
                            set_to_nop(&mut block.instructions[i]);
                            i += 1;
                            continue;
                        }
                        Instruction::UnaryNot => {
                            if matches!(nextop, Some(Instruction::ToBool)) {
                                set_to_nop(&mut block.instructions[i]);
                                block.instructions[i + 1].instr = Instruction::UnaryNot.into();
                                block.instructions[i + 1].arg = OpArg::new(0);
                                i += 1;
                                continue;
                            }
                            if matches!(nextop, Some(Instruction::UnaryNot)) {
                                set_to_nop(&mut block.instructions[i]);
                                set_to_nop(&mut block.instructions[i + 1]);
                                i += 1;
                                continue;
                            }
                            Self::fold_unary_constant_at(metadata, block, i);
                        }
                        Instruction::UnaryInvert | Instruction::UnaryNegative => {
                            Self::fold_unary_constant_at(metadata, block, i);
                        }
                        Instruction::CallIntrinsic1 { func } => match func.get(inst.arg) {
                            IntrinsicFunction1::ListToTuple => {
                                if matches!(nextop, Some(Instruction::GetIter)) {
                                    set_to_nop(&mut block.instructions[i]);
                                } else {
                                    Self::fold_constant_intrinsic_list_to_tuple_at(
                                        metadata, block, i,
                                    );
                                }
                            }
                            IntrinsicFunction1::UnaryPositive => {
                                Self::fold_unary_constant_at(metadata, block, i);
                            }
                            _ => {}
                        },
                        Instruction::BinaryOp { .. } => {
                            Self::fold_binop_constant_at(metadata, block, i);
                        }
                        _ => {}
                    }

                    i += 1;
                }
            }
            jump_threading_block(&mut self.blocks, block_idx)?;
            Self::apply_static_swaps_block(&mut self.blocks[block_idx]);
        }
        Ok(())
    }

    /// flowgraph.c remove_redundant_nops_and_pairs
    fn remove_redundant_nops_and_pairs(&mut self) {
        loop {
            let mut changed = false;
            let mut prev: Option<(BlockIdx, usize)> = None;
            let mut block_idx = BlockIdx::new(0);

            while block_idx != BlockIdx::NULL {
                basicblock_remove_redundant_nops(&mut self.blocks, block_idx);
                if self.blocks[block_idx.idx()].has_cpython_cfg_label() {
                    prev = None;
                }

                let len = self.blocks[block_idx.idx()].instructions.len();
                for instr_idx in 0..len {
                    let instr = self.blocks[block_idx.idx()].instructions[instr_idx];
                    let is_redundant_pair =
                        if matches!(instr.instr.real(), Some(Instruction::PopTop))
                            && let Some((prev_block, prev_instr)) = prev
                        {
                            let prev_info = self.blocks[prev_block.idx()].instructions[prev_instr];
                            matches!(
                                prev_info.instr.real(),
                                Some(
                                    Instruction::LoadConst { .. }
                                        | Instruction::LoadSmallInt { .. }
                                )
                            ) || matches!(
                                prev_info.instr.real(),
                                Some(Instruction::Copy { i }) if i.get(prev_info.arg) == 1
                            )
                        } else {
                            false
                        };

                    if is_redundant_pair {
                        let (prev_block, prev_instr) = prev.expect("redundant pair has previous");
                        set_to_nop(&mut self.blocks[prev_block.idx()].instructions[prev_instr]);
                        set_to_nop(&mut self.blocks[block_idx.idx()].instructions[instr_idx]);
                        changed = true;
                    }
                    prev = Some((block_idx, instr_idx));
                }

                let block = &self.blocks[block_idx.idx()];
                if block
                    .instructions
                    .last()
                    .is_some_and(|info| info.instr.is_unconditional_jump())
                    || !block_has_fallthrough(block)
                {
                    prev = None;
                }
                block_idx = block.next;
            }

            if !changed {
                break;
            }
        }
    }

    /// Convert LOAD_CONST for small integers to LOAD_SMALL_INT
    /// maybe_instr_make_load_smallint
    fn convert_to_load_small_int(&mut self) {
        for block_idx in self.block_next_order() {
            let block = &mut self.blocks[block_idx];
            for instr in &mut block.instructions {
                // Check if it's a LOAD_CONST instruction
                let Some(Instruction::LoadConst { .. }) = instr.instr.real() else {
                    continue;
                };

                // Get the constant value
                let const_idx = u32::from(instr.arg) as usize;
                let Some(constant) = self.metadata.consts.get_index(const_idx) else {
                    continue;
                };

                // Check if it's a small integer
                let ConstantData::Integer { value } = constant else {
                    continue;
                };

                // LOAD_SMALL_INT oparg is unsigned, so only 0..=255 can be encoded
                if let Some(small) = value.to_i32().filter(|v| (0..=255).contains(v)) {
                    // Convert LOAD_CONST to LOAD_SMALL_INT
                    instr.instr = Opcode::LoadSmallInt.into();
                    // The arg is the i32 value stored as u32 (two's complement)
                    instr.arg = OpArg::new(small as u32);
                }
            }
        }
    }

    /// Remove constants that are no longer referenced by LOAD_CONST instructions.
    /// remove_unused_consts
    fn remove_unused_consts(&mut self) {
        let nconsts = self.metadata.consts.len();
        if nconsts == 0 {
            return;
        }

        // Mark used constants
        // The first constant (index 0) is always kept (may be docstring)
        let mut used = vec![false; nconsts];
        used[0] = true;

        for block_idx in self.block_next_order() {
            let block = &self.blocks[block_idx];
            for instr in &block.instructions {
                if let Some(Instruction::LoadConst { .. }) = instr.instr.real() {
                    let idx = u32::from(instr.arg) as usize;
                    if idx < nconsts {
                        used[idx] = true;
                    }
                }
            }
        }

        // Check if any constants can be removed
        let n_used: usize = used.iter().filter(|&&u| u).count();
        if n_used == nconsts {
            return; // Nothing to remove
        }

        // Build old_to_new index mapping
        let mut old_to_new = vec![0usize; nconsts];
        let mut new_idx = 0usize;
        for (old_idx, &is_used) in used.iter().enumerate() {
            if is_used {
                old_to_new[old_idx] = new_idx;
                new_idx += 1;
            }
        }

        // Build new consts list
        let old_consts: Vec<_> = self.metadata.consts.iter().cloned().collect();
        self.metadata.consts.clear();
        for (old_idx, constant) in old_consts.into_iter().enumerate() {
            if used[old_idx] {
                self.metadata.consts.insert(constant);
            }
        }

        // Update LOAD_CONST instruction arguments
        for block_idx in self.block_next_order() {
            let block = &mut self.blocks[block_idx];
            for instr in &mut block.instructions {
                if let Some(Instruction::LoadConst { .. }) = instr.instr.real() {
                    let old_idx = u32::from(instr.arg) as usize;
                    if old_idx < nconsts {
                        instr.arg = OpArg::new(old_to_new[old_idx] as u32);
                    }
                }
            }
        }
    }

    /// insert_superinstructions (flowgraph.c): combine adjacent same-line
    /// LOAD_FAST / STORE_FAST pairs before later flowgraph passes change
    /// block layout.
    fn insert_superinstructions(&mut self) {
        for block_idx in self.block_next_order() {
            let block = &mut self.blocks[block_idx];
            let mut i = 0;
            while i + 1 < block.instructions.len() {
                let curr = &block.instructions[i];
                let next = &block.instructions[i + 1];
                let curr_line = instruction_lineno(curr);
                let next_line = instruction_lineno(next);
                if curr_line >= 0 && next_line >= 0 && curr_line != next_line {
                    i += 1;
                    continue;
                }

                match (curr.instr.real(), next.instr.real()) {
                    (Some(Instruction::LoadFast { .. }), Some(Instruction::LoadFast { .. })) => {
                        let idx1 = u32::from(curr.arg);
                        let idx2 = u32::from(next.arg);
                        if idx1 >= 16 || idx2 >= 16 {
                            i += 1;
                            continue;
                        }
                        let packed = (idx1 << 4) | idx2;
                        block.instructions[i].instr = Instruction::LoadFastLoadFast {
                            var_nums: Arg::marker(),
                        }
                        .into();
                        block.instructions[i].arg = OpArg::new(packed);
                        set_to_nop(&mut block.instructions[i + 1]);
                        i += 1;
                    }
                    (Some(Instruction::StoreFast { .. }), Some(Instruction::LoadFast { .. })) => {
                        let store_idx = u32::from(curr.arg);
                        let load_idx = u32::from(next.arg);
                        if store_idx >= 16 || load_idx >= 16 {
                            i += 1;
                            continue;
                        }
                        let packed = (store_idx << 4) | load_idx;
                        block.instructions[i].instr = Instruction::StoreFastLoadFast {
                            var_nums: Arg::marker(),
                        }
                        .into();
                        block.instructions[i].arg = OpArg::new(packed);
                        set_to_nop(&mut block.instructions[i + 1]);
                        i += 1;
                    }
                    (Some(Instruction::StoreFast { .. }), Some(Instruction::StoreFast { .. })) => {
                        let idx1 = u32::from(curr.arg);
                        let idx2 = u32::from(next.arg);
                        if idx1 >= 16 || idx2 >= 16 {
                            i += 1;
                            continue;
                        }
                        let packed = (idx1 << 4) | idx2;
                        block.instructions[i].instr = Instruction::StoreFastStoreFast {
                            var_nums: Arg::marker(),
                        }
                        .into();
                        block.instructions[i].arg = OpArg::new(packed);
                        set_to_nop(&mut block.instructions[i + 1]);
                        i += 1;
                    }
                    _ => i += 1,
                }
            }
        }
        remove_redundant_nops(&mut self.blocks);
        debug_assert!(no_redundant_nops(&self.blocks));
    }

    fn optimize_load_fast_borrow(&mut self) {
        // NOT_LOCAL marker: instruction didn't come from a LOAD_FAST
        const NOT_LOCAL: usize = usize::MAX;
        const DUMMY_INSTR: isize = -1;
        const SUPPORT_KILLED: u8 = 1;
        const STORED_AS_LOCAL: u8 = 2;
        const REF_UNCONSUMED: u8 = 4;

        #[derive(Clone, Copy)]
        struct AbstractRef {
            instr: isize,
            local: usize,
        }

        fn push_ref(refs: &mut Vec<AbstractRef>, instr: isize, local: usize) {
            refs.push(AbstractRef { instr, local });
        }

        fn pop_ref(refs: &mut Vec<AbstractRef>) -> AbstractRef {
            refs.pop().expect("ref stack underflow")
        }

        fn at_ref(refs: &[AbstractRef], idx: usize) -> AbstractRef {
            refs.get(idx).copied().expect("ref stack index in bounds")
        }

        fn swap_top(refs: &mut [AbstractRef], depth: usize) {
            assert!(depth >= 2 && refs.len() >= depth);
            let top = refs.len() - 1;
            let other = refs.len() - depth;
            refs.swap(top, other);
        }

        fn kill_local(instr_flags: &mut [u8], refs: &[AbstractRef], local: usize) {
            for r in refs.iter().copied().filter(|r| r.local == local) {
                debug_assert!(r.instr >= 0);
                instr_flags[r.instr as usize] |= SUPPORT_KILLED;
            }
        }

        fn store_local(instr_flags: &mut [u8], refs: &[AbstractRef], local: usize, r: AbstractRef) {
            kill_local(instr_flags, refs, local);
            if r.instr != DUMMY_INSTR {
                instr_flags[r.instr as usize] |= STORED_AS_LOCAL;
            }
        }

        fn decode_packed_fast_locals(arg: OpArg) -> (usize, usize) {
            let packed = u32::from(arg);
            (((packed >> 4) & 0xF) as usize, (packed & 0xF) as usize)
        }

        fn push_block(
            worklist: &mut Vec<BlockIdx>,
            visited: &mut [bool],
            blocks: &[Block],
            source: BlockIdx,
            target: BlockIdx,
            start_depth: usize,
        ) {
            debug_assert!(target != BlockIdx::NULL);
            let expected = blocks[target.idx()].start_depth.map(|depth| depth as usize);
            debug_assert!(
                expected == Some(start_depth),
                "optimize_load_fast_borrow start_depth mismatch: source={source:?} target={target:?} expected={expected:?} actual={:?} source_last={:?} target_instrs={:?}",
                Some(start_depth),
                blocks[source.idx()]
                    .instructions
                    .last()
                    .and_then(|info| info.instr.real()),
                blocks[target.idx()]
                    .instructions
                    .iter()
                    .map(|info| info.instr)
                    .collect::<Vec<_>>(),
            );
            if !visited[target.idx()] {
                visited[target.idx()] = true;
                worklist.push(target);
            }
        }

        let mut visited = vec![false; self.blocks.len()];
        let mut worklist = vec![BlockIdx(0)];
        visited[0] = true;
        while let Some(block_idx) = worklist.pop() {
            let block = &self.blocks[block_idx];

            let mut instr_flags = vec![0u8; block.instructions.len()];
            let start_depth = block.start_depth.unwrap_or(0) as usize;
            let mut refs = Vec::with_capacity(block.instructions.len() + start_depth + 2);
            for _ in 0..start_depth {
                push_ref(&mut refs, DUMMY_INSTR, NOT_LOCAL);
            }

            for (i, info) in block.instructions.iter().enumerate() {
                let instr = info.instr;
                let arg_u32 = u32::from(info.arg);

                match instr {
                    AnyInstruction::Real(Instruction::DeleteFast { var_num }) => {
                        kill_local(&mut instr_flags, &refs, usize::from(var_num.get(info.arg)));
                    }
                    AnyInstruction::Real(Instruction::LoadFast { var_num }) => {
                        push_ref(&mut refs, i as isize, usize::from(var_num.get(info.arg)));
                    }
                    AnyInstruction::Real(Instruction::LoadFastAndClear { var_num }) => {
                        let local = usize::from(var_num.get(info.arg));
                        kill_local(&mut instr_flags, &refs, local);
                        push_ref(&mut refs, i as isize, local);
                    }
                    AnyInstruction::Real(Instruction::LoadFastLoadFast { .. }) => {
                        let (local1, local2) = decode_packed_fast_locals(info.arg);
                        push_ref(&mut refs, i as isize, local1);
                        push_ref(&mut refs, i as isize, local2);
                    }
                    AnyInstruction::Real(Instruction::StoreFast { var_num }) => {
                        let r = pop_ref(&mut refs);
                        store_local(
                            &mut instr_flags,
                            &refs,
                            usize::from(var_num.get(info.arg)),
                            r,
                        );
                    }
                    AnyInstruction::Pseudo(PseudoInstruction::StoreFastMaybeNull { var_num }) => {
                        let r = pop_ref(&mut refs);
                        store_local(&mut instr_flags, &refs, var_num.get(info.arg) as usize, r);
                    }
                    AnyInstruction::Real(Instruction::StoreFastLoadFast { .. }) => {
                        let (store_local_idx, load_local_idx) = decode_packed_fast_locals(info.arg);
                        let r = pop_ref(&mut refs);
                        store_local(&mut instr_flags, &refs, store_local_idx, r);
                        push_ref(&mut refs, i as isize, load_local_idx);
                    }
                    AnyInstruction::Real(Instruction::StoreFastStoreFast { .. }) => {
                        let (local1, local2) = decode_packed_fast_locals(info.arg);
                        let r1 = pop_ref(&mut refs);
                        store_local(&mut instr_flags, &refs, local1, r1);
                        let r2 = pop_ref(&mut refs);
                        store_local(&mut instr_flags, &refs, local2, r2);
                    }
                    AnyInstruction::Real(Instruction::Copy { i: _ }) => {
                        let depth = arg_u32 as usize;
                        assert!(depth > 0);
                        assert!(refs.len() >= depth);
                        let r = at_ref(&refs, refs.len() - depth);
                        push_ref(&mut refs, r.instr, r.local);
                    }
                    AnyInstruction::Real(Instruction::Swap { i: _ }) => {
                        let depth = arg_u32 as usize;
                        assert!(depth >= 2);
                        assert!(refs.len() >= depth);
                        swap_top(&mut refs, depth);
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
                        // CPython optimize_load_fast() records the produced
                        // pseudo-ref with the inner loop index here.
                        for produced in 0..net_pushed {
                            push_ref(&mut refs, produced, NOT_LOCAL);
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
                            let _ = pop_ref(&mut refs);
                        }
                    }
                    AnyInstruction::Real(
                        Instruction::EndSend | Instruction::SetFunctionAttribute { .. },
                    ) => {
                        let tos = pop_ref(&mut refs);
                        let _ = pop_ref(&mut refs);
                        push_ref(&mut refs, tos.instr, tos.local);
                    }
                    AnyInstruction::Real(Instruction::CheckExcMatch) => {
                        let _ = pop_ref(&mut refs);
                        push_ref(&mut refs, i as isize, NOT_LOCAL);
                    }
                    AnyInstruction::Real(Instruction::ForIter { .. }) => {
                        let target = info.target;
                        if target != BlockIdx::NULL {
                            push_block(
                                &mut worklist,
                                &mut visited,
                                &self.blocks,
                                block_idx,
                                target,
                                refs.len() + 1,
                            );
                        }
                        push_ref(&mut refs, i as isize, NOT_LOCAL);
                    }
                    AnyInstruction::Real(Instruction::LoadAttr { namei }) => {
                        let self_ref = pop_ref(&mut refs);
                        push_ref(&mut refs, i as isize, NOT_LOCAL);
                        if namei.get(info.arg).is_method() {
                            push_ref(&mut refs, self_ref.instr, self_ref.local);
                        }
                    }
                    AnyInstruction::Real(Instruction::LoadSuperAttr { namei }) => {
                        let self_ref = pop_ref(&mut refs);
                        let _ = pop_ref(&mut refs);
                        let _ = pop_ref(&mut refs);
                        push_ref(&mut refs, i as isize, NOT_LOCAL);
                        if namei.get(info.arg).is_load_method() {
                            push_ref(&mut refs, self_ref.instr, self_ref.local);
                        }
                    }
                    AnyInstruction::Real(
                        Instruction::LoadSpecial { .. } | Instruction::PushExcInfo,
                    ) => {
                        let tos = pop_ref(&mut refs);
                        push_ref(&mut refs, i as isize, NOT_LOCAL);
                        push_ref(&mut refs, tos.instr, tos.local);
                    }
                    AnyInstruction::Real(Instruction::Send { .. }) => {
                        let target = info.target;
                        if target != BlockIdx::NULL {
                            push_block(
                                &mut worklist,
                                &mut visited,
                                &self.blocks,
                                block_idx,
                                target,
                                refs.len(),
                            );
                        }
                        let _ = pop_ref(&mut refs);
                        push_ref(&mut refs, i as isize, NOT_LOCAL);
                    }
                    _ => {
                        let effect = instr.stack_effect_info(arg_u32);
                        let num_popped = effect.popped() as usize;
                        let num_pushed = effect.pushed() as usize;
                        let target = info.target;
                        if instr.has_target() && target != BlockIdx::NULL {
                            let target_depth = refs
                                .len()
                                .saturating_sub(num_popped)
                                .saturating_add(num_pushed);
                            push_block(
                                &mut worklist,
                                &mut visited,
                                &self.blocks,
                                block_idx,
                                target,
                                target_depth,
                            );
                        }
                        if !instr.is_block_push() {
                            for _ in 0..num_popped {
                                let _ = pop_ref(&mut refs);
                            }
                            for _ in 0..num_pushed {
                                push_ref(&mut refs, i as isize, NOT_LOCAL);
                            }
                        }
                    }
                }
            }

            if let Some(term) = block.instructions.last()
                && block.next != BlockIdx::NULL
                && !term.instr.is_unconditional_jump()
                && !term.instr.is_scope_exit()
            {
                debug_assert!(block_has_fallthrough(block));
                push_block(
                    &mut worklist,
                    &mut visited,
                    &self.blocks,
                    block_idx,
                    block.next,
                    refs.len(),
                );
            }

            for r in refs {
                if r.instr != DUMMY_INSTR {
                    instr_flags[r.instr as usize] |= REF_UNCONSUMED;
                }
            }

            let block = &mut self.blocks[block_idx];
            for (i, info) in block.instructions.iter_mut().enumerate() {
                if instr_flags[i] != 0 {
                    continue;
                }
                match info.instr.real() {
                    Some(Instruction::LoadFast { .. }) => {
                        info.instr = Instruction::LoadFastBorrow {
                            var_num: Arg::marker(),
                        }
                        .into();
                    }
                    Some(Instruction::LoadFastLoadFast { .. }) => {
                        info.instr = Instruction::LoadFastBorrowLoadFastBorrow {
                            var_nums: Arg::marker(),
                        }
                        .into();
                    }
                    _ => {}
                }
            }
        }
    }

    fn fast_scan_many_locals(&mut self, nlocals: usize) {
        debug_assert!(nlocals > 64);
        let mut states = vec![0usize; nlocals - 64];
        let mut blocknum = 0usize;
        let mut current = BlockIdx(0);
        while current != BlockIdx::NULL {
            blocknum += 1;
            for info in &mut self.blocks[current.idx()].instructions {
                match info.instr.real() {
                    Some(
                        Instruction::DeleteFast { var_num }
                        | Instruction::LoadFastAndClear { var_num },
                    ) => {
                        let idx = usize::from(var_num.get(info.arg));
                        if idx >= 64 && idx < nlocals {
                            states[idx - 64] = blocknum - 1;
                        }
                    }
                    None if matches!(
                        info.instr.pseudo(),
                        Some(PseudoInstruction::StoreFastMaybeNull { .. })
                    ) =>
                    {
                        let Some(PseudoInstruction::StoreFastMaybeNull { var_num }) =
                            info.instr.pseudo()
                        else {
                            unreachable!();
                        };
                        let idx = var_num.get(info.arg) as usize;
                        if idx >= 64 && idx < nlocals {
                            states[idx - 64] = blocknum - 1;
                        }
                    }
                    Some(Instruction::StoreFast { var_num }) => {
                        let idx = usize::from(var_num.get(info.arg));
                        if idx >= 64 && idx < nlocals {
                            states[idx - 64] = blocknum;
                        }
                    }
                    Some(Instruction::LoadFast { var_num }) => {
                        let idx = usize::from(var_num.get(info.arg));
                        if idx >= 64 && idx < nlocals && states[idx - 64] != blocknum {
                            info.instr = Opcode::LoadFastCheck.into();
                        }
                        if idx >= 64 && idx < nlocals {
                            states[idx - 64] = blocknum;
                        }
                    }
                    _ => {}
                }
            }
            current = self.blocks[current.idx()].next;
        }
    }

    fn add_checks_for_loads_of_uninitialized_variables(&mut self) {
        let mut nlocals = self.metadata.varnames.len();
        if nlocals == 0 {
            return;
        }

        let mut nparams = self.metadata.argcount as usize + self.metadata.kwonlyargcount as usize;
        if self.flags.contains(CodeFlags::VARARGS) {
            nparams += 1;
        }
        if self.flags.contains(CodeFlags::VARKEYWORDS) {
            nparams += 1;
        }
        nparams = nparams.min(nlocals);

        if nlocals > 64 {
            self.fast_scan_many_locals(nlocals);
            nlocals = 64;
        }

        let mut unsafe_masks = vec![0u64; self.blocks.len()];
        let mut on_stack = vec![false; self.blocks.len()];
        let mut worklist = Vec::with_capacity(self.blocks.len());
        let mut start_mask = 0u64;
        for i in nparams..nlocals {
            start_mask |= 1u64 << i;
        }
        maybe_push_local_block(
            &mut worklist,
            &mut on_stack,
            &mut unsafe_masks,
            BlockIdx(0),
            start_mask,
        );

        let mut current = BlockIdx(0);
        while current != BlockIdx::NULL {
            scan_block_for_locals(
                &mut self.blocks,
                current,
                &mut worklist,
                &mut on_stack,
                &mut unsafe_masks,
            );
            current = self.blocks[current.idx()].next;
        }

        while let Some(block_idx) = worklist.pop() {
            on_stack[block_idx.idx()] = false;
            scan_block_for_locals(
                &mut self.blocks,
                block_idx,
                &mut worklist,
                &mut on_stack,
                &mut unsafe_masks,
            );
        }
    }

    fn max_stackdepth(&mut self) -> crate::InternalResult<u32> {
        let mut maxdepth = 0u32;
        let mut stack = Vec::with_capacity(self.blocks.len());
        let mut start_depths = vec![u32::MAX; self.blocks.len()];
        stackdepth_push(&mut stack, &mut start_depths, BlockIdx(0), 0)?;
        const DEBUG: bool = false;
        'process_blocks: while let Some(block_idx) = stack.pop() {
            let idx = block_idx.idx();
            let mut depth = start_depths[idx];
            if DEBUG {
                eprintln!("===BLOCK {}===", block_idx.0);
            }
            let block = &self.blocks[block_idx];
            for ins in &block.instructions {
                let instr = &ins.instr;
                let effect = instr.stack_effect(ins.arg.into());
                if DEBUG {
                    let display_arg = if ins.target == BlockIdx::NULL {
                        ins.arg
                    } else {
                        OpArg::new(ins.target.0)
                    };
                    eprint!("{display_arg:?}: {depth} {effect:+} => ");
                }
                let new_depth = depth.checked_add_signed(effect).ok_or({
                    if effect < 0 {
                        InternalError::StackUnderflow
                    } else {
                        InternalError::StackOverflow
                    }
                })?;
                if DEBUG {
                    eprintln!("{new_depth}");
                }
                maxdepth = maxdepth.max(depth);
                // Process target blocks for branching instructions
                if instr.has_target()
                    && ins.target != BlockIdx::NULL
                    && !matches!(instr.real(), Some(Instruction::EndAsyncFor))
                {
                    let jump_effect = instr.stack_effect_jump(ins.arg.into());
                    let target_depth = depth.checked_add_signed(jump_effect).ok_or({
                        if jump_effect < 0 {
                            InternalError::StackUnderflow
                        } else {
                            InternalError::StackOverflow
                        }
                    })?;
                    maxdepth = maxdepth.max(depth);
                    stackdepth_push(&mut stack, &mut start_depths, ins.target, target_depth)?;
                }
                depth = new_depth;
                if instr.is_no_fallthrough() {
                    continue 'process_blocks;
                }
            }
            // Only push next block if it's not NULL
            if block.next != BlockIdx::NULL {
                stackdepth_push(&mut stack, &mut start_depths, block.next, depth)?;
            }
        }
        if DEBUG {
            eprintln!("DONE: {maxdepth}");
        }

        for block_idx in self.block_next_order() {
            let start_depth = start_depths[block_idx.idx()];
            self.blocks[block_idx].start_depth = (start_depth != u32::MAX).then_some(start_depth);
        }

        // Fix up handler stack_depth in ExceptHandlerInfo using start_depths
        // computed above: depth = start_depth - 1 - preserve_lasti
        for block_idx in self.block_next_order() {
            let block = &mut self.blocks[block_idx];
            for ins in &mut block.instructions {
                if let Some(ref mut handler) = ins.except_handler {
                    let h_start = start_depths[handler.handler_block.idx()];
                    if h_start != u32::MAX {
                        let adjustment = 1 + handler.preserve_lasti as u32;
                        debug_assert!(
                            h_start >= adjustment,
                            "handler start depth {h_start} too shallow for adjustment {adjustment}"
                        );
                        handler.stack_depth = h_start.saturating_sub(adjustment);
                    }
                }
            }
        }

        Ok(maxdepth)
    }
}

#[cfg(test)]
impl CodeInfo {
    fn debug_block_dump(&self) -> String {
        let mut out = String::new();
        for (block_idx, block) in iter_blocks(&self.blocks) {
            use core::fmt::Write;
            let _ = writeln!(
                out,
                "block {} next={} cold={} except={} preserve_lasti={} start_depth={}",
                u32::from(block_idx),
                if block.next == BlockIdx::NULL {
                    String::from("NULL")
                } else {
                    u32::from(block.next).to_string()
                },
                block.cold,
                block.except_handler,
                block.preserve_lasti,
                block
                    .start_depth
                    .map_or_else(|| String::from("None"), |depth| depth.to_string()),
            );
            for info in &block.instructions {
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
        inline_small_or_no_lineno_blocks(&mut self.blocks);
        trace.push((
            "after_inline_small_or_no_lineno_blocks".to_owned(),
            self.debug_block_dump(),
        ));
        self.remove_unreachable_blocks();
        resolve_line_numbers(&mut self.blocks);
        self.convert_to_load_small_int();
        self.peephole_optimize();
        trace.push((
            "after_peephole_optimize".to_owned(),
            self.debug_block_dump(),
        ));
        self.convert_to_load_small_int();
        self.optimize_basic_blocks()?;
        trace.push((
            "after_optimize_basic_block".to_owned(),
            self.debug_block_dump(),
        ));
        self.remove_redundant_nops_and_pairs();
        self.remove_unreachable_blocks();
        remove_redundant_nops_and_jumps(&mut self.blocks)?;
        debug_assert!(no_redundant_jumps(&self.blocks));
        self.remove_unused_consts();
        trace.push((
            "after_optimize_cfg_cleanup".to_owned(),
            self.debug_block_dump(),
        ));
        self.add_checks_for_loads_of_uninitialized_variables();
        self.insert_superinstructions();
        push_cold_blocks_to_end(&mut self.blocks)?;
        trace.push((
            "after_push_cold_before_chain_reorder".to_owned(),
            self.debug_block_dump(),
        ));
        resolve_line_numbers(&mut self.blocks);
        trace.push((
            "after_push_cold_resolve_line_numbers".to_owned(),
            self.debug_block_dump(),
        ));

        trace.push((
            "after_push_cold_blocks_to_end".to_owned(),
            self.debug_block_dump(),
        ));

        convert_pseudo_conditional_jumps(&mut self.blocks);
        trace.push((
            "after_convert_pseudo_conditional_jumps".to_owned(),
            self.debug_block_dump(),
        ));

        let _max_stackdepth = self.max_stackdepth()?;
        let _nlocalsplus = self.prepare_localsplus();
        convert_pseudo_ops(&mut self.blocks)?;
        trace.push((
            "after_convert_pseudo_ops".to_owned(),
            self.debug_block_dump(),
        ));

        normalize_jumps(&mut self.blocks)?;
        debug_assert!(no_redundant_jumps(&self.blocks));
        trace.push(("after_normalize_jumps".to_owned(), self.debug_block_dump()));
        self.optimize_load_fast_borrow();
        trace.push((
            "after_raw_optimize_load_fast_borrow".to_owned(),
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

fn stackdepth_push(
    stack: &mut Vec<BlockIdx>,
    start_depths: &mut [u32],
    target: BlockIdx,
    depth: u32,
) -> crate::InternalResult<()> {
    let idx = target.idx();
    let block_depth = &mut start_depths[idx];
    if *block_depth != u32::MAX && *block_depth != depth {
        return Err(InternalError::InconsistentStackDepth);
    }
    if *block_depth == u32::MAX {
        *block_depth = depth;
        stack.push(target);
    }
    Ok(())
}

fn iter_blocks(blocks: &[Block]) -> impl Iterator<Item = (BlockIdx, &Block)> + '_ {
    let mut next = BlockIdx(0);
    core::iter::from_fn(move || {
        if next == BlockIdx::NULL {
            return None;
        }
        let (idx, b) = (next, &blocks[next]);
        next = b.next;
        Some((idx, b))
    })
}

/// Generate Python 3.11+ format linetable from source locations
fn generate_linetable(
    locations: &[LineTableLocation],
    first_line: i32,
    debug_ranges: bool,
) -> Box<[u8]> {
    if locations.is_empty() {
        return Box::new([]);
    }

    let mut linetable = Vec::new();
    // Initialize prev_line to first_line
    // The first entry's delta is relative to co_firstlineno
    let mut prev_line = first_line;
    let mut i = 0;

    while i < locations.len() {
        let loc = &locations[i];

        // Count consecutive instructions with the same location
        let mut length = 1;
        while i + length < locations.len() && locations[i + length] == locations[i] {
            length += 1;
        }

        // Process in chunks of up to 8 instructions
        while length > 0 {
            let entry_length = length.min(8);

            // Get line information
            let line = loc.line;

            // NO_LOCATION: emit PyCodeLocationInfoKind::None entries (CACHE, etc.)
            if line == -1 {
                linetable.push(
                    0x80 | ((PyCodeLocationInfoKind::None as u8) << 3) | ((entry_length - 1) as u8),
                );
                // Do NOT update prev_line
                length -= entry_length;
                i += entry_length;
                continue;
            }

            let end_line = loc.end_line;
            let line_delta = line - prev_line;
            let end_line_delta = end_line - line;

            // When debug_ranges is disabled, only emit line info (NoColumns format)
            if !debug_ranges {
                // NoColumns format (code 13): line info only, no column data
                linetable.push(
                    0x80 | ((PyCodeLocationInfoKind::NoColumns as u8) << 3)
                        | ((entry_length - 1) as u8),
                );
                write_signed_varint(&mut linetable, line_delta);

                prev_line = line;
                length -= entry_length;
                i += entry_length;
                continue;
            }

            // Get column information (only when debug_ranges is enabled)
            let col = loc.col;
            let end_col = loc.end_col;
            if (col < 0 || end_col < 0) && end_line == line {
                linetable.push(
                    0x80 | ((PyCodeLocationInfoKind::NoColumns as u8) << 3)
                        | ((entry_length - 1) as u8),
                );
                write_signed_varint(&mut linetable, line_delta);

                prev_line = line;
                length -= entry_length;
                i += entry_length;
                continue;
            }

            // Choose the appropriate encoding based on line delta and column info
            if line_delta == 0 && end_line_delta == 0 {
                if col < 80 && end_col - col < 16 && end_col >= col {
                    // Short form (codes 0-9) for common cases
                    let code = (col / 8).min(9) as u8; // Short0 to Short9
                    linetable.push(0x80 | (code << 3) | ((entry_length - 1) as u8));
                    let col_byte = (((col % 8) as u8) << 4) | ((end_col - col) as u8 & 0xf);
                    linetable.push(col_byte);
                } else if col < 128 && end_col < 128 {
                    // One-line form (code 10) for same line
                    linetable.push(
                        0x80 | ((PyCodeLocationInfoKind::OneLine0 as u8) << 3)
                            | ((entry_length - 1) as u8),
                    );
                    linetable.push(col as u8);
                    linetable.push(end_col as u8);
                } else {
                    // Long form for columns >= 128
                    linetable.push(
                        0x80 | ((PyCodeLocationInfoKind::Long as u8) << 3)
                            | ((entry_length - 1) as u8),
                    );
                    write_signed_varint(&mut linetable, 0); // line_delta = 0
                    write_varint(&mut linetable, 0); // end_line delta = 0
                    write_varint(&mut linetable, (col as u32) + 1);
                    write_varint(&mut linetable, (end_col as u32) + 1);
                }
            } else if line_delta > 0 && line_delta < 3 && end_line_delta == 0 {
                // One-line form (codes 11-12) for line deltas 1-2
                if col < 128 && end_col < 128 {
                    let code = (PyCodeLocationInfoKind::OneLine0 as u8) + (line_delta as u8);
                    linetable.push(0x80 | (code << 3) | ((entry_length - 1) as u8));
                    linetable.push(col as u8);
                    linetable.push(end_col as u8);
                } else {
                    // Long form for columns >= 128
                    linetable.push(
                        0x80 | ((PyCodeLocationInfoKind::Long as u8) << 3)
                            | ((entry_length - 1) as u8),
                    );
                    write_signed_varint(&mut linetable, line_delta);
                    write_varint(&mut linetable, 0); // end_line delta = 0
                    write_varint(&mut linetable, (col as u32) + 1);
                    write_varint(&mut linetable, (end_col as u32) + 1);
                }
            } else {
                // Long form (code 14) for all other cases
                // Handles: line_delta < 0, line_delta >= 3, multi-line spans, or columns >= 128
                linetable.push(
                    0x80 | ((PyCodeLocationInfoKind::Long as u8) << 3) | ((entry_length - 1) as u8),
                );
                write_signed_varint(&mut linetable, line_delta);
                write_varint(&mut linetable, end_line_delta as u32);
                write_varint(&mut linetable, if col < 0 { 0 } else { (col as u32) + 1 });
                write_varint(
                    &mut linetable,
                    if end_col < 0 { 0 } else { (end_col as u32) + 1 },
                );
            }

            prev_line = line;
            length -= entry_length;
            i += entry_length;
        }
    }

    linetable.into_boxed_slice()
}

fn no_linetable_location() -> LineTableLocation {
    LineTableLocation {
        line: -1,
        end_line: -1,
        col: -1,
        end_col: -1,
    }
}

fn instruction_is_terminator(op: Instruction) -> bool {
    op.is_terminator()
}

fn resolve_next_locations(instructions: &[CodeUnit], locations: &mut [LineTableLocation]) {
    debug_assert_eq!(instructions.len(), locations.len());
    let mut next_location = no_linetable_location();
    for (instruction, location) in instructions.iter().zip(locations.iter_mut()).rev() {
        if location.line == NEXT_LOCATION_OVERRIDE {
            *location = if instruction_is_terminator(instruction.op) {
                no_linetable_location()
            } else {
                next_location
            };
        }
        next_location = *location;
    }
}

/// assemble.c assemble_exception_table
fn generate_exception_table(
    instrs: &[InstructionSequenceEntry],
    instruction_offsets: &[u32],
    end_offset: u32,
) -> Box<[u8]> {
    let mut entries: Vec<ExceptionTableEntry> = Vec::new();
    let mut current_entry: Option<(InstructionSequenceExceptHandlerInfo, u32)> = None;
    let same_handler = |left: InstructionSequenceExceptHandlerInfo,
                        right: InstructionSequenceExceptHandlerInfo| {
        // CPython assemble_exception_table() starts a new table entry only
        // when h_label changes. h_startdepth and h_preserve_lasti come from
        // the active handler for that label.
        left.target_offset == right.target_offset
    };

    for (idx, instr) in instrs.iter().enumerate() {
        let instr_offset = instruction_offsets[idx];
        match (&current_entry, instr.except_handler) {
            // No current entry, no handler - nothing to do
            (None, None) => {}

            // No current entry, handler starts - begin new entry
            (None, Some(handler)) => {
                current_entry = Some((handler, instr_offset));
            }

            // Current entry exists, same handler - continue
            (Some((curr_handler, _)), Some(handler)) if same_handler(*curr_handler, handler) => {}

            // Current entry exists, different handler - finish current, start new
            (Some((curr_handler, start)), Some(handler)) => {
                let target_offset = instruction_offsets
                    [curr_handler.target_offset.expect("missing handler target")];
                entries.push(ExceptionTableEntry::new(
                    *start,
                    instr_offset,
                    target_offset,
                    curr_handler.stack_depth as u16,
                    curr_handler.preserve_lasti,
                ));
                current_entry = Some((handler, instr_offset));
            }

            // Current entry exists, no handler - finish current entry
            (Some((curr_handler, start)), None) => {
                let target_offset = instruction_offsets
                    [curr_handler.target_offset.expect("missing handler target")];
                entries.push(ExceptionTableEntry::new(
                    *start,
                    instr_offset,
                    target_offset,
                    curr_handler.stack_depth as u16,
                    curr_handler.preserve_lasti,
                ));
                current_entry = None;
            }
        }
    }

    // Finish any remaining entry
    if let Some((curr_handler, start)) = current_entry {
        let target_offset =
            instruction_offsets[curr_handler.target_offset.expect("missing handler target")];
        entries.push(ExceptionTableEntry::new(
            start,
            end_offset,
            target_offset,
            curr_handler.stack_depth as u16,
            curr_handler.preserve_lasti,
        ));
    }

    encode_exception_table(&entries)
}

/// Mark exception handler target blocks.
/// flowgraph.c mark_except_handlers
pub(crate) fn mark_except_handlers(blocks: &mut [Block]) -> crate::InternalResult<()> {
    let block_order = layout_block_order(blocks);
    for block_idx in block_order.iter().copied() {
        debug_assert!(!blocks[block_idx.idx()].except_handler);
    }

    let mut targets = Vec::new();
    for block_idx in block_order.iter().copied() {
        for instr in &blocks[block_idx.idx()].instructions {
            if instr.instr.is_block_push() {
                if instr.target == BlockIdx::NULL {
                    return Err(InternalError::MalformedControlFlowGraph);
                }
                targets.push(instr.target.idx());
            }
        }
    }

    for idx in targets {
        blocks[idx].except_handler = true;
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
/// optimize_load_fast_borrow to terminate fall-through at those placeholders.
fn mark_cold(blocks: &mut [Block]) -> Vec<bool> {
    let n = blocks.len();
    let block_order = layout_block_order(blocks);
    for block_idx in block_order.iter().copied() {
        let block = &blocks[block_idx.idx()];
        debug_assert!(!block.cold);
    }

    let mut warm = vec![false; n];
    let mut stack = Vec::new();
    warm[0] = true;
    stack.push(BlockIdx(0));

    while let Some(block_idx) = stack.pop() {
        let block = &blocks[block_idx.idx()];
        debug_assert!(!block.except_handler);

        if block_has_fallthrough(block) && block.next != BlockIdx::NULL {
            let next_idx = block.next.idx();
            if !warm[next_idx] {
                warm[next_idx] = true;
                stack.push(block.next);
            }
        }

        for instr in &block.instructions {
            if is_jump_instruction(instr) && instr.target != BlockIdx::NULL {
                let target_idx = instr.target.idx();
                if !warm[target_idx] {
                    warm[target_idx] = true;
                    stack.push(instr.target);
                }
            }
        }
    }

    let mut cold = vec![false; n];
    let mut cold_visited = vec![false; n];
    let mut cold_stack = Vec::new();
    for block_idx in block_order.iter().copied() {
        let i = block_idx.idx();
        let block = &blocks[i];
        if block.except_handler {
            debug_assert!(!warm[i]);
            cold_stack.push(block_idx);
            cold_visited[i] = true;
        }
    }
    while let Some(block_idx) = cold_stack.pop() {
        let idx = block_idx.idx();
        cold[idx] = true;
        let block = &blocks[idx];
        if block_has_fallthrough(block) && block.next != BlockIdx::NULL {
            let next_idx = block.next.idx();
            if !warm[next_idx] && !cold_visited[next_idx] {
                cold_visited[next_idx] = true;
                cold_stack.push(block.next);
            }
        }
        let instr_count = block.instructions.len();
        for (i, instr) in block.instructions.iter().enumerate() {
            if is_jump_instruction(instr) && instr.target != BlockIdx::NULL {
                debug_assert_eq!(i, instr_count - 1);
                let target_idx = instr.target.idx();
                if !warm[target_idx] && !cold_visited[target_idx] {
                    cold_visited[target_idx] = true;
                    cold_stack.push(instr.target);
                }
            }
        }
    }

    for block_idx in block_order {
        let i = block_idx.idx();
        blocks[i].cold = cold[i];
    }
    warm
}

/// flowgraph.c push_cold_blocks_to_end
fn push_cold_blocks_to_end(blocks: &mut Vec<Block>) -> crate::InternalResult<()> {
    if blocks.len() <= 1 {
        return Ok(());
    }

    let warm = mark_cold(blocks);

    // If a cold block falls through to a warm block, add an explicit jump
    let fixups: Vec<(BlockIdx, BlockIdx)> = iter_blocks(blocks)
        .filter(|(_, block)| {
            block.cold
                && block.next != BlockIdx::NULL
                && warm[block.next.idx()]
                && block_has_fallthrough(block)
        })
        .map(|(idx, block)| (idx, block.next))
        .collect();

    for (cold_idx, warm_next) in fixups {
        let jump_block_idx = BlockIdx(blocks.len() as u32);
        let mut jump_block = Block {
            cold: true,
            ..Block::default()
        };
        basicblock_add_jump_op(
            &mut jump_block,
            InstructionInfo {
                instr: PseudoOpcode::JumpNoInterrupt.into(),
                arg: OpArg::new(0),
                target: BlockIdx::NULL,
                location: SourceLocation::default(),
                end_location: SourceLocation::default(),
                except_handler: None,
                lineno_override: Some(-1),
                cache_entries: 0,
            },
            warm_next,
        )?;
        jump_block.next = blocks[cold_idx.idx()].next;
        blocks[cold_idx.idx()].next = jump_block_idx;
        blocks.push(jump_block);
    }

    // Extract cold block streaks and append at the end
    let mut cold_head: BlockIdx = BlockIdx::NULL;
    let mut cold_tail: BlockIdx = BlockIdx::NULL;
    let mut current = BlockIdx(0);
    assert!(!blocks[0].cold);

    while current != BlockIdx::NULL {
        let next = blocks[current.idx()].next;
        if next == BlockIdx::NULL {
            break;
        }

        if blocks[next.idx()].cold {
            let cold_start = next;
            let mut cold_end = next;
            while blocks[cold_end.idx()].next != BlockIdx::NULL
                && blocks[blocks[cold_end.idx()].next.idx()].cold
            {
                cold_end = blocks[cold_end.idx()].next;
            }

            let after_cold = blocks[cold_end.idx()].next;
            blocks[current.idx()].next = after_cold;
            blocks[cold_end.idx()].next = BlockIdx::NULL;

            if cold_head == BlockIdx::NULL {
                cold_head = cold_start;
            } else {
                blocks[cold_tail.idx()].next = cold_start;
            }
            cold_tail = cold_end;
        } else {
            current = next;
        }
    }

    if cold_head != BlockIdx::NULL {
        let mut last = current;
        while blocks[last.idx()].next != BlockIdx::NULL {
            last = blocks[last.idx()].next;
        }
        blocks[last.idx()].next = cold_head;
        remove_redundant_nops_and_jumps(blocks)?;
    }
    Ok(())
}

/// flowgraph.c check_cfg
fn check_cfg(blocks: &[Block]) -> crate::InternalResult<()> {
    for (_, block) in iter_blocks(blocks) {
        for (i, ins) in block.instructions.iter().enumerate() {
            debug_assert!(!ins.instr.is_assembler());
            if ins.instr.is_terminator() && i + 1 != block.instructions.len() {
                return Err(InternalError::MalformedControlFlowGraph);
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum JumpThreadKind {
    Plain,
    NoInterrupt,
}

fn jump_thread_kind(instr: AnyInstruction) -> Option<JumpThreadKind> {
    Some(match instr.into() {
        AnyOpcode::Pseudo(PseudoOpcode::Jump)
        | AnyOpcode::Real(Opcode::JumpForward | Opcode::JumpBackward) => JumpThreadKind::Plain,
        AnyOpcode::Pseudo(PseudoOpcode::JumpNoInterrupt)
        | AnyOpcode::Real(Opcode::JumpBackwardNoInterrupt) => JumpThreadKind::NoInterrupt,
        _ => return None,
    })
}

fn threaded_jump_instr(
    source: AnyInstruction,
    target: AnyInstruction,
    conditional: bool,
) -> Option<AnyInstruction> {
    let target_kind = jump_thread_kind(target)?;
    if conditional {
        return (target_kind == JumpThreadKind::Plain).then_some(source);
    }

    let source_kind = jump_thread_kind(source)?;
    let result_kind = if source_kind == JumpThreadKind::NoInterrupt
        && target_kind == JumpThreadKind::NoInterrupt
    {
        JumpThreadKind::NoInterrupt
    } else {
        JumpThreadKind::Plain
    };

    Some(match (source.into(), result_kind) {
        (AnyOpcode::Pseudo(_), JumpThreadKind::Plain) => PseudoOpcode::Jump.into(),
        (AnyOpcode::Pseudo(_), JumpThreadKind::NoInterrupt) => PseudoOpcode::JumpNoInterrupt.into(),
        (AnyOpcode::Real(Opcode::JumpBackwardNoInterrupt), JumpThreadKind::Plain) => {
            Opcode::JumpBackward.into()
        }
        (AnyOpcode::Real(Opcode::JumpBackwardNoInterrupt), JumpThreadKind::NoInterrupt) => source,
        (AnyOpcode::Real(Opcode::JumpForward | Opcode::JumpBackward), JumpThreadKind::Plain) => {
            source
        }
        (
            AnyOpcode::Real(Opcode::JumpForward | Opcode::JumpBackward),
            JumpThreadKind::NoInterrupt,
        ) => PseudoOpcode::JumpNoInterrupt.into(),
        _ => return None,
    })
}

/// flowgraph.c optimize_basic_block + jump_thread
fn jump_threading_block(blocks: &mut [Block], block_idx: BlockIdx) -> crate::InternalResult<()> {
    let bi = block_idx.idx();
    while let Some(last_idx) = blocks[bi].instructions.len().checked_sub(1) {
        let ins = blocks[bi].instructions[last_idx];
        let target = ins.target;
        if target == BlockIdx::NULL {
            return Ok(());
        }
        if !(ins.instr.is_unconditional_jump() || is_conditional_jump(&ins.instr)) {
            return Ok(());
        }
        if blocks[target.idx()].instructions.is_empty() {
            return Ok(());
        }
        let target_ins = blocks[target.idx()].instructions[0];
        match (
            ins.instr.pseudo().map(Into::into),
            target_ins.instr.pseudo().map(Into::into),
        ) {
            (
                Some(source @ (PseudoOpcode::JumpIfFalse | PseudoOpcode::JumpIfTrue)),
                Some(PseudoOpcode::Jump),
            )
            | (Some(source @ PseudoOpcode::JumpIfFalse), Some(PseudoOpcode::JumpIfFalse))
            | (Some(source @ PseudoOpcode::JumpIfTrue), Some(PseudoOpcode::JumpIfTrue)) => {
                let final_target = target_ins.target;
                if final_target == BlockIdx::NULL || final_target == ins.target {
                    return Ok(());
                }
                set_to_nop(&mut blocks[bi].instructions[last_idx]);
                basicblock_add_jump(blocks, block_idx, source.into(), final_target, target_ins)?;
                return Ok(());
            }
            (Some(PseudoOpcode::JumpIfFalse), Some(PseudoOpcode::JumpIfTrue))
            | (Some(PseudoOpcode::JumpIfTrue), Some(PseudoOpcode::JumpIfFalse)) => {
                let next = blocks[target.idx()].next;
                if next == BlockIdx::NULL || next == target {
                    return Ok(());
                }
                blocks[bi].instructions[last_idx].target = next;
                continue;
            }
            _ => {}
        }
        if !target_ins.instr.is_unconditional_jump()
            || target_ins.target == BlockIdx::NULL
            || target_ins.target == target
        {
            return Ok(());
        }
        let conditional = is_conditional_jump(&ins.instr);
        let final_target = target_ins.target;
        let Some(threaded_instr) = (if conditional {
            match jump_thread_kind(target_ins.instr) {
                Some(JumpThreadKind::Plain) => Some(ins.instr),
                _ => None,
            }
        } else {
            threaded_jump_instr(ins.instr, target_ins.instr, false)
        }) else {
            return Ok(());
        };
        if ins.target == final_target {
            return Ok(());
        }
        set_to_nop(&mut blocks[bi].instructions[last_idx]);
        basicblock_add_jump(blocks, block_idx, threaded_instr, final_target, target_ins)?;
        return Ok(());
    }
    Ok(())
}

/// flowgraph.c basicblock_add_jump
fn basicblock_add_jump(
    blocks: &mut [Block],
    block_idx: BlockIdx,
    instr: AnyInstruction,
    target: BlockIdx,
    loc_source: InstructionInfo,
) -> crate::InternalResult<()> {
    let bi = block_idx.idx();
    basicblock_add_jump_op(
        &mut blocks[bi],
        InstructionInfo {
            instr,
            arg: OpArg::new(0),
            target: BlockIdx::NULL,
            location: loc_source.location,
            end_location: loc_source.end_location,
            except_handler: None,
            lineno_override: loc_source.lineno_override,
            cache_entries: 0,
        },
        target,
    )
}

pub(crate) fn is_conditional_jump(instr: &AnyInstruction) -> bool {
    matches!(
        instr.real().map(Into::into),
        Some(
            Opcode::PopJumpIfFalse
                | Opcode::PopJumpIfTrue
                | Opcode::PopJumpIfNone
                | Opcode::PopJumpIfNotNone
        )
    ) || matches!(
        instr.pseudo(),
        Some(PseudoInstruction::JumpIfFalse { .. } | PseudoInstruction::JumpIfTrue { .. })
    )
}

/// flowgraph.c convert_pseudo_conditional_jumps
fn convert_pseudo_conditional_jumps(blocks: &mut [Block]) {
    let block_order = layout_block_order(blocks);
    for block_idx in block_order {
        let block = &mut blocks[block_idx.idx()];
        let mut i = 0;
        while i < block.instructions.len() {
            let Some(pseudo) = block.instructions[i].instr.pseudo() else {
                i += 1;
                continue;
            };
            let jump = match pseudo {
                PseudoInstruction::JumpIfFalse { .. } => {
                    debug_assert_eq!(i, block.instructions.len() - 1);
                    Instruction::PopJumpIfFalse {
                        delta: Arg::marker(),
                    }
                }
                PseudoInstruction::JumpIfTrue { .. } => {
                    debug_assert_eq!(i, block.instructions.len() - 1);
                    Instruction::PopJumpIfTrue {
                        delta: Arg::marker(),
                    }
                }
                _ => {
                    i += 1;
                    continue;
                }
            };

            let jump_info = InstructionInfo {
                instr: jump.into(),
                ..block.instructions[i]
            };
            block.instructions[i].instr = Instruction::Copy { i: Arg::marker() }.into();
            block.instructions[i].arg = OpArg::new(1);
            block.instructions[i].target = BlockIdx::NULL;

            let mut to_bool = block.instructions[i];
            to_bool.instr = Instruction::ToBool.into();
            to_bool.arg = OpArg::new(0);
            to_bool.target = BlockIdx::NULL;

            basicblock_insert_instruction(block, i + 1, to_bool);
            basicblock_insert_instruction(block, i + 2, jump_info);
            i += 3;
        }
    }
}

/// Invert a conditional jump opcode.
fn reversed_conditional(instr: &AnyInstruction) -> Option<AnyInstruction> {
    Some(match AnyOpcode::from(*instr).real()? {
        Opcode::PopJumpIfFalse => Opcode::PopJumpIfTrue.into(),
        Opcode::PopJumpIfTrue => Opcode::PopJumpIfFalse.into(),
        Opcode::PopJumpIfNone => Opcode::PopJumpIfNotNone.into(),
        Opcode::PopJumpIfNotNone => Opcode::PopJumpIfNone.into(),
        _ => return None,
    })
}

/// flowgraph.c normalize_jumps_in_block
fn normalize_jumps_in_block(
    blocks: &mut Vec<Block>,
    block_idx: BlockIdx,
    visited: &mut Vec<bool>,
) -> crate::InternalResult<()> {
    let idx = block_idx.idx();
    let Some(last_ins) = blocks[idx].instructions.last().copied() else {
        return Ok(());
    };
    if !is_conditional_jump(&last_ins.instr) || last_ins.target == BlockIdx::NULL {
        return Ok(());
    }

    let target = last_ins.target;
    let is_forward = !visited[target.idx()];

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
            cache_entries: 0,
        };
        basicblock_addop(&mut blocks[idx], not_taken);
        return Ok(());
    }

    // Backward conditional jump: invert and create new block
    // Transform: `cond_jump T` (backward)
    // Into: `reversed_cond_jump b_next` + new block [NOT_TAKEN, JUMP T]
    let loc = last_ins.location;
    let end_loc = last_ins.end_location;

    if let Some(reversed) = reversed_conditional(&last_ins.instr) {
        let old_next = blocks[idx].next;
        let is_cold = blocks[idx].cold;

        // Create new block with NOT_TAKEN + JUMP to original backward target
        let new_block_idx = BlockIdx(blocks.len() as u32);
        let mut new_block = Block {
            cold: is_cold,
            start_depth: blocks[target.idx()].start_depth,
            ..Block::default()
        };
        basicblock_addop(
            &mut new_block,
            InstructionInfo {
                instr: Opcode::NotTaken.into(),
                arg: OpArg::new(0),
                target: BlockIdx::NULL,
                location: loc,
                end_location: end_loc,
                except_handler: None,
                lineno_override: last_ins.lineno_override,
                cache_entries: 0,
            },
        );
        basicblock_add_jump_op(
            &mut new_block,
            InstructionInfo {
                instr: PseudoOpcode::Jump.into(),
                arg: OpArg::new(0),
                target: BlockIdx::NULL,
                location: loc,
                end_location: end_loc,
                except_handler: None,
                lineno_override: last_ins.lineno_override,
                cache_entries: 0,
            },
            target,
        )?;
        new_block.next = old_next;

        // Update the conditional jump: invert opcode, target = old next block
        let last_mut = blocks[idx].instructions.last_mut().unwrap();
        last_mut.instr = reversed;
        last_mut.target = old_next;

        // Splice new block between current and old next
        blocks[idx].next = new_block_idx;
        blocks.push(new_block);

        // Extend visited array and update visit order
        visited.push(true);
    }
    Ok(())
}

/// flowgraph.c normalize_jumps
fn normalize_jumps(blocks: &mut Vec<Block>) -> crate::InternalResult<()> {
    let mut visited = vec![false; blocks.len()];
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let idx = current.idx();
        visited[idx] = true;
        normalize_jumps_in_block(blocks, current, &mut visited)?;
        current = blocks[idx].next;
    }
    Ok(())
}

/// flowgraph.c basicblock_inline_small_or_no_lineno_blocks
fn basicblock_inline_small_or_no_lineno_blocks(blocks: &mut [Block], block_idx: BlockIdx) -> bool {
    const MAX_COPY_SIZE: usize = 4;

    let Some(last) = blocks[block_idx.idx()].instructions.last().copied() else {
        return false;
    };
    if !last.instr.is_unconditional_jump() || last.target == BlockIdx::NULL {
        return false;
    }

    let target = last.target;
    let small_exit_block = is_scope_exit_block(&blocks[target.idx()])
        && blocks[target.idx()].instructions.len() <= MAX_COPY_SIZE;
    let no_lineno_no_fallthrough =
        block_has_no_lineno(&blocks[target.idx()]) && !block_has_fallthrough(&blocks[target.idx()]);
    if !small_exit_block && !no_lineno_no_fallthrough {
        return false;
    }

    let removed_jump_kind = jump_thread_kind(last.instr);
    let target_instructions = blocks[target.idx()].instructions.clone();
    if let Some(last_instr) = blocks[block_idx.idx()].instructions.last_mut() {
        set_to_nop(last_instr);
    }
    blocks[block_idx.idx()]
        .instructions
        .extend(target_instructions);
    if no_lineno_no_fallthrough
        && removed_jump_kind == Some(JumpThreadKind::Plain)
        && let Some(last) = blocks[block_idx.idx()].instructions.last_mut()
        && jump_thread_kind(last.instr) == Some(JumpThreadKind::NoInterrupt)
    {
        last.instr = match last.instr.into() {
            AnyOpcode::Pseudo(PseudoOpcode::JumpNoInterrupt) => PseudoOpcode::Jump.into(),
            AnyOpcode::Real(Opcode::JumpBackwardNoInterrupt) => Opcode::JumpBackward.into(),
            _ => last.instr,
        };
    }
    true
}

/// flowgraph.c inline_small_or_no_lineno_blocks
fn inline_small_or_no_lineno_blocks(blocks: &mut [Block]) {
    loop {
        let mut changes = false;
        let mut current = BlockIdx(0);
        while current != BlockIdx::NULL {
            let next = blocks[current.idx()].next;
            changes |= basicblock_inline_small_or_no_lineno_blocks(blocks, current);

            current = next;
        }

        if !changes {
            break;
        }
    }
}

/// flowgraph.c basicblock_remove_redundant_nops
fn basicblock_remove_redundant_nops(blocks: &mut [Block], block_idx: BlockIdx) -> usize {
    let mut changes = 0;
    let bi = block_idx.idx();
    let mut instructions = core::mem::take(&mut blocks[bi].instructions);
    let spare_instr_slots = core::mem::take(&mut blocks[bi].cpython_spare_instr_slots);
    let mut dest = 0;
    let mut prev_lineno = -1i32;

    for src in 0..instructions.len() {
        let instr = instructions[src];
        let lineno = instruction_lineno(&instr);
        let mut remove = false;

        if matches!(instr.instr.real(), Some(Instruction::Nop)) {
            if lineno < 0 || prev_lineno == lineno {
                remove = true;
            } else if src < instructions.len() - 1 {
                let next_lineno = instruction_lineno(&instructions[src + 1]);
                if next_lineno == lineno {
                    remove = true;
                } else if next_lineno < 0 {
                    copy_instruction_location(instr, &mut instructions[src + 1]);
                    remove = true;
                }
            } else {
                let next = next_nonempty_block(blocks, blocks[bi].next);
                if next != BlockIdx::NULL {
                    let mut first_next = None;
                    for (next_i, next_instr) in blocks[next.idx()].instructions.iter().enumerate() {
                        let next_lineno = instruction_lineno(next_instr);
                        if matches!(next_instr.instr.real(), Some(Instruction::Nop))
                            && next_lineno < 0
                        {
                            continue;
                        }
                        first_next = Some((next_i, next_lineno));
                        break;
                    }
                    if let Some((_next_i, next_lineno)) = first_next
                        && next_lineno == lineno
                    {
                        remove = true;
                    }
                }
            }
        }

        if remove {
            changes += 1;
        } else {
            if dest != src {
                instructions[dest] = instructions[src];
            }
            dest += 1;
            prev_lineno = lineno;
        }
    }

    blocks[bi]
        .cpython_spare_instr_slots
        .extend_from_slice(&instructions[dest..]);
    blocks[bi]
        .cpython_spare_instr_slots
        .extend_from_slice(&spare_instr_slots);
    instructions.truncate(dest);
    blocks[bi].instructions = instructions;
    changes
}

/// flowgraph.c remove_redundant_nops
fn remove_redundant_nops(blocks: &mut [Block]) -> usize {
    let mut block_order = Vec::new();
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        block_order.push(current);
        current = blocks[current.idx()].next;
    }
    block_order
        .into_iter()
        .map(|block_idx| basicblock_remove_redundant_nops(blocks, block_idx))
        .sum()
}

/// flowgraph.c no_redundant_nops
fn no_redundant_nops(blocks: &[Block]) -> bool {
    let mut blocks = blocks.to_vec();
    remove_redundant_nops(&mut blocks) == 0
}

/// flowgraph.c remove_redundant_jumps
fn remove_redundant_jumps(blocks: &mut [Block]) -> crate::InternalResult<usize> {
    let mut changes = 0;
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let idx = current.idx();
        let Some(last_instr) = blocks[idx].instructions.last().copied() else {
            current = blocks[idx].next;
            continue;
        };
        debug_assert!(!last_instr.instr.is_assembler());
        if last_instr.instr.is_unconditional_jump() {
            let jump_target = next_nonempty_block(blocks, last_instr.target);
            if jump_target == BlockIdx::NULL {
                return Err(InternalError::MalformedControlFlowGraph);
            }
            let next = next_nonempty_block(blocks, blocks[idx].next);
            if jump_target == next {
                let last_instr = blocks[idx].instructions.last_mut().unwrap();
                set_to_nop(last_instr);
                changes += 1;
                current = blocks[idx].next;
                continue;
            }
        }
        current = blocks[idx].next;
    }
    Ok(changes)
}

/// flowgraph.c no_redundant_jumps
fn no_redundant_jumps(blocks: &[Block]) -> bool {
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let block = &blocks[current.idx()];
        let Some(last) = block.instructions.last() else {
            current = block.next;
            continue;
        };
        if last.instr.is_unconditional_jump() {
            let next = next_nonempty_block(blocks, block.next);
            let jump_target = next_nonempty_block(blocks, last.target);
            if jump_target == next {
                if next == BlockIdx::NULL {
                    return false;
                }
                if let Some(first_next) = blocks[next.idx()].instructions.first()
                    && instruction_lineno(last) == instruction_lineno(first_next)
                {
                    return false;
                }
            }
        }
        current = block.next;
    }
    true
}

fn remove_redundant_nops_and_jumps(blocks: &mut [Block]) -> crate::InternalResult<()> {
    loop {
        let removed_nops = remove_redundant_nops(blocks);
        let removed_jumps = remove_redundant_jumps(blocks)?;
        if removed_nops + removed_jumps == 0 {
            break;
        }
    }
    Ok(())
}

fn layout_block_order(blocks: &[Block]) -> Vec<BlockIdx> {
    let mut order = Vec::new();
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        order.push(current);
        current = blocks[current.idx()].next;
    }
    order
}

/// flowgraph.c struct _PyCfgBuilder
struct CfgBuilder {
    blocks: Vec<Block>,
    current: BlockIdx,
    current_label: Option<InstructionSequenceLabel>,
}

impl CfgBuilder {
    /// flowgraph.c init_cfg_builder
    fn new_with_capacity(capacity: usize) -> Self {
        let mut blocks = Vec::with_capacity(capacity.max(1));
        blocks.push(Block::default());
        Self {
            blocks,
            current: BlockIdx(0),
            current_label: None,
        }
    }

    /// flowgraph.c cfg_builder_current_block_is_terminated
    fn current_block_is_terminated(&mut self) -> bool {
        let block = &mut self.blocks[self.current.idx()];
        if block
            .instructions
            .last()
            .is_some_and(|instr| instr.instr.is_terminator())
        {
            return true;
        }
        if let Some(label_id) = self.current_label {
            if !block.instructions.is_empty() || block.has_cpython_cfg_label() {
                return true;
            }
            block.cpython_label_id = Some(label_id);
            self.current_label = None;
        }
        false
    }

    /// flowgraph.c cfg_builder_maybe_start_new_block
    fn maybe_start_new_block(&mut self) {
        if self.current_block_is_terminated() {
            let next = BlockIdx(self.blocks.len() as u32);
            let block = Block {
                cpython_label_id: self.current_label.take(),
                ..Default::default()
            };
            self.blocks[self.current.idx()].next = next;
            self.blocks.push(block);
            self.current = next;
        }
    }

    /// flowgraph.c _PyCfgBuilder_UseLabel
    fn use_label(&mut self, label_id: InstructionSequenceLabel) {
        self.current_label = Some(label_id);
        self.maybe_start_new_block();
    }

    /// flowgraph.c _PyCfgBuilder_Addop
    fn addop(&mut self, info: InstructionInfo) {
        self.maybe_start_new_block();
        basicblock_addop(&mut self.blocks[self.current.idx()], info);
    }

    fn into_blocks(self) -> Vec<Block> {
        self.blocks
    }
}

/// flowgraph.c translate_jump_labels_to_targets
fn translate_jump_labels_to_targets(blocks: &mut [Block]) -> crate::InternalResult<()> {
    let max_label = iter_blocks(blocks)
        .filter_map(|(_, block)| block.cpython_label_id)
        .map(InstructionSequenceLabel::idx)
        .max();
    let mut label_to_block = max_label.map_or_else(Vec::new, |max_label| {
        vec![BlockIdx::NULL; max_label.saturating_add(1)]
    });
    for (block_idx, block) in iter_blocks(blocks) {
        if let Some(label_id) = block.cpython_label_id {
            let slot = label_to_block
                .get_mut(label_id.idx())
                .ok_or(InternalError::MalformedControlFlowGraph)?;
            *slot = block_idx;
        }
    }

    let mut block_idx = BlockIdx(0);
    while block_idx != BlockIdx::NULL {
        let next = blocks[block_idx.idx()].next;
        for info in &mut blocks[block_idx.idx()].instructions {
            if info.instr.has_target() {
                let label_id = InstructionSequenceLabel(u32::from(info.arg) as usize);
                let target = label_to_block
                    .get(label_id.idx())
                    .copied()
                    .filter(|target| *target != BlockIdx::NULL)
                    .ok_or(InternalError::MalformedControlFlowGraph)?;
                info.target = target;
            }
        }
        block_idx = next;
    }
    Ok(())
}

/// flowgraph.c _PyCfg_FromInstructionSequence
fn cfg_from_instruction_sequence(
    mut instr_sequence: InstructionSequence,
) -> crate::InternalResult<Vec<Block>> {
    instr_sequence.apply_label_map()?;
    instr_sequence.mark_targets()?;
    let InstructionSequence {
        instrs,
        label_map,
        annotations_code,
    } = instr_sequence;
    debug_assert!(matches!(
        label_map,
        InstructionSequenceLabelOffsets::Applied
    ));

    let final_capacity = instrs.len() + annotations_code.as_ref().map_or(0, |seq| seq.instrs.len());
    let mut sequence = Vec::with_capacity(final_capacity);
    let mut builder = CfgBuilder::new_with_capacity(final_capacity);
    let mut offset_delta = 0isize;

    for mut entry in instrs {
        if matches!(
            entry.info.instr.pseudo(),
            Some(PseudoInstruction::AnnotationsPlaceholder)
        ) {
            if let Some(annotations_code) = &annotations_code {
                debug_assert!(matches!(
                    annotations_code.label_map,
                    InstructionSequenceLabelOffsets::Applied
                ));
                debug_assert!(annotations_code.annotations_code.is_none());
                for ann_entry in annotations_code.instrs.iter().copied() {
                    if ann_entry.info.instr.has_target() {
                        return Err(InternalError::MalformedControlFlowGraph);
                    }
                    let mut info = ann_entry.info;
                    info.target = BlockIdx::NULL;
                    builder.addop(info);
                    sequence.push(ann_entry);
                }
                offset_delta += annotations_code.instrs.len() as isize - 1;
            } else {
                offset_delta -= 1;
            }
            continue;
        }

        if entry.is_target {
            let label = InstructionSequenceLabel(sequence.len());
            builder.use_label(label);
        }

        if let Some(target_offset) = entry.target_offset {
            entry.target_offset = Some(
                target_offset
                    .checked_add_signed(offset_delta)
                    .ok_or(InternalError::MalformedControlFlowGraph)?,
            );
        }
        let mut info = entry.info;
        if let Some(target_offset) = entry.target_offset {
            info.arg = OpArg::new(
                target_offset
                    .to_u32()
                    .ok_or(InternalError::MalformedControlFlowGraph)?,
            );
        }
        info.target = BlockIdx::NULL;
        builder.addop(info);
        sequence.push(entry);
    }

    for entry in &sequence {
        if entry
            .target_offset
            .is_some_and(|target_offset| target_offset >= sequence.len())
        {
            return Err(InternalError::MalformedControlFlowGraph);
        }
    }

    Ok(builder.into_blocks())
}

fn maybe_push_local_block(
    worklist: &mut Vec<BlockIdx>,
    on_stack: &mut [bool],
    unsafe_masks: &mut [u64],
    block: BlockIdx,
    unsafe_mask: u64,
) {
    if block == BlockIdx::NULL {
        return;
    }

    let idx = block.idx();
    let both = unsafe_masks[idx] | unsafe_mask;
    if unsafe_masks[idx] != both {
        unsafe_masks[idx] = both;
        if !on_stack[idx] {
            worklist.push(block);
            on_stack[idx] = true;
        }
    }
}

fn scan_block_for_locals(
    blocks: &mut [Block],
    block_idx: BlockIdx,
    worklist: &mut Vec<BlockIdx>,
    on_stack: &mut [bool],
    unsafe_masks: &mut [u64],
) {
    let idx = block_idx.idx();
    let mut unsafe_mask = unsafe_masks[idx];
    let instr_count = blocks[idx].instructions.len();

    for i in 0..instr_count {
        let (instr, arg, except_handler) = {
            let info = &blocks[idx].instructions[i];
            (
                info.instr,
                info.arg,
                info.except_handler.map(|eh| eh.handler_block),
            )
        };

        if let Some(handler_block) = except_handler {
            maybe_push_local_block(worklist, on_stack, unsafe_masks, handler_block, unsafe_mask);
        }

        let (local_idx, action) = match instr {
            AnyInstruction::Pseudo(PseudoInstruction::StoreFastMaybeNull { var_num }) => {
                (var_num.get(arg) as usize, LocalScanAction::SetUnsafe)
            }
            AnyInstruction::Real(
                Instruction::DeleteFast { var_num } | Instruction::LoadFastAndClear { var_num },
            ) => (usize::from(var_num.get(arg)), LocalScanAction::SetUnsafe),
            AnyInstruction::Real(Instruction::StoreFast { var_num }) => {
                (usize::from(var_num.get(arg)), LocalScanAction::ClearUnsafe)
            }
            AnyInstruction::Real(Instruction::LoadFastCheck { var_num }) => {
                (usize::from(var_num.get(arg)), LocalScanAction::ClearUnsafe)
            }
            AnyInstruction::Real(Instruction::LoadFast { var_num }) => {
                (usize::from(var_num.get(arg)), LocalScanAction::CheckLoad)
            }
            _ => continue,
        };
        if local_idx >= 64 {
            continue;
        }

        let bit = 1u64 << local_idx;
        match action {
            LocalScanAction::SetUnsafe => unsafe_mask |= bit,
            LocalScanAction::ClearUnsafe => unsafe_mask &= !bit,
            LocalScanAction::CheckLoad => {
                if unsafe_mask & bit != 0 {
                    blocks[idx].instructions[i].instr = Opcode::LoadFastCheck.into();
                }
                unsafe_mask &= !bit;
            }
        }
    }

    let block = &blocks[idx];
    if block.next != BlockIdx::NULL && block_has_fallthrough(block) {
        maybe_push_local_block(worklist, on_stack, unsafe_masks, block.next, unsafe_mask);
    }

    if let Some(last) = block.instructions.last()
        && is_jump_instruction(last)
        && last.target != BlockIdx::NULL
    {
        maybe_push_local_block(worklist, on_stack, unsafe_masks, last.target, unsafe_mask);
    }
}

enum LocalScanAction {
    SetUnsafe,
    ClearUnsafe,
    CheckLoad,
}

/// Follow chain of empty blocks to find first non-empty block.
fn next_nonempty_block(blocks: &[Block], mut idx: BlockIdx) -> BlockIdx {
    while idx != BlockIdx::NULL && blocks[idx.idx()].instructions.is_empty() {
        idx = blocks[idx.idx()].next;
    }
    idx
}

fn instruction_lineno(instr: &InstructionInfo) -> i32 {
    match instr.lineno_override {
        Some(LINE_ONLY_LOCATION_OVERRIDE) | None => instr.location.line.get() as i32,
        Some(lineno) => lineno,
    }
}

fn instruction_has_lineno(instr: &InstructionInfo) -> bool {
    instruction_lineno(instr) >= 0
}

fn copy_instruction_location(source: InstructionInfo, target: &mut InstructionInfo) {
    target.location = source.location;
    target.end_location = source.end_location;
    target.lineno_override = source.lineno_override;
}

fn propagation_location(
    instr: &InstructionInfo,
) -> Option<(SourceLocation, SourceLocation, Option<i32>)> {
    instruction_has_lineno(instr).then_some((
        instr.location,
        instr.end_location,
        instr.lineno_override,
    ))
}

/// flowgraph.c basicblock_nofallthrough / BB_NO_FALLTHROUGH
fn basicblock_nofallthrough(block: &Block) -> bool {
    block
        .instructions
        .last()
        .is_some_and(|ins| ins.instr.is_no_fallthrough())
}

/// flowgraph.c BB_HAS_FALLTHROUGH
fn block_has_fallthrough(block: &Block) -> bool {
    !basicblock_nofallthrough(block)
}

fn is_jump_instruction(instr: &InstructionInfo) -> bool {
    instr.instr.has_jump()
}

fn last_jump_for_line_propagation(block: &Block) -> Option<InstructionInfo> {
    let last = block.instructions.last().copied()?;
    is_jump_instruction(&last).then_some(last)
}

/// flowgraph.c basicblock_exits_scope
fn basicblock_exits_scope(block: &Block) -> bool {
    block
        .instructions
        .last()
        .is_some_and(|instr| instr.instr.is_scope_exit())
}

/// flowgraph.c is_exit_or_eval_check_without_lineno
fn is_exit_or_eval_check_without_lineno(blocks: &[Block], block_idx: BlockIdx) -> bool {
    let block = &blocks[block_idx.idx()];
    if basicblock_exits_scope(block) || basicblock_has_eval_break(block) {
        block_has_no_lineno(block)
    } else {
        false
    }
}

/// flowgraph.c basicblock_has_eval_break
fn basicblock_has_eval_break(block: &Block) -> bool {
    block
        .instructions
        .iter()
        .any(|info| info.instr.has_eval_break())
}

fn block_has_no_lineno(block: &Block) -> bool {
    block
        .instructions
        .iter()
        .all(|ins| !instruction_has_lineno(ins))
}

fn is_scope_exit_block(block: &Block) -> bool {
    basicblock_exits_scope(block)
}

fn maybe_propagate_location(
    instr: &mut InstructionInfo,
    location: SourceLocation,
    end_location: SourceLocation,
    lineno_override: Option<i32>,
) {
    if instr.lineno_override != Some(NEXT_LOCATION_OVERRIDE) && !instruction_has_lineno(instr) {
        instr.location = location;
        instr.end_location = end_location;
        instr.lineno_override = lineno_override;
    }
}

fn propagate_line_numbers_in_block(
    block: &mut Block,
) -> Option<(SourceLocation, SourceLocation, Option<i32>)> {
    let mut prev_location = None;
    for instr in &mut block.instructions {
        if let Some((location, end_location, lineno_override)) = prev_location {
            maybe_propagate_location(instr, location, end_location, lineno_override);
        }
        prev_location = propagation_location(instr);
    }
    prev_location
}

fn overwrite_location(
    instr: &mut InstructionInfo,
    location: SourceLocation,
    end_location: SourceLocation,
    lineno_override: Option<i32>,
) {
    instr.location = location;
    instr.end_location = end_location;
    instr.lineno_override = lineno_override;
}

fn compute_predecessors(blocks: &[Block]) -> Vec<u32> {
    let mut predecessors = vec![0u32; blocks.len()];
    if blocks.is_empty() {
        return predecessors;
    }

    predecessors[0] = 1;
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let block = &blocks[current.idx()];
        if block_has_fallthrough(block) {
            let next = block.next;
            if next != BlockIdx::NULL {
                predecessors[next.idx()] += 1;
            }
        }
        for ins in &block.instructions {
            if ins.instr.has_target() && ins.target != BlockIdx::NULL {
                predecessors[ins.target.idx()] += 1;
            }
        }
        current = block.next;
    }
    predecessors
}

/// flowgraph.c copy_basicblock
fn copy_basicblock(blocks: &[Block], block_idx: BlockIdx) -> Block {
    let block = &blocks[block_idx.idx()];
    debug_assert!(!block_has_fallthrough(block));
    Block {
        instructions: block.instructions.clone(),
        ..Block::default()
    }
}

fn duplicate_exits_without_lineno(blocks: &mut Vec<Block>, predecessors: &mut Vec<u32>) {
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let block = &blocks[current.idx()];
        let last = match block.instructions.last() {
            Some(ins) if ins.target != BlockIdx::NULL && is_jump_instruction(ins) => ins,
            _ => {
                current = blocks[current.idx()].next;
                continue;
            }
        };

        let target = next_nonempty_block(blocks, last.target);
        if target == BlockIdx::NULL
            || !is_exit_or_eval_check_without_lineno(blocks, target)
            || predecessors[target.idx()] <= 1
        {
            current = blocks[current.idx()].next;
            continue;
        }

        let new_idx = BlockIdx(blocks.len() as u32);
        let mut new_block = copy_basicblock(blocks, target);
        if let Some(first) = new_block.instructions.first_mut() {
            overwrite_location(
                first,
                last.location,
                last.end_location,
                last.lineno_override,
            );
        }
        let old_next = blocks[target.idx()].next;
        new_block.next = old_next;
        blocks.push(new_block);
        blocks[target.idx()].next = new_idx;

        let last_mut = blocks[current.idx()].instructions.last_mut().unwrap();
        last_mut.target = new_idx;
        predecessors[target.idx()] -= 1;
        predecessors.push(1);
        current = blocks[current.idx()].next;
    }

    current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let (next_block, last_location) = {
            let block = &blocks[current.idx()];
            (
                block_has_fallthrough(block).then_some(block.next),
                block
                    .instructions
                    .last()
                    .map(|last| (last.location, last.end_location, last.lineno_override)),
            )
        };
        if let (Some(target), Some((location, end_location, lineno_override))) =
            (next_block, last_location)
            && target != BlockIdx::NULL
            && is_exit_or_eval_check_without_lineno(blocks, target)
            && let Some(first) = blocks[target.idx()].instructions.first_mut()
        {
            overwrite_location(first, location, end_location, lineno_override);
        }
        current = blocks[current.idx()].next;
    }
}

fn propagate_line_numbers(blocks: &mut [Block], predecessors: &[u32]) {
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        if !blocks[current.idx()].instructions.is_empty() {
            let (next_block, has_fallthrough) = {
                let block = &blocks[current.idx()];
                (block.next, block_has_fallthrough(block))
            };

            let prev_location = propagate_line_numbers_in_block(&mut blocks[current.idx()]);
            let last_jump = last_jump_for_line_propagation(&blocks[current.idx()]);

            if has_fallthrough
                && next_block != BlockIdx::NULL
                && predecessors[next_block.idx()] == 1
                && let Some((location, end_location, lineno_override)) = prev_location
                && let Some(first) = blocks[next_block.idx()].instructions.first_mut()
            {
                maybe_propagate_location(first, location, end_location, lineno_override);
            }

            if let Some(last_jump) = last_jump {
                let target = last_jump.target;
                if target != BlockIdx::NULL
                    && predecessors[target.idx()] == 1
                    && let Some((location, end_location, lineno_override)) = prev_location
                    && let Some(first) = blocks[target.idx()].instructions.first_mut()
                {
                    maybe_propagate_location(first, location, end_location, lineno_override);
                }
            }
        }
        current = blocks[current.idx()].next;
    }
}

fn resolve_line_numbers(blocks: &mut Vec<Block>) {
    let mut predecessors = compute_predecessors(blocks);
    duplicate_exits_without_lineno(blocks, &mut predecessors);
    propagate_line_numbers(blocks, &predecessors);
}

pub(crate) fn label_exception_targets(blocks: &mut [Block]) -> crate::InternalResult<()> {
    fn except_stack_top(stack: &[BlockIdx], blocks: &[Block]) -> Option<ExceptHandlerInfo> {
        let handler_block = stack.last().copied()?;
        if handler_block == BlockIdx::NULL {
            return None;
        }
        Some(ExceptHandlerInfo {
            handler_block,
            stack_depth: 0,
            preserve_lasti: blocks[handler_block.idx()].preserve_lasti,
        })
    }

    fn push_except_block(
        stack: &mut Vec<BlockIdx>,
        instr: AnyInstruction,
        target: BlockIdx,
        blocks: &mut [Block],
    ) -> crate::InternalResult<Option<ExceptHandlerInfo>> {
        debug_assert!(instr.is_block_push());
        if target == BlockIdx::NULL {
            return Err(InternalError::MalformedControlFlowGraph);
        }
        if matches!(
            instr.pseudo(),
            Some(PseudoInstruction::SetupWith { .. } | PseudoInstruction::SetupCleanup { .. })
        ) {
            blocks[target.idx()].preserve_lasti = true;
        }
        stack.push(target);
        Ok(except_stack_top(stack, blocks))
    }

    fn pop_except_block(
        stack: &mut Vec<BlockIdx>,
        blocks: &[Block],
    ) -> crate::InternalResult<Option<ExceptHandlerInfo>> {
        debug_assert!(!stack.is_empty());
        if stack.is_empty() {
            return Err(InternalError::MalformedControlFlowGraph);
        }
        stack.pop();
        Ok(except_stack_top(stack, blocks))
    }

    let num_blocks = blocks.len();
    if num_blocks == 0 {
        return Ok(());
    }

    let mut visited = vec![false; num_blocks];
    let mut block_stacks: Vec<Option<Vec<BlockIdx>>> = vec![None; num_blocks];

    // Entry block
    visited[0] = true;
    block_stacks[0] = Some(Vec::new());

    let mut todo = vec![BlockIdx(0)];

    while let Some(block_idx) = todo.pop() {
        let bi = block_idx.idx();
        let mut stack = block_stacks[bi].take().unwrap_or_default();
        let mut handler = except_stack_top(&stack, blocks);
        let mut last_yield_except_depth: i32 = -1;

        let instr_count = blocks[bi].instructions.len();
        for i in 0..instr_count {
            let instr = blocks[bi].instructions[i].instr;
            let target = blocks[bi].instructions[i].target;
            let arg = blocks[bi].instructions[i].arg;

            if instr.is_block_push() {
                if target == BlockIdx::NULL {
                    return Err(InternalError::MalformedControlFlowGraph);
                }
                if !visited[target.idx()] {
                    visited[target.idx()] = true;
                    block_stacks[target.idx()] = Some(stack.clone());
                    todo.push(target);
                }
                handler = push_except_block(&mut stack, instr, target, blocks)?;
            } else if instr.is_pop_block() {
                handler = pop_except_block(&mut stack, blocks)?;
                set_to_nop(&mut blocks[bi].instructions[i]);
            } else if is_jump_instruction(&blocks[bi].instructions[i]) {
                blocks[bi].instructions[i].except_handler = handler;
                debug_assert_eq!(i, instr_count - 1);

                // CPython label_exception_targets(): copy the except stack
                // when this block can also fall through, otherwise transfer it
                // to the jump target.
                if target == BlockIdx::NULL {
                    return Err(InternalError::MalformedControlFlowGraph);
                }
                if !visited[target.idx()] {
                    visited[target.idx()] = true;
                    block_stacks[target.idx()] = Some(if block_has_fallthrough(&blocks[bi]) {
                        stack.clone()
                    } else {
                        core::mem::take(&mut stack)
                    });
                    todo.push(target);
                }
            } else if matches!(instr.real(), Some(Instruction::YieldValue { .. })) {
                blocks[bi].instructions[i].except_handler = handler;
                last_yield_except_depth = stack.len() as i32;
            } else if let Some(Instruction::Resume { context }) = instr.real() {
                blocks[bi].instructions[i].except_handler = handler;
                let location = context.get(arg).location();
                if !matches!(location, oparg::ResumeLocation::AtFuncStart) {
                    debug_assert!(last_yield_except_depth >= 0);
                    if last_yield_except_depth == 1 {
                        blocks[bi].instructions[i].arg =
                            OpArg::new(oparg::ResumeContext::new(location, true).as_u32());
                    }
                    last_yield_except_depth = -1;
                }
            } else {
                blocks[bi].instructions[i].except_handler = handler;
            }
        }

        let next = blocks[bi].next;
        if block_has_fallthrough(&blocks[bi]) && next != BlockIdx::NULL && !visited[next.idx()] {
            visited[next.idx()] = true;
            block_stacks[next.idx()] = Some(stack);
            todo.push(next);
        }
    }
    debug_assert!(block_stacks.iter().all(Option::is_none));
    Ok(())
}

/// Convert remaining pseudo ops to real instructions or NOP.
/// flowgraph.c convert_pseudo_ops
pub(crate) fn convert_pseudo_ops(blocks: &mut [Block]) -> crate::InternalResult<()> {
    let block_order = layout_block_order(blocks);
    for block_idx in block_order {
        let block = &mut blocks[block_idx.idx()];
        for info in &mut block.instructions {
            let Some(pseudo) = info.instr.pseudo() else {
                continue;
            };
            match pseudo {
                // Block push pseudo ops → NOP
                PseudoInstruction::SetupCleanup { .. }
                | PseudoInstruction::SetupFinally { .. }
                | PseudoInstruction::SetupWith { .. } => {
                    set_to_nop(info);
                }
                PseudoInstruction::LoadClosure { .. } => {
                    info.instr = Opcode::LoadFast.into();
                }
                // Jump pseudo ops are resolved during block linearization
                PseudoInstruction::Jump { .. } | PseudoInstruction::JumpNoInterrupt { .. } => {}
                PseudoInstruction::StoreFastMaybeNull { .. } => {
                    info.instr = Instruction::StoreFast {
                        var_num: Arg::marker(),
                    }
                    .into();
                }
                // These should have been resolved earlier
                PseudoInstruction::PopBlock
                | PseudoInstruction::AnnotationsPlaceholder
                | PseudoInstruction::JumpIfFalse { .. }
                | PseudoInstruction::JumpIfTrue { .. } => {
                    unreachable!("Unexpected pseudo instruction in convert_pseudo_ops: {pseudo:?}")
                }
            }
        }
    }
    // CPython flowgraph.c::convert_pseudo_ops() finishes by calling
    // remove_redundant_nops_and_jumps().
    remove_redundant_nops_and_jumps(blocks)
}

/// flowgraph.c build_cellfixedoffsets
pub(crate) fn build_cellfixedoffsets(
    varnames: &IndexSet<String>,
    cellvars: &IndexSet<String>,
    freevars: &IndexSet<String>,
) -> Vec<u32> {
    let nlocals = varnames.len();
    let ncells = cellvars.len();
    let nfrees = freevars.len();
    let mut fixed = (0..ncells + nfrees)
        .map(|i| (nlocals + i).to_u32().expect("too many localsplus slots"))
        .collect::<Vec<_>>();
    for (oldindex, cellvar) in cellvars.iter().enumerate() {
        if let Some(local_idx) = varnames.get_index_of(cellvar) {
            fixed[oldindex] = local_idx.to_u32().expect("too many localsplus slots");
        }
    }
    fixed
}

/// First half of flowgraph.c fix_cell_offsets.
fn fix_cellfixedoffsets(nlocals: usize, fixedmap: &mut [u32]) -> usize {
    let mut numdropped = 0usize;
    for (i, fixed) in fixedmap.iter_mut().enumerate() {
        if usize::try_from(*fixed).expect("localsplus index overflow") == i + nlocals {
            *fixed -= numdropped.to_u32().expect("too many dropped cell vars");
        } else {
            numdropped += 1;
        }
    }
    numdropped
}

/// flowgraph.c fix_cell_offsets
pub(crate) fn fix_cell_offsets(
    blocks: &mut [Block],
    nlocals: usize,
    cellfixedoffsets: &mut [u32],
) -> usize {
    let numdropped = fix_cellfixedoffsets(nlocals, cellfixedoffsets);
    let block_order = layout_block_order(blocks);
    for block_idx in block_order {
        let block = &mut blocks[block_idx.idx()];
        for info in &mut block.instructions {
            debug_assert!(
                !matches!(info.instr.real(), Some(Instruction::ExtendedArg)),
                "fix_cell_offsets is called before extended args are generated"
            );
            let needs_fixup = matches!(
                info.instr,
                AnyInstruction::Real(
                    Instruction::LoadDeref { .. }
                        | Instruction::StoreDeref { .. }
                        | Instruction::DeleteDeref { .. }
                        | Instruction::LoadFromDictOrDeref { .. }
                        | Instruction::MakeCell { .. }
                ) | AnyInstruction::Pseudo(PseudoInstruction::LoadClosure { .. })
            );
            if needs_fixup {
                let cell_relative = u32::from(info.arg) as usize;
                debug_assert!(cell_relative < cellfixedoffsets.len());
                info.arg = OpArg::new(cellfixedoffsets[cell_relative]);
            }
        }
    }
    numdropped
}
