use alloc::collections::VecDeque;
use core::cmp::Reverse;
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
        InstrDisplayContext, Instruction, IntrinsicFunction1, Label, OpArg, Opcode,
        PseudoInstruction, PseudoOpcode, PyCodeLocationInfoKind, encode_exception_table, oparg,
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
const MAX_COLLECTION_SIZE: usize = 256;
const MAX_TOTAL_ITEMS: isize = 1024;
const MAX_STR_SIZE: usize = 4096;
const MIN_CONST_SEQUENCE_SIZE: usize = 3;
const STACK_USE_GUIDELINE: usize = 30;

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
    pub folded_from_nonliteral_expr: bool,
    /// Override line number for linetable (e.g., line 0 for module RESUME)
    pub lineno_override: Option<i32>,
    /// Number of CACHE code units emitted after this instruction
    pub cache_entries: u32,
    /// Preserve a redundant jump until final emission so a zero-width jump
    /// materializes as a line-marker NOP, matching CPython's late CFG shape.
    pub preserve_redundant_jump_as_nop: bool,
    /// Drop this NOP before line propagation if it still has no location.
    pub remove_no_location_nop: bool,
    /// This no-location NOP was created by CPython-style nop_out() folding.
    pub folded_operand_nop: bool,
    /// This instruction was emitted as part of a synthetic no-location exit.
    pub no_location_exit: bool,
    /// Keep this no-location NOP until line propagation when it starts a block.
    pub preserve_block_start_no_location_nop: bool,
    /// This is the success jump emitted after a non-final match case body.
    pub match_success_jump: bool,
    /// This is the final jump emitted by codegen_break()/codegen_continue()
    /// after unwinding cleanup blocks.
    pub break_continue_cleanup_jump: bool,
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
    info.folded_from_nonliteral_expr = false;
    info.cache_entries = 0;
    info.preserve_redundant_jump_as_nop = false;
    info.remove_no_location_nop = false;
    info.folded_operand_nop = false;
    info.no_location_exit = false;
    info.preserve_block_start_no_location_nop = false;
    info.match_success_jump = false;
    info.break_continue_cleanup_jump = false;
}

fn nop_out_no_location(info: &mut InstructionInfo) {
    set_to_nop(info);
    info.lineno_override = Some(-1);
    info.remove_no_location_nop = true;
    info.folded_operand_nop = true;
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
    /// Whether LOAD_FAST borrow optimization should be suppressed for this block.
    pub disable_load_fast_borrow: bool,
    /// Entry block for a try-else orelse suite split after POP_BLOCK.
    pub try_else_orelse_entry: bool,
    /// Whether this block corresponds to a compiler label (b_label).
    pub label: bool,
    /// Preserve empty unconditional-jump targets through optimize_load_fast_borrow.
    pub load_fast_barrier: bool,
    /// Let optimize_load_fast_borrow pass through this empty compiler artifact.
    pub load_fast_passthrough: bool,
    /// Continuation label that CPython attaches to a preceding empty block.
    pub load_fast_label_reuse_passthrough: bool,
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
            disable_load_fast_borrow: false,
            try_else_orelse_entry: false,
            label: false,
            load_fast_barrier: false,
            load_fast_passthrough: false,
            load_fast_label_reuse_passthrough: false,
        }
    }
}

pub struct CodeInfo {
    pub flags: CodeFlags,
    pub source_path: String,
    pub private: Option<String>, // For private name mangling, mostly for class

    pub blocks: Vec<Block>,
    pub current_block: BlockIdx,
    pub annotations_blocks: Option<Vec<Block>>,

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

    // Track when compiling the final direct statement in a sync with body.
    pub in_final_with_cleanup_statement: u32,

    // Track when compiling the orelse suite of try/except.
    pub in_try_else_orelse: u32,

    // PEP 649: Next index for conditional annotation tracking
    // u_next_conditional_annotation_index
    pub next_conditional_annotation_index: u32,
}

