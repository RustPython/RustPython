extern crate rustpython_parser;

use self::rustpython_parser::ast;
use std::fmt;
use std::mem;
use std::path::PathBuf;

use super::builtins;
use super::bytecode;
use super::import::{import, import_module};
use super::obj::objbool;
use super::obj::objcode;
use super::obj::objdict;
use super::obj::objiter;
use super::obj::objlist;
use super::obj::objstr;
use super::obj::objtype;
use super::pyobject::{
    DictProtocol, IdProtocol, ParentProtocol, PyFuncArgs, PyObject, PyObjectPayload, PyObjectRef,
    PyResult, TypeProtocol,
};
use super::vm::VirtualMachine;
use num_bigint::BigInt;

#[derive(Clone, Debug)]
enum Block {
    Loop {
        start: bytecode::Label,
        end: bytecode::Label,
    },
    TryExcept {
        handler: bytecode::Label,
    },
    With {
        end: bytecode::Label,
        context_manager: PyObjectRef,
    },
}

#[derive(Clone)]
pub struct Frame {
    pub code: bytecode::CodeObject,
    // We need 1 stack per frame
    stack: Vec<PyObjectRef>, // The main data frame of the stack machine
    blocks: Vec<Block>,      // Block frames, for controlling loops and exceptions
    pub locals: PyObjectRef, // Variables
    pub lasti: usize,        // index of last instruction ran
}

// Running a frame can result in one of the below:
pub enum ExecutionResult {
    Return(PyObjectRef),
    Yield(PyObjectRef),
}

// A valid execution result, or an exception
pub type FrameResult = Result<Option<ExecutionResult>, PyObjectRef>;

impl Frame {
    pub fn new(code: PyObjectRef, globals: PyObjectRef) -> Frame {
        //populate the globals and locals
        //TODO: This is wrong, check https://github.com/nedbat/byterun/blob/31e6c4a8212c35b5157919abff43a7daa0f377c6/byterun/pyvm2.py#L95
        /*
        let globals = match globals {
            Some(g) => g,
            None => HashMap::new(),
        };
        */
        let locals = globals;
        // locals.extend(callargs);

        Frame {
            code: objcode::get_value(&code),
            stack: vec![],
            blocks: vec![],
            // save the callargs as locals
            // globals: locals.clone(),
            locals,
            lasti: 0,
        }
    }

    pub fn run_frame_full(&mut self, vm: &mut VirtualMachine) -> PyResult {
        match self.run_frame(vm)? {
            ExecutionResult::Return(value) => Ok(value),
            _ => panic!("Got unexpected result from function"),
        }
    }

    pub fn run_frame(&mut self, vm: &mut VirtualMachine) -> Result<ExecutionResult, PyObjectRef> {
        let filename = &self.code.source_path.to_string();

        let prev_frame = mem::replace(&mut vm.current_frame, Some(vm.ctx.new_frame(self.clone())));

        // This is the name of the object being run:
        let run_obj_name = &self.code.obj_name.to_string();

        // Execute until return or exception:
        let value = loop {
            let lineno = self.get_lineno();
            let result = self.execute_instruction(vm);
            match result {
                Ok(None) => {}
                Ok(Some(value)) => {
                    break Ok(value);
                }
                Err(exception) => {
                    // unwind block stack on exception and find any handlers.
                    // Add an entry in the traceback:
                    assert!(objtype::isinstance(
                        &exception,
                        &vm.ctx.exceptions.base_exception_type
                    ));
                    let traceback_name = vm.new_str("__traceback__".to_string());
                    let traceback = vm.get_attribute(exception.clone(), traceback_name).unwrap();
                    trace!("Adding to traceback: {:?} {:?}", traceback, lineno);
                    let pos = vm.ctx.new_tuple(vec![
                        vm.ctx.new_str(filename.clone()),
                        vm.ctx.new_int(lineno.get_row()),
                        vm.ctx.new_str(run_obj_name.clone()),
                    ]);
                    objlist::list_append(
                        vm,
                        PyFuncArgs {
                            args: vec![traceback, pos],
                            kwargs: vec![],
                        },
                    )
                    .unwrap();
                    // exception.__trace
                    match self.unwind_exception(vm, exception) {
                        None => {}
                        Some(exception) => {
                            // TODO: append line number to traceback?
                            // traceback.append();
                            break Err(exception);
                        }
                    }
                }
            }
        };

        vm.current_frame = prev_frame;
        value
    }

