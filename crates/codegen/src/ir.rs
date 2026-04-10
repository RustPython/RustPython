use alloc::collections::VecDeque;
use core::ops;

use crate::{IndexMap, IndexSet, error::InternalError};
use malachite_bigint::BigInt;
use num_traits::{ToPrimitive, Zero};

use rustpython_compiler_core::{
    OneIndexed, SourceLocation,
    bytecode::{
        AnyInstruction, Arg, CO_FAST_CELL, CO_FAST_FREE, CO_FAST_HIDDEN, CO_FAST_LOCAL, CodeFlags,
        CodeObject, CodeUnit, CodeUnits, ConstantData, ExceptionTableEntry, InstrDisplayContext,
        Instruction, InstructionMetadata, Label, OpArg, PseudoInstruction, PyCodeLocationInfoKind,
        encode_exception_table, oparg,
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

const MAX_INT_SIZE_BITS: u64 = 128;
const MIN_CONST_SEQUENCE_SIZE: usize = 3;

/// Metadata for a code unit
// = _PyCompile_CodeUnitMetadata
#[derive(Clone, Debug)]
pub struct CodeUnitMetadata {
    pub name: String,                        // u_name (obj_name)
    pub qualname: Option<String>,            // u_qualname
    pub consts: IndexSet<ConstantData>,      // u_consts
    pub names: IndexSet<String>,             // u_names
    pub varnames: IndexSet<String>,          // u_varnames
    pub cellvars: IndexSet<String>,          // u_cellvars
    pub freevars: IndexSet<String>,          // u_freevars
    pub fast_hidden: IndexMap<String, bool>, // u_fast_hidden
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
        }
    }
}

pub struct CodeInfo {
    pub flags: CodeFlags,
    pub source_path: String,
    pub private: Option<String>, // For private name mangling, mostly for class

