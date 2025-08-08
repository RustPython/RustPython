use crate::common::{boxvec::BoxVec, lock::PyMutex};
use crate::protocol::PyMapping;
use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject, VirtualMachine,
    builtins::{
        PyBaseExceptionRef, PyCode, PyCoroutine, PyDict, PyDictRef, PyGenerator, PyList, PySet,
        PySlice, PyStr, PyStrInterned, PyStrRef, PyTraceback, PyType,
        asyncgenerator::PyAsyncGenWrappedValue,
        function::{PyCell, PyCellRef, PyFunction},
        tuple::{PyTuple, PyTupleRef},
    },
    bytecode,
    convert::{IntoObject, ToPyResult},
    coroutine::Coro,
    exceptions::ExceptionCtor,
    function::{ArgMapping, Either, FuncArgs},
    protocol::{PyIter, PyIterReturn},
    scope::Scope,
    stdlib::{builtins, typing},
    vm::{Context, PyMethod},
};
use indexmap::IndexMap;
use itertools::Itertools;
use rustpython_common::wtf8::Wtf8Buf;
use rustpython_compiler_core::SourceLocation;
#[cfg(feature = "threading")]
use std::sync::atomic;
use std::{fmt, iter::zip};

#[derive(Clone, Debug)]
struct Block {
    /// The type of block.
    typ: BlockType,
    /// The level of the value stack when the block was entered.
    level: usize,
}

#[derive(Clone, Debug)]
enum BlockType {
    Loop,
    TryExcept {
        handler: bytecode::Label,
    },
    Finally {
        handler: bytecode::Label,
    },

    /// Active finally sequence
    FinallyHandler {
        reason: Option<UnwindReason>,
        prev_exc: Option<PyBaseExceptionRef>,
    },
    ExceptHandler {
        prev_exc: Option<PyBaseExceptionRef>,
    },
}

pub type FrameRef = PyRef<Frame>;

/// The reason why we might be unwinding a block.
/// This could be return of function, exception being
/// raised, a break or continue being hit, etc..
#[derive(Clone, Debug)]
enum UnwindReason {
    /// We are returning a value from a return statement.
    Returning { value: PyObjectRef },

    /// We hit an exception, so unwind any try-except and finally blocks. The exception should be
    /// on top of the vm exception stack.
    Raising { exception: PyBaseExceptionRef },

    // NoWorries,
    /// We are unwinding blocks, since we hit break
    Break { target: bytecode::Label },

    /// We are unwinding blocks since we hit a continue statements.
    Continue { target: bytecode::Label },
}

#[derive(Debug)]
struct FrameState {
    // We need 1 stack per frame
    /// The main data frame of the stack machine
    stack: BoxVec<PyObjectRef>,
    /// Block frames, for controlling loops and exceptions
    blocks: Vec<Block>,
    /// index of last instruction ran
    #[cfg(feature = "threading")]
    lasti: u32,
}

#[cfg(feature = "threading")]
type Lasti = atomic::AtomicU32;
#[cfg(not(feature = "threading"))]
type Lasti = std::cell::Cell<u32>;

#[pyclass(module = false, name = "frame")]
pub struct Frame {
    pub code: PyRef<PyCode>,
    pub func_obj: Option<PyObjectRef>,

    pub fastlocals: PyMutex<Box<[Option<PyObjectRef>]>>,
    pub(crate) cells_frees: Box<[PyCellRef]>,
    pub locals: ArgMapping,
    pub globals: PyDictRef,
    pub builtins: PyDictRef,

    // on feature=threading, this is a duplicate of FrameState.lasti, but it's faster to do an
    // atomic store than it is to do a fetch_add, for every instruction executed
    /// index of last instruction ran
    pub lasti: Lasti,
    /// tracer function for this frame (usually is None)
    pub trace: PyMutex<PyObjectRef>,
    state: PyMutex<FrameState>,

    // member
    pub trace_lines: PyMutex<bool>,
    pub temporary_refs: PyMutex<Vec<PyObjectRef>>,
}

impl PyPayload for Frame {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.frame_type
    }
}

// Running a frame can result in one of the below:
pub enum ExecutionResult {
    Return(PyObjectRef),
    Yield(PyObjectRef),
}

/// A valid execution result, or an exception
type FrameResult = PyResult<Option<ExecutionResult>>;

impl Frame {
    pub(crate) fn new(
        code: PyRef<PyCode>,
        scope: Scope,
        builtins: PyDictRef,
        closure: &[PyCellRef],
        func_obj: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> Self {
        let cells_frees = std::iter::repeat_with(|| PyCell::default().into_ref(&vm.ctx))
            .take(code.cellvars.len())
            .chain(closure.iter().cloned())
            .collect();

        let state = FrameState {
            stack: BoxVec::new(code.max_stackdepth as usize),
            blocks: Vec::new(),
            #[cfg(feature = "threading")]
            lasti: 0,
        };

        Self {
            fastlocals: PyMutex::new(vec![None; code.varnames.len()].into_boxed_slice()),
            cells_frees,
            locals: scope.locals,
            globals: scope.globals,
            builtins,
            code,
            func_obj,
            lasti: Lasti::new(0),
            state: PyMutex::new(state),
            trace: PyMutex::new(vm.ctx.none()),
            trace_lines: PyMutex::new(true),
            temporary_refs: PyMutex::new(vec![]),
        }
    }

    pub fn current_location(&self) -> SourceLocation {
        self.code.locations[self.lasti() as usize - 1].clone()
    }

    pub fn lasti(&self) -> u32 {
        #[cfg(feature = "threading")]
        {
            self.lasti.load(atomic::Ordering::Relaxed)
        }
        #[cfg(not(feature = "threading"))]
        {
            self.lasti.get()
        }
    }

    pub fn locals(&self, vm: &VirtualMachine) -> PyResult<ArgMapping> {
        let locals = &self.locals;
        let code = &**self.code;
        let map = &code.varnames;
        let j = std::cmp::min(map.len(), code.varnames.len());
        if !code.varnames.is_empty() {
            let fastlocals = self.fastlocals.lock();
            for (&k, v) in zip(&map[..j], &**fastlocals) {
                match locals.mapping().ass_subscript(k, v.clone(), vm) {
                    Ok(()) => {}
                    Err(e) if e.fast_isinstance(vm.ctx.exceptions.key_error) => {}
                    Err(e) => return Err(e),
                }
            }
        }
        if !code.cellvars.is_empty() || !code.freevars.is_empty() {
            let map_to_dict = |keys: &[&PyStrInterned], values: &[PyCellRef]| {
                for (&k, v) in zip(keys, values) {
                    if let Some(value) = v.get() {
                        locals.mapping().ass_subscript(k, Some(value), vm)?;
                    } else {
                        match locals.mapping().ass_subscript(k, None, vm) {
                            Ok(()) => {}
                            Err(e) if e.fast_isinstance(vm.ctx.exceptions.key_error) => {}
                            Err(e) => return Err(e),
                        }
                    }
                }
                Ok(())
            };
            map_to_dict(&code.cellvars, &self.cells_frees)?;
            if code.flags.contains(bytecode::CodeFlags::IS_OPTIMIZED) {
                map_to_dict(&code.freevars, &self.cells_frees[code.cellvars.len()..])?;
            }
        }
        Ok(locals.clone())
    }
}

impl Py<Frame> {
    #[inline(always)]
    fn with_exec<R>(&self, f: impl FnOnce(ExecutingFrame<'_>) -> R) -> R {
        let mut state = self.state.lock();
        let exec = ExecutingFrame {
            code: &self.code,
            fastlocals: &self.fastlocals,
            cells_frees: &self.cells_frees,
            locals: &self.locals,
            globals: &self.globals,
            builtins: &self.builtins,
            lasti: &self.lasti,
            object: self,
            state: &mut state,
        };
        f(exec)
    }

    // #[cfg_attr(feature = "flame-it", flame("Frame"))]
    pub fn run(&self, vm: &VirtualMachine) -> PyResult<ExecutionResult> {
        self.with_exec(|mut exec| exec.run(vm))
    }

    pub(crate) fn resume(
        &self,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<ExecutionResult> {
        self.with_exec(|mut exec| {
            if let Some(value) = value {
                exec.push_value(value)
            }
            exec.run(vm)
        })
    }

    pub(crate) fn gen_throw(
        &self,
        vm: &VirtualMachine,
        exc_type: PyObjectRef,
        exc_val: PyObjectRef,
        exc_tb: PyObjectRef,
    ) -> PyResult<ExecutionResult> {
        self.with_exec(|mut exec| exec.gen_throw(vm, exc_type, exc_val, exc_tb))
    }

    pub fn yield_from_target(&self) -> Option<PyObjectRef> {
        self.with_exec(|exec| exec.yield_from_target().map(PyObject::to_owned))
    }

    pub fn is_internal_frame(&self) -> bool {
        let code = self.f_code();
        let filename = code.co_filename();
        let filename_s = filename.as_str();
        filename_s.contains("importlib") && filename_s.contains("_bootstrap")
    }

    pub fn next_external_frame(&self, vm: &VirtualMachine) -> Option<FrameRef> {
        self.f_back(vm).map(|mut back| {
            loop {
                back = if let Some(back) = back.to_owned().f_back(vm) {
                    back
                } else {
                    break back;
                };

                if !back.is_internal_frame() {
                    break back;
                }
            }
        })
    }
}

/// An executing frame; essentially just a struct to combine the immutable data outside the mutex
/// with the mutable data inside
struct ExecutingFrame<'a> {
    code: &'a PyRef<PyCode>,
    fastlocals: &'a PyMutex<Box<[Option<PyObjectRef>]>>,
    cells_frees: &'a [PyCellRef],
    locals: &'a ArgMapping,
    globals: &'a PyDictRef,
    builtins: &'a PyDictRef,
    object: &'a Py<Frame>,
    lasti: &'a Lasti,
    state: &'a mut FrameState,
}

impl fmt::Debug for ExecutingFrame<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecutingFrame")
            .field("code", self.code)
            // .field("scope", self.scope)
            .field("state", self.state)
            .finish()
    }
}