    pub fn fetch_instruction(&mut self) -> bytecode::Instruction {
        // TODO: an immutable reference is enough, we should not
        // clone the instruction.
        let ins2 = self.code.instructions[self.lasti].clone();
        self.lasti += 1;
        ins2
    }

    // Execute a single instruction:
    fn execute_instruction(&mut self, vm: &mut VirtualMachine) -> FrameResult {
        let instruction = self.fetch_instruction();
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
                ref symbol,
            } => self.import(vm, name, symbol),
            bytecode::Instruction::ImportStar { ref name } => self.import_star(vm, name),
            bytecode::Instruction::LoadName { ref name } => self.load_name(vm, name),
            bytecode::Instruction::StoreName { ref name } => self.store_name(vm, name),
            bytecode::Instruction::DeleteName { ref name } => self.delete_name(vm, name),
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
            bytecode::Instruction::Rotate { amount } => {
                // Shuffles top of stack amount down
                if *amount < 2 {
                    panic!("Can only rotate two or more values");
                }

                let mut values = Vec::new();

                // Pop all values from stack:
                for _ in 0..*amount {
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
            bytecode::Instruction::BuildMap { size, unpack } => {
                let map_obj = vm.ctx.new_dict();
                for _x in 0..*size {
                    let obj = self.pop_value();
                    if *unpack {
                        // Take all key-value pairs from the dict:
                        let dict_elements = objdict::get_key_value_pairs(&obj);
                        for (key, value) in dict_elements.iter() {
                            objdict::set_item(&map_obj, vm, key, value);
                        }
                    } else {
                        let key = self.pop_value();
                        objdict::set_item(&map_obj, vm, &key, &obj);
                    }
                }
                self.push_value(map_obj);
                Ok(None)
            }
            bytecode::Instruction::BuildSlice { size } => {
                assert!(*size == 2 || *size == 3);
                let elements = self.pop_multiple(*size);

                let mut out: Vec<Option<BigInt>> = elements
                    .into_iter()
                    .map(|x| match x.borrow().payload {
                        PyObjectPayload::Integer { ref value } => Some(value.clone()),
                        PyObjectPayload::None => None,
                        _ => panic!("Expect Int or None as BUILD_SLICE arguments, got {:?}", x),
                    })
                    .collect();

                let start = out[0].take();
                let stop = out[1].take();
                let step = if out.len() == 3 { out[2].take() } else { None };

                let obj = PyObject::new(
                    PyObjectPayload::Slice { start, stop, step },
                    vm.ctx.slice_type(),
                );
                self.push_value(obj);
                Ok(None)
            }
            bytecode::Instruction::ListAppend { i } => {
                let list_obj = self.nth_value(*i);
                let item = self.pop_value();
                objlist::list_append(
                    vm,
                    PyFuncArgs {
                        args: vec![list_obj.clone(), item],
                        kwargs: vec![],
                    },
                )?;
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
                if let Some(exc) = self.unwind_blocks(vm) {
                    Err(exc)
                } else {
                    Ok(Some(ExecutionResult::Return(value)))
                }
            }
            bytecode::Instruction::YieldValue => {
                let value = self.pop_value();
                Ok(Some(ExecutionResult::Yield(value)))
            }
            bytecode::Instruction::YieldFrom => {
                // Value send into iterator:
                self.pop_value();

                let top_of_stack = self.last_value();
                let next_obj = objiter::get_next_object(vm, &top_of_stack)?;

                match next_obj {
                    Some(value) => {
                        // Set back program counter:
                        self.lasti -= 1;
                        Ok(Some(ExecutionResult::Yield(value)))
                    }
                    None => {
                        // Pop iterator from stack:
                        self.pop_value();
                        Ok(None)
                    }
                }
            }
            bytecode::Instruction::SetupLoop { start, end } => {
                self.push_block(Block::Loop {
                    start: *start,
                    end: *end,
                });
                Ok(None)
            }
            bytecode::Instruction::SetupExcept { handler } => {
                self.push_block(Block::TryExcept { handler: *handler });
                Ok(None)
            }
            bytecode::Instruction::SetupWith { end } => {
                let context_manager = self.pop_value();
                // Call enter:
                let obj = vm.call_method(&context_manager, "__enter__", vec![])?;
                self.push_block(Block::With {
                    end: *end,
                    context_manager: context_manager.clone(),
                });
                self.push_value(obj);
                Ok(None)
            }
            bytecode::Instruction::CleanupWith { end: end1 } => {
                let block = self.pop_block().unwrap();
                if let Block::With {
                    end: end2,
                    context_manager,
                } = &block
                {
                    assert!(end1 == end2);

                    // call exit now with no exception:
                    self.with_exit(vm, context_manager, None)?;
                    Ok(None)
                } else {
                    panic!("Block stack is incorrect, expected a with block");
                }
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
            bytecode::Instruction::ForIter { target } => {
                // The top of stack contains the iterator, lets push it forward:
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
                        self.jump(*target);
                        Ok(None)
                    }
                    Err(next_error) => {
                        // Pop iterator from stack:
                        self.pop_value();
                        Err(next_error)
                    }
                }
            }
            bytecode::Instruction::MakeFunction { flags } => {
                let _qualified_name = self.pop_value();
                let code_obj = self.pop_value();
                let defaults = if flags.contains(bytecode::FunctionOpArg::HAS_DEFAULTS) {
                    self.pop_value()
                } else {
                    vm.get_none()
                };
                // pop argc arguments
                // argument: name, args, globals
                let scope = self.locals.clone();
                let obj = vm.ctx.new_function(code_obj, scope, defaults);
                self.push_value(obj);
                Ok(None)
            }
            bytecode::Instruction::CallFunction { typ } => {
                let args = match typ {
                    bytecode::CallType::Positional(count) => {
                        let args: Vec<PyObjectRef> = self.pop_multiple(*count);
                        PyFuncArgs {
                            args,
                            kwargs: vec![],
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
                            let kw_dict = self.pop_value();
                            let dict_elements = objdict::get_elements(&kw_dict).clone();
                            dict_elements
                                .into_iter()
                                .map(|elem| (elem.0, (elem.1).1))
                                .collect()
                        } else {
                            vec![]
                        };
                        let args = self.pop_value();
                        let args = vm.extract_elements(&args)?;
                        PyFuncArgs { args, kwargs }
                    }
                };

                // Call function:
                let func_ref = self.pop_value();
                let value = vm.invoke(func_ref, args)?;
                self.push_value(value);
                Ok(None)
            }
            bytecode::Instruction::Jump { target } => {
                self.jump(*target);
                Ok(None)
            }
            bytecode::Instruction::JumpIf { target } => {
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

            bytecode::Instruction::Raise { argc } => {
                let exception = match argc {
                    1 => self.pop_value(),
                    0 | 2 | 3 => panic!("Not implemented!"),
                    _ => panic!("Invalid parameter for RAISE_VARARGS, must be between 0 to 3"),
                };
                if objtype::isinstance(&exception, &vm.ctx.exceptions.base_exception_type) {
                    info!("Exception raised: {:?}", exception);
                    Err(exception)
                } else {
                    let msg = format!(
                        "Can only raise BaseException derived types, not {}",
                        exception.borrow()
                    );
                    let type_error_type = vm.ctx.exceptions.type_error.clone();
                    let type_error = vm.new_exception(type_error_type, msg);
                    Err(type_error)
                }
            }

            bytecode::Instruction::Break => {
                let block = self.unwind_loop(vm);
                if let Block::Loop { end, .. } = block {
                    self.jump(end);
                }
                Ok(None)
            }
            bytecode::Instruction::Pass => {
                // Ah, this is nice, just relax!
                Ok(None)
            }
            bytecode::Instruction::Continue => {
                let block = self.unwind_loop(vm);
                if let Block::Loop { start, .. } = block {
                    self.jump(start);
                } else {
                    assert!(false);
                }
                Ok(None)
            }
            bytecode::Instruction::PrintExpr => {
                let expr = self.pop_value();
                match expr.borrow().payload {
                    PyObjectPayload::None => (),
                    _ => {
                        let repr = vm.to_repr(&expr)?;
                        builtins::builtin_print(
                            vm,
                            PyFuncArgs {
                                args: vec![repr],
                                kwargs: vec![],
                            },
                        )?;
                    }
                }
                Ok(None)
            }
            bytecode::Instruction::LoadBuildClass => {
                let rustfunc = PyObject::new(
                    PyObjectPayload::RustFunction {
                        function: Box::new(builtins::builtin_build_class_),
                    },
                    vm.ctx.type_type(),
                );
                self.push_value(rustfunc);
                Ok(None)
            }
            bytecode::Instruction::StoreLocals => {
                let locals = self.pop_value();
                match self.locals.borrow_mut().payload {
                    PyObjectPayload::Scope { ref mut scope } => {
                        scope.locals = locals;
                    }
                    _ => panic!("We really expect our scope to be a scope!"),
                }
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
                let value = self.pop_value();
                let elements = vm.extract_elements(&value)?;
                let min_expected = *before + *after;
                if elements.len() < min_expected {
                    Err(vm.new_value_error(format!(
                        "Not enough values to unpack (expected at least {}, got {}",
                        min_expected,
                        elements.len()
                    )))
                } else {
                    let middle = elements.len() - *before - *after;

                    // Elements on stack from right-to-left:
                    for element in elements[*before + middle..].iter().rev() {
                        self.push_value(element.clone());
                    }

                    let middle_elements = elements
                        .iter()
                        .skip(*before)
                        .take(middle)
                        .cloned()
                        .collect();
                    let t = vm.ctx.new_list(middle_elements);
                    self.push_value(t);

                    // Lastly the first reversed values:
                    for element in elements[..*before].iter().rev() {
                        self.push_value(element.clone());
                    }

                    Ok(None)
                }
            }
            bytecode::Instruction::Unpack => {
                let value = self.pop_value();
                let elements = vm.extract_elements(&value)?;
                for element in elements.into_iter().rev() {
                    self.push_value(element);
                }
                Ok(None)
            }
            bytecode::Instruction::FormatValue { spec } => {
                let value = self.pop_value();
                let spec = vm.new_str(spec.clone());
                let formatted = vm.call_method(&value, "__format__", vec![spec])?;
                self.push_value(formatted);
                Ok(None)
            }
        }
    }