    pub blocks: Vec<Block>,
    pub current_block: BlockIdx,

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
    pub fn finalize_code(
        mut self,
        opts: &crate::compile::CompileOpts,
    ) -> crate::InternalResult<CodeObject> {
        // Constant folding passes
        self.fold_binop_constants();
        self.remove_nops();
        self.fold_unary_negative();
        self.fold_binop_constants(); // re-run after unary folding: -1 + 2 → 1
        self.remove_nops(); // remove NOPs so tuple/list/set see contiguous LOADs
        self.fold_tuple_constants();
        self.fold_list_constants();
        self.fold_set_constants();
        self.remove_nops(); // remove NOPs from collection folding
        self.fold_const_iterable_for_iter();
        self.convert_to_load_small_int();
        self.remove_unused_consts();
        self.remove_nops();

        // DCE always runs (removes dead code after terminal instructions)
        self.dce();
        // BUILD_TUPLE n + UNPACK_SEQUENCE n → NOP + SWAP (n=2,3) or NOP+NOP (n=1)
        self.optimize_build_tuple_unpack();
        // Dead store elimination for duplicate STORE_FAST targets
        // (apply_static_swaps in CPython's flowgraph.c)
        self.eliminate_dead_stores();
        // apply_static_swaps: reorder stores to eliminate SWAPs
        self.apply_static_swaps();
        // Peephole optimizer creates superinstructions matching CPython
        self.peephole_optimize();

        // Phase 1: _PyCfg_OptimizeCodeUnit (flowgraph.c)
        // Split blocks so each block has at most one branch as its last instruction
        split_blocks_at_jumps(&mut self.blocks);
        mark_except_handlers(&mut self.blocks);
        label_exception_targets(&mut self.blocks);
        // optimize_cfg: jump threading (before push_cold_blocks_to_end)
        jump_threading(&mut self.blocks);
        self.eliminate_unreachable_blocks();
        self.remove_nops();
        // TODO: insert_superinstructions disabled pending StoreFastLoadFast VM fix
        push_cold_blocks_to_end(&mut self.blocks);

        // Phase 2: _PyCfg_OptimizedCfgToInstructionSequence (flowgraph.c)
        normalize_jumps(&mut self.blocks);
        reorder_conditional_exit_and_jump_blocks(&mut self.blocks);
        reorder_conditional_jump_and_exit_blocks(&mut self.blocks);
        inline_small_or_no_lineno_blocks(&mut self.blocks);
        self.dce(); // re-run within-block DCE after normalize_jumps creates new instructions
        self.eliminate_unreachable_blocks();
        resolve_line_numbers(&mut self.blocks);
        duplicate_end_returns(&mut self.blocks);
        self.dce(); // truncate after terminal in blocks that got return duplicated
        self.eliminate_unreachable_blocks(); // remove now-unreachable last block
        remove_redundant_nops_and_jumps(&mut self.blocks);
        // Some jump-only blocks only appear after late CFG cleanup. Thread them
        // once more so loop backedges stay direct instead of becoming
        // JUMP_FORWARD -> JUMP_BACKWARD chains.
        jump_threading_unconditional(&mut self.blocks);
        reorder_conditional_exit_and_jump_blocks(&mut self.blocks);
        reorder_conditional_jump_and_exit_blocks(&mut self.blocks);
        self.eliminate_unreachable_blocks();
        remove_redundant_nops_and_jumps(&mut self.blocks);
        self.add_checks_for_loads_of_uninitialized_variables();
        // optimize_load_fast: after normalize_jumps
        self.optimize_load_fast_borrow();
        self.optimize_load_global_push_null();

        let max_stackdepth = self.max_stackdepth()?;

        let Self {
            flags,
            source_path,
            private: _, // private is only used during compilation

            mut blocks,
            current_block: _,
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
            argcount: arg_count,
            posonlyargcount: posonlyarg_count,
            kwonlyargcount: kwonlyarg_count,
            firstlineno: first_line_number,
        } = metadata;

        let mut instructions = Vec::new();
        let mut locations = Vec::new();
        let mut linetable_locations: Vec<LineTableLocation> = Vec::new();

        // Build cellfixedoffsets for cell-local merging
        let cellfixedoffsets =
            build_cellfixedoffsets(&varname_cache, &cellvar_cache, &freevar_cache);
        // Convert pseudo ops (LoadClosure uses cellfixedoffsets) and fixup DEREF opargs
        convert_pseudo_ops(&mut blocks, &cellfixedoffsets);
        fixup_deref_opargs(&mut blocks, &cellfixedoffsets);
        // Remove redundant NOPs, keeping line-marker NOPs only when
        // they are needed to preserve tracing.
        let mut block_order = Vec::new();
        let mut current = BlockIdx(0);
        while current != BlockIdx::NULL {
            block_order.push(current);
            current = blocks[current.idx()].next;
        }
        for block_idx in block_order {
            let bi = block_idx.idx();
            let mut src_instructions = core::mem::take(&mut blocks[bi].instructions);
            let mut kept = Vec::with_capacity(src_instructions.len());
            let mut prev_lineno = -1i32;

            for src in 0..src_instructions.len() {
                let instr = src_instructions[src];
                let lineno = instr
                    .lineno_override
                    .unwrap_or_else(|| instr.location.line.get() as i32);
                let mut remove = false;

                if matches!(instr.instr.real(), Some(Instruction::Nop)) {
                    // Remove location-less NOPs.
                    if lineno < 0 || prev_lineno == lineno {
                        remove = true;
                    }
                    // Remove if the next instruction has same line or no line.
                    else if src < src_instructions.len() - 1 {
                        let next_lineno =
                            src_instructions[src + 1]
                                .lineno_override
                                .unwrap_or_else(|| {
                                    src_instructions[src + 1].location.line.get() as i32
                                });
                        if next_lineno == lineno {
                            remove = true;
                        } else if next_lineno < 0 {
                            src_instructions[src + 1].lineno_override = Some(lineno);
                            remove = true;
                        }
                    }
                    // Last instruction in block: compare with first real location
                    // in the next non-empty block.
                    else {
                        let mut next = blocks[bi].next;
                        while next != BlockIdx::NULL && blocks[next.idx()].instructions.is_empty() {
                            next = blocks[next.idx()].next;
                        }
                        if next != BlockIdx::NULL {
                            let mut next_lineno = None;
                            for next_instr in &blocks[next.idx()].instructions {
                                let line = next_instr
                                    .lineno_override
                                    .unwrap_or_else(|| next_instr.location.line.get() as i32);
                                if matches!(next_instr.instr.real(), Some(Instruction::Nop))
                                    && line < 0
                                {
                                    continue;
                                }
                                next_lineno = Some(line);
                                break;
                            }
                            if next_lineno.is_some_and(|line| line == lineno) {
                                remove = true;
                            }
                        }
                    }
                }

                if !remove {
                    kept.push(instr);
                    prev_lineno = lineno;
                }
            }

            blocks[bi].instructions = kept;
        }

        // Final DCE: truncate instructions after terminal ops in linearized blocks.
        // This catches dead code created by normalize_jumps after the initial DCE.
        for block in blocks.iter_mut() {
            if let Some(pos) = block
                .instructions
                .iter()
                .position(|ins| ins.instr.is_scope_exit() || ins.instr.is_unconditional_jump())
            {
                block.instructions.truncate(pos + 1);
            }
        }

        // Pre-compute cache_entries for real (non-pseudo) instructions
        for block in blocks.iter_mut() {
            for instr in &mut block.instructions {
                if let AnyInstruction::Real(op) = instr.instr {
                    instr.cache_entries = op.cache_entries() as u32;
                }
            }
        }

        let mut block_to_offset = vec![Label::from_u32(0); blocks.len()];
        // block_to_index: maps block idx to instruction index (for exception table)
        // This is the index into the final instructions array, including EXTENDED_ARG and CACHE
        let mut block_to_index = vec![0u32; blocks.len()];
        // The offset (in code units) of END_SEND from SEND in the yield-from sequence.
        const END_SEND_OFFSET: u32 = 5;
        loop {
            let mut num_instructions = 0;
            for (idx, block) in iter_blocks(&blocks) {
                block_to_offset[idx.idx()] = Label::from_u32(num_instructions as u32);
                // block_to_index uses the same value as block_to_offset but as u32
                // because lasti in frame.rs is the index into instructions array
                // and instructions array index == byte offset (each instruction is 1 CodeUnit)
                block_to_index[idx.idx()] = num_instructions as u32;
                for instr in &block.instructions {
                    num_instructions += instr.arg.instr_size() + instr.cache_entries as usize;
                }
            }

            instructions.reserve_exact(num_instructions);
            locations.reserve_exact(num_instructions);

            let mut recompile = false;
            let mut next_block = BlockIdx(0);
            while next_block != BlockIdx::NULL {
                let block = &mut blocks[next_block];
                // Track current instruction offset for jump direction resolution
                let mut current_offset = block_to_offset[next_block.idx()].as_u32();
                for info in &mut block.instructions {
                    let target = info.target;
                    let mut op = info.instr.expect_real();
                    let old_arg_size = info.arg.instr_size();
                    let old_cache_entries = info.cache_entries;
                    // Keep offsets fixed within this pass: changes in jump
                    // arg/cache sizes only take effect in the next iteration.
                    let offset_after = current_offset + old_arg_size as u32 + old_cache_entries;

                    if target != BlockIdx::NULL {
                        let target_offset = block_to_offset[target.idx()].as_u32();
                        // Direction must be based on concrete instruction offsets.
                        // Empty blocks can share offsets, so block-order-based resolution
                        // may classify some jumps incorrectly.
                        op = match op {
                            Instruction::JumpForward { .. } if target_offset <= current_offset => {
                                Instruction::JumpBackward {
                                    delta: Arg::marker(),
                                }
                            }
                            Instruction::JumpBackward { .. } if target_offset > current_offset => {
                                Instruction::JumpForward {
                                    delta: Arg::marker(),
                                }
                            }
                            Instruction::JumpBackwardNoInterrupt { .. }
                                if target_offset > current_offset =>
                            {
                                Instruction::JumpForward {
                                    delta: Arg::marker(),
                                }
                            }
                            _ => op,
                        };
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
                            op,
                            Instruction::JumpBackward { .. }
                                | Instruction::JumpBackwardNoInterrupt { .. }
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
                    let lt_loc = LineTableLocation {
                        line: info
                            .lineno_override
                            .unwrap_or_else(|| info.location.line.get() as i32),
                        end_line: info.end_location.line.get() as i32,
                        col: info.location.character_offset.to_zero_indexed() as i32,
                        end_col: info.end_location.character_offset.to_zero_indexed() as i32,
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
                    current_offset = offset_after;
                }
                next_block = block.next;
            }

            if !recompile {
                break;
            }

            instructions.clear();
            locations.clear();
            linetable_locations.clear();
        }

        // Generate linetable from linetable_locations (supports line 0 for RESUME)
        let linetable = generate_linetable(
            &linetable_locations,
            first_line_number.get() as i32,
            opts.debug_ranges,
        );

        // Generate exception table before moving source_path
        let exceptiontable = generate_exception_table(&blocks, &block_to_index);

        // Build localspluskinds with cell-local merging
        let nlocals = varname_cache.len();
        let ncells = cellvar_cache.len();
        let nfrees = freevar_cache.len();
        let numdropped = cellvar_cache
            .iter()
            .filter(|cv| varname_cache.contains(cv.as_str()))
            .count();
        let nlocalsplus = nlocals + ncells - numdropped + nfrees;
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
            if hidden && let Some(idx) = varname_cache.get_index_of(name.as_str()) {
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
            locations: locations.into_boxed_slice(),
            constants: constants.into_iter().collect(),
            names: name_cache.into_iter().collect(),
            varnames: varname_cache.into_iter().collect(),
            cellvars: cellvar_cache.into_iter().collect(),
            freevars: freevar_cache.into_iter().collect(),
            localspluskinds: localspluskinds.into_boxed_slice(),
            linetable,
            exceptiontable,
        })
    }

    fn dce(&mut self) {
        // Truncate instructions after terminal instructions within each block
        for block in &mut self.blocks {
            let mut last_instr = None;
            for (i, ins) in block.instructions.iter().enumerate() {
                if ins.instr.is_scope_exit() || ins.instr.is_unconditional_jump() {
                    last_instr = Some(i);
                    break;
                }
            }
            if let Some(i) = last_instr {
                block.instructions.truncate(i + 1);
            }
        }
    }

    /// Clear blocks that are unreachable (not entry, not a jump target,
    /// and only reachable via fall-through from a terminal block).
    fn eliminate_unreachable_blocks(&mut self) {
        let mut reachable = vec![false; self.blocks.len()];
        reachable[0] = true;

        // Fixpoint: only mark targets of already-reachable blocks
        let mut changed = true;
        while changed {
            changed = false;
            for i in 0..self.blocks.len() {
                if !reachable[i] {
                    continue;
                }
                // Mark jump targets and exception handlers
                for ins in &self.blocks[i].instructions {
                    if ins.target != BlockIdx::NULL && !reachable[ins.target.idx()] {
                        reachable[ins.target.idx()] = true;
                        changed = true;
                    }
                    if let Some(eh) = &ins.except_handler
                        && !reachable[eh.handler_block.idx()]
                    {
                        reachable[eh.handler_block.idx()] = true;
                        changed = true;
                    }
                }
                // Mark fall-through
                let next = self.blocks[i].next;
                if next != BlockIdx::NULL
                    && !reachable[next.idx()]
                    && !self.blocks[i].instructions.last().is_some_and(|ins| {
                        ins.instr.is_scope_exit() || ins.instr.is_unconditional_jump()
                    })
                {
                    reachable[next.idx()] = true;
                    changed = true;
                }
            }
        }

        for (i, block) in self.blocks.iter_mut().enumerate() {
            if !reachable[i] {
                block.instructions.clear();
            }
        }
    }

    /// Fold LOAD_CONST/LOAD_SMALL_INT + UNARY_NEGATIVE → LOAD_CONST (negative value)
    fn fold_unary_negative(&mut self) {
        for block in &mut self.blocks {
            let mut i = 0;
            while i + 1 < block.instructions.len() {
                let next = &block.instructions[i + 1];
                let Some(Instruction::UnaryNegative) = next.instr.real() else {
                    i += 1;
                    continue;
                };
                let curr = &block.instructions[i];
                let value = match curr.instr.real() {
                    Some(Instruction::LoadConst { .. }) => {
                        let idx = u32::from(curr.arg) as usize;
                        match self.metadata.consts.get_index(idx) {
                            Some(ConstantData::Integer { value }) => {
                                Some(ConstantData::Integer { value: -value })
                            }
                            Some(ConstantData::Float { value }) => {
                                Some(ConstantData::Float { value: -value })
                            }
                            _ => None,
                        }
                    }
                    Some(Instruction::LoadSmallInt { .. }) => {
                        let v = u32::from(curr.arg) as i32;
                        Some(ConstantData::Integer {
                            value: BigInt::from(-v),
                        })
                    }
                    _ => None,
                };
                if let Some(neg_const) = value {
                    let (const_idx, _) = self.metadata.consts.insert_full(neg_const);
                    // Replace LOAD_CONST/LOAD_SMALL_INT with new LOAD_CONST
                    let load_location = block.instructions[i].location;
                    block.instructions[i].instr = Instruction::LoadConst {
                        consti: Arg::marker(),
                    }
                    .into();
                    block.instructions[i].arg = OpArg::new(const_idx as u32);
                    // Replace UNARY_NEGATIVE with NOP, inheriting the LOAD_CONST
                    // location so that remove_nops can clean it up
                    set_to_nop(&mut block.instructions[i + 1]);
                    block.instructions[i + 1].location = load_location;
                    block.instructions[i + 1].end_location = block.instructions[i].end_location;
                    // Skip the NOP, don't re-check
                    i += 2;
                } else {
                    i += 1;
                }
            }
        }
    }

    /// Constant folding: fold LOAD_CONST/LOAD_SMALL_INT + LOAD_CONST/LOAD_SMALL_INT + BINARY_OP
    /// into a single LOAD_CONST when the result is computable at compile time.
    /// = fold_binops_on_constants in CPython flowgraph.c
    fn fold_binop_constants(&mut self) {
        use oparg::BinaryOperator as BinOp;

        for block in &mut self.blocks {
            let mut i = 0;
            while i + 2 < block.instructions.len() {
                // Check pattern: LOAD_CONST/LOAD_SMALL_INT, LOAD_CONST/LOAD_SMALL_INT, BINARY_OP
                let Some(Instruction::BinaryOp { .. }) = block.instructions[i + 2].instr.real()
                else {
                    i += 1;
                    continue;
                };

                let op_raw = u32::from(block.instructions[i + 2].arg);
                let Ok(op) = BinOp::try_from(op_raw) else {
                    i += 1;
                    continue;
                };

                let left = Self::get_const_value_from(&self.metadata, &block.instructions[i]);
                let right = Self::get_const_value_from(&self.metadata, &block.instructions[i + 1]);

                let (Some(left_val), Some(right_val)) = (left, right) else {
                    i += 1;
                    continue;
                };

                let result = Self::eval_binop(&left_val, &right_val, op);

                if let Some(result_const) = result {
                    // Check result size limit (CPython limits to 4096 bytes)
                    if Self::const_too_big(&result_const) {
                        i += 1;
                        continue;
                    }
                    let (const_idx, _) = self.metadata.consts.insert_full(result_const);
                    // Replace first instruction with LOAD_CONST result
                    block.instructions[i].instr = Instruction::LoadConst {
                        consti: Arg::marker(),
                    }
                    .into();
                    block.instructions[i].arg = OpArg::new(const_idx as u32);
                    // NOP out the second and third instructions
                    let loc = block.instructions[i].location;
                    let end_loc = block.instructions[i].end_location;
                    set_to_nop(&mut block.instructions[i + 1]);
                    block.instructions[i + 1].location = loc;
                    block.instructions[i + 1].end_location = end_loc;
                    set_to_nop(&mut block.instructions[i + 2]);
                    block.instructions[i + 2].location = loc;
                    block.instructions[i + 2].end_location = end_loc;
                    // Don't advance - check if the result can be folded again
                    // (e.g., 2 ** 31 - 1)
                    i = i.saturating_sub(1); // re-check with previous instruction
                } else {
                    i += 1;
                }
            }
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

    fn eval_binop(
        left: &ConstantData,
        right: &ConstantData,
        op: oparg::BinaryOperator,
    ) -> Option<ConstantData> {
        use oparg::BinaryOperator as BinOp;
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
                        // Float floor division uses runtime semantics; skip folding
                        return None;
                    }
                    BinOp::Remainder => {
                        // Float modulo uses fmod() at runtime; Rust arithmetic differs
                        return None;
                    }
                    BinOp::Power => l.powf(*r),
                    _ => return None,
                };
                if !result.is_finite() {
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
            // String concatenation and repetition
            (ConstantData::Str { value: l }, ConstantData::Str { value: r })
                if matches!(op, BinOp::Add) =>
            {
                let mut result = l.to_string();
                result.push_str(&r.to_string());
                Some(ConstantData::Str {
                    value: result.into(),
                })
            }
            (ConstantData::Str { value: s }, ConstantData::Integer { value: n })
                if matches!(op, BinOp::Multiply) =>
            {
                let n: usize = n.try_into().ok()?;
                if n > 4096 {
                    return None;
                }
                let result = s.to_string().repeat(n);
                Some(ConstantData::Str {
                    value: result.into(),
                })
            }
            _ => None,
        }
    }

    fn const_too_big(c: &ConstantData) -> bool {
        match c {
            ConstantData::Integer { value } => value.bits() > 4096 * 8,
            ConstantData::Str { value } => value.len() > 4096,
            _ => false,
        }
    }

    /// Constant folding: fold LOAD_CONST/LOAD_SMALL_INT + BUILD_TUPLE into LOAD_CONST tuple
    /// fold_tuple_of_constants
    fn fold_tuple_constants(&mut self) {
        for block in &mut self.blocks {
            let mut i = 0;
            while i < block.instructions.len() {
                let instr = &block.instructions[i];
                // Look for BUILD_TUPLE
                let Some(Instruction::BuildTuple { .. }) = instr.instr.real() else {
                    i += 1;
                    continue;
                };

                let tuple_size = u32::from(instr.arg) as usize;
                if tuple_size == 0 {
                    // BUILD_TUPLE 0 → LOAD_CONST ()
                    let (const_idx, _) = self.metadata.consts.insert_full(ConstantData::Tuple {
                        elements: Vec::new(),
                    });
                    block.instructions[i].instr = Instruction::LoadConst {
                        consti: Arg::marker(),
                    }
                    .into();
                    block.instructions[i].arg = OpArg::new(const_idx as u32);
                    i += 1;
                    continue;
                }
                if i < tuple_size {
                    i += 1;
                    continue;
                }

                // Check if all preceding instructions are constant-loading
                let start_idx = i - tuple_size;
                let mut elements = Vec::with_capacity(tuple_size);
                let mut all_const = true;

                for j in start_idx..i {
                    let load_instr = &block.instructions[j];
                    match load_instr.instr.real() {
                        Some(Instruction::LoadConst { .. }) => {
                            let const_idx = u32::from(load_instr.arg) as usize;
                            if let Some(constant) =
                                self.metadata.consts.get_index(const_idx).cloned()
                            {
                                elements.push(constant);
                            } else {
                                all_const = false;
                                break;
                            }
                        }
                        Some(Instruction::LoadSmallInt { .. }) => {
                            // arg is the i32 value stored as u32 (two's complement)
                            let value = u32::from(load_instr.arg) as i32;
                            elements.push(ConstantData::Integer {
                                value: BigInt::from(value),
                            });
                        }
                        _ => {
                            all_const = false;
                            break;
                        }
                    }
                }

                if !all_const {
                    i += 1;
                    continue;
                }

                // Note: The first small int is added to co_consts during compilation
                // (in compile_default_arguments).
                // We don't need to add it here again.

                // Create tuple constant and add to consts
                let tuple_const = ConstantData::Tuple { elements };
                let (const_idx, _) = self.metadata.consts.insert_full(tuple_const);

                // Replace preceding LOAD instructions with NOP at the
                // BUILD_TUPLE location so remove_nops() can eliminate them.
                let folded_loc = block.instructions[i].location;
                for j in start_idx..i {
                    set_to_nop(&mut block.instructions[j]);
                    block.instructions[j].location = folded_loc;
                }

                // Replace BUILD_TUPLE with LOAD_CONST
                block.instructions[i].instr = Instruction::LoadConst {
                    consti: Arg::marker(),
                }
                .into();
                block.instructions[i].arg = OpArg::new(const_idx as u32);

                i += 1;
            }
        }
    }

    /// Fold constant list literals: LOAD_CONST* + BUILD_LIST N →
    /// BUILD_LIST 0 + LOAD_CONST (tuple) + LIST_EXTEND 1
    fn fold_list_constants(&mut self) {
        for block in &mut self.blocks {
            let mut i = 0;
            while i < block.instructions.len() {
                let instr = &block.instructions[i];
                let Some(Instruction::BuildList { .. }) = instr.instr.real() else {
                    i += 1;
                    continue;
                };

                let list_size = u32::from(instr.arg) as usize;
                if list_size == 0 || i < list_size {
                    i += 1;
                    continue;
                }

                let start_idx = i - list_size;
                let mut elements = Vec::with_capacity(list_size);
                let mut all_const = true;

                for j in start_idx..i {
                    let load_instr = &block.instructions[j];
                    match load_instr.instr.real() {
                        Some(Instruction::LoadConst { .. }) => {
                            let const_idx = u32::from(load_instr.arg) as usize;
                            if let Some(constant) =
                                self.metadata.consts.get_index(const_idx).cloned()
                            {
                                elements.push(constant);
                            } else {
                                all_const = false;
                                break;
                            }
                        }
                        Some(Instruction::LoadSmallInt { .. }) => {
                            let value = u32::from(load_instr.arg) as i32;
                            elements.push(ConstantData::Integer {
                                value: BigInt::from(value),
                            });
                        }
                        _ => {
                            all_const = false;
                            break;
                        }
                    }
                }

                if !all_const || list_size < MIN_CONST_SEQUENCE_SIZE {
                    i += 1;
                    continue;
                }

                let tuple_const = ConstantData::Tuple { elements };
                let (const_idx, _) = self.metadata.consts.insert_full(tuple_const);

                let folded_loc = block.instructions[i].location;
                let end_loc = block.instructions[i].end_location;
                let eh = block.instructions[i].except_handler;

                // slot[start_idx] → BUILD_LIST 0
                block.instructions[start_idx].instr = Instruction::BuildList {
                    count: Arg::marker(),
                }
                .into();
                block.instructions[start_idx].arg = OpArg::new(0);
                block.instructions[start_idx].location = folded_loc;
                block.instructions[start_idx].end_location = end_loc;
                block.instructions[start_idx].except_handler = eh;

                // slot[start_idx+1] → LOAD_CONST (tuple)
                block.instructions[start_idx + 1].instr = Instruction::LoadConst {
                    consti: Arg::marker(),
                }
                .into();
                block.instructions[start_idx + 1].arg = OpArg::new(const_idx as u32);
                block.instructions[start_idx + 1].location = folded_loc;
                block.instructions[start_idx + 1].end_location = end_loc;
                block.instructions[start_idx + 1].except_handler = eh;

                // NOP the rest
                for j in (start_idx + 2)..i {
                    set_to_nop(&mut block.instructions[j]);
                    block.instructions[j].location = folded_loc;
                }

                // slot[i] (was BUILD_LIST) → LIST_EXTEND 1
                block.instructions[i].instr = Instruction::ListExtend { i: Arg::marker() }.into();
                block.instructions[i].arg = OpArg::new(1);

                i += 1;
            }
        }
    }

    /// Convert constant list construction before GET_ITER to just LOAD_CONST tuple.
    /// BUILD_LIST 0 + LOAD_CONST (tuple) + LIST_EXTEND 1 + GET_ITER
    /// → LOAD_CONST (tuple) + GET_ITER
    fn fold_const_iterable_for_iter(&mut self) {
        for block in &mut self.blocks {
            let mut i = 0;
            while i + 1 < block.instructions.len() {
                let is_build = matches!(
                    block.instructions[i].instr.real(),
                    Some(Instruction::BuildList { .. })
                ) && u32::from(block.instructions[i].arg) == 0;

                let is_const = matches!(
                    block
                        .instructions
                        .get(i + 1)
                        .and_then(|instr| instr.instr.real()),
                    Some(Instruction::LoadConst { .. })
                );

                let is_extend = matches!(
                    block
                        .instructions
                        .get(i + 2)
                        .and_then(|instr| instr.instr.real()),
                    Some(Instruction::ListExtend { .. })
                ) && block
                    .instructions
                    .get(i + 2)
                    .is_some_and(|instr| u32::from(instr.arg) == 1);

                let is_iter = matches!(
                    block
                        .instructions
                        .get(i + 3)
                        .and_then(|instr| instr.instr.real()),
                    Some(Instruction::GetIter)
                );

                if is_build && is_const && is_extend && is_iter {
                    // Replace: BUILD_X 0 → NOP, keep LOAD_CONST, LIST_EXTEND → NOP
                    let loc = block.instructions[i].location;
                    set_to_nop(&mut block.instructions[i]);
                    block.instructions[i].location = loc;
                    set_to_nop(&mut block.instructions[i + 2]);
                    block.instructions[i + 2].location = loc;
                    i += 4;
                } else if matches!(
                    block.instructions[i].instr.real(),
                    Some(Instruction::BuildList { .. })
                ) && matches!(
                    block.instructions[i + 1].instr.real(),
                    Some(Instruction::GetIter)
                ) {
                    let seq_size = u32::from(block.instructions[i].arg) as usize;

                    if seq_size != 0 && i >= seq_size {
                        let start_idx = i - seq_size;
                        let mut elements = Vec::with_capacity(seq_size);
                        let mut all_const = true;

                        for j in start_idx..i {
                            match Self::get_const_value_from(&self.metadata, &block.instructions[j])
                            {
                                Some(constant) => elements.push(constant),
                                None => {
                                    all_const = false;
                                    break;
                                }
                            }
                        }

                        if all_const {
                            let const_data = ConstantData::Tuple { elements };
                            let (const_idx, _) = self.metadata.consts.insert_full(const_data);
                            let folded_loc = block.instructions[i].location;

                            for j in start_idx..i {
                                set_to_nop(&mut block.instructions[j]);
                                block.instructions[j].location = folded_loc;
                            }

                            block.instructions[i].instr = Instruction::LoadConst {
                                consti: Arg::marker(),
                            }
                            .into();
                            block.instructions[i].arg = OpArg::new(const_idx as u32);
                            i += 2;
                            continue;
                        }
                    }

                    block.instructions[i].instr = Instruction::BuildTuple {
                        count: Arg::marker(),
                    }
                    .into();
                    i += 2;
                } else {
                    i += 1;
                }
            }
        }
    }

    /// Fold constant set literals: LOAD_CONST* + BUILD_SET N →
    /// BUILD_SET 0 + LOAD_CONST (frozenset-as-tuple) + SET_UPDATE 1
    fn fold_set_constants(&mut self) {
        for block in &mut self.blocks {
            let mut i = 0;
            while i < block.instructions.len() {
                let instr = &block.instructions[i];
                let Some(Instruction::BuildSet { .. }) = instr.instr.real() else {
                    i += 1;
                    continue;
                };

                let set_size = u32::from(instr.arg) as usize;
                if set_size < 3 || i < set_size {
                    i += 1;
                    continue;
                }

                let start_idx = i - set_size;
                let mut elements = Vec::with_capacity(set_size);
                let mut all_const = true;

                for j in start_idx..i {
                    let load_instr = &block.instructions[j];
                    match load_instr.instr.real() {
                        Some(Instruction::LoadConst { .. }) => {
                            let const_idx = u32::from(load_instr.arg) as usize;
                            if let Some(constant) =
                                self.metadata.consts.get_index(const_idx).cloned()
                            {
                                elements.push(constant);
                            } else {
                                all_const = false;
                                break;
                            }
                        }
                        Some(Instruction::LoadSmallInt { .. }) => {
                            let value = u32::from(load_instr.arg) as i32;
                            elements.push(ConstantData::Integer {
                                value: BigInt::from(value),
                            });
                        }
                        _ => {
                            all_const = false;
                            break;
                        }
                    }
                }

                if !all_const {
                    i += 1;
                    continue;
                }

                // Use FrozenSet constant (stored as Tuple for now)
                let const_data = ConstantData::Tuple { elements };
                let (const_idx, _) = self.metadata.consts.insert_full(const_data);

                let folded_loc = block.instructions[i].location;
                let end_loc = block.instructions[i].end_location;
                let eh = block.instructions[i].except_handler;

                block.instructions[start_idx].instr = Instruction::BuildSet {
                    count: Arg::marker(),
                }
                .into();
                block.instructions[start_idx].arg = OpArg::new(0);
                block.instructions[start_idx].location = folded_loc;
                block.instructions[start_idx].end_location = end_loc;
                block.instructions[start_idx].except_handler = eh;

                block.instructions[start_idx + 1].instr = Instruction::LoadConst {
                    consti: Arg::marker(),
                }
                .into();
                block.instructions[start_idx + 1].arg = OpArg::new(const_idx as u32);
                block.instructions[start_idx + 1].location = folded_loc;
                block.instructions[start_idx + 1].end_location = end_loc;
                block.instructions[start_idx + 1].except_handler = eh;

                for j in (start_idx + 2)..i {
                    set_to_nop(&mut block.instructions[j]);
                    block.instructions[j].location = folded_loc;
                }

                block.instructions[i].instr = Instruction::SetUpdate { i: Arg::marker() }.into();
                block.instructions[i].arg = OpArg::new(1);

                i += 1;
            }
        }
    }

    /// BUILD_TUPLE n + UNPACK_SEQUENCE n optimization.
    ///
    /// Ported from CPython flowgraph.c optimize_basic_block:
    /// - n == 1: both become NOP (identity operation)
    /// - n == 2 or 3: BUILD_TUPLE → NOP, UNPACK_SEQUENCE → SWAP
    fn optimize_build_tuple_unpack(&mut self) {
        for block in &mut self.blocks {
            let instrs = &mut block.instructions;
            let len = instrs.len();
            for i in 0..len.saturating_sub(1) {
                let Some(Instruction::BuildTuple { .. }) = instrs[i].instr.real() else {
                    continue;
                };
                let n = u32::from(instrs[i].arg);
                let Some(Instruction::UnpackSequence { .. }) = instrs[i + 1].instr.real() else {
                    continue;
                };
                if u32::from(instrs[i + 1].arg) != n {
                    continue;
                }
                match n {
                    1 => {
                        instrs[i].instr = AnyInstruction::Real(Instruction::Nop);
                        instrs[i].arg = OpArg::new(0);
                        instrs[i + 1].instr = AnyInstruction::Real(Instruction::Nop);
                        instrs[i + 1].arg = OpArg::new(0);
                    }
                    2 | 3 => {
                        instrs[i].instr = AnyInstruction::Real(Instruction::Nop);
                        instrs[i].arg = OpArg::new(0);
                        instrs[i + 1].instr =
                            AnyInstruction::Real(Instruction::Swap { i: Arg::marker() });
                        instrs[i + 1].arg = OpArg::new(n);
                    }
                    _ => {}
                }
            }
        }
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
    fn apply_static_swaps(&mut self) {
        /// Instruction classes that are safe to reorder around SWAP.
        fn is_swappable(instr: &AnyInstruction) -> bool {
            matches!(
                instr,
                AnyInstruction::Real(Instruction::StoreFast { .. } | Instruction::PopTop)
            )
        }

        /// Variable index that a STORE_FAST writes to, or None.
        fn stores_to(info: &InstructionInfo) -> Option<u32> {
            match info.instr {
                AnyInstruction::Real(Instruction::StoreFast { .. }) => Some(u32::from(info.arg)),
                _ => None,
            }
        }

        /// Next swappable index after `i` in `instrs`, skipping NOPs.
        /// Returns None if a non-NOP non-swappable instruction blocks, or
        /// if `lineno >= 0` and a different lineno is encountered.
        fn next_swappable(instrs: &[InstructionInfo], mut i: usize, lineno: i32) -> Option<usize> {
            loop {
                i += 1;
                if i >= instrs.len() {
                    return None;
                }
                let info = &instrs[i];
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

        for block in &mut self.blocks {
            let instrs = &mut block.instructions;
            let len = instrs.len();
            // Walk forward; for each SWAP attempt elimination.
            let mut i = 0;
            while i < len {
                let swap_arg = match instrs[i].instr {
                    AnyInstruction::Real(Instruction::Swap { .. }) => u32::from(instrs[i].arg),
                    _ => {
                        i += 1;
                        continue;
                    }
                };
                // SWAP oparg < 2 is a no-op; the compiler should not emit
                // these, but be defensive.
                if swap_arg < 2 {
                    i += 1;
                    continue;
                }
                // Find first swappable after SWAP (lineno = -1 initially).
                let Some(j) = next_swappable(instrs, i, -1) else {
                    i += 1;
                    continue;
                };
                let lineno = instrs[j].location.line.get() as i32;
                // Walk (swap_arg - 1) more swappable instructions, with
                // lineno constraint.
                let mut k = j;
                let mut ok = true;
                for _ in 1..swap_arg {
                    match next_swappable(instrs, k, lineno) {
                        Some(next) => k = next,
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                if !ok {
                    i += 1;
                    continue;
                }
                // Conflict check: if either j or k is a STORE_FAST, no
                // intervening store may target the same variable, and
                // they must not target the same variable themselves.
                let store_j = stores_to(&instrs[j]);
                let store_k = stores_to(&instrs[k]);
                if store_j.is_some() || store_k.is_some() {
                    if store_j == store_k {
                        i += 1;
                        continue;
                    }
                    let conflict = instrs[(j + 1)..k].iter().any(|info| {
                        if let Some(store_idx) = stores_to(info) {
                            Some(store_idx) == store_j || Some(store_idx) == store_k
                        } else {
                            false
                        }
                    });
                    if conflict {
                        i += 1;
                        continue;
                    }
                }
                // Safe to reorder. SWAP -> NOP, swap j and k.
                instrs[i].instr = AnyInstruction::Real(Instruction::Nop);
                instrs[i].arg = OpArg::new(0);
                instrs.swap(j, k);
                i += 1;
            }
        }
    }

    /// Eliminate dead stores in STORE_FAST sequences (apply_static_swaps).
    ///
    /// In sequences of consecutive STORE_FAST instructions (from tuple unpacking),
    /// if the same variable is stored to more than once, only the first store
    /// (which gets TOS — the rightmost value) matters. Later stores to the
    /// same variable are dead and replaced with POP_TOP.
    /// Simplified apply_static_swaps (CPython flowgraph.c):
    /// In STORE_FAST sequences that follow UNPACK_SEQUENCE / UNPACK_EX,
    /// replace duplicate stores to the same variable with POP_TOP.
    /// UNPACK pushes values so stores execute left-to-right; the LAST
    /// store to a variable carries the final value, earlier ones are dead.
    fn eliminate_dead_stores(&mut self) {
        for block in &mut self.blocks {
            let instrs = &mut block.instructions;
            let len = instrs.len();
            let mut i = 0;
            while i < len {
                // Look for UNPACK_SEQUENCE or UNPACK_EX
                let is_unpack = matches!(
                    instrs[i].instr,
                    AnyInstruction::Real(
                        Instruction::UnpackSequence { .. } | Instruction::UnpackEx { .. }
                    )
                );
                if !is_unpack {
                    i += 1;
                    continue;
                }
                // Scan the run of STORE_FAST right after the unpack
                let run_start = i + 1;
                let mut run_end = run_start;
                while run_end < len
                    && matches!(
                        instrs[run_end].instr,
                        AnyInstruction::Real(Instruction::StoreFast { .. })
                    )
                {
                    run_end += 1;
                }
                if run_end - run_start >= 2 {
                    // Pass 1: find the LAST occurrence of each variable
                    let mut last_occurrence = std::collections::HashMap::new();
                    for (j, instr) in instrs[run_start..run_end].iter().enumerate() {
                        last_occurrence.insert(u32::from(instr.arg), j);
                    }
                    // Pass 2: non-last stores to the same variable are dead
                    for (j, instr) in instrs[run_start..run_end].iter_mut().enumerate() {
                        let idx = u32::from(instr.arg);
                        if last_occurrence[&idx] != j {
                            instr.instr = AnyInstruction::Real(Instruction::PopTop);
                            instr.arg = OpArg::new(0);
                        }
                    }
                }
                i = run_end.max(i + 1);
            }
        }
    }

    /// Peephole optimization: combine consecutive instructions into super-instructions
    fn peephole_optimize(&mut self) {
        for block in &mut self.blocks {
            let mut i = 0;
            while i + 1 < block.instructions.len() {
                let combined = {
                    let curr = &block.instructions[i];
                    let next = &block.instructions[i + 1];

                    // Only combine if both are real instructions (not pseudo)
                    let (Some(curr_instr), Some(next_instr)) =
                        (curr.instr.real(), next.instr.real())
                    else {
                        i += 1;
                        continue;
                    };

                    match (curr_instr, next_instr) {
                        // LoadFast + LoadFast -> LoadFastLoadFast (if both indices < 16)
                        (Instruction::LoadFast { .. }, Instruction::LoadFast { .. }) => {
                            let line1 = curr.location.line.get() as i32;
                            let line2 = next.location.line.get() as i32;
                            if line1 > 0 && line2 > 0 && line1 != line2 {
                                None
                            } else {
                                let idx1 = u32::from(curr.arg);
                                let idx2 = u32::from(next.arg);
                                if idx1 < 16 && idx2 < 16 {
                                    let packed = (idx1 << 4) | idx2;
                                    Some((
                                        Instruction::LoadFastLoadFast {
                                            var_nums: Arg::marker(),
                                        },
                                        OpArg::new(packed),
                                    ))
                                } else {
                                    None
                                }
                            }
                        }
                        // StoreFast + StoreFast -> StoreFastStoreFast (if both indices < 16)
                        // Dead store elimination: if both store to the same variable,
                        // the first store is dead. Replace it with POP_TOP (like
                        // apply_static_swaps in CPython's flowgraph.c).
                        (Instruction::StoreFast { .. }, Instruction::StoreFast { .. }) => {
                            let line1 = curr.location.line.get() as i32;
                            let line2 = next.location.line.get() as i32;
                            if line1 > 0 && line2 > 0 && line1 != line2 {
                                None
                            } else {
                                let idx1 = u32::from(curr.arg);
                                let idx2 = u32::from(next.arg);
                                if idx1 < 16 && idx2 < 16 {
                                    let packed = (idx1 << 4) | idx2;
                                    Some((
                                        Instruction::StoreFastStoreFast {
                                            var_nums: Arg::marker(),
                                        },
                                        OpArg::new(packed),
                                    ))
                                } else {
                                    None
                                }
                            }
                        }
                        // Note: StoreFast + LoadFast → StoreFastLoadFast is done in a
                        // separate pass AFTER optimize_load_fast_borrow, because CPython
                        // only combines STORE_FAST + LOAD_FAST (not LOAD_FAST_BORROW).
                        (Instruction::LoadConst { consti }, Instruction::ToBool) => {
                            let consti = consti.get(curr.arg);
                            let constant = &self.metadata.consts[consti.as_usize()];
                            if let ConstantData::Boolean { .. } = constant {
                                Some((curr_instr, OpArg::from(consti.as_u32())))
                            } else {
                                None
                            }
                        }
                        (Instruction::LoadConst { consti }, Instruction::UnaryNot) => {
                            let constant = &self.metadata.consts[consti.get(curr.arg).as_usize()];
                            match constant {
                                ConstantData::Boolean { value } => {
                                    let (const_idx, _) = self
                                        .metadata
                                        .consts
                                        .insert_full(ConstantData::Boolean { value: !value });
                                    Some((
                                        (Instruction::LoadConst {
                                            consti: Arg::marker(),
                                        }),
                                        OpArg::new(const_idx as u32),
                                    ))
                                }
                                _ => None,
                            }
                        }
                        _ => None,
                    }
                };

                if let Some((new_instr, new_arg)) = combined {
                    // Combine: keep first instruction's location, replace with combined instruction
                    block.instructions[i].instr = new_instr.into();
                    block.instructions[i].arg = new_arg;
                    // Remove the second instruction
                    block.instructions.remove(i + 1);
                    // Don't increment i - check if we can combine again with the next instruction
                } else {
                    i += 1;
                }
            }
        }
    }

    /// LOAD_GLOBAL <even> + PUSH_NULL -> LOAD_GLOBAL <odd>, NOP
    fn optimize_load_global_push_null(&mut self) {
        for block in &mut self.blocks {
            let mut i = 0;
            while i + 1 < block.instructions.len() {
                let curr = &block.instructions[i];
                let next = &block.instructions[i + 1];

                let (Some(Instruction::LoadGlobal { .. }), Some(Instruction::PushNull)) =
                    (curr.instr.real(), next.instr.real())
                else {
                    i += 1;
                    continue;
                };

                let oparg = u32::from(block.instructions[i].arg);
                if (oparg & 1) != 0 {
                    i += 1;
                    continue;
                }

                block.instructions[i].arg = OpArg::new(oparg | 1);
                block.instructions.remove(i + 1);
            }
        }
    }

    /// Convert LOAD_CONST for small integers to LOAD_SMALL_INT
    /// maybe_instr_make_load_smallint
    fn convert_to_load_small_int(&mut self) {
        for block in &mut self.blocks {
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
                    instr.instr = Instruction::LoadSmallInt { i: Arg::marker() }.into();
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

        for block in &self.blocks {
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
        for block in &mut self.blocks {
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

    /// Remove NOP instructions from all blocks, but keep NOPs that introduce
    /// a new source line (they serve as line markers for monitoring LINE events).
    fn remove_nops(&mut self) {
        for block in &mut self.blocks {
            let mut prev_line = None;
            block.instructions.retain(|ins| {
                if matches!(ins.instr.real(), Some(Instruction::Nop)) {
                    let line = ins.location.line;
                    if prev_line == Some(line) {
                        return false;
                    }
                }
                prev_line = Some(ins.location.line);
                true
            });
        }
    }

    /// Optimize LOAD_FAST to LOAD_FAST_BORROW where safe.
    ///
    /// insert_superinstructions (flowgraph.c): Combine STORE_FAST + LOAD_FAST →
    /// STORE_FAST_LOAD_FAST. Currently disabled pending VM stack null investigation.
    #[allow(dead_code)]
    fn combine_store_fast_load_fast(&mut self) {
        for block in &mut self.blocks {
            let mut i = 0;
            while i + 1 < block.instructions.len() {
                let curr = &block.instructions[i];
                let next = &block.instructions[i + 1];
                let (Some(Instruction::StoreFast { .. }), Some(Instruction::LoadFast { .. })) =
                    (curr.instr.real(), next.instr.real())
                else {
                    i += 1;
                    continue;
                };
                // Skip if instructions are on different lines (matching make_super_instruction)
                let line1 = curr.location.line;
                let line2 = next.location.line;
                if line1 != line2 {
                    i += 1;
                    continue;
                }
                let idx1 = u32::from(curr.arg);
                let idx2 = u32::from(next.arg);
                if idx1 < 16 && idx2 < 16 {
                    let packed = (idx1 << 4) | idx2;
                    block.instructions[i].instr = Instruction::StoreFastLoadFast {
                        var_nums: Arg::marker(),
                    }
                    .into();
                    block.instructions[i].arg = OpArg::new(packed);
                    // Replace second instruction with NOP (CPython: INSTR_SET_OP0(inst2, NOP))
                    set_to_nop(&mut block.instructions[i + 1]);
                    i += 2; // skip the NOP
                } else {
                    i += 1;
                }
            }
        }
    }

    fn optimize_load_fast_borrow(&mut self) {
        // NOT_LOCAL marker: instruction didn't come from a LOAD_FAST
        const NOT_LOCAL: usize = usize::MAX;

        for block in &mut self.blocks {
            if block.instructions.is_empty() {
                continue;
            }

            // Track which instructions' outputs are still on stack at block end
            // For each instruction, we track if its pushed value(s) are unconsumed
            let mut unconsumed = vec![false; block.instructions.len()];

            // Simulate stack: each entry is the instruction index that pushed it
            // (or NOT_LOCAL if not from LOAD_FAST/LOAD_FAST_LOAD_FAST).
            //
            // CPython (flowgraph.c optimize_load_fast) pre-fills the stack with
            // dummy refs for values inherited from predecessor blocks. We take
            // the simpler approach of aborting the optimisation for the whole
            // block on stack underflow.
            let mut stack: Vec<usize> = Vec::new();
            let mut underflow = false;

            for (i, info) in block.instructions.iter().enumerate() {
                let Some(instr) = info.instr.real() else {
                    continue;
                };

                let stack_effect_info = instr.stack_effect_info(info.arg.into());
                let (pushes, pops) = (stack_effect_info.pushed(), stack_effect_info.popped());

                // Pop values from stack
                for _ in 0..pops {
                    if stack.pop().is_none() {
                        // Stack underflow — block receives values from a predecessor.
                        // Abort optimisation for the entire block.
                        underflow = true;
                        break;
                    }
                }
                if underflow {
                    break;
                }

                // Push values to stack with source instruction index
                let source = match instr {
                    Instruction::LoadFast { .. } | Instruction::LoadFastLoadFast { .. } => i,
                    _ => NOT_LOCAL,
                };
                for _ in 0..pushes {
                    stack.push(source);
                }
            }

            if underflow {
                continue;
            }

            // Mark instructions whose values remain on stack at block end
            for &src in &stack {
                if src != NOT_LOCAL {
                    unconsumed[src] = true;
                }
            }

            // Convert LOAD_FAST to LOAD_FAST_BORROW where value is fully consumed
            for (i, info) in block.instructions.iter_mut().enumerate() {
                if unconsumed[i] {
                    continue;
                }
                let Some(instr) = info.instr.real() else {
                    continue;
                };
                match instr {
                    Instruction::LoadFast { .. } => {
                        info.instr = Instruction::LoadFastBorrow {
                            var_num: Arg::marker(),
                        }
                        .into();
                    }
                    Instruction::LoadFastLoadFast { .. } => {
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

    fn add_checks_for_loads_of_uninitialized_variables(&mut self) {
        let nlocals = self.metadata.varnames.len();
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

        let mut in_masks: Vec<Option<Vec<bool>>> = vec![None; self.blocks.len()];
        let mut start_mask = vec![false; nlocals];
        for slot in start_mask.iter_mut().skip(nparams) {
            *slot = true;
        }
        in_masks[0] = Some(start_mask);

        let mut worklist = vec![BlockIdx(0)];
        while let Some(block_idx) = worklist.pop() {
            let idx = block_idx.idx();
            let Some(mut unsafe_mask) = in_masks[idx].clone() else {
                continue;
            };

            let old_instructions = self.blocks[idx].instructions.clone();
            let mut new_instructions = Vec::with_capacity(old_instructions.len());
            let mut changed = false;

            for info in old_instructions {
                let mut info = info;
                if let Some(eh) = info.except_handler {
                    let target = next_nonempty_block(&self.blocks, eh.handler_block);
                    if target != BlockIdx::NULL
                        && merge_unsafe_mask(&mut in_masks[target.idx()], &unsafe_mask)
                    {
                        worklist.push(target);
                    }
                }
                match info.instr.real() {
                    Some(Instruction::DeleteFast { var_num }) => {
                        let var_idx = usize::from(var_num.get(info.arg));
                        if var_idx < nlocals {
                            unsafe_mask[var_idx] = true;
                        }
                        new_instructions.push(info);
                    }
                    Some(Instruction::LoadFastAndClear { var_num }) => {
                        let var_idx = usize::from(var_num.get(info.arg));
                        if var_idx < nlocals {
                            unsafe_mask[var_idx] = true;
                        }
                        new_instructions.push(info);
                    }
                    Some(Instruction::StoreFast { var_num }) => {
                        let var_idx = usize::from(var_num.get(info.arg));
                        if var_idx < nlocals {
                            unsafe_mask[var_idx] = false;
                        }
                        new_instructions.push(info);
                    }
                    Some(Instruction::StoreFastStoreFast { var_nums }) => {
                        let packed = var_nums.get(info.arg);
                        let (idx1, idx2) = packed.indexes();
                        let idx1 = usize::from(idx1);
                        let idx2 = usize::from(idx2);
                        if idx1 < nlocals {
                            unsafe_mask[idx1] = false;
                        }
                        if idx2 < nlocals {
                            unsafe_mask[idx2] = false;
                        }
                        new_instructions.push(info);
                    }
                    Some(Instruction::LoadFastCheck { var_num }) => {
                        let var_idx = usize::from(var_num.get(info.arg));
                        if var_idx < nlocals {
                            unsafe_mask[var_idx] = false;
                        }
                        new_instructions.push(info);
                    }
                    Some(Instruction::LoadFast { var_num }) => {
                        let var_idx = usize::from(var_num.get(info.arg));
                        if var_idx < nlocals && unsafe_mask[var_idx] {
                            info.instr = Instruction::LoadFastCheck {
                                var_num: Arg::marker(),
                            }
                            .into();
                            changed = true;
                        }
                        if var_idx < nlocals {
                            unsafe_mask[var_idx] = false;
                        }
                        new_instructions.push(info);
                    }
                    Some(Instruction::LoadFastLoadFast { var_nums }) => {
                        let packed = var_nums.get(info.arg);
                        let (idx1, idx2) = packed.indexes();
                        let idx1 = usize::from(idx1);
                        let idx2 = usize::from(idx2);
                        let needs_check_1 = idx1 < nlocals && unsafe_mask[idx1];
                        let needs_check_2 = idx2 < nlocals && unsafe_mask[idx2];
                        if needs_check_1 || needs_check_2 {
                            let mut first = info;
                            first.instr = if needs_check_1 {
                                Instruction::LoadFastCheck {
                                    var_num: Arg::marker(),
                                }
                            } else {
                                Instruction::LoadFast {
                                    var_num: Arg::marker(),
                                }
                            }
                            .into();
                            first.arg = OpArg::new(idx1 as u32);

                            let mut second = info;
                            second.instr = if needs_check_2 {
                                Instruction::LoadFastCheck {
                                    var_num: Arg::marker(),
                                }
                            } else {
                                Instruction::LoadFast {
                                    var_num: Arg::marker(),
                                }
                            }
                            .into();
                            second.arg = OpArg::new(idx2 as u32);

                            new_instructions.push(first);
                            new_instructions.push(second);
                            changed = true;
                        } else {
                            new_instructions.push(info);
                        }
                        if idx1 < nlocals {
                            unsafe_mask[idx1] = false;
                        }
                        if idx2 < nlocals {
                            unsafe_mask[idx2] = false;
                        }
                    }
                    _ => new_instructions.push(info),
                }
            }

            if changed {
                self.blocks[idx].instructions = new_instructions;
            }

            let block = &self.blocks[idx];
            if block_has_fallthrough(block) {
                let next = next_nonempty_block(&self.blocks, block.next);
                if next != BlockIdx::NULL
                    && merge_unsafe_mask(&mut in_masks[next.idx()], &unsafe_mask)
                {
                    worklist.push(next);
                }
            }

            if let Some(last) = block.instructions.last()
                && is_jump_instruction(last)
            {
                let target = next_nonempty_block(&self.blocks, last.target);
                if target != BlockIdx::NULL
                    && merge_unsafe_mask(&mut in_masks[target.idx()], &unsafe_mask)
                {
                    worklist.push(target);
                }
            }
        }
    }

    fn max_stackdepth(&mut self) -> crate::InternalResult<u32> {
        let mut maxdepth = 0u32;
        let mut stack = Vec::with_capacity(self.blocks.len());
        let mut start_depths = vec![u32::MAX; self.blocks.len()];
        stackdepth_push(&mut stack, &mut start_depths, BlockIdx(0), 0);
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
                    let instr_display = instr.display(display_arg, self);
                    eprint!("{instr_display}: {depth} {effect:+} => ");
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
                if new_depth > maxdepth {
                    maxdepth = new_depth
                }
                // Process target blocks for branching instructions
                if ins.target != BlockIdx::NULL {
                    if instr.is_block_push() {
                        // SETUP_* pseudo ops: target is a handler block.
                        // Handler entry depth uses the jump-path stack effect:
                        //   SETUP_FINALLY:  +1  (pushes exc)
                        //   SETUP_CLEANUP:  +2  (pushes lasti + exc)
                        //   SETUP_WITH:     +1  (pops __enter__ result, pushes lasti + exc)
                        let handler_effect: u32 = match instr.pseudo() {
                            Some(PseudoInstruction::SetupCleanup { .. }) => 2,
                            _ => 1, // SetupFinally and SetupWith
                        };
                        let handler_depth = depth + handler_effect;
                        if handler_depth > maxdepth {
                            maxdepth = handler_depth;
                        }
                        stackdepth_push(&mut stack, &mut start_depths, ins.target, handler_depth);
                    } else {
                        // SEND jumps to END_SEND with receiver still on stack.
                        // END_SEND performs the receiver pop.
                        let jump_effect = match instr.real() {
                            Some(Instruction::Send { .. }) => 0i32,
                            _ => effect,
                        };
                        let target_depth = depth.checked_add_signed(jump_effect).ok_or({
                            if jump_effect < 0 {
                                InternalError::StackUnderflow
                            } else {
                                InternalError::StackOverflow
                            }
                        })?;
                        if target_depth > maxdepth {
                            maxdepth = target_depth
                        }
                        stackdepth_push(&mut stack, &mut start_depths, ins.target, target_depth);
                    }
                }
                depth = new_depth;
                if instr.is_scope_exit() || instr.is_unconditional_jump() {
                    continue 'process_blocks;
                }
            }
            // Only push next block if it's not NULL
            if block.next != BlockIdx::NULL {
                stackdepth_push(&mut stack, &mut start_depths, block.next, depth);
            }
        }
        if DEBUG {
            eprintln!("DONE: {maxdepth}");
        }

        for (block, &start_depth) in self.blocks.iter_mut().zip(&start_depths) {
            block.start_depth = (start_depth != u32::MAX).then_some(start_depth);
        }

        // Fix up handler stack_depth in ExceptHandlerInfo using start_depths
        // computed above: depth = start_depth - 1 - preserve_lasti
        for block in self.blocks.iter_mut() {
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
) {
    let idx = target.idx();
    let block_depth = &mut start_depths[idx];
    if depth > *block_depth || *block_depth == u32::MAX {
        *block_depth = depth;
        stack.push(target);
    }
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
                write_varint(&mut linetable, (col as u32) + 1);
                write_varint(&mut linetable, (end_col as u32) + 1);
            }

            prev_line = line;
            length -= entry_length;
            i += entry_length;
        }
    }

    linetable.into_boxed_slice()
}

/// Generate Python 3.11+ exception table from instruction handler info
fn generate_exception_table(blocks: &[Block], block_to_index: &[u32]) -> Box<[u8]> {
    let mut entries: Vec<ExceptionTableEntry> = Vec::new();
    let mut current_entry: Option<(ExceptHandlerInfo, u32)> = None; // (handler_info, start_index)
    let mut instr_index = 0u32;

    // Iterate through all instructions in block order
    // instr_index is the index into the final instructions array (including EXTENDED_ARG)
    // This matches how frame.rs uses lasti
    for (_, block) in iter_blocks(blocks) {
        for instr in &block.instructions {
            // instr_size includes EXTENDED_ARG and CACHE entries
            let instr_size = instr.arg.instr_size() as u32 + instr.cache_entries;

            match (&current_entry, instr.except_handler) {
                // No current entry, no handler - nothing to do
                (None, None) => {}

                // No current entry, handler starts - begin new entry
                (None, Some(handler)) => {
                    current_entry = Some((handler, instr_index));
                }

                // Current entry exists, same handler - continue
                (Some((curr_handler, _)), Some(handler))
                    if curr_handler.handler_block == handler.handler_block
                        && curr_handler.stack_depth == handler.stack_depth
                        && curr_handler.preserve_lasti == handler.preserve_lasti => {}

                // Current entry exists, different handler - finish current, start new
                (Some((curr_handler, start)), Some(handler)) => {
                    let target_index = block_to_index[curr_handler.handler_block.idx()];
                    entries.push(ExceptionTableEntry::new(
                        *start,
                        instr_index,
                        target_index,
                        curr_handler.stack_depth as u16,
                        curr_handler.preserve_lasti,
                    ));
                    current_entry = Some((handler, instr_index));
                }

                // Current entry exists, no handler - finish current entry
                (Some((curr_handler, start)), None) => {
                    let target_index = block_to_index[curr_handler.handler_block.idx()];
                    entries.push(ExceptionTableEntry::new(
                        *start,
                        instr_index,
                        target_index,
                        curr_handler.stack_depth as u16,
                        curr_handler.preserve_lasti,
                    ));
                    current_entry = None;
                }
            }

            instr_index += instr_size; // Account for EXTENDED_ARG instructions
        }
    }

    // Finish any remaining entry
    if let Some((curr_handler, start)) = current_entry {
        let target_index = block_to_index[curr_handler.handler_block.idx()];
        entries.push(ExceptionTableEntry::new(
            start,
            instr_index,
            target_index,
            curr_handler.stack_depth as u16,
            curr_handler.preserve_lasti,
        ));
    }

    encode_exception_table(&entries)
}

/// Mark exception handler target blocks.
/// flowgraph.c mark_except_handlers
pub(crate) fn mark_except_handlers(blocks: &mut [Block]) {
    // Reset handler flags
    for block in blocks.iter_mut() {
        block.except_handler = false;
        block.preserve_lasti = false;
    }
    // Mark target blocks of SETUP_* as except handlers
    let targets: Vec<usize> = blocks
        .iter()
        .flat_map(|b| b.instructions.iter())
        .filter(|i| i.instr.is_block_push() && i.target != BlockIdx::NULL)
        .map(|i| i.target.idx())
        .collect();
    for idx in targets {
        blocks[idx].except_handler = true;
    }
}

/// flowgraph.c mark_cold
fn mark_cold(blocks: &mut [Block]) {
    let n = blocks.len();
    let mut warm = vec![false; n];
    let mut queue = VecDeque::new();

    warm[0] = true;
    queue.push_back(BlockIdx(0));

    while let Some(block_idx) = queue.pop_front() {
        let block = &blocks[block_idx.idx()];

        let has_fallthrough = block
            .instructions
            .last()
            .map(|ins| !ins.instr.is_scope_exit() && !ins.instr.is_unconditional_jump())
            .unwrap_or(true);
        if has_fallthrough && block.next != BlockIdx::NULL {
            let next_idx = block.next.idx();
            if !blocks[next_idx].except_handler && !warm[next_idx] {
                warm[next_idx] = true;
                queue.push_back(block.next);
            }
        }

        for instr in &block.instructions {
            if instr.target != BlockIdx::NULL {
                let target_idx = instr.target.idx();
                if !blocks[target_idx].except_handler && !warm[target_idx] {
                    warm[target_idx] = true;
                    queue.push_back(instr.target);
                }
            }
        }
    }

    for (i, block) in blocks.iter_mut().enumerate() {
        block.cold = !warm[i];
    }
}

/// flowgraph.c push_cold_blocks_to_end
fn push_cold_blocks_to_end(blocks: &mut Vec<Block>) {
    if blocks.len() <= 1 {
        return;
    }

    mark_cold(blocks);

    // If a cold block falls through to a warm block, add an explicit jump
    let fixups: Vec<(BlockIdx, BlockIdx)> = iter_blocks(blocks)
        .filter(|(_, block)| {
            block.cold
                && block.next != BlockIdx::NULL
                && !blocks[block.next.idx()].cold
                && block
                    .instructions
                    .last()
                    .map(|ins| !ins.instr.is_scope_exit() && !ins.instr.is_unconditional_jump())
                    .unwrap_or(true)
        })
        .map(|(idx, block)| (idx, block.next))
        .collect();

    for (cold_idx, warm_next) in fixups {
        let jump_block_idx = BlockIdx(blocks.len() as u32);
        let mut jump_block = Block {
            cold: true,
            ..Block::default()
        };
        jump_block.instructions.push(InstructionInfo {
            instr: PseudoInstruction::JumpNoInterrupt {
                delta: Arg::marker(),
            }
            .into(),
            arg: OpArg::new(0),
            target: warm_next,
            location: SourceLocation::default(),
            end_location: SourceLocation::default(),
            except_handler: None,
            lineno_override: Some(-1),
            cache_entries: 0,
        });
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
    }
}

/// Split blocks at branch points so each block has at most one branch
/// (conditional/unconditional jump) as its last instruction.
/// This matches CPython's CFG structure where each basic block has one exit.
fn split_blocks_at_jumps(blocks: &mut Vec<Block>) {
    let mut bi = 0;
    while bi < blocks.len() {
        // Find the first jump/branch instruction in the block
        let split_at = {
            let block = &blocks[bi];
            let mut found = None;
            for (i, ins) in block.instructions.iter().enumerate() {
                if is_conditional_jump(&ins.instr)
                    || ins.instr.is_unconditional_jump()
                    || ins.instr.is_scope_exit()
                {
                    if i + 1 < block.instructions.len() {
                        found = Some(i + 1);
                    }
                    break;
                }
            }
            found
        };
        if let Some(pos) = split_at {
            let new_block_idx = BlockIdx(blocks.len() as u32);
            let tail: Vec<InstructionInfo> = blocks[bi].instructions.drain(pos..).collect();
            let old_next = blocks[bi].next;
            let cold = blocks[bi].cold;
            blocks[bi].next = new_block_idx;
            blocks.push(Block {
                instructions: tail,
                next: old_next,
                cold,
                ..Block::default()
            });
            // Don't increment bi - re-check current block (it might still have issues)
        } else {
            bi += 1;
        }
    }
}

/// Jump threading: when a block's last jump targets a block whose first
/// instruction is an unconditional jump, redirect to the final target.
/// flowgraph.c optimize_basic_block + jump_thread
fn jump_threading(blocks: &mut [Block]) {
    jump_threading_impl(blocks, true);
}

fn jump_threading_unconditional(blocks: &mut [Block]) {
    jump_threading_impl(blocks, false);
}

fn jump_threading_impl(blocks: &mut [Block], include_conditional: bool) {
    let mut changed = true;
    while changed {
        changed = false;
        for bi in 0..blocks.len() {
            let last_idx = match blocks[bi].instructions.len().checked_sub(1) {
                Some(i) => i,
                None => continue,
            };
            let ins = blocks[bi].instructions[last_idx];
            let target = ins.target;
            if target == BlockIdx::NULL {
                continue;
            }
            if !(ins.instr.is_unconditional_jump()
                || include_conditional && is_conditional_jump(&ins.instr))
            {
                continue;
            }
            if include_conditional && is_conditional_jump(&ins.instr) {
                let next = next_nonempty_block(blocks, blocks[bi].next);
                if next != BlockIdx::NULL
                    && blocks[next.idx()]
                        .instructions
                        .last()
                        .is_some_and(|instr| instr.instr.is_scope_exit())
                {
                    continue;
                }
            }
            // Check if target block's first instruction is an unconditional jump
            let target_jump = blocks[target.idx()]
                .instructions
                .iter()
                .find(|ins| !matches!(ins.instr.real(), Some(Instruction::Nop)))
                .copied();
            if let Some(target_ins) = target_jump
                && target_ins.instr.is_unconditional_jump()
                && target_ins.target != BlockIdx::NULL
                && target_ins.target != target
            {
                let final_target = target_ins.target;
                if ins.target == final_target {
                    continue;
                }
                set_to_nop(&mut blocks[bi].instructions[last_idx]);
                let mut threaded = ins;
                threaded.arg = OpArg::new(0);
                threaded.target = final_target;
                threaded.location = target_ins.location;
                threaded.end_location = target_ins.end_location;
                threaded.cache_entries = 0;
                blocks[bi].instructions.push(threaded);
                changed = true;
            }
        }
    }
}

fn is_conditional_jump(instr: &AnyInstruction) -> bool {
    matches!(
        instr.real(),
        Some(
            Instruction::PopJumpIfFalse { .. }
                | Instruction::PopJumpIfTrue { .. }
                | Instruction::PopJumpIfNone { .. }
                | Instruction::PopJumpIfNotNone { .. }
        )
    )
}

/// Invert a conditional jump opcode.
fn reversed_conditional(instr: &AnyInstruction) -> Option<AnyInstruction> {
    Some(match instr.real()? {
        Instruction::PopJumpIfFalse { .. } => Instruction::PopJumpIfTrue {
            delta: Arg::marker(),
        }
        .into(),
        Instruction::PopJumpIfTrue { .. } => Instruction::PopJumpIfFalse {
            delta: Arg::marker(),
        }
        .into(),
        Instruction::PopJumpIfNone { .. } => Instruction::PopJumpIfNotNone {
            delta: Arg::marker(),
        }
        .into(),
        Instruction::PopJumpIfNotNone { .. } => Instruction::PopJumpIfNone {
            delta: Arg::marker(),
        }
        .into(),
        _ => return None,
    })
}

/// flowgraph.c normalize_jumps + remove_redundant_jumps
fn normalize_jumps(blocks: &mut Vec<Block>) {
    let mut visit_order = Vec::new();
    let mut visited = vec![false; blocks.len()];
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        visit_order.push(current);
        visited[current.idx()] = true;
        current = blocks[current.idx()].next;
    }

    visited.fill(false);

    for &block_idx in &visit_order {
        let idx = block_idx.idx();
        visited[idx] = true;

        // Remove redundant unconditional jump to next block
        let next = blocks[idx].next;
        if next != BlockIdx::NULL {
            let last = blocks[idx].instructions.last();
            let is_jump_to_next = last.is_some_and(|ins| {
                ins.instr.is_unconditional_jump()
                    && ins.target != BlockIdx::NULL
                    && ins.target == next
            });
            if is_jump_to_next && let Some(last_instr) = blocks[idx].instructions.last_mut() {
                set_to_nop(last_instr);
            }
        }

        // Normalize conditional jumps: forward gets NOT_TAKEN, backward gets inverted
        let last = blocks[idx].instructions.last();
        if let Some(last_ins) = last
            && is_conditional_jump(&last_ins.instr)
            && last_ins.target != BlockIdx::NULL
        {
            let target = last_ins.target;
            let is_forward = !visited[target.idx()];

            if is_forward {
                // Insert NOT_TAKEN after forward conditional jump
                let not_taken = InstructionInfo {
                    instr: Instruction::NotTaken.into(),
                    arg: OpArg::new(0),
                    target: BlockIdx::NULL,
                    location: last_ins.location,
                    end_location: last_ins.end_location,
                    except_handler: last_ins.except_handler,
                    lineno_override: None,
                    cache_entries: 0,
                };
                blocks[idx].instructions.push(not_taken);
            } else {
                // Backward conditional jump: invert and create new block
                // Transform: `cond_jump T` (backward)
                // Into: `reversed_cond_jump b_next` + new block [NOT_TAKEN, JUMP T]
                let loc = last_ins.location;
                let end_loc = last_ins.end_location;
                let exc_handler = last_ins.except_handler;

                if let Some(reversed) = reversed_conditional(&last_ins.instr) {
                    let old_next = blocks[idx].next;
                    let is_cold = blocks[idx].cold;

                    // Create new block with NOT_TAKEN + JUMP to original backward target
                    let new_block_idx = BlockIdx(blocks.len() as u32);
                    let mut new_block = Block {
                        cold: is_cold,
                        ..Block::default()
                    };
                    new_block.instructions.push(InstructionInfo {
                        instr: Instruction::NotTaken.into(),
                        arg: OpArg::new(0),
                        target: BlockIdx::NULL,
                        location: loc,
                        end_location: end_loc,
                        except_handler: exc_handler,
                        lineno_override: None,
                        cache_entries: 0,
                    });
                    new_block.instructions.push(InstructionInfo {
                        instr: PseudoInstruction::Jump {
                            delta: Arg::marker(),
                        }
                        .into(),
                        arg: OpArg::new(0),
                        target,
                        location: loc,
                        end_location: end_loc,
                        except_handler: exc_handler,
                        lineno_override: None,
                        cache_entries: 0,
                    });
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
            }
        }
    }

    // Rebuild visit_order since backward normalization may have added new blocks
    let mut visit_order = Vec::new();
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        visit_order.push(current);
        current = blocks[current.idx()].next;
    }

    // Resolve JUMP/JUMP_NO_INTERRUPT pseudo instructions before offset fixpoint.
    let mut block_order = vec![0u32; blocks.len()];
    for (pos, &block_idx) in visit_order.iter().enumerate() {
        block_order[block_idx.idx()] = pos as u32;
    }

    for &block_idx in &visit_order {
        let source_pos = block_order[block_idx.idx()];
        for info in &mut blocks[block_idx.idx()].instructions {
            let target = info.target;
            if target == BlockIdx::NULL {
                continue;
            }
            let target_pos = block_order[target.idx()];
            info.instr = match info.instr {
                AnyInstruction::Pseudo(PseudoInstruction::Jump { .. }) => {
                    if target_pos > source_pos {
                        Instruction::JumpForward {
                            delta: Arg::marker(),
                        }
                        .into()
                    } else {
                        Instruction::JumpBackward {
                            delta: Arg::marker(),
                        }
                        .into()
                    }
                }
                AnyInstruction::Pseudo(PseudoInstruction::JumpNoInterrupt { .. }) => {
                    if target_pos > source_pos {
                        Instruction::JumpForward {
                            delta: Arg::marker(),
                        }
                        .into()
                    } else {
                        Instruction::JumpBackwardNoInterrupt {
                            delta: Arg::marker(),
                        }
                        .into()
                    }
                }
                other => other,
            };
        }
    }
}

/// flowgraph.c inline_small_or_no_lineno_blocks
fn inline_small_or_no_lineno_blocks(blocks: &mut [Block]) {
    const MAX_COPY_SIZE: usize = 4;

    let block_exits_scope = |block: &Block| {
        block
            .instructions
            .last()
            .is_some_and(|ins| ins.instr.is_scope_exit())
    };
    let block_has_no_lineno = |block: &Block| {
        block
            .instructions
            .iter()
            .all(|ins| !instruction_has_lineno(ins))
    };

    loop {
        let mut changes = false;
        let mut current = BlockIdx(0);
        while current != BlockIdx::NULL {
            let next = blocks[current.idx()].next;
            let Some(last) = blocks[current.idx()].instructions.last().copied() else {
                current = next;
                continue;
            };
            if !last.instr.is_unconditional_jump() || last.target == BlockIdx::NULL {
                current = next;
                continue;
            }

            let target = last.target;
            let small_exit_block = block_exits_scope(&blocks[target.idx()])
                && blocks[target.idx()].instructions.len() <= MAX_COPY_SIZE;
            let no_lineno_no_fallthrough = block_has_no_lineno(&blocks[target.idx()])
                && !block_has_fallthrough(&blocks[target.idx()]);

            if small_exit_block || no_lineno_no_fallthrough {
                if let Some(last_instr) = blocks[current.idx()].instructions.last_mut() {
                    set_to_nop(last_instr);
                }
                let appended = blocks[target.idx()].instructions.clone();
                blocks[current.idx()].instructions.extend(appended);
                changes = true;
            }

            current = next;
        }

        if !changes {
            break;
        }
    }
}

fn remove_redundant_nops_in_blocks(blocks: &mut [Block]) -> usize {
    let mut changes = 0;
    let mut block_order = Vec::new();
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        block_order.push(current);
        current = blocks[current.idx()].next;
    }

    for block_idx in block_order {
        let bi = block_idx.idx();
        let mut src_instructions = core::mem::take(&mut blocks[bi].instructions);
        let mut kept = Vec::with_capacity(src_instructions.len());
        let mut prev_lineno = -1i32;

        for src in 0..src_instructions.len() {
            let instr = src_instructions[src];
            let lineno = instruction_lineno(&instr);
            let mut remove = false;

            if matches!(instr.instr.real(), Some(Instruction::Nop)) {
                if lineno < 0 || prev_lineno == lineno {
                    remove = true;
                } else if src < src_instructions.len() - 1 {
                    let next_lineno = instruction_lineno(&src_instructions[src + 1]);
                    if next_lineno == lineno {
                        remove = true;
                    } else if next_lineno < 0 {
                        src_instructions[src + 1].lineno_override = Some(lineno);
                        remove = true;
                    }
                } else {
                    let next = next_nonempty_block(blocks, blocks[bi].next);
                    if next != BlockIdx::NULL {
                        let mut next_lineno = None;
                        for next_instr in &blocks[next.idx()].instructions {
                            let line = instruction_lineno(next_instr);
                            if matches!(next_instr.instr.real(), Some(Instruction::Nop)) && line < 0
                            {
                                continue;
                            }
                            next_lineno = Some(line);
                            break;
                        }
                        if next_lineno.is_some_and(|line| line == lineno) {
                            remove = true;
                        }
                    }
                }
            }

            if remove {
                changes += 1;
            } else {
                kept.push(instr);
                prev_lineno = lineno;
            }
        }

        blocks[bi].instructions = kept;
    }

    changes
}

fn remove_redundant_jumps_in_blocks(blocks: &mut [Block]) -> usize {
    let mut changes = 0;
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let idx = current.idx();
        let next = next_nonempty_block(blocks, blocks[idx].next);
        let jump_target = blocks[idx]
            .instructions
            .last()
            .filter(|ins| ins.instr.is_unconditional_jump() && ins.target != BlockIdx::NULL)
            .map(|ins| ins.target);
        if next != BlockIdx::NULL
            && let Some(target) = jump_target
            && next_nonempty_block(blocks, target) == next
            && let Some(last_instr) = blocks[idx].instructions.last_mut()
        {
            set_to_nop(last_instr);
            changes += 1;
        }
        current = blocks[idx].next;
    }
    changes
}

fn remove_redundant_nops_and_jumps(blocks: &mut [Block]) {
    loop {
        let removed_nops = remove_redundant_nops_in_blocks(blocks);
        let removed_jumps = remove_redundant_jumps_in_blocks(blocks);
        if removed_nops + removed_jumps == 0 {
            break;
        }
    }
}

fn merge_unsafe_mask(slot: &mut Option<Vec<bool>>, incoming: &[bool]) -> bool {
    match slot {
        Some(existing) => {
            let mut changed = false;
            for (dst, src) in existing.iter_mut().zip(incoming.iter().copied()) {
                if src && !*dst {
                    *dst = true;
                    changed = true;
                }
            }
            changed
        }
        None => {
            *slot = Some(incoming.to_vec());
            true
        }
    }
}

/// Follow chain of empty blocks to find first non-empty block.
fn next_nonempty_block(blocks: &[Block], mut idx: BlockIdx) -> BlockIdx {
    while idx != BlockIdx::NULL
        && blocks[idx.idx()].instructions.is_empty()
        && blocks[idx.idx()].next != BlockIdx::NULL
    {
        idx = blocks[idx.idx()].next;
    }
    idx
}

fn instruction_lineno(instr: &InstructionInfo) -> i32 {
    instr
        .lineno_override
        .unwrap_or_else(|| instr.location.line.get() as i32)
}

fn instruction_has_lineno(instr: &InstructionInfo) -> bool {
    instruction_lineno(instr) > 0
}

fn block_has_fallthrough(block: &Block) -> bool {
    block
        .instructions
        .last()
        .is_none_or(|ins| !ins.instr.is_scope_exit() && !ins.instr.is_unconditional_jump())
}

fn is_jump_instruction(instr: &InstructionInfo) -> bool {
    instr.instr.is_unconditional_jump() || is_conditional_jump(&instr.instr)
}

fn is_exit_without_lineno(block: &Block) -> bool {
    let Some(first) = block.instructions.first() else {
        return false;
    };
    let Some(last) = block.instructions.last() else {
        return false;
    };
    !instruction_has_lineno(first) && last.instr.is_scope_exit()
}

fn is_jump_only_block(block: &Block) -> bool {
    let [instr] = block.instructions.as_slice() else {
        return false;
    };
    instr.instr.is_unconditional_jump() && instr.target != BlockIdx::NULL
}

fn is_scope_exit_block(block: &Block) -> bool {
    block
        .instructions
        .last()
        .is_some_and(|instr| instr.instr.is_scope_exit())
}

fn trailing_conditional_jump_index(block: &Block) -> Option<usize> {
    let last_idx = block.instructions.len().checked_sub(1)?;
    if is_conditional_jump(&block.instructions[last_idx].instr)
        && block.instructions[last_idx].target != BlockIdx::NULL
    {
        return Some(last_idx);
    }

    let cond_idx = last_idx.checked_sub(1)?;
    if matches!(
        block.instructions[last_idx].instr.real(),
        Some(Instruction::NotTaken)
    ) && is_conditional_jump(&block.instructions[cond_idx].instr)
        && block.instructions[cond_idx].target != BlockIdx::NULL
    {
        Some(cond_idx)
    } else {
        None
    }
}

fn reorder_conditional_exit_and_jump_blocks(blocks: &mut [Block]) {
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let idx = current.idx();
        let next = blocks[idx].next;
        let Some(cond_idx) = trailing_conditional_jump_index(&blocks[idx]) else {
            current = next;
            continue;
        };
        let last = blocks[idx].instructions[cond_idx];

        let Some(reversed) = reversed_conditional(&last.instr) else {
            current = next;
            continue;
        };

        let exit_start = next;
        let jump_start = last.target;
        if exit_start == BlockIdx::NULL || jump_start == BlockIdx::NULL || exit_start == jump_start
        {
            current = next;
            continue;
        }

        let mut exit_end = BlockIdx::NULL;
        let mut exit_block = BlockIdx::NULL;
        let mut cursor = exit_start;
        let mut exit_segment_valid = true;
        while cursor != BlockIdx::NULL && cursor != jump_start {
            if !blocks[cursor.idx()].instructions.is_empty() {
                if exit_block != BlockIdx::NULL {
                    exit_segment_valid = false;
                    break;
                }
                exit_block = cursor;
            }
            exit_end = cursor;
            cursor = blocks[cursor.idx()].next;
        }
        if !exit_segment_valid
            || cursor != jump_start
            || exit_end == BlockIdx::NULL
            || exit_block == BlockIdx::NULL
            || !is_scope_exit_block(&blocks[exit_block.idx()])
        {
            current = next;
            continue;
        }

        let mut jump_end = BlockIdx::NULL;
        let mut jump_block = BlockIdx::NULL;
        cursor = jump_start;
        while cursor != BlockIdx::NULL {
            jump_end = cursor;
            if blocks[cursor.idx()].instructions.is_empty() {
                cursor = blocks[cursor.idx()].next;
                continue;
            }
            if is_jump_only_block(&blocks[cursor.idx()]) {
                jump_block = cursor;
            }
            break;
        }
        if jump_block == BlockIdx::NULL {
            current = next;
            continue;
        }

        let after_jump = blocks[jump_end.idx()].next;
        blocks[idx].next = jump_start;
        blocks[jump_end.idx()].next = exit_start;
        blocks[exit_end.idx()].next = after_jump;

        let cond_mut = &mut blocks[idx].instructions[cond_idx];
        cond_mut.instr = reversed;
        cond_mut.target = exit_start;

        current = after_jump;
    }
}

fn reorder_conditional_jump_and_exit_blocks(blocks: &mut [Block]) {
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let idx = current.idx();
        let next = blocks[idx].next;
        let Some(cond_idx) = trailing_conditional_jump_index(&blocks[idx]) else {
            current = next;
            continue;
        };
        let last = blocks[idx].instructions[cond_idx];

        let Some(reversed) = reversed_conditional(&last.instr) else {
            current = next;
            continue;
        };

        let jump_start = next;
        let exit_start = last.target;
        if jump_start == BlockIdx::NULL || exit_start == BlockIdx::NULL || jump_start == exit_start
        {
            current = next;
            continue;
        }

        let mut jump_end = BlockIdx::NULL;
        let mut jump_block = BlockIdx::NULL;
        let mut cursor = jump_start;
        let mut jump_segment_valid = true;
        while cursor != BlockIdx::NULL && cursor != exit_start {
            if !blocks[cursor.idx()].instructions.is_empty() {
                if jump_block != BlockIdx::NULL || !is_jump_only_block(&blocks[cursor.idx()]) {
                    jump_segment_valid = false;
                    break;
                }
                jump_block = cursor;
            }
            jump_end = cursor;
            cursor = blocks[cursor.idx()].next;
        }
        if !jump_segment_valid || cursor != exit_start || jump_block == BlockIdx::NULL {
            current = next;
            continue;
        }
        let jump_instr = blocks[jump_block.idx()].instructions[0];
        if !matches!(
            jump_instr.instr.real(),
            Some(Instruction::JumpForward { .. })
        ) {
            current = next;
            continue;
        }

        let mut exit_end = BlockIdx::NULL;
        let mut exit_block = BlockIdx::NULL;
        let after_exit = loop {
            if cursor == BlockIdx::NULL {
                break BlockIdx::NULL;
            }
            if !blocks[cursor.idx()].instructions.is_empty() {
                if exit_block != BlockIdx::NULL {
                    break cursor;
                }
                if !is_scope_exit_block(&blocks[cursor.idx()]) {
                    exit_block = BlockIdx::NULL;
                    break BlockIdx::NULL;
                }
                exit_block = cursor;
            }
            exit_end = cursor;
            cursor = blocks[cursor.idx()].next;
        };
        if exit_block == BlockIdx::NULL || exit_end == BlockIdx::NULL {
            current = next;
            continue;
        }

        blocks[idx].next = exit_start;
        blocks[exit_end.idx()].next = jump_start;
        blocks[jump_end.idx()].next = after_exit;

        let cond_mut = &mut blocks[idx].instructions[cond_idx];
        cond_mut.instr = reversed;
        cond_mut.target = jump_start;

        current = after_exit;
    }
}

fn maybe_propagate_location(
    instr: &mut InstructionInfo,
    location: SourceLocation,
    end_location: SourceLocation,
) {
    if !instruction_has_lineno(instr) {
        instr.location = location;
        instr.end_location = end_location;
        instr.lineno_override = None;
    }
}

fn propagate_locations_in_block(
    block: &mut Block,
    location: SourceLocation,
    end_location: SourceLocation,
) {
    let mut prev_location = location;
    let mut prev_end_location = end_location;
    for instr in &mut block.instructions {
        maybe_propagate_location(instr, prev_location, prev_end_location);
        prev_location = instr.location;
        prev_end_location = instr.end_location;
    }
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
            let next = next_nonempty_block(blocks, block.next);
            if next != BlockIdx::NULL {
                predecessors[next.idx()] += 1;
            }
        }
        for ins in &block.instructions {
            if ins.target != BlockIdx::NULL {
                let target = next_nonempty_block(blocks, ins.target);
                if target != BlockIdx::NULL {
                    predecessors[target.idx()] += 1;
                }
            }
        }
        current = block.next;
    }
    predecessors
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
        if target == BlockIdx::NULL || !is_exit_without_lineno(&blocks[target.idx()]) {
            current = blocks[current.idx()].next;
            continue;
        }
        if predecessors[target.idx()] <= 1 {
            current = blocks[current.idx()].next;
            continue;
        }

        // Copy the exit block and splice it into the linked list after current
        let new_idx = BlockIdx(blocks.len() as u32);
        let mut new_block = blocks[target.idx()].clone();
        let jump_loc = last.location;
        let jump_end_loc = last.end_location;
        propagate_locations_in_block(&mut new_block, jump_loc, jump_end_loc);
        let old_next = blocks[current.idx()].next;
        new_block.next = old_next;
        blocks.push(new_block);
        blocks[current.idx()].next = new_idx;

        // Update the jump target
        let last_mut = blocks[current.idx()].instructions.last_mut().unwrap();
        last_mut.target = new_idx;
        predecessors[target.idx()] -= 1;
        predecessors.push(1);

        // Skip past the newly inserted block
        current = old_next;
    }

    current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let block = &blocks[current.idx()];
        if let Some(last) = block.instructions.last()
            && block_has_fallthrough(block)
        {
            let target = next_nonempty_block(blocks, block.next);
            if target != BlockIdx::NULL
                && predecessors[target.idx()] == 1
                && is_exit_without_lineno(&blocks[target.idx()])
            {
                let last_location = last.location;
                let last_end_location = last.end_location;
                propagate_locations_in_block(
                    &mut blocks[target.idx()],
                    last_location,
                    last_end_location,
                );
            }
        }
        current = blocks[current.idx()].next;
    }
}

fn duplicate_jump_targets_without_lineno(blocks: &mut Vec<Block>, predecessors: &mut Vec<u32>) {
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let block = &blocks[current.idx()];
        let last = match block.instructions.last() {
            Some(ins) if ins.instr.is_unconditional_jump() && ins.target != BlockIdx::NULL => *ins,
            _ => {
                current = blocks[current.idx()].next;
                continue;
            }
        };

        let target = next_nonempty_block(blocks, last.target);
        if target == BlockIdx::NULL || !is_jump_only_block(&blocks[target.idx()]) {
            current = blocks[current.idx()].next;
            continue;
        }
        if predecessors[target.idx()] <= 1 {
            current = blocks[current.idx()].next;
            continue;
        }

        let new_idx = BlockIdx(blocks.len() as u32);
        let mut new_block = blocks[target.idx()].clone();
        propagate_locations_in_block(&mut new_block, last.location, last.end_location);
        let old_next = blocks[current.idx()].next;
        new_block.next = old_next;
        blocks.push(new_block);
        blocks[current.idx()].next = new_idx;

        let last_mut = blocks[current.idx()].instructions.last_mut().unwrap();
        last_mut.target = new_idx;
        predecessors[target.idx()] -= 1;
        predecessors.push(1);

        current = old_next;
    }
}

fn propagate_line_numbers(blocks: &mut [Block], predecessors: &[u32]) {
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let last = blocks[current.idx()].instructions.last().copied();
        if let Some(last) = last {
            let (next_block, has_fallthrough) = {
                let block = &blocks[current.idx()];
                (block.next, block_has_fallthrough(block))
            };

            {
                let block = &mut blocks[current.idx()];
                let mut prev_location = None;
                for instr in &mut block.instructions {
                    if let Some((location, end_location)) = prev_location {
                        maybe_propagate_location(instr, location, end_location);
                    }
                    prev_location = Some((instr.location, instr.end_location));
                }
            }

            if has_fallthrough {
                let target = next_nonempty_block(blocks, next_block);
                if target != BlockIdx::NULL && predecessors[target.idx()] == 1 {
                    propagate_locations_in_block(
                        &mut blocks[target.idx()],
                        last.location,
                        last.end_location,
                    );
                }
            }

            if is_jump_instruction(&last) {
                let target = next_nonempty_block(blocks, last.target);
                if target != BlockIdx::NULL && predecessors[target.idx()] == 1 {
                    propagate_locations_in_block(
                        &mut blocks[target.idx()],
                        last.location,
                        last.end_location,
                    );
                }
            }
        }
        current = blocks[current.idx()].next;
    }
}

fn resolve_line_numbers(blocks: &mut Vec<Block>) {
    let mut predecessors = compute_predecessors(blocks);
    duplicate_exits_without_lineno(blocks, &mut predecessors);
    duplicate_jump_targets_without_lineno(blocks, &mut predecessors);
    propagate_line_numbers(blocks, &predecessors);
}

/// Duplicate `LOAD_CONST None + RETURN_VALUE` for blocks that fall through
/// to the final return block.
fn duplicate_end_returns(blocks: &mut [Block]) {
    // Walk the block chain and keep the last non-empty block.
    let mut last_block = BlockIdx::NULL;
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        if !blocks[current.idx()].instructions.is_empty() {
            last_block = current;
        }
        current = blocks[current.idx()].next;
    }
    if last_block == BlockIdx::NULL {
        return;
    }

