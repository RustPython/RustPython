use core::ops;

use crate::{IndexMap, IndexSet, error::InternalError};
use rustpython_compiler_core::{
    OneIndexed, SourceLocation,
    bytecode::{
        Arg, CodeFlags, CodeObject, CodeUnit, CodeUnits, ConstantData, ExceptionTableEntry,
        InstrDisplayContext, Instruction, Label, OpArg, PyCodeLocationInfoKind,
        encode_exception_table, encode_load_attr_arg,
    },
    varint::{write_signed_varint, write_varint},
};

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

#[derive(Debug, Clone)]
pub struct InstructionInfo {
    pub instr: Instruction,
    pub arg: OpArg,
    pub target: BlockIdx,
    pub location: SourceLocation,
    pub end_location: SourceLocation,
    pub except_handler: Option<ExceptHandlerInfo>,
}

/// Exception handler information for an instruction
#[derive(Debug, Clone)]
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
}

impl Default for Block {
    fn default() -> Self {
        Self {
            instructions: Vec::new(),
            next: BlockIdx::NULL,
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
}

impl CodeInfo {
    pub fn finalize_code(
        mut self,
        opts: &crate::compile::CompileOpts,
    ) -> crate::InternalResult<CodeObject> {
        if opts.optimize > 0 {
            self.dce();
        }

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

        // convert_pseudo_ops: instructions before the main loop
        for block in blocks
            .iter_mut()
            .filter(|b| b.next != BlockIdx::NULL || !b.instructions.is_empty())
        {
            for info in &mut block.instructions {
                match info.instr {
                    // LOAD_ATTR_METHOD pseudo → LOAD_ATTR (with method flag=1)
                    Instruction::LoadAttrMethod { idx } => {
                        let encoded = encode_load_attr_arg(idx.get(info.arg), true);
                        info.arg = OpArg(encoded);
                        info.instr = Instruction::LoadAttr { idx: Arg::marker() };
                    }
                    // LOAD_ATTR → encode with method flag=0
                    Instruction::LoadAttr { idx } => {
                        let encoded = encode_load_attr_arg(idx.get(info.arg), false);
                        info.arg = OpArg(encoded);
                        info.instr = Instruction::LoadAttr { idx: Arg::marker() };
                    }
                    // POP_BLOCK pseudo → NOP
                    Instruction::PopBlock => {
                        info.instr = Instruction::Nop;
                    }
                    _ => {}
                }
            }
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
                        let new_arg = OpArg(block_to_offset[target.idx()].0);
                        recompile_extended_arg |= new_arg.instr_size() != info.arg.instr_size();
                        info.arg = new_arg;
                    }

                    // Convert JUMP pseudo to real instructions (direction depends on offset)
                    let op = match info.instr {
                        Instruction::Jump { .. } if target != BlockIdx::NULL => {
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
                        other => other,
                    };

                    let (extras, lo_arg) = info.arg.split();
                    locations.extend(core::iter::repeat_n(
                        (info.location, info.end_location),
                        info.arg.instr_size(),
                    ));
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
            locations.clear()
        }

        // Generate linetable from locations
        let linetable = generate_linetable(
            &locations,
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
                if ins.instr.unconditional_branch() {
                    last_instr = Some(i);
                    break;
                }
            }
            if let Some(i) = last_instr {
                block.instructions.truncate(i + 1);
            }
        }
    }

    fn max_stackdepth(&self) -> crate::InternalResult<u32> {
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
                let effect = instr.stack_effect(ins.arg, false);
                if DEBUG {
                    let display_arg = if ins.target == BlockIdx::NULL {
                        ins.arg
                    } else {
                        OpArg(ins.target.0)
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
                    let effect = instr.stack_effect(ins.arg, true);
                    let target_depth = depth.checked_add_signed(effect).ok_or({
                        if effect < 0 {
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
                // Process exception handler blocks
                // When exception occurs, stack is unwound to handler.stack_depth, then:
                // - If preserve_lasti: push lasti (+1)
                // - Push exception (+1)
                // - Handler block starts with PUSH_EXC_INFO as its first instruction
                // So the starting depth for the handler block (BEFORE PUSH_EXC_INFO) is:
                // handler.stack_depth + preserve_lasti + 1 (exc)
                // PUSH_EXC_INFO will then add +1 when the block is processed
                if let Some(ref handler) = ins.except_handler {
                    let handler_depth = handler.stack_depth + 1 + (handler.preserve_lasti as u32); // +1 for exception, +1 for lasti if preserve_lasti
                    if DEBUG {
                        eprintln!(
                            "  HANDLER: block={} depth={} (base={} lasti={})",
                            handler.handler_block.0,
                            handler_depth,
                            handler.stack_depth,
                            handler.preserve_lasti
                        );
                    }
                    if handler_depth > maxdepth {
                        maxdepth = handler_depth;
                    }
                    stackdepth_push(
                        &mut stack,
                        &mut start_depths,
                        handler.handler_block,
                        handler_depth,
                    );
                }
                depth = new_depth;
                if instr.unconditional_branch() {
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
    locations: &[(SourceLocation, SourceLocation)],
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
        let (loc, end_loc) = &locations[i];

        // Count consecutive instructions with the same location
        let mut length = 1;
        while i + length < locations.len() && locations[i + length] == locations[i] {
            length += 1;
        }

        // Process in chunks of up to 8 instructions
        while length > 0 {
            let entry_length = length.min(8);

            // Get line information
            let line = loc.line.get() as i32;
            let end_line = end_loc.line.get() as i32;
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
            let col = loc.character_offset.to_zero_indexed() as i32;
            let end_col = end_loc.character_offset.to_zero_indexed() as i32;

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

            match (&current_entry, &instr.except_handler) {
                // No current entry, no handler - nothing to do
                (None, None) => {}

                // No current entry, handler starts - begin new entry
                (None, Some(handler)) => {
                    current_entry = Some((handler.clone(), instr_index));
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
                    current_entry = Some((handler.clone(), instr_index));
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