    fn get_elements(
        &mut self,
        vm: &mut VirtualMachine,
        size: usize,
        unpack: bool,
    ) -> Result<Vec<PyObjectRef>, PyObjectRef> {
        let elements = self.pop_multiple(size);
        if unpack {
            let mut result: Vec<PyObjectRef> = vec![];
            for element in elements {
                let expanded = vm.extract_elements(&element)?;
                for inner in expanded {
                    result.push(inner);
                }
            }
            Ok(result)
        } else {
            Ok(elements)
        }
    }

    fn import(
        &mut self,
        vm: &mut VirtualMachine,
        module: &str,
        symbol: &Option<String>,
    ) -> FrameResult {
        let current_path = {
            let mut source_pathbuf = PathBuf::from(&self.code.source_path);
            source_pathbuf.pop();
            source_pathbuf
        };

        let obj = import(vm, current_path, module, symbol)?;

        // Push module on stack:
        self.push_value(obj);
        Ok(None)
    }

    fn import_star(&mut self, vm: &mut VirtualMachine, module: &str) -> FrameResult {
        let current_path = {
            let mut source_pathbuf = PathBuf::from(&self.code.source_path);
            source_pathbuf.pop();
            source_pathbuf
        };

        // Grab all the names from the module and put them in the context
        let obj = import_module(vm, current_path, module)?;

        for (k, v) in obj.get_key_value_pairs().iter() {
            vm.ctx
                .set_attr(&self.locals, &objstr::get_value(k), v.clone());
        }
        Ok(None)
    }