    let last_insts = &blocks[last_block.idx()].instructions;
    // Only apply when the last block is EXACTLY a return-None epilogue
    let is_return_block = last_insts.len() == 2
        && matches!(
            last_insts[0].instr,
            AnyInstruction::Real(Instruction::LoadConst { .. })
        )
        && matches!(
            last_insts[1].instr,
            AnyInstruction::Real(Instruction::ReturnValue)
        );
    if !is_return_block {
        return;
    }

    // Get the return instructions to clone
    let return_insts: Vec<InstructionInfo> = last_insts[last_insts.len() - 2..].to_vec();

    // Find non-cold blocks that fall through to the last block
    let mut blocks_to_fix = Vec::new();
    current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let block = &blocks[current.idx()];
        let next = next_nonempty_block(blocks, block.next);
        if current != last_block && next == last_block && !block.cold && !block.except_handler {
            let last_ins = block.instructions.last();
            let has_fallthrough = last_ins
                .map(|ins| !ins.instr.is_scope_exit() && !ins.instr.is_unconditional_jump())
                .unwrap_or(true);
            // Don't duplicate if block already ends with the same return pattern
            let already_has_return = block.instructions.len() >= 2 && {
                let n = block.instructions.len();
                matches!(
                    block.instructions[n - 2].instr,
                    AnyInstruction::Real(Instruction::LoadConst { .. })
                ) && matches!(
                    block.instructions[n - 1].instr,
                    AnyInstruction::Real(Instruction::ReturnValue)
                )
            };
            if has_fallthrough && !already_has_return {
                blocks_to_fix.push(current);
            }
        }
        current = blocks[current.idx()].next;
    }

    // Duplicate the return instructions at the end of fall-through blocks
    for block_idx in blocks_to_fix {
        blocks[block_idx.idx()]
            .instructions
            .extend_from_slice(&return_insts);
    }
}

