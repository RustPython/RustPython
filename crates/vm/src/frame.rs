use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject, VirtualMachine,
    builtins::{
        PyBaseException, PyBaseExceptionRef, PyCode, PyCoroutine, PyDict, PyDictRef, PyGenerator,
        PyInterpolation, PyList, PySet, PySlice, PyStr, PyStrInterned, PyStrRef, PyTemplate,
        PyTraceback, PyType,
        asyncgenerator::PyAsyncGenWrappedValue,
        function::{PyCell, PyCellRef, PyFunction},
        tuple::{PyTuple, PyTupleRef},
    },
    bytecode::{self, Instruction},
    convert::{IntoObject, ToPyResult},
    coroutine::Coro,
    exceptions::ExceptionCtor,
    function::{ArgMapping, Either, FuncArgs},
    protocol::{PyIter, PyIterReturn},
    scope::Scope,
    stdlib::{builtins, typing},
    types::PyTypeFlags,
    vm::{Context, PyMethod},
};
use alloc::fmt;
use core::iter::zip;
#[cfg(feature = "threading")]
use core::sync::atomic;
use indexmap::IndexMap;
use itertools::Itertools;

use rustpython_common::{boxvec::BoxVec, lock::PyMutex, wtf8::Wtf8Buf};
use rustpython_compiler_core::SourceLocation;

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
    stack: BoxVec<Option<PyObjectRef>>,
    /// index of last instruction ran
    #[cfg(feature = "threading")]
    lasti: u32,
}