    // Unwind all blocks:
    fn unwind_blocks(&mut self, vm: &mut VirtualMachine) -> Option<PyObjectRef> {
        loop {
            let block = self.pop_block();
            match block {
                Some(Block::Loop { .. }) => {}
                Some(Block::TryExcept { .. }) => {
                    // TODO: execute finally handler
                }
                Some(Block::With {
                    context_manager, ..
                }) => {
                    match self.with_exit(vm, &context_manager, None) {
                        Ok(..) => {}
                        Err(exc) => {
                            // __exit__ went wrong,
                            return Some(exc);
                        }
                    }
                }
                None => break None,
            }
        }
    }

    fn unwind_loop(&mut self, vm: &mut VirtualMachine) -> Block {
        loop {
            let block = self.pop_block();
            match block {
                Some(Block::Loop { .. }) => break block.unwrap(),
                Some(Block::TryExcept { .. }) => {
                    // TODO: execute finally handler
                }
                Some(Block::With {
                    context_manager, ..
                }) => match self.with_exit(vm, &context_manager, None) {
                    Ok(..) => {}
                    Err(exc) => {
                        panic!("Exception in with __exit__ {:?}", exc);
                    }
                },
                None => panic!("No block to break / continue"),
            }
        }
    }

    fn unwind_exception(
        &mut self,
        vm: &mut VirtualMachine,
        exc: PyObjectRef,
    ) -> Option<PyObjectRef> {
        // unwind block stack on exception and find any handlers:
        loop {
            let block = self.pop_block();
            match block {
                Some(Block::TryExcept { handler }) => {
                    self.push_value(exc);
                    self.jump(handler);
                    return None;
                }
                Some(Block::With {
                    end,
                    context_manager,
                }) => {
                    match self.with_exit(vm, &context_manager, Some(exc.clone())) {
                        Ok(exit_action) => {
                            match objbool::boolval(vm, exit_action) {
                                Ok(handle_exception) => {
                                    if handle_exception {
                                        // We handle the exception, so return!
                                        self.jump(end);
                                        return None;
                                    } else {
                                        // go on with the stack unwinding.
                                    }
                                }
                                Err(exit_exc) => {
                                    return Some(exit_exc);
                                }
                            }
                            // if objtype::isinstance
                        }
                        Err(exit_exc) => {
                            // TODO: what about original exception?
                            return Some(exit_exc);
                        }
                    }
                }
                Some(Block::Loop { .. }) => {}
                None => break,
            }
        }
        Some(exc)
    }