/// Label exception targets: walk CFG with except stack, set per-instruction
/// handler info and block preserve_lasti flag. Converts POP_BLOCK to NOP.
/// flowgraph.c label_exception_targets + push_except_block
pub(crate) fn label_exception_targets(blocks: &mut [Block]) {
    #[derive(Clone)]
    struct ExceptEntry {
        handler_block: BlockIdx,
        preserve_lasti: bool,
    }

    let num_blocks = blocks.len();
    if num_blocks == 0 {
        return;
    }

    let mut visited = vec![false; num_blocks];
    let mut block_stacks: Vec<Option<Vec<ExceptEntry>>> = vec![None; num_blocks];

    // Entry block
    visited[0] = true;
    block_stacks[0] = Some(Vec::new());

    let mut todo = vec![BlockIdx(0)];

    while let Some(block_idx) = todo.pop() {
        let bi = block_idx.idx();
        let mut stack = block_stacks[bi].take().unwrap_or_default();
        let mut last_yield_except_depth: i32 = -1;

        let instr_count = blocks[bi].instructions.len();
        for i in 0..instr_count {
            // Read all needed fields (each temporary borrow ends immediately)
            let target = blocks[bi].instructions[i].target;
            let arg = blocks[bi].instructions[i].arg;
            let is_push = blocks[bi].instructions[i].instr.is_block_push();
            let is_pop = blocks[bi].instructions[i].instr.is_pop_block();

            if is_push {
                // Determine preserve_lasti from instruction type (push_except_block)
                let preserve_lasti = matches!(
                    blocks[bi].instructions[i].instr.pseudo(),
                    Some(
                        PseudoInstruction::SetupWith { .. }
                            | PseudoInstruction::SetupCleanup { .. }
                    )
                );

                // Set preserve_lasti on handler block
                if preserve_lasti && target != BlockIdx::NULL {
                    blocks[target.idx()].preserve_lasti = true;
                }

                // Propagate except stack to handler block if not visited
                if target != BlockIdx::NULL && !visited[target.idx()] {
                    visited[target.idx()] = true;
                    block_stacks[target.idx()] = Some(stack.clone());
                    todo.push(target);
                }

                // Push handler onto except stack
                stack.push(ExceptEntry {
                    handler_block: target,
                    preserve_lasti,
                });
            } else if is_pop {
                debug_assert!(
                    !stack.is_empty(),
                    "POP_BLOCK with empty except stack at block {bi} instruction {i}"
                );
                stack.pop();
                // POP_BLOCK → NOP
                set_to_nop(&mut blocks[bi].instructions[i]);
            } else {
                // Set except_handler for this instruction from except stack top
                // stack_depth placeholder: filled by fixup_handler_depths
                let handler_info = stack.last().map(|e| ExceptHandlerInfo {
                    handler_block: e.handler_block,
                    stack_depth: 0,
                    preserve_lasti: e.preserve_lasti,
                });
                blocks[bi].instructions[i].except_handler = handler_info;

                // Track YIELD_VALUE except stack depth
                // Record the except stack depth at the point of yield.
                // With the StopIteration wrapper, depth is naturally correct:
                // - plain yield outside try: depth=1 → DEPTH1 set
                // - yield inside try: depth=2+ → no DEPTH1
                // - yield-from/await: has internal SETUP_FINALLY → depth=2+ → no DEPTH1
                if let Some(Instruction::YieldValue { .. }) =
                    blocks[bi].instructions[i].instr.real()
                {
                    last_yield_except_depth = stack.len() as i32;
                }

                // Set RESUME DEPTH1 flag based on last yield's except depth
                if let Some(Instruction::Resume { context }) =
                    blocks[bi].instructions[i].instr.real()
                {
                    let location = context.get(arg).location();
                    match location {
                        oparg::ResumeLocation::AtFuncStart => {}
                        _ => {
                            if last_yield_except_depth == 1 {
                                blocks[bi].instructions[i].arg =
                                    OpArg::new(oparg::ResumeContext::new(location, true).as_u32());
                            }
                            last_yield_except_depth = -1;
                        }
                    }
                }

                // For jump instructions, propagate except stack to target
                if target != BlockIdx::NULL && !visited[target.idx()] {
                    visited[target.idx()] = true;
                    block_stacks[target.idx()] = Some(stack.clone());
                    todo.push(target);
                }
            }
        }

        // Propagate to fallthrough block (block.next)
        let next = blocks[bi].next;
        if next != BlockIdx::NULL && !visited[next.idx()] {
            let has_fallthrough = blocks[bi]
                .instructions
                .last()
                .map(|ins| !ins.instr.is_scope_exit() && !ins.instr.is_unconditional_jump())
                .unwrap_or(true); // Empty block falls through
            if has_fallthrough {
                visited[next.idx()] = true;
                block_stacks[next.idx()] = Some(stack);
                todo.push(next);
            }
        }
    }
}