#[cfg(feature = "threading")]
type Lasti = atomic::AtomicU32;
#[cfg(not(feature = "threading"))]
type Lasti = core::cell::Cell<u32>;

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
        let cells_frees = core::iter::repeat_with(|| PyCell::default().into_ref(&vm.ctx))
            .take(code.cellvars.len())
            .chain(closure.iter().cloned())
            .collect();

        let state = FrameState {
            stack: BoxVec::new(code.max_stackdepth as usize),
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
        self.code.locations[self.lasti() as usize - 1].0
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
        let j = core::cmp::min(map.len(), code.varnames.len());
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
            if code.flags.contains(bytecode::CodeFlags::OPTIMIZED) {
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
                        is_reraise: bool,
                        vm: &VirtualMachine,
                    ) -> FrameResult {
                        // 1. Extract traceback from exception's '__traceback__' attr.
                        // 2. Add new entry with current execution position (filename, lineno, code_object) to traceback.
                        // 3. First, try to find handler in exception table

                        // RERAISE instructions should not add traceback entries - they're just
                        // re-raising an already-processed exception
                        if !is_reraise {
                            // PyTraceBack_Here always adds a new entry without
                            // checking for duplicates. Each time an exception passes through
                            // a frame (e.g., in a loop with repeated raise statements),
                            // a new traceback entry is added.
                            let (loc, _end_loc) = frame.code.locations[idx];
                            let next = exception.__traceback__();

                            let new_traceback = PyTraceback::new(
                                next,
                                frame.object.to_owned(),
                                idx as u32 * 2,
                                loc.line,
                            );
                            vm_trace!("Adding to traceback: {:?} {:?}", new_traceback, loc.line);
                            exception.set_traceback_typed(Some(new_traceback.into_ref(&vm.ctx)));
                        }

                        // Only contextualize exception for new raises, not re-raises
                        // CPython only calls _PyErr_SetObject (which does chaining) on initial raise
                        // RERAISE just propagates the exception without modifying __context__
                        if !is_reraise {
                            vm.contextualize_exception(&exception);
                        }

                        // Use exception table for zero-cost exception handling
                        frame.unwind_blocks(vm, UnwindReason::Raising { exception })
                    }

                    // Check if this is a RERAISE instruction
                    // Both AnyInstruction::Raise { kind: Reraise/ReraiseFromStack } and
                    // AnyInstruction::Reraise are reraise operations that should not add
                    // new traceback entries
                    let is_reraise = match op {
                        Instruction::RaiseVarargs { kind } => matches!(
                            kind.get(arg),
                            bytecode::RaiseKind::BareRaise | bytecode::RaiseKind::ReraiseFromStack
                        ),
                        Instruction::Reraise { .. } => true,
                        _ => false,
                    };

                    match handle_exception(self, exception, idx, is_reraise, vm) {
                        Ok(None) => {}
                        Ok(Some(result)) => break Ok(result),
                        Err(exception) => {
                            // Restore lasti from traceback so frame.f_lineno matches tb_lineno
                            // The traceback was created with the correct lasti when exception
                            // was first raised, but frame.lasti may have changed during cleanup
                            if let Some(tb) = exception.__traceback__()
                                && core::ptr::eq::<Py<Frame>>(&*tb.frame, self.object)
                            {
                                // This traceback entry is for this frame - restore its lasti
                                // tb.lasti is in bytes (idx * 2), convert back to instruction index
                                self.update_lasti(|i| *i = tb.lasti / 2);
                            }
                            break Err(exception);
                        }
                    }
                }
            }
            if !do_extend_arg {
                arg_state.reset()
            }
        }
    }

    fn yield_from_target(&self) -> Option<&PyObject> {
        // checks gi_frame_state == FRAME_SUSPENDED_YIELD_FROM
        // which is set when YIELD_VALUE with oparg >= 1 is executed.
        // In RustPython, we check:
        // 1. lasti points to RESUME (after YIELD_VALUE)
        // 2. The previous instruction was YIELD_VALUE with arg >= 1
        // 3. Stack top is the delegate (receiver)
        //
        // First check if stack is empty - if so, we can't be in yield-from
        if self.state.stack.is_empty() {
            return None;
        }
        let lasti = self.lasti() as usize;
        if let Some(unit) = self.code.instructions.get(lasti) {
            match &unit.op {
                Instruction::Send { .. } => return Some(self.top_value()),
                Instruction::Resume { .. } => {
                    // Check if previous instruction was YIELD_VALUE with arg >= 1
                    // This indicates yield-from/await context
                    if lasti > 0
                        && let Some(prev_unit) = self.code.instructions.get(lasti - 1)
                        && let Instruction::YieldValue { .. } = &prev_unit.op
                    {
                        // YIELD_VALUE arg: 0 = direct yield, >= 1 = yield-from/await
                        // OpArgByte.0 is the raw byte value
                        if prev_unit.arg.0 >= 1 {
                            // In yield-from/await context, delegate is on top of stack
                            return Some(self.top_value());
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Handle throw() on a generator/coroutine.
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
                        .to_pyresult(vm),
                    Either::B(meth) => meth.call((exc_type, exc_val, exc_tb), vm),
                };
                return ret.map(ExecutionResult::Yield).or_else(|err| {
                    // This pushes Py_None to stack and restarts evalloop in exception mode.
                    // Stack before throw: [receiver] (YIELD_VALUE already popped yielded value)
                    // After pushing None: [receiver, None]
                    // Exception handler will push exc: [receiver, None, exc]
                    // CLEANUP_THROW expects: [sub_iter, last_sent_val, exc]
                    self.push_value(vm.ctx.none());

                    // Use unwind_blocks to let exception table route to CLEANUP_THROW
                    match self.unwind_blocks(vm, UnwindReason::Raising { exception: err }) {
                        Ok(None) => self.run(vm),
                        Ok(Some(result)) => Ok(result),
                        Err(exception) => Err(exception),
                    }
                });
            }
        }
        // throw_here: no delegate has throw method, or not in yield-from
        // gen_send_ex pushes Py_None to stack and restarts evalloop in exception mode
        let exception = vm.normalize_exception(exc_type, exc_val, exc_tb)?;

        // Add traceback entry for the generator frame at the yield site
        let idx = self.lasti().saturating_sub(1) as usize;
        if idx < self.code.locations.len() {
            let (loc, _end_loc) = self.code.locations[idx];
            let next = exception.__traceback__();
            let new_traceback =
                PyTraceback::new(next, self.object.to_owned(), idx as u32 * 2, loc.line);
            exception.set_traceback_typed(Some(new_traceback.into_ref(&vm.ctx)));
        }

        // when raising an exception, set __context__ to the current exception
        // This is done in _PyErr_SetObject
        vm.contextualize_exception(&exception);

        // always pushes Py_None before calling gen_send_ex with exc=1
        // This is needed for exception handler to have correct stack state
        self.push_value(vm.ctx.none());

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
        instruction: Instruction,
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
                instruction.display(arg, &self.code.code)
            );
            trace!("=======");
        }

        #[cold]
        fn name_error(name: &'static PyStrInterned, vm: &VirtualMachine) -> PyBaseExceptionRef {
            vm.new_name_error(format!("name '{name}' is not defined"), name.to_owned())
        }

        match instruction {
            Instruction::BeforeAsyncWith => {
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
            Instruction::BinaryOp { op } => self.execute_bin_op(vm, op.get(arg)),
            Instruction::BinarySubscr => {
                let key = self.pop_value();
                let container = self.pop_value();
                let result = container.get_item(key.as_object(), vm)?;
                self.push_value(result);
                Ok(None)
            }

            Instruction::Break { target } => self.unwind_blocks(
                vm,
                UnwindReason::Break {
                    target: target.get(arg),
                },
            ),
            Instruction::BuildListFromTuples { size } => {
                // SAFETY: compiler guarantees `size` tuples are on the stack
                let elements = unsafe { self.flatten_tuples(size.get(arg) as usize) };
                let list_obj = vm.ctx.new_list(elements);
                self.push_value(list_obj.into());
                Ok(None)
            }
            Instruction::BuildList { size } => {
                let elements = self.pop_multiple(size.get(arg) as usize).collect();
                let list_obj = vm.ctx.new_list(elements);
                self.push_value(list_obj.into());
                Ok(None)
            }
            Instruction::BuildMapForCall { size } => {
                self.execute_build_map_for_call(vm, size.get(arg))
            }
            Instruction::BuildMap { size } => self.execute_build_map(vm, size.get(arg)),
            Instruction::BuildSetFromTuples { size } => {
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
            Instruction::BuildSet { size } => {
                let set = PySet::default().into_ref(&vm.ctx);
                for element in self.pop_multiple(size.get(arg) as usize) {
                    set.add(element, vm)?;
                }
                self.push_value(set.into());
                Ok(None)
            }
            Instruction::BuildSlice { argc } => self.execute_build_slice(vm, argc.get(arg)),
            /*
             Instruction::ToBool => {
                 dbg!("Shouldn't be called outside of match statements for now")
                 let value = self.pop_value();
                 // call __bool__
                 let result = value.try_to_bool(vm)?;
                 self.push_value(vm.ctx.new_bool(result).into());
                 Ok(None)
            }
            */
            Instruction::BuildString { size } => {
                let s: Wtf8Buf = self
                    .pop_multiple(size.get(arg) as usize)
                    .map(|pyobj| pyobj.downcast::<PyStr>().unwrap())
                    .collect();
                self.push_value(vm.ctx.new_str(s).into());
                Ok(None)
            }
            Instruction::BuildTupleFromIter => {
                if !self.top_value().class().is(vm.ctx.types.tuple_type) {
                    let elements: Vec<_> = self.pop_value().try_to_value(vm)?;
                    let list_obj = vm.ctx.new_tuple(elements);
                    self.push_value(list_obj.into());
                }
                Ok(None)
            }
            Instruction::BuildTupleFromTuples { size } => {
                // SAFETY: compiler guarantees `size` tuples are on the stack
                let elements = unsafe { self.flatten_tuples(size.get(arg) as usize) };
                let list_obj = vm.ctx.new_tuple(elements);
                self.push_value(list_obj.into());
                Ok(None)
            }
            Instruction::BuildTuple { size } => {
                let elements = self.pop_multiple(size.get(arg) as usize).collect();
                let list_obj = vm.ctx.new_tuple(elements);
                self.push_value(list_obj.into());
                Ok(None)
            }
            Instruction::BuildTemplate => {
                // Stack: [strings_tuple, interpolations_tuple] -> [template]
                let interpolations = self.pop_value();
                let strings = self.pop_value();

                let strings = strings
                    .downcast::<PyTuple>()
                    .map_err(|_| vm.new_type_error("BUILD_TEMPLATE expected tuple for strings"))?;
                let interpolations = interpolations.downcast::<PyTuple>().map_err(|_| {
                    vm.new_type_error("BUILD_TEMPLATE expected tuple for interpolations")
                })?;

                let template = PyTemplate::new(strings, interpolations);
                self.push_value(template.into_pyobject(vm));
                Ok(None)
            }
            Instruction::BuildInterpolation { oparg } => {
                // oparg encoding: (conversion << 2) | has_format_spec
                // Stack: [value, expression_str, (format_spec)?] -> [interpolation]
                let oparg_val = oparg.get(arg);
                let has_format_spec = (oparg_val & 1) != 0;
                let conversion_code = oparg_val >> 2;

                let format_spec = if has_format_spec {
                    self.pop_value().downcast::<PyStr>().map_err(|_| {
                        vm.new_type_error("BUILD_INTERPOLATION expected str for format_spec")
                    })?
                } else {
                    vm.ctx.empty_str.to_owned()
                };

                let expression = self.pop_value().downcast::<PyStr>().map_err(|_| {
                    vm.new_type_error("BUILD_INTERPOLATION expected str for expression")
                })?;
                let value = self.pop_value();

                // conversion: 0=None, 1=Str, 2=Repr, 3=Ascii
                let conversion: PyObjectRef = match conversion_code {
                    0 => vm.ctx.none(),
                    1 => vm.ctx.new_str("s").into(),
                    2 => vm.ctx.new_str("r").into(),
                    3 => vm.ctx.new_str("a").into(),
                    _ => vm.ctx.none(), // should not happen
                };

                let interpolation =
                    PyInterpolation::new(value, expression, conversion, format_spec, vm)?;
                self.push_value(interpolation.into_pyobject(vm));
                Ok(None)
            }
            Instruction::Call { nargs } => {
                // Stack: [callable, self_or_null, arg1, ..., argN]
                let args = self.collect_positional_args(nargs.get(arg));
                self.execute_call(args, vm)
            }
            Instruction::CallKw { nargs } => {
                // Stack: [callable, self_or_null, arg1, ..., argN, kwarg_names]
                let args = self.collect_keyword_args(nargs.get(arg));
                self.execute_call(args, vm)
            }
            Instruction::CallFunctionEx { has_kwargs } => {
                // Stack: [callable, self_or_null, args_tuple, (kwargs_dict)?]
                let args = self.collect_ex_args(vm, has_kwargs.get(arg))?;
                self.execute_call(args, vm)
            }
            Instruction::CallIntrinsic1 { func } => {
                let value = self.pop_value();
                let result = self.call_intrinsic_1(func.get(arg), value, vm)?;
                self.push_value(result);
                Ok(None)
            }
            Instruction::CallIntrinsic2 { func } => {
                let value2 = self.pop_value();
                let value1 = self.pop_value();
                let result = self.call_intrinsic_2(func.get(arg), value1, value2, vm)?;
                self.push_value(result);
                Ok(None)
            }
            Instruction::CheckEgMatch => {
                let match_type = self.pop_value();
                let exc_value = self.pop_value();
                let (rest, matched) =
                    crate::exceptions::exception_group_match(&exc_value, &match_type, vm)?;
                self.push_value(rest);
                self.push_value(matched);
                Ok(None)
            }
            Instruction::CompareOp { op } => self.execute_compare(vm, op.get(arg)),
            Instruction::ContainsOp(invert) => {
                let b = self.pop_value();
                let a = self.pop_value();

                let value = match invert.get(arg) {
                    bytecode::Invert::No => self._in(vm, &a, &b)?,
                    bytecode::Invert::Yes => self._not_in(vm, &a, &b)?,
                };
                self.push_value(vm.ctx.new_bool(value).into());
                Ok(None)
            }
            Instruction::Continue { target } => self.unwind_blocks(
                vm,
                UnwindReason::Continue {
                    target: target.get(arg),
                },
            ),

            Instruction::ConvertValue { oparg: conversion } => {
                self.convert_value(conversion.get(arg), vm)
            }
            Instruction::Copy { index } => {
                // CopyItem { index: 1 } copies TOS
                // CopyItem { index: 2 } copies second from top
                // This is 1-indexed to match CPython
                let idx = index.get(arg) as usize;
                let stack_len = self.state.stack.len();
                if stack_len < idx {
                    eprintln!("CopyItem ERROR: stack_len={}, idx={}", stack_len, idx);
                    eprintln!("  code: {}", self.code.obj_name);
                    eprintln!("  lasti: {}", self.lasti());
                    panic!("CopyItem: stack underflow");
                }
                let value = self.state.stack[stack_len - idx].clone();
                self.push_value_opt(value);
                Ok(None)
            }
            Instruction::DeleteAttr { idx } => self.delete_attr(vm, idx.get(arg)),
            Instruction::DeleteDeref(i) => {
                self.cells_frees[i.get(arg) as usize].set(None);
                Ok(None)
            }
            Instruction::DeleteFast(idx) => {
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
            Instruction::DeleteGlobal(idx) => {
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
            Instruction::DeleteName(idx) => {
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
            Instruction::DeleteSubscr => self.execute_delete_subscript(vm),
            Instruction::DictUpdate { index } => {
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
            Instruction::EndAsyncFor => {
                // END_ASYNC_FOR pops (awaitable, exc) from stack
                // Stack: [awaitable, exc] -> []
                // exception_unwind pushes exception to stack before jumping to handler
                let exc = self.pop_value();
                let _awaitable = self.pop_value();

                let exc = exc
                    .downcast::<PyBaseException>()
                    .expect("EndAsyncFor expects exception on stack");

                if exc.fast_isinstance(vm.ctx.exceptions.stop_async_iteration) {
                    // StopAsyncIteration - normal end of async for loop
                    vm.set_exception(None);
                    Ok(None)
                } else {
                    // Other exception - re-raise
                    Err(exc)
                }
            }
            Instruction::ExtendedArg => {
                *extend_arg = true;
                Ok(None)
            }
            Instruction::ForIter { target } => self.execute_for_iter(vm, target.get(arg)),
            Instruction::FormatSimple => {
                let value = self.pop_value();
                let formatted = vm.format(&value, vm.ctx.new_str(""))?;
                self.push_value(formatted.into());

                Ok(None)
            }
            Instruction::FormatWithSpec => {
                let spec = self.pop_value();
                let value = self.pop_value();
                let formatted = vm.format(&value, spec.downcast::<PyStr>().unwrap())?;
                self.push_value(formatted.into());

                Ok(None)
            }
            Instruction::GetAIter => {
                let aiterable = self.pop_value();
                let aiter = vm.call_special_method(&aiterable, identifier!(vm, __aiter__), ())?;
                self.push_value(aiter);
                Ok(None)
            }
            Instruction::GetANext => {
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
            Instruction::GetAwaitable => {
                use crate::protocol::PyIter;

                let awaited_obj = self.pop_value();
                let awaitable = if let Some(coro) = awaited_obj.downcast_ref::<PyCoroutine>() {
                    // _PyGen_yf() check - detect if coroutine is already being awaited elsewhere
                    if coro.as_coro().frame().yield_from_target().is_some() {
                        return Err(
                            vm.new_runtime_error("coroutine is being awaited already".to_owned())
                        );
                    }
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
                    let result = await_method.call((), vm)?;
                    // Check that __await__ returned an iterator
                    if !PyIter::check(&result) {
                        return Err(vm.new_type_error(format!(
                            "__await__() returned non-iterator of type '{}'",
                            result.class().name()
                        )));
                    }
                    result
                };
                self.push_value(awaitable);
                Ok(None)
            }
            Instruction::GetIter => {
                let iterated_obj = self.pop_value();
                let iter_obj = iterated_obj.get_iter(vm)?;
                self.push_value(iter_obj.into());
                Ok(None)
            }
            Instruction::GetYieldFromIter => {
                // GET_YIELD_FROM_ITER: prepare iterator for yield from
                // If iterable is a coroutine, ensure we're in a coroutine context
                // If iterable is a generator, use it directly
                // Otherwise, call iter() on it
                let iterable = self.pop_value();
                let iter = if iterable.class().is(vm.ctx.types.coroutine_type) {
                    // Coroutine requires CO_COROUTINE or CO_ITERABLE_COROUTINE flag
                    if !self.code.flags.intersects(bytecode::CodeFlags::COROUTINE) {
                        return Err(vm.new_type_error(
                            "cannot 'yield from' a coroutine object in a non-coroutine generator"
                                .to_owned(),
                        ));
                    }
                    iterable
                } else if iterable.class().is(vm.ctx.types.generator_type) {
                    // Generator can be used directly
                    iterable
                } else {
                    // Otherwise, get iterator
                    iterable.get_iter(vm)?.into()
                };
                self.push_value(iter);
                Ok(None)
            }
            Instruction::GetLen => {
                // STACK.append(len(STACK[-1]))
                let obj = self.top_value();
                let len = obj.length(vm)?;
                self.push_value(vm.ctx.new_int(len).into());
                Ok(None)
            }
            Instruction::ImportFrom { idx } => {
                let obj = self.import_from(vm, idx.get(arg))?;
                self.push_value(obj);
                Ok(None)
            }
            Instruction::ImportName { idx } => {
                self.import(vm, Some(self.code.names[idx.get(arg) as usize]))?;
                Ok(None)
            }
            Instruction::IsOp(invert) => {
                let b = self.pop_value();
                let a = self.pop_value();
                let res = a.is(&b);

                let value = match invert.get(arg) {
                    bytecode::Invert::No => res,
                    bytecode::Invert::Yes => !res,
                };
                self.push_value(vm.ctx.new_bool(value).into());
                Ok(None)
            }
            Instruction::JumpIfFalseOrPop { target } => {
                self.jump_if_or_pop(vm, target.get(arg), false)
            }
            Instruction::JumpIfNotExcMatch(target) => {
                let b = self.pop_value();
                let a = self.pop_value();
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

                let value = a.is_instance(&b, vm)?;
                self.push_value(vm.ctx.new_bool(value).into());
                self.pop_jump_if(vm, target.get(arg), false)
            }
            Instruction::JumpIfTrueOrPop { target } => {
                self.jump_if_or_pop(vm, target.get(arg), true)
            }
            Instruction::JumpForward { target } => {
                self.jump(target.get(arg));
                Ok(None)
            }
            Instruction::JumpBackward { target } => {
                self.jump(target.get(arg));
                Ok(None)
            }
            Instruction::JumpBackwardNoInterrupt { target } => {
                self.jump(target.get(arg));
                Ok(None)
            }
            Instruction::ListAppend { i } => {
                let item = self.pop_value();
                let obj = self.nth_value(i.get(arg));
                let list: &Py<PyList> = unsafe {
                    // SAFETY: trust compiler
                    obj.downcast_unchecked_ref()
                };
                list.append(item);
                Ok(None)
            }
            Instruction::LoadAttr { idx } => self.load_attr(vm, idx.get(arg)),
            Instruction::LoadSuperAttr { arg: idx } => self.load_super_attr(vm, idx.get(arg)),
            Instruction::LoadBuildClass => {
                self.push_value(vm.builtins.get_attr(identifier!(vm, __build_class__), vm)?);
                Ok(None)
            }
            Instruction::LoadFromDictOrDeref(i) => {
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
            Instruction::LoadFromDictOrGlobals(idx) => {
                // PEP 649: Pop dict from stack (classdict), check there first, then globals
                let dict = self.pop_value();
                let name = self.code.names[idx.get(arg) as usize];

                // Only treat KeyError as "not found", propagate other exceptions
                let value = if let Some(dict_obj) = dict.downcast_ref::<PyDict>() {
                    dict_obj.get_item_opt(name, vm)?
                } else {
                    // Not an exact dict, use mapping protocol
                    match dict.get_item(name, vm) {
                        Ok(v) => Some(v),
                        Err(e) if e.fast_isinstance(vm.ctx.exceptions.key_error) => None,
                        Err(e) => return Err(e),
                    }
                };

                self.push_value(match value {
                    Some(v) => v,
                    None => self.load_global_or_builtin(name, vm)?,
                });
                Ok(None)
            }
            Instruction::LoadClosure(i) => {
                let value = self.cells_frees[i.get(arg) as usize].clone();
                self.push_value(value.into());
                Ok(None)
            }
            Instruction::LoadConst { idx } => {
                self.push_value(self.code.constants[idx.get(arg) as usize].clone().into());
                Ok(None)
            }
            Instruction::LoadDeref(i) => {
                let i = i.get(arg) as usize;
                let x = self.cells_frees[i]
                    .get()
                    .ok_or_else(|| self.unbound_cell_exception(i, vm))?;
                self.push_value(x);
                Ok(None)
            }
            Instruction::LoadFast(idx) => {
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
            Instruction::LoadFastAndClear(idx) => {
                // Load value and clear the slot (for inlined comprehensions)
                // If slot is empty, push None (not an error - variable may not exist yet)
                let idx = idx.get(arg) as usize;
                let x = self.fastlocals.lock()[idx]
                    .take()
                    .unwrap_or_else(|| vm.ctx.none());
                self.push_value(x);
                Ok(None)
            }
            Instruction::LoadGlobal(idx) => {
                let name = &self.code.names[idx.get(arg) as usize];
                let x = self.load_global_or_builtin(name, vm)?;
                self.push_value(x);
                Ok(None)
            }
            Instruction::LoadName(idx) => {
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
            Instruction::MakeFunction => self.execute_make_function(vm),
            Instruction::MapAdd { i } => {
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
            Instruction::MatchClass(nargs) => {
                // STACK[-1] is a tuple of keyword attribute names, STACK[-2] is the class being matched against, and STACK[-3] is the match subject.
                // nargs is the number of positional sub-patterns.
                let kwd_attrs = self.pop_value();
                let kwd_attrs = kwd_attrs.downcast_ref::<PyTuple>().unwrap();
                let cls = self.pop_value();
                let subject = self.pop_value();
                let nargs_val = nargs.get(arg) as usize;

                // Check if subject is an instance of cls
                if subject.is_instance(cls.as_ref(), vm)? {
                    let mut extracted = vec![];

                    // Get __match_args__ for positional arguments if nargs > 0
                    if nargs_val > 0 {
                        // Get __match_args__ from the class
                        let match_args =
                            vm.get_attribute_opt(cls.clone(), identifier!(vm, __match_args__))?;

                        if let Some(match_args) = match_args {
                            // Convert to tuple
                            let match_args = match match_args.downcast_exact::<PyTuple>(vm) {
                                Ok(tuple) => tuple,
                                Err(match_args) => {
                                    // __match_args__ must be a tuple
                                    // Get type names for error message
                                    let type_name = cls
                                        .downcast::<crate::builtins::PyType>()
                                        .map(|t| t.__name__(vm).as_str().to_owned())
                                        .unwrap_or_else(|_| String::from("?"));
                                    let match_args_type_name = match_args.class().__name__(vm);
                                    return Err(vm.new_type_error(format!(
                                        "{}.__match_args__ must be a tuple (got {})",
                                        type_name, match_args_type_name
                                    )));
                                }
                            };

                            // Check if we have enough match args
                            if match_args.len() < nargs_val {
                                return Err(vm.new_type_error(format!(
                                    "class pattern accepts at most {} positional sub-patterns ({} given)",
                                    match_args.len(),
                                    nargs_val
                                )));
                            }

                            // Extract positional attributes
                            for i in 0..nargs_val {
                                let attr_name = &match_args[i];
                                let attr_name_str = match attr_name.downcast_ref::<PyStr>() {
                                    Some(s) => s,
                                    None => {
                                        return Err(vm.new_type_error(
                                            "__match_args__ elements must be strings".to_string(),
                                        ));
                                    }
                                };
                                match subject.get_attr(attr_name_str, vm) {
                                    Ok(value) => extracted.push(value),
                                    Err(e)
                                        if e.fast_isinstance(vm.ctx.exceptions.attribute_error) =>
                                    {
                                        // Missing attribute → non-match
                                        self.push_value(vm.ctx.none());
                                        return Ok(None);
                                    }
                                    Err(e) => return Err(e),
                                }
                            }
                        } else {
                            // No __match_args__, check if this is a type with MATCH_SELF behavior
                            // For built-in types like bool, int, str, list, tuple, dict, etc.
                            // they match the subject itself as the single positional argument
                            let is_match_self_type = cls
                                .downcast::<PyType>()
                                .is_ok_and(|t| t.slots.flags.contains(PyTypeFlags::_MATCH_SELF));

                            if is_match_self_type {
                                if nargs_val == 1 {
                                    // Match the subject itself as the single positional argument
                                    extracted.push(subject.clone());
                                } else if nargs_val > 1 {
                                    // Too many positional arguments for MATCH_SELF
                                    return Err(vm.new_type_error(
                                        "class pattern accepts at most 1 positional sub-pattern for MATCH_SELF types"
                                            .to_string(),
                                    ));
                                }
                            } else {
                                // No __match_args__ and not a MATCH_SELF type
                                if nargs_val > 0 {
                                    return Err(vm.new_type_error(
                                        "class pattern defines no positional sub-patterns (__match_args__ missing)"
                                            .to_string(),
                                    ));
                                }
                            }
                        }
                    }

                    // Extract keyword attributes
                    for name in kwd_attrs {
                        let name_str = name.downcast_ref::<PyStr>().unwrap();
                        match subject.get_attr(name_str, vm) {
                            Ok(value) => extracted.push(value),
                            Err(e) if e.fast_isinstance(vm.ctx.exceptions.attribute_error) => {
                                self.push_value(vm.ctx.none());
                                return Ok(None);
                            }
                            Err(e) => return Err(e),
                        }
                    }

                    self.push_value(vm.ctx.new_tuple(extracted).into());
                } else {
                    // Not an instance, push None
                    self.push_value(vm.ctx.none());
                }
                Ok(None)
            }
            Instruction::MatchKeys => {
                // MATCH_KEYS doesn't pop subject and keys, only reads them
                let keys_tuple = self.top_value(); // stack[-1]
                let subject = self.nth_value(1); // stack[-2]

                // Check if subject is a mapping and extract values for keys
                if subject.class().slots.flags.contains(PyTypeFlags::MAPPING) {
                    let keys = keys_tuple.downcast_ref::<PyTuple>().unwrap();
                    let mut values = Vec::new();
                    let mut all_match = true;

                    // We use the two argument form of map.get(key, default) for two reasons:
                    // - Atomically check for a key and get its value without error handling.
                    // - Don't cause key creation or resizing in dict subclasses like
                    //   collections.defaultdict that define __missing__ (or similar).
                    // See CPython's _PyEval_MatchKeys

                    if let Some(get_method) = vm
                        .get_method(subject.to_owned(), vm.ctx.intern_str("get"))
                        .transpose()?
                    {
                        let dummy = vm
                            .ctx
                            .new_base_object(vm.ctx.types.object_type.to_owned(), None);

                        for key in keys {
                            // value = map.get(key, dummy)
                            match get_method.call((key.as_object(), dummy.clone()), vm) {
                                Ok(value) => {
                                    // if value == dummy: key not in map!
                                    if value.is(&dummy) {
                                        all_match = false;
                                        break;
                                    }
                                    values.push(value);
                                }
                                Err(e) => return Err(e),
                            }
                        }
                    } else {
                        // Fallback if .get() method is not available (shouldn't happen for mappings)
                        for key in keys {
                            match subject.get_item(key.as_object(), vm) {
                                Ok(value) => values.push(value),
                                Err(e) if e.fast_isinstance(vm.ctx.exceptions.key_error) => {
                                    all_match = false;
                                    break;
                                }
                                Err(e) => return Err(e),
                            }
                        }
                    }

                    if all_match {
                        // Push values tuple on successful match
                        self.push_value(vm.ctx.new_tuple(values).into());
                    } else {
                        // No match - push None
                        self.push_value(vm.ctx.none());
                    }
                } else {
                    // Not a mapping - push None
                    self.push_value(vm.ctx.none());
                }
                Ok(None)
            }
            Instruction::MatchMapping => {
                // Pop and push back the subject to keep it on stack
                let subject = self.pop_value();

                // Check if the type has the MAPPING flag
                let is_mapping = subject.class().slots.flags.contains(PyTypeFlags::MAPPING);

                self.push_value(subject);
                self.push_value(vm.ctx.new_bool(is_mapping).into());
                Ok(None)
            }
            Instruction::MatchSequence => {
                // Pop and push back the subject to keep it on stack
                let subject = self.pop_value();

                // Check if the type has the SEQUENCE flag
                let is_sequence = subject.class().slots.flags.contains(PyTypeFlags::SEQUENCE);

                self.push_value(subject);
                self.push_value(vm.ctx.new_bool(is_sequence).into());
                Ok(None)
            }
            Instruction::Nop => Ok(None),
            Instruction::PopExcept => {
                // Pop prev_exc from value stack and restore it
                let prev_exc = self.pop_value();
                if vm.is_none(&prev_exc) {
                    vm.set_exception(None);
                } else if let Ok(exc) = prev_exc.downcast::<PyBaseException>() {
                    vm.set_exception(Some(exc));
                }

                // NOTE: We do NOT clear the traceback of the exception that was just handled.
                // Python preserves exception tracebacks even after the exception is no longer
                // the "current exception". This is important for code that catches an exception,
                // stores it, and later inspects its traceback.
                // Reference cycles (Exception → Traceback → Frame → locals) are handled by
                // Python's garbage collector which can detect and break cycles.

                Ok(None)
            }
            Instruction::PopJumpIfFalse { target } => self.pop_jump_if(vm, target.get(arg), false),
            Instruction::PopJumpIfTrue { target } => self.pop_jump_if(vm, target.get(arg), true),
            Instruction::PopTop => {
                // Pop value from stack and ignore.
                self.pop_value();
                Ok(None)
            }
            Instruction::PushNull => {
                // Push NULL for self_or_null slot in call protocol
                self.push_null();
                Ok(None)
            }
            Instruction::RaiseVarargs { kind } => self.execute_raise(vm, kind.get(arg)),
            Instruction::Resume { arg: resume_arg } => {
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
            Instruction::ReturnConst { idx } => {
                let value = self.code.constants[idx.get(arg) as usize].clone().into();
                self.unwind_blocks(vm, UnwindReason::Returning { value })
            }
            Instruction::ReturnValue => {
                let value = self.pop_value();
                self.unwind_blocks(vm, UnwindReason::Returning { value })
            }
            Instruction::SetAdd { i } => {
                let item = self.pop_value();
                let obj = self.nth_value(i.get(arg));
                let set: &Py<PySet> = unsafe {
                    // SAFETY: trust compiler
                    obj.downcast_unchecked_ref()
                };
                set.add(item, vm)?;
                Ok(None)
            }
            Instruction::SetExcInfo => {
                // Set the current exception to TOS (for except* handlers)
                // This updates sys.exc_info() so bare 'raise' will reraise the matched exception
                let exc = self.top_value();
                if let Some(exc) = exc.downcast_ref::<PyBaseException>() {
                    vm.set_exception(Some(exc.to_owned()));
                }
                Ok(None)
            }
            Instruction::PushExcInfo => {
                // Stack: [exc] -> [prev_exc, exc]
                let exc = self.pop_value();
                let prev_exc = vm
                    .current_exception()
                    .map(|e| e.into())
                    .unwrap_or_else(|| vm.ctx.none());

                // Set exc as the current exception
                if let Some(exc_ref) = exc.downcast_ref::<PyBaseException>() {
                    vm.set_exception(Some(exc_ref.to_owned()));
                }

                self.push_value(prev_exc);
                self.push_value(exc);
                Ok(None)
            }
            Instruction::CheckExcMatch => {
                // Stack: [exc, type] -> [exc, bool]
                let exc_type = self.pop_value();
                let exc = self.top_value();

                // Validate that exc_type is valid for exception matching
                let result = exc.is_instance(&exc_type, vm)?;
                self.push_value(vm.ctx.new_bool(result).into());
                Ok(None)
            }
            Instruction::Reraise { depth } => {
                // inst(RERAISE, (values[oparg], exc -- values[oparg]))
                //
                // Stack layout: [values..., exc] where len(values) == oparg
                // RERAISE pops exc and oparg additional values from the stack.
                // values[0] is lasti used to set frame->instr_ptr for traceback.
                // We skip the lasti update since RustPython's traceback is already correct.
                let depth_val = depth.get(arg) as usize;

                // Pop exception from TOS
                let exc = self.pop_value();

                // Pop the depth values (lasti and possibly other items like prev_exc)
                for _ in 0..depth_val {
                    self.pop_value();
                }

                if let Some(exc_ref) = exc.downcast_ref::<PyBaseException>() {
                    Err(exc_ref.to_owned())
                } else {
                    // Fallback: use current exception if TOS is not an exception
                    let exc = vm
                        .topmost_exception()
                        .ok_or_else(|| vm.new_runtime_error("No active exception to re-raise"))?;
                    Err(exc)
                }
            }
            Instruction::SetFunctionAttribute { attr } => {
                self.execute_set_function_attribute(vm, attr.get(arg))
            }
            Instruction::SetupAnnotations => self.setup_annotations(vm),
            Instruction::BeforeWith => {
                // TOS: context_manager
                // Result: [..., __exit__, __enter__ result]
                let context_manager = self.pop_value();
                let error_string = || -> String {
                    format!(
                        "'{:.200}' object does not support the context manager protocol",
                        context_manager.class().name(),
                    )
                };

                // Get __exit__ first (before calling __enter__)
                let exit = context_manager
                    .get_attr(identifier!(vm, __exit__), vm)
                    .map_err(|_exc| {
                        vm.new_type_error(format!("{} (missed __exit__ method)", error_string()))
                    })?;

                // Get and call __enter__
                let enter_res = vm
                    .get_special_method(&context_manager, identifier!(vm, __enter__))?
                    .ok_or_else(|| vm.new_type_error(error_string()))?
                    .invoke((), vm)?;

                // Push __exit__ first, then enter result
                self.push_value(exit);
                self.push_value(enter_res);
                Ok(None)
            }
            Instruction::StoreAttr { idx } => self.store_attr(vm, idx.get(arg)),
            Instruction::StoreDeref(i) => {
                let value = self.pop_value();
                self.cells_frees[i.get(arg) as usize].set(Some(value));
                Ok(None)
            }
            Instruction::StoreFast(idx) => {
                let value = self.pop_value();
                self.fastlocals.lock()[idx.get(arg) as usize] = Some(value);
                Ok(None)
            }
            Instruction::StoreFastLoadFast {
                store_idx,
                load_idx,
            } => {
                // Store to one slot and load from another (often the same) - for inlined comprehensions
                let value = self.pop_value();
                let mut locals = self.fastlocals.lock();
                locals[store_idx.get(arg) as usize] = Some(value);
                let load_value = locals[load_idx.get(arg) as usize]
                    .clone()
                    .expect("StoreFastLoadFast: load slot should have value after store");
                drop(locals);
                self.push_value(load_value);
                Ok(None)
            }
            Instruction::StoreGlobal(idx) => {
                let value = self.pop_value();
                self.globals
                    .set_item(self.code.names[idx.get(arg) as usize], value, vm)?;
                Ok(None)
            }
            Instruction::StoreName(idx) => {
                let name = self.code.names[idx.get(arg) as usize];
                let value = self.pop_value();
                self.locals.mapping().ass_subscript(name, Some(value), vm)?;
                Ok(None)
            }
            Instruction::StoreSubscr => self.execute_store_subscript(vm),
            Instruction::Subscript => self.execute_subscript(vm),
            Instruction::Swap { index } => {
                let len = self.state.stack.len();
                debug_assert!(len > 0, "stack underflow in SWAP");
                let i = len - 1; // TOS index
                let index_val = index.get(arg) as usize;
                // CPython: SWAP(n) swaps TOS with PEEK(n) where PEEK(n) = stack_pointer[-n]
                // This means swap TOS with the element at index (len - n)
                debug_assert!(
                    index_val <= len,
                    "SWAP index {} exceeds stack size {}",
                    index_val,
                    len
                );
                let j = len - index_val;
                self.state.stack.swap(i, j);
                Ok(None)
            }
            Instruction::ToBool => {
                let obj = self.pop_value();
                let bool_val = obj.try_to_bool(vm)?;
                self.push_value(vm.ctx.new_bool(bool_val).into());
                Ok(None)
            }
            Instruction::UnpackEx { args } => {
                let args = args.get(arg);
                self.execute_unpack_ex(vm, args.before, args.after)
            }
            Instruction::UnpackSequence { size } => self.unpack_sequence(size.get(arg), vm),
            Instruction::WithExceptStart => {
                // Stack: [..., __exit__, lasti, prev_exc, exc]
                // Call __exit__(type, value, tb) and push result
                // __exit__ is at TOS-3 (below lasti, prev_exc, and exc)
                let exc = vm.current_exception();

                let stack_len = self.state.stack.len();
                let exit = expect_unchecked(
                    self.state.stack[stack_len - 4].clone(),
                    "WithExceptStart: __exit__ is NULL",
                );

                let args = if let Some(ref exc) = exc {
                    vm.split_exception(exc.clone())
                } else {
                    (vm.ctx.none(), vm.ctx.none(), vm.ctx.none())
                };
                let exit_res = exit.call(args, vm)?;
                // Push result on top of stack
                self.push_value(exit_res);

                Ok(None)
            }
            Instruction::YieldValue { arg: oparg } => {
                let value = self.pop_value();
                // arg=0: direct yield (wrapped for async generators)
                // arg=1: yield from await/yield-from (NOT wrapped)
                let wrap = oparg.get(arg) == 0;
                let value = if wrap && self.code.flags.contains(bytecode::CodeFlags::COROUTINE) {
                    PyAsyncGenWrappedValue(value).into_pyobject(vm)
                } else {
                    value
                };
                Ok(Some(ExecutionResult::Yield(value)))
            }
            Instruction::Send { target } => {
                // Stack: (receiver, value) -> (receiver, retval)
                // On StopIteration: replace value with stop value and jump to target
                let exit_label = target.get(arg);
                let val = self.pop_value();
                let receiver = self.top_value();

                match self._send(receiver, val, vm)? {
                    PyIterReturn::Return(value) => {
                        // Value yielded, push it back for YIELD_VALUE
                        // Stack: (receiver, retval)
                        self.push_value(value);
                        Ok(None)
                    }
                    PyIterReturn::StopIteration(value) => {
                        // StopIteration: replace top with stop value, jump to exit
                        // Stack: (receiver, value) - receiver stays, v replaced
                        let value = vm.unwrap_or_none(value);
                        self.push_value(value);
                        self.jump(exit_label);
                        Ok(None)
                    }
                }
            }
            Instruction::EndSend => {
                // Stack: (receiver, value) -> (value)
                // Pops receiver, leaves value
                let value = self.pop_value();
                self.pop_value(); // discard receiver
                self.push_value(value);
                Ok(None)
            }
            Instruction::CleanupThrow => {
                // CLEANUP_THROW: (sub_iter, last_sent_val, exc) -> (None, value) OR re-raise
                // If StopIteration: pop all 3, extract value, push (None, value)
                // Otherwise: pop all 3, return Err(exc) for unwind_blocks to handle
                //
                // Unlike CPython where exception_unwind pops the triple as part of
                // stack cleanup to handler depth, RustPython pops here explicitly
                // and lets unwind_blocks find outer handlers.
                // Compiler sets handler_depth = base + 2 (before exc is pushed).

                // First peek at exc_value (top of stack) without popping
                let exc = self.top_value();

                // Check if it's a StopIteration
                if let Some(exc_ref) = exc.downcast_ref::<PyBaseException>()
                    && exc_ref.fast_isinstance(vm.ctx.exceptions.stop_iteration)
                {
                    // Extract value from StopIteration
                    let value = exc_ref.get_arg(0).unwrap_or_else(|| vm.ctx.none());
                    // Now pop all three
                    self.pop_value(); // exc
                    self.pop_value(); // last_sent_val
                    self.pop_value(); // sub_iter
                    self.push_value(vm.ctx.none());
                    self.push_value(value);
                    return Ok(None);
                }

                // Re-raise other exceptions: pop all three and return Err(exc)
                let exc = self.pop_value(); // exc
                self.pop_value(); // last_sent_val
                self.pop_value(); // sub_iter

                let exc = exc
                    .downcast::<PyBaseException>()
                    .map_err(|_| vm.new_type_error("exception expected".to_owned()))?;
                Err(exc)
            }
            Instruction::UnaryInvert => {
                let a = self.pop_value();
                let value = vm._invert(&a)?;
                self.push_value(value);
                Ok(None)
            }
            Instruction::UnaryNegative => {
                let a = self.pop_value();
                let value = vm._neg(&a)?;
                self.push_value(value);
                Ok(None)
            }
            Instruction::UnaryNot => {
                let obj = self.pop_value();
                let value = obj.try_to_bool(vm)?;
                self.push_value(vm.ctx.new_bool(!value).into());
                Ok(None)
            }
            _ => {
                unreachable!("{instruction:?} instruction should not be executed")
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
        // use exception table for exception handling
        match reason {
            UnwindReason::Raising { exception } => {
                // Look up handler in exception table
                // lasti points to NEXT instruction (already incremented in run loop)
                // The exception occurred at the previous instruction
                // Python uses signed int where INSTR_OFFSET() - 1 = -1 before first instruction
                // We use u32, so check for 0 explicitly (equivalent to CPython's -1)
                if self.lasti() == 0 {
                    // No instruction executed yet, no handler can match
                    return Err(exception);
                }
                let offset = self.lasti() - 1;
                if let Some(entry) =
                    bytecode::find_exception_handler(&self.code.exceptiontable, offset)
                {
                    // 1. Pop stack to entry.depth
                    while self.state.stack.len() > entry.depth as usize {
                        self.state.stack.pop();
                    }

                    // 2. If push_lasti=true (SETUP_CLEANUP), push lasti before exception
                    // pushes lasti as PyLong
                    if entry.push_lasti {
                        self.push_value(vm.ctx.new_int(offset as i32).into());
                    }

                    // 3. Push exception onto stack
                    // always push exception, PUSH_EXC_INFO transforms [exc] -> [prev_exc, exc]
                    // Note: Do NOT call vm.set_exception here! PUSH_EXC_INFO will do it.
                    // PUSH_EXC_INFO needs to get prev_exc from vm.current_exception() BEFORE setting the new one.
                    self.push_value(exception.into());

                    // 4. Jump to handler
                    self.jump(bytecode::Label(entry.target));

                    Ok(None)
                } else {
                    // No handler found, propagate exception
                    Err(exception)
                }
            }
            UnwindReason::Returning { value } => {
                // Clear tracebacks of exceptions in fastlocals to break reference cycles.
                // This is needed because when returning from inside an except block,
                // the exception cleanup code (e = None; del e) is skipped, leaving the
                // exception with a traceback that references this frame, which references
                // the exception in fastlocals, creating a cycle that can't be collected
                // since RustPython doesn't have a tracing GC.
                //
                // We only clear tracebacks of exceptions that:
                // 1. Are not the return value itself (will be needed by caller)
                // 2. Are not the current active exception (still being handled)
                // 3. Have a traceback whose top frame is THIS frame (we created it)
                let current_exc = vm.current_exception();
                let fastlocals = self.fastlocals.lock();
                for obj in fastlocals.iter().flatten() {
                    // Skip if this object is the return value
                    if obj.is(&value) {
                        continue;
                    }
                    if let Ok(exc) = obj.clone().downcast::<PyBaseException>() {
                        // Skip if this is the current active exception
                        if current_exc.as_ref().is_some_and(|cur| exc.is(cur)) {
                            continue;
                        }
                        // Only clear if traceback's top frame is this frame
                        if exc
                            .__traceback__()
                            .is_some_and(|tb| core::ptr::eq::<Py<Frame>>(&*tb.frame, self.object))
                        {
                            exc.set_traceback_typed(None);
                        }
                    }
                }
                drop(fastlocals);
                Ok(Some(ExecutionResult::Return(value)))
            }
            UnwindReason::Break { target } | UnwindReason::Continue { target } => {
                // Break/continue: jump to the target label
                self.jump(target);
                Ok(None)
            }
        }
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

    fn execute_build_slice(
        &mut self,
        vm: &VirtualMachine,
        argc: bytecode::BuildSliceArgCount,
    ) -> FrameResult {
        let step = match argc {
            bytecode::BuildSliceArgCount::Two => None,
            bytecode::BuildSliceArgCount::Three => Some(self.pop_value()),
        };
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
        mapping: &PyObject,
        error_prefix: &str,
        mut key_handler: F,
    ) -> PyResult<()>
    where
        F: FnMut(PyObjectRef) -> PyResult<()>,
    {
        let Some(keys_method) = vm.get_method(mapping.to_owned(), vm.ctx.intern_str("keys")) else {
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
        // Stack: [callable, self_or_null, ...]
        let self_or_null = self.pop_value_opt(); // Option<PyObjectRef>
        let callable = self.pop_value();

        // If self_or_null is Some (not NULL), prepend it to args
        let final_args = if let Some(self_val) = self_or_null {
            // Method call: prepend self to args
            let mut all_args = vec![self_val];
            all_args.extend(args.args);
            FuncArgs {
                args: all_args,
                kwargs: args.kwargs,
            }
        } else {
            // Regular attribute call: self_or_null is NULL
            args
        };

        let value = callable.call(final_args, vm)?;
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
            _ => None,
        };
        let exception = match kind {
            bytecode::RaiseKind::RaiseCause | bytecode::RaiseKind::Raise => {
                ExceptionCtor::try_from_object(vm, self.pop_value())?.instantiate(vm)?
            }
            bytecode::RaiseKind::BareRaise => {
                // RAISE_VARARGS 0: bare `raise` gets exception from VM state
                // This is the current exception set by PUSH_EXC_INFO
                vm.topmost_exception().ok_or_else(|| {
                    vm.new_runtime_error("No active exception to reraise".to_owned())
                })?
            }
            bytecode::RaiseKind::ReraiseFromStack => {
                // RERAISE: gets exception from stack top
                // Used in cleanup blocks where exception is on stack after COPY 3
                let exc = self.pop_value();
                exc.downcast::<PyBaseException>().map_err(|obj| {
                    vm.new_type_error(format!(
                        "exceptions must derive from BaseException, not {}",
                        obj.class().name()
                    ))
                })?
            }
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
            .extend(elements.drain(before + middle..).rev().map(Some));

        let middle_elements = elements.drain(before..).collect();
        let t = vm.ctx.new_list(middle_elements);
        self.push_value(t.into());

        // Lastly the first reversed values:
        self.state
            .stack
            .extend(elements.into_iter().rev().map(Some));

        Ok(None)
    }

    #[inline]
    fn jump(&mut self, label: bytecode::Label) {
        let target_pc = label.0;
        vm_trace!("jump from {:?} to {:?}", self.lasti(), target_pc);
        self.update_lasti(|i| *i = target_pc);
    }

    #[inline]
    fn pop_jump_if(
        &mut self,
        vm: &VirtualMachine,
        target: bytecode::Label,
        flag: bool,
    ) -> FrameResult {
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
        // SET_FUNCTION_ATTRIBUTE sets attributes on a function
        // Stack: [..., attr_value, func] -> [..., func]
        // Stack order: func is at -1, attr_value is at -2

        let func = self.pop_value_opt();
        let attr_value = expect_unchecked(self.replace_top(func), "attr_value must not be null");

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
            bytecode::BinaryOperator::TrueDivide => vm._truediv(a_ref, b_ref),
            bytecode::BinaryOperator::FloorDivide => vm._floordiv(a_ref, b_ref),
            bytecode::BinaryOperator::Remainder => vm._mod(a_ref, b_ref),
            bytecode::BinaryOperator::Lshift => vm._lshift(a_ref, b_ref),
            bytecode::BinaryOperator::Rshift => vm._rshift(a_ref, b_ref),
            bytecode::BinaryOperator::Xor => vm._xor(a_ref, b_ref),
            bytecode::BinaryOperator::Or => vm._or(a_ref, b_ref),
            bytecode::BinaryOperator::And => vm._and(a_ref, b_ref),
            bytecode::BinaryOperator::InplaceSubtract => vm._isub(a_ref, b_ref),
            bytecode::BinaryOperator::InplaceAdd => vm._iadd(a_ref, b_ref),
            bytecode::BinaryOperator::InplaceMultiply => vm._imul(a_ref, b_ref),
            bytecode::BinaryOperator::InplaceMatrixMultiply => vm._imatmul(a_ref, b_ref),
            bytecode::BinaryOperator::InplacePower => {
                vm._ipow(a_ref, b_ref, vm.ctx.none.as_object())
            }
            bytecode::BinaryOperator::InplaceTrueDivide => vm._itruediv(a_ref, b_ref),
            bytecode::BinaryOperator::InplaceFloorDivide => vm._ifloordiv(a_ref, b_ref),
            bytecode::BinaryOperator::InplaceRemainder => vm._imod(a_ref, b_ref),
            bytecode::BinaryOperator::InplaceLshift => vm._ilshift(a_ref, b_ref),
            bytecode::BinaryOperator::InplaceRshift => vm._irshift(a_ref, b_ref),
            bytecode::BinaryOperator::InplaceXor => vm._ixor(a_ref, b_ref),
            bytecode::BinaryOperator::InplaceOr => vm._ior(a_ref, b_ref),
            bytecode::BinaryOperator::InplaceAnd => vm._iand(a_ref, b_ref),
        }?;

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
            core::cmp::Ordering::Equal => {
                // Wrap each element in Some() for Option<PyObjectRef> stack
                self.state
                    .stack
                    .extend(elements.into_iter().rev().map(Some));
                return Ok(None);
            }
            core::cmp::Ordering::Greater => {
                format!("too many values to unpack (expected {size})")
            }
            core::cmp::Ordering::Less => format!(
                "not enough values to unpack (expected {}, got {})",
                size,
                elements.len()
            ),
        };
        Err(vm.new_value_error(msg))
    }

    fn convert_value(
        &mut self,
        conversion: bytecode::ConvertValueOparg,
        vm: &VirtualMachine,
    ) -> FrameResult {
        use bytecode::ConvertValueOparg;
        let value = self.pop_value();
        let value = match conversion {
            ConvertValueOparg::Str => value.str(vm)?.into(),
            ConvertValueOparg::Repr => value.repr(vm)?.into(),
            ConvertValueOparg::Ascii => vm.ctx.new_str(builtins::ascii(value, vm)?).into(),
            ConvertValueOparg::None => value,
        };

        self.push_value(value);
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

    fn load_attr(&mut self, vm: &VirtualMachine, oparg: u32) -> FrameResult {
        let (name_idx, is_method) = bytecode::decode_load_attr_arg(oparg);
        let attr_name = self.code.names[name_idx as usize];
        let parent = self.pop_value();

        if is_method {
            // Method call: push [method, self_or_null]
            let method = PyMethod::get(parent.clone(), attr_name, vm)?;
            match method {
                PyMethod::Function { target: _, func } => {
                    self.push_value(func);
                    self.push_value(parent);
                }
                PyMethod::Attribute(val) => {
                    self.push_value(val);
                    self.push_null();
                }
            }
        } else {
            // Regular attribute access
            let obj = parent.get_attr(attr_name, vm)?;
            self.push_value(obj);
        }
        Ok(None)
    }

    fn load_super_attr(&mut self, vm: &VirtualMachine, oparg: u32) -> FrameResult {
        let (name_idx, load_method, has_class) = bytecode::decode_load_super_attr_arg(oparg);
        let attr_name = self.code.names[name_idx as usize];

        // Stack layout (bottom to top): [super, class, self]
        // Pop in LIFO order: self, class, super
        let self_obj = self.pop_value();
        let class = self.pop_value();
        let global_super = self.pop_value();

        // Create super object - pass args based on has_class flag
        // When super is shadowed, has_class=false means call with 0 args
        let super_obj = if has_class {
            global_super.call((class.clone(), self_obj.clone()), vm)?
        } else {
            global_super.call((), vm)?
        };

        if load_method {
            // Method load: push [method, self_or_null]
            let method = PyMethod::get(super_obj, attr_name, vm)?;
            match method {
                PyMethod::Function { target: _, func } => {
                    self.push_value(func);
                    self.push_value(self_obj);
                }
                PyMethod::Attribute(val) => {
                    self.push_value(val);
                    self.push_null();
                }
            }
        } else {
            // Regular attribute access
            let obj = super_obj.get_attr(attr_name, vm)?;
            self.push_value(obj);
        }
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

    // Block stack functions removed - exception table handles all exception/cleanup

    #[inline]
    #[track_caller] // not a real track_caller but push_value is less useful for debugging
    fn push_value_opt(&mut self, obj: Option<PyObjectRef>) {
        match self.state.stack.try_push(obj) {
            Ok(()) => {}
            Err(_e) => self.fatal("tried to push value onto stack but overflowed max_stackdepth"),
        }
    }

    #[inline]
    #[track_caller]
    fn push_value(&mut self, obj: PyObjectRef) {
        self.push_value_opt(Some(obj));
    }

    #[inline]
    fn push_null(&mut self) {
        self.push_value_opt(None);
    }

    /// Pop a value from the stack, returning None if the stack slot is NULL
    #[inline]
    fn pop_value_opt(&mut self) -> Option<PyObjectRef> {
        match self.state.stack.pop() {
            Some(slot) => slot, // slot is Option<PyObjectRef>
            None => self.fatal("tried to pop from empty stack"),
        }
    }

    #[inline]
    #[track_caller]
    fn pop_value(&mut self) -> PyObjectRef {
        expect_unchecked(
            self.pop_value_opt(),
            "pop value but null found. This is a compiler bug.",
        )
    }

    fn call_intrinsic_1(
        &mut self,
        func: bytecode::IntrinsicFunction1,
        arg: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        match func {
            bytecode::IntrinsicFunction1::Print => {
                let displayhook = vm
                    .sys_module
                    .get_attr("displayhook", vm)
                    .map_err(|_| vm.new_runtime_error("lost sys.displayhook"))?;
                displayhook.call((arg,), vm)
            }
            bytecode::IntrinsicFunction1::ImportStar => {
                // arg is the module object
                self.push_value(arg); // Push module back on stack for import_star
                self.import_star(vm)?;
                Ok(vm.ctx.none())
            }
            bytecode::IntrinsicFunction1::UnaryPositive => vm._pos(&arg),
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

                let name = name.downcast::<crate::builtins::PyStr>().map_err(|_| {
                    vm.new_type_error("TypeAliasType name must be a string".to_owned())
                })?;
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
            bytecode::IntrinsicFunction2::PrepReraiseStar => {
                // arg1 = orig (original exception)
                // arg2 = excs (list of exceptions raised/reraised in except* blocks)
                // Returns: exception to reraise, or None if nothing to reraise
                crate::exceptions::prep_reraise_star(arg1, arg2, vm)
            }
        }
    }

    /// Pop multiple values from the stack. Panics if any slot is NULL.
    fn pop_multiple(&mut self, count: usize) -> impl ExactSizeIterator<Item = PyObjectRef> + '_ {
        let stack_len = self.state.stack.len();
        if count > stack_len {
            let instr = self.code.instructions.get(self.lasti() as usize);
            let op_name = instr
                .map(|i| format!("{:?}", i.op))
                .unwrap_or_else(|| "None".to_string());
            panic!(
                "Stack underflow in pop_multiple: trying to pop {} elements from stack with {} elements. lasti={}, code={}, op={}, source_path={}",
                count,
                stack_len,
                self.lasti(),
                self.code.obj_name,
                op_name,
                self.code.source_path
            );
        }
        self.state.stack.drain(stack_len - count..).map(|obj| {
            expect_unchecked(obj, "pop_multiple but null found. This is a compiler bug.")
        })
    }

    #[inline]
    fn replace_top(&mut self, mut top: Option<PyObjectRef>) -> Option<PyObjectRef> {
        let last = self.state.stack.last_mut().unwrap();
        core::mem::swap(last, &mut top);
        top
    }

    #[inline]
    #[track_caller]
    fn top_value(&self) -> &PyObject {
        match &*self.state.stack {
            [.., Some(last)] => last,
            [.., None] => self.fatal("tried to get top of stack but got NULL"),
            [] => self.fatal("tried to get top of stack but stack is empty"),
        }
    }

    #[inline]
    #[track_caller]
    fn nth_value(&self, depth: u32) -> &PyObject {
        let stack = &self.state.stack;
        match &stack[stack.len() - depth as usize - 1] {
            Some(obj) => obj,
            None => unsafe { std::hint::unreachable_unchecked() },
        }
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
        let stack_str = state.stack.iter().fold(String::new(), |mut s, slot| {
            match slot {
                Some(elem) if elem.downcastable::<Self>() => {
                    s.push_str("\n  > {frame}");
                }
                Some(elem) => {
                    core::fmt::write(&mut s, format_args!("\n  > {elem:?}")).unwrap();
                }
                None => {
                    s.push_str("\n  > NULL");
                }
            }
            s
        });
        // TODO: fix this up
        let locals = self.locals.clone();
        write!(
            f,
            "Frame Object {{ \n Stack:{}\n Locals:{:?}\n}}",
            stack_str,
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

fn expect_unchecked(optional: Option<PyObjectRef>, err_msg: &'static str) -> PyObjectRef {
    if cfg!(debug_assertions) {
        optional.expect(err_msg)
    } else {
        unsafe { optional.unwrap_unchecked() }
    }
}
