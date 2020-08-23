use std::error::Error;
use std::fmt;
use std::mem;

use cranelift::prelude::*;
use cranelift_module::{Backend, FuncId, Linkage, Module, ModuleError};
use cranelift_simplejit::{SimpleJITBackend, SimpleJITBuilder};

use rustpython_bytecode::bytecode;

mod instructions;

use self::instructions::FunctionCompiler;

#[derive(Debug)]
pub enum JITCompileError {
    NotSupported,
    BadBytecode,
    CraneliftError(ModuleError),
}

impl From<ModuleError> for JITCompileError {
    fn from(err: ModuleError) -> Self {
        JITCompileError::CraneliftError(err)
    }
}

impl fmt::Display for JITCompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            JITCompileError::NotSupported => f.write_str("Function can't be jitted."),
            JITCompileError::BadBytecode => f.write_str("Bad bytecode."),
            JITCompileError::CraneliftError(ref err) => err.fmt(f),
        }
    }
}

impl Error for JITCompileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match *self {
            JITCompileError::CraneliftError(ref err) => Some(err),
            _ => None,
        }
    }
}

struct JIT {
    builder_context: FunctionBuilderContext,
    ctx: codegen::Context,
    module: Module<SimpleJITBackend>,
}

impl JIT {
    fn new() -> Self {
        let builder = SimpleJITBuilder::new(cranelift_module::default_libcall_names());
        let module = Module::new(builder);
        Self {
            builder_context: FunctionBuilderContext::new(),
            ctx: module.make_context(),
            module,
        }
    }

    fn build_function(
        &mut self,
        bytecode: &bytecode::CodeObject,
    ) -> Result<FuncId, JITCompileError> {
        // currently always returns an int
        self.ctx
            .func
            .signature
            .returns
            .push(AbiParam::new(types::I64));

        let id = self.module.declare_function(
            &format!("jit_{}", bytecode.obj_name),
            Linkage::Export,
            &self.ctx.func.signature,
        )?;

        let mut builder = FunctionBuilder::new(&mut self.ctx.func, &mut self.builder_context);
        let entry_block = builder.create_block();
        // builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);

        {
            let mut compiler = FunctionCompiler::new(&mut builder);

            for instruction in &bytecode.instructions {
                compiler.add_instruction(instruction)?;
            }
        };

        builder.finalize();

        self.module
            .define_function(id, &mut self.ctx, &mut codegen::binemit::NullTrapSink {})?;

        self.module.clear_context(&mut self.ctx);

        Ok(id)
    }
}

pub fn compile(bytecode: &bytecode::CodeObject) -> Result<CompiledCode, JITCompileError> {
    let mut jit = JIT::new();

    let id = jit.build_function(bytecode)?;

    jit.module.finalize_definitions();

    let code = jit.module.get_finalized_function(id);
    Ok(CompiledCode {
        code,
        memory: jit.module.finish(),
    })
}

pub struct CompiledCode {
    code: *const u8,
    memory: <SimpleJITBackend as Backend>::Product,
}

impl CompiledCode {
    pub fn invoke(&self) -> i64 {
        let func = unsafe { mem::transmute::<_, fn() -> i64>(self.code) };
        func()
    }
}

unsafe impl Send for CompiledCode {}
unsafe impl Sync for CompiledCode {}

impl Drop for CompiledCode {
    fn drop(&mut self) {
        // SAFETY: The only pointer that this memory will also be dropped now
        unsafe { self.memory.free_memory() }
    }
}

impl fmt::Debug for CompiledCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[compiled code]")
    }
}
