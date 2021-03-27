use crate::IndexSet;
use rustpython_bytecode::{CodeFlags, CodeObject, ConstantData, Instruction, Label, Location};

pub type BlockIdx = Label;

#[derive(Debug)]
pub struct InstructionInfo {
    /// If the instruction has a Label argument, it's actually a BlockIdx, not a code offset
    pub instr: Instruction,
    pub location: Location,
}

// TODO: look into using petgraph for handling blocks and stuff? it's heavier than this, but it
// might enable more analysis/optimizations
#[derive(Debug)]
pub struct Block {
    pub instructions: Vec<InstructionInfo>,
    pub next: BlockIdx,
}
impl Default for Block {
    fn default() -> Self {
        Block {
            instructions: Vec::new(),
            next: Label(u32::MAX),
        }
    }
}

pub struct CodeInfo {
    pub flags: CodeFlags,
    pub posonlyarg_count: usize, // Number of positional-only arguments
    pub arg_count: usize,
    pub kwonlyarg_count: usize,
    pub source_path: String,
    pub first_line_number: usize,
    pub obj_name: String, // Name of the object that created this code object

    pub blocks: Vec<Block>,
    pub current_block: BlockIdx,
    pub constants: IndexSet<ConstantData>,
    pub name_cache: IndexSet<String>,
    pub varname_cache: IndexSet<String>,
    pub cellvar_cache: IndexSet<String>,
    pub freevar_cache: IndexSet<String>,
}
impl CodeInfo {
    pub fn finalize_code(mut self, optimize: u8) -> CodeObject {
        let max_stacksize = self.max_stacksize();
        let cell2arg = self.cell2arg();

        if optimize > 0 {
            self.dce();
        }

        let CodeInfo {
            flags,
            posonlyarg_count,
            arg_count,
            kwonlyarg_count,
            source_path,
            first_line_number,
            obj_name,

            blocks,
            current_block: _,
            constants,
            name_cache,
            varname_cache,
            cellvar_cache,
            freevar_cache,
        } = self;

        let mut num_instructions = 0;
        let mut block_to_offset = vec![Label(0); blocks.len()];

        for (idx, block) in iter_blocks(&blocks) {
            block_to_offset[idx.0 as usize] = Label(num_instructions as u32);
            num_instructions += block.instructions.len();
        }

        let mut instructions = Vec::with_capacity(num_instructions);
        let mut locations = Vec::with_capacity(num_instructions);

        for (_, block) in iter_blocks(&blocks) {
            for info in &block.instructions {
                let mut instr = info.instr.clone();
                if let Some(l) = instr.label_arg_mut() {
                    *l = block_to_offset[l.0 as usize];
                }
                instructions.push(instr);
                locations.push(info.location);
            }
        }

        CodeObject {
            flags,
            posonlyarg_count,
            arg_count,
            kwonlyarg_count,
            source_path,
            first_line_number,
            obj_name,

            max_stacksize,
            instructions: instructions.into_boxed_slice(),
            locations: locations.into_boxed_slice(),
            constants: constants.into_iter().collect(),
            names: name_cache.into_iter().collect(),
            varnames: varname_cache.into_iter().collect(),
            cellvars: cellvar_cache.into_iter().collect(),
            freevars: freevar_cache.into_iter().collect(),
            cell2arg,
        }
    }

    fn cell2arg(&self) -> Option<Box<[isize]>> {
        if self.cellvar_cache.is_empty() {
            return None;
        }

        let total_args = self.arg_count
            + self.kwonlyarg_count
            + self.flags.contains(CodeFlags::HAS_VARARGS) as usize
            + self.flags.contains(CodeFlags::HAS_VARKEYWORDS) as usize;

        let mut found_cellarg = false;
        let cell2arg = self
            .cellvar_cache
            .iter()
            .map(|var| {
                self.varname_cache
                    .get_index_of(var)
                    // check that it's actually an arg
                    .filter(|i| *i < total_args)
                    .map_or(-1, |i| {
                        found_cellarg = true;
                        i as isize
                    })
            })
            .collect::<Box<[_]>>();

        if found_cellarg {
            Some(cell2arg)
        } else {
            None
        }
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

    fn max_stacksize(&self) -> u32 {
        let mut maxdepth = 0u32;
        let mut stack = Vec::with_capacity(self.blocks.len());
        let mut startdepths = vec![u32::MAX; self.blocks.len()];
        startdepths[0] = 0;
        stack.push(Label(0));
        let debug = false;
        'process_blocks: while let Some(block) = stack.pop() {
            let mut depth = startdepths[block.0 as usize];
            if debug {
                eprintln!("===BLOCK {}===", block.0);
            }
            let block = &self.blocks[block.0 as usize];
            for i in &block.instructions {
                let instr = &i.instr;
                let effect = instr.stack_effect(false);
                let new_depth = add_ui(depth, effect);
                if debug {
                    eprintln!("{:?}: {:+}, {:+} = {}", instr, effect, depth, new_depth);
                }
                if new_depth > maxdepth {
                    maxdepth = new_depth
                }
                // we don't want to worry about Continue, it uses unwinding to jump to
                // its targets and as such the stack size is taken care of in frame.rs by setting
                // it back to the level it was at when SetupLoop was run
                let jump_label = instr
                    .label_arg()
                    .filter(|_| !matches!(instr, Instruction::Continue { .. }));
                if let Some(&target_block) = jump_label {
                    let effect = instr.stack_effect(true);
                    let target_depth = add_ui(depth, effect);
                    if target_depth > maxdepth {
                        maxdepth = target_depth
                    }
                    stackdepth_push(&mut stack, &mut startdepths, target_block, target_depth);
                }
                depth = new_depth;
                if instr.unconditional_branch() {
                    continue 'process_blocks;
                }
            }
            stackdepth_push(&mut stack, &mut startdepths, block.next, depth);
        }
        maxdepth
    }
}

fn stackdepth_push(stack: &mut Vec<Label>, startdepths: &mut [u32], target: Label, depth: u32) {
    let block_depth = &mut startdepths[target.0 as usize];
    if *block_depth == u32::MAX || depth > *block_depth {
        *block_depth = depth;
        stack.push(target);
    }
}

fn add_ui(a: u32, b: i32) -> u32 {
    if b < 0 {
        a - b.wrapping_abs() as u32
    } else {
        a + b as u32
    }
}

fn iter_blocks(blocks: &[Block]) -> impl Iterator<Item = (BlockIdx, &Block)> + '_ {
    let get_idx = move |i: BlockIdx| blocks.get(i.0 as usize).map(|b| (i, b));
    std::iter::successors(get_idx(Label(0)), move |(_, b)| get_idx(b.next)) // if b.next is u32::MAX that's the end
}
