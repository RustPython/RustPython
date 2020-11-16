use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};

use indexmap::IndexMap;
use itertools::Itertools;

use crate::builtins;
use crate::builtins::asyncgenerator::PyAsyncGenWrappedValue;
use crate::builtins::code::PyCodeRef;
use crate::builtins::coroutine::PyCoroutine;
use crate::builtins::dict::{PyDict, PyDictRef};
use crate::builtins::generator::PyGenerator;
use crate::builtins::pystr::{self, PyStr, PyStrRef};
use crate::builtins::pytype::PyTypeRef;
use crate::builtins::slice::PySlice;
use crate::builtins::traceback::PyTraceback;
use crate::builtins::tuple::PyTuple;
use crate::builtins::{list, pybool, set};
use crate::bytecode;
use crate::common::lock::PyMutex;
use crate::coroutine::Coro;
use crate::exceptions::{self, ExceptionCtor, PyBaseExceptionRef};
use crate::function::FuncArgs;
use crate::iterator;
use crate::pyobject::{
    BorrowValue, IdProtocol, ItemProtocol, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject,
    TypeProtocol,
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
        start: bytecode::Label,
        end: bytecode::Label,
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
    },
    ExceptHandler,
}

pub type FrameRef = PyRef<Frame>;

/// The reason why we might be unwinding a block.
/// This could be return of function, exception being
/// raised, a break or continue being hit, etc..
#[derive(Clone, Debug)]
enum UnwindReason {
    /// We are returning a value from a return statement.
    Returning { value: PyObjectRef },

    /// We hit an exception, so unwind any try-except and finally blocks.
    Raising { exception: PyBaseExceptionRef },

    // NoWorries,
    /// We are unwinding blocks, since we hit break
    Break,

    /// We are unwinding blocks since we hit a continue statements.
    Continue,
}

#[derive(Debug)]
struct FrameState {
    // We need 1 stack per frame
    /// The main data frame of the stack machine
    stack: Vec<PyObjectRef>,
    /// Block frames, for controlling loops and exceptions
    blocks: Vec<Block>,
}

