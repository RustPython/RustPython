use std::fmt;
#[cfg(feature = "threading")]
use std::sync::atomic;

use indexmap::IndexMap;
use itertools::Itertools;

use crate::builtins;
use crate::builtins::asyncgenerator::PyAsyncGenWrappedValue;
use crate::builtins::code::PyCodeRef;
use crate::builtins::coroutine::PyCoroutine;
use crate::builtins::dict::{PyDict, PyDictRef};
use crate::builtins::function::{PyCell, PyCellRef, PyFunction};
use crate::builtins::generator::PyGenerator;
use crate::builtins::pystr::{self, PyStr, PyStrRef};
use crate::builtins::pytype::PyTypeRef;
use crate::builtins::slice::PySlice;
use crate::builtins::traceback::PyTraceback;
use crate::builtins::tuple::{PyTuple, PyTupleTyped};
use crate::builtins::{list, pybool, set};
use crate::bytecode;
use crate::common::boxvec::BoxVec;
use crate::common::lock::PyMutex;
use crate::coroutine::Coro;
use crate::exceptions::{self, ExceptionCtor, PyBaseExceptionRef};
use crate::function::FuncArgs;
use crate::iterator;
use crate::pyobject::{
    BorrowValue, IdProtocol, ItemProtocol, PyMethod, PyObjectRef, PyRef, PyResult, PyValue,
    TryFromObject, TypeProtocol,
};
use crate::scope::Scope;
use crate::slots::PyComparisonOp;
use crate::vm::VirtualMachine;

#[derive(Clone, Debug)]
struct Block {
    /// The type of block.
    typ: BlockType,
    /// The level of the value stack when the block was entered.
    level: usize,
}

#[derive(Clone, Debug)]
enum BlockType {
    Loop {
        break_target: bytecode::Label,
    },
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
    Break,

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
    pub code: PyCodeRef,

    pub fastlocals: PyMutex<Box<[Option<PyObjectRef>]>>,
    pub(crate) cells_frees: Box<[PyCellRef]>,
    pub locals: PyDictRef,
    pub globals: PyDictRef,
    pub builtins: PyDictRef,

    // on feature=threading, this is a duplicate of FrameState.lasti, but it's faster to do an
    // atomic store than it is to do a fetch_add, for every instruction executed
    /// index of last instruction ran
    pub lasti: Lasti,
    /// tracer function for this frame (usually is None)
    pub trace: PyMutex<PyObjectRef>,
    state: PyMutex<FrameState>,
}

impl PyValue for Frame {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.frame_type
    }
}

// Running a frame can result in one of the below:
pub enum ExecutionResult {
    Return(PyObjectRef),
    Yield(PyObjectRef),
}

impl ExecutionResult {
    /// Extract an ExecutionResult from a PyResult returned from e.g. gen.__next__() or gen.send()
    pub fn from_result(vm: &VirtualMachine, res: PyResult) -> PyResult<Self> {
        match res {
            Ok(val) => Ok(ExecutionResult::Yield(val)),
            Err(err) => {
                if err.isinstance(&vm.ctx.exceptions.stop_iteration) {
                    Ok(ExecutionResult::Return(iterator::stop_iter_value(vm, &err)))
                } else {
                    Err(err)
                }
            }
        }
    }

    /// Turn an ExecutionResult into a PyResult that would be returned from a generator or coroutine
    pub fn into_result(self, async_stopiter: bool, vm: &VirtualMachine) -> PyResult {
        match self {
            ExecutionResult::Yield(value) => Ok(value),
            ExecutionResult::Return(value) => {
                let stop_iteration = if async_stopiter {
                    vm.ctx.exceptions.stop_async_iteration.clone()
                } else {
                    vm.ctx.exceptions.stop_iteration.clone()
                };
                let args = if vm.is_none(&value) {
                    vec![]
                } else {
                    vec![value]
                };
                Err(vm.new_exception(stop_iteration, args))
            }
        }
    }
}

/// A valid execution result, or an exception
pub type FrameResult = PyResult<Option<ExecutionResult>>;

impl Frame {
    pub(crate) fn new(
        code: PyCodeRef,
        scope: Scope,
        builtins: PyDictRef,
        closure: &[PyCellRef],
        vm: &VirtualMachine,
    ) -> Frame {
        let cells_frees = std::iter::repeat_with(|| PyCell::default().into_ref(vm))
            .take(code.cellvars.len())
            .chain(closure.iter().cloned())
            .collect();

        let state = FrameState {
            stack: BoxVec::new(code.max_stacksize as usize),
            blocks: Vec::new(),
            #[cfg(feature = "threading")]
            lasti: 0,
        };

        Frame {
            fastlocals: PyMutex::new(vec![None; code.varnames.len()].into_boxed_slice()),
            cells_frees,
            locals: scope.locals,
            globals: scope.globals,
            builtins,
            code,
            lasti: Lasti::new(0),
            state: PyMutex::new(state),
            trace: PyMutex::new(vm.ctx.none()),
        }
    }
}

impl FrameRef {
    #[inline]
    fn with_exec<R>(&self, f: impl FnOnce(ExecutingFrame) -> R) -> R {
        let mut state = self.state.lock();
        let exec = ExecutingFrame {
            code: &self.code,
            fastlocals: &self.fastlocals,
            cells_frees: &self.cells_frees,
            locals: &self.locals,
            globals: &self.globals,
            builtins: &self.builtins,
            lasti: &self.lasti,
            object: &self,
            state: &mut state,
        };
        f(exec)
    }