    fn with_exit(
        &mut self,
        vm: &mut VirtualMachine,
        context_manager: &PyObjectRef,
        exc: Option<PyObjectRef>,
    ) -> PyResult {
        // Assume top of stack is __exit__ method:
        // TODO: do we want to put the exit call on the stack?
        // let exit_method = self.pop_value();
        // let args = PyFuncArgs::default();
        // TODO: what happens when we got an error during handling exception?
        let args = if let Some(exc) = exc {
            let exc_type = exc.typ();
            let exc_val = exc.clone();
            let exc_tb = vm.ctx.none(); // TODO: retrieve traceback?
            vec![exc_type, exc_val, exc_tb]
        } else {
            let exc_type = vm.ctx.none();
            let exc_val = vm.ctx.none();
            let exc_tb = vm.ctx.none();
            vec![exc_type, exc_val, exc_tb]
        };
        vm.call_method(context_manager, "__exit__", args)
    }

    fn store_name(&mut self, vm: &mut VirtualMachine, name: &str) -> FrameResult {
        let obj = self.pop_value();
        vm.ctx.set_attr(&self.locals, name, obj);
        Ok(None)
    }

    fn delete_name(&mut self, vm: &mut VirtualMachine, name: &str) -> FrameResult {
        let locals = match self.locals.borrow().payload {
            PyObjectPayload::Scope { ref scope } => scope.locals.clone(),
            _ => panic!("We really expect our scope to be a scope!"),
        };

        // Assume here that locals is a dict
        let name = vm.ctx.new_str(name.to_string());
        vm.call_method(&locals, "__delitem__", vec![name])?;
        Ok(None)
    }

    fn load_name(&mut self, vm: &mut VirtualMachine, name: &str) -> FrameResult {
        // Lookup name in scope and put it onto the stack!
        let mut scope = self.locals.clone();
        loop {
            if scope.contains_key(name) {
                let obj = scope.get_item(name).unwrap();
                self.push_value(obj);
                break Ok(None);
            } else if scope.has_parent() {
                scope = scope.get_parent();
            } else {
                let name_error_type = vm.ctx.exceptions.name_error.clone();
                let msg = format!("name '{}' is not defined", name);
                let name_error = vm.new_exception(name_error_type, msg);
                break Err(name_error);
            }
        }
    }

    fn subscript(&mut self, vm: &mut VirtualMachine, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        vm.call_method(&a, "__getitem__", vec![b])
    }

    fn execute_store_subscript(&mut self, vm: &mut VirtualMachine) -> FrameResult {
        let idx = self.pop_value();
        let obj = self.pop_value();
        let value = self.pop_value();
        vm.call_method(&obj, "__setitem__", vec![idx, value])?;
        Ok(None)
    }

    fn execute_delete_subscript(&mut self, vm: &mut VirtualMachine) -> FrameResult {
        let idx = self.pop_value();
        let obj = self.pop_value();
        vm.call_method(&obj, "__delitem__", vec![idx])?;
        Ok(None)
    }

