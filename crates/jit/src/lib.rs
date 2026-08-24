mod instructions;

extern crate alloc;

use alloc::fmt;
use alloc::sync::Arc;
use core::mem::ManuallyDrop;
use core::sync::atomic::{AtomicU64, Ordering};
use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module, ModuleError};
use instructions::FunctionCompiler;
use rustpython_compiler_core::bytecode;
use std::sync::{Mutex, PoisonError};

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum JitCompileError {
    #[error("function can't be jitted")]
    NotSupported,
    #[error("bad bytecode")]
    BadBytecode,
    #[error("error while compiling to machine code: {0}")]
    CraneliftError(Box<ModuleError>),
}

impl From<ModuleError> for JitCompileError {
    fn from(err: ModuleError) -> Self {
        Self::CraneliftError(Box::new(err))
    }
}

/// How far the compiled code is allowed to diverge from interpreted semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Safety {
    /// Reject every operation whose machine code can trap, wrap, or otherwise
    /// answer differently from the interpreter. Traps have no handler and kill
    /// the process, and a wrapped integer is a silently wrong result, so code
    /// that was compiled without being asked for must not reach either.
    Strict,
    /// Compile everything the backend supports.
    Permissive,
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
#[non_exhaustive]
pub enum JitArgumentError {
    #[error("argument is of wrong type")]
    ArgumentTypeMismatch,
    #[error("wrong number of arguments")]
    WrongNumberOfArguments,
}

struct Jit {
    builder_context: FunctionBuilderContext,
    ctx: codegen::Context,
    module: ManuallyDrop<JITModule>,
}

impl Jit {
    fn new() -> Self {
        let builder = JITBuilder::new(cranelift_module::default_libcall_names())
            .expect("Failed to build JITBuilder");
        let module = JITModule::new(builder);
        Self {
            builder_context: FunctionBuilderContext::new(),
            ctx: module.make_context(),
            module: ManuallyDrop::new(module),
        }
    }

    /// Build one function into the module. The context is reset even when
    /// compilation fails, so a rejected function leaves nothing behind for the
    /// next one to trip over.
    fn build_function<C: bytecode::Constant>(
        &mut self,
        bytecode: &bytecode::CodeObject<C>,
        args: &[JitType],
        ret: Option<JitType>,
        unique: u64,
        safety: Safety,
    ) -> Result<(FuncId, JitSig), JitCompileError> {
        let result = self.build_function_inner(bytecode, args, ret, unique, safety);
        self.module.clear_context(&mut self.ctx);
        if result.is_err() {
            // Only `FunctionBuilder::finalize` resets the builder context, and
            // a rejected function never reaches it. Leaving it dirty makes the
            // next `FunctionBuilder::new` panic.
            self.builder_context = FunctionBuilderContext::new();
        }
        result
    }

    fn build_function_inner<C: bytecode::Constant>(
        &mut self,
        bytecode: &bytecode::CodeObject<C>,
        args: &[JitType],
        ret: Option<JitType>,
        unique: u64,
        safety: Safety,
    ) -> Result<(FuncId, JitSig), JitCompileError> {
        for arg in args {
            let arg = arg.to_cranelift().ok_or(JitCompileError::NotSupported)?;
            self.ctx.func.signature.params.push(AbiParam::new(arg));
        }

        if let Some(ret) = ret.as_ref().and_then(JitType::to_cranelift) {
            self.ctx.func.signature.returns.push(AbiParam::new(ret));
        }

        let id = self.module.declare_function(
            &format!("jit_{}_{unique}", bytecode.obj_name.as_ref()),
            Linkage::Export,
            &self.ctx.func.signature,
        )?;

        let func_ref = self.module.declare_func_in_func(id, &mut self.ctx.func);

        let mut builder = FunctionBuilder::new(&mut self.ctx.func, &mut self.builder_context);
        let entry_block = builder.create_block();
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);

        let sig = {
            let mut compiler = FunctionCompiler::new(
                &mut builder,
                bytecode.varnames.len(),
                args,
                ret,
                entry_block,
                safety,
            );

            compiler.compile(func_ref, bytecode)?;

            compiler.sig
        };

        builder.seal_all_blocks();
        builder.finalize();

        self.module.define_function(id, &mut self.ctx)?;

        Ok((id, sig))
    }
}

/// Owns the code memory of every function compiled through it. A
/// [`CompiledCode`] keeps its engine alive, so machine code is never freed
/// while something can still call it.
pub struct JitEngine {
    jit: Mutex<Jit>,
    next_id: AtomicU64,
}

impl JitEngine {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            jit: Mutex::new(Jit::new()),
            next_id: AtomicU64::new(0),
        })
    }

    pub fn compile<C: bytecode::Constant>(
        self: &Arc<Self>,
        bytecode: &bytecode::CodeObject<C>,
        args: &[JitType],
        ret: Option<JitType>,
        safety: Safety,
    ) -> Result<CompiledCode, JitCompileError> {
        // Symbol names must be unique within the module, and `obj_name` is not:
        // any two `def f` in different scopes collide.
        let unique = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut jit = self.jit.lock().unwrap_or_else(PoisonError::into_inner);
        let (id, sig) = jit.build_function(bytecode, args, ret, unique, safety)?;
        jit.module.finalize_definitions()?;
        let code = jit.module.get_finalized_function(id);
        drop(jit);
        Ok(CompiledCode {
            sig,
            code,
            _engine: self.clone(),
        })
    }
}