impl CodeInfo {
    pub fn finalize_code(
        mut self,
        opts: &crate::compile::CompileOpts,
    ) -> crate::InternalResult<CodeObject> {
        self.splice_annotations_blocks();
        if self.flags.contains(CodeFlags::OPTIMIZED) {
            // CPython's cfg_builder_maybe_start_new_block() starts a fresh
            // basicblock before emitting any instruction after a terminator,
            // including conditional jumps. Keep that invariant before
            // optimize_cfg-style constant folding so blocks that fold to empty
            // remain as b_next barriers for optimize_load_fast().
            split_blocks_at_jumps(&mut self.blocks);
        }
        // CPython-style per-block forward walk that interleaves tuple, list,
        // set, unary, and binop folding. Matches optimize_basic_block() in
        // flowgraph.c so co_consts get the same insertion order CPython
        // produces (e.g. negated literals like -1 land in co_consts at their
        // source position, not before every tuple folded by a separate pass).
        self.fold_constants_per_block();
        // Legacy global passes still run after the walker so any cascade the
        // single per-block iteration missed gets picked up.
        self.fold_binop_constants();
        self.fold_unary_constants();
        self.fold_binop_constants(); // re-run after unary folding: -1 + 2 → 1
        self.fold_unary_constants(); // re-run after binop folding: -(2.0 ** -54) → const
        self.fold_tuple_constants();
        self.fold_binop_constants(); // re-run after tuple folding: (1,) * 2 → const
        self.fold_list_constants();
        self.fold_set_constants();
        self.optimize_lists_and_sets();
        self.convert_to_load_small_int();
        self.remove_unused_consts();

        // DCE always runs (removes dead code after terminal instructions)
        self.dce();
        // BUILD_TUPLE n + UNPACK_SEQUENCE n → NOP + SWAP (n=2,3) or NOP+NOP (n=1)
        self.optimize_build_tuple_unpack();
        // Dead store elimination for duplicate STORE_FAST targets
        // (apply_static_swaps in CPython's flowgraph.c)
        self.eliminate_dead_stores();
        // apply_static_swaps: reorder stores to eliminate SWAPs
        self.apply_static_swaps();
        // Peephole optimizer handles constant and compare folding.
        self.peephole_optimize();
        // Per-block walker first to preserve CPython-style instruction-order
        // const registration after peephole transformations exposed new
        // foldable patterns.
        self.fold_constants_per_block();
        self.fold_tuple_constants();
        self.fold_binop_constants();
        self.fold_list_constants();
        self.fold_set_constants();
        self.optimize_lists_and_sets();
        self.convert_to_load_small_int();
        self.remove_unused_consts();
        // CPython's CFG builder starts a new basic block after a terminator.
        // Peephole constant-jump folding can create new terminators, so split
        // before DCE clears unreachable successor instructions; otherwise the
        // empty placeholders CPython leaves in b_next are lost.
        split_blocks_at_jumps(&mut self.blocks);
        self.dce();

        // Phase 1: _PyCfg_OptimizeCodeUnit (flowgraph.c)
        // Split blocks so each block has at most one branch as its last instruction
        split_blocks_at_jumps(&mut self.blocks);
        mark_except_handlers(&mut self.blocks);
        label_exception_targets(&mut self.blocks);
        // CPython's CFG builder does not leave empty unconditional-jump targets
        // in front of small exit blocks. Redirect only unconditional jumps
        // here so inline_small_or_no_lineno_blocks() can see direct exit
        // targets without erasing conditional target NOP anchors.
        redirect_empty_unconditional_jump_targets(&mut self.blocks);
        // CPython optimize_cfg starts by inlining tiny exit/no-lineno blocks
        // before unreachable elimination and later jump cleanup.
        inline_small_or_no_lineno_blocks(&mut self.blocks);
        // optimize_cfg: jump threading (before push_cold_blocks_to_end)
        jump_threading(&mut self.blocks);
        self.eliminate_unreachable_blocks_without_exception_roots();
        // CPython optimize_cfg resolves line numbers before local checks and
        // superinstruction insertion, so fusion decisions see propagated
        // source locations.
        resolve_line_numbers(&mut self.blocks);
        self.remove_nops();
        self.add_checks_for_loads_of_uninitialized_variables();
        // CPython inserts superinstructions in _PyCfg_OptimizeCodeUnit, before
        // later jump normalization / block reordering can create adjacencies
        // that never exist at this stage in flowgraph.c.
        self.insert_superinstructions();
        inline_single_predecessor_artificial_expr_exit_blocks(&mut self.blocks);
        push_cold_blocks_to_end(&mut self.blocks);
        // CPython resolves line numbers again after cold-block extraction.
        resolve_line_numbers(&mut self.blocks);
        reorder_conditional_chain_and_jump_back_blocks(&mut self.blocks);
        reorder_conditional_scope_exit_and_jump_back_blocks(&mut self.blocks, true, true);

        // Phase 2: _PyCfg_OptimizedCfgToInstructionSequence (flowgraph.c)
        normalize_jumps(&mut self.blocks);
        reorder_conditional_exit_and_jump_blocks(&mut self.blocks);
        reorder_conditional_jump_and_exit_blocks(&mut self.blocks);
        reorder_conditional_break_continue_blocks(&mut self.blocks);
        reorder_conditional_explicit_continue_scope_exit_blocks(&mut self.blocks);
        reorder_conditional_implicit_continue_scope_exit_blocks(&mut self.blocks);
        reorder_conditional_scope_exit_and_jump_back_blocks(&mut self.blocks, true, true);
        reorder_exception_handler_conditional_continue_scope_exit_blocks(&mut self.blocks);
        deduplicate_adjacent_jump_back_blocks(&mut self.blocks);
        reorder_conditional_body_and_implicit_continue_blocks(&mut self.blocks);
        reorder_conditional_scope_exit_and_jump_back_blocks(&mut self.blocks, true, true);
        reorder_conditional_scope_exit_and_jump_back_blocks(&mut self.blocks, false, true);
        reorder_conditional_scope_exit_and_jump_back_blocks(&mut self.blocks, false, true);
        reorder_conditional_scope_exit_and_jump_back_blocks(&mut self.blocks, false, false);
        self.dce(); // re-run within-block DCE after normalize_jumps creates new instructions
        self.eliminate_unreachable_blocks();
        resolve_line_numbers(&mut self.blocks);
        // Keep these empty targets until after optimize_load_fast_borrow.
        // CPython's optimize_load_fast() visits empty blocks directly; because
        // basicblock_last_instr() is NULL, they do not push fallthrough.
        materialize_empty_conditional_exit_targets(&mut self.blocks);
        retarget_assert_conditional_jumps_to_empty_predecessor(&mut self.blocks);
        canonicalize_empty_label_blocks(&mut self.blocks);
        inline_small_fast_return_blocks(&mut self.blocks);
        duplicate_end_returns(&mut self.blocks, &self.metadata);
        duplicate_fallthrough_jump_back_targets(&mut self.blocks);
        duplicate_shared_jump_back_targets(&mut self.blocks);
        self.dce(); // truncate after terminal in blocks that got return duplicated
        self.eliminate_unreachable_blocks(); // remove now-unreachable last block
        self.remove_redundant_const_pop_top_pairs();
        remove_redundant_nops_and_jumps(&mut self.blocks);
        // Some jump-only blocks only appear after late CFG cleanup. Thread them
        // once more so loop backedges stay direct instead of becoming
        // JUMP_FORWARD -> JUMP_BACKWARD chains.
        jump_threading_unconditional(&mut self.blocks);
        self.eliminate_unreachable_blocks();
        remove_redundant_nops_and_jumps(&mut self.blocks);
        inline_with_suppress_return_blocks(&mut self.blocks);
        inline_pop_except_return_blocks(&mut self.blocks);
        duplicate_named_except_cleanup_returns(&mut self.blocks, &self.metadata);
        self.eliminate_unreachable_blocks();
        resolve_line_numbers(&mut self.blocks);
        reorder_lineful_jump_back_runs_by_descending_line(&mut self.blocks);
        let cellfixedoffsets = build_cellfixedoffsets(
            &self.metadata.varnames,
            &self.metadata.cellvars,
            &self.metadata.freevars,
        );
        // Late CFG cleanup can create or reshuffle handler entry blocks.
        // Refresh exceptional block flags before optimize_load_fast_borrow so
        // borrow loads are not introduced into exception-handler paths.
        mark_except_handlers(&mut self.blocks);
        // CPython's optimize_load_fast runs with block start depths already known.
        // Compute them here so the abstract stack simulation can use the real
        // CFG entry depth for each block.
        let max_stackdepth = self.max_stackdepth()?;
        // Match CPython order: pseudo ops are lowered after stackdepth
        // calculation, then redundant NOPs from pseudo lowering are removed
        // before optimize_load_fast.
        convert_pseudo_ops(&mut self.blocks, &cellfixedoffsets);
        remove_redundant_nops_and_jumps(&mut self.blocks);
        inline_named_except_cleanup_jump_blocks(&mut self.blocks, &self.metadata);
        deduplicate_adjacent_pop_except_jump_back_blocks(&mut self.blocks);
        self.eliminate_unreachable_blocks();
        remove_redundant_nops_and_jumps(&mut self.blocks);
        // CPython optimize_load_fast() runs after CFG cleanup, so synthetic
        // empty labels that are not intended as load-fast barriers must not
        // stop its worklist traversal.
        redirect_load_fast_passthrough_targets(&mut self.blocks);
        self.compute_load_fast_start_depths();
        // optimize_load_fast: after normalize_jumps
        self.optimize_load_fast_borrow();
        // Assembly/late cleanup still wants concrete instruction targets, so
        // redirect empty targets after preserving CPython's load-fast barrier.
        redirect_empty_block_targets(&mut self.blocks);
        self.apply_static_swaps();
        self.optimize_load_global_push_null();
        self.reorder_entry_prefix_cell_setup();
        self.remove_unused_consts();

        let Self {
            flags,
            source_path,
            private: _, // private is only used during compilation

            mut blocks,
            current_block: _,
            annotations_blocks: _,
            metadata,
            static_attributes: _,
            in_inlined_comp: _,
            fblock: _,
            symbol_table_index: _,
            in_conditional_block: _,
            in_final_with_cleanup_statement: _,
            in_try_else_orelse: _,
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
                    if instr.preserve_redundant_jump_as_nop {
                        remove = false;
                    } else if lineno < 0 || prev_lineno == lineno {
                        remove = true;
                    } else if src < src_instructions.len() - 1 {
                        if src_instructions[src + 1].instr.is_block_push() {
                            remove = false;
                        } else if src_instructions[src + 1].folded_from_nonliteral_expr {
                            remove = true;
                        } else {
                            let next_lineno = src_instructions[src + 1]
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
                    } else {
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
        for block in &mut blocks {
            if let Some(pos) = block
                .instructions
                .iter()
                .position(|ins| ins.instr.is_scope_exit() || ins.instr.is_unconditional_jump())
            {
                block.instructions.truncate(pos + 1);
            }
        }

        // Pre-compute cache_entries for real (non-pseudo) instructions
        for block in &mut blocks {
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
                        if info.instr.is_unconditional_jump() && target_offset == offset_after {
                            op = Opcode::Nop.into();
                            info.instr = op.into();
                            info.target = BlockIdx::NULL;
                            recompile = true;
                            let updated_cache = op.cache_entries() as u32;
                            recompile |= updated_cache != old_cache_entries;
                            info.cache_entries = updated_cache;
                            let new_arg = OpArg::NULL;
                            recompile |= new_arg.instr_size() != old_arg_size;
                            info.arg = new_arg;
                        } else {
                            // Direction must be based on concrete instruction offsets.
                            // Empty blocks can share offsets, so block-order-based resolution
                            // may classify some jumps incorrectly.
                            op = match op.into() {
                                Opcode::JumpForward if target_offset < offset_after => {
                                    Opcode::JumpBackward.into()
                                }
                                Opcode::JumpBackward if target_offset >= offset_after => {
                                    Opcode::JumpForward.into()
                                }
                                Opcode::JumpBackwardNoInterrupt
                                    if target_offset >= offset_after =>
                                {
                                    Opcode::JumpForward.into()
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
        let locations = rustpython_compiler_core::marshal::linetable_to_locations(
            &linetable,
            first_line_number.get() as i32,
            instructions.len(),
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

    fn reorder_entry_prefix_cell_setup(&mut self) {
        let Some(entry) = self.blocks.first_mut() else {
            return;
        };
        let ncells = self.metadata.cellvars.len();
        let nfrees = self.metadata.freevars.len();
        if ncells == 0 && nfrees == 0 {
            return;
        }

        let prefix_len = entry
            .instructions
            .iter()
            .take_while(|info| {
                matches!(
                    info.instr.real(),
                    Some(Instruction::MakeCell { .. } | Instruction::CopyFreeVars { .. })
                )
            })
            .count();
        if prefix_len == 0 {
            return;
        }

        let original_prefix = entry.instructions[..prefix_len].to_vec();
        let anchor = original_prefix[0];
        let rest = entry.instructions.split_off(prefix_len);
        entry.instructions.clear();

        if nfrees > 0 {
            entry.instructions.push(InstructionInfo {
                instr: Instruction::CopyFreeVars { n: Arg::marker() }.into(),
                arg: OpArg::new(nfrees as u32),
                ..anchor
            });
        }

        let cellfixedoffsets = build_cellfixedoffsets(
            &self.metadata.varnames,
            &self.metadata.cellvars,
            &self.metadata.freevars,
        );
        let mut sorted = vec![None; self.metadata.varnames.len() + ncells];
        for (oldindex, fixed) in cellfixedoffsets.iter().copied().take(ncells).enumerate() {
            sorted[fixed as usize] = Some(oldindex);
        }
        for oldindex in sorted.into_iter().flatten() {
            entry.instructions.push(InstructionInfo {
                instr: Instruction::MakeCell { i: Arg::marker() }.into(),
                arg: OpArg::new(oldindex as u32),
                ..anchor
            });
        }

        entry.instructions.extend(rest);
    }

    /// Clear blocks that are unreachable (not entry, not a jump target,
    /// and only reachable via fall-through from a terminal block).
    fn eliminate_unreachable_blocks(&mut self) {
        self.eliminate_unreachable_blocks_impl(true);
    }

    fn eliminate_unreachable_blocks_without_exception_roots(&mut self) {
        self.eliminate_unreachable_blocks_impl(false);
    }

    fn eliminate_unreachable_blocks_impl(&mut self, root_exception_handlers: bool) {
        let mut reachable = vec![false; self.blocks.len()];
        reachable[0] = true;
        if root_exception_handlers {
            // Late Rust CFG cleanup can run after pseudo SETUP_* opcodes have
            // been lowered. At that point exception-handler blocks still need
            // to remain roots even when no concrete block-push instruction is
            // left to point at them.
            for (i, block) in self.blocks.iter().enumerate() {
                if block.except_handler {
                    reachable[i] = true;
                }
            }
        }
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
        let (const_idx, _) = metadata.consts.insert_full(folded_const);
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
        block.instructions[i].instr = Instruction::LoadConst {
            consti: Arg::marker(),
        }
        .into();
        block.instructions[i].arg = OpArg::new(const_idx as u32);
        block.instructions[i].folded_from_nonliteral_expr = false;
        true
    }

    /// Fold constant unary operations following CPython fold_const_unaryop().
    fn fold_unary_constants(&mut self) {
        for block in &mut self.blocks {
            let mut i = 0;
            while i < block.instructions.len() {
                if Self::fold_unary_constant_at(&mut self.metadata, block, i) {
                    i = i.saturating_sub(1);
                } else {
                    i += 1;
                }
            }
        }
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
            let load_instr = &block.instructions[j];
            if load_instr.folded_from_nonliteral_expr {
                return None;
            }
            elements.push(Self::get_const_value_from(metadata, load_instr)?);
        }

        Some((operand_indices, elements))
    }

    fn get_non_nop_instr_indices(block: &Block, start: usize, count: usize) -> Option<Vec<usize>> {
        let mut indices = Vec::with_capacity(count);
        for idx in start..block.instructions.len() {
            if !matches!(block.instructions[idx].instr.real(), Some(Instruction::Nop)) {
                indices.push(idx);
                if indices.len() == count {
                    return Some(indices);
                }
            }
        }
        None
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
        let (const_idx, _) = metadata.consts.insert_full(result_const);
        let folded_from_nonliteral_expr = operand_indices
            .iter()
            .any(|&idx| block.instructions[idx].folded_from_nonliteral_expr);
        for &idx in &operand_indices {
            nop_out_no_location(&mut block.instructions[idx]);
        }
        block.instructions[i].instr = Instruction::LoadConst {
            consti: Arg::marker(),
        }
        .into();
        block.instructions[i].arg = OpArg::new(const_idx as u32);
        block.instructions[i].folded_from_nonliteral_expr = folded_from_nonliteral_expr;
        true
    }

    /// Constant folding: fold LOAD_CONST/LOAD_SMALL_INT + LOAD_CONST/LOAD_SMALL_INT + BINARY_OP
    /// into a single LOAD_CONST when the result is computable at compile time.
    /// = fold_binops_on_constants in CPython flowgraph.c
    fn fold_binop_constants(&mut self) {
        for block in &mut self.blocks {
            let mut i = 0;
            while i < block.instructions.len() {
                if Self::fold_binop_constant_at(&mut self.metadata, block, i) {
                    i = i.saturating_sub(1);
                } else {
                    i += 1;
                }
            }
        }
    }

    /// CPython-style per-block forward walk that interleaves tuple, list, set,
    /// unary, and binop constant folding. Mirrors optimize_basic_block() in
    /// flowgraph.c so constants are registered in co_consts in instruction
    /// order rather than in the order separate global passes would discover
    /// them. Iterates per block to a fixed point so an inner fold can enable
    /// a surrounding outer fold within the same block.
    fn fold_constants_per_block(&mut self) {
        for block in &mut self.blocks {
            loop {
                let mut changed = false;
                let mut i = 0;
                while i < block.instructions.len() {
                    let folded = Self::fold_tuple_constant_at(&mut self.metadata, block, i)
                        || Self::fold_list_constant_at(&mut self.metadata, block, i)
                        || Self::fold_set_constant_at(&mut self.metadata, block, i)
                        || Self::fold_unary_constant_at(&mut self.metadata, block, i)
                        || Self::fold_binop_constant_at(&mut self.metadata, block, i);
                    if folded {
                        changed = true;
                    }
                    i += 1;
                }
                if !changed {
                    break;
                }
            }
        }
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

        let folded_from_nonliteral_expr = block.instructions[i].folded_from_nonliteral_expr;
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
        block.instructions[i].folded_from_nonliteral_expr = folded_from_nonliteral_expr;
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
                let mut folded_from_nonliteral_expr = false;
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
                        folded_from_nonliteral_expr |= instr.folded_from_nonliteral_expr;
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
                block.instructions[i].folded_from_nonliteral_expr = folded_from_nonliteral_expr;
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

    /// Constant folding: fold LOAD_CONST/LOAD_SMALL_INT + BUILD_TUPLE into LOAD_CONST tuple
    /// fold_tuple_of_constants.  This also folds constant list/set literals
    /// in block order to match CPython's optimize_basic_block() const-table order.
    fn fold_tuple_constants(&mut self) {
        for block in &mut self.blocks {
            let mut i = 0;
            while i < block.instructions.len() {
                if Self::fold_tuple_constant_at(&mut self.metadata, block, i)
                    || Self::fold_list_constant_at(&mut self.metadata, block, i)
                    || Self::fold_set_constant_at(&mut self.metadata, block, i)
                {
                    i += 1;
                    continue;
                }
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
                if list_size == 0 || list_size > STACK_USE_GUIDELINE {
                    i += 1;
                    continue;
                }

                let Some((operand_indices, elements)) =
                    Self::get_const_sequence(&self.metadata, block, i, list_size)
                else {
                    i += 1;
                    continue;
                };
                if list_size < MIN_CONST_SEQUENCE_SIZE {
                    i += 1;
                    continue;
                }

                let tuple_const = ConstantData::Tuple { elements };
                let (const_idx, _) = self.metadata.consts.insert_full(tuple_const);

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

                // NOP the rest
                for &j in &operand_indices[2..] {
                    set_to_nop(&mut block.instructions[j]);
                    block.instructions[j].location = folded_loc;
                }

                // slot[i] (was BUILD_LIST) → LIST_EXTEND 1
                block.instructions[i].instr = Opcode::ListExtend.into();
                block.instructions[i].arg = OpArg::new(1);

                i += 1;
            }
        }
    }

    /// Port of CPython's flowgraph.c optimize_lists_and_sets().
    ///
    /// For GET_ITER / CONTAINS_OP users:
    /// - Constant BUILD_LIST/BUILD_SET becomes LOAD_CONST tuple/frozenset.
    /// - Non-constant BUILD_LIST becomes BUILD_TUPLE.
    /// - Previously folded BUILD_LIST 0 + LOAD_CONST + LIST_EXTEND and
    ///   BUILD_SET 0 + LOAD_CONST + SET_UPDATE collapse back to LOAD_CONST.
    fn optimize_lists_and_sets(&mut self) {
        for block in &mut self.blocks {
            let mut i = 0;
            while i + 1 < block.instructions.len() {
                if matches!(
                    block.instructions[i].instr.real(),
                    Some(Instruction::CallIntrinsic1 { func })
                        if func.get(block.instructions[i].arg) == IntrinsicFunction1::ListToTuple
                ) {
                    if matches!(
                        block
                            .instructions
                            .get(i + 1)
                            .and_then(|instr| instr.instr.real()),
                        Some(Instruction::GetIter)
                    ) {
                        set_to_nop(&mut block.instructions[i]);
                        i += 2;
                        continue;
                    }
                    if Self::fold_constant_intrinsic_list_to_tuple_at(&mut self.metadata, block, i)
                    {
                        i += 1;
                        continue;
                    }
                }

                if let Some(non_nop4) = Self::get_non_nop_instr_indices(block, i, 4) {
                    let is_build_list = non_nop4[0] == i
                        && matches!(
                            block.instructions[non_nop4[0]].instr.real(),
                            Some(Instruction::BuildList { .. })
                        )
                        && u32::from(block.instructions[non_nop4[0]].arg) == 0;
                    let is_const = matches!(
                        block.instructions[non_nop4[1]].instr.real(),
                        Some(Instruction::LoadConst { .. })
                    );
                    let is_list_extend = matches!(
                        block.instructions[non_nop4[2]].instr.real(),
                        Some(Instruction::ListExtend { .. })
                    ) && u32::from(block.instructions[non_nop4[2]].arg) == 1;
                    let uses_iter_or_contains = matches!(
                        block.instructions[non_nop4[3]].instr.real(),
                        Some(Instruction::GetIter | Instruction::ContainsOp { .. })
                    );

                    if is_build_list && is_const && is_list_extend && uses_iter_or_contains {
                        let loc = block.instructions[i].location;
                        set_to_nop(&mut block.instructions[i]);
                        block.instructions[i].location = loc;
                        set_to_nop(&mut block.instructions[non_nop4[2]]);
                        block.instructions[non_nop4[2]].location = loc;
                        i += 1;
                        continue;
                    }

                    let is_build_set = non_nop4[0] == i
                        && matches!(
                            block.instructions[non_nop4[0]].instr.real(),
                            Some(Instruction::BuildSet { .. })
                        )
                        && u32::from(block.instructions[non_nop4[0]].arg) == 0;
                    let is_set_update = matches!(
                        block.instructions[non_nop4[2]].instr.real(),
                        Some(Instruction::SetUpdate { .. })
                    ) && u32::from(block.instructions[non_nop4[2]].arg) == 1;

                    if is_build_set && is_const && is_set_update && uses_iter_or_contains {
                        let loc = block.instructions[i].location;
                        set_to_nop(&mut block.instructions[i]);
                        block.instructions[i].location = loc;
                        set_to_nop(&mut block.instructions[non_nop4[2]]);
                        block.instructions[non_nop4[2]].location = loc;
                        i += 1;
                        continue;
                    }
                }

                let Some(non_nop2) = Self::get_non_nop_instr_indices(block, i, 2) else {
                    i += 1;
                    continue;
                };
                let uses_iter_or_contains = non_nop2[0] == i
                    && matches!(
                        block.instructions[non_nop2[1]].instr.real(),
                        Some(Instruction::GetIter | Instruction::ContainsOp { .. })
                    );
                if !uses_iter_or_contains {
                    i += 1;
                    continue;
                }

                if matches!(
                    block.instructions[i].instr.real(),
                    Some(Instruction::BuildList { .. })
                ) {
                    let seq_size = u32::from(block.instructions[i].arg) as usize;
                    if seq_size > STACK_USE_GUIDELINE {
                        i += 2;
                        continue;
                    }
                    if let Some((operand_indices, elements)) =
                        Self::get_const_sequence(&self.metadata, block, i, seq_size)
                    {
                        let const_data = ConstantData::Tuple { elements };
                        let (const_idx, _) = self.metadata.consts.insert_full(const_data);
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
                        i += 2;
                        continue;
                    }

                    block.instructions[i].instr = Opcode::BuildTuple.into();
                    i += 2;
                } else if matches!(
                    block.instructions[i].instr.real(),
                    Some(Instruction::BuildSet { .. })
                ) {
                    let seq_size = u32::from(block.instructions[i].arg) as usize;
                    if seq_size > STACK_USE_GUIDELINE {
                        i += 2;
                        continue;
                    }
                    let Some((operand_indices, elements)) =
                        Self::get_const_sequence(&self.metadata, block, i, seq_size)
                    else {
                        i += 2;
                        continue;
                    };
                    let const_data = ConstantData::Frozenset { elements };
                    let (const_idx, _) = self.metadata.consts.insert_full(const_data);
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
                    i += 2;
                } else {
                    i += 1;
                }
            }
        }
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
                if !(3..=STACK_USE_GUIDELINE).contains(&set_size) {
                    i += 1;
                    continue;
                }

                let Some((operand_indices, elements)) =
                    Self::get_const_sequence(&self.metadata, block, i, set_size)
                else {
                    i += 1;
                    continue;
                };
                let const_data = ConstantData::Frozenset { elements };
                let (const_idx, _) = self.metadata.consts.insert_full(const_data);

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
            let instructions = &mut block.instructions;
            let len = instructions.len();
            for i in 0..len.saturating_sub(1) {
                let Some(Instruction::BuildTuple { .. }) = instructions[i].instr.real() else {
                    continue;
                };
                let n = u32::from(instructions[i].arg);
                let Some(Instruction::UnpackSequence { .. }) = instructions[i + 1].instr.real()
                else {
                    continue;
                };
                if u32::from(instructions[i + 1].arg) != n {
                    continue;
                }
                match n {
                    1 => {
                        instructions[i].instr = Opcode::Nop.into();
                        instructions[i].arg = OpArg::new(0);
                        instructions[i + 1].instr = Opcode::Nop.into();
                        instructions[i + 1].arg = OpArg::new(0);
                    }
                    2 | 3 => {
                        instructions[i].instr = Opcode::Nop.into();
                        instructions[i].arg = OpArg::new(0);
                        instructions[i + 1].instr = Opcode::Swap.into();
                        instructions[i + 1].arg = OpArg::new(n);
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

        for block in &mut self.blocks {
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
    }

    /// Eliminate dead stores in STORE_FAST sequences (apply_static_swaps).
    ///
    /// In sequences of consecutive STORE_FAST instructions (from tuple unpacking),
    /// only collapse directly adjacent duplicate targets.
    ///
    /// CPython preserves non-adjacent duplicates such as `_, expr, _` so the
    /// store layout still reflects the original unpack order. Replacing the
    /// first `_` with POP_TOP there changes the emitted superinstructions and
    /// bytecode shape even though the final value is the same.
    fn eliminate_dead_stores(&mut self) {
        for block in &mut self.blocks {
            let instructions = &mut block.instructions;
            let len = instructions.len();
            let mut i = 0;
            while i < len {
                // Look for UNPACK_SEQUENCE or UNPACK_EX
                let is_unpack = matches!(
                    instructions[i].instr.into(),
                    AnyOpcode::Real(Opcode::UnpackSequence | Opcode::UnpackEx)
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
                        instructions[run_end].instr.into(),
                        AnyOpcode::Real(Opcode::StoreFast)
                    )
                {
                    run_end += 1;
                }
                if run_end - run_start >= 2 {
                    let mut j = run_start;
                    while j < run_end {
                        let arg = u32::from(instructions[j].arg);
                        let mut group_end = j + 1;
                        while group_end < run_end && u32::from(instructions[group_end].arg) == arg {
                            group_end += 1;
                        }
                        for instr in &mut instructions[j..group_end.saturating_sub(1)] {
                            instr.instr = Opcode::PopTop.into();
                            instr.arg = OpArg::new(0);
                        }
                        j = group_end;
                    }
                }
                i = run_end.max(i + 1);
            }

            // General same-line duplicate STORE_FAST elimination from
            // flowgraph.c optimize_basic_block(). This is required for
            // apply_static_swaps() patterns such as `a, a = x, y`.
            for i in 0..instructions.len().saturating_sub(1) {
                let lhs = &instructions[i];
                let rhs = &instructions[i + 1];
                if !matches!(lhs.instr.real(), Some(Instruction::StoreFast { .. }))
                    || !matches!(rhs.instr.real(), Some(Instruction::StoreFast { .. }))
                    || u32::from(lhs.arg) != u32::from(rhs.arg)
                    || instruction_lineno(lhs) != instruction_lineno(rhs)
                {
                    continue;
                }
                instructions[i].instr = Instruction::PopTop.into();
                instructions[i].arg = OpArg::NULL;
                instructions[i].target = BlockIdx::NULL;
            }
        }
    }

    /// Peephole optimization: combine consecutive instructions into super-instructions
    fn peephole_optimize(&mut self) {
        let const_truthiness =
            |instr: Instruction, arg: OpArg, metadata: &CodeUnitMetadata| match instr {
                Instruction::LoadConst { consti } => {
                    let constant = &metadata.consts[consti.get(arg).as_usize()];
                    Some(match constant {
                        ConstantData::Tuple { elements } => !elements.is_empty(),
                        ConstantData::Integer { value } => !value.is_zero(),
                        ConstantData::Float { value } => *value != 0.0,
                        ConstantData::Complex { value } => value.re != 0.0 || value.im != 0.0,
                        ConstantData::Boolean { value } => *value,
                        ConstantData::Str { value } => !value.is_empty(),
                        ConstantData::Bytes { value } => !value.is_empty(),
                        ConstantData::Code { .. } => true,
                        ConstantData::Slice { .. } => true,
                        ConstantData::Frozenset { elements } => !elements.is_empty(),
                        ConstantData::None => false,
                        ConstantData::Ellipsis => true,
                    })
                }
                Instruction::LoadSmallInt { i } => Some(i.get(arg) != 0),
                _ => None,
            };
        for (block_idx, block) in self.blocks.iter_mut().enumerate() {
            let mut i = 0;
            while i + 1 < block.instructions.len() {
                let curr = &block.instructions[i];
                let next = &block.instructions[i + 1];
                let curr_arg = curr.arg;
                let next_arg = next.arg;

                // Only combine if both are real instructions (not pseudo)
                let (Some(curr_instr), Some(next_instr)) = (curr.instr.real(), next.instr.real())
                else {
                    i += 1;
                    continue;
                };

                if matches!(curr_instr, Instruction::ToBool)
                    && matches!(next_instr, Instruction::UnaryNot)
                    && let Some(Instruction::ToBool) = block
                        .instructions
                        .get(i + 2)
                        .and_then(|info| info.instr.real())
                    && let Some(Instruction::UnaryNot) = block
                        .instructions
                        .get(i + 3)
                        .and_then(|info| info.instr.real())
                {
                    block.instructions.drain(i + 1..=i + 3);
                    continue;
                }

                if matches!(curr_instr, Instruction::UnaryNot | Instruction::ToBool)
                    && matches!(next_instr, Instruction::ToBool)
                {
                    block.instructions.remove(i + 1);
                    continue;
                }

                if matches!(
                    curr_instr,
                    Instruction::ContainsOp { .. } | Instruction::IsOp { .. }
                ) && matches!(next_instr, Instruction::UnaryNot)
                {
                    set_to_nop(&mut block.instructions[i]);
                    block.instructions[i + 1].instr = curr_instr.into();
                    block.instructions[i + 1].arg = OpArg::new(u32::from(curr_arg) ^ 1);
                    i += 1;
                    continue;
                }

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
                        let preserves_pure_self_loop_anchor = i == 0
                            && block.instructions[i + 1..].iter().all(|info| {
                                if info.target == BlockIdx(block_idx as u32)
                                    && info.instr.is_unconditional_jump()
                                {
                                    return true;
                                }
                                matches!(info.instr.real(), Some(Instruction::Nop))
                            })
                            && block.instructions[i + 1..].iter().any(|info| {
                                info.target == BlockIdx(block_idx as u32)
                                    && info.instr.is_unconditional_jump()
                            });
                        if preserves_pure_self_loop_anchor {
                            block.instructions[i].preserve_block_start_no_location_nop = true;
                        }
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
                ) && matches!(next_instr, Instruction::PopTop)
                {
                    set_to_nop(&mut block.instructions[i]);
                    set_to_nop(&mut block.instructions[i + 1]);
                    i += 1;
                    continue;
                }

                if matches!(curr_instr, Instruction::Copy { i } if i.get(curr.arg) == 1)
                    && matches!(next_instr, Instruction::PopTop)
                {
                    set_to_nop(&mut block.instructions[i]);
                    set_to_nop(&mut block.instructions[i + 1]);
                    i += 1;
                    continue;
                }

                let combined = {
                    match (curr_instr, next_instr) {
                        // Note: StoreFast + LoadFast → StoreFastLoadFast is done in a
                        // later pass aligned with CPython insert_superinstructions().
                        (
                            Instruction::LoadConst { .. } | Instruction::LoadSmallInt { .. },
                            Instruction::ToBool,
                        ) => {
                            if let Some(value) =
                                const_truthiness(curr_instr, curr.arg, &self.metadata)
                            {
                                let (const_idx, _) = self
                                    .metadata
                                    .consts
                                    .insert_full(ConstantData::Boolean { value });
                                Some((
                                    Instruction::LoadConst {
                                        consti: Arg::marker(),
                                    },
                                    OpArg::new(const_idx as u32),
                                ))
                            } else {
                                None
                            }
                        }
                        (Instruction::CompareOp { .. }, Instruction::ToBool) => Some((
                            curr_instr,
                            OpArg::new(u32::from(curr.arg) | oparg::COMPARE_OP_BOOL_MASK),
                        )),
                        (
                            Instruction::ContainsOp { .. } | Instruction::IsOp { .. },
                            Instruction::ToBool,
                        ) => Some((curr_instr, curr.arg)),
                        (Instruction::LoadConst { consti }, Instruction::UnaryNot) => {
                            let constant = &self.metadata.consts[consti.get(curr.arg).as_usize()];
                            match constant {
                                ConstantData::Boolean { value } => {
                                    let (const_idx, _) = self
                                        .metadata
                                        .consts
                                        .insert_full(ConstantData::Boolean { value: !value });
                                    Some(((Opcode::LoadConst.into()), OpArg::new(const_idx as u32)))
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

    fn remove_redundant_const_pop_top_pairs(&mut self) {
        loop {
            let mut changed = false;
            let mut prev: Option<(BlockIdx, usize)> = None;
            let mut block_idx = BlockIdx::new(0);

            while block_idx != BlockIdx::NULL {
                if self.blocks[block_idx.idx()].label {
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
            for block in &mut self.blocks {
                block.instructions.retain(|info| {
                    !matches!(info.instr.real(), Some(Instruction::Nop))
                        || instruction_lineno(info) >= 0
                        || info.preserve_redundant_jump_as_nop
                        || info.preserve_block_start_no_location_nop
                });
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
        let layout_predecessors = compute_layout_predecessors(&self.blocks);
        let keep_target_start_nops: Vec<_> = (0..self.blocks.len())
            .map(|idx| {
                keep_target_start_no_location_nop(
                    &self.blocks,
                    BlockIdx(idx as u32),
                    &layout_predecessors,
                )
            })
            .collect();
        let mut conditional_targets = vec![false; self.blocks.len()];
        for block in &self.blocks {
            for instr in &block.instructions {
                if instr.target != BlockIdx::NULL && is_conditional_jump(&instr.instr) {
                    let target = next_nonempty_block(&self.blocks, instr.target);
                    if target != BlockIdx::NULL {
                        conditional_targets[target.idx()] = true;
                    }
                }
            }
        }
        let preserve_loop_exit_pop_block_nops: Vec<_> = (0..self.blocks.len())
            .map(|idx| {
                let block_idx = BlockIdx(idx as u32);
                let block = &self.blocks[idx];
                let layout_pred = layout_predecessors[idx];
                block.instructions.first().is_some_and(|instr| {
                    matches!(instr.instr.real(), Some(Instruction::Nop))
                        && instr.remove_no_location_nop
                        && instruction_lineno(instr) < 0
                        && conditional_targets[idx]
                        && layout_pred != BlockIdx::NULL
                        && self.blocks[layout_pred.idx()]
                            .instructions
                            .last()
                            .is_some_and(|last| {
                                matches!(
                                    last.instr.real(),
                                    Some(
                                        Instruction::JumpBackward { .. }
                                            | Instruction::JumpBackwardNoInterrupt { .. }
                                    )
                                ) && next_nonempty_block(&self.blocks, last.target) != block_idx
                            })
                })
            })
            .collect();

        for (block_idx, block) in self.blocks.iter_mut().enumerate() {
            let mut prev_line = None;
            let mut src = 0usize;
            block.instructions.retain(|ins| {
                let keep = 'keep: {
                    if matches!(ins.instr.real(), Some(Instruction::Nop)) {
                        let keep_loop_exit_pop_block = src == 0
                            && preserve_loop_exit_pop_block_nops
                                .get(block_idx)
                                .copied()
                                .unwrap_or(false);
                        let keep_target_start = src == 0
                            && keep_target_start_nops
                                .get(block_idx)
                                .copied()
                                .unwrap_or(false);
                        if ins.remove_no_location_nop
                            && instruction_lineno(ins) < 0
                            && !keep_loop_exit_pop_block
                            && (!keep_target_start || ins.folded_operand_nop)
                        {
                            break 'keep false;
                        }
                        let line = ins.location.line.get() as i32;
                        if prev_line == Some(line) {
                            break 'keep false;
                        }
                    }
                    prev_line = Some(instruction_lineno(ins));
                    true
                };
                src += 1;
                keep
            });
        }
    }

    /// insert_superinstructions (flowgraph.c): combine adjacent same-line
    /// LOAD_FAST / STORE_FAST pairs before later flowgraph passes change
    /// block layout.
    fn insert_superinstructions(&mut self) {
        for block in &mut self.blocks {
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
        self.remove_nops();
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
            let expected = blocks[target.idx()].start_depth.map(|depth| depth as usize);
            if expected != Some(start_depth) {
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
                return;
            }
            if !visited[target.idx()] {
                visited[target.idx()] = true;
                worklist.push(target);
            }
        }

        fn folded_load_escapes_block(instructions: &[InstructionInfo], idx: usize) -> bool {
            if !instructions[idx].folded_from_nonliteral_expr {
                return false;
            }
            instructions[idx + 1..]
                .iter()
                .filter_map(|info| info.instr.real())
                .find(|instr| !matches!(instr, Instruction::Cache))
                .is_some_and(|instr| {
                    matches!(
                        instr,
                        Instruction::ReturnValue | Instruction::YieldValue { .. }
                    )
                })
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
                        if depth == 0 || refs.len() < depth {
                            continue;
                        }
                        let r = at_ref(&refs, refs.len() - depth);
                        push_ref(&mut refs, r.instr, r.local);
                    }
                    AnyInstruction::Real(Instruction::Swap { i: _ }) => {
                        let depth = arg_u32 as usize;
                        if depth < 2 || refs.len() < depth {
                            continue;
                        }
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
                        if target != BlockIdx::NULL {
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
                && block_has_fallthrough(block)
                && !block.disable_load_fast_borrow
                && !term.instr.is_unconditional_jump()
                && !term.instr.is_scope_exit()
            {
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
            if block.disable_load_fast_borrow {
                continue;
            }
            let folded_load_escapes: Vec<_> = (0..block.instructions.len())
                .map(|i| folded_load_escapes_block(&block.instructions, i))
                .collect();
            for (i, info) in block.instructions.iter_mut().enumerate() {
                if instr_flags[i] != 0 || folded_load_escapes[i] {
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

    fn compute_load_fast_start_depths(&mut self) {
        fn stackdepth_push(
            stack: &mut Vec<BlockIdx>,
            start_depths: &mut [u32],
            target: BlockIdx,
            depth: u32,
        ) {
            let idx = target.idx();
            let block_depth = &mut start_depths[idx];
            debug_assert!(
                *block_depth == u32::MAX || *block_depth == depth,
                "Invalid CFG, inconsistent optimize_load_fast stackdepth for block {:?}: existing={}, new={}",
                target,
                *block_depth,
                depth,
            );
            if *block_depth == u32::MAX {
                *block_depth = depth;
                stack.push(target);
            }
        }

        let mut stack = Vec::with_capacity(self.blocks.len());
        let mut start_depths = vec![u32::MAX; self.blocks.len()];
        stackdepth_push(&mut stack, &mut start_depths, BlockIdx(0), 0);

        'process_blocks: while let Some(block_idx) = stack.pop() {
            let mut depth = start_depths[block_idx.idx()];
            let block = &self.blocks[block_idx];
            for ins in &block.instructions {
                let instr = &ins.instr;
                let effect = instr.stack_effect(ins.arg.into());
                let new_depth = depth.saturating_add_signed(effect);
                if ins.target != BlockIdx::NULL {
                    let jump_effect = instr.stack_effect_jump(ins.arg.into());
                    let target_depth = depth.saturating_add_signed(jump_effect);
                    stackdepth_push(&mut stack, &mut start_depths, ins.target, target_depth);
                }
                depth = new_depth;
                if instr.is_scope_exit() || instr.is_unconditional_jump() {
                    continue 'process_blocks;
                }
            }
            if block.next != BlockIdx::NULL {
                stackdepth_push(&mut stack, &mut start_depths, block.next, depth);
            }
        }

        for (block, &start_depth) in self.blocks.iter_mut().zip(&start_depths) {
            block.start_depth = (start_depth != u32::MAX).then_some(start_depth);
        }
    }

    fn fast_scan_many_locals(
        &mut self,
        nlocals: usize,
        nparams: usize,
        merged_cell_local: &impl Fn(usize) -> Option<usize>,
    ) {
        const PARAM_INITIALIZED: usize = usize::MAX;

        debug_assert!(nlocals > 64);
        let mut states = vec![0usize; nlocals - 64];
        let high_params = nparams.saturating_sub(64).min(states.len());
        for state in states.iter_mut().take(high_params) {
            *state = PARAM_INITIALIZED;
        }

        let is_known = |idx: usize, state: usize, blocknum: usize| {
            state == blocknum || (idx < nparams && state == PARAM_INITIALIZED)
        };

        let mut blocknum = 0usize;
        let mut current = BlockIdx(0);
        while current != BlockIdx::NULL {
            blocknum += 1;
            let old_instructions = self.blocks[current.idx()].instructions.clone();
            let mut new_instructions = Vec::with_capacity(old_instructions.len());
            let mut changed = false;

            for mut info in old_instructions {
                match info.instr.real() {
                    Some(
                        Instruction::DeleteFast { var_num }
                        | Instruction::LoadFastAndClear { var_num },
                    ) => {
                        let idx = usize::from(var_num.get(info.arg));
                        if idx >= 64 && idx < nlocals {
                            states[idx - 64] = blocknum - 1;
                        }
                        new_instructions.push(info);
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
                        new_instructions.push(info);
                    }
                    Some(Instruction::DeleteDeref { i }) => {
                        let cell_relative = usize::from(i.get(info.arg));
                        if let Some(idx) = merged_cell_local(cell_relative)
                            && idx >= 64
                            && idx < nlocals
                        {
                            states[idx - 64] = blocknum - 1;
                        }
                        new_instructions.push(info);
                    }
                    Some(Instruction::StoreFast { var_num }) => {
                        let idx = usize::from(var_num.get(info.arg));
                        if idx >= 64 && idx < nlocals {
                            states[idx - 64] = blocknum;
                        }
                        new_instructions.push(info);
                    }
                    Some(Instruction::StoreDeref { i }) => {
                        let cell_relative = usize::from(i.get(info.arg));
                        if let Some(idx) = merged_cell_local(cell_relative)
                            && idx >= 64
                            && idx < nlocals
                        {
                            states[idx - 64] = blocknum;
                        }
                        new_instructions.push(info);
                    }
                    Some(Instruction::StoreFastStoreFast { var_nums }) => {
                        let packed = var_nums.get(info.arg);
                        let (idx1, idx2) = packed.indexes();
                        let idx1 = usize::from(idx1);
                        let idx2 = usize::from(idx2);
                        if idx1 >= 64 && idx1 < nlocals {
                            states[idx1 - 64] = blocknum;
                        }
                        if idx2 >= 64 && idx2 < nlocals {
                            states[idx2 - 64] = blocknum;
                        }
                        new_instructions.push(info);
                    }
                    Some(Instruction::StoreFastLoadFast { var_nums }) => {
                        let packed = var_nums.get(info.arg);
                        let (store_idx, load_idx) = packed.indexes();
                        let store_idx = usize::from(store_idx);
                        let load_idx = usize::from(load_idx);
                        if store_idx >= 64 && store_idx < nlocals {
                            states[store_idx - 64] = blocknum;
                        }
                        if load_idx >= 64 && load_idx < nlocals {
                            if !is_known(load_idx, states[load_idx - 64], blocknum) {
                                let mut first = info;
                                first.instr = Instruction::StoreFast {
                                    var_num: Arg::marker(),
                                }
                                .into();
                                first.arg = OpArg::new(store_idx as u32);

                                let mut second = info;
                                second.instr = Opcode::LoadFastCheck.into();
                                second.arg = OpArg::new(load_idx as u32);

                                new_instructions.push(first);
                                new_instructions.push(second);
                                changed = true;
                            } else {
                                new_instructions.push(info);
                            }
                        } else {
                            new_instructions.push(info);
                        }
                    }
                    Some(Instruction::LoadFast { var_num }) => {
                        let idx = usize::from(var_num.get(info.arg));
                        if idx >= 64 && idx < nlocals && !is_known(idx, states[idx - 64], blocknum)
                        {
                            info.instr = Opcode::LoadFastCheck.into();
                            states[idx - 64] = blocknum;
                            changed = true;
                        }
                        new_instructions.push(info);
                    }
                    Some(Instruction::LoadFastLoadFast { var_nums }) => {
                        let packed = var_nums.get(info.arg);
                        let (idx1, idx2) = packed.indexes();
                        let idx1 = usize::from(idx1);
                        let idx2 = usize::from(idx2);
                        let needs_check_1 = idx1 >= 64
                            && idx1 < nlocals
                            && !is_known(idx1, states[idx1 - 64], blocknum);
                        if needs_check_1 {
                            states[idx1 - 64] = blocknum;
                        }
                        let needs_check_2 = idx2 >= 64
                            && idx2 < nlocals
                            && !is_known(idx2, states[idx2 - 64], blocknum);

                        if needs_check_1 || needs_check_2 {
                            let mut first = info;
                            first.instr = if needs_check_1 {
                                Opcode::LoadFastCheck
                            } else {
                                Opcode::LoadFast
                            }
                            .into();
                            first.arg = OpArg::new(idx1 as u32);

                            let mut second = info;
                            second.instr = if needs_check_2 {
                                Opcode::LoadFastCheck.into()
                            } else {
                                Opcode::LoadFast.into()
                            };
                            second.arg = OpArg::new(idx2 as u32);

                            new_instructions.push(first);
                            new_instructions.push(second);
                            changed = true;
                            if needs_check_2 {
                                states[idx2 - 64] = blocknum;
                            }
                        } else {
                            new_instructions.push(info);
                        }
                    }
                    Some(Instruction::LoadFastCheck { var_num }) => {
                        let idx = usize::from(var_num.get(info.arg));
                        if idx >= 64 && idx < nlocals {
                            states[idx - 64] = blocknum;
                        }
                        new_instructions.push(info);
                    }
                    _ => new_instructions.push(info),
                }
            }

            if changed {
                self.blocks[current.idx()].instructions = new_instructions;
            }
            current = self.blocks[current.idx()].next;
        }
    }

    fn add_checks_for_loads_of_uninitialized_variables(&mut self) {
        let mut nlocals = self.metadata.varnames.len();
        if nlocals == 0 {
            return;
        }

        let cell_to_local: Vec<_> = self
            .metadata
            .cellvars
            .iter()
            .map(|name| self.metadata.varnames.get_index_of(name.as_str()))
            .collect();
        let merged_cell_local =
            |cell_relative: usize| cell_to_local.get(cell_relative).copied().flatten();

        let mut nparams = self.metadata.argcount as usize + self.metadata.kwonlyargcount as usize;
        if self.flags.contains(CodeFlags::VARARGS) {
            nparams += 1;
        }
        if self.flags.contains(CodeFlags::VARKEYWORDS) {
            nparams += 1;
        }
        nparams = nparams.min(nlocals);

        if nlocals > 64 {
            self.fast_scan_many_locals(nlocals, nparams, &merged_cell_local);
            nlocals = 64;
        }

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
                if matches!(info.instr.real(), Some(Instruction::ForIter { .. }))
                    && info.target != BlockIdx::NULL
                    && merge_unsafe_mask(&mut in_masks[info.target.idx()], &unsafe_mask)
                {
                    worklist.push(info.target);
                }
                match info.instr.real() {
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
                        let var_idx = var_num.get(info.arg) as usize;
                        if var_idx < nlocals {
                            unsafe_mask[var_idx] = true;
                        }
                        new_instructions.push(info);
                    }
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
                    Some(Instruction::StoreDeref { i }) => {
                        let cell_relative = usize::from(i.get(info.arg));
                        if let Some(var_idx) = merged_cell_local(cell_relative)
                            && var_idx < nlocals
                        {
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
                    Some(Instruction::DeleteDeref { i }) => {
                        let cell_relative = usize::from(i.get(info.arg));
                        if let Some(var_idx) = merged_cell_local(cell_relative)
                            && var_idx < nlocals
                        {
                            unsafe_mask[var_idx] = true;
                        }
                        new_instructions.push(info);
                    }
                    Some(
                        Instruction::LoadFast { var_num } | Instruction::LoadFastBorrow { var_num },
                    ) => {
                        let var_idx = usize::from(var_num.get(info.arg));
                        if var_idx < nlocals && unsafe_mask[var_idx] {
                            info.instr = Opcode::LoadFastCheck.into();
                            changed = true;
                        }
                        if var_idx < nlocals {
                            unsafe_mask[var_idx] = false;
                        }
                        new_instructions.push(info);
                    }
                    Some(
                        Instruction::LoadFastLoadFast { var_nums }
                        | Instruction::LoadFastBorrowLoadFastBorrow { var_nums },
                    ) => {
                        let packed = var_nums.get(info.arg);
                        let (idx1, idx2) = packed.indexes();
                        let idx1 = usize::from(idx1);
                        let idx2 = usize::from(idx2);
                        let needs_check_1 = idx1 < nlocals && unsafe_mask[idx1];
                        let needs_check_2 = idx2 < nlocals && unsafe_mask[idx2];
                        if needs_check_1 || needs_check_2 {
                            let mut first = info;
                            first.instr = if needs_check_1 {
                                Opcode::LoadFastCheck
                            } else {
                                Opcode::LoadFast
                            }
                            .into();
                            first.arg = OpArg::new(idx1 as u32);

                            let mut second = info;
                            second.instr = if needs_check_2 {
                                Opcode::LoadFastCheck.into()
                            } else {
                                Opcode::LoadFast.into()
                            };
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
                if new_depth > maxdepth {
                    maxdepth = new_depth
                }
                // Process target blocks for branching instructions
                if ins.target != BlockIdx::NULL {
                    let jump_effect = instr.stack_effect_jump(ins.arg.into());
                    let target_depth = depth.checked_add_signed(jump_effect).ok_or({
                        if jump_effect < 0 {
                            InternalError::StackUnderflow
                        } else {
                            InternalError::StackOverflow
                        }
                    })?;
                    if target_depth > maxdepth {
                        maxdepth = target_depth;
                    }
                    stackdepth_push(&mut stack, &mut start_depths, ins.target, target_depth);
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
        for block in &mut self.blocks {
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
                "block {} next={} cold={} except={} preserve_lasti={} disable_borrow={} start_depth={}",
                u32::from(block_idx),
                if block.next == BlockIdx::NULL {
                    String::from("NULL")
                } else {
                    u32::from(block.next).to_string()
                },
                block.cold,
                block.except_handler,
                block.preserve_lasti,
                block.disable_load_fast_borrow,
                block
                    .start_depth
                    .map_or_else(|| String::from("None"), |depth| depth.to_string()),
            );
            for info in &block.instructions {
                let lineno = instruction_lineno(info);
                let _ = writeln!(
                    out,
                    "  [disp={} raw={} override={:?}] {:?} arg={} target={}",
                    lineno,
                    info.location.line.get(),
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

        self.splice_annotations_blocks();
        if self.flags.contains(CodeFlags::OPTIMIZED) {
            split_blocks_at_jumps(&mut self.blocks);
            trace.push((
                "after_initial_split_blocks_at_jumps".to_owned(),
                self.debug_block_dump(),
            ));
        }
        self.fold_binop_constants();
        self.fold_unary_constants();
        self.fold_binop_constants();
        self.fold_unary_constants();
        self.fold_tuple_constants();
        self.fold_binop_constants();
        self.fold_list_constants();
        self.fold_set_constants();
        self.optimize_lists_and_sets();
        self.convert_to_load_small_int();
        self.remove_unused_consts();
        self.dce();
        self.optimize_build_tuple_unpack();
        self.eliminate_dead_stores();
        self.apply_static_swaps();
        self.peephole_optimize();
        trace.push((
            "after_peephole_optimize".to_owned(),
            self.debug_block_dump(),
        ));
        self.fold_tuple_constants();
        self.fold_binop_constants();
        self.fold_list_constants();
        self.fold_set_constants();
        self.optimize_lists_and_sets();
        self.convert_to_load_small_int();
        self.remove_unused_consts();
        split_blocks_at_jumps(&mut self.blocks);
        trace.push((
            "after_split_blocks_at_jumps".to_owned(),
            self.debug_block_dump(),
        ));
        self.dce();
        trace.push((
            "after_split_dce_unreachable".to_owned(),
            self.debug_block_dump(),
        ));
        mark_except_handlers(&mut self.blocks);
        label_exception_targets(&mut self.blocks);
        redirect_empty_unconditional_jump_targets(&mut self.blocks);
        inline_small_or_no_lineno_blocks(&mut self.blocks);
        trace.push((
            "after_inline_small_or_no_lineno_blocks".to_owned(),
            self.debug_block_dump(),
        ));
        jump_threading(&mut self.blocks);
        trace.push(("after_jump_threading".to_owned(), self.debug_block_dump()));
        self.eliminate_unreachable_blocks();
        resolve_line_numbers(&mut self.blocks);
        trace.push((
            "after_first_resolve_line_numbers".to_owned(),
            self.debug_block_dump(),
        ));
        self.remove_nops();
        trace.push((
            "after_early_remove_nops".to_owned(),
            self.debug_block_dump(),
        ));
        self.add_checks_for_loads_of_uninitialized_variables();
        self.insert_superinstructions();
        inline_single_predecessor_artificial_expr_exit_blocks(&mut self.blocks);
        push_cold_blocks_to_end(&mut self.blocks);
        trace.push((
            "after_push_cold_before_chain_reorder".to_owned(),
            self.debug_block_dump(),
        ));
        resolve_line_numbers(&mut self.blocks);
        trace.push((
            "after_push_cold_resolve_line_numbers".to_owned(),
            self.debug_block_dump(),
        ));
        reorder_conditional_chain_and_jump_back_blocks(&mut self.blocks);
        reorder_conditional_scope_exit_and_jump_back_blocks(&mut self.blocks, true, true);

        trace.push((
            "after_push_cold_blocks_to_end".to_owned(),
            self.debug_block_dump(),
        ));

        normalize_jumps(&mut self.blocks);
        trace.push(("after_normalize_jumps".to_owned(), self.debug_block_dump()));
        reorder_conditional_exit_and_jump_blocks(&mut self.blocks);
        reorder_conditional_jump_and_exit_blocks(&mut self.blocks);
        reorder_conditional_break_continue_blocks(&mut self.blocks);
        reorder_conditional_explicit_continue_scope_exit_blocks(&mut self.blocks);
        reorder_conditional_implicit_continue_scope_exit_blocks(&mut self.blocks);
        reorder_conditional_scope_exit_and_jump_back_blocks(&mut self.blocks, true, true);
        reorder_exception_handler_conditional_continue_scope_exit_blocks(&mut self.blocks);
        deduplicate_adjacent_jump_back_blocks(&mut self.blocks);
        reorder_conditional_body_and_implicit_continue_blocks(&mut self.blocks);
        reorder_conditional_scope_exit_and_jump_back_blocks(&mut self.blocks, true, true);
        reorder_conditional_scope_exit_and_jump_back_blocks(&mut self.blocks, false, true);
        reorder_conditional_scope_exit_and_jump_back_blocks(&mut self.blocks, false, true);
        reorder_conditional_scope_exit_and_jump_back_blocks(&mut self.blocks, false, false);
        trace.push(("after_reorder".to_owned(), self.debug_block_dump()));

        self.dce();
        self.eliminate_unreachable_blocks();
        trace.push(("after_dce_unreachable".to_owned(), self.debug_block_dump()));

        resolve_line_numbers(&mut self.blocks);
        trace.push((
            "after_resolve_line_numbers".to_owned(),
            self.debug_block_dump(),
        ));

        materialize_empty_conditional_exit_targets(&mut self.blocks);
        trace.push((
            "after_materialize_empty_conditional_exit_targets".to_owned(),
            self.debug_block_dump(),
        ));
        retarget_assert_conditional_jumps_to_empty_predecessor(&mut self.blocks);
        trace.push((
            "after_retarget_assert_conditional_jumps_to_empty_predecessor".to_owned(),
            self.debug_block_dump(),
        ));
        canonicalize_empty_label_blocks(&mut self.blocks);
        trace.push((
            "after_canonicalize_empty_label_blocks".to_owned(),
            self.debug_block_dump(),
        ));
        inline_small_fast_return_blocks(&mut self.blocks);
        trace.push((
            "after_inline_small_fast_return_blocks".to_owned(),
            self.debug_block_dump(),
        ));

        duplicate_end_returns(&mut self.blocks, &self.metadata);
        duplicate_fallthrough_jump_back_targets(&mut self.blocks);
        duplicate_shared_jump_back_targets(&mut self.blocks);
        trace.push((
            "after_duplicate_jump_back_targets".to_owned(),
            self.debug_block_dump(),
        ));

        self.dce();
        self.eliminate_unreachable_blocks();
        trace.push((
            "after_second_dce_unreachable".to_owned(),
            self.debug_block_dump(),
        ));

        resolve_line_numbers(&mut self.blocks);
        trace.push((
            "after_final_resolve_line_numbers".to_owned(),
            self.debug_block_dump(),
        ));

        self.remove_redundant_const_pop_top_pairs();
        remove_redundant_nops_and_jumps(&mut self.blocks);
        trace.push((
            "after_remove_redundant_nops_and_jumps".to_owned(),
            self.debug_block_dump(),
        ));

        jump_threading_unconditional(&mut self.blocks);
        self.eliminate_unreachable_blocks();
        remove_redundant_nops_and_jumps(&mut self.blocks);
        inline_with_suppress_return_blocks(&mut self.blocks);
        inline_pop_except_return_blocks(&mut self.blocks);
        duplicate_named_except_cleanup_returns(&mut self.blocks, &self.metadata);
        self.eliminate_unreachable_blocks();
        trace.push((
            "after_final_cfg_cleanup".to_owned(),
            self.debug_block_dump(),
        ));

        resolve_line_numbers(&mut self.blocks);
        reorder_lineful_jump_back_runs_by_descending_line(&mut self.blocks);
        trace.push((
            "after_post_cleanup_resolve_line_numbers".to_owned(),
            self.debug_block_dump(),
        ));

        let cellfixedoffsets = build_cellfixedoffsets(
            &self.metadata.varnames,
            &self.metadata.cellvars,
            &self.metadata.freevars,
        );
        mark_except_handlers(&mut self.blocks);
        let _ = self.max_stackdepth()?;
        convert_pseudo_ops(&mut self.blocks, &cellfixedoffsets);
        remove_redundant_nops_and_jumps(&mut self.blocks);
        inline_named_except_cleanup_jump_blocks(&mut self.blocks, &self.metadata);
        deduplicate_adjacent_pop_except_jump_back_blocks(&mut self.blocks);
        self.eliminate_unreachable_blocks();
        remove_redundant_nops_and_jumps(&mut self.blocks);
        // Keep debug_late_cfg_trace in the same order as finalize().
        redirect_load_fast_passthrough_targets(&mut self.blocks);
        trace.push((
            "after_convert_pseudo_ops".to_owned(),
            self.debug_block_dump(),
        ));
        self.compute_load_fast_start_depths();
        trace.push((
            "after_compute_load_fast_start_depths".to_owned(),
            self.debug_block_dump(),
        ));
        self.optimize_load_fast_borrow();
        redirect_empty_block_targets(&mut self.blocks);
        trace.push((
            "after_raw_optimize_load_fast_borrow".to_owned(),
            self.debug_block_dump(),
        ));
        self.apply_static_swaps();
        self.optimize_load_global_push_null();
        self.reorder_entry_prefix_cell_setup();
        self.remove_unused_consts();

        Ok(trace)
    }
}

impl CodeInfo {
    fn remap_block_idx(idx: BlockIdx, base: u32) -> BlockIdx {
        if idx == BlockIdx::NULL {
            idx
        } else {
            BlockIdx::new(u32::from(idx) + base)
        }
    }

    fn splice_annotations_blocks(&mut self) {
        let mut placeholder = None;
        for (block_idx, block) in self.blocks.iter().enumerate() {
            if let Some(instr_idx) = block.instructions.iter().position(|info| {
                matches!(
                    info.instr.pseudo(),
                    Some(PseudoInstruction::AnnotationsPlaceholder)
                )
            }) {
                placeholder = Some((block_idx, instr_idx));
                break;
            }
        }

        let Some((block_idx, instr_idx)) = placeholder else {
            return;
        };

        let Some(mut annotations_blocks) = self.annotations_blocks.take() else {
            self.blocks[block_idx].instructions.remove(instr_idx);
            return;
        };
        if annotations_blocks.is_empty() {
            self.blocks[block_idx].instructions.remove(instr_idx);
            return;
        }

        let base = self.blocks.len() as u32;
        for block in &mut annotations_blocks {
            block.next = Self::remap_block_idx(block.next, base);
            for info in &mut block.instructions {
                info.target = Self::remap_block_idx(info.target, base);
                if let Some(handler) = &mut info.except_handler {
                    handler.handler_block = Self::remap_block_idx(handler.handler_block, base);
                }
            }
        }

        let ann_entry = BlockIdx::new(base);
        let ann_tail = {
            let mut cursor = ann_entry;
            while annotations_blocks[(u32::from(cursor) - base) as usize].next != BlockIdx::NULL {
                cursor = annotations_blocks[(u32::from(cursor) - base) as usize].next;
            }
            cursor
        };

        let old_next = self.blocks[block_idx].next;
        let suffix = self.blocks[block_idx].instructions.split_off(instr_idx + 1);
        self.blocks[block_idx].instructions.pop();

        let suffix_block = if suffix.is_empty() {
            old_next
        } else {
            let suffix_idx = BlockIdx::new(base + annotations_blocks.len() as u32);
            let disable_load_fast_borrow = self.blocks[block_idx].disable_load_fast_borrow;
            let block = Block {
                instructions: suffix,
                next: old_next,
                disable_load_fast_borrow,
                ..Default::default()
            };
            annotations_blocks.push(block);
            suffix_idx
        };

        self.blocks[block_idx].next = ann_entry;
        let ann_tail_local = (u32::from(ann_tail) - base) as usize;
        annotations_blocks[ann_tail_local].next = suffix_block;
        self.blocks.extend(annotations_blocks);
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
    let instructions: Vec<&InstructionInfo> = iter_blocks(blocks)
        .flat_map(|(_, block)| block.instructions.iter())
        .collect();
    let same_handler = |left: ExceptHandlerInfo, right: ExceptHandlerInfo| {
        block_to_index[left.handler_block.idx()] == block_to_index[right.handler_block.idx()]
            && left.stack_depth == right.stack_depth
            && left.preserve_lasti == right.preserve_lasti
    };

    // Iterate through all instructions in block order
    // instr_index is the index into the final instructions array (including EXTENDED_ARG)
    // This matches how frame.rs uses lasti
    for (pos, instr) in instructions.iter().enumerate() {
        // CPython's final exception table is keyed by bytecode offsets after
        // empty cleanup labels have been resolved.  RustPython can still have
        // distinct block ids for those labels here, so compare handler offsets.
        let effective_except_handler = if instr.except_handler.is_none()
            && matches!(instr.instr.real(), Some(Instruction::NotTaken))
            && let Some((current_handler, _)) = current_entry
            && let Some(next) = instructions.get(pos + 1)
            && let Some(next_handler) = next.except_handler
            && same_handler(current_handler, next_handler)
            && !next.instr.is_scope_exit()
        {
            Some(current_handler)
        } else {
            instr.except_handler
        };

        // instr_size includes EXTENDED_ARG and CACHE entries
        let instr_size = instr.arg.instr_size() as u32 + instr.cache_entries;

        match (&current_entry, effective_except_handler) {
            // No current entry, no handler - nothing to do
            (None, None) => {}

            // No current entry, handler starts - begin new entry
            (None, Some(handler)) => {
                current_entry = Some((handler, instr_index));
            }

            // Current entry exists, same handler - continue
            (Some((curr_handler, _)), Some(handler)) if same_handler(*curr_handler, handler) => {}

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

/// flowgraph.c mark_cold (two-pass to match CPython).
///
/// Phase 1 (mark_warm): propagate "warm" from entry via forward edges
/// (fall-through and jump targets). Skip except_handler targets, matching
/// the practical effect of CPython's assertion in mark_warm.
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
            .is_none_or(|ins| !ins.instr.is_scope_exit() && !ins.instr.is_unconditional_jump());
        if has_fallthrough && block.next != BlockIdx::NULL {
            let next_idx = block.next.idx();
            if !blocks[next_idx].except_handler && !warm[next_idx] {
                warm[next_idx] = true;
                queue.push_back(block.next);
            }
        }

        for instr in &block.instructions {
            if instr.target != BlockIdx::NULL && !instr.instr.is_block_push() {
                let target_idx = instr.target.idx();
                if !blocks[target_idx].except_handler && !warm[target_idx] {
                    warm[target_idx] = true;
                    queue.push_back(instr.target);
                }
            }
        }
    }

    let mut cold = vec![false; n];
    let mut cold_visited = vec![false; n];
    let mut cold_queue: VecDeque<BlockIdx> = VecDeque::new();
    for (i, block) in blocks.iter().enumerate() {
        if block.except_handler {
            cold_queue.push_back(BlockIdx::new(i as u32));
            cold_visited[i] = true;
        }
    }
    while let Some(block_idx) = cold_queue.pop_front() {
        let idx = block_idx.idx();
        cold[idx] = true;
        let block = &blocks[idx];
        let has_fallthrough = block
            .instructions
            .last()
            .is_none_or(|ins| !ins.instr.is_scope_exit() && !ins.instr.is_unconditional_jump());
        if has_fallthrough && block.next != BlockIdx::NULL {
            let next_idx = block.next.idx();
            if !warm[next_idx] && !cold_visited[next_idx] {
                cold_visited[next_idx] = true;
                cold_queue.push_back(block.next);
            }
        }
        for instr in &block.instructions {
            if instr.target != BlockIdx::NULL && !instr.instr.is_block_push() {
                let target_idx = instr.target.idx();
                if !warm[target_idx] && !cold_visited[target_idx] {
                    cold_visited[target_idx] = true;
                    cold_queue.push_back(instr.target);
                }
            }
        }
    }

    for (i, block) in blocks.iter_mut().enumerate() {
        block.cold = cold[i];
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
                && block.instructions.last().is_none_or(|ins| {
                    !ins.instr.is_scope_exit() && !ins.instr.is_unconditional_jump()
                })
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
            instr: PseudoOpcode::JumpNoInterrupt.into(),
            arg: OpArg::new(0),
            target: warm_next,
            location: SourceLocation::default(),
            end_location: SourceLocation::default(),
            except_handler: None,
            folded_from_nonliteral_expr: false,
            lineno_override: Some(-1),
            cache_entries: 0,
            preserve_redundant_jump_as_nop: false,
            remove_no_location_nop: false,
            folded_operand_nop: false,
            no_location_exit: false,
            preserve_block_start_no_location_nop: false,
            match_success_jump: false,
            break_continue_cleanup_jump: false,
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
        remove_redundant_nops_and_jumps(blocks);
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
            let disable_load_fast_borrow = blocks[bi].disable_load_fast_borrow;
            blocks[bi].next = new_block_idx;
            blocks.push(Block {
                instructions: tail,
                next: old_next,
                cold,
                disable_load_fast_borrow,
                ..Block::default()
            });
            // Don't increment bi - re-check current block (it might still have issues)
        } else {
            bi += 1;
        }
    }
}

fn retarget_assert_conditional_jumps_to_empty_predecessor(blocks: &mut [Block]) {
    fn is_assertion_error_load(info: &InstructionInfo) -> bool {
        matches!(
            info.instr.real(),
            Some(Instruction::LoadCommonConstant { idx })
                if idx.get(info.arg) == oparg::CommonConstant::AssertionError
        )
    }

    fn is_plain_empty_label(block: &Block) -> bool {
        block.instructions.is_empty()
            && block.next != BlockIdx::NULL
            && !block.except_handler
            && !block.preserve_lasti
            && block.start_depth.is_none()
            && !block.cold
            && !block.disable_load_fast_borrow
            && !block.try_else_orelse_entry
            && !block.label
    }

    fn assertion_failure_start_line(block: &Block) -> Option<i32> {
        block
            .instructions
            .iter()
            .find(|info| is_assertion_error_load(info))
            .map(instruction_lineno)
    }

    let empty_predecessors: Vec<Option<BlockIdx>> = (0..blocks.len())
        .map(|target| {
            let target = BlockIdx::new(target as u32);
            blocks
                .iter()
                .enumerate()
                .find(|(_, block)| is_plain_empty_label(block) && block.next == target)
                .map(|(idx, _)| BlockIdx::new(idx as u32))
        })
        .collect();
    let assertion_lines: Vec<Option<i32>> =
        blocks.iter().map(assertion_failure_start_line).collect();

    for block in blocks {
        for instr in &mut block.instructions {
            if instr.target == BlockIdx::NULL || !is_conditional_jump(&instr.instr) {
                continue;
            }
            let target = instr.target;
            let Some(empty) = empty_predecessors[target.idx()] else {
                continue;
            };
            let Some(assert_line) = assertion_lines[target.idx()] else {
                continue;
            };
            if instruction_lineno(instr) != assert_line {
                instr.target = empty;
            }
        }
    }
}

fn canonicalize_empty_label_blocks(blocks: &mut [Block]) {
    fn is_assertion_error_load(info: &InstructionInfo) -> bool {
        matches!(
            info.instr.real(),
            Some(Instruction::LoadCommonConstant { idx })
                if idx.get(info.arg) == oparg::CommonConstant::AssertionError
        )
    }

    fn block_has_explicit_reference(blocks: &[Block], idx: BlockIdx) -> bool {
        blocks.iter().any(|block| {
            block.instructions.iter().any(|info| {
                info.target == idx
                    || info
                        .except_handler
                        .is_some_and(|handler| handler.handler_block == idx)
            })
        })
    }

    fn is_plain_empty_label(blocks: &[Block], idx: BlockIdx) -> bool {
        let block = &blocks[idx.idx()];
        block.instructions.is_empty()
            && block.next != BlockIdx::NULL
            && !block.except_handler
            && !block.preserve_lasti
            && block.start_depth.is_none()
            && !block.cold
            && !block.disable_load_fast_borrow
            && !block.try_else_orelse_entry
            && !block.load_fast_barrier
            && (!block.label
                || block.load_fast_passthrough
                || !block_has_explicit_reference(blocks, idx))
    }

    fn block_falls_through(block: &Block) -> bool {
        block
            .instructions
            .last()
            .is_some_and(|ins| !ins.instr.is_scope_exit() && !ins.instr.is_unconditional_jump())
    }

    fn has_fallthrough_predecessor_matching<F>(
        blocks: &[Block],
        idx: BlockIdx,
        mut matches_predecessor: F,
    ) -> bool
    where
        F: FnMut(&Block) -> bool,
    {
        let mut seen = vec![false; blocks.len()];
        let mut stack = vec![idx];
        while let Some(target) = stack.pop() {
            if target == BlockIdx::NULL {
                continue;
            }
            for (block_idx, block) in blocks.iter().enumerate() {
                if block.next != target {
                    continue;
                }
                if block_falls_through(block) {
                    if matches_predecessor(block) {
                        return true;
                    }
                } else if block.instructions.is_empty()
                    && block.load_fast_passthrough
                    && !seen[block_idx]
                {
                    seen[block_idx] = true;
                    stack.push(BlockIdx::new(block_idx as u32));
                }
            }
        }
        false
    }

    fn block_has_exception_setup_or_handler(block: &Block) -> bool {
        block_is_protected(block)
            || block.instructions.iter().any(|info| {
                matches!(
                    info.instr,
                    AnyInstruction::Pseudo(
                        PseudoInstruction::SetupFinally { .. }
                            | PseudoInstruction::SetupCleanup { .. }
                    )
                )
            })
    }

    fn block_has_fast_load(block: &Block) -> bool {
        block.instructions.iter().any(|info| {
            matches!(
                info.instr.real(),
                Some(
                    Instruction::LoadFast { .. }
                        | Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastLoadFast { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            )
        })
    }

    fn block_starts_with_make_closure_from_fast_loads(block: &Block) -> bool {
        let mut iter = block.instructions.iter().map(|info| info.instr.real());
        let mut fast_loads = 0;
        while matches!(
            iter.clone().next(),
            Some(Some(
                Instruction::LoadFast { .. }
                    | Instruction::LoadFastBorrow { .. }
                    | Instruction::LoadFastLoadFast { .. }
                    | Instruction::LoadFastBorrowLoadFastBorrow { .. }
            ))
        ) {
            fast_loads += 1;
            iter.next();
        }
        fast_loads > 0
            && matches!(iter.next(), Some(Some(Instruction::BuildTuple { .. })))
            && matches!(iter.next(), Some(Some(Instruction::LoadConst { .. })))
            && matches!(iter.next(), Some(Some(Instruction::MakeFunction)))
            && matches!(
                iter.next(),
                Some(Some(Instruction::SetFunctionAttribute { .. }))
            )
    }

    fn block_is_for_cleanup(block: &Block) -> bool {
        matches!(
            block.instructions.as_slice(),
            [end_for, pop_iter]
                if matches!(end_for.instr.real(), Some(Instruction::EndFor))
                    && matches!(pop_iter.instr.real(), Some(Instruction::PopIter))
        )
    }

    fn block_returns_call_with_fast_load(block: &Block) -> bool {
        let mut seen_fast_load = false;
        let mut previous_was_call_after_fast_load = false;
        for info in &block.instructions {
            match info.instr.real() {
                Some(
                    Instruction::LoadFast { .. }
                    | Instruction::LoadFastBorrow { .. }
                    | Instruction::LoadFastLoadFast { .. }
                    | Instruction::LoadFastBorrowLoadFastBorrow { .. },
                ) => {
                    seen_fast_load = true;
                    previous_was_call_after_fast_load = false;
                }
                Some(Instruction::Call { .. } | Instruction::CallKw { .. }) if seen_fast_load => {
                    previous_was_call_after_fast_load = true;
                }
                Some(Instruction::ReturnValue) if previous_was_call_after_fast_load => {
                    return true;
                }
                Some(_) => previous_was_call_after_fast_load = false,
                None => {}
            }
        }
        false
    }

    fn block_has_exception_match_handler(blocks: &[Block], block: &Block) -> bool {
        let mut seen = vec![false; blocks.len()];
        let mut stack: Vec<_> = block
            .instructions
            .iter()
            .filter_map(|info| info.except_handler.map(|handler| handler.handler_block))
            .collect();
        while let Some(idx) = stack.pop() {
            if idx == BlockIdx::NULL || seen[idx.idx()] {
                continue;
            }
            seen[idx.idx()] = true;
            let handler = &blocks[idx.idx()];
            if handler.instructions.iter().any(|info| {
                matches!(
                    info.instr.real(),
                    Some(Instruction::CheckExcMatch | Instruction::CheckEgMatch)
                )
            }) {
                return true;
            }
            for info in &handler.instructions {
                if info.target != BlockIdx::NULL {
                    stack.push(info.target);
                }
            }
            if block_has_fallthrough(handler) && handler.next != BlockIdx::NULL {
                stack.push(handler.next);
            }
        }
        false
    }

    fn block_is_calling_finally_cleanup(block: &Block) -> bool {
        let has_call = block.instructions.iter().any(|info| {
            matches!(
                info.instr.real(),
                Some(Instruction::Call { .. } | Instruction::CallKw { .. })
            )
        });
        has_call
            && block
                .instructions
                .iter()
                .any(|info| matches!(info.instr.real(), Some(Instruction::Reraise { .. })))
            && !block.instructions.iter().any(|info| {
                matches!(
                    info.instr.real(),
                    Some(Instruction::CheckExcMatch | Instruction::CheckEgMatch)
                )
            })
    }

    fn handler_chain_calls_finally_cleanup(blocks: &[Block], handler: BlockIdx) -> bool {
        let mut seen = vec![false; blocks.len()];
        let mut stack = vec![handler];
        while let Some(idx) = stack.pop() {
            if idx == BlockIdx::NULL || seen[idx.idx()] {
                continue;
            }
            seen[idx.idx()] = true;
            let block = &blocks[idx.idx()];
            if block_is_calling_finally_cleanup(block) {
                return true;
            }
            for info in &block.instructions {
                if info.target != BlockIdx::NULL {
                    stack.push(info.target);
                }
            }
            if block_has_fallthrough(block) && block.next != BlockIdx::NULL {
                stack.push(block.next);
            }
        }
        false
    }

    fn block_has_finally_cleanup_handler(blocks: &[Block], block: &Block) -> bool {
        block.instructions.iter().any(|info| {
            let setup_finally_handler = matches!(
                info.instr,
                AnyInstruction::Pseudo(PseudoInstruction::SetupFinally { .. })
            )
            .then_some(info.target);
            setup_finally_handler
                .into_iter()
                .chain(info.except_handler.map(|handler| handler.handler_block))
                .any(|handler| handler_chain_calls_finally_cleanup(blocks, handler))
        })
    }

    fn has_cpython_empty_load_fast_barrier(blocks: &[Block], idx: BlockIdx) -> bool {
        if !is_plain_empty_label(blocks, idx) {
            return false;
        }
        let target = next_nonempty_block(blocks, blocks[idx.idx()].next);
        if target == BlockIdx::NULL {
            return false;
        }
        let Some(assertion) = blocks[target.idx()]
            .instructions
            .iter()
            .find(|info| is_assertion_error_load(info))
        else {
            return false;
        };
        let assert_line = instruction_lineno(assertion);
        let mut has_jump_predecessor = false;
        for block in blocks {
            if block.instructions.iter().any(|info| {
                info.target == idx
                    && instruction_lineno(info) != assert_line
                    && is_conditional_jump(&info.instr)
            }) {
                has_jump_predecessor = true;
            }
        }
        has_jump_predecessor
    }

    fn has_cpython_empty_join_barrier(blocks: &[Block], idx: BlockIdx) -> bool {
        if !is_plain_empty_label(blocks, idx) {
            return false;
        }
        let target = next_nonempty_block(blocks, blocks[idx.idx()].next);
        if target != BlockIdx::NULL
            && blocks.iter().any(|block| {
                !block.cold
                    && !block.except_handler
                    && block.instructions.iter().any(|info| {
                        info.target == target
                            && matches!(
                                info.instr.real(),
                                Some(
                                    Instruction::JumpBackward { .. }
                                        | Instruction::JumpBackwardNoInterrupt { .. }
                                )
                            )
                    })
            })
        {
            return false;
        }
        let has_conditional_target_predecessor = blocks.iter().any(|block| {
            block
                .instructions
                .iter()
                .any(|info| info.target == idx && is_conditional_jump(&info.instr))
        });
        let has_fallthrough_predecessor = blocks
            .iter()
            .any(|block| block.next == idx && block_falls_through(block));
        has_conditional_target_predecessor && has_fallthrough_predecessor
    }

    fn has_cpython_empty_try_end_barrier(blocks: &[Block], idx: BlockIdx) -> bool {
        if !is_plain_empty_label(blocks, idx) {
            return false;
        }
        let target = next_nonempty_block(blocks, blocks[idx.idx()].next);
        if target == BlockIdx::NULL {
            return false;
        }
        let falls_through_from_finally_cleanup =
            has_fallthrough_predecessor_matching(blocks, idx, |block| {
                block_has_finally_cleanup_handler(blocks, block)
            });
        if blocks[idx.idx()].load_fast_passthrough && falls_through_from_finally_cleanup {
            return false;
        }
        if block_starts_with_make_closure_from_fast_loads(&blocks[target.idx()]) {
            // CPython codegen_make_closure() emits LOAD_CLOSURE/BUILD_TUPLE,
            // LOAD_CONST, MAKE_FUNCTION, SET_FUNCTION_ATTRIBUTE in the
            // continuation block.  A Rust-only empty try-end label before that
            // sequence must not stop optimize_load_fast() from visiting it.
            return false;
        }
        let handler_resumes_to_target = blocks.iter().any(|block| {
            (block.cold || block.except_handler)
                && block.instructions.iter().any(|info| {
                    (info.target == target || next_nonempty_block(blocks, info.target) == target)
                        && matches!(
                            info.instr.real(),
                            Some(
                                Instruction::JumpBackward { .. }
                                    | Instruction::JumpBackwardNoInterrupt { .. }
                                    | Instruction::JumpForward { .. }
                            )
                        )
                })
        });
        let normal_backedge_to_target = blocks.iter().any(|block| {
            !block.cold
                && !block.except_handler
                && block.instructions.iter().any(|info| {
                    info.target == target
                        && matches!(
                            info.instr.real(),
                            Some(
                                Instruction::JumpBackward { .. }
                                    | Instruction::JumpBackwardNoInterrupt { .. }
                            )
                        )
                })
        });
        if handler_resumes_to_target && normal_backedge_to_target {
            return false;
        }
        let falls_through_from_for_cleanup = blocks.iter().any(|block| {
            block.next == idx && block_falls_through(block) && block_is_for_cleanup(block)
        });
        if handler_resumes_to_target && falls_through_from_for_cleanup {
            return false;
        }
        if handler_resumes_to_target
            && block_has_fast_load(&blocks[target.idx()])
            && has_fallthrough_predecessor_matching(blocks, idx, |block| {
                !block.cold && !block.except_handler
            })
        {
            // CPython's codegen_try_except() can leave an empty USE_LABEL(end)
            // between the normal protected path and the following statement,
            // while a cold handler resumes directly to that following block.
            // flowgraph.c::optimize_load_fast() visits the empty end block and
            // stops because basicblock_last_instr() is NULL, so the following
            // block's LOAD_FAST instructions are not borrowed.
            return true;
        }
        if !handler_resumes_to_target
            && block_has_fast_load(&blocks[target.idx()])
            && has_fallthrough_predecessor_matching(blocks, idx, |block| {
                block_has_exception_setup_or_handler(block)
                    && block_has_exception_match_handler(blocks, block)
                    && !(handler_resumes_to_target
                        && block_returns_call_with_fast_load(&blocks[target.idx()]))
            })
        {
            return true;
        }
        if block_has_fast_load(&blocks[target.idx()])
            && has_fallthrough_predecessor_matching(blocks, idx, |block| {
                block_has_exception_setup_or_handler(block)
            })
        {
            // CPython optimize_load_fast() only pushes b_next when
            // basicblock_last_instr(block) is not NULL.  codegen_try_except()
            // can leave the normal try path falling through an empty end label,
            // including bare except handlers that have no CHECK_EXC_MATCH.
            return true;
        }
        let reaches_backedge = normal_region_reaches_backedge(blocks, target);
        has_fallthrough_predecessor_matching(blocks, idx, |block| {
            if reaches_backedge {
                block_has_exception_setup_or_handler(block)
                    || block_has_exception_match_handler(blocks, block)
            } else {
                block_has_finally_cleanup_handler(blocks, block)
                    && block_has_exception_match_handler(blocks, block)
                    && block_has_fast_load(&blocks[target.idx()])
            }
        })
    }

    fn block_tail_calls_with_fast_pair(block: &Block) -> bool {
        let mut seen_fast_pair = false;
        for info in &block.instructions {
            if matches!(
                info.instr.real(),
                Some(
                    Instruction::LoadFastLoadFast { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            ) {
                seen_fast_pair = true;
            } else if seen_fast_pair
                && matches!(
                    info.instr.real(),
                    Some(Instruction::Call { .. } | Instruction::CallKw { .. })
                )
            {
                return true;
            }
        }
        false
    }

    fn has_cpython_empty_while_break_call_barrier(blocks: &[Block], idx: BlockIdx) -> bool {
        if !is_plain_empty_label(blocks, idx) {
            return false;
        }
        let target = next_nonempty_block(blocks, blocks[idx.idx()].next);
        if target == BlockIdx::NULL || !block_tail_calls_with_fast_pair(&blocks[target.idx()]) {
            return false;
        }
        blocks.iter().any(|block| {
            if block.next != idx {
                return false;
            }
            if matches!(
                block.instructions.as_slice(),
                [
                    InstructionInfo {
                        instr: AnyInstruction::Real(Instruction::Nop),
                        ..
                    },
                    jump
                ] if jump.instr.is_unconditional_jump()
                    && jump.target != BlockIdx::NULL
                    && next_nonempty_block(blocks, jump.target) == target
            ) {
                return true;
            }
            block_falls_through(block)
                && block
                    .instructions
                    .last()
                    .is_some_and(|info| matches!(info.instr.real(), Some(Instruction::Nop)))
        })
    }

    fn has_cpython_empty_loop_entry_barrier(blocks: &[Block], idx: BlockIdx) -> bool {
        if !is_plain_empty_label(blocks, idx) {
            return false;
        }
        let target = next_nonempty_block(blocks, blocks[idx.idx()].next);
        if target == BlockIdx::NULL {
            return false;
        }
        let Some(target_first) = blocks[target.idx()].instructions.first() else {
            return false;
        };
        if !matches!(target_first.instr.real(), Some(Instruction::Nop)) {
            return false;
        }
        let target_line = instruction_lineno(target_first);
        if target_line <= 0 {
            return false;
        }
        let has_loop_backedge = blocks.iter().any(|block| {
            block.instructions.iter().any(|info| {
                info.target != BlockIdx::NULL
                    && next_nonempty_block(blocks, info.target) == target
                    && matches!(
                        info.instr.real(),
                        Some(
                            Instruction::JumpBackward { .. }
                                | Instruction::JumpBackwardNoInterrupt { .. }
                        )
                    )
            })
        });
        if !has_loop_backedge {
            return false;
        }
        blocks.iter().any(|block| {
            if block.next != idx {
                return false;
            }
            if matches!(
                block.instructions.as_slice(),
                [nop, jump] if matches!(nop.instr.real(), Some(Instruction::Nop))
                    && {
                        let line = instruction_lineno(nop);
                        line > 0 && line < target_line
                    }
                    && jump.instr.is_unconditional_jump()
                    && jump.target != BlockIdx::NULL
                    && next_nonempty_block(blocks, jump.target) == target
            ) {
                return true;
            }
            block_falls_through(block)
                && block.instructions.last().is_some_and(|info| {
                    matches!(info.instr.real(), Some(Instruction::Nop)) && {
                        let line = instruction_lineno(info);
                        line > 0 && line < target_line
                    }
                })
        })
    }

    fn has_cpython_empty_unreachable_fallthrough_barrier(blocks: &[Block], idx: BlockIdx) -> bool {
        if !is_plain_empty_label(blocks, idx) {
            return false;
        }
        let target = next_nonempty_block(blocks, blocks[idx.idx()].next);
        if target == BlockIdx::NULL || !block_has_fast_load(&blocks[target.idx()]) {
            return false;
        }
        blocks.iter().any(|block| {
            if block.next != idx {
                return false;
            }
            if block.instructions.last().is_some_and(|info| {
                info.instr.is_unconditional_jump()
                    && info.target != BlockIdx::NULL
                    && next_nonempty_block(blocks, info.target) == target
            }) {
                return true;
            }
            blocks[target.idx()].disable_load_fast_borrow
                && block_falls_through(block)
                && block
                    .instructions
                    .last()
                    .is_some_and(|info| matches!(info.instr.real(), Some(Instruction::Nop)))
        })
    }

    fn has_cpython_empty_assert_success_barrier(blocks: &[Block], idx: BlockIdx) -> bool {
        if !is_plain_empty_label(blocks, idx) {
            return false;
        }
        let target = next_nonempty_block(blocks, blocks[idx.idx()].next);
        if target == BlockIdx::NULL || !block_has_fast_load(&blocks[target.idx()]) {
            return false;
        }
        blocks.iter().any(|block| {
            let jumps_to_empty = block
                .instructions
                .last()
                .is_some_and(|info| info.target == idx && is_conditional_jump(&info.instr));
            if !jumps_to_empty || block.next == BlockIdx::NULL {
                return false;
            }
            blocks[block.next.idx()]
                .instructions
                .iter()
                .any(is_assertion_error_load)
        })
    }

    fn normal_region_reaches_backedge(blocks: &[Block], target: BlockIdx) -> bool {
        let mut stack = vec![target];
        let mut seen = vec![false; blocks.len()];
        while let Some(idx) = stack.pop() {
            if idx == BlockIdx::NULL || seen[idx.idx()] {
                continue;
            }
            seen[idx.idx()] = true;
            let block = &blocks[idx.idx()];
            if block.cold || block.except_handler {
                continue;
            }
            for info in &block.instructions {
                if matches!(
                    info.instr,
                    AnyInstruction::Pseudo(PseudoInstruction::SetupWith { .. })
                ) {
                    return false;
                }
                if matches!(
                    info.instr.real(),
                    Some(
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. }
                    )
                ) {
                    return true;
                }
                if info.target != BlockIdx::NULL && is_conditional_jump(&info.instr) {
                    stack.push(info.target);
                }
            }
            let last_real = block
                .instructions
                .iter()
                .rev()
                .find(|info| info.instr.real().is_some());
            if last_real.is_some_and(|info| {
                info.target != BlockIdx::NULL && info.instr.is_unconditional_jump()
            }) {
                stack.push(last_real.unwrap().target);
            } else if !last_real.is_some_and(|info| info.instr.is_scope_exit()) {
                stack.push(block.next);
            }
        }
        false
    }

    fn canonical_target(blocks: &[Block], mut idx: BlockIdx) -> BlockIdx {
        while idx != BlockIdx::NULL
            && is_plain_empty_label(blocks, idx)
            && ((blocks[idx.idx()].load_fast_passthrough
                && !has_cpython_empty_try_end_barrier(blocks, idx))
                || (!has_cpython_empty_load_fast_barrier(blocks, idx)
                    && !has_cpython_empty_join_barrier(blocks, idx)
                    && !has_cpython_empty_try_end_barrier(blocks, idx)
                    && !has_cpython_empty_while_break_call_barrier(blocks, idx)
                    && !has_cpython_empty_loop_entry_barrier(blocks, idx)
                    && !has_cpython_empty_unreachable_fallthrough_barrier(blocks, idx)
                    && !has_cpython_empty_assert_success_barrier(blocks, idx)))
        {
            idx = blocks[idx.idx()].next;
        }
        idx
    }

    let replacements: Vec<BlockIdx> = (0..blocks.len())
        .map(|idx| canonical_target(blocks, BlockIdx(idx as u32)))
        .collect();

    for block in blocks.iter_mut() {
        if block.next != BlockIdx::NULL {
            block.next = replacements[block.next.idx()];
        }
        for instr in &mut block.instructions {
            if instr.target != BlockIdx::NULL {
                instr.target = replacements[instr.target.idx()];
            }
            if let Some(handler) = &mut instr.except_handler
                && handler.handler_block != BlockIdx::NULL
            {
                handler.handler_block = replacements[handler.handler_block.idx()];
            }
        }
    }

    for idx in 1..blocks.len() {
        if replacements[idx] != BlockIdx(idx as u32) {
            blocks[idx].next = BlockIdx::NULL;
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

fn short_circuit_stub_conditional(block: &Block) -> Option<Instruction> {
    let cond_idx = trailing_conditional_jump_index(block)?;
    if cond_idx < 2 {
        return None;
    }
    let [first, second, ..] = block.instructions.as_slice() else {
        return None;
    };
    if !matches!(first.instr.real(), Some(Instruction::Copy { i }) if i.get(first.arg) == 1)
        || !matches!(second.instr.real(), Some(Instruction::ToBool))
    {
        return None;
    }

    let only_markers_between = block.instructions[2..cond_idx].iter().all(|info| {
        matches!(
            info.instr.real(),
            None | Some(Instruction::Nop | Instruction::NotTaken)
        )
    });
    if !only_markers_between {
        return None;
    }

    block.instructions[cond_idx].instr.real()
}

fn opposite_short_circuit_target(block: &Block, source: AnyInstruction) -> bool {
    let Some(conditional) = short_circuit_stub_conditional(block) else {
        return false;
    };
    matches!(
        (source.real(), Some(conditional)),
        (
            Some(Instruction::PopJumpIfFalse { .. }),
            Some(Instruction::PopJumpIfTrue { .. })
        ) | (
            Some(Instruction::PopJumpIfTrue { .. }),
            Some(Instruction::PopJumpIfFalse { .. })
        )
    )
}

fn same_short_circuit_target(block: &Block, source: AnyInstruction) -> Option<BlockIdx> {
    let conditional = short_circuit_stub_conditional(block)?;
    matches!(
        (source.real(), Some(conditional)),
        (
            Some(Instruction::PopJumpIfFalse { .. }),
            Some(Instruction::PopJumpIfFalse { .. })
        ) | (
            Some(Instruction::PopJumpIfTrue { .. }),
            Some(Instruction::PopJumpIfTrue { .. })
        )
    )
    .then_some(block.instructions[trailing_conditional_jump_index(block)?].target)
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

fn can_thread_conditional_through_forward_nointerrupt(
    source: AnyInstruction,
    target_pos: u32,
    final_target_pos: u32,
) -> bool {
    matches!(
        source.real(),
        Some(Instruction::PopJumpIfNone { .. } | Instruction::PopJumpIfNotNone { .. })
    ) && final_target_pos > target_pos
}

fn block_has_with_suppress_prefix(block: &Block, jump_idx: usize) -> bool {
    let tail: Vec<_> = block.instructions[..jump_idx]
        .iter()
        .filter_map(|info| info.instr.real())
        .rev()
        .take(5)
        .collect();
    matches!(
        tail.as_slice(),
        [
            Instruction::PopTop,
            Instruction::PopTop,
            Instruction::PopTop,
            Instruction::PopExcept,
            Instruction::PopTop,
        ]
    )
}

fn jump_threading_impl(blocks: &mut [Block], include_conditional: bool) {
    let mut changed = true;
    while changed {
        changed = false;
        let mut block_order = vec![u32::MAX; blocks.len()];
        let mut cursor = BlockIdx(0);
        let mut pos = 0u32;
        while cursor != BlockIdx::NULL {
            block_order[cursor.idx()] = pos;
            pos += 1;
            cursor = blocks[cursor.idx()].next;
        }
        for bi in 0..blocks.len() {
            let last_idx = match blocks[bi].instructions.len().checked_sub(1) {
                Some(i) => i,
                None => continue,
            };
            let ins = blocks[bi].instructions[last_idx];
            let mut target = ins.target;
            if target == BlockIdx::NULL {
                continue;
            }
            if !(ins.instr.is_unconditional_jump()
                || include_conditional && is_conditional_jump(&ins.instr))
            {
                continue;
            }
            target = next_nonempty_block(blocks, target);
            if target == BlockIdx::NULL {
                continue;
            }
            if include_conditional && is_conditional_jump(&ins.instr) {
                let next = next_nonempty_block(blocks, blocks[bi].next);
                let next_is_scope_exit = next != BlockIdx::NULL
                    && blocks[next.idx()]
                        .instructions
                        .last()
                        .is_some_and(|instr| instr.instr.is_scope_exit());
                if next_is_scope_exit {
                    let target_pos = block_order.get(target.idx()).copied().unwrap_or(u32::MAX);
                    let target_first_jump = blocks[target.idx()].instructions.first().copied();
                    let threads_match_success_jump_to_forward_nointerrupt =
                        matches!(ins.instr.real(), Some(Instruction::PopJumpIfNone { .. }))
                            && target_first_jump
                                .filter(|target_ins| target_ins.instr.is_unconditional_jump())
                                .filter(|target_ins| target_ins.target != BlockIdx::NULL)
                                .is_some_and(|target_ins| {
                                    let final_target_pos = block_order
                                        .get(target_ins.target.idx())
                                        .copied()
                                        .unwrap_or(u32::MAX);
                                    jump_thread_kind(target_ins.instr)
                                        == Some(JumpThreadKind::NoInterrupt)
                                        && target_ins.match_success_jump
                                        && final_target_pos > target_pos
                                });
                    let next_raises = blocks[next.idx()].instructions.iter().any(|instr| {
                        matches!(instr.instr.real(), Some(Instruction::RaiseVarargs { .. }))
                    });
                    let target_is_loop_backedge = blocks[target.idx()]
                        .instructions
                        .first()
                        .filter(|target_ins| target_ins.instr.is_unconditional_jump())
                        .map(|target_ins| next_nonempty_block(blocks, target_ins.target))
                        .is_some_and(|final_target| {
                            final_target == BlockIdx(bi as u32)
                                || comes_before(blocks, final_target, BlockIdx(bi as u32))
                        });
                    if !(threads_match_success_jump_to_forward_nointerrupt
                        || block_is_protected(&blocks[bi])
                            && next_raises
                            && target_is_loop_backedge)
                    {
                        continue;
                    }
                }
            }
            if include_conditional
                && is_conditional_jump(&ins.instr)
                && opposite_short_circuit_target(&blocks[target.idx()], ins.instr)
            {
                let final_target = next_nonempty_block(blocks, blocks[target.idx()].next);
                if final_target != BlockIdx::NULL && ins.target != final_target {
                    blocks[bi].instructions[last_idx].target = final_target;
                    changed = true;
                    continue;
                }
            }
            if include_conditional
                && is_conditional_jump(&ins.instr)
                && let Some(final_target) =
                    same_short_circuit_target(&blocks[target.idx()], ins.instr)
                && final_target != BlockIdx::NULL
                && ins.target != final_target
            {
                blocks[bi].instructions[last_idx].target = final_target;
                changed = true;
                continue;
            }
            if include_conditional && is_conditional_jump(&ins.instr) {
                let source_pos = block_order[bi];
                let target_pos = block_order.get(target.idx()).copied().unwrap_or(u32::MAX);
                if target_pos <= source_pos {
                    continue;
                }
            }
            // Match CPython's early flowgraph jump threading: inspect the
            // target block's first instruction only. A later unconditional-only
            // cleanup pass may thread through line-anchor NOPs introduced after
            // jump normalization.
            let target_jump = if include_conditional {
                blocks[target.idx()].instructions.first().copied()
            } else {
                blocks[target.idx()]
                    .instructions
                    .iter()
                    .find(|info| !matches!(info.instr.real(), Some(Instruction::Nop)))
                    .copied()
            };
            if let Some(target_ins) = target_jump
                && target_ins.instr.is_unconditional_jump()
                && target_ins.target != BlockIdx::NULL
                && target_ins.target != target
            {
                if !include_conditional
                    && blocks[target.idx()]
                        .instructions
                        .iter()
                        .take_while(|info| matches!(info.instr.real(), Some(Instruction::Nop)))
                        .any(instruction_has_lineno)
                {
                    continue;
                }
                let threads_with_suppress_exit = !include_conditional
                    && block_has_with_suppress_prefix(&blocks[bi], last_idx)
                    && blocks[target.idx()].instructions.len() == 1;
                if !include_conditional
                    && instruction_has_lineno(&target_ins)
                    && !threads_with_suppress_exit
                {
                    continue;
                }
                let source_pos = block_order[bi];
                let target_pos = block_order.get(target.idx()).copied().unwrap_or(u32::MAX);
                let final_target = target_ins.target;
                let final_target_pos = block_order
                    .get(final_target.idx())
                    .copied()
                    .unwrap_or(u32::MAX);
                let conditional = is_conditional_jump(&ins.instr);
                if !include_conditional && source_pos < target_pos && final_target_pos < target_pos
                {
                    // Keep the forward hop when threading would turn it into a
                    // backward edge. CPython preserves this shape for chained
                    // compare loop exits to avoid wraparound-style jumps.
                    continue;
                }
                if !include_conditional
                    && matches!(
                        jump_thread_kind(ins.instr),
                        Some(JumpThreadKind::NoInterrupt)
                    )
                    && matches!(
                        jump_thread_kind(target_ins.instr),
                        Some(JumpThreadKind::Plain)
                    )
                {
                    // CPython does not late-thread WITH suppress exits through
                    // the line-anchored continue/break jump that follows.
                    continue;
                }
                let Some(threaded_instr) = (if conditional {
                    match jump_thread_kind(target_ins.instr) {
                        Some(JumpThreadKind::Plain) => Some(ins.instr),
                        Some(JumpThreadKind::NoInterrupt)
                            if target_ins.match_success_jump
                                && can_thread_conditional_through_forward_nointerrupt(
                                    ins.instr,
                                    target_pos,
                                    final_target_pos,
                                ) =>
                        {
                            // A forward JUMP_NO_INTERRUPT assembles to the same
                            // JUMP_FORWARD opcode as CPython's plain match
                            // success jump. Limit this to None-check match
                            // success tests; boolean finally-cleanup jumps need
                            // the stronger no-interrupt shape for later CFG
                            // cleanup decisions.
                            Some(ins.instr)
                        }
                        _ => None,
                    }
                } else {
                    threaded_jump_instr(ins.instr, target_ins.instr, false)
                }) else {
                    continue;
                };
                if ins.target == final_target {
                    continue;
                }
                set_to_nop(&mut blocks[bi].instructions[last_idx]);
                let mut threaded = ins;
                threaded.instr = threaded_instr;
                threaded.arg = OpArg::new(0);
                threaded.target = final_target;
                threaded.location = target_ins.location;
                threaded.end_location = target_ins.end_location;
                threaded.cache_entries = 0;
                blocks[bi].instructions.push(threaded);
                changed = true;
            }
        }
        if include_conditional {
            break;
        }
    }
}

fn is_conditional_jump(instr: &AnyInstruction) -> bool {
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

fn is_false_path_conditional_jump(instr: &AnyInstruction) -> bool {
    matches!(
        instr.real().map(Into::into),
        Some(Opcode::PopJumpIfFalse | Opcode::PopJumpIfNone | Opcode::PopJumpIfNotNone)
    )
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

/// flowgraph.c normalize_jumps
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
                    instr: Opcode::NotTaken.into(),
                    arg: OpArg::new(0),
                    target: BlockIdx::NULL,
                    location: last_ins.location,
                    end_location: last_ins.end_location,
                    // CPython adds NOT_TAKEN in normalize_jumps(), after
                    // label_exception_targets(), so the synthetic instruction
                    // has no i_except edge.
                    except_handler: None,
                    folded_from_nonliteral_expr: false,
                    lineno_override: None,
                    cache_entries: 0,
                    preserve_redundant_jump_as_nop: false,
                    remove_no_location_nop: false,
                    folded_operand_nop: false,
                    no_location_exit: false,
                    preserve_block_start_no_location_nop: false,
                    match_success_jump: false,
                    break_continue_cleanup_jump: false,
                };
                blocks[idx].instructions.push(not_taken);
            } else {
                // Backward conditional jump: invert and create new block
                // Transform: `cond_jump T` (backward)
                // Into: `reversed_cond_jump b_next` + new block [NOT_TAKEN, JUMP T]
                let loc = last_ins.location;
                let end_loc = last_ins.end_location;

                if let Some(reversed) = reversed_conditional(&last_ins.instr) {
                    let old_next = blocks[idx].next;
                    let is_cold = blocks[idx].cold;
                    let disable_load_fast_borrow = blocks[idx].disable_load_fast_borrow;

                    // Create new block with NOT_TAKEN + JUMP to original backward target
                    let new_block_idx = BlockIdx(blocks.len() as u32);
                    let mut new_block = Block {
                        cold: is_cold,
                        disable_load_fast_borrow,
                        start_depth: blocks[target.idx()].start_depth,
                        ..Block::default()
                    };
                    new_block.instructions.push(InstructionInfo {
                        instr: Opcode::NotTaken.into(),
                        arg: OpArg::new(0),
                        target: BlockIdx::NULL,
                        location: loc,
                        end_location: end_loc,
                        // CPython creates this block in normalize_jumps(),
                        // after exception targets were labelled.
                        except_handler: None,
                        folded_from_nonliteral_expr: false,
                        lineno_override: None,
                        cache_entries: 0,
                        preserve_redundant_jump_as_nop: false,
                        remove_no_location_nop: false,
                        folded_operand_nop: false,
                        no_location_exit: false,
                        preserve_block_start_no_location_nop: false,
                        match_success_jump: false,
                        break_continue_cleanup_jump: false,
                    });
                    new_block.instructions.push(InstructionInfo {
                        instr: PseudoOpcode::Jump.into(),
                        arg: OpArg::new(0),
                        target,
                        location: loc,
                        end_location: end_loc,
                        // CPython's synthetic NOT_TAKEN/JUMP pair is not in
                        // an exception-table range.
                        except_handler: None,
                        folded_from_nonliteral_expr: false,
                        lineno_override: None,
                        cache_entries: 0,
                        preserve_redundant_jump_as_nop: false,
                        remove_no_location_nop: false,
                        folded_operand_nop: false,
                        no_location_exit: false,
                        preserve_block_start_no_location_nop: false,
                        match_success_jump: false,
                        break_continue_cleanup_jump: false,
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
            info.instr = match info.instr.into() {
                AnyOpcode::Pseudo(PseudoOpcode::Jump) => {
                    if target_pos > source_pos {
                        Opcode::JumpForward.into()
                    } else {
                        Opcode::JumpBackward.into()
                    }
                }
                AnyOpcode::Pseudo(PseudoOpcode::JumpNoInterrupt) => {
                    if target_pos > source_pos {
                        Opcode::JumpForward.into()
                    } else {
                        Opcode::JumpBackwardNoInterrupt.into()
                    }
                }
                _ => info.instr,
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

            let direct_target = last.target;
            let nonempty_target = next_nonempty_block(blocks, direct_target);
            let target = if blocks[direct_target.idx()].instructions.is_empty()
                && nonempty_target != BlockIdx::NULL
            {
                // CPython's cfg_builder_maybe_start_new_block() attaches a
                // label to the current empty block instead of leaving a
                // separate empty block in front of a small exit target. Treat
                // Rust-only empty labels the same way for this CPython
                // flowgraph.c::basicblock_inline_small_or_no_lineno_blocks()
                // transform.
                nonempty_target
            } else {
                direct_target
            };
            let small_exit_block = block_exits_scope(&blocks[target.idx()])
                && blocks[target.idx()].instructions.len() <= MAX_COPY_SIZE;
            let no_lineno_no_fallthrough = block_has_no_lineno(&blocks[target.idx()])
                && !block_has_fallthrough(&blocks[target.idx()]);
            if small_exit_block || no_lineno_no_fallthrough {
                let removed_jump_kind = jump_thread_kind(last.instr);
                if let Some(last_instr) = blocks[current.idx()].instructions.last_mut() {
                    set_to_nop(last_instr);
                }
                blocks[current.idx()]
                    .instructions
                    .extend(blocks[target.idx()].instructions.clone());
                if no_lineno_no_fallthrough
                    && removed_jump_kind == Some(JumpThreadKind::Plain)
                    && let Some(last) = blocks[current.idx()].instructions.last_mut()
                    && jump_thread_kind(last.instr) == Some(JumpThreadKind::NoInterrupt)
                {
                    last.instr = match last.instr.into() {
                        AnyOpcode::Pseudo(PseudoOpcode::JumpNoInterrupt) => {
                            PseudoOpcode::Jump.into()
                        }
                        AnyOpcode::Real(Opcode::JumpBackwardNoInterrupt) => {
                            Opcode::JumpBackward.into()
                        }
                        _ => last.instr,
                    };
                }
                changes = true;
            }

            current = next;
        }

        if !changes {
            break;
        }
    }
}

fn is_artificial_expr_stmt_exit_block(block: &Block) -> bool {
    matches!(
        block.instructions.as_slice(),
        [
            InstructionInfo {
                instr: AnyInstruction::Real(Instruction::PopTop),
                ..
            },
            InstructionInfo {
                instr: AnyInstruction::Real(Instruction::LoadConst { .. }),
                ..
            },
            InstructionInfo {
                instr: AnyInstruction::Real(Instruction::ReturnValue),
                ..
            }
        ]
    )
}

fn inline_single_predecessor_artificial_expr_exit_blocks(blocks: &mut [Block]) {
    let predecessors = compute_predecessors(blocks);

    for idx in 0..blocks.len() {
        let Some(last) = blocks[idx].instructions.last().copied() else {
            continue;
        };
        if !last.instr.is_unconditional_jump() || last.target == BlockIdx::NULL {
            continue;
        }

        let target = next_nonempty_block(blocks, last.target);
        if target == BlockIdx::NULL
            || predecessors[target.idx()] != 1
            || !is_artificial_expr_stmt_exit_block(&blocks[target.idx()])
        {
            continue;
        }

        let is_jump_wrapper = blocks[idx]
            .instructions
            .split_last()
            .is_some_and(|(_, prefix)| {
                prefix
                    .iter()
                    .all(|ins| matches!(ins.instr.real(), Some(Instruction::Nop)))
            });
        if is_jump_wrapper {
            continue;
        }

        if blocks[idx]
            .instructions
            .last()
            .is_some_and(instruction_has_lineno)
        {
            if let Some(last_instr) = blocks[idx].instructions.last_mut() {
                set_to_nop(last_instr);
            }
        } else {
            let _ = blocks[idx].instructions.pop();
        }
        blocks[idx]
            .instructions
            .extend(blocks[target.idx()].instructions.clone());
    }
}

struct TargetPredecessorFlags {
    plain_jump: Vec<bool>,
}

fn compute_target_predecessor_flags(blocks: &[Block]) -> TargetPredecessorFlags {
    let mut plain_jump = vec![false; blocks.len()];
    for block in blocks {
        for instr in &block.instructions {
            if instr.target == BlockIdx::NULL {
                continue;
            }
            let target = next_nonempty_block(blocks, instr.target);
            if target == BlockIdx::NULL {
                continue;
            }
            let idx = target.idx();
            if matches!(jump_thread_kind(instr.instr), Some(JumpThreadKind::Plain)) {
                plain_jump[idx] = true;
            }
        }
    }
    TargetPredecessorFlags { plain_jump }
}

fn compute_break_continue_cleanup_jump_target_lines(blocks: &[Block]) -> Vec<Vec<i32>> {
    let mut lines = vec![Vec::new(); blocks.len()];
    for block in blocks {
        for instr in &block.instructions {
            if instr.target == BlockIdx::NULL
                || !instr.break_continue_cleanup_jump
                || !matches!(jump_thread_kind(instr.instr), Some(JumpThreadKind::Plain))
            {
                continue;
            }
            let target = next_nonempty_block(blocks, instr.target);
            if target == BlockIdx::NULL {
                continue;
            }
            let lineno = if instr.lineno_override.is_some_and(|line| line < 0) {
                instr.location.line.get() as i32
            } else {
                instruction_lineno(instr)
            };
            if lineno > 0 {
                lines[target.idx()].push(lineno);
            }
        }
    }
    lines
}

fn remove_redundant_nops_in_blocks(blocks: &mut [Block]) -> usize {
    let mut changes = 0;
    let plain_jump_targets = compute_target_predecessor_flags(blocks).plain_jump;
    let break_continue_cleanup_jump_target_lines =
        compute_break_continue_cleanup_jump_target_lines(blocks);
    let layout_predecessors = compute_layout_predecessors(blocks);
    let mut block_order = Vec::new();
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        block_order.push(current);
        current = blocks[current.idx()].next;
    }
    for block_idx in block_order {
        let bi = block_idx.idx();
        let keep_target_start_nop =
            keep_target_start_no_location_nop(blocks, block_idx, &layout_predecessors);
        let follows_same_line_pop_iter =
            layout_predecessor_ends_with_pop_iter_on_line(blocks, block_idx, &layout_predecessors);
        let mut src_instructions = core::mem::take(&mut blocks[bi].instructions);
        let mut kept = Vec::with_capacity(src_instructions.len());
        let mut prev_lineno = -1i32;

        for src in 0..src_instructions.len() {
            let instr = src_instructions[src];
            let lineno = instruction_lineno(&instr);
            let mut remove = false;

            if matches!(instr.instr.real(), Some(Instruction::Nop)) {
                if instr.remove_no_location_nop && instr.folded_operand_nop {
                    remove = true;
                } else if instr.no_location_exit && instr.preserve_redundant_jump_as_nop {
                    remove = false;
                } else if src == 0
                    && lineno > 0
                    && ((!keep_target_start_nop && follows_same_line_pop_iter == Some(lineno))
                        || (instr.preserve_block_start_no_location_nop
                            && block_tail_starts_with_async_with_normal_exit(
                                &src_instructions[src + 1..],
                            )))
                {
                    remove = true;
                } else if src == 0
                    && lineno > 0
                    && plain_jump_targets[block_idx.idx()]
                    && !instr.preserve_redundant_jump_as_nop
                    && !instr.preserve_block_start_no_location_nop
                    && break_continue_cleanup_jump_target_lines[block_idx.idx()].contains(&lineno)
                {
                    // CPython basicblock_remove_redundant_nops() removes NOPs that
                    // only carry the same source line as neighboring control-flow.
                    // RustPython may hold the target block's instructions out of
                    // the block array while filtering, so keep incoming jump lines
                    // from the unmodified CFG.
                    let next_lineno = src_instructions[src + 1..].iter().find_map(|next_instr| {
                        let line = instruction_lineno(next_instr);
                        if matches!(next_instr.instr.real(), Some(Instruction::Nop)) {
                            None
                        } else {
                            Some(line)
                        }
                    });
                    if next_lineno.is_some_and(|next_lineno| next_lineno > lineno) {
                        remove = true;
                    }
                } else if instr.preserve_redundant_jump_as_nop
                    || instr.preserve_block_start_no_location_nop
                {
                    remove = false;
                } else if lineno < 0 {
                    remove = true;
                } else if instr.remove_no_location_nop
                    && src == 0
                    && plain_jump_targets[block_idx.idx()]
                    && instr.lineno_override.is_some()
                    && !keep_target_start_nop
                {
                    let next_lineno = src_instructions[src + 1..].iter().find_map(|next_instr| {
                        let line = instruction_lineno(next_instr);
                        if matches!(next_instr.instr.real(), Some(Instruction::Nop)) && line < 0 {
                            None
                        } else {
                            Some(line)
                        }
                    });
                    if next_lineno.is_some_and(|next_lineno| lineno < next_lineno) {
                        remove = true;
                    }
                } else if prev_lineno == lineno {
                    remove = true;
                } else if src < src_instructions.len() - 1 {
                    if src_instructions[src + 1].instr.is_block_push() {
                        remove = false;
                    } else if src_instructions[src + 1].instr.is_unconditional_jump()
                        && src_instructions[src + 1].target != block_idx
                    {
                        let next_lineno = instruction_lineno(&src_instructions[src + 1]);
                        if next_lineno == lineno || next_lineno < 0 {
                            src_instructions[src + 1].lineno_override = Some(lineno);
                            remove = true;
                        }
                    } else if src_instructions[src + 1].folded_from_nonliteral_expr {
                        remove = true;
                    } else {
                        let next_lineno = instruction_lineno(&src_instructions[src + 1]);
                        if next_lineno == lineno {
                            remove = true;
                        } else if next_lineno < 0 {
                            src_instructions[src + 1].lineno_override = Some(lineno);
                            remove = true;
                        }
                    }
                } else {
                    let next = next_nonempty_block(blocks, blocks[bi].next);
                    if next != BlockIdx::NULL {
                        let mut next_info = None;
                        for next_instr in &blocks[next.idx()].instructions {
                            let line = instruction_lineno(next_instr);
                            if matches!(next_instr.instr.real(), Some(Instruction::Nop)) && line < 0
                            {
                                continue;
                            }
                            next_info = Some(line);
                            break;
                        }
                        if let Some(next_lineno) = next_info {
                            // CPython basicblock_remove_redundant_nops()
                            // does not copy a block-ending NOP's line onto a
                            // no-location instruction in the next block.
                            if next_lineno == lineno {
                                remove = true;
                            }
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
        if next != BlockIdx::NULL {
            let Some(last_instr) = blocks[idx].instructions.last().copied() else {
                current = blocks[idx].next;
                continue;
            };
            if last_instr.instr.is_unconditional_jump()
                && last_instr.target != BlockIdx::NULL
                && next_nonempty_block(blocks, last_instr.target) == next
            {
                let preserve_redundant_jump_nop = if last_instr.preserve_redundant_jump_as_nop {
                    let line = instruction_lineno(&last_instr);
                    let next_line = blocks[next.idx()].instructions.iter().find_map(|instr| {
                        let line = instruction_lineno(instr);
                        (!matches!(instr.instr.real(), Some(Instruction::Nop)) || line >= 0)
                            .then_some(line)
                    });
                    line < 0
                        || line > 0
                            && !block_jump_follows_async_send_pop(&blocks[idx])
                            && !(block_jump_follows_with_normal_exit(&blocks[idx])
                                && block_tail_starts_with_async_with_normal_exit(
                                    &blocks[next.idx()].instructions,
                                ))
                            && next_line.is_some_and(|next_line| next_line < line)
                } else {
                    false
                };
                let last_instr = blocks[idx].instructions.last_mut().unwrap();
                let remove_no_location_nop = last_instr.remove_no_location_nop;
                let folded_operand_nop = last_instr.folded_operand_nop;
                let preserve_block_start_no_location_nop =
                    last_instr.preserve_block_start_no_location_nop;
                set_to_nop(last_instr);
                last_instr.preserve_redundant_jump_as_nop = preserve_redundant_jump_nop;
                last_instr.remove_no_location_nop = remove_no_location_nop;
                last_instr.folded_operand_nop = folded_operand_nop;
                last_instr.preserve_block_start_no_location_nop =
                    preserve_block_start_no_location_nop || preserve_redundant_jump_nop;
                changes += 1;
                current = blocks[idx].next;
                continue;
            }
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

fn redirect_empty_block_targets(blocks: &mut [Block]) {
    let redirected_targets: Vec<Vec<BlockIdx>> = blocks
        .iter()
        .map(|block| {
            block
                .instructions
                .iter()
                .map(|instr| {
                    if instr.target == BlockIdx::NULL {
                        BlockIdx::NULL
                    } else {
                        next_nonempty_block(blocks, instr.target)
                    }
                })
                .collect()
        })
        .collect();

    for (block, block_targets) in blocks.iter_mut().zip(redirected_targets) {
        for (instr, target) in block.instructions.iter_mut().zip(block_targets) {
            if target != BlockIdx::NULL {
                instr.target = target;
            }
        }
    }
}

fn redirect_load_fast_passthrough_targets(blocks: &mut [Block]) {
    fn is_assertion_error_load(info: &InstructionInfo) -> bool {
        matches!(
            info.instr.real(),
            Some(Instruction::LoadCommonConstant { idx })
                if idx.get(info.arg) == oparg::CommonConstant::AssertionError
        )
    }

    fn block_returns_call_with_fast_load(block: &Block) -> bool {
        let mut seen_fast_load = false;
        let mut previous_was_call_after_fast_load = false;
        for info in &block.instructions {
            match info.instr.real() {
                Some(
                    Instruction::LoadFast { .. }
                    | Instruction::LoadFastBorrow { .. }
                    | Instruction::LoadFastLoadFast { .. }
                    | Instruction::LoadFastBorrowLoadFastBorrow { .. },
                ) => {
                    seen_fast_load = true;
                    previous_was_call_after_fast_load = false;
                }
                Some(Instruction::Call { .. } | Instruction::CallKw { .. }) if seen_fast_load => {
                    previous_was_call_after_fast_load = true;
                }
                Some(Instruction::ReturnValue) if previous_was_call_after_fast_load => {
                    return true;
                }
                Some(_) => previous_was_call_after_fast_load = false,
                None => {}
            }
        }
        false
    }

    fn handler_resumes_to_target(blocks: &[Block], target: BlockIdx) -> bool {
        blocks.iter().any(|block| {
            (block.cold || block.except_handler)
                && block.instructions.iter().any(|info| {
                    info.target != BlockIdx::NULL
                        && (info.target == target
                            || next_nonempty_block(blocks, info.target) == target)
                        && matches!(
                            info.instr.real(),
                            Some(
                                Instruction::JumpBackward { .. }
                                    | Instruction::JumpBackwardNoInterrupt { .. }
                                    | Instruction::JumpForward { .. }
                            )
                        )
                })
        })
    }

    fn has_warm_fallthrough_predecessor(blocks: &[Block], target: BlockIdx) -> bool {
        let mut seen = vec![false; blocks.len()];
        let mut stack = vec![target];
        while let Some(target) = stack.pop() {
            if target == BlockIdx::NULL {
                continue;
            }
            for (block_idx, block) in blocks.iter().enumerate() {
                if block.next != target || block.cold || block.except_handler {
                    continue;
                }
                if block.instructions.is_empty()
                    && (block.load_fast_passthrough || block.load_fast_label_reuse_passthrough)
                {
                    if !seen[block_idx] {
                        seen[block_idx] = true;
                        stack.push(BlockIdx::new(block_idx as u32));
                    }
                    continue;
                }
                if block_has_fallthrough(block) {
                    return true;
                }
            }
        }
        false
    }

    fn assertion_success_nop_passthrough(blocks: &[Block], target: BlockIdx) -> bool {
        let block = &blocks[target.idx()];
        if !block.instructions.is_empty()
            || block.next == BlockIdx::NULL
            || block.except_handler
            || block.preserve_lasti
            || block.cold
            || block.disable_load_fast_borrow
        {
            return false;
        }
        // CPython's assertion success jump targets the following statement's
        // NOP block directly.  RustPython can synthesize an empty label before
        // that NOP while preserving assertion failure layout, so do not let
        // that synthetic label stop flowgraph.c::optimize_load_fast() parity.
        if !blocks[block.next.idx()]
            .instructions
            .first()
            .is_some_and(|info| matches!(info.instr.real(), Some(Instruction::Nop)))
        {
            return false;
        }
        let has_conditional_target = blocks.iter().any(|block| {
            block
                .instructions
                .iter()
                .any(|info| info.target == target && is_conditional_jump(&info.instr))
        });
        let has_assertion_failure_predecessor = blocks.iter().any(|block| {
            block.next == target
                && block.instructions.iter().any(is_assertion_error_load)
                && block
                    .instructions
                    .last()
                    .is_some_and(|info| info.instr.is_scope_exit())
        });
        has_conditional_target && has_assertion_failure_predecessor
    }

    fn assertion_failure_passthrough(blocks: &[Block], target: BlockIdx) -> bool {
        let block = &blocks[target.idx()];
        if !block.instructions.is_empty()
            || block.next == BlockIdx::NULL
            || block.except_handler
            || block.preserve_lasti
            || block.cold
            || block.disable_load_fast_borrow
            || blocks.iter().any(|predecessor| {
                predecessor.instructions.iter().any(|info| {
                    (info.target == target || info.target == block.next)
                        && is_conditional_jump(&info.instr)
                })
            })
        {
            return false;
        }
        if !blocks.iter().any(|predecessor| {
            predecessor.next == target
                && predecessor.instructions.last().is_some_and(|info| {
                    matches!(
                        info.instr.real(),
                        Some(Instruction::EndFor | Instruction::PopIter)
                    )
                })
        }) {
            return false;
        }
        // CPython codegen_assert() emits LOAD_COMMON_CONSTANT AssertionError
        // immediately after codegen_jump_if() for a failing assertion.  If
        // RustPython leaves an empty label before that failure block, do not
        // let it stop flowgraph.c::optimize_load_fast() from visiting the
        // AssertionError construction.
        blocks[block.next.idx()]
            .instructions
            .first()
            .is_some_and(is_assertion_error_load)
    }

    fn try_else_orelse_entry_passthrough(blocks: &[Block], target: BlockIdx) -> bool {
        let block = &blocks[target.idx()];
        if !block.instructions.is_empty()
            || block.next == BlockIdx::NULL
            || block.except_handler
            || block.preserve_lasti
            || block.cold
            || block.disable_load_fast_borrow
        {
            return false;
        }

        // CPython codegen_try_except() emits Try.orelse with VISIT_SEQ()
        // immediately after the normal try-body path.  RustPython can leave
        // an empty join block immediately before the marked orelse entry,
        // often from a conditional at the end of the try body.
        block.try_else_orelse_entry || blocks[block.next.idx()].try_else_orelse_entry
    }

    fn labeled_passthrough_successor(blocks: &[Block], target: BlockIdx) -> bool {
        let block = &blocks[target.idx()];
        if !block.instructions.is_empty()
            || block.next == BlockIdx::NULL
            || block.label
            || block.load_fast_barrier
            || block.except_handler
            || block.preserve_lasti
            || block.cold
            || block.disable_load_fast_borrow
        {
            return false;
        }

        // CPython flowgraph.c::cfg_builder_current_block_is_terminated()
        // attaches a pending label to the current empty block instead of
        // creating an intervening b_next block.  RustPython can leave that
        // unlabeled empty block immediately before the labeled continuation.
        blocks[block.next.idx()].load_fast_label_reuse_passthrough
    }

    fn passthrough_target(blocks: &[Block], mut target: BlockIdx) -> BlockIdx {
        while target != BlockIdx::NULL {
            let block = &blocks[target.idx()];
            let next = next_nonempty_block(blocks, block.next);
            let handler_resume_end = block.instructions.is_empty()
                && next != BlockIdx::NULL
                && handler_resumes_to_target(blocks, next)
                && block_returns_call_with_fast_load(&blocks[next.idx()]);
            let handler_resume_end =
                handler_resume_end && !has_warm_fallthrough_predecessor(blocks, target);
            let assertion_success_nop = assertion_success_nop_passthrough(blocks, target);
            let assertion_failure = assertion_failure_passthrough(blocks, target);
            let try_else_orelse_entry = try_else_orelse_entry_passthrough(blocks, target);
            let labeled_passthrough_successor = labeled_passthrough_successor(blocks, target);
            if !(block.load_fast_passthrough
                || handler_resume_end
                || assertion_success_nop
                || assertion_failure
                || try_else_orelse_entry
                || labeled_passthrough_successor)
                || !block.instructions.is_empty()
            {
                break;
            }
            target = block.next;
        }
        target
    }

    let redirected_next: Vec<_> = blocks
        .iter()
        .map(|block| passthrough_target(blocks, block.next))
        .collect();
    let redirected_targets: Vec<Vec<BlockIdx>> = blocks
        .iter()
        .map(|block| {
            block
                .instructions
                .iter()
                .map(|instr| passthrough_target(blocks, instr.target))
                .collect()
        })
        .collect();

    for ((block, next), block_targets) in blocks
        .iter_mut()
        .zip(redirected_next)
        .zip(redirected_targets)
    {
        block.next = next;
        for (instr, target) in block.instructions.iter_mut().zip(block_targets) {
            instr.target = target;
        }
    }
}

fn redirect_empty_unconditional_jump_targets(blocks: &mut [Block]) {
    const MAX_COPY_SIZE: usize = 4;

    let block_exits_to_large_reraise = |block_idx: BlockIdx| {
        let block = &blocks[block_idx.idx()];
        let Some(last) = block.instructions.last() else {
            return false;
        };
        let reraise_block = if matches!(last.instr.real(), Some(Instruction::Reraise { .. })) {
            block_idx
        } else if last.instr.is_unconditional_jump() && last.target != BlockIdx::NULL {
            next_nonempty_block(blocks, last.target)
        } else {
            BlockIdx::NULL
        };
        reraise_block != BlockIdx::NULL
            && blocks[reraise_block.idx()].instructions.len() > MAX_COPY_SIZE
            && blocks[reraise_block.idx()]
                .instructions
                .last()
                .is_some_and(|instr| {
                    matches!(instr.instr.real(), Some(Instruction::Reraise { .. }))
                })
    };

    let mut raw_predecessors = vec![0u32; blocks.len()];
    for block in blocks.iter() {
        if block_has_fallthrough(block) && block.next != BlockIdx::NULL {
            raw_predecessors[block.next.idx()] += 1;
        }
        for instr in &block.instructions {
            if instr.target != BlockIdx::NULL {
                raw_predecessors[instr.target.idx()] += 1;
            }
        }
    }

    let redirected_targets: Vec<Vec<BlockIdx>> = blocks
        .iter()
        .map(|block| {
            block
                .instructions
                .iter()
                .map(|instr| {
                    if instr.target == BlockIdx::NULL
                        || !instr.instr.is_unconditional_jump()
                        || blocks[instr.target.idx()].load_fast_barrier
                    {
                        instr.target
                    } else {
                        if blocks[instr.target.idx()].instructions.is_empty()
                            && raw_predecessors[instr.target.idx()] > 1
                            && {
                                let target = next_nonempty_block(blocks, instr.target);
                                target != BlockIdx::NULL && block_exits_to_large_reraise(target)
                            }
                        {
                            return instr.target;
                        }
                        let target = next_nonempty_block(blocks, instr.target);
                        if matches!(
                            jump_thread_kind(instr.instr),
                            Some(JumpThreadKind::NoInterrupt)
                        ) && (target == BlockIdx::NULL
                            || is_jump_back_only_block(blocks, target))
                        {
                            instr.target
                        } else {
                            target
                        }
                    }
                })
                .collect()
        })
        .collect();

    for (block, block_targets) in blocks.iter_mut().zip(redirected_targets) {
        for (instr, target) in block.instructions.iter_mut().zip(block_targets) {
            if target != BlockIdx::NULL {
                instr.target = target;
            }
        }
    }
}

fn materialize_empty_conditional_exit_targets(blocks: &mut [Block]) {
    fn block_starts_with_with_normal_exit(block: &Block) -> bool {
        matches!(
            block.instructions.as_slice(),
            [
                InstructionInfo {
                    instr: AnyInstruction::Real(Instruction::LoadConst { .. }),
                    ..
                },
                InstructionInfo {
                    instr: AnyInstruction::Real(Instruction::LoadConst { .. }),
                    ..
                },
                InstructionInfo {
                    instr: AnyInstruction::Real(Instruction::LoadConst { .. }),
                    ..
                },
                InstructionInfo {
                    instr: AnyInstruction::Real(Instruction::Call { .. }),
                    ..
                },
                InstructionInfo {
                    instr: AnyInstruction::Real(Instruction::PopTop),
                    ..
                },
                ..
            ]
        )
    }

    fn with_normal_exit_is_followed_by_try(blocks: &[Block], block_idx: BlockIdx) -> bool {
        if block_idx == BlockIdx::NULL
            || !block_starts_with_with_normal_exit(&blocks[block_idx.idx()])
        {
            return false;
        }
        let next = next_nonempty_block(blocks, blocks[block_idx.idx()].next);
        next != BlockIdx::NULL
            && blocks[next.idx()].instructions.first().is_some_and(|info| {
                matches!(
                    info.instr,
                    AnyInstruction::Pseudo(PseudoInstruction::SetupFinally { .. })
                        | AnyInstruction::Real(Instruction::Nop)
                )
            })
    }

    fn has_loop_backedge_to(blocks: &[Block], target: BlockIdx) -> bool {
        blocks.iter().enumerate().any(|(source_idx, block)| {
            let source = BlockIdx(source_idx as u32);
            comes_before(blocks, target, source)
                && block.instructions.iter().any(|info| {
                    info.instr.is_unconditional_jump()
                        && info.target != BlockIdx::NULL
                        && next_nonempty_block(blocks, info.target) == target
                })
        })
    }

    let mut jump_back_inserts = Vec::new();
    let mut inserts = Vec::new();
    let mut target_start_inserts = Vec::new();
    let mut jump_back_target_locations = Vec::new();
    for (block_idx, block) in blocks.iter().enumerate() {
        let source = BlockIdx(block_idx as u32);
        let (last, allow_scope_exit_target) = if let Some(last) = block
            .instructions
            .last()
            .filter(|info| is_conditional_jump(&info.instr))
        {
            (last, true)
        } else if let Some(cond_idx) = trailing_conditional_jump_index(block) {
            (&block.instructions[cond_idx], false)
        } else {
            continue;
        };
        if last.target == BlockIdx::NULL {
            continue;
        }
        let target = last.target;
        if !blocks[target.idx()].instructions.is_empty() {
            if is_jump_back_only_block(blocks, target) && block_has_no_lineno(&blocks[target.idx()])
            {
                jump_back_target_locations.push((*last, target));
            }
            if with_normal_exit_is_followed_by_try(blocks, target)
                && has_loop_backedge_to(blocks, source)
                && !matches!(
                    blocks[target.idx()]
                        .instructions
                        .first()
                        .and_then(|info| info.instr.real()),
                    Some(Instruction::Nop)
                )
            {
                target_start_inserts.push((*last, target));
            }
            continue;
        }
        let next = next_nonempty_block(blocks, blocks[target.idx()].next);
        if next != BlockIdx::NULL
            && is_jump_only_block(&blocks[next.idx()])
            && block_has_no_lineno(&blocks[next.idx()])
            && comes_before(
                blocks,
                next_nonempty_block(blocks, blocks[next.idx()].instructions[0].target),
                next,
            )
        {
            jump_back_inserts.push((BlockIdx(block_idx as u32), target, next));
            continue;
        }
        if next == BlockIdx::NULL
            || !((allow_scope_exit_target && is_scope_exit_block(&blocks[next.idx()]))
                || (with_normal_exit_is_followed_by_try(blocks, next)
                    && has_loop_backedge_to(blocks, source)))
        {
            continue;
        }
        inserts.push((*last, target));
    }

    for (source, target) in jump_back_target_locations {
        if !is_jump_back_only_block(blocks, target) || !block_has_no_lineno(&blocks[target.idx()]) {
            continue;
        }
        if let Some(first) = blocks[target.idx()].instructions.first_mut() {
            overwrite_location(first, source.location, source.end_location);
        }
    }

    for (source, target, next) in jump_back_inserts {
        if !blocks[target.idx()].instructions.is_empty() {
            continue;
        }
        let Some(last) = blocks[source.idx()].instructions.last().copied() else {
            continue;
        };
        let mut cloned = blocks[next.idx()].instructions[0];
        overwrite_location(&mut cloned, last.location, last.end_location);
        blocks[target.idx()].instructions.push(cloned);
    }

    for (source, target) in inserts {
        if !blocks[target.idx()].instructions.is_empty() {
            continue;
        }
        blocks[target.idx()].instructions.push(InstructionInfo {
            instr: Instruction::Nop.into(),
            arg: OpArg::NULL,
            target: BlockIdx::NULL,
            location: source.location,
            end_location: source.end_location,
            except_handler: None,
            folded_from_nonliteral_expr: false,
            lineno_override: None,
            cache_entries: 0,
            preserve_redundant_jump_as_nop: false,
            remove_no_location_nop: false,
            folded_operand_nop: false,
            no_location_exit: false,
            preserve_block_start_no_location_nop: false,
            match_success_jump: false,
            break_continue_cleanup_jump: false,
        });
    }

    for (source, target) in target_start_inserts.into_iter().rev() {
        if !with_normal_exit_is_followed_by_try(blocks, target)
            || matches!(
                blocks[target.idx()]
                    .instructions
                    .first()
                    .and_then(|info| info.instr.real()),
                Some(Instruction::Nop)
            )
        {
            continue;
        }
        blocks[target.idx()].instructions.insert(
            0,
            InstructionInfo {
                instr: Instruction::Nop.into(),
                arg: OpArg::NULL,
                target: BlockIdx::NULL,
                location: source.location,
                end_location: source.end_location,
                except_handler: None,
                folded_from_nonliteral_expr: false,
                lineno_override: None,
                cache_entries: 0,
                preserve_redundant_jump_as_nop: false,
                remove_no_location_nop: false,
                folded_operand_nop: false,
                no_location_exit: false,
                preserve_block_start_no_location_nop: false,
                match_success_jump: false,
                break_continue_cleanup_jump: false,
            },
        );
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
    while idx != BlockIdx::NULL && blocks[idx.idx()].instructions.is_empty() {
        idx = blocks[idx.idx()].next;
    }
    idx
}

fn is_load_const_none(instr: &InstructionInfo, metadata: &CodeUnitMetadata) -> bool {
    matches!(instr.instr.real(), Some(Instruction::LoadConst { .. }))
        && matches!(
            metadata.consts.get_index(u32::from(instr.arg) as usize),
            Some(ConstantData::None)
        )
}

fn block_tail_starts_with_async_with_normal_exit(instructions: &[InstructionInfo]) -> bool {
    matches!(
        instructions,
        [
            InstructionInfo {
                instr: AnyInstruction::Real(Instruction::LoadConst { .. }),
                ..
            },
            InstructionInfo {
                instr: AnyInstruction::Real(Instruction::LoadConst { .. }),
                ..
            },
            InstructionInfo {
                instr: AnyInstruction::Real(Instruction::LoadConst { .. }),
                ..
            },
            InstructionInfo {
                instr: AnyInstruction::Real(Instruction::Call { .. }),
                ..
            },
            InstructionInfo {
                instr: AnyInstruction::Real(Instruction::GetAwaitable { .. }),
                ..
            },
            ..
        ]
    )
}

fn instruction_lineno(instr: &InstructionInfo) -> i32 {
    instr
        .lineno_override
        .unwrap_or_else(|| instr.location.line.get() as i32)
}

fn instruction_has_lineno(instr: &InstructionInfo) -> bool {
    instruction_lineno(instr) > 0
}

fn propagation_location(instr: &InstructionInfo) -> Option<(SourceLocation, SourceLocation)> {
    instruction_has_lineno(instr).then_some((instr.location, instr.end_location))
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

fn is_exit_without_lineno(blocks: &[Block], block_idx: BlockIdx) -> bool {
    let block = &blocks[block_idx.idx()];
    let Some(first) = block.instructions.first() else {
        return false;
    };
    if instruction_has_lineno(first) || !block_has_no_lineno(block) {
        return false;
    }

    if block
        .instructions
        .last()
        .is_some_and(|last| last.instr.is_scope_exit())
    {
        return true;
    }

    // CPython duplicates no-lineno exit blocks before propagating locations.
    // RustPython's late CFG can inline the following synthetic jump-back block
    // into that exit block first, collapsing `POP_EXCEPT; JUMP_BACKWARD` into a
    // single block. Treat that merged tail as exit-like only for real loop
    // backedges so resolve_line_numbers() can recover CPython's loop cleanup
    // structure without duplicating cold exception-handler jumps to normal
    // forward continuation code.
    let Some((last, prefix)) = block.instructions.split_last() else {
        return false;
    };
    last.instr.is_unconditional_jump()
        && prefix.iter().all(|info| {
            matches!(
                info.instr.real(),
                Some(Instruction::PopExcept | Instruction::Nop)
            )
        })
        && prefix
            .iter()
            .any(|info| matches!(info.instr.real(), Some(Instruction::PopExcept)))
        && has_non_exception_loop_backedge_to(blocks, block_idx, last.target)
}

fn is_eval_break_without_lineno(blocks: &[Block], block_idx: BlockIdx) -> bool {
    let block = &blocks[block_idx.idx()];
    let Some(first) = block.instructions.first() else {
        return false;
    };
    !instruction_has_lineno(first) && block_has_no_lineno(block) && block_has_eval_break(block)
}

fn block_has_eval_break(block: &Block) -> bool {
    block.instructions.iter().any(|info| {
        matches!(
            info.instr,
            AnyInstruction::Pseudo(PseudoInstruction::Jump { .. })
                | AnyInstruction::Real(
                    Instruction::Call { .. }
                        | Instruction::CallFunctionEx
                        | Instruction::CallKw { .. }
                        | Instruction::JumpBackward { .. }
                        | Instruction::Resume { .. }
                )
        )
    })
}

fn block_has_no_lineno(block: &Block) -> bool {
    block
        .instructions
        .iter()
        .all(|ins| !instruction_has_lineno(ins))
}

fn shared_jump_back_target(block: &Block) -> Option<BlockIdx> {
    if !block_has_no_lineno(block) {
        return None;
    }

    let (last, prefix) = block.instructions.split_last()?;
    if !last.instr.is_unconditional_jump() || last.target == BlockIdx::NULL {
        return None;
    }

    if !prefix.iter().all(|info| {
        matches!(
            info.instr.real(),
            Some(Instruction::PopExcept | Instruction::Nop)
        )
    }) {
        return None;
    }

    Some(last.target)
}

fn block_has_non_exception_loop_backedge_to(
    blocks: &[Block],
    source: BlockIdx,
    target: BlockIdx,
) -> bool {
    let target = next_nonempty_block(blocks, target);
    source != BlockIdx::NULL
        && target != BlockIdx::NULL
        && !block_is_exceptional(&blocks[source.idx()])
        && comes_before(blocks, target, source)
        && blocks[source.idx()].instructions.iter().any(|info| {
            info.instr.is_unconditional_jump()
                && info.target != BlockIdx::NULL
                && next_nonempty_block(blocks, info.target) == target
        })
}

fn has_non_exception_loop_backedge_to(
    blocks: &[Block],
    cleanup_block: BlockIdx,
    target: BlockIdx,
) -> bool {
    blocks.iter().enumerate().any(|(source_idx, block)| {
        let source = BlockIdx(source_idx as u32);
        source != cleanup_block
            && !block_is_exceptional(block)
            && block_has_non_exception_loop_backedge_to(blocks, source, target)
    })
}

fn is_jump_only_block(block: &Block) -> bool {
    let [instr] = block.instructions.as_slice() else {
        return false;
    };
    instr.instr.is_unconditional_jump() && instr.target != BlockIdx::NULL
}

fn is_jump_back_only_block(blocks: &[Block], block_idx: BlockIdx) -> bool {
    if block_idx == BlockIdx::NULL || !is_jump_only_block(&blocks[block_idx.idx()]) {
        return false;
    }
    comes_before(
        blocks,
        next_nonempty_block(blocks, blocks[block_idx.idx()].instructions[0].target),
        block_idx,
    )
}

fn is_pop_top_jump_block(block: &Block) -> bool {
    let mut real_instrs = block
        .instructions
        .iter()
        .filter(|info| !matches!(info.instr.real(), Some(Instruction::Nop)));
    let Some(first) = real_instrs.next() else {
        return false;
    };
    let Some(second) = real_instrs.next() else {
        return false;
    };
    real_instrs.next().is_none()
        && matches!(first.instr.real(), Some(Instruction::PopTop))
        && second.instr.is_unconditional_jump()
        && second.target != BlockIdx::NULL
}

fn is_scope_exit_block(block: &Block) -> bool {
    block
        .instructions
        .last()
        .is_some_and(|instr| instr.instr.is_scope_exit())
}

fn is_pop_top_scope_exit_block(block: &Block) -> bool {
    is_scope_exit_block(block)
        && matches!(
            block
                .instructions
                .first()
                .and_then(|info| info.instr.real()),
            Some(Instruction::PopTop)
        )
}

fn is_pop_top_exit_like_block(block: &Block) -> bool {
    is_pop_top_scope_exit_block(block) || is_pop_top_jump_block(block)
}

fn is_loop_cleanup_block(block: &Block) -> bool {
    block
        .instructions
        .iter()
        .find_map(|info| info.instr.real())
        .is_some_and(|instr| {
            matches!(
                instr,
                Instruction::EndFor | Instruction::EndAsyncFor | Instruction::PopIter
            )
        })
}

fn is_async_loop_cleanup_block(block: &Block) -> bool {
    block
        .instructions
        .iter()
        .any(|info| matches!(info.instr.real(), Some(Instruction::EndAsyncFor)))
}

fn is_exception_cleanup_block(block: &Block) -> bool {
    block
        .instructions
        .iter()
        .any(|instr| matches!(instr.instr.real(), Some(Instruction::PopExcept)))
        && block
            .instructions
            .last()
            .is_some_and(|instr| matches!(instr.instr.real(), Some(Instruction::Reraise { .. })))
}

fn is_reraise_scope_exit_block(block: &Block) -> bool {
    block
        .instructions
        .last()
        .is_some_and(|instr| matches!(instr.instr.real(), Some(Instruction::Reraise { .. })))
}

fn block_starts_with_with_exit_none_call(block: &Block) -> bool {
    let real_instrs: Vec<_> = block
        .instructions
        .iter()
        .filter_map(|info| {
            let instr = info.instr.real()?;
            (!matches!(instr, Instruction::Nop)).then_some(instr)
        })
        .take(4)
        .collect();
    matches!(
        real_instrs.as_slice(),
        [
            Instruction::LoadConst { .. },
            Instruction::LoadConst { .. },
            Instruction::LoadConst { .. },
            Instruction::Call { .. },
        ]
    )
}

fn keep_target_start_no_location_nop(
    blocks: &[Block],
    target: BlockIdx,
    layout_predecessors: &[BlockIdx],
) -> bool {
    if target == BlockIdx::NULL {
        return false;
    }
    let Some(first) = blocks[target.idx()].instructions.first() else {
        return false;
    };
    if !matches!(first.instr.real(), Some(Instruction::Nop)) {
        return false;
    }
    let layout_pred = layout_predecessors[target.idx()];
    if layout_pred == BlockIdx::NULL {
        return false;
    }
    if is_async_loop_cleanup_block(&blocks[layout_pred.idx()]) {
        return true;
    }
    is_exception_cleanup_block(&blocks[layout_pred.idx()])
        && !block_starts_with_with_exit_none_call(&blocks[target.idx()])
}

fn layout_predecessor_ends_with_pop_iter_on_line(
    blocks: &[Block],
    target: BlockIdx,
    layout_predecessors: &[BlockIdx],
) -> Option<i32> {
    let layout_pred = layout_predecessors[target.idx()];
    if layout_pred == BlockIdx::NULL
        || !block_has_fallthrough(&blocks[layout_pred.idx()])
        || next_nonempty_block(blocks, blocks[layout_pred.idx()].next) != target
    {
        return None;
    }
    let last = blocks[layout_pred.idx()].instructions.last()?;
    matches!(last.instr.real(), Some(Instruction::PopIter)).then_some(instruction_lineno(last))
}

fn is_with_suppress_exit_block(block: &Block) -> bool {
    let real_instrs: Vec<_> = block
        .instructions
        .iter()
        .filter_map(|info| info.instr.real())
        .collect();
    matches!(
        real_instrs.as_slice(),
        [
            Instruction::PopTop,
            Instruction::PopExcept,
            Instruction::PopTop,
            Instruction::PopTop,
            Instruction::PopTop,
            last,
        ] if last.is_unconditional_jump()
    )
}

fn block_is_protected(block: &Block) -> bool {
    block
        .instructions
        .iter()
        .any(|info| info.except_handler.is_some())
}

fn block_contains_suspension_point(block: &Block) -> bool {
    block
        .instructions
        .iter()
        .filter_map(|info| info.instr.real())
        .any(|instr| {
            matches!(
                instr,
                Instruction::YieldValue { .. }
                    | Instruction::GetAwaitable { .. }
                    | Instruction::GetAnext
                    | Instruction::EndAsyncFor
            )
        })
}

fn block_jump_follows_async_send_pop(block: &Block) -> bool {
    let mut before_jump =
        block
            .instructions
            .iter()
            .rev()
            .skip(1)
            .filter_map(|info| match info.instr.real() {
                Some(Instruction::Nop) => None,
                instr => instr,
            });
    matches!(
        (before_jump.next(), before_jump.next()),
        (Some(Instruction::PopTop), Some(Instruction::EndSend))
    )
}

fn block_jump_follows_with_normal_exit(block: &Block) -> bool {
    let mut before_jump =
        block
            .instructions
            .iter()
            .rev()
            .skip(1)
            .filter_map(|info| match info.instr.real() {
                Some(Instruction::Nop) => None,
                instr => instr,
            });
    matches!(
        (
            before_jump.next(),
            before_jump.next(),
            before_jump.next(),
            before_jump.next(),
            before_jump.next(),
        ),
        (
            Some(Instruction::PopTop),
            Some(Instruction::Call { .. }),
            Some(Instruction::LoadConst { .. }),
            Some(Instruction::LoadConst { .. }),
            Some(Instruction::LoadConst { .. }),
        )
    )
}

fn is_stop_iteration_error_handler_block(block: &Block) -> bool {
    matches!(
        block.instructions.as_slice(),
        [
            InstructionInfo {
                instr: AnyInstruction::Real(Instruction::CallIntrinsic1 { func }),
                arg,
                ..
            },
            InstructionInfo {
                instr: AnyInstruction::Real(Instruction::Reraise { .. }),
                ..
            }
        ] if matches!(func.get(*arg), oparg::IntrinsicFunction1::StopIterationError)
    )
}

fn block_has_only_stop_iteration_error_handlers(block: &Block, blocks: &[Block]) -> bool {
    let mut saw_handler = false;
    for info in &block.instructions {
        let Some(handler) = info.except_handler else {
            continue;
        };
        saw_handler = true;
        let target = next_nonempty_block(blocks, handler.handler_block);
        if target == BlockIdx::NULL || !is_stop_iteration_error_handler_block(&blocks[target.idx()])
        {
            return false;
        }
    }
    saw_handler
}

fn block_is_exceptional(block: &Block) -> bool {
    block.except_handler || block.preserve_lasti || is_exception_cleanup_block(block)
}

fn has_exceptional_duplicate_lineno(blocks: &[Block], source: BlockIdx, lineno: i32) -> bool {
    blocks.iter().enumerate().any(|(idx, block)| {
        BlockIdx(idx as u32) != source
            && (block.cold || block_is_exceptional(block) || block_is_protected(block))
            && block
                .instructions
                .iter()
                .any(|info| instruction_lineno(info) == lineno)
    })
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

fn block_is_pure_conditional_test(block: &Block) -> bool {
    let Some(cond_idx) = trailing_conditional_jump_index(block) else {
        return false;
    };
    block.instructions[..cond_idx].iter().all(|info| {
        matches!(
            info.instr.real(),
            Some(
                Instruction::Nop
                    | Instruction::LoadFast { .. }
                    | Instruction::LoadFastBorrow { .. }
                    | Instruction::LoadFastLoadFast { .. }
                    | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                    | Instruction::LoadDeref { .. }
                    | Instruction::LoadGlobal { .. }
                    | Instruction::LoadConst { .. }
                    | Instruction::LoadSmallInt { .. }
                    | Instruction::LoadAttr { .. }
                    | Instruction::BinaryOp { .. }
                    | Instruction::ContainsOp { .. }
                    | Instruction::IsOp { .. }
                    | Instruction::CompareOp { .. }
                    | Instruction::ToBool
            )
        )
    })
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
            if block_is_exceptional(&blocks[cursor.idx()]) {
                exit_segment_valid = false;
                break;
            }
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
            if block_is_exceptional(&blocks[cursor.idx()])
                || block_is_protected(&blocks[cursor.idx()])
            {
                jump_block = BlockIdx::NULL;
                break;
            }
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
        let jump_instr = blocks[jump_block.idx()].instructions[0];
        if jump_instr.match_success_jump {
            // CPython codegen_pattern_or() emits an explicit success
            // JUMP after every alternative, including the last one.
            current = next;
            continue;
        }
        let jump_is_forward = matches!(
            jump_instr.instr.real(),
            Some(Instruction::JumpForward { .. })
        );
        let jump_is_backward = matches!(
            jump_instr.instr.real(),
            Some(Instruction::JumpBackward { .. })
        );
        let exit_is_reorderable =
            blocks[exit_block.idx()]
                .instructions
                .last()
                .is_some_and(|info| {
                    matches!(
                        info.instr.real(),
                        Some(Instruction::ReturnValue | Instruction::RaiseVarargs { .. })
                    )
                });
        if !(jump_is_forward || jump_is_backward && exit_is_reorderable) {
            current = next;
            continue;
        }
        if jump_is_backward {
            if block_is_protected(&blocks[idx])
                || block_is_protected(&blocks[exit_block.idx()])
                || block_is_protected(&blocks[jump_block.idx()])
            {
                current = next;
                continue;
            }
            let after_jump_start = blocks[jump_end.idx()].next;
            let after_jump = next_nonempty_block(blocks, after_jump_start);
            let after_jump_is_end_async_for = after_jump != BlockIdx::NULL
                && blocks[after_jump.idx()]
                    .instructions
                    .iter()
                    .any(|info| matches!(info.instr.real(), Some(Instruction::EndAsyncFor)));
            let after_jump_is_adjacent_scope_exit = after_jump != BlockIdx::NULL
                && is_pop_top_exit_like_block(&blocks[after_jump.idx()]);
            if after_jump_is_end_async_for || after_jump_is_adjacent_scope_exit {
                current = next;
                continue;
            }
            // CPython flowgraph.c::normalize_jumps_in_block() only checks
            // whether the conditional target was already visited.  When the
            // jump block is a backward edge to a loop header, the following
            // outer block does not prevent rewriting the condition so the
            // backedge remains the fallthrough path.
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
            if block_is_exceptional(&blocks[cursor.idx()]) {
                jump_segment_valid = false;
                break;
            }
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
        if jump_instr.match_success_jump {
            // CPython codegen_pattern_or() emits an explicit success
            // JUMP after every alternative, including the last one.
            current = next;
            continue;
        }
        if !matches!(
            jump_instr.instr.real(),
            Some(Instruction::JumpForward { .. })
        ) {
            current = next;
            continue;
        }
        if jump_instr.lineno_override.is_some_and(|line| line >= 0)
            && instruction_lineno(&jump_instr) != instruction_lineno(&last)
        {
            current = next;
            continue;
        }

        let mut exit_end = BlockIdx::NULL;
        let mut exit_block = BlockIdx::NULL;
        let after_exit = loop {
            if cursor == BlockIdx::NULL {
                break BlockIdx::NULL;
            }
            if block_is_exceptional(&blocks[cursor.idx()]) {
                if exit_block != BlockIdx::NULL {
                    break cursor;
                }
                exit_block = BlockIdx::NULL;
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

fn reorder_conditional_chain_and_jump_back_blocks(blocks: &mut Vec<Block>) {
    let target_comes_before = |target: BlockIdx, block: BlockIdx, blocks: &[Block]| -> bool {
        let mut current = BlockIdx(0);
        while current != BlockIdx::NULL {
            if current == target {
                return true;
            }
            if current == block {
                return false;
            }
            current = blocks[current.idx()].next;
        }
        false
    };

    fn is_single_delete_subscr_body(block: &Block) -> bool {
        let real: Vec<_> = block
            .instructions
            .iter()
            .filter_map(|info| info.instr.real())
            .filter(|instr| !matches!(instr, Instruction::Nop | Instruction::NotTaken))
            .collect();
        real.iter()
            .filter(|instr| matches!(instr, Instruction::DeleteSubscr))
            .count()
            == 1
            && matches!(real.last(), Some(Instruction::DeleteSubscr))
            && real.iter().all(|instr| {
                matches!(
                    instr,
                    Instruction::LoadFast { .. }
                        | Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastLoadFast { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                        | Instruction::LoadSmallInt { .. }
                        | Instruction::LoadConst { .. }
                        | Instruction::Copy { .. }
                        | Instruction::Swap { .. }
                        | Instruction::BinaryOp { .. }
                        | Instruction::BinarySlice
                        | Instruction::BuildSlice { .. }
                        | Instruction::StoreSubscr
                        | Instruction::DeleteSubscr
                )
            })
    }

    fn chain_is_conditional_single_delete_body(
        blocks: &[Block],
        chain_start: BlockIdx,
        jump_start: BlockIdx,
    ) -> bool {
        if chain_start == BlockIdx::NULL || jump_start == BlockIdx::NULL {
            return false;
        }
        let Some(chain_cond_idx) = trailing_conditional_jump_index(&blocks[chain_start.idx()])
        else {
            return false;
        };
        let chain_cond = blocks[chain_start.idx()].instructions[chain_cond_idx];
        if !is_false_path_conditional_jump(&chain_cond.instr) || chain_cond.target != jump_start {
            return false;
        }
        let body = next_nonempty_block(blocks, blocks[chain_start.idx()].next);
        body != BlockIdx::NULL
            && !block_is_exceptional(&blocks[body.idx()])
            && !block_is_protected(&blocks[body.idx()])
            && next_nonempty_block(blocks, blocks[body.idx()].next) == jump_start
            && is_single_delete_subscr_body(&blocks[body.idx()])
    }

    fn jump_targets_for_iter(blocks: &[Block], jump_block: BlockIdx) -> bool {
        if jump_block == BlockIdx::NULL {
            return false;
        }
        let Some(info) = blocks[jump_block.idx()].instructions.first() else {
            return false;
        };
        let target = next_nonempty_block(blocks, info.target);
        target != BlockIdx::NULL
            && blocks[target.idx()]
                .instructions
                .first()
                .is_some_and(|target_info| {
                    matches!(target_info.instr.real(), Some(Instruction::ForIter { .. }))
                })
    }

    fn true_body_false_backedge_is_already_normalized(
        blocks: &[Block],
        conditional: InstructionInfo,
        false_backedge: BlockIdx,
        true_body: BlockIdx,
    ) -> bool {
        fn comes_before(blocks: &[Block], target: BlockIdx, block: BlockIdx) -> bool {
            let mut current = BlockIdx(0);
            while current != BlockIdx::NULL {
                if current == target {
                    return true;
                }
                if current == block {
                    return false;
                }
                current = blocks[current.idx()].next;
            }
            false
        }

        if !matches!(
            conditional.instr.real(),
            Some(Instruction::PopJumpIfTrue { .. })
        ) || false_backedge == BlockIdx::NULL
            || true_body == BlockIdx::NULL
            || !is_jump_only_block(&blocks[false_backedge.idx()])
        {
            return false;
        }
        let false_target = blocks[false_backedge.idx()].instructions[0].target;
        if false_target == BlockIdx::NULL || !comes_before(blocks, false_target, false_backedge) {
            return false;
        }
        let true_tail = next_nonempty_block(blocks, blocks[true_body.idx()].next);
        true_tail != BlockIdx::NULL
            && is_jump_only_block(&blocks[true_tail.idx()])
            && blocks[true_tail.idx()].instructions[0].target == false_target
    }

    fn has_other_conditional_predecessor_to(
        blocks: &[Block],
        conditional_target_counts: &[usize],
        target: BlockIdx,
        current: BlockIdx,
    ) -> bool {
        let current_targets = blocks[current.idx()]
            .instructions
            .iter()
            .filter(|info| info.target == target && is_conditional_jump(&info.instr))
            .count();
        conditional_target_counts[target.idx()] > current_targets
    }

    let has_exceptional_lineno_sources = blocks
        .iter()
        .any(|block| block.cold || block_is_exceptional(block) || block_is_protected(block));
    let mut conditional_target_counts = vec![0usize; blocks.len()];
    for block in blocks.iter() {
        for info in &block.instructions {
            if info.target != BlockIdx::NULL && is_conditional_jump(&info.instr) {
                conditional_target_counts[info.target.idx()] += 1;
            }
        }
    }

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

        let chain_start = next;
        let jump_start = last.target;
        if chain_start == BlockIdx::NULL
            || jump_start == BlockIdx::NULL
            || chain_start == jump_start
        {
            current = next;
            continue;
        }
        if true_body_false_backedge_is_already_normalized(blocks, last, chain_start, jump_start) {
            current = next;
            continue;
        }
        let mut chain_has_suspension_point = false;
        let mut scan = chain_start;
        while scan != BlockIdx::NULL && scan != jump_start {
            if block_contains_suspension_point(&blocks[scan.idx()]) {
                chain_has_suspension_point = true;
                break;
            }
            scan = blocks[scan.idx()].next;
        }
        let chain_starts_with_false_path_jump = trailing_conditional_jump_index(
            &blocks[chain_start.idx()],
        )
        .is_some_and(|chain_cond_idx| {
            is_false_path_conditional_jump(
                &blocks[chain_start.idx()].instructions[chain_cond_idx].instr,
            )
        });
        let chain_is_single_exit_block = is_scope_exit_block(&blocks[chain_start.idx()])
            && next_nonempty_block(blocks, blocks[chain_start.idx()].next) == jump_start;
        let chain_is_jump_only_exit_block = is_jump_only_block(&blocks[chain_start.idx()])
            && !target_comes_before(
                blocks[chain_start.idx()].instructions[0].target,
                chain_start,
                blocks,
            );
        let chain_is_jump_back_exit_block = is_jump_only_block(&blocks[chain_start.idx()])
            && target_comes_before(
                blocks[chain_start.idx()].instructions[0].target,
                chain_start,
                blocks,
            );
        let allow_true_path_jump_back_reorder =
            matches!(last.instr.real(), Some(Instruction::PopJumpIfTrue { .. }))
                && (chain_has_suspension_point
                    || chain_starts_with_false_path_jump
                    || chain_is_single_exit_block
                    || chain_is_jump_only_exit_block
                    || chain_is_jump_back_exit_block);
        let is_generic_false_path_reorder = !allow_true_path_jump_back_reorder;
        if !is_false_path_conditional_jump(&last.instr) && !allow_true_path_jump_back_reorder {
            current = next;
            continue;
        }
        if is_generic_false_path_reorder && is_scope_exit_block(&blocks[chain_start.idx()]) {
            current = next;
            continue;
        }
        if has_exceptional_lineno_sources
            && is_generic_false_path_reorder
            && has_exceptional_duplicate_lineno(blocks, current, instruction_lineno(&last))
        {
            current = next;
            continue;
        }
        if is_generic_false_path_reorder
            && has_other_conditional_predecessor_to(
                blocks,
                &conditional_target_counts,
                chain_start,
                current,
            )
        {
            current = next;
            continue;
        }
        if block_is_protected(&blocks[idx]) && block_contains_suspension_point(&blocks[idx]) {
            current = next;
            continue;
        }
        if let Some(chain_cond_idx) = trailing_conditional_jump_index(&blocks[chain_start.idx()]) {
            let chain_cond = blocks[chain_start.idx()].instructions[chain_cond_idx];
            if matches!(
                chain_cond.instr.real().map(Into::into),
                Some(Opcode::PopJumpIfTrue)
            ) {
                let chain_true_target = next_nonempty_block(blocks, chain_cond.target);
                if chain_true_target != BlockIdx::NULL
                    && !is_scope_exit_block(&blocks[chain_true_target.idx()])
                    && !is_jump_only_block(&blocks[chain_true_target.idx()])
                    && !is_pop_top_jump_block(&blocks[chain_true_target.idx()])
                {
                    current = next;
                    continue;
                }
            }
        }

        let mut chain_end = BlockIdx::NULL;
        let mut saw_nonempty = false;
        let mut nonempty_blocks = 0usize;
        let mut real_instr_count = 0usize;
        let mut cursor = chain_start;
        let mut chain_valid = true;
        while cursor != BlockIdx::NULL && cursor != jump_start {
            if block_is_exceptional(&blocks[cursor.idx()])
                || (block_is_protected(&blocks[cursor.idx()])
                    && block_contains_suspension_point(&blocks[cursor.idx()])
                    && !block_has_only_stop_iteration_error_handlers(&blocks[cursor.idx()], blocks))
            {
                chain_valid = false;
                break;
            }
            if !blocks[cursor.idx()].instructions.is_empty() {
                saw_nonempty = true;
                nonempty_blocks += 1;
                real_instr_count += blocks[cursor.idx()]
                    .instructions
                    .iter()
                    .filter(|info| info.instr.real().is_some())
                    .count();
            }
            chain_end = cursor;
            cursor = blocks[cursor.idx()].next;
        }
        if !chain_valid || !saw_nonempty || chain_end == BlockIdx::NULL || cursor != jump_start {
            current = next;
            continue;
        }
        let chain_is_conditional_single_delete_body =
            chain_is_conditional_single_delete_body(blocks, chain_start, jump_start);
        if is_generic_false_path_reorder
            && nonempty_blocks > 1
            && !chain_is_conditional_single_delete_body
        {
            current = next;
            continue;
        }
        if !is_generic_false_path_reorder && (nonempty_blocks > 8 || real_instr_count > 80) {
            current = next;
            continue;
        }

        let mut jump_end = BlockIdx::NULL;
        let mut jump_block = BlockIdx::NULL;
        cursor = jump_start;
        while cursor != BlockIdx::NULL {
            if block_is_exceptional(&blocks[cursor.idx()]) {
                jump_block = BlockIdx::NULL;
                break;
            }
            jump_end = cursor;
            if blocks[cursor.idx()].instructions.is_empty() {
                cursor = blocks[cursor.idx()].next;
                continue;
            }
            if !is_jump_only_block(&blocks[cursor.idx()])
                || !target_comes_before(blocks[cursor.idx()].instructions[0].target, cursor, blocks)
            {
                jump_block = BlockIdx::NULL;
            } else {
                jump_block = cursor;
            }
            break;
        }
        if jump_block == BlockIdx::NULL || jump_end == BlockIdx::NULL {
            current = next;
            continue;
        }
        let after_jump = next_nonempty_block(blocks, blocks[jump_block.idx()].next);
        let jump_is_artificial = blocks[jump_block.idx()]
            .instructions
            .first()
            .is_some_and(|info| matches!(info.lineno_override, Some(line) if line < 0));
        let after_jump_is_adjacent_scope_exit =
            after_jump != BlockIdx::NULL && is_pop_top_exit_like_block(&blocks[after_jump.idx()]);
        if !is_generic_false_path_reorder
            && chain_is_single_exit_block
            && after_jump_is_adjacent_scope_exit
        {
            current = next;
            continue;
        }
        if is_generic_false_path_reorder
            && jump_is_artificial
            && after_jump != BlockIdx::NULL
            && is_loop_cleanup_block(&blocks[after_jump.idx()])
            && !chain_is_conditional_single_delete_body
        {
            current = next;
            continue;
        }
        if !is_generic_false_path_reorder
            && jump_is_artificial
            && jump_targets_for_iter(blocks, jump_block)
        {
            current = next;
            continue;
        }
        if nonempty_blocks == 1
            && !is_jump_only_block(&blocks[chain_start.idx()])
            && after_jump != BlockIdx::NULL
            && !blocks[after_jump.idx()].cold
            && !block_is_exceptional(&blocks[after_jump.idx()])
            && !is_scope_exit_block(&blocks[after_jump.idx()])
            && !is_loop_cleanup_block(&blocks[after_jump.idx()])
        {
            current = next;
            continue;
        }

        let mut cloned_jump = blocks[jump_block.idx()].clone();
        cloned_jump.next = chain_start;
        cloned_jump.start_depth = None;
        let cloned_idx = BlockIdx::new(blocks.len() as u32);
        blocks.push(cloned_jump);
        conditional_target_counts.push(0);
        blocks[idx].next = cloned_idx;
        let cond_mut = &mut blocks[idx].instructions[cond_idx];
        if cond_mut.target != BlockIdx::NULL {
            conditional_target_counts[cond_mut.target.idx()] -= 1;
        }
        conditional_target_counts[chain_start.idx()] += 1;
        cond_mut.instr = reversed;
        cond_mut.target = chain_start;

        current = next;
    }
}

fn reorder_conditional_scope_exit_and_jump_back_blocks(
    blocks: &mut [Block],
    allow_for_iter_jump_targets: bool,
    allow_true_scope_exit_reorder: bool,
) {
    fn retarget_empty_chain_targets(
        blocks: &mut [Block],
        chain_start: BlockIdx,
        chain_end: BlockIdx,
        replacement: BlockIdx,
    ) {
        if chain_start == BlockIdx::NULL
            || chain_end == BlockIdx::NULL
            || replacement == BlockIdx::NULL
        {
            return;
        }

        let mut in_chain = vec![false; blocks.len()];
        let mut cursor = chain_start;
        while cursor != BlockIdx::NULL && cursor != chain_end {
            let block = &blocks[cursor.idx()];
            if !block.instructions.is_empty() {
                return;
            }
            in_chain[cursor.idx()] = true;
            cursor = block.next;
        }
        if cursor != chain_end {
            return;
        }

        for block in blocks {
            for instr in &mut block.instructions {
                if instr.target != BlockIdx::NULL && in_chain[instr.target.idx()] {
                    instr.target = replacement;
                }
            }
        }
    }

    fn jump_targets_for_iter(blocks: &[Block], jump_block: BlockIdx) -> bool {
        if jump_block == BlockIdx::NULL {
            return false;
        }
        let Some(info) = blocks[jump_block.idx()].instructions.first() else {
            return false;
        };
        let target = next_nonempty_block(blocks, info.target);
        target != BlockIdx::NULL
            && blocks[target.idx()]
                .instructions
                .first()
                .is_some_and(|target_info| {
                    matches!(target_info.instr.real(), Some(Instruction::ForIter { .. }))
                })
    }

    fn is_explicit_continue_to_for_iter(blocks: &[Block], jump_block: BlockIdx) -> bool {
        if jump_block == BlockIdx::NULL {
            return false;
        }
        let Some(info) = blocks[jump_block.idx()].instructions.first() else {
            return false;
        };
        matches!(
            info.instr.real(),
            Some(Instruction::JumpBackward { .. } | Instruction::JumpBackwardNoInterrupt { .. })
        ) && !matches!(info.lineno_override, Some(line) if line < 0)
            && jump_targets_for_iter(blocks, jump_block)
    }

    fn is_explicit_continue_after_conditional(
        blocks: &[Block],
        jump_block: BlockIdx,
        cond: InstructionInfo,
    ) -> bool {
        if !is_explicit_continue_to_for_iter(blocks, jump_block) {
            return false;
        }
        let Some(info) = blocks[jump_block.idx()].instructions.first() else {
            return false;
        };
        instruction_lineno(info) > instruction_lineno(&cond)
    }

    fn is_explicit_non_for_jump_back(blocks: &[Block], jump_block: BlockIdx) -> bool {
        if jump_block == BlockIdx::NULL || jump_targets_for_iter(blocks, jump_block) {
            return false;
        }
        let Some(info) = blocks[jump_block.idx()].instructions.first() else {
            return false;
        };
        matches!(info.instr.real(), Some(Instruction::JumpBackward { .. }))
            && info.lineno_override.is_some_and(|line| line >= 0)
    }

    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let idx = current.idx();
        let next = blocks[idx].next;
        let Some(cond_idx) = trailing_conditional_jump_index(&blocks[idx]) else {
            current = next;
            continue;
        };
        let cond = blocks[idx].instructions[cond_idx];
        if matches!(cond.instr.real(), Some(Instruction::PopJumpIfFalse { .. })) {
            let exit_start = next;
            let exit_block = next_nonempty_block(blocks, exit_start);
            let jump_start = cond.target;
            let jump_block = next_nonempty_block(blocks, jump_start);
            let after_jump = if jump_block != BlockIdx::NULL {
                next_nonempty_block(blocks, blocks[jump_block.idx()].next)
            } else {
                BlockIdx::NULL
            };
            let after_jump_continues_conditional_chain = after_jump != BlockIdx::NULL
                && block_is_pure_conditional_test(&blocks[after_jump.idx()]);
            // Allow reorder when all three blocks share the same except handler
            // (or none have one). Reordering within a shared protection region
            // matches CPython, which reorders regardless of try/except context.
            let cond_handler = blocks[idx]
                .instructions
                .first()
                .and_then(|info| info.except_handler);
            let jump_handler = blocks[jump_block.idx()]
                .instructions
                .first()
                .and_then(|info| info.except_handler);
            let exit_handler = blocks[exit_block.idx()]
                .instructions
                .first()
                .and_then(|info| info.except_handler);
            let mismatched_protection =
                !(cond_handler == jump_handler && jump_handler == exit_handler);
            if exit_start == BlockIdx::NULL
                || exit_block == BlockIdx::NULL
                || jump_start == BlockIdx::NULL
                || jump_block == BlockIdx::NULL
                || jump_block == exit_block
                || after_jump_continues_conditional_chain
                || block_is_exceptional(&blocks[idx])
                || block_is_exceptional(&blocks[jump_block.idx()])
                || block_is_exceptional(&blocks[exit_block.idx()])
                || mismatched_protection
                || !is_scope_exit_block(&blocks[exit_block.idx()])
                || !is_jump_back_only_block(blocks, jump_block)
                || (!allow_for_iter_jump_targets
                    && is_explicit_continue_to_for_iter(blocks, jump_block))
                    && blocks[exit_block.idx()].instructions.iter().any(|info| {
                        matches!(info.instr.real(), Some(Instruction::RaiseVarargs { .. }))
                    })
                || (!allow_for_iter_jump_targets
                    && is_explicit_non_for_jump_back(blocks, jump_block))
                || (after_jump != BlockIdx::NULL
                    && is_pop_top_exit_like_block(&blocks[after_jump.idx()]))
                || (after_jump != BlockIdx::NULL
                    && !blocks[after_jump.idx()].cold
                    && !is_scope_exit_block(&blocks[after_jump.idx()])
                    && !is_loop_cleanup_block(&blocks[after_jump.idx()])
                    && jump_targets_for_iter(blocks, jump_block))
                || next_nonempty_block(blocks, blocks[exit_block.idx()].next) != jump_block
            {
                current = next;
                continue;
            }

            let after_jump = blocks[jump_block.idx()].next;
            blocks[idx].instructions[cond_idx].instr = reversed_conditional(&cond.instr)
                .expect("PopJumpIfFalse has a reversed conditional");
            blocks[idx].instructions[cond_idx].target = exit_start;
            blocks[idx].next = jump_start;
            blocks[jump_block.idx()].next = exit_start;
            blocks[exit_block.idx()].next = after_jump;
            current = next;
            continue;
        }
        if !matches!(cond.instr.real(), Some(Instruction::PopJumpIfTrue { .. })) {
            current = next;
            continue;
        }
        if !allow_true_scope_exit_reorder {
            current = next;
            continue;
        }
        let exit_block = next_nonempty_block(blocks, cond.target);
        let jump_block = next_nonempty_block(blocks, next);
        if exit_block == BlockIdx::NULL
            || jump_block == BlockIdx::NULL
            || !is_scope_exit_block(&blocks[exit_block.idx()])
            || !is_jump_only_block(&blocks[jump_block.idx()])
            // CPython flowgraph.c::normalize_jumps_in_block() leaves a
            // backward conditional as reversed_conditional-to-exit followed by
            // a fallthrough backward jump. This Rust layout pass must not
            // undo that normalized shape.
            || (is_jump_back_only_block(blocks, jump_block)
                && next_nonempty_block(blocks, blocks[jump_block.idx()].next) == exit_block)
            || (jump_targets_for_iter(blocks, jump_block)
                && !is_explicit_continue_after_conditional(blocks, jump_block, cond))
            || next_nonempty_block(blocks, blocks[jump_block.idx()].next) != exit_block
            || !comes_before(
                blocks,
                next_nonempty_block(blocks, blocks[jump_block.idx()].instructions[0].target),
                jump_block,
            )
        {
            current = next;
            continue;
        }
        let after_exit = blocks[exit_block.idx()].next;
        retarget_empty_chain_targets(blocks, next, jump_block, jump_block);
        blocks[idx].instructions[cond_idx].instr =
            reversed_conditional(&cond.instr).expect("PopJumpIfTrue has a reversed conditional");
        blocks[idx].instructions[cond_idx].target = jump_block;
        blocks[idx].next = exit_block;
        blocks[exit_block.idx()].next = jump_block;
        blocks[jump_block.idx()].next = after_exit;
        current = next;
    }
}

fn reorder_conditional_break_continue_blocks(blocks: &mut [Block]) {
    fn is_break_exit_like(blocks: &[Block], block_idx: BlockIdx) -> bool {
        let block = &blocks[block_idx.idx()];
        if !is_jump_only_block(block) {
            return false;
        }
        matches!(
            block.instructions[0].instr.real(),
            Some(Instruction::JumpForward { .. })
        ) && !comes_before(
            blocks,
            next_nonempty_block(blocks, block.instructions[0].target),
            block_idx,
        )
    }

    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let idx = current.idx();
        let next = blocks[idx].next;
        let Some(cond_idx) = trailing_conditional_jump_index(&blocks[idx]) else {
            current = next;
            continue;
        };
        let cond = blocks[idx].instructions[cond_idx];
        if !matches!(cond.instr.real(), Some(Instruction::PopJumpIfTrue { .. })) {
            current = next;
            continue;
        }

        let jump_start = next;
        let jump_block = next_nonempty_block(blocks, jump_start);
        let exit_start = cond.target;
        let exit_block = next_nonempty_block(blocks, exit_start);
        if jump_start == BlockIdx::NULL
            || jump_block == BlockIdx::NULL
            || exit_start == BlockIdx::NULL
            || exit_block == BlockIdx::NULL
            || jump_block == exit_block
            || block_is_exceptional(&blocks[idx])
            || block_is_exceptional(&blocks[jump_block.idx()])
            || block_is_exceptional(&blocks[exit_block.idx()])
            || block_is_protected(&blocks[idx])
            || block_is_protected(&blocks[jump_block.idx()])
            || block_is_protected(&blocks[exit_block.idx()])
            || !is_jump_back_only_block(blocks, jump_block)
            || !is_break_exit_like(blocks, exit_block)
            || next_nonempty_block(blocks, blocks[jump_block.idx()].next) != exit_block
        {
            current = next;
            continue;
        }

        let after_exit = blocks[exit_block.idx()].next;
        blocks[idx].instructions[cond_idx].instr =
            reversed_conditional(&cond.instr).expect("PopJumpIfTrue has a reversed conditional");
        blocks[idx].instructions[cond_idx].target = jump_start;
        blocks[idx].next = exit_start;
        blocks[exit_block.idx()].next = jump_start;
        blocks[jump_block.idx()].next = after_exit;
        current = next;
    }
}

fn reorder_conditional_explicit_continue_scope_exit_blocks(blocks: &mut [Block]) {
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let idx = current.idx();
        let next = blocks[idx].next;
        let Some(cond_idx) = trailing_conditional_jump_index(&blocks[idx]) else {
            current = next;
            continue;
        };
        let cond = blocks[idx].instructions[cond_idx];
        if !matches!(cond.instr.real(), Some(Instruction::PopJumpIfTrue { .. })) {
            current = next;
            continue;
        }

        let jump_start = next;
        let jump_block = next_nonempty_block(blocks, jump_start);
        let exit_start = cond.target;
        let exit_block = next_nonempty_block(blocks, exit_start);
        if jump_start == BlockIdx::NULL
            || jump_block == BlockIdx::NULL
            || exit_start == BlockIdx::NULL
            || exit_block == BlockIdx::NULL
            || jump_block == exit_block
            || block_is_exceptional(&blocks[idx])
            || block_is_exceptional(&blocks[jump_block.idx()])
            || block_is_exceptional(&blocks[exit_block.idx()])
            || block_is_protected(&blocks[idx])
            || block_is_protected(&blocks[jump_block.idx()])
            || block_is_protected(&blocks[exit_block.idx()])
            || !is_scope_exit_block(&blocks[exit_block.idx()])
            || !is_jump_back_only_block(blocks, jump_block)
            || next_nonempty_block(blocks, blocks[jump_block.idx()].next) != exit_block
            || blocks[jump_block.idx()]
                .instructions
                .first()
                .is_none_or(|info| info.lineno_override.is_none_or(|lineno| lineno < 0))
        {
            current = next;
            continue;
        }

        let after_exit = blocks[exit_block.idx()].next;
        blocks[idx].instructions[cond_idx].instr =
            reversed_conditional(&cond.instr).expect("PopJumpIfTrue has a reversed conditional");
        blocks[idx].instructions[cond_idx].target = jump_start;
        blocks[idx].next = exit_start;
        blocks[exit_block.idx()].next = jump_start;
        blocks[jump_block.idx()].next = after_exit;
        current = next;
    }
}

fn reorder_conditional_implicit_continue_scope_exit_blocks(blocks: &mut [Block]) {
    fn jump_back_target(block: &Block) -> Option<BlockIdx> {
        let [info] = block.instructions.as_slice() else {
            return None;
        };
        if matches!(info.instr.real(), Some(Instruction::JumpBackward { .. }))
            && info.target != BlockIdx::NULL
        {
            Some(info.target)
        } else {
            None
        }
    }

    fn scope_exit_segment_tail_before_jump(
        blocks: &[Block],
        start: BlockIdx,
        jump_start: BlockIdx,
    ) -> Option<BlockIdx> {
        let jump_block = next_nonempty_block(blocks, jump_start);
        if jump_block == BlockIdx::NULL {
            return None;
        }

        let mut segment = Vec::new();
        let mut cursor = next_nonempty_block(blocks, start);
        while cursor != BlockIdx::NULL && cursor != jump_block {
            if segment.len() >= blocks.len() {
                return None;
            }
            let block = &blocks[cursor.idx()];
            if block_is_exceptional(block) || block_is_protected(block) {
                return None;
            }
            segment.push(cursor);
            cursor = next_nonempty_block(blocks, block.next);
        }
        if cursor != jump_block || segment.is_empty() {
            return None;
        }

        let mut in_segment = vec![false; blocks.len()];
        for block_idx in &segment {
            in_segment[block_idx.idx()] = true;
        }

        let mut has_scope_exit = false;
        for block_idx in &segment {
            let block = &blocks[block_idx.idx()];
            if is_scope_exit_block(block) {
                has_scope_exit = true;
                continue;
            }

            if block_has_fallthrough(block) {
                let next = next_nonempty_block(blocks, block.next);
                if next == BlockIdx::NULL || !in_segment[next.idx()] {
                    return None;
                }
            }

            for info in &block.instructions {
                if info.target == BlockIdx::NULL {
                    continue;
                }
                let target = next_nonempty_block(blocks, info.target);
                if target == BlockIdx::NULL || !in_segment[target.idx()] {
                    return None;
                }
            }
        }

        has_scope_exit.then_some(*segment.last().expect("non-empty segment"))
    }

    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let idx = current.idx();
        let next = blocks[idx].next;
        let Some(cond_idx) = trailing_conditional_jump_index(&blocks[idx]) else {
            current = next;
            continue;
        };
        let cond = blocks[idx].instructions[cond_idx];
        if !matches!(cond.instr.real(), Some(Instruction::PopJumpIfFalse { .. })) {
            current = next;
            continue;
        }

        let exit_start = next;
        let exit_block = next_nonempty_block(blocks, exit_start);
        let jump_start = cond.target;
        let jump_block = next_nonempty_block(blocks, jump_start);
        let jump_target = if jump_block != BlockIdx::NULL {
            jump_back_target(&blocks[jump_block.idx()])
        } else {
            None
        };
        let jumps_to_for_iter = jump_target.is_some_and(|target| {
            blocks[target.idx()]
                .instructions
                .first()
                .is_some_and(|info| matches!(info.instr.real(), Some(Instruction::ForIter { .. })))
        });
        let after_jump_start = if jump_block != BlockIdx::NULL {
            blocks[jump_block.idx()].next
        } else {
            BlockIdx::NULL
        };
        let after_jump = next_nonempty_block(blocks, after_jump_start);
        let jump_exits_to_loop_exit = jump_target.is_some_and(|target| {
            trailing_conditional_jump_index(&blocks[target.idx()]).is_some_and(|cond_idx| {
                let loop_exit_start = blocks[target.idx()].instructions[cond_idx].target;
                loop_exit_start == after_jump_start
                    || next_nonempty_block(blocks, loop_exit_start) == after_jump
            })
        });
        let jump_has_lineno = blocks[jump_block.idx()]
            .instructions
            .first()
            .is_some_and(|info| instruction_lineno(info) >= 0);
        let exit_segment_tail = scope_exit_segment_tail_before_jump(blocks, exit_start, jump_start);
        if exit_start == BlockIdx::NULL
            || exit_block == BlockIdx::NULL
            || jump_start == BlockIdx::NULL
            || jump_block == BlockIdx::NULL
            || exit_block == jump_block
            || block_is_exceptional(&blocks[idx])
            || block_is_exceptional(&blocks[exit_block.idx()])
            || block_is_exceptional(&blocks[jump_block.idx()])
            || block_is_protected(&blocks[idx])
            || block_is_protected(&blocks[exit_block.idx()])
            || block_is_protected(&blocks[jump_block.idx()])
            || exit_segment_tail.is_none()
            || !is_jump_back_only_block(blocks, jump_block)
            || jumps_to_for_iter
            || (after_jump != BlockIdx::NULL
                && !blocks[after_jump.idx()].cold
                && !jump_exits_to_loop_exit
                && !jump_has_lineno)
        {
            current = next;
            continue;
        }

        let exit_segment_tail = exit_segment_tail.expect("checked above");
        let after_jump = blocks[jump_block.idx()].next;
        blocks[idx].instructions[cond_idx].instr =
            reversed_conditional(&cond.instr).expect("PopJumpIfFalse has a reversed conditional");
        blocks[idx].instructions[cond_idx].target = exit_start;
        blocks[idx].next = jump_start;
        blocks[jump_block.idx()].next = exit_start;
        blocks[exit_segment_tail.idx()].next = after_jump;
        current = next;
    }
}

fn reorder_exception_handler_conditional_continue_scope_exit_blocks(blocks: &mut [Block]) {
    fn handler_scope_exit_returns(block: &Block) -> bool {
        block
            .instructions
            .iter()
            .any(|info| matches!(info.instr.real(), Some(Instruction::PopExcept)))
            && block
                .instructions
                .last()
                .is_some_and(|info| matches!(info.instr.real(), Some(Instruction::ReturnValue)))
    }

    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let idx = current.idx();
        let next = blocks[idx].next;
        let Some(cond_idx) = trailing_conditional_jump_index(&blocks[idx]) else {
            current = next;
            continue;
        };
        let cond = blocks[idx].instructions[cond_idx];
        if !matches!(
            cond.instr.real(),
            Some(Instruction::PopJumpIfNotNone { .. } | Instruction::PopJumpIfFalse { .. })
        ) || !(block_is_exceptional(&blocks[idx]) || blocks[idx].cold)
        {
            current = next;
            continue;
        }

        let exit_start = next;
        let exit_block = next_nonempty_block(blocks, exit_start);
        let jump_start = cond.target;
        let jump_block = next_nonempty_block(blocks, jump_start);
        let jump_target = if jump_block != BlockIdx::NULL {
            next_nonempty_block(blocks, blocks[jump_block.idx()].instructions[0].target)
        } else {
            BlockIdx::NULL
        };
        let exit_raises = exit_block != BlockIdx::NULL
            && blocks[exit_block.idx()]
                .instructions
                .iter()
                .any(|info| matches!(info.instr.real(), Some(Instruction::RaiseVarargs { .. })));
        let exit_returns =
            exit_block != BlockIdx::NULL && handler_scope_exit_returns(&blocks[exit_block.idx()]);
        if exit_start == BlockIdx::NULL
            || exit_block == BlockIdx::NULL
            || jump_start == BlockIdx::NULL
            || jump_block == BlockIdx::NULL
            || jump_target == BlockIdx::NULL
            || exit_block == jump_block
            || !is_scope_exit_block(&blocks[exit_block.idx()])
            || !is_jump_back_only_block(blocks, jump_block)
            || !(exit_raises || exit_returns)
            || !blocks[jump_target.idx()]
                .instructions
                .first()
                .is_some_and(|info| matches!(info.instr.real(), Some(Instruction::ForIter { .. })))
            || next_nonempty_block(blocks, blocks[exit_block.idx()].next) != jump_block
        {
            current = next;
            continue;
        }

        let after_jump = blocks[jump_block.idx()].next;
        blocks[idx].instructions[cond_idx].instr = reversed_conditional(&cond.instr)
            .expect("handler conditional has a reversed conditional");
        blocks[idx].instructions[cond_idx].target = exit_start;
        blocks[idx].next = jump_start;
        blocks[jump_block.idx()].next = exit_start;
        blocks[exit_block.idx()].next = after_jump;
        current = next;
    }
}

fn deduplicate_adjacent_jump_back_blocks(blocks: &mut [Block]) {
    fn jump_back_target(block: &Block) -> Option<BlockIdx> {
        let [info] = block.instructions.as_slice() else {
            return None;
        };
        if matches!(info.instr.real(), Some(Instruction::JumpBackward { .. }))
            && info.target != BlockIdx::NULL
        {
            Some(info.target)
        } else {
            None
        }
    }

    fn jump_back_lineno(block: &Block) -> Option<i32> {
        let [info] = block.instructions.as_slice() else {
            return None;
        };
        matches!(info.instr.real(), Some(Instruction::JumpBackward { .. }))
            .then(|| instruction_lineno(info))
    }

    fn has_protected_conditional_predecessor(
        blocks: &[Block],
        incoming_origins: &[Vec<BlockIdx>],
        target: BlockIdx,
    ) -> bool {
        incoming_origins[target.idx()]
            .iter()
            .copied()
            .any(|origin| {
                blocks[origin.idx()].instructions.iter().any(|info| {
                    info.except_handler.is_some()
                        && is_conditional_jump(&info.instr)
                        && next_nonempty_block(blocks, info.target) == target
                })
            })
    }

    let reachable = compute_reachable_blocks(blocks);
    let incoming_origins = compute_incoming_origins(blocks, &reachable);
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let Some(target) = jump_back_target(&blocks[current.idx()]) else {
            current = blocks[current.idx()].next;
            continue;
        };
        if block_is_exceptional(&blocks[current.idx()])
            || block_is_protected(&blocks[current.idx()])
        {
            current = blocks[current.idx()].next;
            continue;
        }

        let duplicate = next_nonempty_block(blocks, blocks[current.idx()].next);
        if duplicate == BlockIdx::NULL
            || duplicate == current
            || block_is_exceptional(&blocks[duplicate.idx()])
            || block_is_protected(&blocks[duplicate.idx()])
            || jump_back_target(&blocks[duplicate.idx()]) != Some(target)
            || jump_back_lineno(&blocks[duplicate.idx()])
                != jump_back_lineno(&blocks[current.idx()])
            || has_protected_conditional_predecessor(blocks, &incoming_origins, current)
            || has_protected_conditional_predecessor(blocks, &incoming_origins, duplicate)
        {
            current = blocks[current.idx()].next;
            continue;
        }

        for block in blocks.iter_mut() {
            for info in &mut block.instructions {
                if info.target == duplicate {
                    info.target = current;
                }
            }
        }
        current = blocks[current.idx()].next;
    }
}

fn reorder_conditional_body_and_implicit_continue_blocks(blocks: &mut Vec<Block>) {
    fn jump_back_target(blocks: &[Block], block_idx: BlockIdx) -> Option<BlockIdx> {
        if block_idx == BlockIdx::NULL {
            return None;
        }
        let instructions = blocks[block_idx.idx()].instructions.as_slice();
        let jump = match instructions {
            [jump] if jump.instr.is_unconditional_jump() => jump,
            [not_taken, jump]
                if matches!(not_taken.instr.real(), Some(Instruction::NotTaken))
                    && jump.instr.is_unconditional_jump() =>
            {
                jump
            }
            _ => return None,
        };
        if jump.target == BlockIdx::NULL
            || !comes_before(blocks, next_nonempty_block(blocks, jump.target), block_idx)
        {
            return None;
        }
        Some(jump.target)
    }

    fn find_body_tail(
        blocks: &[Block],
        body_start: BlockIdx,
        false_jump: BlockIdx,
        target: BlockIdx,
    ) -> Option<BlockIdx> {
        let mut cursor = body_start;
        let mut visited = vec![false; blocks.len()];
        while cursor != BlockIdx::NULL && cursor != false_jump {
            if visited[cursor.idx()] {
                return None;
            }
            visited[cursor.idx()] = true;
            if block_is_exceptional(&blocks[cursor.idx()]) {
                return None;
            }
            if jump_back_target(blocks, cursor) == Some(target) {
                return Some(cursor);
            }
            cursor = blocks[cursor.idx()].next;
        }
        None
    }

    fn find_body_tail_before_jump(
        blocks: &[Block],
        body_start: BlockIdx,
        jump_start: BlockIdx,
    ) -> Option<BlockIdx> {
        let mut cursor = body_start;
        let mut tail = BlockIdx::NULL;
        let mut visited = vec![false; blocks.len()];
        while cursor != BlockIdx::NULL && cursor != jump_start {
            if visited[cursor.idx()] {
                return None;
            }
            visited[cursor.idx()] = true;
            if block_is_exceptional(&blocks[cursor.idx()]) {
                return None;
            }
            tail = cursor;
            cursor = blocks[cursor.idx()].next;
        }
        (tail != BlockIdx::NULL && cursor == jump_start).then_some(tail)
    }

    fn body_segment_contains_for_iter(
        blocks: &[Block],
        body_start: BlockIdx,
        body_tail: BlockIdx,
    ) -> bool {
        let mut cursor = body_start;
        let mut visited = vec![false; blocks.len()];
        while cursor != BlockIdx::NULL {
            if visited[cursor.idx()] {
                return false;
            }
            visited[cursor.idx()] = true;
            if blocks[cursor.idx()]
                .instructions
                .iter()
                .any(|info| matches!(info.instr.real(), Some(Instruction::ForIter { .. })))
            {
                return true;
            }
            if cursor == body_tail {
                return false;
            }
            cursor = blocks[cursor.idx()].next;
        }
        false
    }

    fn body_segment_contains_protected_block(
        blocks: &[Block],
        body_start: BlockIdx,
        body_tail: BlockIdx,
    ) -> bool {
        let mut cursor = body_start;
        let mut visited = vec![false; blocks.len()];
        while cursor != BlockIdx::NULL {
            if visited[cursor.idx()] {
                return false;
            }
            visited[cursor.idx()] = true;
            if block_is_protected(&blocks[cursor.idx()]) {
                return true;
            }
            if cursor == body_tail {
                return false;
            }
            cursor = blocks[cursor.idx()].next;
        }
        false
    }

    fn body_segment_contains_scope_exit(
        blocks: &[Block],
        body_start: BlockIdx,
        body_tail: BlockIdx,
    ) -> bool {
        let mut cursor = body_start;
        let mut visited = vec![false; blocks.len()];
        while cursor != BlockIdx::NULL {
            if visited[cursor.idx()] {
                return false;
            }
            visited[cursor.idx()] = true;
            if blocks[cursor.idx()]
                .instructions
                .iter()
                .any(|info| info.instr.is_scope_exit())
            {
                return true;
            }
            if cursor == body_tail {
                return false;
            }
            cursor = blocks[cursor.idx()].next;
        }
        false
    }

    fn body_segment_contains_jump_back_to(
        blocks: &[Block],
        body_start: BlockIdx,
        body_tail: BlockIdx,
        target: BlockIdx,
    ) -> bool {
        let mut cursor = body_start;
        let mut visited = vec![false; blocks.len()];
        while cursor != BlockIdx::NULL {
            if visited[cursor.idx()] {
                return false;
            }
            visited[cursor.idx()] = true;
            if jump_back_target(blocks, cursor) == Some(target) {
                return true;
            }
            if cursor == body_tail {
                return false;
            }
            cursor = blocks[cursor.idx()].next;
        }
        false
    }

    fn body_segment_contains_any_jump_back(
        blocks: &[Block],
        body_start: BlockIdx,
        body_tail: BlockIdx,
    ) -> bool {
        fn jump_back_or_self_target(blocks: &[Block], block_idx: BlockIdx) -> Option<BlockIdx> {
            if block_idx == BlockIdx::NULL {
                return None;
            }
            let jump = blocks[block_idx.idx()].instructions.last()?;
            if !jump.instr.is_unconditional_jump() {
                return None;
            }
            if jump.target == BlockIdx::NULL {
                return None;
            }
            let target = next_nonempty_block(blocks, jump.target);
            if target == block_idx || comes_before(blocks, target, block_idx) {
                Some(jump.target)
            } else {
                None
            }
        }

        let mut cursor = body_start;
        let mut visited = vec![false; blocks.len()];
        while cursor != BlockIdx::NULL {
            if visited[cursor.idx()] {
                return false;
            }
            visited[cursor.idx()] = true;
            if jump_back_or_self_target(blocks, cursor).is_some() {
                return true;
            }
            if cursor == body_tail {
                return false;
            }
            cursor = blocks[cursor.idx()].next;
        }
        false
    }

    fn block_ends_with_jump_back_to(
        blocks: &[Block],
        block_idx: BlockIdx,
        target: BlockIdx,
    ) -> bool {
        let Some(last) = blocks[block_idx.idx()].instructions.last() else {
            return false;
        };
        last.instr.is_unconditional_jump()
            && last.target != BlockIdx::NULL
            && next_nonempty_block(blocks, last.target) == next_nonempty_block(blocks, target)
            && comes_before(blocks, next_nonempty_block(blocks, target), block_idx)
    }

    fn conditional_jump_target_count(blocks: &[Block], target: BlockIdx) -> usize {
        let target = next_nonempty_block(blocks, target);
        blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .filter(|info| {
                is_conditional_jump(&info.instr)
                    && info.target != BlockIdx::NULL
                    && next_nonempty_block(blocks, info.target) == target
            })
            .count()
    }

    fn empty_chain_reaches(blocks: &[Block], start: BlockIdx, target: BlockIdx) -> bool {
        let mut cursor = start;
        let mut visited = vec![false; blocks.len()];
        while cursor != BlockIdx::NULL && cursor != target {
            if visited[cursor.idx()]
                || block_is_exceptional(&blocks[cursor.idx()])
                || block_is_protected(&blocks[cursor.idx()])
                || !blocks[cursor.idx()].instructions.is_empty()
            {
                return false;
            }
            visited[cursor.idx()] = true;
            cursor = blocks[cursor.idx()].next;
        }
        cursor == target
    }

    fn block_starts_loop_cleanup(blocks: &[Block], block_idx: BlockIdx) -> bool {
        block_idx != BlockIdx::NULL
            && matches!(
                blocks[block_idx.idx()]
                    .instructions
                    .first()
                    .and_then(|info| info.instr.real()),
                Some(Instruction::EndFor | Instruction::PopIter)
            )
    }

    fn is_single_delete_subscr_body(block: &Block) -> bool {
        let real: Vec<_> = block
            .instructions
            .iter()
            .filter_map(|info| info.instr.real())
            .filter(|instr| !matches!(instr, Instruction::Nop | Instruction::NotTaken))
            .collect();
        real.iter()
            .filter(|instr| matches!(instr, Instruction::DeleteSubscr))
            .count()
            == 1
            && matches!(real.last(), Some(Instruction::DeleteSubscr))
            && real.iter().all(|instr| {
                matches!(
                    instr,
                    Instruction::LoadFast { .. }
                        | Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastLoadFast { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                        | Instruction::LoadSmallInt { .. }
                        | Instruction::LoadConst { .. }
                        | Instruction::Copy { .. }
                        | Instruction::Swap { .. }
                        | Instruction::BinaryOp { .. }
                        | Instruction::BinarySlice
                        | Instruction::BuildSlice { .. }
                        | Instruction::StoreSubscr
                        | Instruction::DeleteSubscr
                )
            })
    }

    fn block_has_call(block: &Block) -> bool {
        block
            .instructions
            .iter()
            .any(|info| matches!(info.instr.real(), Some(Instruction::Call { .. })))
    }

    fn protected_region_has_prior_scope_exit(
        blocks: &[Block],
        loop_target: BlockIdx,
        current: BlockIdx,
    ) -> bool {
        let mut cursor = next_nonempty_block(blocks, loop_target);
        let mut visited = vec![false; blocks.len()];
        let mut saw_protected = false;
        while cursor != BlockIdx::NULL && cursor != current {
            if visited[cursor.idx()] {
                return false;
            }
            visited[cursor.idx()] = true;
            let block = &blocks[cursor.idx()];
            saw_protected |= block_is_protected(block);
            if saw_protected
                && block
                    .instructions
                    .iter()
                    .any(|info| info.instr.is_scope_exit())
            {
                return true;
            }
            cursor = block.next;
        }
        false
    }

    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let idx = current.idx();
        let next = blocks[idx].next;
        let Some(cond_idx) = trailing_conditional_jump_index(&blocks[idx]) else {
            current = next;
            continue;
        };
        let cond = blocks[idx].instructions[cond_idx];
        let Some(reversed_cond) = reversed_conditional(&cond.instr) else {
            current = next;
            continue;
        };

        let true_jump_start = cond.target;
        let true_jump = next_nonempty_block(blocks, true_jump_start);
        let body_start = next;
        let body = next_nonempty_block(blocks, body_start);
        let true_jump_loop_target = jump_back_target(blocks, true_jump);
        let normalized_forward_conditional = blocks[idx]
            .instructions
            .get(cond_idx + 1)
            .is_some_and(|info| matches!(info.instr.real(), Some(Instruction::NotTaken)));
        if true_jump_loop_target.is_some()
            && (true_jump_start == true_jump
                || empty_chain_reaches(blocks, true_jump_start, true_jump))
            && body_start != BlockIdx::NULL
            && body != BlockIdx::NULL
            && true_jump != BlockIdx::NULL
            && !block_is_exceptional(&blocks[idx])
            && !block_is_exceptional(&blocks[true_jump.idx()])
            && !block_is_protected(&blocks[idx])
            && !block_is_protected(&blocks[true_jump.idx()])
            && next_nonempty_block(blocks, blocks[true_jump.idx()].next) != body
            && let Some(body_tail) = find_body_tail_before_jump(blocks, body, true_jump_start)
        {
            let loop_target = true_jump_loop_target.expect("checked above");
            let after_jump = blocks[true_jump.idx()].next;
            let body_tail_is_conditional =
                trailing_conditional_jump_index(&blocks[body_tail.idx()]).is_some();
            let body_is_single_block = body == body_tail;
            let body_has_scope_exit = body_segment_contains_scope_exit(blocks, body, body_tail);
            let body_tail_has_scope_exit = is_scope_exit_block(&blocks[body_tail.idx()]);
            let body_starts_conditional_chain = block_is_pure_conditional_test(&blocks[body.idx()]);
            let body_tail_ends_with_loop_backedge =
                block_ends_with_jump_back_to(blocks, body_tail, loop_target);
            let body_has_loop_backedge = body_tail_ends_with_loop_backedge
                || body_segment_contains_jump_back_to(blocks, body, body_tail, loop_target);
            if body_has_scope_exit && !body_tail_has_scope_exit {
                current = next;
                continue;
            }
            let body_has_inner_for_iter = body_segment_contains_for_iter(blocks, body, body_tail);
            let body_has_any_loop_backedge = body_has_inner_for_iter
                && body_segment_contains_any_jump_back(blocks, body, body_tail);
            let normalized_single_block_can_reorder =
                !normalized_forward_conditional || !block_has_call(&blocks[body.idx()]);
            let has_exceptional_duplicate_condition_line =
                has_exceptional_duplicate_lineno(blocks, current, instruction_lineno(&cond));
            let has_prior_protected_scope_exit =
                protected_region_has_prior_scope_exit(blocks, loop_target, current);
            let after_jump_target = next_nonempty_block(blocks, after_jump);
            let after_jump_starts_loop_cleanup =
                block_starts_loop_cleanup(blocks, after_jump_target);
            let after_jump_continues_conditional_chain = after_jump_target != BlockIdx::NULL
                && block_is_pure_conditional_test(&blocks[after_jump_target.idx()]);
            let body_is_pop_top_exit_like =
                body_is_single_block && is_pop_top_exit_like_block(&blocks[body.idx()]);
            if (body_has_scope_exit || body_is_pop_top_exit_like)
                && after_jump_target != BlockIdx::NULL
                && is_pop_top_exit_like_block(&blocks[after_jump_target.idx()])
            {
                current = next;
                continue;
            }
            if after_jump_continues_conditional_chain {
                current = next;
                continue;
            }
            let jump_target_has_multiple_conditional_predecessors =
                conditional_jump_target_count(blocks, true_jump_start) > 1;
            let simple_single_block_can_reorder = body_is_single_block
                && !body_tail_is_conditional
                && !has_exceptional_duplicate_condition_line
                && after_jump_starts_loop_cleanup
                && !body_has_scope_exit
                && (!blocks[body.idx()]
                    .instructions
                    .iter()
                    .any(|info| info.instr.is_unconditional_jump())
                    || body_tail_ends_with_loop_backedge)
                && !after_jump_continues_conditional_chain
                && matches!(cond.instr.real(), Some(Instruction::PopJumpIfFalse { .. }));
            let trailing_implicit_continue_can_reorder = after_jump != BlockIdx::NULL
                && next_nonempty_block(blocks, after_jump) != body
                && (!after_jump_starts_loop_cleanup
                    || body_has_loop_backedge
                    || body_has_any_loop_backedge)
                && !(body_has_scope_exit
                    && after_jump_target != BlockIdx::NULL
                    && is_scope_exit_block(&blocks[after_jump_target.idx()])
                    && !body_starts_conditional_chain)
                && !is_scope_exit_block(&blocks[body.idx()]);
            let can_reorder = !has_exceptional_duplicate_condition_line
                && !has_prior_protected_scope_exit
                && (!jump_target_has_multiple_conditional_predecessors || body_tail_has_scope_exit)
                && ((!body_tail_is_conditional
                    && ((normalized_single_block_can_reorder
                        && body_is_single_block
                        && matches!(cond.instr.real(), Some(Instruction::PopJumpIfTrue { .. })))
                        || (body == body_tail
                            && is_single_delete_subscr_body(&blocks[body.idx()]))
                        || simple_single_block_can_reorder))
                    || (trailing_implicit_continue_can_reorder
                        && ((body_has_scope_exit && body_tail_has_scope_exit)
                            || body_has_loop_backedge
                            || (after_jump_starts_loop_cleanup && body_has_any_loop_backedge))
                        && !after_jump_continues_conditional_chain));
            if can_reorder && after_jump != BlockIdx::NULL && after_jump != body_start {
                blocks[idx].instructions[cond_idx].instr = reversed_cond;
                blocks[idx].instructions[cond_idx].target = body_start;
                if body_tail_ends_with_loop_backedge && true_jump_start == true_jump {
                    blocks[idx].next = true_jump_start;
                    blocks[true_jump.idx()].next = body_start;
                    blocks[body_tail.idx()].next = after_jump;
                } else {
                    let cloned_jump_idx = BlockIdx(blocks.len() as u32);
                    let mut cloned_jump = blocks[true_jump.idx()].clone();
                    cloned_jump.next = body_start;
                    blocks.push(cloned_jump);

                    blocks[idx].next = cloned_jump_idx;
                    blocks[body_tail.idx()].next = true_jump_start;
                }
                current = blocks[idx].next;
                continue;
            }
        }

        if !matches!(cond.instr.real(), Some(Instruction::PopJumpIfTrue { .. })) {
            current = next;
            continue;
        }

        let false_jump_start = next;
        let false_jump = next_nonempty_block(blocks, false_jump_start);
        let body_start = cond.target;
        let body = next_nonempty_block(blocks, body_start);
        let Some(loop_target) = jump_back_target(blocks, false_jump) else {
            current = next;
            continue;
        };
        if false_jump_start == BlockIdx::NULL
            || false_jump == BlockIdx::NULL
            || body_start == BlockIdx::NULL
            || body == BlockIdx::NULL
            || false_jump == body
            || block_is_exceptional(&blocks[idx])
            || block_is_exceptional(&blocks[false_jump.idx()])
            || block_is_exceptional(&blocks[body.idx()])
            || block_is_protected(&blocks[idx])
            || block_is_protected(&blocks[false_jump.idx()])
            || block_is_protected(&blocks[body.idx()])
            || next_nonempty_block(blocks, blocks[false_jump.idx()].next) != body
        {
            current = next;
            continue;
        }

        let Some(body_tail) = find_body_tail(blocks, body, false_jump, loop_target) else {
            current = next;
            continue;
        };
        let after_body = blocks[body_tail.idx()].next;
        if after_body == BlockIdx::NULL || after_body == false_jump_start {
            current = next;
            continue;
        }
        let body_contains_for_iter = body_segment_contains_for_iter(blocks, body, body_tail);
        let simple_body_can_reorder = !block_has_call(&blocks[body.idx()])
            && jump_back_target(blocks, body_tail) == Some(loop_target);
        let false_jump_starts_with_not_taken = blocks[false_jump.idx()]
            .instructions
            .first()
            .is_some_and(|info| matches!(info.instr.real(), Some(Instruction::NotTaken)));
        if simple_body_can_reorder && false_jump_starts_with_not_taken && !body_contains_for_iter {
            current = next;
            continue;
        }
        if (!body_contains_for_iter && !simple_body_can_reorder)
            || body_segment_contains_protected_block(blocks, body, body_tail)
            || trailing_conditional_jump_index(&blocks[body.idx()]).is_some()
            || block_starts_loop_cleanup(blocks, next_nonempty_block(blocks, after_body))
        {
            current = next;
            continue;
        }

        if blocks[idx]
            .instructions
            .get(cond_idx + 1)
            .is_none_or(|info| !matches!(info.instr.real(), Some(Instruction::NotTaken)))
            && matches!(
                blocks[false_jump.idx()]
                    .instructions
                    .first()
                    .and_then(|info| info.instr.real()),
                Some(Instruction::NotTaken)
            )
        {
            let not_taken = blocks[false_jump.idx()].instructions.remove(0);
            blocks[idx].instructions.insert(cond_idx + 1, not_taken);
        }
        blocks[idx].instructions[cond_idx].instr =
            reversed_conditional(&cond.instr).expect("PopJumpIfTrue has a reversed conditional");
        blocks[idx].instructions[cond_idx].target = false_jump_start;
        blocks[idx].next = body_start;
        blocks[body_tail.idx()].next = false_jump_start;
        blocks[false_jump.idx()].next = after_body;
        current = blocks[idx].next;
    }
}

#[allow(dead_code)]
fn reorder_jump_over_exception_cleanup_blocks(blocks: &mut [Block]) {
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let idx = current.idx();
        let next = blocks[idx].next;
        if blocks[idx].cold && is_with_suppress_exit_block(&blocks[idx]) {
            current = next;
            continue;
        }
        let Some(last) = blocks[idx].instructions.last().copied() else {
            current = next;
            continue;
        };
        if !matches!(last.instr.real(), Some(Instruction::JumpForward { .. }))
            || last.target == BlockIdx::NULL
        {
            current = next;
            continue;
        }

        let cleanup_start = next;
        let target_start = last.target;
        let target = next_nonempty_block(blocks, target_start);
        if cleanup_start == BlockIdx::NULL || target == BlockIdx::NULL || cleanup_start == target {
            current = next;
            continue;
        }
        // Keep the target anchored to the first target block. If we have to
        // skip leading empty blocks here, reordering can leave the jump shape
        // inconsistent in nested cleanup chains such as poplib.POP3.close().
        if target_start != target {
            current = next;
            continue;
        }

        let mut cleanup_end = BlockIdx::NULL;
        let mut saw_exceptional = false;
        let mut cursor = cleanup_start;
        while cursor != BlockIdx::NULL && cursor != target {
            if blocks[cursor.idx()].instructions.is_empty() {
                cleanup_end = cursor;
                cursor = blocks[cursor.idx()].next;
                continue;
            }
            if !(block_is_exceptional(&blocks[cursor.idx()])
                || is_exception_cleanup_block(&blocks[cursor.idx()])
                || blocks[cursor.idx()].cold && is_reraise_scope_exit_block(&blocks[cursor.idx()]))
            {
                cleanup_end = BlockIdx::NULL;
                break;
            }
            saw_exceptional = true;
            cleanup_end = cursor;
            cursor = blocks[cursor.idx()].next;
        }
        if !saw_exceptional || cleanup_end == BlockIdx::NULL || cursor != target {
            current = next;
            continue;
        }

        let mut target_end = BlockIdx::NULL;
        let mut target_exit = BlockIdx::NULL;
        let mut nonempty_target_blocks = 0usize;
        cursor = target;
        while cursor != BlockIdx::NULL {
            if block_is_exceptional(&blocks[cursor.idx()]) {
                break;
            }
            target_end = cursor;
            if !blocks[cursor.idx()].instructions.is_empty() {
                nonempty_target_blocks += 1;
                target_exit = cursor;
            }
            cursor = blocks[cursor.idx()].next;
        }

        const MAX_REORDERED_EXIT_BLOCK_SIZE: usize = 4;

        if target_end == BlockIdx::NULL
            || target_exit == BlockIdx::NULL
            || nonempty_target_blocks != 1
            || target_exit != target_end
            || !is_scope_exit_block(&blocks[target_exit.idx()])
            || blocks[target_exit.idx()].instructions.len() > MAX_REORDERED_EXIT_BLOCK_SIZE
            || blocks[target_exit.idx()]
                .instructions
                .iter()
                .any(|info| info.instr.is_block_push())
        {
            current = next;
            continue;
        }

        let after_target = blocks[target_end.idx()].next;
        blocks[idx].next = target_start;
        blocks[target_end.idx()].next = cleanup_start;
        blocks[cleanup_end.idx()].next = after_target;
        current = after_target;
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

fn overwrite_location(
    instr: &mut InstructionInfo,
    location: SourceLocation,
    end_location: SourceLocation,
) {
    instr.location = location;
    instr.end_location = end_location;
    instr.lineno_override = None;
}

fn compute_reachable_blocks(blocks: &[Block]) -> Vec<bool> {
    let mut reachable = vec![false; blocks.len()];
    if blocks.is_empty() {
        return reachable;
    }

    reachable[0] = true;
    let mut changed = true;
    while changed {
        changed = false;
        for i in 0..blocks.len() {
            if !reachable[i] {
                continue;
            }
            for ins in &blocks[i].instructions {
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
            let next = blocks[i].next;
            if next != BlockIdx::NULL
                && !reachable[next.idx()]
                && !blocks[i].instructions.last().is_some_and(|ins| {
                    ins.instr.is_scope_exit() || ins.instr.is_unconditional_jump()
                })
            {
                reachable[next.idx()] = true;
                changed = true;
            }
        }
    }

    reachable
}

fn compute_predecessors(blocks: &[Block]) -> Vec<u32> {
    let mut predecessors = vec![0u32; blocks.len()];
    if blocks.is_empty() {
        return predecessors;
    }

    let reachable = compute_reachable_blocks(blocks);
    predecessors[0] = 1;
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        if !reachable[current.idx()] {
            current = blocks[current.idx()].next;
            continue;
        }

        let block = &blocks[current.idx()];
        if block_has_fallthrough(block) {
            let next = next_nonempty_block(blocks, block.next);
            if next != BlockIdx::NULL && reachable[next.idx()] {
                predecessors[next.idx()] += 1;
            }
        }
        for ins in &block.instructions {
            if ins.target != BlockIdx::NULL {
                let target = next_nonempty_block(blocks, ins.target);
                if target != BlockIdx::NULL && reachable[target.idx()] {
                    predecessors[target.idx()] += 1;
                }
            }
        }
        current = block.next;
    }
    predecessors
}

fn record_incoming_origin(origins: &mut [Vec<BlockIdx>], target: BlockIdx, source: BlockIdx) {
    let incoming = &mut origins[target.idx()];
    if !incoming.contains(&source) {
        incoming.push(source);
    }
}

fn compute_incoming_origins(blocks: &[Block], reachable: &[bool]) -> Vec<Vec<BlockIdx>> {
    let mut origins = vec![Vec::new(); blocks.len()];
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        if !reachable[current.idx()] {
            current = blocks[current.idx()].next;
            continue;
        }

        let block = &blocks[current.idx()];
        if block_has_fallthrough(block) {
            let next = next_nonempty_block(blocks, block.next);
            if next != BlockIdx::NULL && reachable[next.idx()] {
                record_incoming_origin(&mut origins, next, current);
            }
        }
        for ins in &block.instructions {
            if ins.target != BlockIdx::NULL {
                let target = next_nonempty_block(blocks, ins.target);
                if target != BlockIdx::NULL && reachable[target.idx()] {
                    record_incoming_origin(&mut origins, target, current);
                }
            }
        }
        current = block.next;
    }
    origins
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
        if target == BlockIdx::NULL {
            current = blocks[current.idx()].next;
            continue;
        }
        let target_is_exit_without_lineno = is_exit_without_lineno(blocks, target);
        let target_is_protected_eval_break =
            is_eval_break_without_lineno(blocks, target) && last.except_handler.is_some();
        if !target_is_exit_without_lineno && !target_is_protected_eval_break {
            current = blocks[current.idx()].next;
            continue;
        }
        if predecessors[target.idx()] <= 1 {
            current = blocks[current.idx()].next;
            continue;
        }

        // Copy the exit block and splice it into the linked list after the
        // original target block, matching CPython's copy_basicblock() layout.
        let new_idx = BlockIdx(blocks.len() as u32);
        let mut new_block = blocks[target.idx()].clone();
        if let Some(first) = new_block.instructions.first_mut()
            && let Some((location, end_location)) = propagation_location(last)
        {
            overwrite_location(first, location, end_location);
        }
        let old_next = blocks[target.idx()].next;
        new_block.next = old_next;
        blocks.push(new_block);
        blocks[target.idx()].next = new_idx;

        // Update the jump target
        let last_mut = blocks[current.idx()].instructions.last_mut().unwrap();
        last_mut.target = new_idx;
        predecessors[target.idx()] -= 1;
        predecessors.push(1);
        current = blocks[current.idx()].next;
    }

    let reachable = compute_reachable_blocks(blocks);
    let incoming_origins = compute_incoming_origins(blocks, &reachable);
    current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let block = &blocks[current.idx()];
        if let Some(last) = block.instructions.last()
            && block_has_fallthrough(block)
        {
            let target = next_nonempty_block(blocks, block.next);
            if target != BlockIdx::NULL
                && (predecessors[target.idx()] == 1
                    || has_unique_fallthrough_origin(
                        blocks,
                        &reachable,
                        &incoming_origins,
                        current,
                        target,
                    ))
                && (is_exit_without_lineno(blocks, target)
                    || is_eval_break_without_lineno(blocks, target))
                && let Some((location, end_location)) = propagation_location(last)
                && let Some(first) = blocks[target.idx()].instructions.first_mut()
            {
                maybe_propagate_location(first, location, end_location);
            }
        }
        current = blocks[current.idx()].next;
    }
}

fn propagate_line_numbers(blocks: &mut [Block], predecessors: &[u32]) {
    let reachable = compute_reachable_blocks(blocks);
    let incoming_origins = compute_incoming_origins(blocks, &reachable);
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        if !blocks[current.idx()].instructions.is_empty() {
            let (next_block, has_fallthrough) = {
                let block = &blocks[current.idx()];
                (block.next, block_has_fallthrough(block))
            };

            let prev_location = {
                let block = &mut blocks[current.idx()];
                let mut prev_location = None;
                for instr in &mut block.instructions {
                    if let Some((location, end_location)) = prev_location {
                        maybe_propagate_location(instr, location, end_location);
                    }
                    prev_location = propagation_location(instr);
                }
                prev_location
            };
            let last = blocks[current.idx()].instructions.last().copied().unwrap();

            if has_fallthrough {
                let target = next_nonempty_block(blocks, next_block);
                if target != BlockIdx::NULL
                    && (predecessors[target.idx()] == 1
                        || has_unique_fallthrough_origin(
                            blocks,
                            &reachable,
                            &incoming_origins,
                            current,
                            target,
                        ))
                    && let Some((location, end_location)) = prev_location
                    && let Some(first) = blocks[target.idx()].instructions.first_mut()
                {
                    maybe_propagate_location(first, location, end_location);
                }
            }

            if is_jump_instruction(&last) {
                let mut target = next_nonempty_block(blocks, last.target);
                while target != BlockIdx::NULL
                    && blocks[target.idx()].instructions.is_empty()
                    && predecessors[target.idx()] == 1
                {
                    target = blocks[target.idx()].next;
                }
                if target != BlockIdx::NULL
                    && (predecessors[target.idx()] == 1
                        || has_unique_jump_origin(
                            blocks,
                            &reachable,
                            &incoming_origins,
                            current,
                            target,
                        ))
                    && let Some((location, end_location)) = prev_location
                    && let Some(first) = blocks[target.idx()].instructions.first_mut()
                {
                    maybe_propagate_location(first, location, end_location);
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

fn find_layout_predecessor(blocks: &[Block], target: BlockIdx) -> BlockIdx {
    if target == BlockIdx::NULL {
        return BlockIdx::NULL;
    }
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        if blocks[current.idx()].next == target {
            return current;
        }
        current = blocks[current.idx()].next;
    }
    BlockIdx::NULL
}

fn compute_layout_predecessors(blocks: &[Block]) -> Vec<BlockIdx> {
    let mut predecessors = vec![BlockIdx::NULL; blocks.len()];
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let next = blocks[current.idx()].next;
        if next != BlockIdx::NULL {
            predecessors[next.idx()] = current;
        }
        current = next;
    }
    predecessors
}

fn has_unique_fallthrough_origin(
    blocks: &[Block],
    reachable: &[bool],
    incoming_origins: &[Vec<BlockIdx>],
    source: BlockIdx,
    target: BlockIdx,
) -> bool {
    if source == BlockIdx::NULL
        || target == BlockIdx::NULL
        || !reachable[source.idx()]
        || !block_has_fallthrough(&blocks[source.idx()])
        || next_nonempty_block(blocks, blocks[source.idx()].next) != target
    {
        return false;
    }

    let chain_start = blocks[source.idx()].next;
    let mut current = chain_start;
    while current != target {
        if current == BlockIdx::NULL {
            return false;
        }
        if !blocks[current.idx()].instructions.is_empty() {
            return false;
        }
        current = blocks[current.idx()].next;
    }

    fn empty_chain_contains(
        blocks: &[Block],
        mut current: BlockIdx,
        target: BlockIdx,
        needle: BlockIdx,
    ) -> bool {
        while current != target {
            if current == needle {
                return true;
            }
            current = blocks[current.idx()].next;
        }
        false
    }

    incoming_origins[target.idx()].iter().all(|&origin| {
        origin == source || empty_chain_contains(blocks, chain_start, target, origin)
    })
}

fn has_unique_jump_origin(
    blocks: &[Block],
    reachable: &[bool],
    incoming_origins: &[Vec<BlockIdx>],
    source: BlockIdx,
    target: BlockIdx,
) -> bool {
    if source == BlockIdx::NULL
        || target == BlockIdx::NULL
        || !reachable[source.idx()]
        || !blocks[source.idx()]
            .instructions
            .last()
            .is_some_and(|instr| is_jump_instruction(instr) && instr.target != BlockIdx::NULL)
    {
        return false;
    }

    incoming_origins[target.idx()].iter().all(|&origin| {
        origin == source
            || (blocks[origin.idx()].instructions.is_empty()
                && next_nonempty_block(blocks, blocks[origin.idx()].next) == target)
    })
}

fn comes_before(blocks: &[Block], first: BlockIdx, second: BlockIdx) -> bool {
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        if current == first {
            return true;
        }
        if current == second {
            return false;
        }
        current = blocks[current.idx()].next;
    }
    false
}

fn duplicate_shared_jump_back_targets(blocks: &mut Vec<Block>) {
    let predecessors = compute_predecessors(blocks);
    let mut clones = Vec::new();
    let mut lineful_clones_before_target = Vec::new();
    let mut block_order = vec![usize::MAX; blocks.len()];
    let mut current = BlockIdx(0);
    let mut pos = 0usize;
    while current != BlockIdx::NULL {
        block_order[current.idx()] = pos;
        pos += 1;
        current = blocks[current.idx()].next;
    }

    for target in 0..blocks.len() {
        let target = BlockIdx(target as u32);
        if is_jump_back_only_block(blocks, target)
            && instruction_lineno(&blocks[target.idx()].instructions[0]) >= 0
        {
            let jump_target =
                next_nonempty_block(blocks, blocks[target.idx()].instructions[0].target);
            let layout_pred = find_layout_predecessor(blocks, target);
            if jump_target != BlockIdx::NULL
                && comes_before(blocks, jump_target, target)
                && layout_pred != BlockIdx::NULL
                && !block_has_fallthrough(&blocks[layout_pred.idx()])
                && predecessors[target.idx()] >= 2
            {
                let target_location = blocks[target.idx()].instructions[0].location;
                let target_end_location = blocks[target.idx()].instructions[0].end_location;
                let target_follows_forward_jump = blocks[layout_pred.idx()]
                    .instructions
                    .last()
                    .is_some_and(|info| {
                        matches!(info.instr.real(), Some(Instruction::JumpForward { .. }))
                    });
                for block_idx in 0..blocks.len() {
                    let block_idx = BlockIdx(block_idx as u32);
                    for (instr_idx, info) in blocks[block_idx.idx()].instructions.iter().enumerate()
                    {
                        if !is_conditional_jump(&info.instr)
                            || info.target == BlockIdx::NULL
                            || next_nonempty_block(blocks, info.target) != target
                            || block_order[block_idx.idx()] >= block_order[target.idx()]
                        {
                            continue;
                        }
                        let jump_lineno = instruction_lineno(info);
                        // CPython codegen_if() gives each nested if/elif arm
                        // its own end/next labels.  When those labels collapse
                        // to the same loop backedge, keep distinct lineful
                        // jump-back blocks unless the incoming conditional and
                        // the existing target came from the same source span.
                        if target_follows_forward_jump && target_location.line == info.location.line
                        {
                            continue;
                        }
                        if jump_lineno >= 0
                            && (info.location != target_location
                                || info.end_location != target_end_location)
                        {
                            lineful_clones_before_target.push((target, block_idx, instr_idx));
                        }
                    }
                }
            }
        }

        let Some(jump_target) = shared_jump_back_target(&blocks[target.idx()]) else {
            continue;
        };
        if blocks[target.idx()]
            .instructions
            .iter()
            .any(|info| matches!(info.instr.real(), Some(Instruction::PopExcept)))
        {
            continue;
        }

        let jump_target = next_nonempty_block(blocks, jump_target);
        if jump_target == BlockIdx::NULL || !comes_before(blocks, jump_target, target) {
            continue;
        }
        if !has_non_exception_loop_backedge_to(blocks, target, jump_target) {
            continue;
        }

        let layout_pred = find_layout_predecessor(blocks, target);
        if layout_pred != BlockIdx::NULL
            && (!block_has_fallthrough(&blocks[layout_pred.idx()])
                || next_nonempty_block(blocks, blocks[layout_pred.idx()].next) != target)
            && predecessors[target.idx()] >= 2
        {
            let mut jump_predecessors = Vec::new();
            for block_idx in 0..blocks.len() {
                let block_idx = BlockIdx(block_idx as u32);
                for (instr_idx, info) in blocks[block_idx.idx()].instructions.iter().enumerate() {
                    if !is_jump_instruction(info) || info.target == BlockIdx::NULL {
                        continue;
                    }
                    if next_nonempty_block(blocks, info.target) == target {
                        jump_predecessors.push((block_idx, instr_idx));
                    }
                }
            }
            if jump_predecessors.len() >= 2
                && jump_predecessors.iter().all(|(block_idx, instr_idx)| {
                    is_conditional_jump(&blocks[block_idx.idx()].instructions[*instr_idx].instr)
                        && block_order[block_idx.idx()] < block_order[target.idx()]
                })
                && let Some((keep_block, keep_instr)) = jump_predecessors
                    .iter()
                    .max_by_key(|(block_idx, _)| block_order[block_idx.idx()])
                    .copied()
            {
                for (block_idx, instr_idx) in jump_predecessors {
                    if block_idx == keep_block && instr_idx == keep_instr {
                        continue;
                    }
                    clones.push((target, block_idx, instr_idx));
                }
                continue;
            }
        }

        if layout_pred == BlockIdx::NULL
            || !block_has_fallthrough(&blocks[layout_pred.idx()])
            || next_nonempty_block(blocks, blocks[layout_pred.idx()].next) != target
            || predecessors[target.idx()] < 2
        {
            continue;
        }

        for block_idx in 0..blocks.len() {
            let block_idx = BlockIdx(block_idx as u32);
            if block_idx == target || block_idx == layout_pred {
                continue;
            }

            for (instr_idx, info) in blocks[block_idx.idx()].instructions.iter().enumerate() {
                if !is_jump_instruction(info) || info.target == BlockIdx::NULL {
                    continue;
                }
                if next_nonempty_block(blocks, info.target) != target {
                    continue;
                }
                clones.push((target, block_idx, instr_idx));
            }
        }
    }

    lineful_clones_before_target.sort_by_key(|(target, block_idx, instr_idx)| {
        let jump_lineno = instruction_lineno(&blocks[block_idx.idx()].instructions[*instr_idx]);
        (
            block_order[target.idx()],
            Reverse(jump_lineno),
            usize::MAX - block_order[block_idx.idx()],
        )
    });
    for (target, block_idx, instr_idx) in lineful_clones_before_target {
        if next_nonempty_block(
            blocks,
            blocks[block_idx.idx()].instructions[instr_idx].target,
        ) != target
        {
            continue;
        }
        let jump = blocks[block_idx.idx()].instructions[instr_idx];
        let layout_pred = find_layout_predecessor(blocks, target);
        if layout_pred == BlockIdx::NULL {
            continue;
        }

        let mut cloned = blocks[target.idx()].clone();
        if let Some(first) = cloned.instructions.first_mut() {
            overwrite_location(first, jump.location, jump.end_location);
        }
        let new_idx = BlockIdx(blocks.len() as u32);
        cloned.next = target;
        blocks.push(cloned);
        blocks[layout_pred.idx()].next = new_idx;
        blocks[block_idx.idx()].instructions[instr_idx].target = new_idx;
    }

    for (target, block_idx, instr_idx) in clones.into_iter().rev() {
        let jump = blocks[block_idx.idx()].instructions[instr_idx];
        let mut cloned = blocks[target.idx()].clone();
        if let Some(first) = cloned.instructions.first_mut() {
            overwrite_location(first, jump.location, jump.end_location);
        }

        let new_idx = BlockIdx(blocks.len() as u32);
        let old_next = blocks[target.idx()].next;
        cloned.next = old_next;
        blocks.push(cloned);
        blocks[target.idx()].next = new_idx;
        blocks[block_idx.idx()].instructions[instr_idx].target = new_idx;
    }
}

fn reorder_lineful_jump_back_runs_by_descending_line(blocks: &mut [Block]) {
    let mut prev = BlockIdx::NULL;
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let Some(first_target) = blocks[current.idx()]
            .instructions
            .first()
            .filter(|_| is_jump_back_only_block(blocks, current))
            .map(|info| next_nonempty_block(blocks, info.target))
        else {
            prev = current;
            current = blocks[current.idx()].next;
            continue;
        };
        if first_target == BlockIdx::NULL || !comes_before(blocks, first_target, current) {
            prev = current;
            current = blocks[current.idx()].next;
            continue;
        }

        let mut run = Vec::new();
        let mut scan = current;
        while scan != BlockIdx::NULL
            && is_jump_back_only_block(blocks, scan)
            && blocks[scan.idx()]
                .instructions
                .first()
                .is_some_and(|info| next_nonempty_block(blocks, info.target) == first_target)
            && instruction_lineno(&blocks[scan.idx()].instructions[0]) >= 0
        {
            run.push(scan);
            scan = blocks[scan.idx()].next;
        }

        if run.len() < 2 {
            prev = current;
            current = blocks[current.idx()].next;
            continue;
        }

        run.sort_by_key(|block_idx| {
            Reverse(instruction_lineno(&blocks[block_idx.idx()].instructions[0]))
        });
        if prev != BlockIdx::NULL {
            blocks[prev.idx()].next = run[0];
        }
        for pair in run.windows(2) {
            blocks[pair[0].idx()].next = pair[1];
        }
        let last = *run.last().expect("non-empty jump-back run");
        blocks[last.idx()].next = scan;
        prev = last;
        current = scan;
    }
}

fn duplicate_fallthrough_jump_back_targets(blocks: &mut Vec<Block>) {
    fn block_has_real_fallthrough_body(block: &Block) -> bool {
        block.instructions.iter().any(|info| {
            !matches!(
                info.instr.real(),
                Some(Instruction::Nop | Instruction::NotTaken | Instruction::PopTop)
            )
        })
    }

    let predecessors = compute_predecessors(blocks);
    let mut clones = Vec::new();

    let mut layout_pred = BlockIdx(0);
    while layout_pred != BlockIdx::NULL {
        if !block_has_fallthrough(&blocks[layout_pred.idx()])
            || !block_has_real_fallthrough_body(&blocks[layout_pred.idx()])
        {
            layout_pred = blocks[layout_pred.idx()].next;
            continue;
        }

        let target = next_nonempty_block(blocks, blocks[layout_pred.idx()].next);
        if target == BlockIdx::NULL
            || predecessors[target.idx()] < 2
            || !is_jump_back_only_block(blocks, target)
        {
            layout_pred = blocks[layout_pred.idx()].next;
            continue;
        }
        if blocks[target.idx()].instructions[0]
            .lineno_override
            .is_some_and(|lineno| lineno >= 0)
        {
            layout_pred = blocks[layout_pred.idx()].next;
            continue;
        }
        if !block_has_no_lineno(&blocks[target.idx()])
            && trailing_conditional_jump_index(&blocks[layout_pred.idx()]).is_some()
        {
            layout_pred = blocks[layout_pred.idx()].next;
            continue;
        }
        let jump_target = next_nonempty_block(blocks, blocks[target.idx()].instructions[0].target);
        if jump_target == BlockIdx::NULL
            || (!has_non_exception_loop_backedge_to(blocks, target, jump_target)
                && !block_has_non_exception_loop_backedge_to(blocks, target, jump_target))
        {
            layout_pred = blocks[layout_pred.idx()].next;
            continue;
        }

        let has_non_layout_jump_predecessor = blocks.iter().enumerate().any(|(idx, block)| {
            let block_idx = BlockIdx(idx as u32);
            block_idx != layout_pred
                && block_idx != target
                && block.instructions.iter().any(|info| {
                    is_jump_instruction(info)
                        && info.target != BlockIdx::NULL
                        && next_nonempty_block(blocks, info.target) == target
                })
        });
        if has_non_layout_jump_predecessor {
            clones.push((layout_pred, target));
        }

        layout_pred = blocks[layout_pred.idx()].next;
    }

    for (layout_pred, target) in clones.into_iter().rev() {
        if next_nonempty_block(blocks, blocks[layout_pred.idx()].next) != target {
            continue;
        }
        let Some(last) = blocks[layout_pred.idx()].instructions.last().copied() else {
            continue;
        };

        let new_idx = BlockIdx(blocks.len() as u32);
        let mut cloned = blocks[target.idx()].clone();
        if let Some(first) = cloned.instructions.first_mut() {
            overwrite_location(first, last.location, last.end_location);
        }
        cloned.next = blocks[layout_pred.idx()].next;
        blocks.push(cloned);
        blocks[layout_pred.idx()].next = new_idx;
    }
}

/// Duplicate `LOAD_CONST None + RETURN_VALUE` for blocks that fall through
/// to the final return block.
fn duplicate_end_returns(blocks: &mut Vec<Block>, metadata: &CodeUnitMetadata) {
    // Walk the block chain and keep the last non-cold non-empty block.
    // After cold exception handlers are pushed to the end, the mainline
    // return epilogue can sit before trailing cold blocks.
    let mut last_block = BlockIdx::NULL;
    let mut last_nonempty_block = BlockIdx::NULL;
    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        if !blocks[current.idx()].instructions.is_empty() {
            last_nonempty_block = current;
            if !blocks[current.idx()].cold {
                last_block = current;
            }
        }
        current = blocks[current.idx()].next;
    }
    if last_block == BlockIdx::NULL {
        last_block = last_nonempty_block;
    }
    if last_block == BlockIdx::NULL {
        return;
    }

    let last_insts = &blocks[last_block.idx()].instructions;
    // Only apply when the last block is EXACTLY a return-None epilogue.
    let is_return_block = last_insts.len() == 2
        && matches!(
            last_insts[0].instr,
            AnyInstruction::Real(Instruction::LoadConst { .. })
        )
        && is_load_const_none(&last_insts[0], metadata)
        && last_insts[0].no_location_exit
        && last_insts[1].no_location_exit
        && matches!(
            last_insts[1].instr,
            AnyInstruction::Real(Instruction::ReturnValue)
        );
    if !is_return_block {
        return;
    }

    // Get the return instructions to clone
    let return_insts: Vec<InstructionInfo> = last_insts[last_insts.len() - 2..].to_vec();
    let predecessors = compute_predecessors(blocks);

    // Find non-cold blocks that reach the last return block either by
    // fallthrough or as an unconditional jump target that should get its own
    // cloned epilogue.
    let mut fallthrough_blocks_to_fix = Vec::new();
    let mut jump_targets_to_fix = Vec::new();
    current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let block = &blocks[current.idx()];
        let next = next_nonempty_block(blocks, block.next);
        if current != last_block && !block.cold {
            let last_ins = block.instructions.last();
            let has_fallthrough = last_ins
                .is_none_or(|ins| !ins.instr.is_scope_exit() && !ins.instr.is_unconditional_jump());
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
            if !block.except_handler
                && next == last_block
                && has_fallthrough
                && trailing_conditional_jump_index(block).is_none()
                && !already_has_return
            {
                fallthrough_blocks_to_fix.push(current);
            }
            let jump_idx = trailing_conditional_jump_index(block).or_else(|| {
                block.instructions.last().and_then(|last| {
                    (last.instr.is_unconditional_jump() && last.target != BlockIdx::NULL)
                        .then_some(block.instructions.len() - 1)
                })
            });
            if let Some(jump_idx) = jump_idx {
                let jump = &block.instructions[jump_idx];
                if jump.target != BlockIdx::NULL
                    && next_nonempty_block(blocks, jump.target) == last_block
                    && (is_conditional_jump(&jump.instr) || predecessors[last_block.idx()] > 1)
                {
                    jump_targets_to_fix.push((current, jump_idx));
                }
            }
        }
        current = blocks[current.idx()].next;
    }

    // Duplicate the return instructions at the end of fall-through blocks
    for block_idx in fallthrough_blocks_to_fix {
        let propagated_location = blocks[block_idx.idx()]
            .instructions
            .last()
            .map(|instr| (instr.location, instr.end_location));
        let mut cloned_return = return_insts.clone();
        if !instruction_has_lineno(&cloned_return[0])
            && let Some((location, end_location)) = propagated_location
        {
            for instr in &mut cloned_return {
                overwrite_location(instr, location, end_location);
            }
        }
        blocks[block_idx.idx()].instructions.extend(cloned_return);
    }

    // Clone the final return block for jump predecessors so their target layout
    // matches CPython's duplicated exit blocks.
    for (block_idx, instr_idx) in jump_targets_to_fix.into_iter().rev() {
        let jump = blocks[block_idx.idx()].instructions[instr_idx];
        let mut cloned_return = return_insts.clone();
        if let Some(first) = cloned_return.first_mut() {
            overwrite_location(first, jump.location, jump.end_location);
        }
        let new_idx = BlockIdx(blocks.len() as u32);
        let is_conditional = is_conditional_jump(&jump.instr);
        let new_block = Block {
            cold: blocks[last_block.idx()].cold,
            except_handler: blocks[last_block.idx()].except_handler,
            disable_load_fast_borrow: blocks[last_block.idx()].disable_load_fast_borrow,
            instructions: cloned_return,
            next: if is_conditional {
                last_block
            } else {
                blocks[block_idx.idx()].next
            },
            ..Block::default()
        };
        blocks.push(new_block);
        if is_conditional {
            let layout_pred = find_layout_predecessor(blocks, last_block);
            if layout_pred != BlockIdx::NULL {
                blocks[layout_pred.idx()].next = new_idx;
            }
        } else {
            blocks[block_idx.idx()].next = new_idx;
        }
        blocks[block_idx.idx()].instructions[instr_idx].target = new_idx;
    }
}

fn inline_small_fast_return_blocks(blocks: &mut [Block]) {
    fn block_is_small_fast_return(block: &Block) -> bool {
        if block.instructions.len() > 3 {
            return false;
        }
        let real: Vec<_> = block
            .instructions
            .iter()
            .filter(|info| !matches!(info.instr.real(), Some(Instruction::Nop)))
            .collect();
        matches!(
            real.as_slice(),
            [load, ret]
                if matches!(
                    load.instr.real(),
                    Some(Instruction::LoadFast { .. } | Instruction::LoadFastBorrow { .. })
                ) && matches!(ret.instr.real(), Some(Instruction::ReturnValue))
        )
    }

    loop {
        let mut changed = false;
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
            let target = next_nonempty_block(blocks, last.target);
            if target == BlockIdx::NULL || !block_is_small_fast_return(&blocks[target.idx()]) {
                current = next;
                continue;
            }

            if jump_thread_kind(last.instr) == Some(JumpThreadKind::NoInterrupt)
                || instruction_has_lineno(&last)
            {
                let last_instr = blocks[current.idx()].instructions.last_mut().unwrap();
                let lineno_override = last_instr.lineno_override;
                set_to_nop(last_instr);
                last_instr.lineno_override = lineno_override;
            } else {
                blocks[current.idx()].instructions.pop();
            }
            let cloned = blocks[target.idx()].instructions.clone();
            blocks[current.idx()].instructions.extend(cloned);
            changed = true;
            current = next;
        }
        if !changed {
            break;
        }
    }
}

fn inline_with_suppress_return_blocks(blocks: &mut [Block]) {
    for block_idx in 0..blocks.len() {
        let Some(jump_idx) = blocks[block_idx].instructions.len().checked_sub(1) else {
            continue;
        };
        let jump = blocks[block_idx].instructions[jump_idx];
        if !jump.instr.is_unconditional_jump() || jump.target == BlockIdx::NULL {
            continue;
        }
        if !block_has_with_suppress_prefix(&blocks[block_idx], jump_idx) {
            continue;
        }

        let target = next_nonempty_block(blocks, jump.target);
        if target == BlockIdx::NULL || !is_const_return_block(&blocks[target.idx()]) {
            continue;
        }

        let mut cloned_return = blocks[target.idx()].instructions.clone();
        for instr in &mut cloned_return {
            overwrite_location(instr, jump.location, jump.end_location);
        }
        blocks[block_idx].instructions.pop();
        blocks[block_idx].instructions.extend(cloned_return);
    }
}

fn is_named_except_cleanup_return_block(block: &Block, metadata: &CodeUnitMetadata) -> bool {
    matches!(
        block.instructions.as_slice(),
        [pop_except, load_none1, store, delete, load_none2, ret]
            if matches!(pop_except.instr.real(), Some(Instruction::PopExcept))
                && is_load_const_none(load_none1, metadata)
                && matches!(
                    store.instr.real(),
                    Some(Instruction::StoreFast { .. } | Instruction::StoreName { .. })
                )
                && matches!(
                    delete.instr.real(),
                    Some(Instruction::DeleteFast { .. } | Instruction::DeleteName { .. })
                )
                && is_load_const_none(load_none2, metadata)
                && matches!(ret.instr.real(), Some(Instruction::ReturnValue))
    )
}

fn duplicate_named_except_cleanup_returns(blocks: &mut Vec<Block>, metadata: &CodeUnitMetadata) {
    let predecessors = compute_predecessors(blocks);
    let mut clones = Vec::new();

    for target in 0..blocks.len() {
        let target = BlockIdx(target as u32);
        if !is_named_except_cleanup_return_block(&blocks[target.idx()], metadata) {
            continue;
        }

        let layout_pred = find_layout_predecessor(blocks, target);
        if layout_pred == BlockIdx::NULL
            || next_nonempty_block(blocks, blocks[layout_pred.idx()].next) != target
        {
            continue;
        }

        let fallthroughs_into_target = blocks[layout_pred.idx()]
            .instructions
            .last()
            .is_none_or(|ins| !ins.instr.is_scope_exit() && !ins.instr.is_unconditional_jump());
        if !fallthroughs_into_target || predecessors[target.idx()] < 2 {
            continue;
        }

        let target_lineno = blocks[target.idx()]
            .instructions
            .first()
            .map_or(-1, instruction_lineno);
        let layout_pred_lineno = blocks[layout_pred.idx()]
            .instructions
            .iter()
            .rev()
            .find(|info| info.instr.real().is_some())
            .map_or(-1, instruction_lineno);
        if target_lineno > 0 && layout_pred_lineno > 0 && target_lineno != layout_pred_lineno {
            continue;
        }

        for block_idx in 0..blocks.len() {
            if block_idx == target.idx() {
                continue;
            }
            let Some(instr_idx) = trailing_conditional_jump_index(&blocks[block_idx]) else {
                continue;
            };
            if next_nonempty_block(blocks, blocks[block_idx].instructions[instr_idx].target)
                != target
            {
                continue;
            }
            clones.push((BlockIdx(block_idx as u32), instr_idx, target));
        }
    }

    for (block_idx, instr_idx, target) in clones.into_iter().rev() {
        let jump = blocks[block_idx.idx()].instructions[instr_idx];
        let mut cloned = blocks[target.idx()].instructions.clone();
        if let Some(first) = cloned.first_mut() {
            overwrite_location(first, jump.location, jump.end_location);
        }

        let new_idx = BlockIdx(blocks.len() as u32);
        let next = blocks[target.idx()].next;
        blocks.push(Block {
            cold: blocks[target.idx()].cold,
            except_handler: blocks[target.idx()].except_handler,
            disable_load_fast_borrow: blocks[target.idx()].disable_load_fast_borrow,
            instructions: cloned,
            next,
            ..Block::default()
        });
        blocks[target.idx()].next = new_idx;
        blocks[block_idx.idx()].instructions[instr_idx].target = new_idx;
    }
}

fn is_const_return_block(block: &Block) -> bool {
    block.instructions.len() == 2
        && matches!(
            block.instructions[0].instr.real(),
            Some(Instruction::LoadConst { .. })
        )
        && matches!(
            block.instructions[1].instr.real(),
            Some(Instruction::ReturnValue)
        )
}

fn inline_pop_except_return_blocks(blocks: &mut [Block]) {
    for block_idx in 0..blocks.len() {
        let Some(jump_idx) = blocks[block_idx].instructions.len().checked_sub(1) else {
            continue;
        };
        let jump = blocks[block_idx].instructions[jump_idx];
        if !jump.instr.is_unconditional_jump() || jump.target == BlockIdx::NULL {
            continue;
        }

        let Some(last_real_before_jump) = blocks[block_idx].instructions[..jump_idx]
            .iter()
            .rev()
            .find_map(|info| info.instr.real())
        else {
            continue;
        };
        if !matches!(last_real_before_jump, Instruction::PopExcept) {
            continue;
        }

        let target = next_nonempty_block(blocks, jump.target);
        if target == BlockIdx::NULL || !is_const_return_block(&blocks[target.idx()]) {
            continue;
        }

        let mut cloned_return = blocks[target.idx()].instructions.clone();
        for instr in &mut cloned_return {
            overwrite_location(instr, jump.location, jump.end_location);
        }
        blocks[block_idx].instructions.pop();
        blocks[block_idx].instructions.extend(cloned_return);
    }
}

fn is_named_except_cleanup_jump_block(block: &Block, metadata: &CodeUnitMetadata) -> bool {
    matches!(
        block.instructions.as_slice(),
        [pop_except, load_none, store, delete, jump]
            if matches!(pop_except.instr.real(), Some(Instruction::PopExcept))
                && is_load_const_none(load_none, metadata)
                && matches!(
                    store.instr.real(),
                    Some(Instruction::StoreFast { .. } | Instruction::StoreName { .. })
                )
                && matches!(
                    delete.instr.real(),
                    Some(Instruction::DeleteFast { .. } | Instruction::DeleteName { .. })
                )
                && jump.instr.is_unconditional_jump()
                && jump.target != BlockIdx::NULL
    )
}

fn inline_named_except_cleanup_jump_blocks(blocks: &mut [Block], metadata: &CodeUnitMetadata) {
    loop {
        let mut replacements = Vec::new();
        for block_idx in 0..blocks.len() {
            let Some(jump_idx) = blocks[block_idx].instructions.len().checked_sub(1) else {
                continue;
            };
            let jump = blocks[block_idx].instructions[jump_idx];
            if !matches!(jump.instr.real(), Some(Instruction::JumpForward { .. }))
                || jump.target == BlockIdx::NULL
            {
                continue;
            }
            let target = next_nonempty_block(blocks, jump.target);
            if target == BlockIdx::NULL
                || !is_named_except_cleanup_jump_block(&blocks[target.idx()], metadata)
            {
                continue;
            }
            replacements.push((BlockIdx(block_idx as u32), jump_idx, target));
        }
        if replacements.is_empty() {
            break;
        }

        for (block_idx, jump_idx, target) in replacements {
            if next_nonempty_block(
                blocks,
                blocks[block_idx.idx()].instructions[jump_idx].target,
            ) != target
            {
                continue;
            }
            let mut cloned = blocks[target.idx()].instructions.clone();
            blocks[block_idx.idx()].instructions.truncate(jump_idx);
            blocks[block_idx.idx()].instructions.append(&mut cloned);
        }
    }
}

fn deduplicate_adjacent_pop_except_jump_back_blocks(blocks: &mut [Block]) {
    fn pop_except_jump_back_target(block: &Block) -> Option<BlockIdx> {
        let [pop_except, jump] = block.instructions.as_slice() else {
            return None;
        };
        if matches!(pop_except.instr.real(), Some(Instruction::PopExcept))
            && matches!(
                jump.instr.real(),
                Some(Instruction::JumpBackwardNoInterrupt { .. })
            )
            && jump.target != BlockIdx::NULL
        {
            Some(jump.target)
        } else {
            None
        }
    }

    let mut current = BlockIdx(0);
    while current != BlockIdx::NULL {
        let next = blocks[current.idx()].next;
        let Some(target) = pop_except_jump_back_target(&blocks[current.idx()]) else {
            current = next;
            continue;
        };
        let layout_pred = find_layout_predecessor(blocks, current);
        if layout_pred != BlockIdx::NULL
            && block_has_fallthrough(&blocks[layout_pred.idx()])
            && next_nonempty_block(blocks, blocks[layout_pred.idx()].next) == current
            && blocks[layout_pred.idx()]
                .instructions
                .iter()
                .rev()
                .find_map(|info| info.instr.real())
                .is_some_and(|instr| {
                    matches!(
                        instr,
                        Instruction::StoreFast { .. }
                            | Instruction::StoreName { .. }
                            | Instruction::StoreGlobal { .. }
                            | Instruction::StoreDeref { .. }
                            | Instruction::PopExcept
                    )
                })
        {
            current = next;
            continue;
        }
        let duplicate = next_nonempty_block(blocks, next);
        if duplicate == BlockIdx::NULL
            || duplicate == current
            || pop_except_jump_back_target(&blocks[duplicate.idx()]) != Some(target)
        {
            current = next;
            continue;
        }

        for block in blocks.iter_mut() {
            for info in &mut block.instructions {
                if info.target == duplicate {
                    info.target = current;
                }
            }
        }
        blocks[current.idx()].next = blocks[duplicate.idx()].next;
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
                let remove_no_location_nop = blocks[bi].instructions[i].remove_no_location_nop;
                let folded_operand_nop = blocks[bi].instructions[i].folded_operand_nop;
                let preserve_block_start_no_location_nop =
                    blocks[bi].instructions[i].preserve_block_start_no_location_nop;
                set_to_nop(&mut blocks[bi].instructions[i]);
                blocks[bi].instructions[i].remove_no_location_nop = remove_no_location_nop;
                blocks[bi].instructions[i].folded_operand_nop = folded_operand_nop;
                blocks[bi].instructions[i].preserve_block_start_no_location_nop =
                    preserve_block_start_no_location_nop;
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
                .is_none_or(|ins| !ins.instr.is_scope_exit() && !ins.instr.is_unconditional_jump()); // Empty block falls through
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
                    let preserve_block_start_no_location_nop =
                        info.preserve_block_start_no_location_nop;
                    set_to_nop(info);
                    info.preserve_block_start_no_location_nop =
                        preserve_block_start_no_location_nop;
                }
                // PopBlock in reachable blocks is converted to NOP by
                // label_exception_targets. Dead blocks may still have them.
                PseudoInstruction::PopBlock => {
                    let remove_no_location_nop = info.remove_no_location_nop;
                    let folded_operand_nop = info.folded_operand_nop;
                    let preserve_block_start_no_location_nop =
                        info.preserve_block_start_no_location_nop;
                    set_to_nop(info);
                    info.remove_no_location_nop = remove_no_location_nop;
                    info.folded_operand_nop = folded_operand_nop;
                    info.preserve_block_start_no_location_nop =
                        preserve_block_start_no_location_nop;
                }
                // LOAD_CLOSURE → LOAD_FAST (using cellfixedoffsets for merged layout)
                PseudoInstruction::LoadClosure { i } => {
                    let cell_relative = i.get(info.arg) as usize;
                    let new_idx = cellfixedoffsets[cell_relative];
                    info.arg = OpArg::new(new_idx);
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
                PseudoInstruction::AnnotationsPlaceholder
                | PseudoInstruction::JumpIfFalse { .. }
                | PseudoInstruction::JumpIfTrue { .. } => {
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
                instr.into(),
                Opcode::LoadDeref
                    | Opcode::StoreDeref
                    | Opcode::DeleteDeref
                    | Opcode::LoadFromDictOrDeref
                    | Opcode::MakeCell
            );
            if needs_fixup {
                let cell_relative = u32::from(info.arg) as usize;
                info.arg = OpArg::new(cellfixedoffsets[cell_relative]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instruction_info(instr: Instruction, arg: u32, target: BlockIdx) -> InstructionInfo {
        InstructionInfo {
            instr: instr.into(),
            arg: OpArg::new(arg),
            target,
            location: SourceLocation::default(),
            end_location: SourceLocation::default(),
            except_handler: None,
            folded_from_nonliteral_expr: false,
            lineno_override: None,
            cache_entries: 0,
            preserve_redundant_jump_as_nop: false,
            remove_no_location_nop: false,
            folded_operand_nop: false,
            no_location_exit: false,
            preserve_block_start_no_location_nop: false,
            match_success_jump: false,
            break_continue_cleanup_jump: false,
        }
    }

    #[test]
    fn short_circuit_stub_allows_only_marker_instructions_before_jump() {
        let final_target = BlockIdx(7);
        let block = Block {
            instructions: vec![
                instruction_info(Instruction::Copy { i: Arg::marker() }, 1, BlockIdx::NULL),
                instruction_info(Instruction::ToBool, 0, BlockIdx::NULL),
                instruction_info(Instruction::Nop, 0, BlockIdx::NULL),
                instruction_info(Instruction::NotTaken, 0, BlockIdx::NULL),
                instruction_info(
                    Instruction::PopJumpIfFalse {
                        delta: Arg::marker(),
                    },
                    0,
                    final_target,
                ),
            ],
            ..Block::default()
        };

        assert_eq!(
            same_short_circuit_target(
                &block,
                Instruction::PopJumpIfFalse {
                    delta: Arg::marker(),
                }
                .into(),
            ),
            Some(final_target)
        );
    }

    #[test]
    fn short_circuit_stub_rejects_real_instruction_before_jump() {
        let block = Block {
            instructions: vec![
                instruction_info(Instruction::Copy { i: Arg::marker() }, 1, BlockIdx::NULL),
                instruction_info(Instruction::ToBool, 0, BlockIdx::NULL),
                instruction_info(Instruction::PopTop, 0, BlockIdx::NULL),
                instruction_info(
                    Instruction::PopJumpIfFalse {
                        delta: Arg::marker(),
                    },
                    0,
                    BlockIdx(7),
                ),
            ],
            ..Block::default()
        };

        assert_eq!(
            same_short_circuit_target(
                &block,
                Instruction::PopJumpIfFalse {
                    delta: Arg::marker(),
                }
                .into(),
            ),
            None
        );
        assert!(!opposite_short_circuit_target(
            &block,
            Instruction::PopJumpIfTrue {
                delta: Arg::marker(),
            }
            .into()
        ));
    }
}
