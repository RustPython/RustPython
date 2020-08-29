use std::fmt;

use cranelift::prelude::*;
use cranelift_module::{Backend, FuncId, Linkage, Module, ModuleError};
use cranelift_simplejit::{SimpleJITBackend, SimpleJITBuilder};

use rustpython_bytecode::bytecode;

mod instructions;

use instructions::FunctionCompiler;

#[derive(Debug, thiserror::Error)]
pub enum JitCompileError {
    #[error("function can't be jitted")]
    NotSupported,
    #[error("bad bytecode")]
    BadBytecode,
    #[error("error while compiling to machine code: {0}")]
    CraneliftError(#[from] ModuleError),
}

#[derive(Debug, thiserror::Error)]
pub enum JitExecutionError {
    #[error("argument type differs from what is expected")]
    ArgTypeMismatch,
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
        args: &[JitType],
    ) -> Result<(FuncId, JitSig), JitCompileError> {
        for arg in args {
            self.ctx
                .func
                .signature
                .params
                .push(AbiParam::new(arg.to_cranelift()));
        }

        let mut builder = FunctionBuilder::new(&mut self.ctx.func, &mut self.builder_context);
        let entry_block = builder.create_block();
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);

        let sig = {
            let mut arg_names = bytecode.arg_names.clone();
            arg_names.extend(bytecode.kwonlyarg_names.iter().cloned());
            let mut compiler = FunctionCompiler::new(&mut builder, &arg_names, args, entry_block);

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

pub fn compile(
    bytecode: &bytecode::CodeObject,
    args: &[JitType],
) -> Result<CompiledCode, JitCompileError> {
    let mut jit = Jit::new();

    let (id, sig) = jit.build_function(bytecode, args)?;

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
    pub fn invoke(&self, args: &[AbiValue]) -> Result<Option<AbiValue>, JitExecutionError> {
        let mut cif_args = Vec::new();
        for (val, ty) in args.iter().zip(&self.sig.args) {
            match (val, ty) {
                (AbiValue::Int(ref val), JitType::Int) => {
                    cif_args.push(libffi::middle::Arg::new(val))
                }
                (AbiValue::Float(ref val), JitType::Float) => {
                    cif_args.push(libffi::middle::Arg::new(val))
                }
                _ => return Err(JitExecutionError::ArgTypeMismatch),
            }
        }
        let cif = self.sig.to_cif();
        unsafe {
            let value = cif.call::<UnTypedAbiValue>(
                libffi::middle::CodePtr::from_ptr(self.code as *const _),
                &cif_args,
            );
            Ok(self.sig.ret.as_ref().map(|ty| value.to_typed(ty)))
        }
    }
}

#[derive(Default)]
struct JitSig {
    args: Vec<JitType>,
    ret: Option<JitType>,
}

impl JitSig {
    fn to_cif(&self) -> libffi::middle::Cif {
        let ret = match self.ret {
            Some(ref ty) => ty.to_libffi(),
            None => libffi::middle::Type::void(),
        };
        libffi::middle::Cif::new(Vec::new(), ret)
    }
}

#[derive(Copy, Clone, PartialEq)]
pub enum JitType {
    Int,
    Float,
}

impl JitType {
    fn to_cranelift(&self) -> types::Type {
        match self {
            Self::Int => types::I64,
            Self::Float => types::F64,
        }
    }

    fn to_libffi(&self) -> libffi::middle::Type {
        match self {
            Self::Int => libffi::middle::Type::i64(),
            Self::Float => libffi::middle::Type::f64(),
        }
    }
}

#[derive(Clone)]
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