    fn jump(&mut self, label: bytecode::Label) {
        let target_pc = self.code.label_map[&label];
        trace!("program counter from {:?} to {:?}", self.lasti, target_pc);
        self.lasti = target_pc;
    }

    fn execute_binop(
        &mut self,
        vm: &mut VirtualMachine,
        op: &bytecode::BinaryOperator,
        inplace: bool,
    ) -> FrameResult {
        let b_ref = self.pop_value();
        let a_ref = self.pop_value();
        let value = match *op {
            bytecode::BinaryOperator::Subtract if inplace => vm._isub(a_ref, b_ref),
            bytecode::BinaryOperator::Subtract => vm._sub(a_ref, b_ref),
            bytecode::BinaryOperator::Add if inplace => vm._iadd(a_ref, b_ref),
            bytecode::BinaryOperator::Add => vm._add(a_ref, b_ref),
            bytecode::BinaryOperator::Multiply if inplace => vm._imul(a_ref, b_ref),
            bytecode::BinaryOperator::Multiply => vm._mul(a_ref, b_ref),
            bytecode::BinaryOperator::MatrixMultiply if inplace => vm._imatmul(a_ref, b_ref),
            bytecode::BinaryOperator::MatrixMultiply => vm._matmul(a_ref, b_ref),
            bytecode::BinaryOperator::Power if inplace => vm._ipow(a_ref, b_ref),
            bytecode::BinaryOperator::Power => vm._pow(a_ref, b_ref),
            bytecode::BinaryOperator::Divide if inplace => vm._itruediv(a_ref, b_ref),
            bytecode::BinaryOperator::Divide => vm._truediv(a_ref, b_ref),
            bytecode::BinaryOperator::FloorDivide if inplace => vm._ifloordiv(a_ref, b_ref),
            bytecode::BinaryOperator::FloorDivide => vm._floordiv(a_ref, b_ref),
            // TODO: Subscript should probably have its own op
            bytecode::BinaryOperator::Subscript if inplace => unreachable!(),
            bytecode::BinaryOperator::Subscript => self.subscript(vm, a_ref, b_ref),
            bytecode::BinaryOperator::Modulo if inplace => vm._imod(a_ref, b_ref),
            bytecode::BinaryOperator::Modulo => vm._mod(a_ref, b_ref),
            bytecode::BinaryOperator::Lshift if inplace => vm._ilshift(a_ref, b_ref),
            bytecode::BinaryOperator::Lshift => vm._lshift(a_ref, b_ref),
            bytecode::BinaryOperator::Rshift if inplace => vm._irshift(a_ref, b_ref),
            bytecode::BinaryOperator::Rshift => vm._rshift(a_ref, b_ref),
            bytecode::BinaryOperator::Xor if inplace => vm._ixor(a_ref, b_ref),
            bytecode::BinaryOperator::Xor => vm._xor(a_ref, b_ref),
            bytecode::BinaryOperator::Or if inplace => vm._ior(a_ref, b_ref),
            bytecode::BinaryOperator::Or => vm._or(a_ref, b_ref),
            bytecode::BinaryOperator::And if inplace => vm._iand(a_ref, b_ref),
            bytecode::BinaryOperator::And => vm._and(a_ref, b_ref),
        }?;

        self.push_value(value);
        Ok(None)
    }