impl ExecutingFrame<'_> {
    #[inline(always)]
    fn update_lasti(&mut self, f: impl FnOnce(&mut u32)) {
        #[cfg(feature = "threading")]
        {
            f(&mut self.state.lasti);
            self.lasti
                .store(self.state.lasti, atomic::Ordering::Relaxed);
        }
        #[cfg(not(feature = "threading"))]
        {
            let mut lasti = self.lasti.get();
            f(&mut lasti);
            self.lasti.set(lasti);
        }
    }

    #[inline(always)]
    const fn lasti(&self) -> u32 {
        #[cfg(feature = "threading")]
        {
            self.state.lasti
        }
        #[cfg(not(feature = "threading"))]
        {
            self.lasti.get()
        }
    }

    fn run(&mut self, vm: &VirtualMachine) -> PyResult<ExecutionResult> {
        flame_guard!(format!(
            "Frame::run({obj_name})",
            obj_name = self.code.obj_name
        ));
        // Execute until return or exception:
        let instructions = &self.code.instructions;
        let mut arg_state = bytecode::OpArgState::default();
        loop {
            let idx = self.lasti() as usize;
            // eprintln!(
            //     "location: {:?} {}",
            //     self.code.locations[idx], self.code.source_path
            // );
            self.update_lasti(|i| *i += 1);
            let bytecode::CodeUnit { op, arg } = instructions[idx];
            let arg = arg_state.extend(arg);
            let mut do_extend_arg = false;
            let result = self.execute_instruction(op, arg, &mut do_extend_arg, vm);
            match result {
                Ok(None) => {}
                Ok(Some(value)) => {
                    break Ok(value);
                }
                // Instruction raised an exception
                Err(exception) => {
                    #[cold]
                    fn handle_exception(
                        frame: &mut ExecutingFrame<'_>,
                        exception: PyBaseExceptionRef,
                        idx: usize,
                        vm: &VirtualMachine,
                    ) -> FrameResult {
                        // 1. Extract traceback from exception's '__traceback__' attr.
                        // 2. Add new entry with current execution position (filename, lineno, code_object) to traceback.
                        // 3. Unwind block stack till appropriate handler is found.

                        let loc = frame.code.locations[idx].clone();
                        let next = exception.__traceback__();
                        let new_traceback =
                            PyTraceback::new(next, frame.object.to_owned(), frame.lasti(), loc.row);
                        vm_trace!("Adding to traceback: {:?} {:?}", new_traceback, loc.row);
                        exception.set_traceback_typed(Some(new_traceback.into_ref(&vm.ctx)));

                        vm.contextualize_exception(&exception);

                        frame.unwind_blocks(vm, UnwindReason::Raising { exception })
                    }

                    match handle_exception(self, exception, idx, vm) {
                        Ok(None) => {}
                        Ok(Some(result)) => break Ok(result),
                        // TODO: append line number to traceback?
                        // traceback.append();
                        Err(exception) => break Err(exception),
                    }
                }
            }
            if !do_extend_arg {
                arg_state.reset()
            }
        }
    }

    fn yield_from_target(&self) -> Option<&PyObject> {
        if let Some(bytecode::CodeUnit {
            op: bytecode::Instruction::YieldFrom,
            ..
        }) = self.code.instructions.get(self.lasti() as usize)
        {
            Some(self.top_value())
        } else {
            None
        }
    }

    /// Ok(Err(e)) means that an error occurred while calling throw() and the generator should try
    /// sending it
    fn gen_throw(
        &mut self,
        vm: &VirtualMachine,
        exc_type: PyObjectRef,
        exc_val: PyObjectRef,
        exc_tb: PyObjectRef,
    ) -> PyResult<ExecutionResult> {
        if let Some(jen) = self.yield_from_target() {
            // borrow checker shenanigans - we only need to use exc_type/val/tb if the following
            // variable is Some
            let thrower = if let Some(coro) = self.builtin_coro(jen) {
                Some(Either::A(coro))
            } else {
                vm.get_attribute_opt(jen.to_owned(), "throw")?
                    .map(Either::B)
            };
            if let Some(thrower) = thrower {
                let ret = match thrower {
                    Either::A(coro) => coro
                        .throw(jen, exc_type, exc_val, exc_tb, vm)
                        .to_pyresult(vm), // FIXME:
                    Either::B(meth) => meth.call((exc_type, exc_val, exc_tb), vm),
                };
                return ret.map(ExecutionResult::Yield).or_else(|err| {
                    self.pop_value();
                    self.update_lasti(|i| *i += 1);
                    if err.fast_isinstance(vm.ctx.exceptions.stop_iteration) {
                        let val = vm.unwrap_or_none(err.get_arg(0));
                        self.push_value(val);
                        self.run(vm)
                    } else {
                        let (ty, val, tb) = vm.split_exception(err);
                        self.gen_throw(vm, ty, val, tb)
                    }
                });
            }
        }
        let exception = vm.normalize_exception(exc_type, exc_val, exc_tb)?;
        match self.unwind_blocks(vm, UnwindReason::Raising { exception }) {
            Ok(None) => self.run(vm),
            Ok(Some(result)) => Ok(result),
            Err(exception) => Err(exception),
        }
    }

    fn unbound_cell_exception(&self, i: usize, vm: &VirtualMachine) -> PyBaseExceptionRef {
        if let Some(&name) = self.code.cellvars.get(i) {
            vm.new_exception_msg(
                vm.ctx.exceptions.unbound_local_error.to_owned(),
                format!("local variable '{name}' referenced before assignment"),
            )
        } else {
            let name = self.code.freevars[i - self.code.cellvars.len()];
            vm.new_name_error(
                format!("free variable '{name}' referenced before assignment in enclosing scope"),
                name.to_owned(),
            )
        }
    }

    /// Execute a single instruction.
    #[inline(always)]
    fn execute_instruction(
        &mut self,
        instruction: bytecode::Instruction,
        arg: bytecode::OpArg,
        extend_arg: &mut bool,
        vm: &VirtualMachine,
    ) -> FrameResult {
        vm.check_signals()?;

        flame_guard!(format!(
            "Frame::execute_instruction({})",
            instruction.display(arg, &self.code.code).to_string()
        ));

        #[cfg(feature = "vm-tracing-logging")]
        {
            trace!("=======");
            /* TODO:
            for frame in self.frames.iter() {
                trace!("  {:?}", frame);
            }
            */
            trace!("  {:#?}", self);
            trace!(
                "  Executing op code: {}",
                instruction.display(arg, &self.code.code).to_string()
            );
            trace!("=======");
        }

        #[cold]
        fn name_error(name: &'static PyStrInterned, vm: &VirtualMachine) -> PyBaseExceptionRef {
            vm.new_name_error(format!("name '{name}' is not defined"), name.to_owned())
        }

        match instruction {
            bytecode::Instruction::Nop => Ok(None),
            bytecode::Instruction::LoadConst { idx } => {
                self.push_value(self.code.constants[idx.get(arg) as usize].clone().into());
                Ok(None)
            }
            bytecode::Instruction::ImportName { idx } => {
                self.import(vm, Some(self.code.names[idx.get(arg) as usize]))?;
                Ok(None)
            }
            bytecode::Instruction::ImportNameless => {
                self.import(vm, None)?;
                Ok(None)
            }
            bytecode::Instruction::ImportFrom { idx } => {
                let obj = self.import_from(vm, idx.get(arg))?;
                self.push_value(obj);
                Ok(None)
            }
            bytecode::Instruction::LoadFast(idx) => {
                #[cold]
                fn reference_error(
                    varname: &'static PyStrInterned,
                    vm: &VirtualMachine,
                ) -> PyBaseExceptionRef {
                    vm.new_exception_msg(
                        vm.ctx.exceptions.unbound_local_error.to_owned(),
                        format!("local variable '{varname}' referenced before assignment",),
                    )
                }
                let idx = idx.get(arg) as usize;
                let x = self.fastlocals.lock()[idx]
                    .clone()
                    .ok_or_else(|| reference_error(self.code.varnames[idx], vm))?;
                self.push_value(x);
                Ok(None)
            }
            bytecode::Instruction::LoadNameAny(idx) => {
                let name = self.code.names[idx.get(arg) as usize];
                let result = self.locals.mapping().subscript(name, vm);
                match result {
                    Ok(x) => self.push_value(x),
                    Err(e) if e.fast_isinstance(vm.ctx.exceptions.key_error) => {
                        self.push_value(self.load_global_or_builtin(name, vm)?);
                    }
                    Err(e) => return Err(e),
                }
                Ok(None)
            }
            bytecode::Instruction::LoadGlobal(idx) => {
                let name = &self.code.names[idx.get(arg) as usize];
                let x = self.load_global_or_builtin(name, vm)?;
                self.push_value(x);
                Ok(None)
            }
            bytecode::Instruction::LoadDeref(i) => {
                let i = i.get(arg) as usize;
                let x = self.cells_frees[i]
                    .get()
                    .ok_or_else(|| self.unbound_cell_exception(i, vm))?;
                self.push_value(x);
                Ok(None)
            }
            bytecode::Instruction::LoadClassDeref(i) => {
                let i = i.get(arg) as usize;
                let name = if i < self.code.cellvars.len() {
                    self.code.cellvars[i]
                } else {
                    self.code.freevars[i - self.code.cellvars.len()]
                };
                let value = self.locals.mapping().subscript(name, vm).ok();
                self.push_value(match value {
                    Some(v) => v,
                    None => self.cells_frees[i]
                        .get()
                        .ok_or_else(|| self.unbound_cell_exception(i, vm))?,
                });
                Ok(None)
            }
            bytecode::Instruction::StoreFast(idx) => {
                let value = self.pop_value();
                self.fastlocals.lock()[idx.get(arg) as usize] = Some(value);
                Ok(None)
            }
            bytecode::Instruction::StoreLocal(idx) => {
                let name = self.code.names[idx.get(arg) as usize];
                let value = self.pop_value();
                self.locals.mapping().ass_subscript(name, Some(value), vm)?;
                Ok(None)
            }
            bytecode::Instruction::StoreGlobal(idx) => {
                let value = self.pop_value();
                self.globals
                    .set_item(self.code.names[idx.get(arg) as usize], value, vm)?;
                Ok(None)
            }
            bytecode::Instruction::StoreDeref(i) => {
                let value = self.pop_value();
                self.cells_frees[i.get(arg) as usize].set(Some(value));
                Ok(None)
            }
            bytecode::Instruction::DeleteFast(idx) => {
                let mut fastlocals = self.fastlocals.lock();
                let idx = idx.get(arg) as usize;
                if fastlocals[idx].is_none() {
                    return Err(vm.new_exception_msg(
                        vm.ctx.exceptions.unbound_local_error.to_owned(),
                        format!(
                            "local variable '{}' referenced before assignment",
                            self.code.varnames[idx]
                        ),
                    ));
                }
                fastlocals[idx] = None;
                Ok(None)
            }
            bytecode::Instruction::DeleteLocal(idx) => {
                let name = self.code.names[idx.get(arg) as usize];
                let res = self.locals.mapping().ass_subscript(name, None, vm);

                match res {
                    Ok(()) => {}
                    Err(e) if e.fast_isinstance(vm.ctx.exceptions.key_error) => {
                        return Err(name_error(name, vm));
                    }
                    Err(e) => return Err(e),
                }
                Ok(None)
            }
            bytecode::Instruction::DeleteGlobal(idx) => {
                let name = self.code.names[idx.get(arg) as usize];
                match self.globals.del_item(name, vm) {
                    Ok(()) => {}
                    Err(e) if e.fast_isinstance(vm.ctx.exceptions.key_error) => {
                        return Err(name_error(name, vm));
                    }
                    Err(e) => return Err(e),
                }
                Ok(None)
            }
            bytecode::Instruction::DeleteDeref(i) => {
                self.cells_frees[i.get(arg) as usize].set(None);
                Ok(None)
            }
            bytecode::Instruction::LoadClosure(i) => {
                let value = self.cells_frees[i.get(arg) as usize].clone();
                self.push_value(value.into());
                Ok(None)
            }
            bytecode::Instruction::Subscript => self.execute_subscript(vm),
            bytecode::Instruction::StoreSubscript => self.execute_store_subscript(vm),
            bytecode::Instruction::DeleteSubscript => self.execute_delete_subscript(vm),
            bytecode::Instruction::CopyItem { index } => {
                let value = self
                    .state
                    .stack
                    .len()
                    .checked_sub(index.get(arg) as usize)
                    .map(|i| &self.state.stack[i])
                    .unwrap();
                self.push_value(value.clone());
                Ok(None)
            }
            bytecode::Instruction::Pop => {
                // Pop value from stack and ignore.
                self.pop_value();
                Ok(None)
            }
            bytecode::Instruction::Swap { index } => {
                let len = self.state.stack.len();
                let i = len - 1; // TOS index
                let index_val = index.get(arg) as usize;
                // SWAP(i) swaps TOS with element i positions down from TOS
                // So the target index is len - index_val
                let j = len - index_val;
                self.state.stack.swap(i, j);
                Ok(None)
            }
            // bytecode::Instruction::ToBool => {
            //     dbg!("Shouldn't be called outside of match statements for now")
            //     let value = self.pop_value();
            //     // call __bool__
            //     let result = value.try_to_bool(vm)?;
            //     self.push_value(vm.ctx.new_bool(result).into());
            //     Ok(None)
            // }
            bytecode::Instruction::Duplicate => {
                // Duplicate top of stack
                let value = self.top_value();
                self.push_value(value.to_owned());
                Ok(None)
            }
            bytecode::Instruction::Duplicate2 => {
                // Duplicate top 2 of stack
                let len = self.state.stack.len();
                self.push_value(self.state.stack[len - 2].clone());
                self.push_value(self.state.stack[len - 1].clone());
                Ok(None)
            }
            // splitting the instructions like this offloads the cost of "dynamic" dispatch (on the
            // amount to rotate) to the opcode dispatcher, and generates optimized code for the
            // concrete cases we actually have
            bytecode::Instruction::Rotate2 => self.execute_rotate(2),
            bytecode::Instruction::Rotate3 => self.execute_rotate(3),
            bytecode::Instruction::BuildString { size } => {
                let s = self
                    .pop_multiple(size.get(arg) as usize)
                    .as_slice()
                    .iter()
                    .map(|pyobj| pyobj.downcast_ref::<PyStr>().unwrap())
                    .collect::<Wtf8Buf>();
                let str_obj = vm.ctx.new_str(s);
                self.push_value(str_obj.into());
                Ok(None)
            }
            bytecode::Instruction::BuildList { size } => {
                let elements = self.pop_multiple(size.get(arg) as usize).collect();
                let list_obj = vm.ctx.new_list(elements);
                self.push_value(list_obj.into());
                Ok(None)
            }
            bytecode::Instruction::BuildListFromTuples { size } => {
                // SAFETY: compiler guarantees `size` tuples are on the stack
                let elements = unsafe { self.flatten_tuples(size.get(arg) as usize) };
                let list_obj = vm.ctx.new_list(elements);
                self.push_value(list_obj.into());
                Ok(None)
            }
            bytecode::Instruction::BuildSet { size } => {
                let set = PySet::default().into_ref(&vm.ctx);
                for element in self.pop_multiple(size.get(arg) as usize) {
                    set.add(element, vm)?;
                }
                self.push_value(set.into());
                Ok(None)
            }
            bytecode::Instruction::BuildSetFromTuples { size } => {
                let set = PySet::default().into_ref(&vm.ctx);
                for element in self.pop_multiple(size.get(arg) as usize) {
                    // SAFETY: trust compiler
                    let tup = unsafe { element.downcast_unchecked::<PyTuple>() };
                    for item in tup.iter() {
                        set.add(item.clone(), vm)?;
                    }
                }
                self.push_value(set.into());
                Ok(None)
            }
            bytecode::Instruction::BuildTuple { size } => {
                let elements = self.pop_multiple(size.get(arg) as usize).collect();
                let list_obj = vm.ctx.new_tuple(elements);
                self.push_value(list_obj.into());
                Ok(None)
            }
            bytecode::Instruction::BuildTupleFromTuples { size } => {
                // SAFETY: compiler guarantees `size` tuples are on the stack
                let elements = unsafe { self.flatten_tuples(size.get(arg) as usize) };
                let list_obj = vm.ctx.new_tuple(elements);
                self.push_value(list_obj.into());
                Ok(None)
            }
            bytecode::Instruction::BuildTupleFromIter => {
                if !self.top_value().class().is(vm.ctx.types.tuple_type) {
                    let elements: Vec<_> = self.pop_value().try_to_value(vm)?;
                    let list_obj = vm.ctx.new_tuple(elements);
                    self.push_value(list_obj.into());
                }
                Ok(None)
            }
            bytecode::Instruction::BuildMap { size } => self.execute_build_map(vm, size.get(arg)),
            bytecode::Instruction::BuildMapForCall { size } => {
                self.execute_build_map_for_call(vm, size.get(arg))
            }
            bytecode::Instruction::DictUpdate { index } => {
                // Stack before: [..., dict, ..., source]  (source at TOS)
                // Stack after:  [..., dict, ...]  (source consumed)
                // The dict to update is at position TOS-i (before popping source)

                let idx = index.get(arg);

                // Pop the source from TOS
                let source = self.pop_value();

                // Get the dict to update (it's now at TOS-(i-1) after popping source)
                let dict = if idx <= 1 {
                    // DICT_UPDATE 0 or 1: dict is at TOS (after popping source)
                    self.top_value()
                } else {
                    // DICT_UPDATE n: dict is at TOS-(n-1)
                    self.nth_value(idx - 1)
                };

                let dict = dict.downcast_ref::<PyDict>().expect("exact dict expected");

                // For dictionary unpacking {**x}, x must be a mapping
                // Check if the object has the mapping protocol (keys method)
                if vm
                    .get_method(source.clone(), vm.ctx.intern_str("keys"))
                    .is_none()
                {
                    return Err(vm.new_type_error(format!(
                        "'{}' object is not a mapping",
                        source.class().name()
                    )));
                }

                dict.merge_object(source, vm)?;
                Ok(None)
            }
            bytecode::Instruction::BuildSlice { step } => {
                self.execute_build_slice(vm, step.get(arg))
            }
            bytecode::Instruction::ListAppend { i } => {
                let item = self.pop_value();
                let obj = self.nth_value(i.get(arg));
                let list: &Py<PyList> = unsafe {
                    // SAFETY: trust compiler
                    obj.downcast_unchecked_ref()
                };
                list.append(item);
                Ok(None)
            }
            bytecode::Instruction::SetAdd { i } => {
                let item = self.pop_value();
                let obj = self.nth_value(i.get(arg));
                let set: &Py<PySet> = unsafe {
                    // SAFETY: trust compiler
                    obj.downcast_unchecked_ref()
                };
                set.add(item, vm)?;
                Ok(None)
            }
            bytecode::Instruction::MapAdd { i } => {
                let value = self.pop_value();
                let key = self.pop_value();
                let obj = self.nth_value(i.get(arg));
                let dict: &Py<PyDict> = unsafe {
                    // SAFETY: trust compiler
                    obj.downcast_unchecked_ref()
                };
                dict.set_item(&*key, value, vm)?;
                Ok(None)
            }
            bytecode::Instruction::BinaryOperation { op } => self.execute_bin_op(vm, op.get(arg)),
            bytecode::Instruction::BinaryOperationInplace { op } => {
                self.execute_bin_op_inplace(vm, op.get(arg))
            }
            bytecode::Instruction::BinarySubscript => {
                let key = self.pop_value();
                let container = self.pop_value();
                self.state
                    .stack
                    .push(container.get_item(key.as_object(), vm)?);
                Ok(None)
            }
            bytecode::Instruction::LoadAttr { idx } => self.load_attr(vm, idx.get(arg)),
            bytecode::Instruction::StoreAttr { idx } => self.store_attr(vm, idx.get(arg)),
            bytecode::Instruction::DeleteAttr { idx } => self.delete_attr(vm, idx.get(arg)),
            bytecode::Instruction::UnaryOperation { op } => self.execute_unary_op(vm, op.get(arg)),
            bytecode::Instruction::TestOperation { op } => self.execute_test(vm, op.get(arg)),
            bytecode::Instruction::CompareOperation { op } => self.execute_compare(vm, op.get(arg)),
            bytecode::Instruction::ReturnValue => {
                let value = self.pop_value();
                self.unwind_blocks(vm, UnwindReason::Returning { value })
            }
            bytecode::Instruction::ReturnConst { idx } => {
                let value = self.code.constants[idx.get(arg) as usize].clone().into();
                self.unwind_blocks(vm, UnwindReason::Returning { value })
            }
            bytecode::Instruction::YieldValue => {
                let value = self.pop_value();
                let value = if self.code.flags.contains(bytecode::CodeFlags::IS_COROUTINE) {
                    PyAsyncGenWrappedValue(value).into_pyobject(vm)
                } else {
                    value
                };
                Ok(Some(ExecutionResult::Yield(value)))
            }
            bytecode::Instruction::YieldFrom => self.execute_yield_from(vm),
            bytecode::Instruction::Resume { arg: resume_arg } => {
                // Resume execution after yield, await, or at function start
                // In CPython, this checks instrumentation and eval breaker
                // For now, we just check for signals/interrupts
                let _resume_type = resume_arg.get(arg);

                // Check for interrupts if not resuming from yield_from
                // if resume_type < bytecode::ResumeType::AfterYieldFrom as u32 {
                //     vm.check_signals()?;
                // }
                Ok(None)
            }
            bytecode::Instruction::SetupAnnotation => self.setup_annotations(vm),
            bytecode::Instruction::SetupLoop => {
                self.push_block(BlockType::Loop);
                Ok(None)
            }
            bytecode::Instruction::SetupExcept { handler } => {
                self.push_block(BlockType::TryExcept {
                    handler: handler.get(arg),
                });
                Ok(None)
            }
            bytecode::Instruction::SetupFinally { handler } => {
                self.push_block(BlockType::Finally {
                    handler: handler.get(arg),
                });
                Ok(None)
            }
            bytecode::Instruction::EnterFinally => {
                self.push_block(BlockType::FinallyHandler {
                    reason: None,
                    prev_exc: vm.current_exception(),
                });
                Ok(None)
            }
            bytecode::Instruction::EndFinally => {
                // Pop the finally handler from the stack, and recall
                // what was the reason we were in this finally clause.
                let block = self.pop_block();

                if let BlockType::FinallyHandler { reason, prev_exc } = block.typ {
                    vm.set_exception(prev_exc);
                    if let Some(reason) = reason {
                        self.unwind_blocks(vm, reason)
                    } else {
                        Ok(None)
                    }
                } else {
                    self.fatal(
                        "Block type must be finally handler when reaching EndFinally instruction!",
                    );
                }
            }
            bytecode::Instruction::SetupWith { end } => {
                let context_manager = self.pop_value();
                let error_string = || -> String {
                    format!(
                        "'{:.200}' object does not support the context manager protocol",
                        context_manager.class().name(),
                    )
                };
                let enter_res = vm
                    .get_special_method(&context_manager, identifier!(vm, __enter__))?
                    .ok_or_else(|| vm.new_type_error(error_string()))?
                    .invoke((), vm)?;

                let exit = context_manager
                    .get_attr(identifier!(vm, __exit__), vm)
                    .map_err(|_exc| {
                        vm.new_type_error({
                            format!("{} (missed __exit__ method)", error_string())
                        })
                    })?;
                self.push_value(exit);
                self.push_block(BlockType::Finally {
                    handler: end.get(arg),
                });
                self.push_value(enter_res);
                Ok(None)
            }
            bytecode::Instruction::BeforeAsyncWith => {
                let mgr = self.pop_value();
                let error_string = || -> String {
                    format!(
                        "'{:.200}' object does not support the asynchronous context manager protocol",
                        mgr.class().name(),
                    )
                };

                let aenter_res = vm
                    .get_special_method(&mgr, identifier!(vm, __aenter__))?
                    .ok_or_else(|| vm.new_type_error(error_string()))?
                    .invoke((), vm)?;
                let aexit = mgr
                    .get_attr(identifier!(vm, __aexit__), vm)
                    .map_err(|_exc| {
                        vm.new_type_error({
                            format!("{} (missed __aexit__ method)", error_string())
                        })
                    })?;
                self.push_value(aexit);
                self.push_value(aenter_res);

                Ok(None)
            }
            bytecode::Instruction::SetupAsyncWith { end } => {
                let enter_res = self.pop_value();
                self.push_block(BlockType::Finally {
                    handler: end.get(arg),
                });
                self.push_value(enter_res);
                Ok(None)
            }
            bytecode::Instruction::WithCleanupStart => {
                let block = self.current_block().unwrap();
                let reason = match block.typ {
                    BlockType::FinallyHandler { reason, .. } => reason,
                    _ => self.fatal("WithCleanupStart expects a FinallyHandler block on stack"),
                };
                let exc = match reason {
                    Some(UnwindReason::Raising { exception }) => Some(exception),
                    _ => None,
                };

                let exit = self.top_value();

                let args = if let Some(exc) = exc {
                    vm.split_exception(exc)
                } else {
                    (vm.ctx.none(), vm.ctx.none(), vm.ctx.none())
                };
                let exit_res = exit.call(args, vm)?;
                self.replace_top(exit_res);

                Ok(None)
            }
            bytecode::Instruction::WithCleanupFinish => {
                let block = self.pop_block();
                let (reason, prev_exc) = match block.typ {
                    BlockType::FinallyHandler { reason, prev_exc } => (reason, prev_exc),
                    _ => self.fatal("WithCleanupFinish expects a FinallyHandler block on stack"),
                };

                vm.set_exception(prev_exc);

                let suppress_exception = self.pop_value().try_to_bool(vm)?;

                if suppress_exception {
                    Ok(None)
                } else if let Some(reason) = reason {
                    self.unwind_blocks(vm, reason)
                } else {
                    Ok(None)
                }
            }
            bytecode::Instruction::PopBlock => {
                self.pop_block();
                Ok(None)
            }
            bytecode::Instruction::GetIter => {
                let iterated_obj = self.pop_value();
                let iter_obj = iterated_obj.get_iter(vm)?;
                self.push_value(iter_obj.into());
                Ok(None)
            }
            bytecode::Instruction::GetLen => {
                // STACK.append(len(STACK[-1]))
                let obj = self.top_value();
                let len = obj.length(vm)?;
                self.push_value(vm.ctx.new_int(len).into());
                Ok(None)
            }
            bytecode::Instruction::CallIntrinsic1 { func } => {
                let value = self.pop_value();
                let result = self.call_intrinsic_1(func.get(arg), value, vm)?;
                self.push_value(result);
                Ok(None)
            }
            bytecode::Instruction::CallIntrinsic2 { func } => {
                let value2 = self.pop_value();
                let value1 = self.pop_value();
                let result = self.call_intrinsic_2(func.get(arg), value1, value2, vm)?;
                self.push_value(result);
                Ok(None)
            }
            bytecode::Instruction::GetAwaitable => {
                let awaited_obj = self.pop_value();
                let awaitable = if awaited_obj.downcastable::<PyCoroutine>() {
                    awaited_obj
                } else {
                    let await_method = vm.get_method_or_type_error(
                        awaited_obj.clone(),
                        identifier!(vm, __await__),
                        || {
                            format!(
                                "object {} can't be used in 'await' expression",
                                awaited_obj.class().name(),
                            )
                        },
                    )?;
                    await_method.call((), vm)?
                };
                self.push_value(awaitable);
                Ok(None)
            }
            bytecode::Instruction::GetAIter => {
                let aiterable = self.pop_value();
                let aiter = vm.call_special_method(&aiterable, identifier!(vm, __aiter__), ())?;
                self.push_value(aiter);
                Ok(None)
            }
            bytecode::Instruction::GetANext => {
                #[cfg(debug_assertions)] // remove when GetANext is fully implemented
                let orig_stack_len = self.state.stack.len();

                let aiter = self.top_value();
                let awaitable = if aiter.class().is(vm.ctx.types.async_generator) {
                    vm.call_special_method(aiter, identifier!(vm, __anext__), ())?
                } else {
                    if !aiter.has_attr("__anext__", vm).unwrap_or(false) {
                        // TODO: __anext__ must be protocol
                        let msg = format!(
                            "'async for' requires an iterator with __anext__ method, got {:.100}",
                            aiter.class().name()
                        );
                        return Err(vm.new_type_error(msg));
                    }
                    let next_iter =
                        vm.call_special_method(aiter, identifier!(vm, __anext__), ())?;

                    // _PyCoro_GetAwaitableIter in CPython
                    fn get_awaitable_iter(next_iter: &PyObject, vm: &VirtualMachine) -> PyResult {
                        let gen_is_coroutine = |_| {
                            // TODO: cpython gen_is_coroutine
                            true
                        };
                        if next_iter.class().is(vm.ctx.types.coroutine_type)
                            || gen_is_coroutine(next_iter)
                        {
                            return Ok(next_iter.to_owned());
                        }
                        // TODO: error handling
                        vm.call_special_method(next_iter, identifier!(vm, __await__), ())
                    }
                    get_awaitable_iter(&next_iter, vm).map_err(|_| {
                        vm.new_type_error(format!(
                            "'async for' received an invalid object from __anext__: {:.200}",
                            next_iter.class().name()
                        ))
                    })?
                };
                self.push_value(awaitable);
                #[cfg(debug_assertions)]
                debug_assert_eq!(orig_stack_len + 1, self.state.stack.len());
                Ok(None)
            }
            bytecode::Instruction::EndAsyncFor => {
                let exc = self.pop_value();
                let except_block = self.pop_block(); // pushed by TryExcept unwind
                debug_assert_eq!(except_block.level, self.state.stack.len());
                let _async_iterator = self.pop_value(); // __anext__ provider in the loop
                if exc.fast_isinstance(vm.ctx.exceptions.stop_async_iteration) {
                    vm.take_exception().expect("Should have exception in stack");
                    Ok(None)
                } else {
                    Err(exc.downcast().unwrap())
                }
            }
            bytecode::Instruction::ForIter { target } => self.execute_for_iter(vm, target.get(arg)),
            bytecode::Instruction::MakeFunction => self.execute_make_function(vm),
            bytecode::Instruction::SetFunctionAttribute { attr } => {
                self.execute_set_function_attribute(vm, attr.get(arg))
            }
            bytecode::Instruction::CallFunctionPositional { nargs } => {
                let args = self.collect_positional_args(nargs.get(arg));
                self.execute_call(args, vm)
            }
            bytecode::Instruction::CallFunctionKeyword { nargs } => {
                let args = self.collect_keyword_args(nargs.get(arg));
                self.execute_call(args, vm)
            }
            bytecode::Instruction::CallFunctionEx { has_kwargs } => {
                let args = self.collect_ex_args(vm, has_kwargs.get(arg))?;
                self.execute_call(args, vm)
            }
            bytecode::Instruction::LoadMethod { idx } => {
                let obj = self.pop_value();
                let method_name = self.code.names[idx.get(arg) as usize];
                let method = PyMethod::get(obj, method_name, vm)?;
                let (target, is_method, func) = match method {
                    PyMethod::Function { target, func } => (target, true, func),
                    PyMethod::Attribute(val) => (vm.ctx.none(), false, val),
                };
                // TODO: figure out a better way to communicate PyMethod::Attribute - CPython uses
                // target==NULL, maybe we could use a sentinel value or something?
                self.push_value(target);
                self.push_value(vm.ctx.new_bool(is_method).into());
                self.push_value(func);
                Ok(None)
            }
            bytecode::Instruction::CallMethodPositional { nargs } => {
                let args = self.collect_positional_args(nargs.get(arg));
                self.execute_method_call(args, vm)
            }
            bytecode::Instruction::CallMethodKeyword { nargs } => {
                let args = self.collect_keyword_args(nargs.get(arg));
                self.execute_method_call(args, vm)
            }
            bytecode::Instruction::CallMethodEx { has_kwargs } => {
                let args = self.collect_ex_args(vm, has_kwargs.get(arg))?;
                self.execute_method_call(args, vm)
            }
            bytecode::Instruction::Jump { target } => {
                self.jump(target.get(arg));
                Ok(None)
            }
            bytecode::Instruction::JumpIfTrue { target } => self.jump_if(vm, target.get(arg), true),
            bytecode::Instruction::JumpIfFalse { target } => {
                self.jump_if(vm, target.get(arg), false)
            }
            bytecode::Instruction::JumpIfTrueOrPop { target } => {
                self.jump_if_or_pop(vm, target.get(arg), true)
            }
            bytecode::Instruction::JumpIfFalseOrPop { target } => {
                self.jump_if_or_pop(vm, target.get(arg), false)
            }

            bytecode::Instruction::Raise { kind } => self.execute_raise(vm, kind.get(arg)),

            bytecode::Instruction::Break { target } => self.unwind_blocks(
                vm,
                UnwindReason::Break {
                    target: target.get(arg),
                },
            ),
            bytecode::Instruction::Continue { target } => self.unwind_blocks(
                vm,
                UnwindReason::Continue {
                    target: target.get(arg),
                },
            ),
            bytecode::Instruction::PrintExpr => self.print_expr(vm),
            bytecode::Instruction::LoadBuildClass => {
                self.push_value(vm.builtins.get_attr(identifier!(vm, __build_class__), vm)?);
                Ok(None)
            }
            bytecode::Instruction::UnpackSequence { size } => {
                self.unpack_sequence(size.get(arg), vm)
            }
            bytecode::Instruction::UnpackEx { args } => {
                let args = args.get(arg);
                self.execute_unpack_ex(vm, args.before, args.after)
            }
            bytecode::Instruction::FormatValue { conversion } => {
                self.format_value(conversion.get(arg), vm)
            }
            bytecode::Instruction::PopException => {
                let block = self.pop_block();
                if let BlockType::ExceptHandler { prev_exc } = block.typ {
                    vm.set_exception(prev_exc);
                    Ok(None)
                } else {
                    self.fatal("block type must be ExceptHandler here.")
                }
            }
            bytecode::Instruction::Reverse { amount } => {
                let stack_len = self.state.stack.len();
                self.state.stack[stack_len - amount.get(arg) as usize..stack_len].reverse();
                Ok(None)
            }
            bytecode::Instruction::ExtendedArg => {
                *extend_arg = true;
                Ok(None)
            }
            bytecode::Instruction::MatchMapping => {
                // Pop the subject from stack
                let subject = self.pop_value();

                // Decide if it's a mapping, push True/False or handle error
                let is_mapping = PyMapping::check(&subject);
                self.push_value(vm.ctx.new_bool(is_mapping).into());
                Ok(None)
            }
            bytecode::Instruction::MatchSequence => {
                // Pop the subject from stack
                let subject = self.pop_value();

                // Decide if it's a sequence (but not a mapping)
                let is_sequence = subject.to_sequence().check();
                self.push_value(vm.ctx.new_bool(is_sequence).into());
                Ok(None)
            }
            bytecode::Instruction::MatchKeys => {
                // Typically we pop a sequence of keys first
                let _keys = self.pop_value();
                let subject = self.pop_value();

                // Check if subject is a dict (or mapping) and all keys match
                if let Ok(_dict) = subject.downcast::<PyDict>() {
                    // Example: gather the values corresponding to keys
                    // If keys match, push the matched values & success
                    self.push_value(vm.ctx.new_bool(true).into());
                } else {
                    // Push a placeholder to indicate no match
                    self.push_value(vm.ctx.new_bool(false).into());
                }
                Ok(None)
            }
            bytecode::Instruction::MatchClass(_arg) => {
                // STACK[-1] is a tuple of keyword attribute names, STACK[-2] is the class being matched against, and STACK[-3] is the match subject.
                // count is the number of positional sub-patterns.
                // Pop STACK[-1], STACK[-2], and STACK[-3].
                let names = self.pop_value();
                let names = names.downcast_ref::<PyTuple>().unwrap();
                let cls = self.pop_value();
                let subject = self.pop_value();
                // If STACK[-3] is an instance of STACK[-2] and has the positional and keyword attributes required by count and STACK[-1],
                // push a tuple of extracted attributes.
                if subject.is_instance(cls.as_ref(), vm)? {
                    let mut extracted = vec![];
                    for name in names {
                        let name_str = name.downcast_ref::<PyStr>().unwrap();
                        let value = subject.get_attr(name_str, vm)?;
                        extracted.push(value);
                    }
                    self.push_value(vm.ctx.new_tuple(extracted).into());
                } else {
                    // Otherwise, push None.
                    self.push_value(vm.ctx.none());
                }
                Ok(None)
            }
        }
    }

    #[inline]
    fn load_global_or_builtin(&self, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
        self.globals
            .get_chain(self.builtins, name, vm)?
            .ok_or_else(|| {
                vm.new_name_error(format!("name '{name}' is not defined"), name.to_owned())
            })
    }

    unsafe fn flatten_tuples(&mut self, size: usize) -> Vec<PyObjectRef> {
        let mut elements = Vec::new();
        for tup in self.pop_multiple(size) {
            // SAFETY: caller ensures that the elements are tuples
            let tup = unsafe { tup.downcast_unchecked::<PyTuple>() };
            elements.extend(tup.iter().cloned());
        }
        elements
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn import(&mut self, vm: &VirtualMachine, module_name: Option<&Py<PyStr>>) -> PyResult<()> {
        let module_name = module_name.unwrap_or(vm.ctx.empty_str);
        let top = self.pop_value();
        let from_list = match <Option<PyTupleRef>>::try_from_object(vm, top)? {
            Some(from_list) => from_list.try_into_typed::<PyStr>(vm)?,
            None => vm.ctx.empty_tuple_typed().to_owned(),
        };
        let level = usize::try_from_object(vm, self.pop_value())?;

        let module = vm.import_from(module_name, &from_list, level)?;

        self.push_value(module);
        Ok(())
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn import_from(&mut self, vm: &VirtualMachine, idx: bytecode::NameIdx) -> PyResult {
        let module = self.top_value();
        let name = self.code.names[idx as usize];

        // Load attribute, and transform any error into import error.
        if let Some(obj) = vm.get_attribute_opt(module.to_owned(), name)? {
            return Ok(obj);
        }
        // fallback to importing '{module.__name__}.{name}' from sys.modules
        let fallback_module = (|| {
            let mod_name = module.get_attr(identifier!(vm, __name__), vm).ok()?;
            let mod_name = mod_name.downcast_ref::<PyStr>()?;
            let full_mod_name = format!("{mod_name}.{name}");
            let sys_modules = vm.sys_module.get_attr("modules", vm).ok()?;
            sys_modules.get_item(&full_mod_name, vm).ok()
        })();

        if let Some(sub_module) = fallback_module {
            return Ok(sub_module);
        }

        if is_module_initializing(module, vm) {
            let module_name = module
                .get_attr(identifier!(vm, __name__), vm)
                .ok()
                .and_then(|n| n.downcast_ref::<PyStr>().map(|s| s.as_str().to_owned()))
                .unwrap_or_else(|| "<unknown>".to_owned());

            let msg = format!(
                "cannot import name '{name}' from partially initialized module '{module_name}' (most likely due to a circular import)",
            );
            Err(vm.new_import_error(msg, name.to_owned()))
        } else {
            Err(vm.new_import_error(format!("cannot import name '{name}'"), name.to_owned()))
        }
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn import_star(&mut self, vm: &VirtualMachine) -> PyResult<()> {
        let module = self.pop_value();

        // Grab all the names from the module and put them in the context
        if let Some(dict) = module.dict() {
            let filter_pred: Box<dyn Fn(&str) -> bool> =
                if let Ok(all) = dict.get_item(identifier!(vm, __all__), vm) {
                    let all: Vec<PyStrRef> = all.try_to_value(vm)?;
                    let all: Vec<String> = all
                        .into_iter()
                        .map(|name| name.as_str().to_owned())
                        .collect();
                    Box::new(move |name| all.contains(&name.to_owned()))
                } else {
                    Box::new(|name| !name.starts_with('_'))
                };
            for (k, v) in dict {
                let k = PyStrRef::try_from_object(vm, k)?;
                if filter_pred(k.as_str()) {
                    self.locals.mapping().ass_subscript(&k, Some(v), vm)?;
                }
            }
        }
        Ok(())
    }

    /// Unwind blocks.
    /// The reason for unwinding gives a hint on what to do when
    /// unwinding a block.
    /// Optionally returns an exception.
    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn unwind_blocks(&mut self, vm: &VirtualMachine, reason: UnwindReason) -> FrameResult {
        // First unwind all existing blocks on the block stack:
        while let Some(block) = self.current_block() {
            // eprintln!("unwinding block: {:.60?} {:.60?}", block.typ, reason);
            match block.typ {
                BlockType::Loop => match reason {
                    UnwindReason::Break { target } => {
                        self.pop_block();
                        self.jump(target);
                        return Ok(None);
                    }
                    UnwindReason::Continue { target } => {
                        self.jump(target);
                        return Ok(None);
                    }
                    _ => {
                        self.pop_block();
                    }
                },
                BlockType::Finally { handler } => {
                    self.pop_block();
                    let prev_exc = vm.current_exception();
                    if let UnwindReason::Raising { exception } = &reason {
                        vm.set_exception(Some(exception.clone()));
                    }
                    self.push_block(BlockType::FinallyHandler {
                        reason: Some(reason),
                        prev_exc,
                    });
                    self.jump(handler);
                    return Ok(None);
                }
                BlockType::TryExcept { handler } => {
                    self.pop_block();
                    if let UnwindReason::Raising { exception } = reason {
                        self.push_block(BlockType::ExceptHandler {
                            prev_exc: vm.current_exception(),
                        });
                        vm.contextualize_exception(&exception);
                        vm.set_exception(Some(exception.clone()));
                        self.push_value(exception.into());
                        self.jump(handler);
                        return Ok(None);
                    }
                }
                BlockType::FinallyHandler { prev_exc, .. }
                | BlockType::ExceptHandler { prev_exc } => {
                    self.pop_block();
                    vm.set_exception(prev_exc);
                }
            }
        }

        // We do not have any more blocks to unwind. Inspect the reason we are here:
        match reason {
            UnwindReason::Raising { exception } => Err(exception),
            UnwindReason::Returning { value } => Ok(Some(ExecutionResult::Return(value))),
            UnwindReason::Break { .. } | UnwindReason::Continue { .. } => {
                self.fatal("break or continue must occur within a loop block.")
            } // UnwindReason::NoWorries => Ok(None),
        }
    }

    #[inline(always)]
    fn execute_rotate(&mut self, amount: usize) -> FrameResult {
        let i = self.state.stack.len() - amount;
        self.state.stack[i..].rotate_right(1);
        Ok(None)
    }

    fn execute_subscript(&mut self, vm: &VirtualMachine) -> FrameResult {
        let b_ref = self.pop_value();
        let a_ref = self.pop_value();
        let value = a_ref.get_item(&*b_ref, vm)?;
        self.push_value(value);
        Ok(None)
    }

    fn execute_store_subscript(&mut self, vm: &VirtualMachine) -> FrameResult {
        let idx = self.pop_value();
        let obj = self.pop_value();
        let value = self.pop_value();
        obj.set_item(&*idx, value, vm)?;
        Ok(None)
    }

    fn execute_delete_subscript(&mut self, vm: &VirtualMachine) -> FrameResult {
        let idx = self.pop_value();
        let obj = self.pop_value();
        obj.del_item(&*idx, vm)?;
        Ok(None)
    }

    fn execute_build_map(&mut self, vm: &VirtualMachine, size: u32) -> FrameResult {
        let size = size as usize;
        let map_obj = vm.ctx.new_dict();
        for (key, value) in self.pop_multiple(2 * size).tuples() {
            map_obj.set_item(&*key, value, vm)?;
        }

        self.push_value(map_obj.into());
        Ok(None)
    }

    fn execute_build_map_for_call(&mut self, vm: &VirtualMachine, size: u32) -> FrameResult {
        let size = size as usize;
        let map_obj = vm.ctx.new_dict();
        for obj in self.pop_multiple(size) {
            // Use keys() method for all mapping objects to preserve order
            Self::iterate_mapping_keys(vm, &obj, "keyword argument", |key| {
                // Check for keyword argument restrictions
                if key.downcast_ref::<PyStr>().is_none() {
                    return Err(vm.new_type_error("keywords must be strings"));
                }
                if map_obj.contains_key(&*key, vm) {
                    let key_repr = &key.repr(vm)?;
                    let msg = format!(
                        "got multiple values for keyword argument {}",
                        key_repr.as_str()
                    );
                    return Err(vm.new_type_error(msg));
                }

                let value = obj.get_item(&*key, vm)?;
                map_obj.set_item(&*key, value, vm)?;
                Ok(())
            })?;
        }

        self.push_value(map_obj.into());
        Ok(None)
    }

    fn execute_build_slice(&mut self, vm: &VirtualMachine, step: bool) -> FrameResult {
        let step = if step { Some(self.pop_value()) } else { None };
        let stop = self.pop_value();
        let start = self.pop_value();

        let obj = PySlice {
            start: Some(start),
            stop,
            step,
        }
        .into_ref(&vm.ctx);
        self.push_value(obj.into());
        Ok(None)
    }

    fn collect_positional_args(&mut self, nargs: u32) -> FuncArgs {
        FuncArgs {
            args: self.pop_multiple(nargs as usize).collect(),
            kwargs: IndexMap::new(),
        }
    }

    fn collect_keyword_args(&mut self, nargs: u32) -> FuncArgs {
        let kwarg_names = self
            .pop_value()
            .downcast::<PyTuple>()
            .expect("kwarg names should be tuple of strings");
        let args = self.pop_multiple(nargs as usize);

        let kwarg_names = kwarg_names
            .as_slice()
            .iter()
            .map(|pyobj| pyobj.downcast_ref::<PyStr>().unwrap().as_str().to_owned());
        FuncArgs::with_kwargs_names(args, kwarg_names)
    }

    fn collect_ex_args(&mut self, vm: &VirtualMachine, has_kwargs: bool) -> PyResult<FuncArgs> {
        let kwargs = if has_kwargs {
            let kw_obj = self.pop_value();
            let mut kwargs = IndexMap::new();

            // Use keys() method for all mapping objects to preserve order
            Self::iterate_mapping_keys(vm, &kw_obj, "argument after **", |key| {
                let key_str = key
                    .downcast_ref::<PyStr>()
                    .ok_or_else(|| vm.new_type_error("keywords must be strings"))?;
                let value = kw_obj.get_item(&*key, vm)?;
                kwargs.insert(key_str.as_str().to_owned(), value);
                Ok(())
            })?;
            kwargs
        } else {
            IndexMap::new()
        };
        // SAFETY: trust compiler
        let args = unsafe { self.pop_value().downcast_unchecked::<PyTuple>() }
            .as_slice()
            .to_vec();
        Ok(FuncArgs { args, kwargs })
    }

    /// Helper function to iterate over mapping keys using the keys() method.
    /// This ensures proper order preservation for OrderedDict and other custom mappings.
    fn iterate_mapping_keys<F>(
        vm: &VirtualMachine,
        mapping: &PyObjectRef,
        error_prefix: &str,
        mut key_handler: F,
    ) -> PyResult<()>
    where
        F: FnMut(PyObjectRef) -> PyResult<()>,
    {
        let Some(keys_method) = vm.get_method(mapping.clone(), vm.ctx.intern_str("keys")) else {
            return Err(vm.new_type_error(format!("{error_prefix} must be a mapping")));
        };

        let keys = keys_method?.call((), vm)?.get_iter(vm)?;
        while let PyIterReturn::Return(key) = keys.next(vm)? {
            key_handler(key)?;
        }
        Ok(())
    }

    #[inline]
    fn execute_call(&mut self, args: FuncArgs, vm: &VirtualMachine) -> FrameResult {
        let func_ref = self.pop_value();
        let value = func_ref.call(args, vm)?;
        self.push_value(value);
        Ok(None)
    }

    #[inline]
    fn execute_method_call(&mut self, args: FuncArgs, vm: &VirtualMachine) -> FrameResult {
        let func = self.pop_value();
        let is_method = self.pop_value().is(&vm.ctx.true_value);
        let target = self.pop_value();

        // TODO: It was PyMethod before #4873. Check if it's correct.
        let func = if is_method {
            if let Some(descr_get) = func.class().mro_find_map(|cls| cls.slots.descr_get.load()) {
                let cls = target.class().to_owned().into();
                descr_get(func, Some(target), Some(cls), vm)?
            } else {
                func
            }
        } else {
            drop(target); // should be None
            func
        };
        let value = func.call(args, vm)?;
        self.push_value(value);
        Ok(None)
    }

    fn execute_raise(&mut self, vm: &VirtualMachine, kind: bytecode::RaiseKind) -> FrameResult {
        let cause = match kind {
            bytecode::RaiseKind::RaiseCause => {
                let val = self.pop_value();
                Some(if vm.is_none(&val) {
                    // if the cause arg is none, we clear the cause
                    None
                } else {
                    // if the cause arg is an exception, we overwrite it
                    let ctor = ExceptionCtor::try_from_object(vm, val).map_err(|_| {
                        vm.new_type_error("exception causes must derive from BaseException")
                    })?;
                    Some(ctor.instantiate(vm)?)
                })
            }
            // if there's no cause arg, we keep the cause as is
            bytecode::RaiseKind::Raise | bytecode::RaiseKind::Reraise => None,
        };
        let exception = match kind {
            bytecode::RaiseKind::RaiseCause | bytecode::RaiseKind::Raise => {
                ExceptionCtor::try_from_object(vm, self.pop_value())?.instantiate(vm)?
            }
            bytecode::RaiseKind::Reraise => vm
                .topmost_exception()
                .ok_or_else(|| vm.new_runtime_error("No active exception to reraise"))?,
        };
        #[cfg(debug_assertions)]
        debug!("Exception raised: {exception:?} with cause: {cause:?}");
        if let Some(cause) = cause {
            exception.set___cause__(cause);
        }
        Err(exception)
    }

    fn builtin_coro<'a>(&self, coro: &'a PyObject) -> Option<&'a Coro> {
        match_class!(match coro {
            ref g @ PyGenerator => Some(g.as_coro()),
            ref c @ PyCoroutine => Some(c.as_coro()),
            _ => None,
        })
    }

    fn _send(
        &self,
        jen: &PyObject,
        val: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyIterReturn> {
        match self.builtin_coro(jen) {
            Some(coro) => coro.send(jen, val, vm),
            // FIXME: turn return type to PyResult<PyIterReturn> then ExecutionResult will be simplified
            None if vm.is_none(&val) => PyIter::new(jen).next(vm),
            None => {
                let meth = jen.get_attr("send", vm)?;
                PyIterReturn::from_pyresult(meth.call((val,), vm), vm)
            }
        }
    }

    fn execute_yield_from(&mut self, vm: &VirtualMachine) -> FrameResult {
        // Value send into iterator:
        let val = self.pop_value();
        let coro = self.top_value();
        let result = self._send(coro, val, vm)?;

        // PyIterReturn returned from e.g. gen.__next__() or gen.send()
        match result {
            PyIterReturn::Return(value) => {
                // Set back program counter:
                self.update_lasti(|i| *i -= 1);
                Ok(Some(ExecutionResult::Yield(value)))
            }
            PyIterReturn::StopIteration(value) => {
                let value = vm.unwrap_or_none(value);
                self.replace_top(value);
                Ok(None)
            }
        }
    }

    fn execute_unpack_ex(&mut self, vm: &VirtualMachine, before: u8, after: u8) -> FrameResult {
        let (before, after) = (before as usize, after as usize);
        let value = self.pop_value();
        let elements: Vec<_> = value.try_to_value(vm)?;
        let min_expected = before + after;

        let middle = elements.len().checked_sub(min_expected).ok_or_else(|| {
            vm.new_value_error(format!(
                "not enough values to unpack (expected at least {}, got {})",
                min_expected,
                elements.len()
            ))
        })?;

        let mut elements = elements;
        // Elements on stack from right-to-left:
        self.state
            .stack
            .extend(elements.drain(before + middle..).rev());

        let middle_elements = elements.drain(before..).collect();
        let t = vm.ctx.new_list(middle_elements);
        self.push_value(t.into());

        // Lastly the first reversed values:
        self.state.stack.extend(elements.into_iter().rev());

        Ok(None)
    }

    #[inline]
    fn jump(&mut self, label: bytecode::Label) {
        let target_pc = label.0;
        vm_trace!("jump from {:?} to {:?}", self.lasti(), target_pc);
        self.update_lasti(|i| *i = target_pc);
    }

    #[inline]
    fn jump_if(&mut self, vm: &VirtualMachine, target: bytecode::Label, flag: bool) -> FrameResult {
        let obj = self.pop_value();
        let value = obj.try_to_bool(vm)?;
        if value == flag {
            self.jump(target);
        }
        Ok(None)
    }

    #[inline]
    fn jump_if_or_pop(
        &mut self,
        vm: &VirtualMachine,
        target: bytecode::Label,
        flag: bool,
    ) -> FrameResult {
        let obj = self.top_value();
        let value = obj.to_owned().try_to_bool(vm)?;
        if value == flag {
            self.jump(target);
        } else {
            self.pop_value();
        }
        Ok(None)
    }

    /// The top of stack contains the iterator, lets push it forward
    fn execute_for_iter(&mut self, vm: &VirtualMachine, target: bytecode::Label) -> FrameResult {
        let top_of_stack = PyIter::new(self.top_value());
        let next_obj = top_of_stack.next(vm);

        // Check the next object:
        match next_obj {
            Ok(PyIterReturn::Return(value)) => {
                self.push_value(value);
                Ok(None)
            }
            Ok(PyIterReturn::StopIteration(_)) => {
                // Pop iterator from stack:
                self.pop_value();

                // End of for loop
                self.jump(target);
                Ok(None)
            }
            Err(next_error) => {
                // Pop iterator from stack:
                self.pop_value();
                Err(next_error)
            }
        }
    }
    fn execute_make_function(&mut self, vm: &VirtualMachine) -> FrameResult {
        // MakeFunction only takes code object, no flags
        let code_obj: PyRef<PyCode> = self
            .pop_value()
            .downcast()
            .expect("Stack value should be code object");

        // Create function with minimal attributes
        let func_obj = PyFunction::new(code_obj, self.globals.clone(), vm)?.into_pyobject(vm);

        self.push_value(func_obj);
        Ok(None)
    }

    fn execute_set_function_attribute(
        &mut self,
        vm: &VirtualMachine,
        attr: bytecode::MakeFunctionFlags,
    ) -> FrameResult {
        // CPython 3.13 style: SET_FUNCTION_ATTRIBUTE sets attributes on a function
        // Stack: [..., attr_value, func] -> [..., func]
        // Stack order: func is at -1, attr_value is at -2

        let func = self.pop_value();
        let attr_value = self.replace_top(func);

        let func = self.top_value();
        // Get the function reference and call the new method
        let func_ref = func
            .downcast_ref::<PyFunction>()
            .expect("SET_FUNCTION_ATTRIBUTE expects function on stack");

        let payload: &PyFunction = func_ref.payload();
        // SetFunctionAttribute always follows MakeFunction, so at this point
        // there are no other references to func. It is therefore safe to treat it as mutable.
        unsafe {
            let payload_ptr = payload as *const PyFunction as *mut PyFunction;
            (*payload_ptr).set_function_attribute(attr, attr_value, vm)?;
        };

        Ok(None)
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn execute_bin_op(&mut self, vm: &VirtualMachine, op: bytecode::BinaryOperator) -> FrameResult {
        let b_ref = &self.pop_value();
        let a_ref = &self.pop_value();
        let value = match op {
            bytecode::BinaryOperator::Subtract => vm._sub(a_ref, b_ref),
            bytecode::BinaryOperator::Add => vm._add(a_ref, b_ref),
            bytecode::BinaryOperator::Multiply => vm._mul(a_ref, b_ref),
            bytecode::BinaryOperator::MatrixMultiply => vm._matmul(a_ref, b_ref),
            bytecode::BinaryOperator::Power => vm._pow(a_ref, b_ref, vm.ctx.none.as_object()),
            bytecode::BinaryOperator::Divide => vm._truediv(a_ref, b_ref),
            bytecode::BinaryOperator::FloorDivide => vm._floordiv(a_ref, b_ref),
            bytecode::BinaryOperator::Modulo => vm._mod(a_ref, b_ref),
            bytecode::BinaryOperator::Lshift => vm._lshift(a_ref, b_ref),
            bytecode::BinaryOperator::Rshift => vm._rshift(a_ref, b_ref),
            bytecode::BinaryOperator::Xor => vm._xor(a_ref, b_ref),
            bytecode::BinaryOperator::Or => vm._or(a_ref, b_ref),
            bytecode::BinaryOperator::And => vm._and(a_ref, b_ref),
        }?;

        self.push_value(value);
        Ok(None)
    }
    fn execute_bin_op_inplace(
        &mut self,
        vm: &VirtualMachine,
        op: bytecode::BinaryOperator,
    ) -> FrameResult {
        let b_ref = &self.pop_value();
        let a_ref = &self.pop_value();
        let value = match op {
            bytecode::BinaryOperator::Subtract => vm._isub(a_ref, b_ref),
            bytecode::BinaryOperator::Add => vm._iadd(a_ref, b_ref),
            bytecode::BinaryOperator::Multiply => vm._imul(a_ref, b_ref),
            bytecode::BinaryOperator::MatrixMultiply => vm._imatmul(a_ref, b_ref),
            bytecode::BinaryOperator::Power => vm._ipow(a_ref, b_ref, vm.ctx.none.as_object()),
            bytecode::BinaryOperator::Divide => vm._itruediv(a_ref, b_ref),
            bytecode::BinaryOperator::FloorDivide => vm._ifloordiv(a_ref, b_ref),
            bytecode::BinaryOperator::Modulo => vm._imod(a_ref, b_ref),
            bytecode::BinaryOperator::Lshift => vm._ilshift(a_ref, b_ref),
            bytecode::BinaryOperator::Rshift => vm._irshift(a_ref, b_ref),
            bytecode::BinaryOperator::Xor => vm._ixor(a_ref, b_ref),
            bytecode::BinaryOperator::Or => vm._ior(a_ref, b_ref),
            bytecode::BinaryOperator::And => vm._iand(a_ref, b_ref),
        }?;

        self.push_value(value);
        Ok(None)
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn execute_unary_op(
        &mut self,
        vm: &VirtualMachine,
        op: bytecode::UnaryOperator,
    ) -> FrameResult {
        let a = self.pop_value();
        let value = match op {
            bytecode::UnaryOperator::Minus => vm._neg(&a)?,
            bytecode::UnaryOperator::Plus => vm._pos(&a)?,
            bytecode::UnaryOperator::Invert => vm._invert(&a)?,
            bytecode::UnaryOperator::Not => {
                let value = a.try_to_bool(vm)?;
                vm.ctx.new_bool(!value).into()
            }
        };
        self.push_value(value);
        Ok(None)
    }

    #[cold]
    fn setup_annotations(&mut self, vm: &VirtualMachine) -> FrameResult {
        let __annotations__ = identifier!(vm, __annotations__);
        // Try using locals as dict first, if not, fallback to generic method.
        let has_annotations = match self
            .locals
            .clone()
            .into_object()
            .downcast_exact::<PyDict>(vm)
        {
            Ok(d) => d.contains_key(__annotations__, vm),
            Err(o) => {
                let needle = __annotations__.as_object();
                self._in(vm, needle, &o)?
            }
        };
        if !has_annotations {
            self.locals
                .as_object()
                .set_item(__annotations__, vm.ctx.new_dict().into(), vm)?;
        }
        Ok(None)
    }

    fn print_expr(&mut self, vm: &VirtualMachine) -> FrameResult {
        let expr = self.pop_value();

        let displayhook = vm
            .sys_module
            .get_attr("displayhook", vm)
            .map_err(|_| vm.new_runtime_error("lost sys.displayhook"))?;
        displayhook.call((expr,), vm)?;

        Ok(None)
    }

    fn unpack_sequence(&mut self, size: u32, vm: &VirtualMachine) -> FrameResult {
        let value = self.pop_value();
        let elements: Vec<_> = value.try_to_value(vm).map_err(|e| {
            if e.class().is(vm.ctx.exceptions.type_error) {
                vm.new_type_error(format!(
                    "cannot unpack non-iterable {} object",
                    value.class().name()
                ))
            } else {
                e
            }
        })?;
        let msg = match elements.len().cmp(&(size as usize)) {
            std::cmp::Ordering::Equal => {
                self.state.stack.extend(elements.into_iter().rev());
                return Ok(None);
            }
            std::cmp::Ordering::Greater => {
                format!("too many values to unpack (expected {size})")
            }
            std::cmp::Ordering::Less => format!(
                "not enough values to unpack (expected {}, got {})",
                size,
                elements.len()
            ),
        };
        Err(vm.new_value_error(msg))
    }

    fn format_value(
        &mut self,
        conversion: bytecode::ConversionFlag,
        vm: &VirtualMachine,
    ) -> FrameResult {
        use bytecode::ConversionFlag;
        let value = self.pop_value();
        let value = match conversion {
            ConversionFlag::Str => value.str(vm)?.into(),
            ConversionFlag::Repr => value.repr(vm)?.into(),
            ConversionFlag::Ascii => vm.ctx.new_str(builtins::ascii(value, vm)?).into(),
            ConversionFlag::None => value,
        };

        let spec = self.pop_value();
        let formatted = vm.format(&value, spec.downcast::<PyStr>().unwrap())?;
        self.push_value(formatted.into());
        Ok(None)
    }

    fn _in(&self, vm: &VirtualMachine, needle: &PyObject, haystack: &PyObject) -> PyResult<bool> {
        let found = vm._contains(haystack, needle)?;
        Ok(found)
    }

    #[inline(always)]
    fn _not_in(
        &self,
        vm: &VirtualMachine,
        needle: &PyObject,
        haystack: &PyObject,
    ) -> PyResult<bool> {
        Ok(!self._in(vm, needle, haystack)?)
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn execute_test(&mut self, vm: &VirtualMachine, op: bytecode::TestOperator) -> FrameResult {
        let b = self.pop_value();
        let a = self.pop_value();
        let value = match op {
            bytecode::TestOperator::Is => a.is(&b),
            bytecode::TestOperator::IsNot => !a.is(&b),
            bytecode::TestOperator::In => self._in(vm, &a, &b)?,
            bytecode::TestOperator::NotIn => self._not_in(vm, &a, &b)?,
            bytecode::TestOperator::ExceptionMatch => {
                if let Some(tuple_of_exceptions) = b.downcast_ref::<PyTuple>() {
                    for exception in tuple_of_exceptions {
                        if !exception
                            .is_subclass(vm.ctx.exceptions.base_exception_type.into(), vm)?
                        {
                            return Err(vm.new_type_error(
                                "catching classes that do not inherit from BaseException is not allowed",
                            ));
                        }
                    }
                } else if !b.is_subclass(vm.ctx.exceptions.base_exception_type.into(), vm)? {
                    return Err(vm.new_type_error(
                        "catching classes that do not inherit from BaseException is not allowed",
                    ));
                }

                a.is_instance(&b, vm)?
            }
        };

        self.push_value(vm.ctx.new_bool(value).into());
        Ok(None)
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn execute_compare(
        &mut self,
        vm: &VirtualMachine,
        op: bytecode::ComparisonOperator,
    ) -> FrameResult {
        let b = self.pop_value();
        let a = self.pop_value();
        let value = a.rich_compare(b, op.into(), vm)?;
        self.push_value(value);
        Ok(None)
    }

    fn load_attr(&mut self, vm: &VirtualMachine, attr: bytecode::NameIdx) -> FrameResult {
        let attr_name = self.code.names[attr as usize];
        let parent = self.pop_value();
        let obj = parent.get_attr(attr_name, vm)?;
        self.push_value(obj);
        Ok(None)
    }

    fn store_attr(&mut self, vm: &VirtualMachine, attr: bytecode::NameIdx) -> FrameResult {
        let attr_name = self.code.names[attr as usize];
        let parent = self.pop_value();
        let value = self.pop_value();
        parent.set_attr(attr_name, value, vm)?;
        Ok(None)
    }

    fn delete_attr(&mut self, vm: &VirtualMachine, attr: bytecode::NameIdx) -> FrameResult {
        let attr_name = self.code.names[attr as usize];
        let parent = self.pop_value();
        parent.del_attr(attr_name, vm)?;
        Ok(None)
    }

    fn push_block(&mut self, typ: BlockType) {
        // eprintln!("block pushed: {:.60?} {}", typ, self.state.stack.len());
        self.state.blocks.push(Block {
            typ,
            level: self.state.stack.len(),
        });
    }

    #[track_caller]
    fn pop_block(&mut self) -> Block {
        let block = self.state.blocks.pop().expect("No more blocks to pop!");
        // eprintln!(
        //     "block popped: {:.60?}  {} -> {} ",
        //     block.typ,
        //     self.state.stack.len(),
        //     block.level
        // );
        #[cfg(debug_assertions)]
        if self.state.stack.len() < block.level {
            dbg!(&self);
            panic!(
                "stack size reversion: current size({}) < truncates target({}).",
                self.state.stack.len(),
                block.level
            );
        }
        self.state.stack.truncate(block.level);
        block
    }

    #[inline]
    fn current_block(&self) -> Option<Block> {
        self.state.blocks.last().cloned()
    }

    #[inline]
    #[track_caller] // not a real track_caller but push_value is not very useful
    fn push_value(&mut self, obj: PyObjectRef) {
        // eprintln!(
        //     "push_value {} / len: {} +1",
        //     obj.class().name(),
        //     self.state.stack.len()
        // );
        match self.state.stack.try_push(obj) {
            Ok(()) => {}
            Err(_e) => self.fatal("tried to push value onto stack but overflowed max_stackdepth"),
        }
    }

    #[inline]
    #[track_caller] // not a real track_caller but pop_value is not very useful
    fn pop_value(&mut self) -> PyObjectRef {
        match self.state.stack.pop() {
            Some(x) => {
                // eprintln!(
                //     "pop_value {} / len: {}",
                //     x.class().name(),
                //     self.state.stack.len()
                // );
                x
            }
            None => self.fatal("tried to pop value but there was nothing on the stack"),
        }
    }

    fn call_intrinsic_1(
        &mut self,
        func: bytecode::IntrinsicFunction1,
        arg: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        match func {
            bytecode::IntrinsicFunction1::ImportStar => {
                // arg is the module object
                self.push_value(arg); // Push module back on stack for import_star
                self.import_star(vm)?;
                Ok(vm.ctx.none())
            }
            bytecode::IntrinsicFunction1::SubscriptGeneric => {
                // Used for PEP 695: Generic[*type_params]
                crate::builtins::genericalias::subscript_generic(arg, vm)
            }
            bytecode::IntrinsicFunction1::TypeVar => {
                let type_var: PyObjectRef =
                    typing::TypeVar::new(vm, arg.clone(), vm.ctx.none(), vm.ctx.none())
                        .into_ref(&vm.ctx)
                        .into();
                Ok(type_var)
            }
            bytecode::IntrinsicFunction1::ParamSpec => {
                let param_spec: PyObjectRef = typing::ParamSpec::new(arg.clone(), vm)
                    .into_ref(&vm.ctx)
                    .into();
                Ok(param_spec)
            }
            bytecode::IntrinsicFunction1::TypeVarTuple => {
                let type_var_tuple: PyObjectRef = typing::TypeVarTuple::new(arg.clone(), vm)
                    .into_ref(&vm.ctx)
                    .into();
                Ok(type_var_tuple)
            }
            bytecode::IntrinsicFunction1::TypeAlias => {
                // TypeAlias receives a tuple of (name, type_params, value)
                let tuple: PyTupleRef = arg
                    .downcast()
                    .map_err(|_| vm.new_type_error("TypeAlias expects a tuple argument"))?;

                if tuple.len() != 3 {
                    return Err(vm.new_type_error(format!(
                        "TypeAlias expects exactly 3 arguments, got {}",
                        tuple.len()
                    )));
                }

                let name = tuple.as_slice()[0].clone();
                let type_params_obj = tuple.as_slice()[1].clone();
                let value = tuple.as_slice()[2].clone();

                let type_params: PyTupleRef = if vm.is_none(&type_params_obj) {
                    vm.ctx.empty_tuple.clone()
                } else {
                    type_params_obj
                        .downcast()
                        .map_err(|_| vm.new_type_error("Type params must be a tuple."))?
                };

                let type_alias = typing::TypeAliasType::new(name, type_params, value);
                Ok(type_alias.into_ref(&vm.ctx).into())
            }
            bytecode::IntrinsicFunction1::ListToTuple => {
                // Convert list to tuple
                let list = arg
                    .downcast::<PyList>()
                    .map_err(|_| vm.new_type_error("LIST_TO_TUPLE expects a list"))?;
                Ok(vm.ctx.new_tuple(list.borrow_vec().to_vec()).into())
            }
        }
    }

    fn call_intrinsic_2(
        &mut self,
        func: bytecode::IntrinsicFunction2,
        arg1: PyObjectRef,
        arg2: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        match func {
            bytecode::IntrinsicFunction2::SetTypeparamDefault => {
                crate::stdlib::typing::set_typeparam_default(arg1, arg2, vm)
            }
            bytecode::IntrinsicFunction2::SetFunctionTypeParams => {
                // arg1 is the function, arg2 is the type params tuple
                // Set __type_params__ attribute on the function
                arg1.set_attr("__type_params__", arg2, vm)?;
                Ok(arg1)
            }
            bytecode::IntrinsicFunction2::TypeVarWithBound => {
                let type_var: PyObjectRef =
                    typing::TypeVar::new(vm, arg1.clone(), arg2, vm.ctx.none())
                        .into_ref(&vm.ctx)
                        .into();
                Ok(type_var)
            }
            bytecode::IntrinsicFunction2::TypeVarWithConstraint => {
                let type_var: PyObjectRef =
                    typing::TypeVar::new(vm, arg1.clone(), vm.ctx.none(), arg2)
                        .into_ref(&vm.ctx)
                        .into();
                Ok(type_var)
            }
        }
    }

    fn pop_multiple(&mut self, count: usize) -> crate::common::boxvec::Drain<'_, PyObjectRef> {
        let stack_len = self.state.stack.len();
        self.state.stack.drain(stack_len - count..)
    }

    #[inline]
    fn replace_top(&mut self, mut top: PyObjectRef) -> PyObjectRef {
        let last = self.state.stack.last_mut().unwrap();
        std::mem::swap(&mut top, last);
        top
    }

    #[inline]
    #[track_caller] // not a real track_caller but top_value is not very useful
    fn top_value(&self) -> &PyObject {
        match &*self.state.stack {
            [.., last] => last,
            [] => self.fatal("tried to get top of stack but stack is empty"),
        }
    }

    #[inline]
    #[track_caller]
    fn nth_value(&self, depth: u32) -> &PyObject {
        let stack = &self.state.stack;
        &stack[stack.len() - depth as usize - 1]
    }

    #[cold]
    #[inline(never)]
    #[track_caller]
    fn fatal(&self, msg: &'static str) -> ! {
        dbg!(self);
        panic!("{msg}")
    }
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state.lock();
        let stack_str = state.stack.iter().fold(String::new(), |mut s, elem| {
            if elem.downcastable::<Self>() {
                s.push_str("\n  > {frame}");
            } else {
                std::fmt::write(&mut s, format_args!("\n  > {elem:?}")).unwrap();
            }
            s
        });
        let block_str = state.blocks.iter().fold(String::new(), |mut s, elem| {
            std::fmt::write(&mut s, format_args!("\n  > {elem:?}")).unwrap();
            s
        });
        // TODO: fix this up
        let locals = self.locals.clone();
        write!(
            f,
            "Frame Object {{ \n Stack:{}\n Blocks:{}\n Locals:{:?}\n}}",
            stack_str,
            block_str,
            locals.into_object()
        )
    }
}

fn is_module_initializing(module: &PyObject, vm: &VirtualMachine) -> bool {
    let Ok(spec) = module.get_attr(&vm.ctx.new_str("__spec__"), vm) else {
        return false;
    };
    if vm.is_none(&spec) {
        return false;
    }
    let Ok(initializing_attr) = spec.get_attr(&vm.ctx.new_str("_initializing"), vm) else {
        return false;
    };
    initializing_attr.try_to_bool(vm).unwrap_or(false)
}