impl Drop for JitEngine {
    fn drop(&mut self) {
        let jit = self.jit.get_mut().unwrap_or_else(PoisonError::into_inner);
        // SAFETY: every CompiledCode holds an Arc to this engine, so no
        // compiled function is reachable any more once we get here.
        unsafe { ManuallyDrop::take(&mut jit.module).free_memory() }
    }
}

// The module is only ever touched under the mutex, and the code pointers it
// hands out stay valid for as long as the engine lives.
unsafe impl Send for JitEngine {}
unsafe impl Sync for JitEngine {}

pub fn compile<C: bytecode::Constant>(
    bytecode: &bytecode::CodeObject<C>,
    args: &[JitType],
    ret: Option<JitType>,
) -> Result<CompiledCode, JitCompileError> {
    JitEngine::new().compile(bytecode, args, ret, Safety::Permissive)
}

pub struct CompiledCode {
    sig: JitSig,
    code: *const u8,
    /// Keeps the code memory alive; never read.
    _engine: Arc<JitEngine>,
}

impl CompiledCode {
    #[must_use]
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

    unsafe fn invoke_raw(&self, cif_args: &[libffi::middle::Arg<'_>]) -> Option<AbiValue> {
        unsafe {
            let cif = self.sig.to_cif();
            let value = cif.call::<UnTypedAbiValue>(
                libffi::middle::CodePtr::from_ptr(self.code as *const _),
                cif_args,
            );
            match self.sig.ret.as_ref() {
                Some(JitType::None) | None => None,
                Some(ty) => Some(value.to_typed(ty)),
            }
        }
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

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum JitType {
    Int,
    Float,
    Bool,
    None,
}

impl JitType {
    fn to_cranelift(&self) -> Option<types::Type> {
        match self {
            Self::Int => Some(types::I64),
            Self::Float => Some(types::F64),
            Self::Bool => Some(types::I8),
            Self::None => None,
        }
    }

    fn to_libffi(&self) -> libffi::middle::Type {
        match self {
            Self::Int => libffi::middle::Type::i64(),
            Self::Float => libffi::middle::Type::f64(),
            Self::Bool => libffi::middle::Type::u8(),
            Self::None => libffi::middle::Type::void(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum AbiValue {
    Float(f64),
    Int(i64),
    Bool(bool),
}

impl AbiValue {
    fn to_libffi_arg(&self) -> libffi::middle::Arg<'_> {
        match self {
            Self::Int(i) => libffi::middle::Arg::new(i),
            Self::Float(f) => libffi::middle::Arg::new(f),
            Self::Bool(b) => libffi::middle::Arg::new(b),
        }
    }
}

impl From<i64> for AbiValue {
    fn from(i: i64) -> Self {
        Self::Int(i)
    }
}

impl From<f64> for AbiValue {
    fn from(f: f64) -> Self {
        Self::Float(f)
    }
}

impl From<bool> for AbiValue {
    fn from(b: bool) -> Self {
        Self::Bool(b)
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
    unsafe fn to_typed(self, ty: &JitType) -> AbiValue {
        unsafe {
            match ty {
                JitType::Int => AbiValue::Int(self.int),
                JitType::Float => AbiValue::Float(self.float),
                JitType::Bool => AbiValue::Bool(self.boolean != 0),
                JitType::None => unreachable!("None has no ABI value"),
            }
        }
    }
}

// we don't actually ever touch CompiledCode til we drop it, it should be safe.
// TODO: confirm with wasmtime ppl that it's not unsound?
unsafe impl Send for CompiledCode {}
unsafe impl Sync for CompiledCode {}

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
    #[must_use]
    fn new(code: &'a CompiledCode) -> Self {
        Self {
            values: vec![None; code.sig.args.len()],
            code,
        }
    }

    pub fn set(&mut self, idx: usize, value: AbiValue) -> Result<(), JitArgumentError> {
        type_check(&self.code.sig.args[idx], &value).map(|_| {
            self.values[idx] = Some(value);
        })
    }

    #[must_use]
    pub fn is_set(&self, idx: usize) -> bool {
        self.values[idx].is_some()
    }

    #[must_use]
    pub fn into_args(self) -> Option<Args<'a>> {
        // Ensure all values are set
        if self.values.iter().any(|v| v.is_none()) {
            return None;
        }
        Some(Args {
            values: self.values.into_iter().map(|v| v.unwrap()).collect(),
            code: self.code,
        })
    }
}

pub struct Args<'a> {
    values: Vec<AbiValue>,
    code: &'a CompiledCode,
}

impl Args<'_> {
    #[must_use]
    pub fn invoke(&self) -> Option<AbiValue> {
        let cif_args: Vec<_> = self.values.iter().map(AbiValue::to_libffi_arg).collect();
        unsafe { self.code.invoke_raw(&cif_args) }
    }
}
