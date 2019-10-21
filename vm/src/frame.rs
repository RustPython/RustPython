use std::cell::{Cell, RefCell};
use std::fmt;

use indexmap::IndexMap;
use itertools::Itertools;

use crate::bytecode;
use crate::function::PyFuncArgs;
use crate::obj::objbool;
use crate::obj::objcode::PyCodeRef;
use crate::obj::objdict::{PyDict, PyDictRef};
use crate::obj::objiter;
use crate::obj::objlist;
use crate::obj::objslice::PySlice;
use crate::obj::objstr;
use crate::obj::objstr::PyString;
use crate::obj::objtuple::PyTuple;
use crate::obj::objtype;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{
    IdProtocol, ItemProtocol, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject, TypeProtocol,
};
use crate::scope::{NameProtocol, Scope};
use crate::vm::VirtualMachine;

#[cfg(not(target_arch = "wasm32"))]
use crate::stdlib::signal::check_signals;

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
    With {
        end: bytecode::Label,
        context_manager: PyObjectRef,
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
    Raising { exception: PyObjectRef },

    // NoWorries,
    /// We are unwinding blocks, since we hit break
    Break,

    /// We are unwinding blocks since we hit a continue statements.
    Continue,
}

#[pyclass]
pub struct Frame {
    pub code: PyCodeRef,
    // We need 1 stack per frame
    /// The main data frame of the stack machine
    stack: RefCell<Vec<PyObjectRef>>,
    /// Block frames, for controlling loops and exceptions
    blocks: RefCell<Vec<Block>>,
    /// Variables
    pub scope: Scope,
    /// index of last instruction ran
    pub lasti: Cell<usize>,
}

impl PyValue for Frame {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.frame_type()
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
                if objtype::isinstance(&err, &vm.ctx.exceptions.stop_iteration) {
                    objiter::stop_iter_value(vm, &err).map(ExecutionResult::Return)
                } else {
                    Err(err)
                }
            }
        }
    }

    /// Turn an ExecutionResult into a PyResult that would be returned from a generator or coroutine
    pub fn into_result(self, vm: &VirtualMachine) -> PyResult {
        match self {
            ExecutionResult::Yield(value) => Ok(value),
            ExecutionResult::Return(value) => {
                let stop_iteration = vm.ctx.exceptions.stop_iteration.clone();
                let args = if vm.is_none(&value) {
                    vec![]
                } else {
                    vec![value]
                };
                Err(vm.new_exception_obj(stop_iteration, args).unwrap())
            }
        }
    }
}

/// A valid execution result, or an exception
pub type FrameResult = PyResult<Option<ExecutionResult>>;

