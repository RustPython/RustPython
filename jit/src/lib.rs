use std::fmt;

use cranelift::prelude::*;
use cranelift_module::{Backend, FuncId, Linkage, Module, ModuleError};
use cranelift_simplejit::{SimpleJITBackend, SimpleJITBuilder};

use rustpython_bytecode::bytecode;

mod instructions;

use instructions::{FunctionCompiler, JitSig, JitType};

#[derive(Debug, thiserror::Error)]
pub enum JitCompileError {
    #[error("function can't be jitted")]
    NotSupported,
    #[error("bad bytecode")]
    BadBytecode,
    #[error("error while compiling to machine code: {0}")]
    CraneliftError(#[from] ModuleError),
}

struct Jit {
    builder_context: FunctionBuilderContext,
    ctx: codegen::Context,
    module: Module<SimpleJITBackend>,
}

impl Jit {
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
    ) -> Result<(FuncId, JitSig), JitCompileError> {
        let mut builder = FunctionBuilder::new(&mut self.ctx.func, &mut self.builder_context);
        let entry_block = builder.create_block();
        // builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);

        let sig = {
            let mut compiler = FunctionCompiler::new(&mut builder);

            for instruction in &bytecode.instructions {
                compiler.add_instruction(instruction)?;
            }

            compiler.sig
        };

        builder.finalize();

        let id = self.module.declare_function(
            &format!("jit_{}", bytecode.obj_name),
            Linkage::Export,
            &self.ctx.func.signature,
        )?;

        self.module
            .define_function(id, &mut self.ctx, &mut codegen::binemit::NullTrapSink {})?;

        self.module.clear_context(&mut self.ctx);

        Ok((id, sig))
    }
}

pub fn compile(bytecode: &bytecode::CodeObject) -> Result<CompiledCode, JitCompileError> {
    let mut jit = Jit::new();

    let (id, sig) = jit.build_function(bytecode)?;

    jit.module.finalize_definitions();

    let code = jit.module.get_finalized_function(id);
    Ok(CompiledCode {
        sig,
        code,
        memory: jit.module.finish(),
    })
}

pub struct CompiledCode {
    sig: JitSig,
    code: *const u8,
    memory: <SimpleJITBackend as Backend>::Product,
}

impl CompiledCode {
    pub fn invoke(&self) -> Option<AbiValue> {
        let cif = self.sig.to_cif();
        unsafe {
            let value = cif.call::<UnTypedAbiValue>(
                libffi::middle::CodePtr::from_ptr(self.code as *const _),
                &[],
            );
            self.sig.ret.as_ref().map(|ty| value.to_typed(ty))
        }
    }
}

pub enum AbiValue {
    Float(f64),
    Int(i64),
}

union UnTypedAbiValue {
    float: f64,
    int: i64,
    _void: (),
}

impl UnTypedAbiValue {
    unsafe fn to_typed(&self, ty: &JitType) -> AbiValue {
        match ty {
            JitType::Int => AbiValue::Int(self.int),
            JitType::Float => AbiValue::Float(self.float),
        }
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
