mod instructions;

extern crate alloc;

use alloc::fmt;
use alloc::sync::Arc;
use core::mem::{self, ManuallyDrop};
use core::sync::atomic::{AtomicU64, Ordering};
use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module, ModuleError};
use instructions::FunctionCompiler;
use rustpython_compiler_core::bytecode;
use std::sync::{Mutex, PoisonError};

/// Arguments cross into compiled code through a flat buffer of 64-bit slots:
/// an int sign-extended, a bool as 0 or 1, a float as its bits. The buffer is
/// a fixed-size array so that a call allocates nothing, which caps how many
/// parameters a function can have and still be compiled.
const MAX_ARGS: usize = 16;
const SLOT_SIZE: usize = size_of::<u64>();

/// A guard that fires leaves its record in a second flat buffer:
///
/// ```text
/// deopt[0]   status: 0 when the call returned, otherwise the site index plus one
/// deopt[1]   bound mask: bit i set when the site's i-th listed local is bound
/// deopt[2..] the listed locals, then the value stack bottom to top
/// ```
///
/// Like the argument buffer it is a fixed-size array so that a call allocates
/// nothing, which caps how much state a guard can spill.
const MAX_DEOPT_SLOTS: usize = 64;
/// Slots taken by the status and the bound mask, before the record starts.
#[expect(dead_code, reason = "read once a guard writes a record")]
const DEOPT_HEADER_SLOTS: usize = 2;