impl Frame {
    pub fn new(code: PyCodeRef, scope: Scope) -> Frame {
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
            stack: RefCell::new(vec![]),
            blocks: RefCell::new(vec![]),
            // save the callargs as locals
            // globals: locals.clone(),
            scope,
            lasti: Cell::new(0),
        }
    }

    // #[cfg_attr(feature = "flame-it", flame("Frame"))]
    pub fn run(&self, vm: &VirtualMachine) -> PyResult<ExecutionResult> {
        flame_guard!(format!("Frame::run({})", self.code.obj_name));

        let filename = &self.code.source_path.to_string();

        // This is the name of the object being run:
        let run_obj_name = &self.code.obj_name.to_string();

        // Execute until return or exception:
        loop {
            let lineno = self.get_lineno();
            let result = self.execute_instruction(vm);
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
                    assert!(objtype::isinstance(
                        &exception,
                        &vm.ctx.exceptions.base_exception_type
                    ));
                    let traceback = vm
                        .get_attribute(exception.clone(), "__traceback__")
                        .unwrap();
                    vm_trace!("Adding to traceback: {:?} {:?}", traceback, lineno);
                    let raise_location = vm.ctx.new_tuple(vec![
                        vm.ctx.new_str(filename.clone()),
                        vm.ctx.new_int(lineno.row()),
                        vm.ctx.new_str(run_obj_name.clone()),
                    ]);
                    objlist::PyListRef::try_from_object(vm, traceback)?.append(raise_location, vm);
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

    pub(crate) fn gen_throw(
        &self,
        vm: &VirtualMachine,
        exc_type: PyClassRef,
        exc_val: PyObjectRef,
        exc_tb: PyObjectRef,
    ) -> PyResult {
        if let bytecode::Instruction::YieldFrom = self.code.instructions[self.lasti.get()] {
            let coro = self.last_value();
            vm.call_method(
                &coro,
                "throw",
                vec![exc_type.into_object(), exc_val, exc_tb],
            )
            .or_else(|err| {
                self.pop_value();
                self.lasti.set(self.lasti.get() + 1);
                let val = objiter::stop_iter_value(vm, &err)?;
                self._send(coro, val, vm)
            })
        } else {
            let exception = vm.new_exception_obj(exc_type, vec![exc_val])?;
            match self.unwind_blocks(vm, UnwindReason::Raising { exception }) {
                Ok(None) => self.run(vm),
                Ok(Some(result)) => Ok(result),
                Err(exception) => Err(exception),
            }
            .and_then(|res| res.into_result(vm))
        }
    }

    pub fn fetch_instruction(&self) -> &bytecode::Instruction {
        let ins2 = &self.code.instructions[self.lasti.get()];
        self.lasti.set(self.lasti.get() + 1);
        ins2
    }

    /// Execute a single instruction.
    fn execute_instruction(&self, vm: &VirtualMachine) -> FrameResult {
        #[cfg(not(target_arch = "wasm32"))]
        check_signals(vm)?;

        let instruction = self.fetch_instruction();

        flame_guard!(format!("Frame::execute_instruction({:?})", instruction));

        #[cfg(feature = "vm-tracing-logging")]
        {
            trace!("=======");
            /* TODO:
            for frame in self.frames.iter() {
                trace!("  {:?}", frame);
            }
            */
            trace!("  {:?}", self);
            trace!("  Executing op code: {:?}", instruction);
            trace!("=======");
        }

        match &instruction {
            bytecode::Instruction::LoadConst { ref value } => {
                let obj = vm.ctx.unwrap_constant(value);
                self.push_value(obj);
                Ok(None)
            }
            bytecode::Instruction::Import {
                ref name,
                ref symbols,
                ref level,
            } => self.import(vm, name, symbols, *level),
            bytecode::Instruction::ImportStar => self.import_star(vm),
            bytecode::Instruction::ImportFrom { ref name } => self.import_from(vm, name),
            bytecode::Instruction::LoadName {
                ref name,
                ref scope,
            } => self.load_name(vm, name, scope),
            bytecode::Instruction::StoreName {
                ref name,
                ref scope,
            } => self.store_name(vm, name, scope),
            bytecode::Instruction::DeleteName { ref name } => self.delete_name(vm, name),
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
                    .map(|pyobj| objstr::get_value(&pyobj))
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
                let py_obj = vm.ctx.new_set();
                for item in elements {
                    vm.call_method(&py_obj, "add", vec![item])?;
                }
                self.push_value(py_obj);
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
                objlist::PyListRef::try_from_object(vm, list_obj)?.append(item, vm);
                Ok(None)
            }
            bytecode::Instruction::SetAdd { i } => {
                let set_obj = self.nth_value(*i);
                let item = self.pop_value();
                vm.call_method(&set_obj, "add", vec![item])?;
                Ok(None)
            }
            bytecode::Instruction::MapAdd { i } => {
                let dict_obj = self.nth_value(*i + 1);
                let key = self.pop_value();
                let value = self.pop_value();
                vm.call_method(&dict_obj, "__setitem__", vec![key, value])?;
                Ok(None)
            }
            bytecode::Instruction::BinaryOperation { ref op, inplace } => {
                self.execute_binop(vm, op, *inplace)
            }
            bytecode::Instruction::LoadAttr { ref name } => self.load_attr(vm, name),
            bytecode::Instruction::StoreAttr { ref name } => self.store_attr(vm, name),
            bytecode::Instruction::DeleteAttr { ref name } => self.delete_attr(vm, name),
            bytecode::Instruction::UnaryOperation { ref op } => self.execute_unop(vm, op),
            bytecode::Instruction::CompareOperation { ref op } => self.execute_compare(vm, op),
            bytecode::Instruction::ReturnValue => {
                let value = self.pop_value();
                self.unwind_blocks(vm, UnwindReason::Returning { value })
            }
            bytecode::Instruction::YieldValue => {
                let value = self.pop_value();
                Ok(Some(ExecutionResult::Yield(value)))
            }
            bytecode::Instruction::YieldFrom => self.execute_yield_from(vm),
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
                // Call enter:
                let obj = vm.call_method(&context_manager, "__enter__", vec![])?;
                self.push_block(BlockType::With {
                    end: *end,
                    context_manager: context_manager.clone(),
                });
                self.push_value(obj);
                Ok(None)
            }
            bytecode::Instruction::CleanupWith { end: end1 } => {
                let block = self.pop_block();
                if let BlockType::With {
                    end: end2,
                    context_manager,
                } = &block.typ
                {
                    debug_assert!(end1 == end2);
                    self.call_context_manager_exit(vm, &context_manager, None)?;
                } else {
                    unreachable!("Block stack is incorrect, expected a with block");
                }

                Ok(None)
            }
            bytecode::Instruction::PopBlock => {
                self.pop_block();
                Ok(None)
            }
            bytecode::Instruction::GetIter => {
                let iterated_obj = self.pop_value();
                let iter_obj = objiter::get_iter(vm, &iterated_obj)?;
                self.push_value(iter_obj);
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
                let value = objbool::boolval(vm, obj)?;
                if value {
                    self.jump(*target);
                }
                Ok(None)
            }

            bytecode::Instruction::JumpIfFalse { target } => {
                let obj = self.pop_value();
                let value = objbool::boolval(vm, obj)?;
                if !value {
                    self.jump(*target);
                }
                Ok(None)
            }

            bytecode::Instruction::JumpIfTrueOrPop { target } => {
                let obj = self.last_value();
                let value = objbool::boolval(vm, obj)?;
                if value {
                    self.jump(*target);
                } else {
                    self.pop_value();
                }
                Ok(None)
            }

            bytecode::Instruction::JumpIfFalseOrPop { target } => {
                let obj = self.last_value();
                let value = objbool::boolval(vm, obj)?;
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
                if !expr.is(&vm.get_none()) {
                    let repr = vm.to_repr(&expr)?;
                    // TODO: implement sys.displayhook
                    if let Ok(ref print) = vm.get_attribute(vm.builtins.clone(), "print") {
                        vm.invoke(print, vec![repr.into_object()])?;
                    }
                }
                Ok(None)
            }
            bytecode::Instruction::LoadBuildClass => {
                self.push_value(vm.get_attribute(vm.builtins.clone(), "__build_class__")?);
                Ok(None)
            }
            bytecode::Instruction::UnpackSequence { size } => {
                let value = self.pop_value();
                let elements = vm.extract_elements(&value)?;
                if elements.len() != *size {
                    Err(vm.new_value_error("Wrong number of values to unpack".to_string()))
                } else {
                    for element in elements.into_iter().rev() {
                        self.push_value(element);
                    }
                    Ok(None)
                }
            }
            bytecode::Instruction::UnpackEx { before, after } => {
                self.execute_unpack_ex(vm, *before, *after)
            }
            bytecode::Instruction::FormatValue { conversion, spec } => {
                use bytecode::ConversionFlag::*;
                let value = match conversion {
                    Some(Str) => vm.to_str(&self.pop_value())?.into_object(),
                    Some(Repr) => vm.to_repr(&self.pop_value())?.into_object(),
                    Some(Ascii) => vm.to_ascii(&self.pop_value())?,
                    None => self.pop_value(),
                };

                let spec = vm.new_str(spec.clone());
                let formatted = vm.call_method(&value, "__format__", vec![spec])?;
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
                let mut stack = self.stack.borrow_mut();
                let stack_len = stack.len();
                stack[stack_len - amount..stack_len].reverse();
                Ok(None)
            }
        }
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn get_elements(
        &self,
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
        &self,
        vm: &VirtualMachine,
        module: &Option<String>,
        symbols: &[String],
        level: usize,
    ) -> FrameResult {
        let module = module.clone().unwrap_or_default();
        let module = vm.import(&module, symbols, level)?;

        self.push_value(module);
        Ok(None)
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn import_from(&self, vm: &VirtualMachine, name: &str) -> FrameResult {
        let module = self.last_value();
        // Load attribute, and transform any error into import error.
        let obj = vm
            .get_attribute(module, name)
            .map_err(|_| vm.new_import_error(format!("cannot import name '{}'", name)))?;
        self.push_value(obj);
        Ok(None)
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn import_star(&self, vm: &VirtualMachine) -> FrameResult {
        let module = self.pop_value();

        // Grab all the names from the module and put them in the context
        if let Some(dict) = &module.dict {
            for (k, v) in dict {
                let k = vm.to_str(&k)?;
                let k = k.as_str();
                if !k.starts_with('_') {
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
    fn unwind_blocks(&self, vm: &VirtualMachine, mut reason: UnwindReason) -> FrameResult {
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
                        reason: Some(reason.clone()),
                    });
                    self.jump(handler);
                    return Ok(None);
                }
                BlockType::TryExcept { handler } => {
                    self.pop_block();
                    if let UnwindReason::Raising { exception } = &reason {
                        self.push_block(BlockType::ExceptHandler {});
                        self.push_value(exception.clone());
                        vm.push_exception(exception.clone());
                        self.jump(handler);
                        return Ok(None);
                    }
                }
                BlockType::With {
                    context_manager,
                    end,
                } => {
                    self.pop_block();
                    match &reason {
                        UnwindReason::Raising { exception } => {
                            match self.call_context_manager_exit(
                                vm,
                                &context_manager,
                                Some(exception.clone()),
                            ) {
                                Ok(exit_result_obj) => {
                                    match objbool::boolval(vm, exit_result_obj) {
                                        // If __exit__ method returned True, suppress the exception and continue execution.
                                        Ok(suppress_exception) => {
                                            if suppress_exception {
                                                self.jump(end);
                                                return Ok(None);
                                            } else {
                                                // go on with the stack unwinding.
                                            }
                                        }
                                        Err(exit_exc) => {
                                            reason = UnwindReason::Raising {
                                                exception: exit_exc,
                                            };
                                            // return Err(exit_exc);
                                        }
                                    }
                                }
                                Err(exit_exc) => {
                                    // TODO: what about original exception?
                                    reason = UnwindReason::Raising {
                                        exception: exit_exc,
                                    };
                                    // return Err(exit_exc);
                                }
                            }
                        }
                        _ => {
                            match self.call_context_manager_exit(vm, &context_manager, None) {
                                Ok(..) => {}
                                Err(exit_exc) => {
                                    // __exit__ went wrong,
                                    reason = UnwindReason::Raising {
                                        exception: exit_exc,
                                    };
                                    // return Err(exc);
                                }
                            }
                        }
                    }
                }
                BlockType::FinallyHandler { .. } => {
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

    fn call_context_manager_exit(
        &self,
        vm: &VirtualMachine,
        context_manager: &PyObjectRef,
        exc: Option<PyObjectRef>,
    ) -> PyResult {
        // TODO: do we want to put the exit call on the stack?
        // TODO: what happens when we got an error during execution of __exit__?
        let (exc_type, exc_val, exc_tb) = if let Some(exc) = exc {
            let exc_type = exc.class().into_object();
            let exc_val = exc.clone();
            let exc_tb = vm.ctx.none(); // TODO: retrieve traceback?
            (exc_type, exc_val, exc_tb)
        } else {
            (vm.ctx.none(), vm.ctx.none(), vm.ctx.none())
        };

        vm.call_method(context_manager, "__exit__", vec![exc_type, exc_val, exc_tb])
    }

    fn store_name(
        &self,
        vm: &VirtualMachine,
        name: &str,
        name_scope: &bytecode::NameScope,
    ) -> FrameResult {
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

    fn delete_name(&self, vm: &VirtualMachine, name: &str) -> FrameResult {
        match self.scope.delete_name(vm, name) {
            Ok(_) => Ok(None),
            Err(_) => Err(vm.new_name_error(format!("name '{}' is not defined", name))),
        }
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn load_name(
        &self,
        vm: &VirtualMachine,
        name: &str,
        name_scope: &bytecode::NameScope,
    ) -> FrameResult {
        let optional_value = match name_scope {
            bytecode::NameScope::Global => self.scope.load_global(vm, name),
            bytecode::NameScope::NonLocal => self.scope.load_cell(vm, name),
            bytecode::NameScope::Local => self.scope.load_local(&vm, name),
            bytecode::NameScope::Free => self.scope.load_name(&vm, name),
        };

        let value = match optional_value {
            Some(value) => value,
            None => {
                return Err(vm.new_name_error(format!("name '{}' is not defined", name)));
            }
        };

        self.push_value(value);
        Ok(None)
    }

    fn execute_rotate(&self, amount: usize) -> FrameResult {
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
        values.reverse();
        for value in values {
            self.push_value(value);
        }
        Ok(None)
    }

    fn execute_subscript(&self, vm: &VirtualMachine) -> FrameResult {
        let b_ref = self.pop_value();
        let a_ref = self.pop_value();
        let value = a_ref.get_item(&b_ref, vm)?;
        self.push_value(value);
        Ok(None)
    }

    fn execute_store_subscript(&self, vm: &VirtualMachine) -> FrameResult {
        let idx = self.pop_value();
        let obj = self.pop_value();
        let value = self.pop_value();
        obj.set_item(&idx, value, vm)?;
        Ok(None)
    }

    fn execute_delete_subscript(&self, vm: &VirtualMachine) -> FrameResult {
        let idx = self.pop_value();
        let obj = self.pop_value();
        obj.del_item(&idx, vm)?;
        Ok(None)
    }

    #[allow(clippy::collapsible_if)]
    fn execute_build_map(
        &self,
        vm: &VirtualMachine,
        size: usize,
        unpack: bool,
        for_call: bool,
    ) -> FrameResult {
        let map_obj = vm.ctx.new_dict();
        if unpack {
            for obj in self.pop_multiple(size) {
                // Take all key-value pairs from the dict:
                let dict: PyDictRef = obj.downcast().expect("Need a dictionary to build a map.");
                for (key, value) in dict {
                    if for_call {
                        if map_obj.contains_key(&key, vm) {
                            let key_repr = vm.to_repr(&key)?;
                            let msg = format!(
                                "got multiple values for keyword argument {}",
                                key_repr.as_str()
                            );
                            return Err(vm.new_type_error(msg));
                        }
                    }
                    map_obj.set_item(&key, value, vm).unwrap();
                }
            }
        } else {
            for (key, value) in self.pop_multiple(2 * size).into_iter().tuples() {
                map_obj.set_item(&key, value, vm).unwrap();
            }
        }

        self.push_value(map_obj.into_object());
        Ok(None)
    }

    fn execute_build_slice(&self, vm: &VirtualMachine, size: usize) -> FrameResult {
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

    fn execute_call_function(&self, vm: &VirtualMachine, typ: &bytecode::CallType) -> FrameResult {
        let args = match typ {
            bytecode::CallType::Positional(count) => {
                let args: Vec<PyObjectRef> = self.pop_multiple(*count);
                PyFuncArgs {
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
                    .map(|pyobj| objstr::get_value(pyobj))
                    .collect();
                PyFuncArgs::new(args, kwarg_names)
            }
            bytecode::CallType::Ex(has_kwargs) => {
                let kwargs = if *has_kwargs {
                    let kw_dict: PyDictRef =
                        self.pop_value().downcast().expect("Kwargs must be a dict.");
                    kw_dict
                        .into_iter()
                        .map(|elem| (objstr::get_value(&elem.0), elem.1))
                        .collect()
                } else {
                    IndexMap::new()
                };
                let args = self.pop_value();
                let args = vm.extract_elements(&args)?;
                PyFuncArgs { args, kwargs }
            }
        };

        // Call function:
        let func_ref = self.pop_value();
        let value = vm.invoke(&func_ref, args)?;
        self.push_value(value);
        Ok(None)
    }

    fn execute_raise(&self, vm: &VirtualMachine, argc: usize) -> FrameResult {
        let cause = match argc {
            2 => self.get_exception(vm, true)?,
            _ => vm.get_none(),
        };
        let exception = match argc {
            0 => match vm.current_exception() {
                Some(exc) => exc,
                None => {
                    return Err(vm.new_exception(
                        vm.ctx.exceptions.runtime_error.clone(),
                        "No active exception to reraise".to_string(),
                    ));
                }
            },
            1 | 2 => self.get_exception(vm, false)?,
            3 => panic!("Not implemented!"),
            _ => panic!("Invalid parameter for RAISE_VARARGS, must be between 0 to 3"),
        };
        let context = match argc {
            0 => vm.get_none(), // We have already got the exception,
            _ => match vm.current_exception() {
                Some(exc) => exc,
                None => vm.get_none(),
            },
        };
        info!(
            "Exception raised: {:?} with cause: {:?} and context: {:?}",
            exception, cause, context
        );
        vm.set_attr(&exception, vm.new_str("__cause__".to_string()), cause)?;
        vm.set_attr(&exception, vm.new_str("__context__".to_string()), context)?;
        Err(exception)
    }

    fn _send(&self, coro: PyObjectRef, val: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if vm.is_none(&val) {
            objiter::call_next(vm, &coro)
        } else {
            vm.call_method(&coro, "send", vec![val])
        }
    }

    fn execute_yield_from(&self, vm: &VirtualMachine) -> FrameResult {
        // Value send into iterator:
        let val = self.pop_value();

        let coro = self.last_value();

        let result = self._send(coro, val, vm);

        let result = ExecutionResult::from_result(vm, result)?;

        match result {
            ExecutionResult::Yield(value) => {
                // Set back program counter:
                self.lasti.set(self.lasti.get() - 1);
                Ok(Some(ExecutionResult::Yield(value)))
            }
            ExecutionResult::Return(value) => {
                self.pop_value();
                self.push_value(value);
                Ok(None)
            }
        }
    }

    fn execute_unpack_ex(&self, vm: &VirtualMachine, before: usize, after: usize) -> FrameResult {
        let value = self.pop_value();
        let elements = vm.extract_elements::<PyObjectRef>(&value)?;
        let min_expected = before + after;
        if elements.len() < min_expected {
            Err(vm.new_value_error(format!(
                "Not enough values to unpack (expected at least {}, got {}",
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

    fn jump(&self, label: bytecode::Label) {
        let target_pc = self.code.label_map[&label];
        #[cfg(feature = "vm-tracing-logging")]
        trace!("jump from {:?} to {:?}", self.lasti, target_pc);
        self.lasti.set(target_pc);
    }

    /// The top of stack contains the iterator, lets push it forward
    fn execute_for_iter(&self, vm: &VirtualMachine, target: bytecode::Label) -> FrameResult {
        let top_of_stack = self.last_value();
        let next_obj = objiter::get_next_object(vm, &top_of_stack);

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
    fn execute_make_function(&self, vm: &VirtualMachine) -> FrameResult {
        let qualified_name = self
            .pop_value()
            .downcast::<PyString>()
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
            .new_function(code_obj, scope, defaults, kw_only_defaults);

        let name = qualified_name.as_str().split('.').next_back().unwrap();
        vm.set_attr(&func_obj, "__name__", vm.new_str(name.to_string()))?;
        vm.set_attr(&func_obj, "__qualname__", qualified_name)?;
        let module = self
            .scope
            .globals
            .get_item_option("__name__", vm)?
            .unwrap_or_else(|| vm.get_none());
        vm.set_attr(&func_obj, "__module__", module)?;
        vm.set_attr(&func_obj, "__annotations__", annotations)?;

        self.push_value(func_obj);
        Ok(None)
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn execute_binop(
        &self,
        vm: &VirtualMachine,
        op: &bytecode::BinaryOperator,
        inplace: bool,
    ) -> FrameResult {
        let b_ref = self.pop_value();
        let a_ref = self.pop_value();
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
    fn execute_unop(&self, vm: &VirtualMachine, op: &bytecode::UnaryOperator) -> FrameResult {
        let a = self.pop_value();
        let value = match *op {
            bytecode::UnaryOperator::Minus => vm.call_method(&a, "__neg__", vec![])?,
            bytecode::UnaryOperator::Plus => vm.call_method(&a, "__pos__", vec![])?,
            bytecode::UnaryOperator::Invert => vm.call_method(&a, "__invert__", vec![])?,
            bytecode::UnaryOperator::Not => {
                let value = objbool::boolval(vm, a)?;
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
        let found = vm._membership(haystack.clone(), needle)?;
        Ok(objbool::boolval(vm, found)?)
    }

    fn _not_in(
        &self,
        vm: &VirtualMachine,
        needle: PyObjectRef,
        haystack: PyObjectRef,
    ) -> PyResult<bool> {
        let found = vm._membership(haystack.clone(), needle)?;
        Ok(!objbool::boolval(vm, found)?)
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
        &self,
        vm: &VirtualMachine,
        op: &bytecode::ComparisonOperator,
    ) -> FrameResult {
        let b = self.pop_value();
        let a = self.pop_value();
        let value = match *op {
            bytecode::ComparisonOperator::Equal => vm._eq(a, b)?,
            bytecode::ComparisonOperator::NotEqual => vm._ne(a, b)?,
            bytecode::ComparisonOperator::Less => vm._lt(a, b)?,
            bytecode::ComparisonOperator::LessOrEqual => vm._le(a, b)?,
            bytecode::ComparisonOperator::Greater => vm._gt(a, b)?,
            bytecode::ComparisonOperator::GreaterOrEqual => vm._ge(a, b)?,
            bytecode::ComparisonOperator::Is => vm.ctx.new_bool(self._is(a, b)),
            bytecode::ComparisonOperator::IsNot => vm.ctx.new_bool(self._is_not(a, b)),
            bytecode::ComparisonOperator::In => vm.ctx.new_bool(self._in(vm, a, b)?),
            bytecode::ComparisonOperator::NotIn => vm.ctx.new_bool(self._not_in(vm, a, b)?),
        };

        self.push_value(value);
        Ok(None)
    }

    fn load_attr(&self, vm: &VirtualMachine, attr_name: &str) -> FrameResult {
        let parent = self.pop_value();
        let obj = vm.get_attribute(parent, attr_name)?;
        self.push_value(obj);
        Ok(None)
    }

    fn store_attr(&self, vm: &VirtualMachine, attr_name: &str) -> FrameResult {
        let parent = self.pop_value();
        let value = self.pop_value();
        vm.set_attr(&parent, vm.new_str(attr_name.to_string()), value)?;
        Ok(None)
    }

    fn delete_attr(&self, vm: &VirtualMachine, attr_name: &str) -> FrameResult {
        let parent = self.pop_value();
        let name = vm.ctx.new_str(attr_name.to_string());
        vm.del_attr(&parent, name)?;
        Ok(None)
    }

    pub fn get_lineno(&self) -> bytecode::Location {
        self.code.locations[self.lasti.get()].clone()
    }

    fn push_block(&self, typ: BlockType) {
        self.blocks.borrow_mut().push(Block {
            typ,
            level: self.stack.borrow().len(),
        });
    }

    fn pop_block(&self) -> Block {
        let block = self
            .blocks
            .borrow_mut()
            .pop()
            .expect("No more blocks to pop!");
        self.stack.borrow_mut().truncate(block.level);
        block
    }

    fn current_block(&self) -> Option<Block> {
        self.blocks.borrow().last().cloned()
    }

    pub fn push_value(&self, obj: PyObjectRef) {
        self.stack.borrow_mut().push(obj);
    }

    fn pop_value(&self) -> PyObjectRef {
        self.stack
            .borrow_mut()
            .pop()
            .expect("Tried to pop value but there was nothing on the stack")
    }

    fn pop_multiple(&self, count: usize) -> Vec<PyObjectRef> {
        let mut stack = self.stack.borrow_mut();
        let stack_len = stack.len();
        stack.drain(stack_len - count..stack_len).collect()
    }

    fn last_value(&self) -> PyObjectRef {
        self.stack.borrow().last().unwrap().clone()
    }

    fn nth_value(&self, depth: usize) -> PyObjectRef {
        let stack = self.stack.borrow();
        stack[stack.len() - depth - 1].clone()
    }

    #[cfg_attr(feature = "flame-it", flame("Frame"))]
    fn get_exception(&self, vm: &VirtualMachine, none_allowed: bool) -> PyResult {
        let exception = self.pop_value();
        if none_allowed && vm.get_none().is(&exception)
            || objtype::isinstance(&exception, &vm.ctx.exceptions.base_exception_type)
        {
            Ok(exception)
        } else if let Ok(exc_type) = PyClassRef::try_from_object(vm, exception) {
            if objtype::issubclass(&exc_type, &vm.ctx.exceptions.base_exception_type) {
                let exception = vm.new_empty_exception(exc_type)?;
                Ok(exception)
            } else {
                let msg = format!(
                    "Can only raise BaseException derived types, not {}",
                    exc_type
                );
                Err(vm.new_type_error(msg))
            }
        } else {
            Err(vm.new_type_error("exceptions must derive from BaseException".to_string()))
        }
    }
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let stack_str = self
            .stack
            .borrow()
            .iter()
            .map(|elem| {
                if elem.payload.as_any().is::<Frame>() {
                    "\n  > {frame}".to_string()
                } else {
                    format!("\n  > {:?}", elem)
                }
            })
            .collect::<String>();
        let block_str = self
            .blocks
            .borrow()
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
