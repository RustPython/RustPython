use core::ops;

use crate::{IndexMap, IndexSet, error::InternalError};
use malachite_bigint::BigInt;
use num_traits::ToPrimitive;

use rustpython_compiler_core::{
    OneIndexed, SourceLocation,
    bytecode::{
        AnyInstruction, Arg, CodeFlags, CodeObject, CodeUnit, CodeUnits, ConstantData,
        ExceptionTableEntry, InstrDisplayContext, Instruction, InstructionMetadata, Label, OpArg,
        PseudoInstruction, PyCodeLocationInfoKind, encode_exception_table,
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

// spell-checker:ignore petgraph
// TODO: look into using petgraph for handling blocks and stuff? it's heavier than this, but it
// might enable more analysis/optimizations
#[derive(Debug)]
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
}

impl Default for Block {
    fn default() -> Self {
        Self {
            instructions: Vec::new(),
            next: BlockIdx::NULL,
            except_handler: false,
            preserve_lasti: false,
            start_depth: None,
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
        // Always fold tuple constants
        self.fold_tuple_constants();
        // Python only applies LOAD_SMALL_INT conversion to module-level code
        // (not inside functions). Module code lacks OPTIMIZED flag.
        // Note: RustPython incorrectly sets NEWLOCALS on modules, so only check OPTIMIZED
        let is_module_level = !self.flags.contains(CodeFlags::OPTIMIZED);
        if is_module_level {
            self.convert_to_load_small_int();
        }
        self.remove_unused_consts();
        self.remove_nops();

        if opts.optimize > 0 {
            self.dce();
            self.peephole_optimize();
        }

        // Always apply LOAD_FAST_BORROW optimization
        self.optimize_load_fast_borrow();

        // Post-codegen CFG analysis passes (flowgraph.c pipeline)
        mark_except_handlers(&mut self.blocks);
        label_exception_targets(&mut self.blocks);

        let max_stackdepth = self.max_stackdepth()?;
        let cell2arg = self.cell2arg();

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
            fast_hidden: _,
            argcount: arg_count,
            posonlyargcount: posonlyarg_count,
            kwonlyargcount: kwonlyarg_count,
            firstlineno: first_line_number,
        } = metadata;

        let mut instructions = Vec::new();
        let mut locations = Vec::new();
        let mut linetable_locations: Vec<LineTableLocation> = Vec::new();

        // Convert pseudo ops and remove resulting NOPs
        convert_pseudo_ops(&mut blocks, varname_cache.len() as u32);
        for block in blocks
            .iter_mut()
            .filter(|b| b.next != BlockIdx::NULL || !b.instructions.is_empty())
        {
            block
                .instructions
                .retain(|ins| !matches!(ins.instr.real(), Some(Instruction::Nop)));
        }

        let mut block_to_offset = vec![Label(0); blocks.len()];
        // block_to_index: maps block idx to instruction index (for exception table)
        // This is the index into the final instructions array, including EXTENDED_ARG
        let mut block_to_index = vec![0u32; blocks.len()];
        loop {
            let mut num_instructions = 0;
            for (idx, block) in iter_blocks(&blocks) {
                block_to_offset[idx.idx()] = Label(num_instructions as u32);
                // block_to_index uses the same value as block_to_offset but as u32
                // because lasti in frame.rs is the index into instructions array
                // and instructions array index == byte offset (each instruction is 1 CodeUnit)
                block_to_index[idx.idx()] = num_instructions as u32;
                for instr in &block.instructions {
                    num_instructions += instr.arg.instr_size();
                }
            }

            instructions.reserve_exact(num_instructions);
            locations.reserve_exact(num_instructions);

            let mut recompile_extended_arg = false;
            let mut next_block = BlockIdx(0);
            while next_block != BlockIdx::NULL {
                let block = &mut blocks[next_block];
                // Track current instruction offset for jump direction resolution
                let mut current_offset = block_to_offset[next_block.idx()].0;
                for info in &mut block.instructions {
                    let target = info.target;
                    if target != BlockIdx::NULL {
                        let new_arg = OpArg::new(block_to_offset[target.idx()].0);
                        recompile_extended_arg |= new_arg.instr_size() != info.arg.instr_size();
                        info.arg = new_arg;
                    }

                    // Convert JUMP pseudo to real instructions (direction depends on offset)
                    let op = match info.instr {
                        AnyInstruction::Pseudo(PseudoInstruction::Jump { .. })
                            if target != BlockIdx::NULL =>
                        {
                            let target_offset = block_to_offset[target.idx()].0;
                            if target_offset > current_offset {
                                Instruction::JumpForward {
                                    target: Arg::marker(),
                                }
                            } else {
                                Instruction::JumpBackward {
                                    target: Arg::marker(),
                                }
                            }
                        }
                        AnyInstruction::Pseudo(PseudoInstruction::JumpNoInterrupt { .. })
                            if target != BlockIdx::NULL =>
                        {
                            // JumpNoInterrupt is always backward (used in yield-from/await loops)
                            Instruction::JumpBackwardNoInterrupt {
                                target: Arg::marker(),
                            }
                        }
                        other => other.expect_real(),
                    };

                    let (extras, lo_arg) = info.arg.split();
                    locations.extend(core::iter::repeat_n(
                        (info.location, info.end_location),
                        info.arg.instr_size(),
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
                    instructions.extend(
                        extras
                            .map(|byte| CodeUnit::new(Instruction::ExtendedArg, byte))
                            .chain([CodeUnit { op, arg: lo_arg }]),
                    );
                    current_offset += info.arg.instr_size() as u32;
                }
                next_block = block.next;
            }

            if !recompile_extended_arg {
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
            cell2arg,
            linetable,
            exceptiontable,
        })
    }

    fn cell2arg(&self) -> Option<Box<[i32]>> {
        if self.metadata.cellvars.is_empty() {
            return None;
        }

        let total_args = self.metadata.argcount
            + self.metadata.kwonlyargcount
            + self.flags.contains(CodeFlags::VARARGS) as u32
            + self.flags.contains(CodeFlags::VARKEYWORDS) as u32;

        let mut found_cellarg = false;
        let cell2arg = self
            .metadata
            .cellvars
            .iter()
            .map(|var| {
                self.metadata
                    .varnames
                    .get_index_of(var)
                    // check that it's actually an arg
                    .filter(|i| *i < total_args as usize)
                    .map_or(-1, |i| {
                        found_cellarg = true;
                        i as i32
                    })
            })
            .collect::<Box<[_]>>();

        if found_cellarg { Some(cell2arg) } else { None }
    }

    fn dce(&mut self) {
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
                if tuple_size == 0 || i < tuple_size {
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

                // Replace preceding LOAD instructions with NOP
                for j in start_idx..i {
                    block.instructions[j].instr = Instruction::Nop.into();
                }

                // Replace BUILD_TUPLE with LOAD_CONST
                block.instructions[i].instr = Instruction::LoadConst { idx: Arg::marker() }.into();
                block.instructions[i].arg = OpArg::new(const_idx as u32);

                i += 1;
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
                        (Instruction::LoadFast(_), Instruction::LoadFast(_)) => {
                            let idx1 = u32::from(curr.arg);
                            let idx2 = u32::from(next.arg);
                            if idx1 < 16 && idx2 < 16 {
                                let packed = (idx1 << 4) | idx2;
                                Some((
                                    Instruction::LoadFastLoadFast { arg: Arg::marker() },
                                    OpArg::new(packed),
                                ))
                            } else {
                                None
                            }
                        }
                        // StoreFast + StoreFast -> StoreFastStoreFast (if both indices < 16)
                        (Instruction::StoreFast(_), Instruction::StoreFast(_)) => {
                            let idx1 = u32::from(curr.arg);
                            let idx2 = u32::from(next.arg);
                            if idx1 < 16 && idx2 < 16 {
                                let packed = (idx1 << 4) | idx2;
                                Some((
                                    Instruction::StoreFastStoreFast { arg: Arg::marker() },
                                    OpArg::new(packed),
                                ))
                            } else {
                                None
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

                // Check if it's in small int range: -5 to 256 (_PY_IS_SMALL_INT)
                if let Some(small) = value.to_i32().filter(|v| (-5..=256).contains(v)) {
                    // Convert LOAD_CONST to LOAD_SMALL_INT
                    instr.instr = Instruction::LoadSmallInt { idx: Arg::marker() }.into();
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

    /// Remove NOP instructions from all blocks
    fn remove_nops(&mut self) {
        for block in &mut self.blocks {
            block
                .instructions
                .retain(|ins| !matches!(ins.instr.real(), Some(Instruction::Nop)));
        }
    }

    /// Optimize LOAD_FAST to LOAD_FAST_BORROW where safe.
    ///
    /// A LOAD_FAST can be converted to LOAD_FAST_BORROW if its value is
    /// consumed within the same basic block (not passed to another block).
    /// This is a reference counting optimization in CPython; in RustPython
    /// we implement it for bytecode compatibility.
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
                    Instruction::LoadFast(_) | Instruction::LoadFastLoadFast { .. } => i,
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
                    Instruction::LoadFast(_) => {
                        info.instr = Instruction::LoadFastBorrow(Arg::marker()).into();
                    }
                    Instruction::LoadFastLoadFast { .. } => {
                        info.instr =
                            Instruction::LoadFastBorrowLoadFastBorrow { arg: Arg::marker() }.into();
                    }
                    _ => {}
                }
            }
        }
    }

    fn max_stackdepth(&mut self) -> crate::InternalResult<u32> {
        let mut maxdepth = 0u32;
        let mut stack = Vec::with_capacity(self.blocks.len());
        let mut start_depths = vec![u32::MAX; self.blocks.len()];
        start_depths[0] = 0;
        stack.push(BlockIdx(0));
        const DEBUG: bool = false;
        // Global iteration limit as safety guard
        // The algorithm is monotonic (depths only increase), so it should converge quickly.
        // Max iterations = blocks * max_possible_depth_increases per block
        let max_iterations = self.blocks.len() * 100;
        let mut iterations = 0usize;
        'process_blocks: while let Some(block_idx) = stack.pop() {
            iterations += 1;
            if iterations > max_iterations {
                // Safety guard: should never happen in valid code
                // Return error instead of silently breaking to avoid underestimated stack depth
                return Err(InternalError::StackOverflow);
            }
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
    fn get_constant(&self, i: usize) -> &ConstantData {
        &self.metadata.consts[i]
    }
    fn get_name(&self, i: usize) -> &str {
        self.metadata.names[i].as_ref()
    }
    fn get_varname(&self, i: usize) -> &str {
        self.metadata.varnames[i].as_ref()
    }
    fn get_cell_name(&self, i: usize) -> &str {
        self.metadata
            .cellvars
            .get_index(i)
            .unwrap_or_else(|| &self.metadata.freevars[i - self.metadata.cellvars.len()])
            .as_ref()
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
        // Found a path with higher depth (or first visit): update max and queue
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
            // instr_size includes EXTENDED_ARG instructions
            let instr_size = instr.arg.instr_size() as u32;

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
                blocks[bi].instructions[i].instr = Instruction::Nop.into();
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
                if matches!(
                    blocks[bi].instructions[i].instr.real(),
                    Some(Instruction::YieldValue { .. })
                ) {
                    last_yield_except_depth = stack.len() as i32;
                }

                // Set RESUME DEPTH1 flag based on last yield's except depth
                if matches!(
                    blocks[bi].instructions[i].instr.real(),
                    Some(Instruction::Resume { .. })
                ) {
                    const RESUME_AT_FUNC_START: u32 = 0;
                    const RESUME_OPARG_LOCATION_MASK: u32 = 0x3;
                    const RESUME_OPARG_DEPTH1_MASK: u32 = 0x4;

                    if (u32::from(arg) & RESUME_OPARG_LOCATION_MASK) != RESUME_AT_FUNC_START {
                        if last_yield_except_depth == 1 {
                            blocks[bi].instructions[i].arg =
                                OpArg::new(u32::from(arg) | RESUME_OPARG_DEPTH1_MASK);
                        }
                        last_yield_except_depth = -1;
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
pub(crate) fn convert_pseudo_ops(blocks: &mut [Block], varnames_len: u32) {
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
                    info.instr = Instruction::Nop.into();
                }
                // PopBlock in reachable blocks is converted to NOP by
                // label_exception_targets. Dead blocks may still have them.
                PseudoInstruction::PopBlock => {
                    info.instr = Instruction::Nop.into();
                }
                // LOAD_CLOSURE → LOAD_FAST (with varnames offset)
                PseudoInstruction::LoadClosure(idx) => {
                    let new_idx = varnames_len + idx.get(info.arg);
                    info.arg = OpArg::new(new_idx);
                    info.instr = Instruction::LoadFast(Arg::marker()).into();
                }
                // Jump pseudo ops are resolved during block linearization
                PseudoInstruction::Jump { .. } | PseudoInstruction::JumpNoInterrupt { .. } => {}
                // These should have been resolved earlier
                PseudoInstruction::AnnotationsPlaceholder
                | PseudoInstruction::JumpIfFalse { .. }
                | PseudoInstruction::JumpIfTrue { .. }
                | PseudoInstruction::StoreFastMaybeNull(_) => {
                    unreachable!("Unexpected pseudo instruction in convert_pseudo_ops: {pseudo:?}")
                }
            }
        }
    }
}
