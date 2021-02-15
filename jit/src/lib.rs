#![allow(clippy::unnecessary_wraps)]
use std::fmt;

use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module, ModuleError};

use rustpython_bytecode as bytecode;

mod instructions;

use instructions::FunctionCompiler;
use std::convert::TryFrom;

#[derive(Debug, thiserror::Error)]
pub enum JitCompileError {
    #[error("function can't be jitted")]
    NotSupported,
    #[error("bad bytecode")]
    BadBytecode,
    #[error("error while compiling to machine code: {0}")]
    CraneliftError(#[from] ModuleError),
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum JitArgumentError {
    #[error("argument is of wrong type")]
    ArgumentTypeMismatch,
    #[error("wrong number of arguments")]
    WrongNumberOfArguments,
}

struct Jit {
    builder_context: FunctionBuilderContext,
    ctx: codegen::Context,
    module: JITModule,
}

impl Jit {
    fn new() -> Self {
        let builder = JITBuilder::new(cranelift_module::default_libcall_names());
        let module = JITModule::new(builder);
        Self {
            builder_context: FunctionBuilderContext::new(),
            ctx: module.make_context(),
            module,
        }
    }

    fn build_function<C: bytecode::Constant>(
        &mut self,
        bytecode: &bytecode::CodeObject<C>,
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

        let sig = {
            let mut compiler =
                FunctionCompiler::new(&mut builder, bytecode.varnames.len(), args, entry_block);

            compiler.compile(bytecode)?;

            compiler.sig
        };

        builder.seal_all_blocks();
        builder.finalize();

        let id = self.module.declare_function(
            &format!("jit_{}", bytecode.obj_name.as_ref()),
            Linkage::Export,
            &self.ctx.func.signature,
        )?;

        self.module
            .define_function(id, &mut self.ctx, &mut codegen::binemit::NullTrapSink {})?;

        self.module.clear_context(&mut self.ctx);

        Ok((id, sig))
    }
}

pub fn compile<C: bytecode::Constant>(
    bytecode: &bytecode::CodeObject<C>,
    args: &[JitType],
) -> Result<CompiledCode, JitCompileError> {
    let mut jit = Jit::new();

    let (id, sig) = jit.build_function(bytecode, args)?;

    jit.module.finalize_definitions();

    let code = jit.module.get_finalized_function(id);
    Ok(CompiledCode {
        sig,
        code,
        module: jit.module,
    })
}

pub struct CompiledCode {
    sig: JitSig,
    code: *const u8,
    module: JITModule,
}

impl CompiledCode {
    pub fn args_builder(&self) -> ArgsBuilder<'_> {
        ArgsBuilder::new(self)
    }

    pub fn invoke(&self, args: &[AbiValue]) -> Result<Option<AbiValue>, JitArgumentError> {
        if self.sig.args.len() != args.len() {
            return Err(JitArgumentError::WrongNumberOfArguments);
        }

        let cif_args = self
            .sig
            .args
            .iter()
            .zip(args.iter())
            .map(|(ty, val)| type_check(ty, val).map(|_| val))
            .map(|v| v.map(AbiValue::to_libffi_arg))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(unsafe { self.invoke_raw(&cif_args) })
    }

    unsafe fn invoke_raw(&self, cif_args: &[libffi::middle::Arg]) -> Option<AbiValue> {
        let cif = self.sig.to_cif();
        let value = cif.call::<UnTypedAbiValue>(
            libffi::middle::CodePtr::from_ptr(self.code as *const _),
            cif_args,
        );
        self.sig.ret.as_ref().map(|ty| value.to_typed(ty))
    }
}

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
        libffi::middle::Cif::new(self.args.iter().map(JitType::to_libffi), ret)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum JitType {
    Int,
    Float,
    Bool,
}

impl JitType {
    fn to_cranelift(&self) -> types::Type {
        match self {
            Self::Int => types::I64,
            Self::Float => types::F64,
            Self::Bool => types::I8,
        }
    }