    pub fn locals(&self, vm: &VirtualMachine) -> PyResult<PyDictRef> {
        let locals = &self.locals;
        let code = &**self.code;
        let map = &code.varnames;
        let j = std::cmp::min(map.len(), code.varnames.len());
        if !code.varnames.is_empty() {
            let fastlocals = self.fastlocals.lock();
            for (k, v) in itertools::zip(&map[..j], &**fastlocals) {
                if let Some(v) = v {
                    locals.set_item(k.clone(), v.clone(), vm)?;
                } else {
                    match locals.del_item(k.clone(), vm) {
                        Ok(()) => {}
                        Err(e) if e.isinstance(&vm.ctx.exceptions.key_error) => {}
                        Err(e) => return Err(e),
                    }
                }
            }
        }
        if !code.cellvars.is_empty() || !code.freevars.is_empty() {
            let map_to_dict = |keys: &[PyStrRef], values: &[PyCellRef]| {
                for (k, v) in itertools::zip(keys, values) {
                    if let Some(v) = v.get() {
                        locals.set_item(k.clone(), v, vm)?;
                    } else {
                        match locals.del_item(k.clone(), vm) {
                            Ok(()) => {}
                            Err(e) if e.isinstance(&vm.ctx.exceptions.key_error) => {}
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

    pub fn current_location(&self) -> bytecode::Location {
        self.code.locations[self.lasti() as usize - 1]
    }

    pub fn yield_from_target(&self) -> Option<PyObjectRef> {
        self.with_exec(|exec| exec.yield_from_target().cloned())
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
}

/// An executing frame; essentially just a struct to combine the immutable data outside the mutex
/// with the mutable data inside
struct ExecutingFrame<'a> {
    code: &'a PyCodeRef,
    fastlocals: &'a PyMutex<Box<[Option<PyObjectRef>]>>,
    cells_frees: &'a [PyCellRef],
    locals: &'a PyDictRef,
    globals: &'a PyDictRef,
    builtins: &'a PyDictRef,
    object: &'a FrameRef,
    lasti: &'a Lasti,
    state: &'a mut FrameState,
}

impl fmt::Debug for ExecutingFrame<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
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
    fn lasti(&self) -> u32 {
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
        flame_guard!(format!("Frame::run({})", self.code.obj_name));
        // Execute until return or exception:
        let instrs = &self.code.instructions;
        loop {
            let idx = self.lasti() as usize;
            self.update_lasti(|i| *i += 1);
            let instr = &instrs[idx];
            let result = self.execute_instruction(instr, vm);
            match result {
                Ok(None) => continue,
                Ok(Some(value)) => {
                    break Ok(value);
                }
                // Instruction raised an exception
                Err(exception) => {
                    // 1. Extract traceback from exception's '__traceback__' attr.
                    // 2. Add new entry with current execution position (filename, lineno, code_object) to traceback.
                    // 3. Unwind block stack till appropriate handler is found.

                    let loc = self.code.locations[idx];

                    let next = exception.traceback();

                    let new_traceback =
                        PyTraceback::new(next, self.object.clone(), self.lasti(), loc.row());
                    vm_trace!("Adding to traceback: {:?} {:?}", new_traceback, loc.row());
                    exception.set_traceback(Some(new_traceback.into_ref(vm)));

                    vm.contextualize_exception(&exception);

                    match self.unwind_blocks(vm, UnwindReason::Raising { exception }) {
                        Ok(None) => continue,
                        Ok(Some(result)) => {
                            break Ok(result);
                        }
                        Err(exception) => {
                            // TODO: append line number to traceback?
                            // traceback.append();
                            break Err(exception);
                        }
                    }
                }
            }
        }
    }

    fn yield_from_target(&self) -> Option<&PyObjectRef> {
        if let Some(bytecode::Instruction::YieldFrom) =
            self.code.instructions.get(self.lasti() as usize)
        {
            Some(self.last_value_ref())
        } else {
            None
        }
    }

    /// Ok(Err(e)) means that an error ocurred while calling throw() and the generator should try
    /// sending it
    fn gen_throw(
        &mut self,
        vm: &VirtualMachine,
        exc_type: PyObjectRef,
        exc_val: PyObjectRef,
        exc_tb: PyObjectRef,
    ) -> PyResult<ExecutionResult> {
        if let Some(coro) = self.yield_from_target() {
            use crate::pyobject::Either;
            // borrow checker shenanigans - we only need to use exc_type/val/tb if the following
            // variable is Some
            let thrower = if let Some(coro) = self.builtin_coro(coro) {
                Some(Either::A(coro))
            } else if let Some(meth) = vm.get_attribute_opt(coro.clone(), "throw")? {
                Some(Either::B(meth))
            } else {
                None
            };
            if let Some(thrower) = thrower {
                let ret = match thrower {
                    Either::A(coro) => coro.throw(exc_type, exc_val, exc_tb, vm),
                    Either::B(meth) => vm.invoke(&meth, vec![exc_type, exc_val, exc_tb]),
                };
                return ret.map(ExecutionResult::Yield).or_else(|err| {
                    self.pop_value();
                    self.update_lasti(|i| *i += 1);
                    if err.isinstance(&vm.ctx.exceptions.stop_iteration) {
                        let val = iterator::stop_iter_value(vm, &err);
                        self.push_value(val);
                        self.run(vm)
                    } else {
                        let (ty, val, tb) = exceptions::split(err, vm);
                        self.gen_throw(vm, ty, val, tb)
                    }
                });
            }
        }
        let exception = exceptions::normalize(exc_type, exc_val, exc_tb, vm)?;
        match self.unwind_blocks(vm, UnwindReason::Raising { exception }) {
            Ok(None) => self.run(vm),
            Ok(Some(result)) => Ok(result),
            Err(exception) => Err(exception),
        }
    }

    fn unbound_cell_exception(&self, i: usize, vm: &VirtualMachine) -> PyBaseExceptionRef {
        if let Some(name) = self.code.cellvars.get(i) {
            vm.new_exception_msg(
                vm.ctx.exceptions.unbound_local_error.clone(),
                format!("local variable '{}' referenced before assignment", name),
            )
        } else {
            let name = &self.code.freevars[i - self.code.cellvars.len()];
            vm.new_name_error(format!(
                "free variable '{}' referenced before assignment in enclosing scope",
                name
            ))
        }
    }

    /// Execute a single instruction.
    #[inline(always)]
    fn execute_instruction(
        &mut self,
        instruction: &bytecode::Instruction,
        vm: &VirtualMachine,
    ) -> FrameResult {
        vm.check_signals()?;

        flame_guard!(format!("Frame::execute_instruction({:?})", instruction));

        #[cfg(feature = "vm-tracing-logging")]
        {
            trace!("=======");
            /* TODO:
            for frame in self.frames.iter() {
                trace!("  {:?}", frame);
            }
            */
            trace!("  {:#?}", self);
            trace!("  Executing op code: {:?}", instruction);
            trace!("=======");
        }

        match instruction {
            bytecode::Instruction::LoadConst { idx } => {
                self.push_value(self.code.constants[*idx as usize].0.clone());
                Ok(None)
            }
            bytecode::Instruction::ImportName { idx } => {
                self.import(vm, Some(self.code.names[*idx as usize].clone()))
            }
            bytecode::Instruction::ImportNameless => self.import(vm, None),
            bytecode::Instruction::ImportStar => self.import_star(vm),
            bytecode::Instruction::ImportFrom { idx } => {
                let obj = self.import_from(vm, *idx)?;
                self.push_value(obj);
                Ok(None)
            }
            bytecode::Instruction::LoadFast(idx) => {
                let idx = *idx as usize;
                let x = self.fastlocals.lock()[idx].clone().ok_or_else(|| {
                    vm.new_exception_msg(
                        vm.ctx.exceptions.unbound_local_error.clone(),
                        format!(
                            "local variable '{}' referenced before assignment",
                            self.code.varnames[idx]
                        ),
                    )
                })?;
                self.push_value(x);
                Ok(None)
            }
            bytecode::Instruction::LoadNameAny(idx) => {
                let name = &self.code.names[*idx as usize];
                let x = self.locals.get_item_option(name.clone(), vm)?;
                let x = match x {
                    Some(x) => x,
                    None => self.load_global_or_builtin(name, vm)?,
                };
                self.push_value(x);
                Ok(None)
            }
            bytecode::Instruction::LoadGlobal(idx) => {
                let name = &self.code.names[*idx as usize];
                let x = self.load_global_or_builtin(name, vm)?;
                self.push_value(x);
                Ok(None)
            }
            bytecode::Instruction::LoadDeref(i) => {
                let i = *i as usize;
                let x = self.cells_frees[i]
                    .get()
                    .ok_or_else(|| self.unbound_cell_exception(i, vm))?;
                self.push_value(x);
                Ok(None)
            }
            bytecode::Instruction::LoadClassDeref(i) => {
                let i = *i as usize;
                let name = self.code.freevars[i - self.code.cellvars.len()].clone();
                let value = if let Some(value) = self.locals.get_item_option(name, vm)? {
                    value
                } else {
                    self.cells_frees[i]
                        .get()
                        .ok_or_else(|| self.unbound_cell_exception(i, vm))?
                };
                self.push_value(value);
                Ok(None)
            }
            bytecode::Instruction::StoreFast(idx) => {
                let value = self.pop_value();
                self.fastlocals.lock()[*idx as usize] = Some(value);
                Ok(None)
            }
            bytecode::Instruction::StoreLocal(idx) => {
                let value = self.pop_value();
                self.locals
                    .set_item(self.code.names[*idx as usize].clone(), value, vm)?;
                Ok(None)
            }
            bytecode::Instruction::StoreGlobal(idx) => {
                let value = self.pop_value();
                self.globals
                    .set_item(self.code.names[*idx as usize].clone(), value, vm)?;
                Ok(None)
            }
            bytecode::Instruction::StoreDeref(i) => {
                let value = self.pop_value();
                self.cells_frees[*i as usize].set(Some(value));
                Ok(None)
            }
            bytecode::Instruction::DeleteFast(idx) => {
                self.fastlocals.lock()[*idx as usize] = None;
                Ok(None)
            }
            bytecode::Instruction::DeleteLocal(idx) => {
                let name = &self.code.names[*idx as usize];
                match self.locals.del_item(name.clone(), vm) {
                    Ok(()) => {}
                    Err(e) if e.isinstance(&vm.ctx.exceptions.key_error) => {
                        return Err(vm.new_name_error(format!("name '{}' is not defined", name)))
                    }
                    Err(e) => return Err(e),
                }
                Ok(None)
            }
            bytecode::Instruction::DeleteGlobal(idx) => {
                let name = &self.code.names[*idx as usize];
                match self.globals.del_item(name.clone(), vm) {
                    Ok(()) => {}
                    Err(e) if e.isinstance(&vm.ctx.exceptions.key_error) => {
                        return Err(vm.new_name_error(format!("name '{}' is not defined", name)))
                    }
                    Err(e) => return Err(e),
                }
                Ok(None)
            }
            bytecode::Instruction::DeleteDeref(i) => {
                self.cells_frees[*i as usize].set(None);
                Ok(None)
            }
            bytecode::Instruction::LoadClosure(i) => {
                let value = self.cells_frees[*i as usize].clone();
                self.push_value(value.into_object());
                Ok(None)
            }
            bytecode::Instruction::Subscript => self.execute_subscript(vm),
            bytecode::Instruction::StoreSubscript => self.execute_store_subscript(vm),
            bytecode::Instruction::DeleteSubscript => self.execute_delete_subscript(vm),
            bytecode::Instruction::Pop => {
                // Pop value from stack and ignore.
                self.pop_value();
                Ok(None)
            }
            bytecode::Instruction::Duplicate => {
                // Duplicate top of stack
                let value = self.pop_value();
                self.push_value(value.clone());
                self.push_value(value);
                Ok(None)
            }
            bytecode::Instruction::Rotate { amount } => self.execute_rotate(*amount),
            bytecode::Instruction::BuildString { size } => {
                let s = self
                    .pop_multiple(*size as usize)
                    .as_slice()
                    .iter()
                    .map(|pyobj| pystr::borrow_value(&pyobj))
                    .collect::<String>();
                let str_obj = vm.ctx.new_str(s);
                self.push_value(str_obj);
                Ok(None)
            }
            bytecode::Instruction::BuildList { size, unpack } => {
                let elements = self.get_elements(vm, *size as usize, *unpack)?;
                let list_obj = vm.ctx.new_list(elements);
                self.push_value(list_obj);
                Ok(None)
            }
            bytecode::Instruction::BuildSet { size, unpack } => {
                let set = vm.ctx.new_set();
                {
                    let elements = self.pop_multiple(*size as usize);
                    if *unpack {
                        for element in elements {
                            vm.map_iterable_object(&element, |x| set.add(x, vm))??;
                        }
                    } else {
                        for element in elements {
                            set.add(element, vm)?;
                        }
                    }
                }
                self.push_value(set.into_object());
                Ok(None)
            }
            bytecode::Instruction::BuildTuple { size, unpack } => {
                let elements = self.get_elements(vm, *size as usize, *unpack)?;
                let list_obj = vm.ctx.new_tuple(elements);
                self.push_value(list_obj);
                Ok(None)
            }
            bytecode::Instruction::BuildMap {
                size,
                unpack,
                for_call,
            } => self.execute_build_map(vm, *size, *unpack, *for_call),
            bytecode::Instruction::BuildSlice { step } => self.execute_build_slice(vm, *step),
            bytecode::Instruction::ListAppend { i } => {
                let list_obj = self.nth_value(*i);
                let item = self.pop_value();
                list::PyListRef::try_from_object(vm, list_obj)?.append(item);
                Ok(None)
            }
            bytecode::Instruction::SetAdd { i } => {
                let set_obj = self.nth_value(*i);
                let item = self.pop_value();
                set::PySetRef::try_from_object(vm, set_obj)?.add(item, vm)?;
                Ok(None)
            }
            bytecode::Instruction::MapAdd { i } => {
                let dict_obj = self.nth_value(*i + 1);
                let key = self.pop_value();
                let value = self.pop_value();
                PyDictRef::try_from_object(vm, dict_obj)?.set_item(key, value, vm)?;
                Ok(None)
            }
            bytecode::Instruction::MapAddRev { i } => {
                // change order of evalutio of key and value to support Py3.8 Named expressions in dict comprehension
                let dict_obj = self.nth_value(*i + 1);
                let value = self.pop_value();
                let key = self.pop_value();
                PyDictRef::try_from_object(vm, dict_obj)?.set_item(key, value, vm)?;
                Ok(None)
            }
            bytecode::Instruction::BinaryOperation { op } => self.execute_binop(vm, *op),
            bytecode::Instruction::BinaryOperationInplace { op } => {
                self.execute_binop_inplace(vm, *op)
            }
            bytecode::Instruction::LoadAttr { idx } => self.load_attr(vm, *idx),
            bytecode::Instruction::StoreAttr { idx } => self.store_attr(vm, *idx),
            bytecode::Instruction::DeleteAttr { idx } => self.delete_attr(vm, *idx),
            bytecode::Instruction::UnaryOperation { ref op } => self.execute_unop(vm, op),
            bytecode::Instruction::CompareOperation { ref op } => self.execute_compare(vm, op),
            bytecode::Instruction::ReturnValue => {
                let value = self.pop_value();
                self.unwind_blocks(vm, UnwindReason::Returning { value })
            }
            bytecode::Instruction::YieldValue => {
                let value = self.pop_value();
                let value = if self.code.flags.contains(bytecode::CodeFlags::IS_COROUTINE) {
                    PyAsyncGenWrappedValue(value).into_object(vm)
                } else {
                    value
                };
                Ok(Some(ExecutionResult::Yield(value)))
            }
            bytecode::Instruction::YieldFrom => self.execute_yield_from(vm),
            bytecode::Instruction::SetupAnnotation => {
                if !self.locals.contains_key("__annotations__", vm) {
                    self.locals
                        .set_item("__annotations__", vm.ctx.new_dict().into_object(), vm)?;
                }
                Ok(None)
            }
            bytecode::Instruction::SetupLoop { break_target } => {
                self.push_block(BlockType::Loop {
                    break_target: *break_target,
                });
                Ok(None)
            }
            bytecode::Instruction::SetupExcept { handler } => {
                self.push_block(BlockType::TryExcept { handler: *handler });
                Ok(None)
            }
            bytecode::Instruction::SetupFinally { handler } => {
                self.push_block(BlockType::Finally { handler: *handler });
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
                let exit = vm.get_attribute(context_manager.clone(), "__exit__")?;
                self.push_value(exit);
                // Call enter:
                let enter_res = vm.call_special_method(context_manager, "__enter__", ())?;
                self.push_block(BlockType::Finally { handler: *end });
                self.push_value(enter_res);
                Ok(None)
            }
            bytecode::Instruction::BeforeAsyncWith => {
                let mgr = self.pop_value();
                let aexit = vm.get_attribute(mgr.clone(), "__aexit__")?;
                self.push_value(aexit);
                let aenter_res = vm.call_special_method(mgr, "__aenter__", ())?;
                self.push_value(aenter_res);

                Ok(None)
            }
            bytecode::Instruction::SetupAsyncWith { end } => {
                let enter_res = self.pop_value();
                self.push_block(BlockType::Finally { handler: *end });
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

                let exit = self.pop_value();

                let args = if let Some(exc) = exc {
                    exceptions::split(exc, vm)
                } else {
                    (vm.ctx.none(), vm.ctx.none(), vm.ctx.none())
                };
                let exit_res = vm.invoke(&exit, args)?;
                self.push_value(exit_res);

                Ok(None)
            }
            bytecode::Instruction::WithCleanupFinish => {
                let block = self.pop_block();
                let (reason, prev_exc) = match block.typ {
                    BlockType::FinallyHandler { reason, prev_exc } => (reason, prev_exc),
                    _ => self.fatal("WithCleanupFinish expects a FinallyHandler block on stack"),
                };

                let suppress_exception = pybool::boolval(vm, self.pop_value())?;

                vm.set_exception(prev_exc);

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
                let iter_obj = iterator::get_iter(vm, iterated_obj)?;
                self.push_value(iter_obj);
                Ok(None)
            }
            bytecode::Instruction::GetAwaitable => {
                let awaited_obj = self.pop_value();
                let awaitable = if awaited_obj.payload_is::<PyCoroutine>() {
                    awaited_obj
                } else {
                    let await_method =
                        vm.get_method_or_type_error(awaited_obj.clone(), "__await__", || {
                            format!(
                                "object {} can't be used in 'await' expression",
                                awaited_obj.class().name,
                            )
                        })?;
                    vm.invoke(&await_method, ())?
                };
                self.push_value(awaitable);
                Ok(None)
            }
            bytecode::Instruction::GetAIter => {
                let aiterable = self.pop_value();
                let aiter = vm.call_special_method(aiterable, "__aiter__", ())?;
                self.push_value(aiter);
                Ok(None)
            }
            bytecode::Instruction::GetANext => {
                let aiter = self.last_value();
                let awaitable = vm.call_special_method(aiter, "__anext__", ())?;
                let awaitable = if awaitable.payload_is::<PyCoroutine>() {
                    awaitable
                } else {
                    vm.call_special_method(awaitable, "__await__", ())?
                };
                self.push_value(awaitable);
                Ok(None)
            }
            bytecode::Instruction::EndAsyncFor => {
                let exc = self.pop_value();
                self.pop_value(); // async iterator we were calling __anext__ on
                if exc.isinstance(&vm.ctx.exceptions.stop_async_iteration) {
                    vm.take_exception().expect("Should have exception in stack");
                    Ok(None)
                } else {
                    Err(exc.downcast().unwrap())
                }
            }
            bytecode::Instruction::ForIter { target } => self.execute_for_iter(vm, *target),
            bytecode::Instruction::MakeFunction(flags) => self.execute_make_function(vm, *flags),
            bytecode::Instruction::CallFunctionPositional { nargs } => {
                let args = self.collect_positional_args(*nargs);
                self.execute_call(args, vm)
            }
            bytecode::Instruction::CallFunctionKeyword { nargs } => {
                let args = self.collect_keyword_args(*nargs);
                self.execute_call(args, vm)
            }
            bytecode::Instruction::CallFunctionEx { has_kwargs } => {
                let args = self.collect_ex_args(vm, *has_kwargs)?;
                self.execute_call(args, vm)
            }
            bytecode::Instruction::LoadMethod { idx } => {
                let obj = self.pop_value();
                let method_name = self.code.names[*idx as usize].clone();
                let method = PyMethod::get(obj, method_name, vm)?;
                let (target, is_method, func) = match method {
                    PyMethod::Function { target, func } => (target, true, func),
                    PyMethod::Attribute(val) => (vm.ctx.none(), false, val),
                };
                // TODO: figure out a better way to communicate PyMethod::Attribute - CPython uses
                // target==NULL, maybe we could use a sentinel value or something?
                self.push_value(target);
                self.push_value(vm.ctx.new_bool(is_method));
                self.push_value(func);
                Ok(None)
            }
            bytecode::Instruction::CallMethodPositional { nargs } => {
                let args = self.collect_positional_args(*nargs);
                self.execute_method_call(args, vm)
            }
            bytecode::Instruction::CallMethodKeyword { nargs } => {
                let args = self.collect_keyword_args(*nargs);
                self.execute_method_call(args, vm)
            }
            bytecode::Instruction::CallMethodEx { has_kwargs } => {
                let args = self.collect_ex_args(vm, *has_kwargs)?;
                self.execute_method_call(args, vm)
            }
            bytecode::Instruction::Jump { target } => {
                self.jump(*target);
                Ok(None)
            }
            bytecode::Instruction::JumpIfTrue { target } => {
                let obj = self.pop_value();
                let value = pybool::boolval(vm, obj)?;
                if value {
                    self.jump(*target);
                }
                Ok(None)
            }

            bytecode::Instruction::JumpIfFalse { target } => {
                let obj = self.pop_value();
                let value = pybool::boolval(vm, obj)?;
                if !value {
                    self.jump(*target);
                }
                Ok(None)
            }

            bytecode::Instruction::JumpIfTrueOrPop { target } => {
                let obj = self.last_value();
                let value = pybool::boolval(vm, obj)?;
                if value {
                    self.jump(*target);
                } else {
                    self.pop_value();
                }
                Ok(None)
            }

            bytecode::Instruction::JumpIfFalseOrPop { target } => {
                let obj = self.last_value();
                let value = pybool::boolval(vm, obj)?;
                if !value {
                    self.jump(*target);
                } else {
                    self.pop_value();
                }
                Ok(None)
            }

            bytecode::Instruction::Raise { kind } => self.execute_raise(vm, *kind),

            bytecode::Instruction::Break => self.unwind_blocks(vm, UnwindReason::Break),
            bytecode::Instruction::Continue { target } => {
                self.unwind_blocks(vm, UnwindReason::Continue { target: *target })
            }
            bytecode::Instruction::PrintExpr => {
                let expr = self.pop_value();

                let displayhook = vm
                    .get_attribute(vm.sys_module.clone(), "displayhook")
                    .map_err(|_| vm.new_runtime_error("lost sys.displayhook".to_owned()))?;
                vm.invoke(&displayhook, (expr,))?;

                Ok(None)
            }
            bytecode::Instruction::LoadBuildClass => {
                self.push_value(vm.get_attribute(vm.builtins.clone(), "__build_class__")?);
                Ok(None)
            }
            bytecode::Instruction::UnpackSequence { size } => {
                let value = self.pop_value();
                let elements = vm.extract_elements(&value).map_err(|e| {
                    if e.class().is(&vm.ctx.exceptions.type_error) {
                        vm.new_type_error(format!(
                            "cannot unpack non-iterable {} object",
                            value.class().name
                        ))
                    } else {
                        e
                    }
                })?;
                let msg = match elements.len().cmp(&(*size as usize)) {
                    std::cmp::Ordering::Equal => {
                        self.state.stack.extend(elements.into_iter().rev());
                        None
                    }
                    std::cmp::Ordering::Greater => {
                        Some(format!("too many values to unpack (expected {})", size))
                    }
                    std::cmp::Ordering::Less => Some(format!(
                        "not enough values to unpack (expected {}, got {})",
                        size,
                        elements.len()
                    )),
                };
                if let Some(msg) = msg {
                    Err(vm.new_value_error(msg))
                } else {
                    Ok(None)
                }
            }
            bytecode::Instruction::UnpackEx { before, after } => {
                self.execute_unpack_ex(vm, *before, *after)
            }
            bytecode::Instruction::FormatValue { conversion } => {
                use bytecode::ConversionFlag;
                let value = self.pop_value();
                let value = match conversion {
                    ConversionFlag::Str => vm.to_str(&value)?.into_object(),
                    ConversionFlag::Repr => vm.to_repr(&value)?.into_object(),
                    ConversionFlag::Ascii => vm.ctx.new_str(builtins::ascii(value, vm)?),
                    ConversionFlag::None => value,
                };

                let spec = self.pop_value();
                let formatted = vm.call_special_method(value, "__format__", (spec,))?;
                self.push_value(formatted);
                Ok(None)
            }
            bytecode::Instruction::PopException {} => {
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
                self.state.stack[stack_len - *amount as usize..stack_len].reverse();
                Ok(None)
            }
        }
    }

    #[inline]
    fn load_global_or_builtin(&self, name: &PyStrRef, vm: &VirtualMachine) -> PyResult {
        self.globals
            .get_chain(self.builtins, name.clone(), vm)?
            .ok_or_else(|| vm.new_name_error(format!("name '{}' is not defined", name)))
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn get_elements(
        &mut self,
        vm: &VirtualMachine,
        size: usize,
        unpack: bool,
    ) -> PyResult<Vec<PyObjectRef>> {
        let elements = self.pop_multiple(size);
        if unpack {
            let mut result: Vec<PyObjectRef> = vec![];
            for element in elements {
                result.extend(vm.extract_elements(&element)?);
            }
            Ok(result)
        } else {
            Ok(elements.collect())
        }
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn import(&mut self, vm: &VirtualMachine, module: Option<PyStrRef>) -> FrameResult {
        let module = module.unwrap_or_else(|| PyStr::from("").into_ref(vm));
        let from_list = <Option<PyTupleTyped<PyStrRef>>>::try_from_object(vm, self.pop_value())?;
        let level = usize::try_from_object(vm, self.pop_value())?;

        let module = vm.import(module, from_list, level)?;

        self.push_value(module);
        Ok(None)
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn import_from(&mut self, vm: &VirtualMachine, idx: bytecode::NameIdx) -> PyResult {
        let module = self.last_value();
        let name = &self.code.names[idx as usize];
        let err = || vm.new_import_error(format!("cannot import name '{}'", name), name.clone());
        // Load attribute, and transform any error into import error.
        if let Some(obj) = vm.get_attribute_opt(module.clone(), name.clone())? {
            return Ok(obj);
        }
        // fallback to importing '{module.__name__}.{name}' from sys.modules
        let mod_name = vm.get_attribute(module, "__name__").map_err(|_| err())?;
        let mod_name = mod_name.downcast::<PyStr>().map_err(|_| err())?;
        let full_mod_name = format!("{}.{}", mod_name, name);
        let sys_modules = vm
            .get_attribute(vm.sys_module.clone(), "modules")
            .map_err(|_| err())?;
        sys_modules.get_item(full_mod_name, vm).map_err(|_| err())
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn import_star(&mut self, vm: &VirtualMachine) -> FrameResult {
        let module = self.pop_value();

        // Grab all the names from the module and put them in the context
        if let Some(dict) = module.dict() {
            let filter_pred: Box<dyn Fn(&str) -> bool> =
                if let Ok(all) = dict.get_item("__all__", vm) {
                    let all: Vec<PyStrRef> = vm.extract_elements(&all)?;
                    let all: Vec<String> = all
                        .into_iter()
                        .map(|name| name.as_ref().to_owned())
                        .collect();
                    Box::new(move |name| all.contains(&name.to_owned()))
                } else {
                    Box::new(|name| !name.starts_with('_'))
                };
            for (k, v) in &dict {
                let k = PyStrRef::try_from_object(vm, k)?;
                if filter_pred(k.borrow_value()) {
                    self.locals.set_item(k, v, vm)?;
                }
            }
        }
        Ok(None)
    }

    /// Unwind blocks.
    /// The reason for unwinding gives a hint on what to do when
    /// unwinding a block.
    /// Optionally returns an exception.
    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn unwind_blocks(&mut self, vm: &VirtualMachine, reason: UnwindReason) -> FrameResult {
        // First unwind all existing blocks on the block stack:
        while let Some(block) = self.current_block() {
            match block.typ {
                BlockType::Loop { break_target } => match reason {
                    UnwindReason::Break => {
                        self.pop_block();
                        self.jump(break_target);
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
                        self.push_value(exception.into_object());
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

    fn execute_rotate(&mut self, amount: u32) -> FrameResult {
        // Shuffles top of stack amount down
        if amount < 2 {
            self.fatal("Can only rotate two or more values");
        }

        let mut values = Vec::new();

        // Pop all values from stack:
        for _ in 0..amount {
            values.push(self.pop_value());
        }

        // Push top of stack back first:
        self.push_value(values.remove(0));

        // Push other value back in order:
        for value in values.into_iter().rev() {
            self.push_value(value);
        }
        Ok(None)
    }

    fn execute_subscript(&mut self, vm: &VirtualMachine) -> FrameResult {
        let b_ref = self.pop_value();
        let a_ref = self.pop_value();
        let value = a_ref.get_item(b_ref, vm)?;
        self.push_value(value);
        Ok(None)
    }

    fn execute_store_subscript(&mut self, vm: &VirtualMachine) -> FrameResult {
        let idx = self.pop_value();
        let obj = self.pop_value();
        let value = self.pop_value();
        obj.set_item(idx, value, vm)?;
        Ok(None)
    }

    fn execute_delete_subscript(&mut self, vm: &VirtualMachine) -> FrameResult {
        let idx = self.pop_value();
        let obj = self.pop_value();
        obj.del_item(idx, vm)?;
        Ok(None)
    }

    #[allow(clippy::collapsible_if)]
    fn execute_build_map(
        &mut self,
        vm: &VirtualMachine,
        size: u32,
        unpack: bool,
        for_call: bool,
    ) -> FrameResult {
        let size = size as usize;
        let map_obj = vm.ctx.new_dict();
        if unpack {
            for obj in self.pop_multiple(size) {
                // Take all key-value pairs from the dict:
                let dict: PyDictRef = obj.downcast().map_err(|obj| {
                    vm.new_type_error(format!("'{}' object is not a mapping", obj.class().name))
                })?;
                for (key, value) in dict {
                    if for_call {
                        if map_obj.contains_key(key.clone(), vm) {
                            let key_repr = vm.to_repr(&key)?;
                            let msg = format!(
                                "got multiple values for keyword argument {}",
                                key_repr.borrow_value()
                            );
                            return Err(vm.new_type_error(msg));
                        }
                    }
                    map_obj.set_item(key, value, vm).unwrap();
                }
            }
        } else {
            for (key, value) in self.pop_multiple(2 * size).tuples() {
                map_obj.set_item(key, value, vm).unwrap();
            }
        }

        self.push_value(map_obj.into_object());
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
        .into_ref(vm);
        self.push_value(obj.into_object());
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
            .borrow_value()
            .iter()
            .map(|pyobj| pystr::clone_value(pyobj));
        FuncArgs::with_kwargs_names(args, kwarg_names)
    }

    fn collect_ex_args(&mut self, vm: &VirtualMachine, has_kwargs: bool) -> PyResult<FuncArgs> {
        let kwargs = if has_kwargs {
            let kw_dict: PyDictRef = self.pop_value().downcast().map_err(|_| {
                // TODO: check collections.abc.Mapping
                vm.new_type_error("Kwargs must be a dict.".to_owned())
            })?;
            let mut kwargs = IndexMap::new();
            for (key, value) in kw_dict.into_iter() {
                let key = key
                    .payload_if_subclass::<pystr::PyStr>(vm)
                    .ok_or_else(|| vm.new_type_error("keywords must be strings".to_owned()))?;
                kwargs.insert(key.borrow_value().to_owned(), value);
            }
            kwargs
        } else {
            IndexMap::new()
        };
        let args = self.pop_value();
        let args = vm.extract_elements(&args)?;
        Ok(FuncArgs { args, kwargs })
    }

    #[inline]
    fn execute_call(&mut self, args: FuncArgs, vm: &VirtualMachine) -> FrameResult {
        let func_ref = self.pop_value();
        let value = vm.invoke(&func_ref, args)?;
        self.push_value(value);
        Ok(None)
    }

    #[inline]
    fn execute_method_call(&mut self, args: FuncArgs, vm: &VirtualMachine) -> FrameResult {
        let func = self.pop_value();
        let is_method = self.pop_value().is(&vm.ctx.true_value);
        let target = self.pop_value();
        let method = if is_method {
            PyMethod::Function { target, func }
        } else {
            drop(target); // should be None
            PyMethod::Attribute(func)
        };
        let value = method.invoke(args, vm)?;
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
                        vm.new_type_error(
                            "exception causes must derive from BaseException".to_owned(),
                        )
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
                .ok_or_else(|| vm.new_runtime_error("No active exception to reraise".to_owned()))?,
        };
        info!("Exception raised: {:?} with cause: {:?}", exception, cause);
        if let Some(cause) = cause {
            exception.set_cause(cause);
        }
        Err(exception)
    }

    fn builtin_coro<'a>(&self, coro: &'a PyObjectRef) -> Option<&'a Coro> {
        match_class!(match coro {
            ref g @ PyGenerator => Some(g.as_coro()),
            ref c @ PyCoroutine => Some(c.as_coro()),
            _ => None,
        })
    }

    fn _send(&self, coro: &PyObjectRef, val: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match self.builtin_coro(coro) {
            Some(coro) => coro.send(val, vm),
            None if vm.is_none(&val) => iterator::call_next(vm, coro),
            None => {
                let meth = vm.get_attribute(coro.clone(), "send")?;
                vm.invoke(&meth, (val,))
            }
        }
    }

    fn execute_yield_from(&mut self, vm: &VirtualMachine) -> FrameResult {
        // Value send into iterator:
        let val = self.pop_value();

        let coro = self.last_value_ref();

        let result = self._send(coro, val, vm);

        let result = ExecutionResult::from_result(vm, result)?;

        match result {
            ExecutionResult::Yield(value) => {
                // Set back program counter:
                self.update_lasti(|i| *i -= 1);
                Ok(Some(ExecutionResult::Yield(value)))
            }
            ExecutionResult::Return(value) => {
                self.pop_value();
                self.push_value(value);
                Ok(None)
            }
        }
    }

    fn execute_unpack_ex(&mut self, vm: &VirtualMachine, before: u8, after: u8) -> FrameResult {
        let (before, after) = (before as usize, after as usize);
        let value = self.pop_value();
        let mut elements = vm.extract_elements::<PyObjectRef>(&value)?;
        let min_expected = before + after;

        let middle = elements.len().checked_sub(min_expected).ok_or_else(|| {
            vm.new_value_error(format!(
                "not enough values to unpack (expected at least {}, got {})",
                min_expected,
                elements.len()
            ))
        })?;

        // Elements on stack from right-to-left:
        self.state
            .stack
            .extend(elements.drain(before + middle..).rev());

        let middle_elements = elements.drain(before..).collect();
        let t = vm.ctx.new_list(middle_elements);
        self.push_value(t);

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

    /// The top of stack contains the iterator, lets push it forward
    fn execute_for_iter(&mut self, vm: &VirtualMachine, target: bytecode::Label) -> FrameResult {
        let top_of_stack = self.last_value();
        let next_obj = iterator::get_next_object(vm, &top_of_stack);

        // Check the next object:
        match next_obj {
            Ok(Some(value)) => {
                self.push_value(value);
                Ok(None)
            }
            Ok(None) => {
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
    fn execute_make_function(
        &mut self,
        vm: &VirtualMachine,
        flags: bytecode::MakeFunctionFlags,
    ) -> FrameResult {
        let qualified_name = self
            .pop_value()
            .downcast::<PyStr>()
            .expect("qualified name to be a string");
        let code_obj: PyCodeRef = self
            .pop_value()
            .downcast()
            .expect("Second to top value on the stack must be a code object");

        let closure = if flags.contains(bytecode::MakeFunctionFlags::CLOSURE) {
            Some(PyTupleTyped::try_from_object(vm, self.pop_value()).unwrap())
        } else {
            None
        };

        let annotations = if flags.contains(bytecode::MakeFunctionFlags::ANNOTATIONS) {
            self.pop_value()
        } else {
            vm.ctx.new_dict().into_object()
        };

        let kw_only_defaults = if flags.contains(bytecode::MakeFunctionFlags::KW_ONLY_DEFAULTS) {
            Some(
                self.pop_value()
                    .downcast::<PyDict>()
                    .expect("Stack value for keyword only defaults expected to be a dict"),
            )
        } else {
            None
        };

        let defaults = if flags.contains(bytecode::MakeFunctionFlags::DEFAULTS) {
            Some(
                self.pop_value()
                    .downcast::<PyTuple>()
                    .expect("Stack value for defaults expected to be a tuple"),
            )
        } else {
            None
        };

        // pop argc arguments
        // argument: name, args, globals
        // let scope = self.scope.clone();
        let func_obj = PyFunction::new(
            code_obj,
            self.globals.clone(),
            closure,
            defaults,
            kw_only_defaults,
        )
        .into_object(vm);

        vm.set_attr(&func_obj, "__doc__", vm.ctx.none())?;

        let name = qualified_name
            .borrow_value()
            .split('.')
            .next_back()
            .unwrap();
        vm.set_attr(&func_obj, "__name__", vm.ctx.new_str(name))?;
        vm.set_attr(&func_obj, "__qualname__", qualified_name)?;
        let module = vm.unwrap_or_none(self.globals.get_item_option("__name__", vm)?);
        vm.set_attr(&func_obj, "__module__", module)?;
        vm.set_attr(&func_obj, "__annotations__", annotations)?;

        self.push_value(func_obj);
        Ok(None)
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn execute_binop(&mut self, vm: &VirtualMachine, op: bytecode::BinaryOperator) -> FrameResult {
        let b_ref = &self.pop_value();
        let a_ref = &self.pop_value();
        let value = match op {
            bytecode::BinaryOperator::Subtract => vm._sub(a_ref, b_ref),
            bytecode::BinaryOperator::Add => vm._add(a_ref, b_ref),
            bytecode::BinaryOperator::Multiply => vm._mul(a_ref, b_ref),
            bytecode::BinaryOperator::MatrixMultiply => vm._matmul(a_ref, b_ref),
            bytecode::BinaryOperator::Power => vm._pow(a_ref, b_ref),
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
    fn execute_binop_inplace(
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
            bytecode::BinaryOperator::Power => vm._ipow(a_ref, b_ref),
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
    fn execute_unop(&mut self, vm: &VirtualMachine, op: &bytecode::UnaryOperator) -> FrameResult {
        let a = self.pop_value();
        let value = match *op {
            bytecode::UnaryOperator::Minus => vm
                .get_special_method(a, "__neg__")?
                .map_err(|a| {
                    vm.new_type_error(format!(
                        "bad operand type for unary -: '{}'",
                        a.class().name
                    ))
                })?
                .invoke((), vm)?,
            bytecode::UnaryOperator::Plus => vm
                .get_special_method(a, "__pos__")?
                .map_err(|a| {
                    vm.new_type_error(format!(
                        "bad operand type for unary +: '{}'",
                        a.class().name
                    ))
                })?
                .invoke((), vm)?,
            bytecode::UnaryOperator::Invert => vm
                .get_special_method(a, "__invert__")?
                .map_err(|a| {
                    vm.new_type_error(format!(
                        "bad operand type for unary ~: '{}'",
                        a.class().name
                    ))
                })?
                .invoke((), vm)?,
            bytecode::UnaryOperator::Not => {
                let value = pybool::boolval(vm, a)?;
                vm.ctx.new_bool(!value)
            }
        };
        self.push_value(value);
        Ok(None)
    }

    fn _id(&self, a: PyObjectRef) -> usize {
        a.get_id()
    }

    fn _in(
        &self,
        vm: &VirtualMachine,
        needle: PyObjectRef,
        haystack: PyObjectRef,
    ) -> PyResult<bool> {
        let found = vm._membership(haystack, needle)?;
        pybool::boolval(vm, found)
    }

    fn _not_in(
        &self,
        vm: &VirtualMachine,
        needle: PyObjectRef,
        haystack: PyObjectRef,
    ) -> PyResult<bool> {
        let found = vm._membership(haystack, needle)?;
        Ok(!pybool::boolval(vm, found)?)
    }

    fn _is(&self, a: PyObjectRef, b: PyObjectRef) -> bool {
        // Pointer equal:
        a.is(&b)
    }

    fn _is_not(&self, a: PyObjectRef, b: PyObjectRef) -> bool {
        !a.is(&b)
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn execute_compare(
        &mut self,
        vm: &VirtualMachine,
        op: &bytecode::ComparisonOperator,
    ) -> FrameResult {
        let b = self.pop_value();
        let a = self.pop_value();
        let value = match *op {
            bytecode::ComparisonOperator::Equal => vm.obj_cmp(a, b, PyComparisonOp::Eq)?,
            bytecode::ComparisonOperator::NotEqual => vm.obj_cmp(a, b, PyComparisonOp::Ne)?,
            bytecode::ComparisonOperator::Less => vm.obj_cmp(a, b, PyComparisonOp::Lt)?,
            bytecode::ComparisonOperator::LessOrEqual => vm.obj_cmp(a, b, PyComparisonOp::Le)?,
            bytecode::ComparisonOperator::Greater => vm.obj_cmp(a, b, PyComparisonOp::Gt)?,
            bytecode::ComparisonOperator::GreaterOrEqual => vm.obj_cmp(a, b, PyComparisonOp::Ge)?,
            bytecode::ComparisonOperator::Is => vm.ctx.new_bool(self._is(a, b)),
            bytecode::ComparisonOperator::IsNot => vm.ctx.new_bool(self._is_not(a, b)),
            bytecode::ComparisonOperator::In => vm.ctx.new_bool(self._in(vm, a, b)?),
            bytecode::ComparisonOperator::NotIn => vm.ctx.new_bool(self._not_in(vm, a, b)?),
            bytecode::ComparisonOperator::ExceptionMatch => {
                vm.ctx.new_bool(builtins::isinstance(a, b, vm)?)
            }
        };

        self.push_value(value);
        Ok(None)
    }

    fn load_attr(&mut self, vm: &VirtualMachine, attr: bytecode::NameIdx) -> FrameResult {
        let attr_name = self.code.names[attr as usize].clone();
        let parent = self.pop_value();
        let obj = vm.get_attribute(parent, attr_name)?;
        self.push_value(obj);
        Ok(None)
    }

    fn store_attr(&mut self, vm: &VirtualMachine, attr: bytecode::NameIdx) -> FrameResult {
        let attr_name = self.code.names[attr as usize].clone();
        let parent = self.pop_value();
        let value = self.pop_value();
        vm.set_attr(&parent, attr_name, value)?;
        Ok(None)
    }

    fn delete_attr(&mut self, vm: &VirtualMachine, attr: bytecode::NameIdx) -> FrameResult {
        let attr_name = self.code.names[attr as usize].clone().into_object();
        let parent = self.pop_value();
        vm.del_attr(&parent, attr_name)?;
        Ok(None)
    }

    fn push_block(&mut self, typ: BlockType) {
        self.state.blocks.push(Block {
            typ,
            level: self.state.stack.len(),
        });
    }

    fn pop_block(&mut self) -> Block {
        let block = self.state.blocks.pop().expect("No more blocks to pop!");
        self.state.stack.truncate(block.level);
        block
    }

    fn current_block(&self) -> Option<Block> {
        self.state.blocks.last().cloned()
    }

    fn push_value(&mut self, obj: PyObjectRef) {
        match self.state.stack.try_push(obj) {
            Ok(()) => {}
            Err(_e) => self.fatal("tried to push value onto stack but overflowed max_stacksize"),
        }
    }

    fn pop_value(&mut self) -> PyObjectRef {
        match self.state.stack.pop() {
            Some(x) => x,
            None => self.fatal("tried to pop value but there was nothing on the stack"),
        }
    }

    fn pop_multiple(&mut self, count: usize) -> crate::common::boxvec::Drain<PyObjectRef> {
        let stack_len = self.state.stack.len();
        self.state.stack.drain(stack_len - count..stack_len)
    }

    fn last_value(&self) -> PyObjectRef {
        self.last_value_ref().clone()
    }

    #[inline]
    fn last_value_ref(&self) -> &PyObjectRef {
        match &*self.state.stack {
            [.., last] => last,
            [] => self.fatal("tried to get top of stack but stack is empty"),
        }
    }

    fn nth_value(&self, depth: u32) -> PyObjectRef {
        self.state.stack[self.state.stack.len() - depth as usize - 1].clone()
    }

    // redox still has an old nightly, and edition 2021 won't be out for a while
    #[allow(non_fmt_panic)]
    #[cold]
    #[inline(never)]
    fn fatal(&self, msg: &'static str) -> ! {
        dbg!(self);
        panic!(msg)
    }
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let state = self.state.lock();
        let stack_str = state
            .stack
            .iter()
            .map(|elem| {
                if elem.payload_is::<Frame>() {
                    "\n  > {frame}".to_owned()
                } else {
                    format!("\n  > {:?}", elem)
                }
            })
            .collect::<String>();
        let block_str = state
            .blocks
            .iter()
            .map(|elem| format!("\n  > {:?}", elem))
            .collect::<String>();
        // TODO: fix this up
        let dict = self.locals.clone();
        let local_str = dict
            .into_iter()
            .map(|elem| format!("\n  {:?} = {:?}", elem.0, elem.1))
            .collect::<String>();
        write!(
            f,
            "Frame Object {{ \n Stack:{}\n Blocks:{}\n Locals:{}\n}}",
            stack_str, block_str, local_str
        )
    }
}
