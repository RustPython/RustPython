use std::ops;

use crate::{IndexMap, IndexSet, error::InternalError};
use rustpython_compiler_core::{
    OneIndexed, SourceLocation,
    bytecode::{
        CodeFlags, CodeObject, CodeUnit, ConstantData, InstrDisplayContext, Instruction, Label,
        OpArg, PyCodeLocationInfoKind,
    },
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
pub struct BlockIdx(pub u32);
impl BlockIdx {
    pub const NULL: Self = Self(u32::MAX);
    const fn idx(self) -> usize {
        self.0 as usize
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
    // pub range: TextRange,
    pub location: SourceLocation,
    // TODO: end_location for debug ranges
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
    pub fn finalize_code(mut self, optimize: u8) -> crate::InternalResult<CodeObject> {
        if optimize > 0 {
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

        let mut block_to_offset = vec![Label(0); blocks.len()];
        loop {
            let mut num_instructions = 0;
            for (idx, block) in iter_blocks(&blocks) {
                block_to_offset[idx.idx()] = Label(num_instructions as u32);
                for instr in &block.instructions {
                    num_instructions += instr.arg.instr_size()
                }
            }

            instructions.reserve_exact(num_instructions);
            locations.reserve_exact(num_instructions);

            let mut recompile_extended_arg = false;
            let mut next_block = BlockIdx(0);
            while next_block != BlockIdx::NULL {
                let block = &mut blocks[next_block];
                for info in &mut block.instructions {
                    let (op, arg, target) = (info.instr, &mut info.arg, info.target);
                    if target != BlockIdx::NULL {
                        let new_arg = OpArg(block_to_offset[target.idx()].0);
                        recompile_extended_arg |= new_arg.instr_size() != arg.instr_size();
                        *arg = new_arg;
                    }
                    let (extras, lo_arg) = arg.split();
                    locations.extend(std::iter::repeat_n(info.location, arg.instr_size()));
                    instructions.extend(
                        extras
                            .map(|byte| CodeUnit::new(Instruction::ExtendedArg, byte))
                            .chain([CodeUnit { op, arg: lo_arg }]),
                    );
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
        let linetable = generate_linetable(&locations, first_line_number.get() as i32);

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
            instructions: instructions.into_boxed_slice(),
            locations: locations.into_boxed_slice(),
            constants: constants.into_iter().collect(),
            names: name_cache.into_iter().collect(),
            varnames: varname_cache.into_iter().collect(),
            cellvars: cellvar_cache.into_iter().collect(),
            freevars: freevar_cache.into_iter().collect(),
            cell2arg,
            linetable,
            exceptiontable: Box::new([]), // TODO: Generate actual exception table
        })
    }

    fn cell2arg(&self) -> Option<Box<[i32]>> {
        if self.metadata.cellvars.is_empty() {
            return None;
        }

        let total_args = self.metadata.argcount
            + self.metadata.kwonlyargcount
            + self.flags.contains(CodeFlags::HAS_VARARGS) as u32
            + self.flags.contains(CodeFlags::HAS_VARKEYWORDS) as u32;

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
        'process_blocks: while let Some(block) = stack.pop() {
            let mut depth = start_depths[block.idx()];
            if DEBUG {
                eprintln!("===BLOCK {}===", block.0);
            }
            let block = &self.blocks[block];
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
                // we don't want to worry about Break/Continue, they use unwinding to jump to
                // their targets and as such the stack size is taken care of in frame.rs by setting
                // it back to the level it was at when SetupLoop was run
                if ins.target != BlockIdx::NULL
                    && !matches!(
                        instr,
                        Instruction::Continue { .. } | Instruction::Break { .. }
                    )
                {
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
    let block_depth = &mut start_depths[target.idx()];
    if *block_depth == u32::MAX || depth > *block_depth {
        *block_depth = depth;
        stack.push(target);
    }
}

fn iter_blocks(blocks: &[Block]) -> impl Iterator<Item = (BlockIdx, &Block)> + '_ {
    let mut next = BlockIdx(0);
    std::iter::from_fn(move || {
        if next == BlockIdx::NULL {
            return None;
        }
        let (idx, b) = (next, &blocks[next]);
        next = b.next;
        Some((idx, b))
    })
}

/// Generate CPython 3.11+ format linetable from source locations
fn generate_linetable(locations: &[SourceLocation], first_line: i32) -> Box<[u8]> {
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

            // Get line and column information
            // SourceLocation always has row and column (both are OneIndexed)
            let line = loc.line.get() as i32;
            let col = loc.character_offset.to_zero_indexed() as i32;

            let line_delta = line - prev_line;

            // Choose the appropriate encoding based on line delta and column info
            // Note: SourceLocation always has valid column, so we never get NO_COLUMNS case
            if line_delta == 0 {
                let end_col = col; // Use same column for end (no range info available)

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
                    write_varint(&mut linetable, (col as u32) + 1); // column + 1 for encoding
                    write_varint(&mut linetable, (end_col as u32) + 1); // end_col + 1
                }
            } else if line_delta > 0 && line_delta < 3
            /* && column.is_some() */
            {
                // One-line form (codes 11-12) for line deltas 1-2
                let end_col = col; // Use same column for end

                if col < 128 && end_col < 128 {
                    let code = (PyCodeLocationInfoKind::OneLine0 as u8) + (line_delta as u8); // 11 for delta=1, 12 for delta=2
                    linetable.push(0x80 | (code << 3) | ((entry_length - 1) as u8));
                    linetable.push(col as u8);
                    linetable.push(end_col as u8);
                } else {
                    // Long form for columns >= 128 or negative line delta
                    linetable.push(
                        0x80 | ((PyCodeLocationInfoKind::Long as u8) << 3)
                            | ((entry_length - 1) as u8),
                    );
                    write_signed_varint(&mut linetable, line_delta);
                    write_varint(&mut linetable, 0); // end_line delta = 0
                    write_varint(&mut linetable, (col as u32) + 1); // column + 1 for encoding
                    write_varint(&mut linetable, (end_col as u32) + 1); // end_col + 1
                }
            } else {
                // Long form (code 14) for all other cases
                // This handles: line_delta < 0, line_delta >= 3, or columns >= 128
                let end_col = col; // Use same column for end
                linetable.push(
                    0x80 | ((PyCodeLocationInfoKind::Long as u8) << 3) | ((entry_length - 1) as u8),
                );
                write_signed_varint(&mut linetable, line_delta);
                write_varint(&mut linetable, 0); // end_line delta = 0
                write_varint(&mut linetable, (col as u32) + 1); // column + 1 for encoding
                write_varint(&mut linetable, (end_col as u32) + 1); // end_col + 1
            }

            prev_line = line;
            length -= entry_length;
            i += entry_length;
        }
    }

    linetable.into_boxed_slice()
}

/// Write a variable-length unsigned integer (6-bit chunks)
/// Returns the number of bytes written
fn write_varint(buf: &mut Vec<u8>, mut val: u32) -> usize {
    let start_len = buf.len();
    while val >= 64 {
        buf.push(0x40 | (val & 0x3f) as u8);
        val >>= 6;
    }
    buf.push(val as u8);
    buf.len() - start_len
}

/// Write a variable-length signed integer
/// Returns the number of bytes written
fn write_signed_varint(buf: &mut Vec<u8>, val: i32) -> usize {
    let uval = if val < 0 {
        // (unsigned int)(-val) has an undefined behavior for INT_MIN
        // So we use (0 - val as u32) to handle it correctly
        ((0u32.wrapping_sub(val as u32)) << 1) | 1
    } else {
        (val as u32) << 1
    };
    write_varint(buf, uval)
}