    fn to_libffi(&self) -> libffi::middle::Type {
        match self {
            Self::Int => libffi::middle::Type::i64(),
            Self::Float => libffi::middle::Type::f64(),
            Self::Bool => libffi::middle::Type::u8(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AbiValue {
    Float(f64),
    Int(i64),
    Bool(bool),
}

impl AbiValue {
    fn to_libffi_arg(&self) -> libffi::middle::Arg {
        match self {
            AbiValue::Int(ref i) => libffi::middle::Arg::new(i),
            AbiValue::Float(ref f) => libffi::middle::Arg::new(f),
            AbiValue::Bool(ref b) => libffi::middle::Arg::new(b),
        }
    }
}

impl From<i64> for AbiValue {
    fn from(i: i64) -> Self {
        AbiValue::Int(i)
    }
}

impl From<f64> for AbiValue {
    fn from(f: f64) -> Self {
        AbiValue::Float(f)
    }
}

impl From<bool> for AbiValue {
    fn from(b: bool) -> Self {
        AbiValue::Bool(b)
    }
}

impl TryFrom<AbiValue> for i64 {
    type Error = ();

    fn try_from(value: AbiValue) -> Result<Self, Self::Error> {
        match value {
            AbiValue::Int(i) => Ok(i),
            _ => Err(()),
        }
    }
}

impl TryFrom<AbiValue> for f64 {
    type Error = ();

    fn try_from(value: AbiValue) -> Result<Self, Self::Error> {
        match value {
            AbiValue::Float(f) => Ok(f),
            _ => Err(()),
        }
    }
}

impl TryFrom<AbiValue> for bool {
    type Error = ();

    fn try_from(value: AbiValue) -> Result<Self, Self::Error> {
        match value {
            AbiValue::Bool(b) => Ok(b),
            _ => Err(()),
        }
    }
}

fn type_check(ty: &JitType, val: &AbiValue) -> Result<(), JitArgumentError> {
    match (ty, val) {
        (JitType::Int, AbiValue::Int(_))
        | (JitType::Float, AbiValue::Float(_))
        | (JitType::Bool, AbiValue::Bool(_)) => Ok(()),
        _ => Err(JitArgumentError::ArgumentTypeMismatch),
    }
}

#[derive(Copy, Clone)]
union UnTypedAbiValue {
    float: f64,
    int: i64,
    boolean: u8,
    _void: (),
}

impl UnTypedAbiValue {
    unsafe fn to_typed(&self, ty: &JitType) -> AbiValue {
        match ty {
            JitType::Int => AbiValue::Int(self.int),
            JitType::Float => AbiValue::Float(self.float),
            JitType::Bool => AbiValue::Bool(self.boolean != 0),
        }
    }
}

unsafe impl Send for CompiledCode {}
unsafe impl Sync for CompiledCode {}

impl Drop for CompiledCode {
    fn drop(&mut self) {
        // SAFETY: The only pointer that this memory will also be dropped now
        unsafe { self.module.free_memory() }
    }
}

impl fmt::Debug for CompiledCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[compiled code]")
    }
}

pub struct ArgsBuilder<'a> {
    values: Vec<Option<AbiValue>>,
    code: &'a CompiledCode,
}

impl<'a> ArgsBuilder<'a> {
    fn new(code: &'a CompiledCode) -> ArgsBuilder<'a> {
        ArgsBuilder {
            values: vec![None; code.sig.args.len()],
            code,
        }
    }

    pub fn set(&mut self, idx: usize, value: AbiValue) -> Result<(), JitArgumentError> {
        type_check(&self.code.sig.args[idx], &value).map(|_| {
            self.values[idx] = Some(value);
        })
    }

    pub fn is_set(&self, idx: usize) -> bool {
        self.values[idx].is_some()
    }

    pub fn into_args(self) -> Option<Args<'a>> {
        self.values
            .iter()
            .map(|v| v.as_ref().map(AbiValue::to_libffi_arg))
            .collect::<Option<_>>()
            .map(|cif_args| Args {
                _values: self.values,
                cif_args,
                code: self.code,
            })
    }
}

pub struct Args<'a> {
    _values: Vec<Option<AbiValue>>,
    cif_args: Vec<libffi::middle::Arg>,
    code: &'a CompiledCode,
}

impl<'a> Args<'a> {
    pub fn invoke(&self) -> Option<AbiValue> {
        unsafe { self.code.invoke_raw(&self.cif_args) }
    }
}
