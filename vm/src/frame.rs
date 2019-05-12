use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use rustpython_parser::ast;

use crate::builtins;
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
    IdProtocol, ItemProtocol, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject,
    TypeProtocol,
};
use crate::vm::VirtualMachine;
use itertools::Itertools;

/*
 * So a scope is a linked list of scopes.
 * When a name is looked up, it is check in its scope.
 */
#[derive(Debug)]
struct RcListNode<T> {
    elem: T,
    next: Option<Rc<RcListNode<T>>>,
}

#[derive(Debug, Clone)]
struct RcList<T> {
    head: Option<Rc<RcListNode<T>>>,
}

struct Iter<'a, T: 'a> {
    next: Option<&'a RcListNode<T>>,
}

impl<T> RcList<T> {
    pub fn new() -> Self {
        RcList { head: None }
    }

    pub fn insert(self, elem: T) -> Self {
        RcList {
            head: Some(Rc::new(RcListNode {
                elem,
                next: self.head,
            })),
        }
    }

    pub fn iter(&self) -> Iter<T> {
        Iter {
            next: self.head.as_ref().map(|node| &**node),
        }
    }
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        self.next.map(|node| {
            self.next = node.next.as_ref().map(|node| &**node);
            &node.elem
        })
    }
}

#[derive(Clone)]
pub struct Scope {
    locals: RcList<PyDictRef>,
    pub globals: PyDictRef,
}

impl fmt::Debug for Scope {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: have a more informative Debug impl that DOESN'T recurse and cause a stack overflow
        f.write_str("Scope")
    }
}

impl Scope {
    pub fn new(locals: Option<PyDictRef>, globals: PyDictRef) -> Scope {
        let locals = match locals {
            Some(dict) => RcList::new().insert(dict),
            None => RcList::new(),
        };
        Scope { locals, globals }
    }

    pub fn with_builtins(
        locals: Option<PyDictRef>,
        globals: PyDictRef,
        vm: &VirtualMachine,
    ) -> Scope {
        if !globals.contains_key("__builtins__", vm) {
            globals
                .clone()
                .set_item("__builtins__", vm.builtins.clone(), vm)
                .unwrap();
        }
        Scope::new(locals, globals)
    }

    pub fn get_locals(&self) -> PyDictRef {
        match self.locals.iter().next() {
            Some(dict) => dict.clone(),
            None => self.globals.clone(),
        }
    }

    pub fn get_only_locals(&self) -> Option<PyDictRef> {
        self.locals.iter().next().cloned()
    }

    pub fn child_scope_with_locals(&self, locals: PyDictRef) -> Scope {
        Scope {
            locals: self.locals.clone().insert(locals),
            globals: self.globals.clone(),
        }
    }

    pub fn child_scope(&self, ctx: &PyContext) -> Scope {
        self.child_scope_with_locals(ctx.new_dict())
    }
}

pub trait NameProtocol {
    fn load_name(&self, vm: &VirtualMachine, name: &str) -> Option<PyObjectRef>;
    fn store_name(&self, vm: &VirtualMachine, name: &str, value: PyObjectRef);
    fn delete_name(&self, vm: &VirtualMachine, name: &str) -> PyResult;
    fn load_cell(&self, vm: &VirtualMachine, name: &str) -> Option<PyObjectRef>;
    fn store_cell(&self, vm: &VirtualMachine, name: &str, value: PyObjectRef);
    fn load_global(&self, vm: &VirtualMachine, name: &str) -> Option<PyObjectRef>;
    fn store_global(&self, vm: &VirtualMachine, name: &str, value: PyObjectRef);
}

impl NameProtocol for Scope {
    fn load_name(&self, vm: &VirtualMachine, name: &str) -> Option<PyObjectRef> {
        for dict in self.locals.iter() {
            if let Some(value) = dict.get_item_option(name, vm).unwrap() {
                return Some(value);
            }
        }

        if let Some(value) = self.globals.get_item_option(name, vm).unwrap() {
            return Some(value);
        }

        vm.get_attribute(vm.builtins.clone(), name).ok()
    }

    fn load_cell(&self, vm: &VirtualMachine, name: &str) -> Option<PyObjectRef> {
        for dict in self.locals.iter().skip(1) {
            if let Some(value) = dict.get_item_option(name, vm).unwrap() {
                return Some(value);
            }
        }
        None
    }