    fn execute_unop(
        &mut self,
        vm: &mut VirtualMachine,
        op: &bytecode::UnaryOperator,
    ) -> FrameResult {
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

    // https://docs.python.org/3/reference/expressions.html#membership-test-operations
    fn _membership(
        &mut self,
        vm: &mut VirtualMachine,
        needle: PyObjectRef,
        haystack: &PyObjectRef,
    ) -> PyResult {
        vm.call_method(&haystack, "__contains__", vec![needle])
        // TODO: implement __iter__ and __getitem__ cases when __contains__ is
        // not implemented.
    }

    fn _in(
        &mut self,
        vm: &mut VirtualMachine,
        needle: PyObjectRef,
        haystack: PyObjectRef,
    ) -> PyResult {
        match self._membership(vm, needle, &haystack) {
            Ok(found) => Ok(found),
            Err(_) => Err(vm.new_type_error(format!(
                "{} has no __contains__ method",
                objtype::get_type_name(&haystack.typ())
            ))),
        }
    }

    fn _not_in(
        &mut self,
        vm: &mut VirtualMachine,
        needle: PyObjectRef,
        haystack: PyObjectRef,
    ) -> PyResult {
        match self._membership(vm, needle, &haystack) {
            Ok(found) => Ok(vm.ctx.new_bool(!objbool::get_value(&found))),
            Err(_) => Err(vm.new_type_error(format!(
                "{} has no __contains__ method",
                objtype::get_type_name(&haystack.typ())
            ))),
        }
    }

    fn _is(&self, a: PyObjectRef, b: PyObjectRef) -> bool {
        // Pointer equal:
        a.is(&b)
    }

    fn _is_not(&self, vm: &VirtualMachine, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        let result_bool = !a.is(&b);
        let result = vm.ctx.new_bool(result_bool);
        Ok(result)
    }

    fn execute_compare(
        &mut self,
        vm: &mut VirtualMachine,
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
            bytecode::ComparisonOperator::IsNot => self._is_not(vm, a, b)?,
            bytecode::ComparisonOperator::In => self._in(vm, a, b)?,
            bytecode::ComparisonOperator::NotIn => self._not_in(vm, a, b)?,
        };

        self.push_value(value);
        Ok(None)
    }

    fn load_attr(&mut self, vm: &mut VirtualMachine, attr_name: &str) -> FrameResult {
        let parent = self.pop_value();
        let attr_name = vm.new_str(attr_name.to_string());
        let obj = vm.get_attribute(parent, attr_name)?;
        self.push_value(obj);
        Ok(None)
    }

    fn store_attr(&mut self, vm: &mut VirtualMachine, attr_name: &str) -> FrameResult {
        let parent = self.pop_value();
        let value = self.pop_value();
        vm.ctx.set_attr(&parent, attr_name, value);
        Ok(None)
    }

    fn delete_attr(&mut self, vm: &mut VirtualMachine, attr_name: &str) -> FrameResult {
        let parent = self.pop_value();
        let name = vm.ctx.new_str(attr_name.to_string());
        vm.del_attr(&parent, name)?;
        Ok(None)
    }

    pub fn get_lineno(&self) -> ast::Location {
        self.code.locations[self.lasti].clone()
    }

    fn push_block(&mut self, block: Block) {
        self.blocks.push(block);
    }

    fn pop_block(&mut self) -> Option<Block> {
        self.blocks.pop()
    }

    pub fn push_value(&mut self, obj: PyObjectRef) {
        self.stack.push(obj);
    }

    fn pop_value(&mut self) -> PyObjectRef {
        self.stack.pop().unwrap()
    }

    fn pop_multiple(&mut self, count: usize) -> Vec<PyObjectRef> {
        let mut objs: Vec<PyObjectRef> = Vec::new();
        for _x in 0..count {
            objs.push(self.stack.pop().unwrap());
        }
        objs.reverse();
        objs
    }

    fn last_value(&self) -> PyObjectRef {
        self.stack.last().unwrap().clone()
    }

    fn nth_value(&self, depth: usize) -> PyObjectRef {
        self.stack[self.stack.len() - depth - 1].clone()
    }
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let stack_str = self
            .stack
            .iter()
            .map(|elem| format!("\n  > {:?}", elem.borrow()))
            .collect::<Vec<_>>()
            .join("");
        let block_str = self
            .blocks
            .iter()
            .map(|elem| format!("\n  > {:?}", elem))
            .collect::<Vec<_>>()
            .join("");
        let local_str = match self.locals.borrow().payload {
            PyObjectPayload::Scope { ref scope } => match scope.locals.borrow().payload {
                PyObjectPayload::Dict { ref elements } => {
                    objdict::get_key_value_pairs_from_content(elements)
                        .iter()
                        .map(|elem| format!("\n  {:?} = {:?}", elem.0.borrow(), elem.1.borrow()))
                        .collect::<Vec<_>>()
                        .join("")
                }
                ref unexpected => panic!(
                    "locals unexpectedly not wrapping a dict! instead: {:?}",
                    unexpected
                ),
            },
            ref unexpected => panic!("locals unexpectedly not a scope! instead: {:?}", unexpected),
        };
        write!(
            f,
            "Frame Object {{ \n Stack:{}\n Blocks:{}\n Locals:{}\n}}",
            stack_str, block_str, local_str
        )
    }
}