#[pyclass(module = false, name = "frame")]
pub struct Frame {
    pub code: PyCodeRef,
    pub scope: Scope,
    /// index of last instruction ran
    pub lasti: AtomicUsize,
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
                    iterator::stop_iter_value(vm, &err).map(ExecutionResult::Return)
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
    pub fn new(code: PyCodeRef, scope: Scope, vm: &VirtualMachine) -> Frame {
        //populate the globals and locals
        //TODO: This is wrong, check https://github.com/nedbat/byterun/blob/31e6c4a8212c35b5157919abff43a7daa0f377c6/byterun/pyvm2.py#L95
        /*
        let globals = match globals {
            Some(g) => g,
            None => HashMap::new(),
        };
        */
        // let locals = globals;
        // locals.extend(callargs);

        Frame {
            code,
            scope,
            lasti: AtomicUsize::new(0),
            state: PyMutex::new(FrameState {
                stack: Vec::new(),
                blocks: Vec::new(),
            }),
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
            scope: &self.scope,
            lasti: &self.lasti,
            object: &self,
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
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<ExecutionResult> {
        self.with_exec(|mut exec| {
            exec.push_value(value);
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
        self.code.locations[self.lasti.load(Ordering::Relaxed) - 1]
    }

    pub fn yield_from_target(&self) -> Option<PyObjectRef> {
        self.with_exec(|exec| exec.yield_from_target())
    }

    pub fn lasti(&self) -> usize {
        self.lasti.load(Ordering::Relaxed)
    }
}

/// An executing frame; essentially just a struct to combine the immutable data outside the mutex
/// with the mutable data inside
struct ExecutingFrame<'a> {
    code: &'a PyCodeRef,
    scope: &'a Scope,
    object: &'a FrameRef,
    lasti: &'a AtomicUsize,
    state: &'a mut FrameState,
}

impl fmt::Debug for ExecutingFrame<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("ExecutingFrame")
            .field("code", self.code)
            .field("scope", self.scope)
            .field("lasti", self.lasti)
            .field("state", self.state)
            .finish()
    }
}

impl ExecutingFrame<'_> {
    fn run(&mut self, vm: &VirtualMachine) -> PyResult<ExecutionResult> {
        flame_guard!(format!("Frame::run({})", self.code.obj_name));
        // Execute until return or exception:
        loop {
            let idx = self.lasti.fetch_add(1, Ordering::Relaxed);
            let loc = self.code.locations[idx];
            let instr = &self.code.instructions[idx];
            let result = self.execute_instruction(instr, vm);
            match result {
                Ok(None) => {}
                Ok(Some(value)) => {
                    break Ok(value);
                }
                // Instruction raised an exception
                Err(exception) => {
                    // 1. Extract traceback from exception's '__traceback__' attr.
                    // 2. Add new entry with current execution position (filename, lineno, code_object) to traceback.
                    // 3. Unwind block stack till appropriate handler is found.

                    let next = exception.traceback();

                    let new_traceback =
                        PyTraceback::new(next, self.object.clone(), self.lasti(), loc.row());
                    vm_trace!("Adding to traceback: {:?} {:?}", new_traceback, loc.row());
                    exception.set_traceback(Some(new_traceback.into_ref(vm)));

                    match self.unwind_blocks(vm, UnwindReason::Raising { exception }) {
                        Ok(None) => {}
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

    fn yield_from_target(&self) -> Option<PyObjectRef> {
        if let Some(bytecode::Instruction::YieldFrom) = self.code.instructions.get(self.lasti()) {
            Some(self.last_value())
        } else {
            None
        }
    }

    fn gen_throw(
        &mut self,
        vm: &VirtualMachine,
        exc_type: PyObjectRef,
        exc_val: PyObjectRef,
        exc_tb: PyObjectRef,
    ) -> PyResult<ExecutionResult> {
        if let Some(coro) = self.yield_from_target() {
            let res = match self.builtin_coro(&coro) {
                Some(coro) => coro.throw(exc_type, exc_val, exc_tb, vm),
                None => vm.call_method(&coro, "throw", vec![exc_type, exc_val, exc_tb]),
            };
            res.or_else(|err| {
                self.pop_value();
                self.lasti.fetch_add(1, Ordering::Relaxed);
                let val = iterator::stop_iter_value(vm, &err)?;
                self._send(coro, val, vm)
            })
            .map(ExecutionResult::Yield)
        } else {
            let exception = exceptions::normalize(exc_type, exc_val, exc_tb, vm)?;
            match self.unwind_blocks(vm, UnwindReason::Raising { exception }) {
                Ok(None) => self.run(vm),
                Ok(Some(result)) => Ok(result),
                Err(exception) => Err(exception),
            }
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
                self.push_value(self.code.constants[*idx].0.clone());
                Ok(None)
            }
            bytecode::Instruction::Import {
                name_idx,
                symbols_idx,
                level,
            } => self.import(vm, *name_idx, symbols_idx, *level),
            bytecode::Instruction::ImportStar => self.import_star(vm),
            bytecode::Instruction::ImportFrom { idx } => self.import_from(vm, *idx),
            bytecode::Instruction::LoadName { idx, scope } => self.load_name(vm, *idx, *scope),
            bytecode::Instruction::StoreName { idx, scope } => self.store_name(vm, *idx, *scope),
            bytecode::Instruction::DeleteName { idx } => self.delete_name(vm, *idx),
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
                    .pop_multiple(*size)
                    .into_iter()
                    .map(|pyobj| pystr::clone_value(&pyobj))
                    .collect::<String>();
                let str_obj = vm.ctx.new_str(s);
                self.push_value(str_obj);
                Ok(None)
            }
            bytecode::Instruction::BuildList { size, unpack } => {
                let elements = self.get_elements(vm, *size, *unpack)?;
                let list_obj = vm.ctx.new_list(elements);
                self.push_value(list_obj);
                Ok(None)
            }
            bytecode::Instruction::BuildSet { size, unpack } => {
                let elements = self.get_elements(vm, *size, *unpack)?;
                let set = vm.ctx.new_set();
                for item in elements {
                    set.add(item, vm)?;
                }
                self.push_value(set.into_object());
                Ok(None)
            }
            bytecode::Instruction::BuildTuple { size, unpack } => {
                let elements = self.get_elements(vm, *size, *unpack)?;
                let list_obj = vm.ctx.new_tuple(elements);
                self.push_value(list_obj);
                Ok(None)
            }
            bytecode::Instruction::BuildMap {
                size,
                unpack,
                for_call,
            } => self.execute_build_map(vm, *size, *unpack, *for_call),
            bytecode::Instruction::BuildSlice { size } => self.execute_build_slice(vm, *size),
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
            bytecode::Instruction::BinaryOperation { ref op, inplace } => {
                self.execute_binop(vm, op, *inplace)
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
                let locals = self.scope.get_locals();
                if !locals.contains_key("__annotations__", vm) {
                    locals.set_item("__annotations__", vm.ctx.new_dict().into_object(), vm)?;
                }
                Ok(None)
            }
            bytecode::Instruction::SetupLoop { start, end } => {
                self.push_block(BlockType::Loop {
                    start: *start,
                    end: *end,
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
                self.push_block(BlockType::FinallyHandler { reason: None });
                Ok(None)
            }
            bytecode::Instruction::EndFinally => {
                // Pop the finally handler from the stack, and recall
                // what was the reason we were in this finally clause.
                let block = self.pop_block();

                if let BlockType::FinallyHandler { reason } = block.typ {
                    if let Some(reason) = reason {
                        self.unwind_blocks(vm, reason)
                    } else {
                        Ok(None)
                    }
                } else {
                    panic!(
                        "Block type must be finally handler when reaching EndFinally instruction!"
                    );
                }
            }
            bytecode::Instruction::SetupWith { end } => {
                let context_manager = self.pop_value();
                let exit = vm.get_attribute(context_manager.clone(), "__exit__")?;
                self.push_value(exit);
                // Call enter:
                let enter_res = vm.call_method(&context_manager, "__enter__", ())?;
                self.push_block(BlockType::Finally { handler: *end });
                self.push_value(enter_res);
                Ok(None)
            }
            bytecode::Instruction::BeforeAsyncWith => {
                let mgr = self.pop_value();
                let aexit = vm.get_attribute(mgr.clone(), "__aexit__")?;
                self.push_value(aexit);
                let aenter_res = vm.call_method(&mgr, "__aenter__", ())?;
                self.push_value(aenter_res);

                Ok(None)
            }
            bytecode::Instruction::SetupAsyncWith { end } => {
                self.push_block(BlockType::Finally { handler: *end });
                Ok(None)
            }
            bytecode::Instruction::WithCleanupStart => {
                let block = self.current_block().unwrap();
                let reason = match block.typ {
                    BlockType::FinallyHandler { reason } => reason,
                    _ => panic!("WithCleanupStart expects a FinallyHandler block on stack"),
                };
                let exc = reason.and_then(|reason| match reason {
                    UnwindReason::Raising { exception } => Some(exception),
                    _ => None,
                });

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
                let reason = match block.typ {
                    BlockType::FinallyHandler { reason } => reason,
                    _ => panic!("WithCleanupFinish expects a FinallyHandler block on stack"),
                };

                let suppress_exception = pybool::boolval(vm, self.pop_value())?;
                if suppress_exception {
                    // suppress exception
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
                let aiter = vm.call_method(&aiterable, "__aiter__", ())?;
                self.push_value(aiter);
                Ok(None)
            }
            bytecode::Instruction::GetANext => {
                let aiter = self.last_value();
                let awaitable = vm.call_method(&aiter, "__anext__", ())?;
                let awaitable = if awaitable.payload_is::<PyCoroutine>() {
                    awaitable
                } else {
                    vm.call_method(&awaitable, "__await__", ())?
                };
                self.push_value(awaitable);
                Ok(None)
            }
            bytecode::Instruction::ForIter { target } => self.execute_for_iter(vm, *target),
            bytecode::Instruction::MakeFunction => self.execute_make_function(vm),
            bytecode::Instruction::CallFunction { typ } => self.execute_call_function(vm, typ),
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

            bytecode::Instruction::Raise { argc } => self.execute_raise(vm, *argc),

            bytecode::Instruction::Break => self.unwind_blocks(vm, UnwindReason::Break),
            bytecode::Instruction::Continue => self.unwind_blocks(vm, UnwindReason::Continue),
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
                let msg = match elements.len().cmp(size) {
                    std::cmp::Ordering::Equal => {
                        for element in elements.into_iter().rev() {
                            self.push_value(element);
                        }
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
                use bytecode::ConversionFlag::*;
                let value = match conversion {
                    Some(Str) => vm.to_str(&self.pop_value())?.into_object(),
                    Some(Repr) => vm.to_repr(&self.pop_value())?.into_object(),
                    Some(Ascii) => vm.ctx.new_str(builtins::ascii(self.pop_value(), vm)?),
                    None => self.pop_value(),
                };

                let spec = self.pop_value();
                let formatted = vm.call_method(&value, "__format__", (spec,))?;
                self.push_value(formatted);
                Ok(None)
            }
            bytecode::Instruction::PopException {} => {
                let block = self.pop_block();
                if let BlockType::ExceptHandler = block.typ {
                    vm.pop_exception().expect("Should have exception in stack");
                    Ok(None)
                } else {
                    panic!("Block type must be ExceptHandler here.")
                }
            }
            bytecode::Instruction::Reverse { amount } => {
                let stack_len = self.state.stack.len();
                self.state.stack[stack_len - amount..stack_len].reverse();
                Ok(None)
            }
        }
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
            Ok(elements)
        }
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn import(
        &mut self,
        vm: &VirtualMachine,
        module: Option<bytecode::NameIdx>,
        symbols: &[bytecode::NameIdx],
        level: usize,
    ) -> FrameResult {
        let module = match module {
            Some(idx) => self.code.names[idx].borrow_value(),
            None => "",
        };
        let from_list = symbols
            .iter()
            .map(|&idx| self.code.names[idx].clone())
            .collect::<Vec<_>>();
        let module = vm.import(&module, &from_list, level)?;

        self.push_value(module);
        Ok(None)
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn import_from(&mut self, vm: &VirtualMachine, idx: bytecode::NameIdx) -> FrameResult {
        let module = self.last_value();
        let name = &self.code.names[idx];
        // Load attribute, and transform any error into import error.
        let obj = vm.get_attribute(module, name.clone()).map_err(|_| {
            vm.new_import_error(format!("cannot import name '{}'", name), name.clone())
        })?;
        self.push_value(obj);
        Ok(None)
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
                let k = k.as_ref();
                if filter_pred(k) {
                    self.scope.store_name(&vm, k, v);
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
                BlockType::Loop { start, end } => match &reason {
                    UnwindReason::Break => {
                        self.pop_block();
                        self.jump(end);
                        return Ok(None);
                    }
                    UnwindReason::Continue => {
                        self.jump(start);
                        return Ok(None);
                    }
                    _ => {
                        self.pop_block();
                    }
                },
                BlockType::Finally { handler } => {
                    self.pop_block();
                    self.push_block(BlockType::FinallyHandler {
                        reason: Some(reason),
                    });
                    self.jump(handler);
                    return Ok(None);
                }
                BlockType::TryExcept { handler } => {
                    self.pop_block();
                    if let UnwindReason::Raising { exception } = &reason {
                        self.push_block(BlockType::ExceptHandler {});
                        self.push_value(exception.clone().into_object());
                        vm.push_exception(exception.clone());
                        self.jump(handler);
                        return Ok(None);
                    }
                }
                BlockType::FinallyHandler {
                    reason: finally_reason,
                } => {
                    if let Some(UnwindReason::Raising { exception }) = finally_reason {
                        let finally_exc = exception;
                        if let UnwindReason::Raising { exception } = &reason {
                            exception.set_context(Some(finally_exc))
                        }
                    }
                    self.pop_block();
                }
                BlockType::ExceptHandler => {
                    self.pop_block();
                    vm.pop_exception().expect("Should have exception in stack");
                }
            }
        }

        // We do not have any more blocks to unwind. Inspect the reason we are here:
        match reason {
            UnwindReason::Raising { exception } => Err(exception),
            UnwindReason::Returning { value } => Ok(Some(ExecutionResult::Return(value))),
            UnwindReason::Break | UnwindReason::Continue => {
                panic!("Internal error: break or continue must occur within a loop block.")
            } // UnwindReason::NoWorries => Ok(None),
        }
    }

    fn store_name(
        &mut self,
        vm: &VirtualMachine,
        idx: bytecode::NameIdx,
        name_scope: bytecode::NameScope,
    ) -> FrameResult {
        let name = self.code.names[idx].clone();
        let obj = self.pop_value();
        match name_scope {
            bytecode::NameScope::Global => {
                self.scope.store_global(vm, name, obj);
            }
            bytecode::NameScope::NonLocal => {
                self.scope.store_cell(vm, name, obj);
            }
            bytecode::NameScope::Local => {
                self.scope.store_name(vm, name, obj);
            }
            bytecode::NameScope::Free => {
                self.scope.store_name(vm, name, obj);
            }
        }
        Ok(None)
    }

    fn delete_name(&self, vm: &VirtualMachine, idx: bytecode::NameIdx) -> FrameResult {
        let name = &self.code.names[idx];
        match self.scope.delete_name(vm, name.clone()) {
            Ok(_) => Ok(None),
            Err(_) => Err(vm.new_name_error(format!("name '{}' is not defined", name))),
        }
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    #[inline]
    fn load_name(
        &mut self,
        vm: &VirtualMachine,
        idx: bytecode::NameIdx,
        name_scope: bytecode::NameScope,
    ) -> FrameResult {
        let name = &self.code.names[idx];
        let optional_value = match name_scope {
            bytecode::NameScope::Global => self.scope.load_global(vm, name.clone()),
            bytecode::NameScope::NonLocal => self.scope.load_cell(vm, name.clone()),
            bytecode::NameScope::Local => self.scope.load_local(vm, name.clone()),
            bytecode::NameScope::Free => self.scope.load_name(vm, name.clone()),
        };

        let value = optional_value
            .ok_or_else(|| vm.new_name_error(format!("name '{}' is not defined", name)))?;
        self.push_value(value);
        Ok(None)
    }

    fn execute_rotate(&mut self, amount: usize) -> FrameResult {
        // Shuffles top of stack amount down
        if amount < 2 {
            panic!("Can only rotate two or more values");
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
        size: usize,
        unpack: bool,
        for_call: bool,
    ) -> FrameResult {
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
            for (key, value) in self.pop_multiple(2 * size).into_iter().tuples() {
                map_obj.set_item(key, value, vm).unwrap();
            }
        }

        self.push_value(map_obj.into_object());
        Ok(None)
    }

    fn execute_build_slice(&mut self, vm: &VirtualMachine, size: usize) -> FrameResult {
        assert!(size == 2 || size == 3);

        let step = if size == 3 {
            Some(self.pop_value())
        } else {
            None
        };
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

    fn execute_call_function(
        &mut self,
        vm: &VirtualMachine,
        typ: &bytecode::CallType,
    ) -> FrameResult {
        let args = match typ {
            bytecode::CallType::Positional(count) => {
                let args: Vec<PyObjectRef> = self.pop_multiple(*count);
                FuncArgs {
                    args,
                    kwargs: IndexMap::new(),
                }
            }
            bytecode::CallType::Keyword(count) => {
                let kwarg_names = self.pop_value();
                let args: Vec<PyObjectRef> = self.pop_multiple(*count);

                let kwarg_names = vm
                    .extract_elements(&kwarg_names)?
                    .iter()
                    .map(|pyobj| pystr::clone_value(pyobj))
                    .collect();
                FuncArgs::with_kwargs_names(args, kwarg_names)
            }
            bytecode::CallType::Ex(has_kwargs) => {
                let kwargs = if *has_kwargs {
                    let kw_dict: PyDictRef = self.pop_value().downcast().map_err(|_| {
                        // TODO: check collections.abc.Mapping
                        vm.new_type_error("Kwargs must be a dict.".to_owned())
                    })?;
                    let mut kwargs = IndexMap::new();
                    for (key, value) in kw_dict.into_iter() {
                        let key = key.payload_if_subclass::<pystr::PyStr>(vm).ok_or_else(|| {
                            vm.new_type_error("keywords must be strings".to_owned())
                        })?;
                        kwargs.insert(key.borrow_value().to_owned(), value);
                    }
                    kwargs
                } else {
                    IndexMap::new()
                };
                let args = self.pop_value();
                let args = vm.extract_elements(&args)?;
                FuncArgs { args, kwargs }
            }
        };

        // Call function:
        let func_ref = self.pop_value();
        let value = vm.invoke(&func_ref, args)?;
        self.push_value(value);
        Ok(None)
    }

    fn execute_raise(&mut self, vm: &VirtualMachine, argc: usize) -> FrameResult {
        let cause = match argc {
            2 => {
                let val = self.pop_value();
                Some(if vm.is_none(&val) {
                    // if the cause arg is none, we clear the cause
                    None
                } else {
                    // if the cause arg is an exception, we overwrite it
                    Some(ExceptionCtor::try_from_object(vm, val)?.instantiate(vm)?)
                })
            }
            // if there's no cause arg, we keep the cause as is
            _ => None,
        };
        let exception = match argc {
            0 => vm
                .current_exception()
                .ok_or_else(|| vm.new_runtime_error("No active exception to reraise".to_owned()))?,
            1 | 2 => ExceptionCtor::try_from_object(vm, self.pop_value())?.instantiate(vm)?,
            3 => panic!("Not implemented!"),
            _ => panic!("Invalid parameter for RAISE_VARARGS, must be between 0 to 3"),
        };
        let context = match argc {
            0 => None, // We have already got the exception,
            _ => vm.current_exception(),
        };
        info!(
            "Exception raised: {:?} with cause: {:?} and context: {:?}",
            exception, cause, context
        );
        if let Some(cause) = cause {
            exception.set_cause(cause);
        }
        exception.set_context(context);
        Err(exception)
    }

    fn builtin_coro<'a>(&self, coro: &'a PyObjectRef) -> Option<&'a Coro> {
        match_class!(match coro {
            ref g @ PyGenerator => Some(g.as_coro()),
            ref c @ PyCoroutine => Some(c.as_coro()),
            _ => None,
        })
    }

    fn _send(&self, coro: PyObjectRef, val: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match self.builtin_coro(&coro) {
            Some(coro) => coro.send(val, vm),
            None if vm.is_none(&val) => iterator::call_next(vm, &coro),
            None => vm.call_method(&coro, "send", (val,)),
        }
    }

    fn execute_yield_from(&mut self, vm: &VirtualMachine) -> FrameResult {
        // Value send into iterator:
        let val = self.pop_value();

        let coro = self.last_value();

        let result = self._send(coro, val, vm);

        let result = ExecutionResult::from_result(vm, result)?;

        match result {
            ExecutionResult::Yield(value) => {
                // Set back program counter:
                self.lasti.fetch_sub(1, Ordering::Relaxed);
                Ok(Some(ExecutionResult::Yield(value)))
            }
            ExecutionResult::Return(value) => {
                self.pop_value();
                self.push_value(value);
                Ok(None)
            }
        }
    }

    fn execute_unpack_ex(
        &mut self,
        vm: &VirtualMachine,
        before: usize,
        after: usize,
    ) -> FrameResult {
        let value = self.pop_value();
        let elements = vm.extract_elements::<PyObjectRef>(&value)?;
        let min_expected = before + after;
        if elements.len() < min_expected {
            Err(vm.new_value_error(format!(
                "not enough values to unpack (expected at least {}, got {})",
                min_expected,
                elements.len()
            )))
        } else {
            let middle = elements.len() - before - after;

            // Elements on stack from right-to-left:
            for element in elements[before + middle..].iter().rev() {
                self.push_value(element.clone());
            }

            let middle_elements = elements.iter().skip(before).take(middle).cloned().collect();
            let t = vm.ctx.new_list(middle_elements);
            self.push_value(t);

            // Lastly the first reversed values:
            for element in elements[..before].iter().rev() {
                self.push_value(element.clone());
            }

            Ok(None)
        }
    }

    fn jump(&mut self, label: bytecode::Label) {
        let target_pc = self.code.label_map[&label];
        vm_trace!("jump from {:?} to {:?}", self.lasti(), target_pc);
        self.lasti.store(target_pc, Ordering::Relaxed);
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
    fn execute_make_function(&mut self, vm: &VirtualMachine) -> FrameResult {
        let qualified_name = self
            .pop_value()
            .downcast::<PyStr>()
            .expect("qualified name to be a string");
        let code_obj: PyCodeRef = self
            .pop_value()
            .downcast()
            .expect("Second to top value on the stack must be a code object");

        let flags = code_obj.flags;

        let annotations = if flags.contains(bytecode::CodeFlags::HAS_ANNOTATIONS) {
            self.pop_value()
        } else {
            vm.ctx.new_dict().into_object()
        };

        let kw_only_defaults = if flags.contains(bytecode::CodeFlags::HAS_KW_ONLY_DEFAULTS) {
            Some(
                self.pop_value()
                    .downcast::<PyDict>()
                    .expect("Stack value for keyword only defaults expected to be a dict"),
            )
        } else {
            None
        };

        let defaults = if flags.contains(bytecode::CodeFlags::HAS_DEFAULTS) {
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
        let scope = self.scope.clone();
        let func_obj = vm
            .ctx
            .new_pyfunction(code_obj, scope, defaults, kw_only_defaults);

        vm.set_attr(&func_obj, "__doc__", vm.ctx.none())?;

        let name = qualified_name
            .borrow_value()
            .split('.')
            .next_back()
            .unwrap();
        vm.set_attr(&func_obj, "__name__", vm.ctx.new_str(name))?;
        vm.set_attr(&func_obj, "__qualname__", qualified_name)?;
        let module = vm.unwrap_or_none(self.scope.globals.get_item_option("__name__", vm)?);
        vm.set_attr(&func_obj, "__module__", module)?;
        vm.set_attr(&func_obj, "__annotations__", annotations)?;

        self.push_value(func_obj);
        Ok(None)
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn execute_binop(
        &mut self,
        vm: &VirtualMachine,
        op: &bytecode::BinaryOperator,
        inplace: bool,
    ) -> FrameResult {
        let b_ref = &self.pop_value();
        let a_ref = &self.pop_value();
        let value = if inplace {
            match *op {
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
            }?
        } else {
            match *op {
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
            }?
        };

        self.push_value(value);
        Ok(None)
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn execute_unop(&mut self, vm: &VirtualMachine, op: &bytecode::UnaryOperator) -> FrameResult {
        let a = self.pop_value();
        let value = match *op {
            bytecode::UnaryOperator::Minus => vm.call_method(&a, "__neg__", ())?,
            bytecode::UnaryOperator::Plus => vm.call_method(&a, "__pos__", ())?,
            bytecode::UnaryOperator::Invert => vm.call_method(&a, "__invert__", ())?,
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
        Ok(pybool::boolval(vm, found)?)
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
        let attr_name = self.code.names[attr].clone();
        let parent = self.pop_value();
        let obj = vm.get_attribute(parent, attr_name)?;
        self.push_value(obj);
        Ok(None)
    }

    fn store_attr(&mut self, vm: &VirtualMachine, attr: bytecode::NameIdx) -> FrameResult {
        let attr_name = self.code.names[attr].clone();
        let parent = self.pop_value();
        let value = self.pop_value();
        vm.set_attr(&parent, attr_name, value)?;
        Ok(None)
    }

    fn delete_attr(&mut self, vm: &VirtualMachine, attr: bytecode::NameIdx) -> FrameResult {
        let attr_name = self.code.names[attr].clone().into_object();
        let parent = self.pop_value();
        vm.del_attr(&parent, attr_name)?;
        Ok(None)
    }

    fn lasti(&self) -> usize {
        // it's okay to make this Relaxed, because we know that we only
        // mutate lasti if the mutex is held, and any other thread that
        // wants to guarantee the value of this will use a Lock anyway
        self.lasti.load(Ordering::Relaxed)
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
        self.state.stack.push(obj);
    }

    fn pop_value(&mut self) -> PyObjectRef {
        self.state
            .stack
            .pop()
            .expect("Tried to pop value but there was nothing on the stack")
    }

    fn pop_multiple(&mut self, count: usize) -> Vec<PyObjectRef> {
        let stack_len = self.state.stack.len();
        self.state
            .stack
            .drain(stack_len - count..stack_len)
            .collect()
    }

    fn last_value(&self) -> PyObjectRef {
        self.state.stack.last().unwrap().clone()
    }

    fn nth_value(&self, depth: usize) -> PyObjectRef {
        self.state.stack[self.state.stack.len() - depth - 1].clone()
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
        let dict = self.scope.get_locals();
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