    fn store_cell(&self, vm: &VirtualMachine, name: &str, value: PyObjectRef) {
        self.locals
            .iter()
            .skip(1)
            .next()
            .expect("no outer scope for non-local")
            .set_item(name, value, vm)
            .unwrap();
    }

    fn store_name(&self, vm: &VirtualMachine, key: &str, value: PyObjectRef) {
        self.get_locals().set_item(key, value, vm).unwrap();
    }

    fn delete_name(&self, vm: &VirtualMachine, key: &str) -> PyResult {
        self.get_locals().del_item(key, vm)
    }

    fn load_global(&self, vm: &VirtualMachine, name: &str) -> Option<PyObjectRef> {
        self.globals.get_item_option(name, vm).unwrap()
    }

    fn store_global(&self, vm: &VirtualMachine, name: &str, value: PyObjectRef) {
        self.globals.set_item(name, value, vm).unwrap();
    }
}

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
    With {
        end: bytecode::Label,
        context_manager: PyObjectRef,
    },
    ExceptHandler,
}

pub type FrameRef = PyRef<Frame>;

pub struct Frame {
    pub code: bytecode::CodeObject,
    // We need 1 stack per frame
    stack: RefCell<Vec<PyObjectRef>>, // The main data frame of the stack machine
    blocks: RefCell<Vec<Block>>,      // Block frames, for controlling loops and exceptions
    pub scope: Scope,                 // Variables
    pub lasti: RefCell<usize>,        // index of last instruction ran
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

// A valid execution result, or an exception
pub type FrameResult = Result<Option<ExecutionResult>, PyObjectRef>;

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
            code: code.code.clone(),
            stack: RefCell::new(vec![]),
            blocks: RefCell::new(vec![]),
            // save the callargs as locals
            // globals: locals.clone(),
            scope,
            lasti: RefCell::new(0),
        }
    }

    pub fn run(&self, vm: &VirtualMachine) -> Result<ExecutionResult, PyObjectRef> {
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
                Err(exception) => {
                    // unwind block stack on exception and find any handlers.
                    // Add an entry in the traceback:
                    assert!(objtype::isinstance(
                        &exception,
                        &vm.ctx.exceptions.base_exception_type
                    ));
                    let traceback = vm
                        .get_attribute(exception.clone(), "__traceback__")
                        .unwrap();
                    trace!("Adding to traceback: {:?} {:?}", traceback, lineno);
                    let pos = vm.ctx.new_tuple(vec![
                        vm.ctx.new_str(filename.clone()),
                        vm.ctx.new_int(lineno.get_row()),
                        vm.ctx.new_str(run_obj_name.clone()),
                    ]);
                    objlist::PyListRef::try_from_object(vm, traceback)?.append(pos, vm);
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
        }
    }

    pub fn throw(
        &self,
        vm: &VirtualMachine,
        exception: PyObjectRef,
    ) -> Result<ExecutionResult, PyObjectRef> {
        match self.unwind_exception(vm, exception) {
            None => self.run(vm),
            Some(exception) => Err(exception),
        }
    }

    pub fn fetch_instruction(&self) -> &bytecode::Instruction {
        let ins2 = &self.code.instructions[*self.lasti.borrow()];
        *self.lasti.borrow_mut() += 1;
        ins2
    }

    // Execute a single instruction:
    fn execute_instruction(&self, vm: &VirtualMachine) -> FrameResult {
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
            bytecode::Instruction::LoadName {
                ref name,
                ref scope,
            } => self.load_name(vm, name, scope),
            bytecode::Instruction::StoreName {
                ref name,
                ref scope,
            } => self.store_name(vm, name, scope),
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
                if *unpack {
                    for obj in self.pop_multiple(*size) {
                        // Take all key-value pairs from the dict:
                        let dict: PyDictRef =
                            obj.downcast().expect("Need a dictionary to build a map.");
                        for (key, value) in dict {
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
            bytecode::Instruction::BuildSlice { size } => {
                assert!(*size == 2 || *size == 3);

                let step = if *size == 3 {
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
                        *self.lasti.borrow_mut() -= 1;
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
                let block = self.pop_block().unwrap();
                if let BlockType::With {
                    end: end2,
                    context_manager,
                } = &block.typ
                {
                    debug_assert!(end1 == end2);

                    // call exit now with no exception:
                    self.with_exit(vm, &context_manager, None)?;
                } else {
                    unreachable!("Block stack is incorrect, expected a with block");
                }

                Ok(None)
            }
            bytecode::Instruction::PopBlock => {
                self.pop_block().expect("no pop to block");
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
                let qualified_name = self
                    .pop_value()
                    .downcast::<PyString>()
                    .expect("qualified name to be a string");
                let code_obj = self
                    .pop_value()
                    .downcast()
                    .expect("Second to top value on the stack must be a code object");

                let annotations = if flags.contains(bytecode::FunctionOpArg::HAS_ANNOTATIONS) {
                    self.pop_value()
                } else {
                    vm.ctx.new_dict().into_object()
                };

                let kw_only_defaults =
                    if flags.contains(bytecode::FunctionOpArg::HAS_KW_ONLY_DEFAULTS) {
                        Some(
                            self.pop_value().downcast::<PyDict>().expect(
                                "Stack value for keyword only defaults expected to be a dict",
                            ),
                        )
                    } else {
                        None
                    };

                let defaults = if flags.contains(bytecode::FunctionOpArg::HAS_DEFAULTS) {
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
                let obj = vm
                    .ctx
                    .new_function(code_obj, scope, defaults, kw_only_defaults);

                let name = qualified_name.value.split('.').next_back().unwrap();
                vm.set_attr(&obj, "__name__", vm.new_str(name.to_string()))?;
                vm.set_attr(&obj, "__qualname__", qualified_name)?;
                let module = self
                    .scope
                    .globals
                    .get_item_option("__name__", vm)?
                    .unwrap_or_else(|| vm.get_none());
                vm.set_attr(&obj, "__module__", module)?;
                vm.set_attr(&obj, "__annotations__", annotations)?;

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
                            let kw_dict: PyDictRef =
                                self.pop_value().downcast().expect("Kwargs must be a dict.");
                            kw_dict
                                .into_iter()
                                .map(|elem| (objstr::get_value(&elem.0), elem.1))
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
                let cause = match argc {
                    2 => self.get_exception(vm, true)?,
                    _ => vm.get_none(),
                };
                let exception = match argc {
                    0 => match vm.pop_exception() {
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
                    _ => match vm.pop_exception() {
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

            bytecode::Instruction::Break => {
                let block = self.unwind_loop(vm);
                if let BlockType::Loop { end, .. } = block.typ {
                    self.pop_block();
                    self.jump(end);
                } else {
                    unreachable!()
                }
                Ok(None)
            }
            bytecode::Instruction::Pass => {
                // Ah, this is nice, just relax!
                Ok(None)
            }
            bytecode::Instruction::Continue => {
                let block = self.unwind_loop(vm);
                if let BlockType::Loop { start, .. } = block.typ {
                    self.jump(start);
                } else {
                    unreachable!();
                }
                Ok(None)
            }
            bytecode::Instruction::PrintExpr => {
                let expr = self.pop_value();
                if !expr.is(&vm.get_none()) {
                    let repr = vm.to_repr(&expr)?;
                    // TODO: implement sys.displayhook
                    if let Ok(print) = vm.get_attribute(vm.builtins.clone(), "print") {
                        vm.invoke(print, vec![repr.into_object()])?;
                    }
                }
                Ok(None)
            }
            bytecode::Instruction::LoadBuildClass => {
                self.push_value(vm.ctx.new_rustfunc(builtins::builtin_build_class_));
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
            bytecode::Instruction::FormatValue { conversion, spec } => {
                use ast::ConversionFlag::*;
                let value = match conversion {
                    Some(Str) => vm.to_str(&self.pop_value())?.into_object(),
                    Some(Repr) => vm.to_repr(&self.pop_value())?.into_object(),
                    Some(Ascii) => self.pop_value(), // TODO
                    None => self.pop_value(),
                };

                let spec = vm.new_str(spec.clone());
                let formatted = vm.call_method(&value, "__format__", vec![spec])?;
                self.push_value(formatted);
                Ok(None)
            }
            bytecode::Instruction::PopException {} => {
                let block = self.pop_block().unwrap(); // this asserts that the block is_some.
                if let BlockType::ExceptHandler = block.typ {
                    assert!(vm.pop_exception().is_some());
                    Ok(None)
                } else {
                    panic!("Block type must be ExceptHandler here.")
                }
            }
        }
    }

    fn get_elements(
        &self,
        vm: &VirtualMachine,
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

    fn import(&self, vm: &VirtualMachine, module: &str, symbol: &Option<String>) -> FrameResult {
        let module = vm.import(module)?;

        // If we're importing a symbol, look it up and use it, otherwise construct a module and return
        // that
        let obj = match symbol {
            Some(symbol) => vm.get_attribute(module, symbol.as_str()).map_err(|_| {
                let import_error = vm.context().exceptions.import_error.clone();
                vm.new_exception(import_error, format!("cannot import name '{}'", symbol))
            }),
            None => Ok(module),
        };

        // Push module on stack:
        self.push_value(obj?);
        Ok(None)
    }

    fn import_star(&self, vm: &VirtualMachine, module: &str) -> FrameResult {
        let module = vm.import(module)?;

        // Grab all the names from the module and put them in the context
        if let Some(dict) = &module.dict {
            for (k, v) in dict {
                self.scope.store_name(&vm, &objstr::get_value(&k), v);
            }
        }
        Ok(None)
    }

    // Unwind all blocks:
    fn unwind_blocks(&self, vm: &VirtualMachine) -> Option<PyObjectRef> {
        while let Some(block) = self.pop_block() {
            match block.typ {
                BlockType::Loop { .. } => {}
                BlockType::TryExcept { .. } => {
                    // TODO: execute finally handler
                }
                BlockType::With {
                    context_manager, ..
                } => {
                    match self.with_exit(vm, &context_manager, None) {
                        Ok(..) => {}
                        Err(exc) => {
                            // __exit__ went wrong,
                            return Some(exc);
                        }
                    }
                }
                BlockType::ExceptHandler => {
                    vm.pop_exception();
                }
            }
        }

        None
    }

    fn unwind_loop(&self, vm: &VirtualMachine) -> Block {
        loop {
            let block = self.current_block().expect("not in a loop");
            match block.typ {
                BlockType::Loop { .. } => break block,
                BlockType::TryExcept { .. } => {
                    // TODO: execute finally handler
                }
                BlockType::With {
                    context_manager, ..
                } => match self.with_exit(vm, &context_manager, None) {
                    Ok(..) => {}
                    Err(exc) => {
                        panic!("Exception in with __exit__ {:?}", exc);
                    }
                },
                BlockType::ExceptHandler => {
                    vm.pop_exception();
                }
            }

            self.pop_block();
        }
    }

    fn unwind_exception(&self, vm: &VirtualMachine, exc: PyObjectRef) -> Option<PyObjectRef> {
        // unwind block stack on exception and find any handlers:
        while let Some(block) = self.pop_block() {
            match block.typ {
                BlockType::TryExcept { handler } => {
                    self.push_block(BlockType::ExceptHandler {});
                    self.push_value(exc.clone());
                    vm.push_exception(exc);
                    self.jump(handler);
                    return None;
                }
                BlockType::With {
                    end,
                    context_manager,
                } => {
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
                BlockType::Loop { .. } => {}
                // Exception was already poped on Raised.
                BlockType::ExceptHandler => {}
            }
        }
        Some(exc)
    }

    fn with_exit(
        &self,
        vm: &VirtualMachine,
        context_manager: &PyObjectRef,
        exc: Option<PyObjectRef>,
    ) -> PyResult {
        // Assume top of stack is __exit__ method:
        // TODO: do we want to put the exit call on the stack?
        // let exit_method = self.pop_value();
        // let args = PyFuncArgs::default();
        // TODO: what happens when we got an error during handling exception?
        let args = if let Some(exc) = exc {
            let exc_type = exc.class().into_object();
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
        }
        Ok(None)
    }

    fn delete_name(&self, vm: &VirtualMachine, name: &str) -> FrameResult {
        match self.scope.delete_name(vm, name) {
            Ok(_) => Ok(None),
            Err(_) => Err(vm.new_name_error(format!("name '{}' is not defined", name))),
        }
    }

    fn load_name(
        &self,
        vm: &VirtualMachine,
        name: &str,
        name_scope: &bytecode::NameScope,
    ) -> FrameResult {
        let optional_value = match name_scope {
            bytecode::NameScope::Global => self.scope.load_global(vm, name),
            bytecode::NameScope::NonLocal => self.scope.load_cell(vm, name),
            bytecode::NameScope::Local => self.scope.load_name(&vm, name),
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

    fn execute_store_subscript(&self, vm: &VirtualMachine) -> FrameResult {
        let idx = self.pop_value();
        let obj = self.pop_value();
        let value = self.pop_value();
        obj.set_item(idx, value, vm)?;
        Ok(None)
    }

    fn execute_delete_subscript(&self, vm: &VirtualMachine) -> FrameResult {
        let idx = self.pop_value();
        let obj = self.pop_value();
        obj.del_item(idx, vm)?;
        Ok(None)
    }

    fn jump(&self, label: bytecode::Label) {
        let target_pc = self.code.label_map[&label];
        trace!("program counter from {:?} to {:?}", self.lasti, target_pc);
        *self.lasti.borrow_mut() = target_pc;
    }

    fn execute_binop(
        &self,
        vm: &VirtualMachine,
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
            bytecode::BinaryOperator::Subscript => a_ref.get_item(b_ref, vm),
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

    fn _in(&self, vm: &VirtualMachine, needle: PyObjectRef, haystack: PyObjectRef) -> PyResult {
        let found = vm._membership(haystack.clone(), needle)?;
        Ok(vm.ctx.new_bool(objbool::boolval(vm, found)?))
    }

    fn _not_in(&self, vm: &VirtualMachine, needle: PyObjectRef, haystack: PyObjectRef) -> PyResult {
        let found = vm._membership(haystack.clone(), needle)?;
        Ok(vm.ctx.new_bool(!objbool::boolval(vm, found)?))
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
            bytecode::ComparisonOperator::IsNot => self._is_not(vm, a, b)?,
            bytecode::ComparisonOperator::In => self._in(vm, a, b)?,
            bytecode::ComparisonOperator::NotIn => self._not_in(vm, a, b)?,
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

    pub fn get_lineno(&self) -> ast::Location {
        self.code.locations[*self.lasti.borrow()].clone()
    }

    fn push_block(&self, typ: BlockType) {
        self.blocks.borrow_mut().push(Block {
            typ,
            level: self.stack.borrow().len(),
        });
    }

    fn pop_block(&self) -> Option<Block> {
        let block = self.blocks.borrow_mut().pop()?;
        self.stack.borrow_mut().truncate(block.level);
        Some(block)
    }

    fn current_block(&self) -> Option<Block> {
        self.blocks.borrow().last().cloned()
    }

    pub fn push_value(&self, obj: PyObjectRef) {
        self.stack.borrow_mut().push(obj);
    }

    fn pop_value(&self) -> PyObjectRef {
        self.stack.borrow_mut().pop().unwrap()
    }

    fn pop_multiple(&self, count: usize) -> Vec<PyObjectRef> {
        let mut objs: Vec<PyObjectRef> = Vec::new();
        for _x in 0..count {
            objs.push(self.pop_value());
        }
        objs.reverse();
        objs
    }

    fn last_value(&self) -> PyObjectRef {
        self.stack.borrow().last().unwrap().clone()
    }

    fn nth_value(&self, depth: usize) -> PyObjectRef {
        let stack = self.stack.borrow_mut();
        stack[stack.len() - depth - 1].clone()
    }

    fn get_exception(&self, vm: &VirtualMachine, none_allowed: bool) -> PyResult {
        let exception = self.pop_value();
        if none_allowed && vm.get_none().is(&exception)
            || objtype::isinstance(&exception, &vm.ctx.exceptions.base_exception_type)
        {
            Ok(exception)
        } else if let Ok(exception) = PyClassRef::try_from_object(vm, exception) {
            if objtype::issubclass(&exception, &vm.ctx.exceptions.base_exception_type) {
                let exception = vm.new_empty_exception(exception)?;
                Ok(exception)
            } else {
                let msg = format!(
                    "Can only raise BaseException derived types, not {}",
                    exception
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