/// Convert remaining pseudo ops to real instructions or NOP.
/// flowgraph.c convert_pseudo_ops
pub(crate) fn convert_pseudo_ops(blocks: &mut [Block], cellfixedoffsets: &[u32]) {
    for block in blocks.iter_mut() {
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
                // PopBlock in reachable blocks is converted to NOP by
                // label_exception_targets. Dead blocks may still have them.
                PseudoInstruction::PopBlock => {
                    set_to_nop(info);
                }
                // LOAD_CLOSURE → LOAD_FAST (using cellfixedoffsets for merged layout)
                PseudoInstruction::LoadClosure { i } => {
                    let cell_relative = i.get(info.arg) as usize;
                    let new_idx = cellfixedoffsets[cell_relative];
                    info.arg = OpArg::new(new_idx);
                    info.instr = Instruction::LoadFast {
                        var_num: Arg::marker(),
                    }
                    .into();
                }
                // Jump pseudo ops are resolved during block linearization
                PseudoInstruction::Jump { .. } | PseudoInstruction::JumpNoInterrupt { .. } => {}
                // These should have been resolved earlier
                PseudoInstruction::AnnotationsPlaceholder
                | PseudoInstruction::JumpIfFalse { .. }
                | PseudoInstruction::JumpIfTrue { .. }
                | PseudoInstruction::StoreFastMaybeNull { .. } => {
                    unreachable!("Unexpected pseudo instruction in convert_pseudo_ops: {pseudo:?}")
                }
            }
        }
    }
}