/// The entry point of a compiled function: `(args, ret, deopt)`.
type JitEntry = unsafe extern "C" fn(*const u64, *mut u64, *mut u64);

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
        let ptr_type = self.module.target_config().pointer_type();
        // The deopt buffer comes first so that a guard can reach it without
        // depending on how many parameters the function has.
        self.ctx.func.signature.params.push(AbiParam::new(ptr_type));

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

        // Compiling the body can widen the signature: a return type that was
        // not annotated is only learned from the return statements.
        let body_signature = self.ctx.func.signature.clone();
        self.module.define_function(id, &mut self.ctx)?;
        self.module.clear_context(&mut self.ctx);

        let entry = self.build_entry(id, body_signature, &sig, unique)?;

        Ok((entry, sig))
    }

    /// Build the entry point callers go through: it takes a flat buffer of
    /// 64-bit slots, unpacks them into the parameters the compiled body
    /// actually takes, and writes the result back into the caller's slot.
    ///
    /// This is what makes the call a plain indirect call. Handing the same
    /// arguments to a foreign-function library instead means describing the
    /// signature and boxing every argument on every call.
    fn build_entry(
        &mut self,
        target: FuncId,
        target_signature: Signature,
        sig: &JitSig,
        unique: u64,
    ) -> Result<FuncId, JitCompileError> {
        let ptr_type = self.module.target_config().pointer_type();
        // (args, ret, deopt)
        self.ctx.func.signature.params.push(AbiParam::new(ptr_type));
        self.ctx.func.signature.params.push(AbiParam::new(ptr_type));
        self.ctx.func.signature.params.push(AbiParam::new(ptr_type));

        let id = self.module.declare_function(
            &format!("jit_entry_{unique}"),
            Linkage::Export,
            &self.ctx.func.signature,
        )?;

        let callee = self.module.declare_func_in_func(target, &mut self.ctx.func);
        // The import carries the signature the target was declared with, which
        // is the one from before the body widened it.
        let callee_signature = self.ctx.func.import_signature(target_signature);
        self.ctx.func.dfg.ext_funcs[callee].signature = callee_signature;

        let mut builder = FunctionBuilder::new(&mut self.ctx.func, &mut self.builder_context);
        let block = builder.create_block();
        builder.append_block_params_for_function_params(block);
        builder.switch_to_block(block);
        let args_ptr = builder.block_params(block)[0];
        let ret_ptr = builder.block_params(block)[1];
        let deopt_ptr = builder.block_params(block)[2];
        // Written before anything can read it, so the buffer never has to be zeroed.
        let zero = builder.ins().iconst(types::I64, 0);
        builder.ins().store(MemFlags::trusted(), zero, deopt_ptr, 0);

        let mut call_args = vec![deopt_ptr];
        for (i, ty) in sig.args.iter().enumerate() {
            let offset = i32::try_from(i * SLOT_SIZE).map_err(|_| JitCompileError::NotSupported)?;
            let mut load = |ty| {
                builder
                    .ins()
                    .load(ty, MemFlags::trusted(), args_ptr, offset)
            };
            call_args.push(match ty {
                JitType::Int => load(types::I64),
                JitType::Float => load(types::F64),
                // A slot holds 0 or 1, so the low byte carries the whole value
                // whichever end of the slot it sits at.
                JitType::Bool => {
                    let slot = load(types::I64);
                    builder.ins().ireduce(types::I8, slot)
                }
                JitType::None => return Err(JitCompileError::NotSupported),
            });
        }

        let call = builder.ins().call(callee, &call_args);
        let returned = match sig.ret.as_ref().filter(|ty| ty.to_cranelift().is_some()) {
            Some(ty) => match *builder.inst_results(call) {
                [result] => Some((ty, result)),
                _ => return Err(JitCompileError::NotSupported),
            },
            None => None,
        };
        if let Some((ty, result)) = returned {
            let result = if *ty == JitType::Bool {
                builder.ins().uextend(types::I64, result)
            } else {
                result
            };
            builder.ins().store(MemFlags::trusted(), result, ret_ptr, 0);
        }
        builder.ins().return_(&[]);

        builder.seal_all_blocks();
        builder.finalize();

        self.module.define_function(id, &mut self.ctx)?;

        Ok(id)
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
        if args.len() > MAX_ARGS {
            return Err(JitCompileError::NotSupported);
        }
        // Symbol names must be unique within the module, and `obj_name` is not:
        // any two `def f` in different scopes collide.
        let unique = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut jit = self.jit.lock().unwrap_or_else(PoisonError::into_inner);
        let (id, sig) = jit.build_function(bytecode, args, ret, unique, safety)?;
        jit.module.finalize_definitions()?;
        let code = jit.module.get_finalized_function(id);
        drop(jit);
        // SAFETY: `build_entry` defined this function with exactly this
        // signature, and the engine keeps its code alive.
        let entry = unsafe { mem::transmute::<*const u8, JitEntry>(code) };
        Ok(CompiledCode {
            sig,
            entry,
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

/// Whether the backend could plausibly compile this code object.
///
/// A cheap pre-filter for callers that compile speculatively: one pass over the
/// bytecode, ruling out the shapes there is no lowering for at all. Passing is
/// not a promise that compilation will succeed - the argument types decide much
/// of that, and they are not visible here.
pub fn supports_code<C: bytecode::Constant>(code: &bytecode::CodeObject<C>) -> bool {
    // A frame is never built, so there is nowhere to put varargs, a generator's
    // suspended state, or an exception handler's stack.
    if code.flags.intersects(
        bytecode::CodeFlags::VARARGS
            | bytecode::CodeFlags::VARKEYWORDS
            | bytecode::CodeFlags::GENERATOR
            | bytecode::CodeFlags::COROUTINE
            | bytecode::CodeFlags::ASYNC_GENERATOR,
    ) {
        return false;
    }
    if !code.exceptiontable.is_empty() {
        return false;
    }
    // Cells and frees live past `varnames`, which is all the compiler allocates
    // locals for, and are read through opcodes it has no lowering for.
    if !code.cellvars.is_empty() || !code.freevars.is_empty() {
        return false;
    }

    let mut state = bytecode::OpArgState::default();
    code.instructions.iter().all(|&word| {
        let (instruction, _) = state.get(word);
        instructions::instruction_is_supported(instruction)
    })
}

pub fn compile<C: bytecode::Constant>(
    bytecode: &bytecode::CodeObject<C>,
    args: &[JitType],
    ret: Option<JitType>,
) -> Result<CompiledCode, JitCompileError> {
    JitEngine::new().compile(bytecode, args, ret, Safety::Permissive)
}

pub struct CompiledCode {
    sig: JitSig,
    entry: JitEntry,
    /// Keeps the code memory alive; never read.
    _engine: Arc<JitEngine>,
}

impl CompiledCode {
    #[must_use]
    pub fn args_builder(&self) -> ArgsBuilder<'_> {
        ArgsBuilder::new(self)
    }

    pub fn invoke(&self, args: &[AbiValue]) -> Result<Outcome, JitArgumentError> {
        if self.sig.args.len() != args.len() {
            return Err(JitArgumentError::WrongNumberOfArguments);
        }

        let mut slots = [0; MAX_ARGS];
        for ((slot, ty), value) in slots.iter_mut().zip(&self.sig.args).zip(args) {
            type_check(ty, value)?;
            *slot = value.to_slot();
        }
        Ok(unsafe { self.invoke_raw(&slots) })
    }

    /// # Safety
    /// `slots` must hold a value of the right type for each parameter.
    unsafe fn invoke_raw(&self, slots: &[u64; MAX_ARGS]) -> Outcome {
        let mut ret = 0;
        // Only slot 0 is written before it is read, by the entry point itself;
        // zeroing the rest would cost more per call than the code being called.
        let mut deopt = core::mem::MaybeUninit::<[u64; MAX_DEOPT_SLOTS]>::uninit();
        let deopt_ptr = deopt.as_mut_ptr().cast::<u64>();
        // SAFETY: the entry point reads one slot per parameter, writes the return
        // slot only when the signature says it returns something, and writes the
        // deopt status before returning.
        unsafe { (self.entry)(slots.as_ptr(), &raw mut ret, deopt_ptr) }
        // SAFETY: the entry point stores the status first thing.
        let status = unsafe { deopt_ptr.read() };
        if status != 0 {
            // SAFETY: the site describes exactly which slots the guard wrote.
            return Outcome::Deopt(unsafe { self.read_deopt(status, deopt_ptr) });
        }
        Outcome::Returned(match self.sig.ret.as_ref() {
            Some(JitType::None) | None => None,
            Some(ty) => Some(AbiValue::from_slot(ty, ret)),
        })
    }

    /// # Safety
    /// `status` must be a status this code's guards can produce, and `deopt` must
    /// point at the buffer they wrote.
    unsafe fn read_deopt(&self, _status: u64, _deopt: *const u64) -> DeoptState {
        unreachable!("nothing writes a non-zero status yet")
    }
}

/// What a call to compiled code did.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Returned(Option<AbiValue>),
    /// A guard fired. The record is decoded here, off the hot path, so that
    /// nothing borrows the buffer once it goes out of scope.
    Deopt(DeoptState),
}

/// Everything the interpreter needs to pick up where the guard stopped.
#[derive(Debug, PartialEq)]
pub struct DeoptState {
    /// Bytecode offset to resume at.
    pub offset: u32,
    /// One entry per varname slot; `None` where the local is not live or is
    /// unbound.
    pub locals: Vec<Option<AbiValue>>,
    /// Bottom to top.
    pub stack: Vec<AbiValue>,
}

struct JitSig {
    args: Vec<JitType>,
    ret: Option<JitType>,
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
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum AbiValue {
    Float(f64),
    Int(i64),
    Bool(bool),
}

impl AbiValue {
    /// Pack into the 64-bit slot the entry point reads.
    fn to_slot(&self) -> u64 {
        match *self {
            Self::Int(i) => i as u64,
            Self::Float(f) => f.to_bits(),
            Self::Bool(b) => b.into(),
        }
    }

    fn from_slot(ty: &JitType, slot: u64) -> Self {
        match ty {
            JitType::Int => Self::Int(slot as i64),
            JitType::Float => Self::Float(f64::from_bits(slot)),
            JitType::Bool => Self::Bool(slot != 0),
            JitType::None => unreachable!("None has no slot"),
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

impl fmt::Debug for CompiledCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[compiled code]")
    }
}

pub struct ArgsBuilder<'a> {
    slots: [u64; MAX_ARGS],
    /// One bit per filled slot, so a caller can tell an argument it has
    /// already placed from one still to come.
    filled: u32,
    code: &'a CompiledCode,
}

impl<'a> ArgsBuilder<'a> {
    #[must_use]
    fn new(code: &'a CompiledCode) -> Self {
        Self {
            slots: [0; MAX_ARGS],
            filled: 0,
            code,
        }
    }

    pub fn set(&mut self, idx: usize, value: AbiValue) -> Result<(), JitArgumentError> {
        type_check(&self.code.sig.args[idx], &value).map(|()| {
            self.slots[idx] = value.to_slot();
            self.filled |= 1 << idx;
        })
    }

    #[must_use]
    pub fn is_set(&self, idx: usize) -> bool {
        self.filled & (1 << idx) != 0
    }

    #[must_use]
    pub fn into_args(self) -> Option<Args<'a>> {
        let wanted = (1 << self.code.sig.args.len()) - 1;
        (self.filled == wanted).then_some(Args {
            slots: self.slots,
            code: self.code,
        })
    }
}

pub struct Args<'a> {
    slots: [u64; MAX_ARGS],
    code: &'a CompiledCode,
}

impl Args<'_> {
    #[must_use]
    pub fn invoke(&self) -> Outcome {
        // SAFETY: `into_args` only hands out `Args` once every parameter has a
        // slot, and `set` type-checked each one against the signature.
        unsafe { self.code.invoke_raw(&self.slots) }
    }
}