/// Build cellfixedoffsets mapping: cell/free index -> localsplus index.
/// Merged cells (cellvar also in varnames) get the local slot index.
/// Non-merged cells get slots after nlocals. Free vars follow.
pub(crate) fn build_cellfixedoffsets(
    varnames: &IndexSet<String>,
    cellvars: &IndexSet<String>,
    freevars: &IndexSet<String>,
) -> Vec<u32> {
    let nlocals = varnames.len();
    let ncells = cellvars.len();
    let nfrees = freevars.len();
    let mut fixed = Vec::with_capacity(ncells + nfrees);
    let mut numdropped = 0usize;
    for (i, cellvar) in cellvars.iter().enumerate() {
        if let Some(local_idx) = varnames.get_index_of(cellvar) {
            fixed.push(local_idx as u32);
            numdropped += 1;
        } else {
            fixed.push((nlocals + i - numdropped) as u32);
        }
    }
    for i in 0..nfrees {
        fixed.push((nlocals + ncells - numdropped + i) as u32);
    }
    fixed
}

/// Convert DEREF instruction opargs from cell-relative indices to localsplus indices
/// using the cellfixedoffsets mapping.
pub(crate) fn fixup_deref_opargs(blocks: &mut [Block], cellfixedoffsets: &[u32]) {
    for block in blocks.iter_mut() {
        for info in &mut block.instructions {
            let Some(instr) = info.instr.real() else {
                continue;
            };
            let needs_fixup = matches!(
                instr,
                Instruction::LoadDeref { .. }
                    | Instruction::StoreDeref { .. }
                    | Instruction::DeleteDeref { .. }
                    | Instruction::LoadFromDictOrDeref { .. }
                    | Instruction::MakeCell { .. }
            );
            if needs_fixup {
                let cell_relative = u32::from(info.arg) as usize;
                info.arg = OpArg::new(cellfixedoffsets[cell_relative]);
            }
        }
    }
}
