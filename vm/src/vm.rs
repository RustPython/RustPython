/*
 * Implement virtual machine to run instructions.
 * See also:
 *   https://github.com/ProgVal/pythonvm-rust/blob/master/src/processor/mod.rs
 */

extern crate rustpython_parser;

use self::rustpython_parser::ast;
use std::collections::hash_map::HashMap;

use super::builtins;
use super::bytecode;
use super::frame::{copy_code, Block, Frame};
use super::import::import;
use super::obj::objbool;
use super::obj::objiter;
use super::obj::objlist;
use super::obj::objobject;
use super::obj::objstr;
use super::obj::objtuple;
use super::obj::objtype;
use super::pyobject::{
    AttributeProtocol, DictProtocol, IdProtocol, ParentProtocol, PyContext, PyFuncArgs, PyObject,
    PyObjectKind, PyObjectRef, PyResult, ToRust, TypeProtocol,
};
use super::stdlib;
use super::sysmodule;

// use objects::objects;

// Objects are live when they are on stack, or referenced by a name (for now)

pub struct VirtualMachine {
    frames: Vec<Frame>,
    builtins: PyObjectRef,
    pub sys_module: PyObjectRef,
    pub stdlib_inits: HashMap<String, stdlib::StdlibInitFunc>,
    pub ctx: PyContext,
}

impl VirtualMachine {
    pub fn run_code_obj(&mut self, code: PyObjectRef, scope: PyObjectRef) -> PyResult {
        let frame = Frame::new(code, scope);
        self.run_frame(frame)
    }

    pub fn new_str(&self, s: String) -> PyObjectRef {
        self.ctx.new_str(s)
    }

    pub fn new_bool(&self, b: bool) -> PyObjectRef {
        self.ctx.new_bool(b)
    }

    pub fn new_dict(&self) -> PyObjectRef {
        self.ctx.new_dict()
    }

    pub fn new_exception(&mut self, exc_type: PyObjectRef, msg: String) -> PyObjectRef {
        // TODO: maybe there is a clearer way to create an instance:
        info!("New exception created: {}", msg);
        let pymsg = self.new_str(msg);
        let args: Vec<PyObjectRef> = vec![pymsg];
        let args = PyFuncArgs {
            args: args,
            kwargs: vec![],
        };

        // Call function:
        let exception = self.invoke(exc_type, args).unwrap();
        exception
    }

    pub fn new_type_error(&mut self, msg: String) -> PyObjectRef {
        let type_error = self.ctx.exceptions.type_error.clone();
        self.new_exception(type_error, msg)
    }

    pub fn new_value_error(&mut self, msg: String) -> PyObjectRef {
        let value_error = self.ctx.exceptions.value_error.clone();
        self.new_exception(value_error, msg)
    }

    pub fn new_scope(&mut self) -> PyObjectRef {
        let parent_scope = self.current_frame_mut().locals.clone();
        self.ctx.new_scope(Some(parent_scope))
    }

    pub fn get_none(&self) -> PyObjectRef {
        self.ctx.none()
    }

    pub fn new_bound_method(&self, function: PyObjectRef, object: PyObjectRef) -> PyObjectRef {
        self.ctx.new_bound_method(function, object)
    }

    pub fn get_type(&self) -> PyObjectRef {
        self.ctx.type_type()
    }

    pub fn get_object(&self) -> PyObjectRef {
        self.ctx.object()
    }

    pub fn get_locals(&self) -> PyObjectRef {
        let scope = &self.frames.last().unwrap().locals;
        scope.clone()
        /*
        match (*scope).kind {
            PyObjectKind::Scope { scope } => { scope.locals.clone() },
            _ => { panic!("Should be scope") },
        } // .clone()
        */
    }

    pub fn context(&self) -> &PyContext {
        &self.ctx
    }

    pub fn new() -> VirtualMachine {
        let ctx = PyContext::new();
        let builtins = builtins::make_module(&ctx);
        let sysmod = sysmodule::mk_module(&ctx);
        // Add builtins as builtins module:
        // sysmod.get_attr("modules").unwrap().set_item("builtins", builtins.clone());
        let stdlib_inits = stdlib::get_module_inits();
        VirtualMachine {
            frames: vec![],
            builtins: builtins,
            sys_module: sysmod,
            stdlib_inits,
            ctx: ctx,
        }
    }

    pub fn get_builtin_scope(&mut self) -> PyObjectRef {
        let a2 = &*self.builtins.borrow();
        match a2.kind {
            PyObjectKind::Module { name: _, ref dict } => dict.clone(),
            _ => {
                panic!("OMG");
            }
        }
    }

    // Container of the virtual machine state:
    pub fn to_str(&mut self, obj: PyObjectRef) -> PyResult {
        self.call_method(&obj, "__str__", vec![])
    }

    pub fn to_repr(&mut self, obj: PyObjectRef) -> PyResult {
        self.call_method(&obj, "__repr__", vec![])
    }

    pub fn current_frame(&self) -> &Frame {
        self.frames.last().unwrap()
    }

    fn current_frame_mut(&mut self) -> &mut Frame {
        self.frames.last_mut().unwrap()
    }

    fn pop_frame(&mut self) -> Frame {
        self.frames.pop().unwrap()
    }

    fn push_block(&mut self, block: Block) {
        self.current_frame_mut().push_block(block);
    }

    fn pop_block(&mut self) -> Option<Block> {
        self.current_frame_mut().pop_block()
    }

    fn last_block(&self) -> &Block {
        self.current_frame().last_block()
    }

    fn with_exit(&mut self, context_manager: &PyObjectRef, exc: Option<PyObjectRef>) -> PyResult {
        // Assume top of stack is __exit__ method:
        // TODO: do we want to put the exit call on the stack?
        // let exit_method = self.pop_value();
        // let args = PyFuncArgs::default();
        // TODO: what happens when we got an error during handling exception?
        let args = if let Some(exc) = exc {
            let exc_type = exc.typ();
            let exc_val = exc.clone();
            let exc_tb = self.ctx.none(); // TODO: retrieve traceback?
            vec![exc_type, exc_val, exc_tb]
        } else {
            let exc_type = self.ctx.none();
            let exc_val = self.ctx.none();
            let exc_tb = self.ctx.none();
            vec![exc_type, exc_val, exc_tb]
        };
        self.call_method(context_manager, "__exit__", args)
    }

    // Unwind all blocks:
    fn unwind_blocks(&mut self) -> Option<PyObjectRef> {
        loop {
            let block = self.pop_block();
            match block {
                Some(Block::Loop { .. }) => {}
                Some(Block::TryExcept { .. }) => {
                    // TODO: execute finally handler
                }
                Some(Block::With {
                    end: _,
                    context_manager,
                }) => {
                    match self.with_exit(&context_manager, None) {
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

    fn unwind_loop(&mut self) -> Block {
        loop {
            let block = self.pop_block();
            match block {
                Some(Block::Loop { start: _, end: __ }) => break block.unwrap(),
                Some(Block::TryExcept { .. }) => {
                    // TODO: execute finally handler
                }
                Some(Block::With {
                    end: _,
                    context_manager,
                }) => match self.with_exit(&context_manager, None) {
                    Ok(..) => {}
                    Err(exc) => {
                        panic!("Exception in with __exit__ {:?}", exc);
                    }
                },
                None => panic!("No block to break / continue"),
            }
        }
    }

    fn unwind_exception(&mut self, exc: PyObjectRef) -> Option<PyObjectRef> {
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
                    match self.with_exit(&context_manager, Some(exc.clone())) {
                        Ok(exit_action) => {
                            match objbool::boolval(self, exit_action) {
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

    fn push_value(&mut self, obj: PyObjectRef) {
        self.current_frame_mut().push_value(obj);
    }

    fn pop_value(&mut self) -> PyObjectRef {
        self.current_frame_mut().pop_value()
    }

    fn pop_multiple(&mut self, count: usize) -> Vec<PyObjectRef> {
        self.current_frame_mut().pop_multiple(count)
    }

    fn last_value(&self) -> PyObjectRef {
        self.current_frame().last_value()
    }

    fn nth_value(&self, i: usize) -> PyObjectRef {
        self.current_frame().nth_value(i)
    }

    fn store_name(&mut self, name: &str) -> Option<PyResult> {
        let obj = self.pop_value();
        self.current_frame_mut().locals.set_item(name, obj);
        None
    }

    fn delete_name(&mut self, name: &str) -> Option<PyResult> {
        let locals = match self.current_frame().locals.borrow().kind {
            PyObjectKind::Scope { ref scope } => scope.locals.clone(),
            _ => panic!("We really expect our scope to be a scope!"),
        };

        // Assume here that locals is a dict
        let name = self.ctx.new_str(name.to_string());
        match self.call_method(&locals, "__delitem__", vec![name]) {
            Ok(_) => None,
            err => Some(err),
        }
    }

    fn load_name(&mut self, name: &str) -> Option<PyResult> {
        // Lookup name in scope and put it onto the stack!
        let mut scope = self.current_frame().locals.clone();
        loop {
            if scope.contains_key(name) {
                let obj = scope.get_item(name).unwrap();
                self.push_value(obj);
                break None;
            } else if scope.has_parent() {
                scope = scope.get_parent();
            } else {
                let name_error_type = self.ctx.exceptions.name_error.clone();
                let msg = format!("Has not attribute '{}'", name);
                let name_error = self.new_exception(name_error_type, msg);
                break Some(Err(name_error));
            }
        }
    }

    fn run_frame(&mut self, frame: Frame) -> PyResult {
        self.frames.push(frame);
        let filename = if let Some(source_path) = &self.current_frame().code.source_path {
            source_path.to_string()
        } else {
            "<unknown>".to_string()
        };

        // This is the name of the object being run:
        let run_obj_name = &self.current_frame().code.obj_name.to_string();

        // Execute until return or exception:
        let value = loop {
            let lineno = self.get_lineno();
            let result = self.execute_instruction();
            match result {
                None => {}
                Some(Ok(value)) => {
                    break Ok(value);
                }
                Some(Err(exception)) => {
                    // unwind block stack on exception and find any handlers.
                    // Add an entry in the traceback:
                    assert!(objtype::isinstance(
                        &exception,
                        self.ctx.exceptions.base_exception_type.clone()
                    ));
                    let traceback = self
                        .get_attribute(exception.clone(), &"__traceback__".to_string())
                        .unwrap();
                    trace!("Adding to traceback: {:?} {:?}", traceback, lineno);
                    let pos = self.ctx.new_tuple(vec![
                        self.ctx.new_str(filename.clone()),
                        self.ctx.new_int(lineno.get_row() as i32),
                        self.ctx.new_str(run_obj_name.clone()),
                    ]);
                    objlist::list_append(
                        self,
                        PyFuncArgs {
                            args: vec![traceback, pos],
                            kwargs: vec![],
                        },
                    )
                    .unwrap();
                    // exception.__trace
                    match self.unwind_exception(exception) {
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

        self.pop_frame();
        value
    }

    fn subscript(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_method(&a, "__getitem__", vec![b])
    }

    fn execute_store_subscript(&mut self) -> Option<PyResult> {
        let idx = self.pop_value();
        let obj = self.pop_value();
        let value = self.pop_value();
        let a2 = &mut *obj.borrow_mut();
        let result = match &mut a2.kind {
            PyObjectKind::List { ref mut elements } => {
                objlist::set_item(self, elements, idx, value)
            }
            _ => Err(self.new_type_error(format!(
                "TypeError: __setitem__ assign type {:?} with index {:?} is not supported (yet?)",
                obj, idx
            ))),
        };

        match result {
            Ok(_) => None,
            Err(value) => Some(Err(value)),
        }
    }

    fn execute_delete_subscript(&mut self) -> Option<PyResult> {
        let idx = self.pop_value();
        let obj = self.pop_value();
        match self.call_method(&obj, "__delitem__", vec![idx]) {
            Ok(_) => None,
            err => Some(err),
        }
    }

    fn _sub(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_method(&a, "__sub__", vec![b])
    }

    fn _add(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_method(&a, "__add__", vec![b])
    }

    fn _mul(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_method(&a, "__mul__", vec![b])
    }

    fn _div(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_method(&a, "__truediv__", vec![b])
    }

    pub fn call_method(
        &mut self,
        obj: &PyObjectRef,
        method_name: &str,
        args: Vec<PyObjectRef>,
    ) -> PyResult {
        let func = self.get_attribute(obj.clone(), method_name)?;
        let args = PyFuncArgs {
            args: args,
            kwargs: vec![],
        };
        self.invoke(func, args)
    }

    fn _pow(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_method(&a, "__pow__", vec![b])
    }

    fn _modulo(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_method(&a, "__mod__", vec![b])
    }

    fn _xor(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_method(&a, "__xor__", vec![b])
    }

    fn _or(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_method(&a, "__or__", vec![b])
    }

    fn _and(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_method(&a, "__and__", vec![b])
    }

    fn execute_binop(&mut self, op: &bytecode::BinaryOperator) -> Option<PyResult> {
        let b_ref = self.pop_value();
        let a_ref = self.pop_value();
        // TODO: if the left hand side provides __add__, invoke that function.
        //
        let result = match op {
            &bytecode::BinaryOperator::Subtract => self._sub(a_ref, b_ref),
            &bytecode::BinaryOperator::Add => self._add(a_ref, b_ref),
            &bytecode::BinaryOperator::Multiply => self._mul(a_ref, b_ref),
            &bytecode::BinaryOperator::Power => self._pow(a_ref, b_ref),
            &bytecode::BinaryOperator::Divide => self._div(a_ref, b_ref),
            &bytecode::BinaryOperator::Subscript => self.subscript(a_ref, b_ref),
            &bytecode::BinaryOperator::Modulo => self._modulo(a_ref, b_ref),
            &bytecode::BinaryOperator::Xor => self._xor(a_ref, b_ref),
            &bytecode::BinaryOperator::Or => self._or(a_ref, b_ref),
            &bytecode::BinaryOperator::And => self._and(a_ref, b_ref),
            _ => panic!("NOT IMPL {:?}", op),
        };
        match result {
            Ok(value) => {
                self.push_value(value);
                None
            }
            Err(value) => Some(Err(value)),
        }
    }

    fn execute_unop(&mut self, op: &bytecode::UnaryOperator) -> Option<PyResult> {
        let a = self.pop_value();
        let result = match op {
            &bytecode::UnaryOperator::Minus => {
                // TODO:
                // self.invoke('__neg__'
                match a.borrow().kind {
                    PyObjectKind::Integer { value: ref value1 } => Ok(self.ctx.new_int(-*value1)),
                    PyObjectKind::Float { value: ref value1 } => Ok(self.ctx.new_float(-*value1)),
                    _ => panic!("Not impl {:?}", a),
                }
            }
            &bytecode::UnaryOperator::Not => match objbool::boolval(self, a) {
                Ok(result) => Ok(self.ctx.new_bool(!result)),
                Err(err) => Err(err),
            },
            _ => panic!("Not impl {:?}", op),
        };
        match result {
            Ok(value) => {
                self.push_value(value);
                None
            }
            Err(value) => Some(Err(value)),
        }
    }

    fn _eq(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_method(&a, "__eq__", vec![b])
    }

    fn _ne(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_method(&a, "__ne__", vec![b])
    }

    fn _lt(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        let b2 = &*b.borrow();
        let a2 = &*a.borrow();
        let result_bool = a2 < b2;
        let result = self.ctx.new_bool(result_bool);
        Ok(result)
    }

    fn _le(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        let b2 = &*b.borrow();
        let a2 = &*a.borrow();
        let result_bool = a2 <= b2;
        let result = self.ctx.new_bool(result_bool);
        Ok(result)
    }

    fn _gt(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        let b2 = &*b.borrow();
        let a2 = &*a.borrow();
        let result_bool = a2 > b2;
        let result = self.ctx.new_bool(result_bool);
        Ok(result)
    }

    fn _ge(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        let b2 = &*b.borrow();
        let a2 = &*a.borrow();
        let result_bool = a2 >= b2;
        let result = self.ctx.new_bool(result_bool);
        Ok(result)
    }

    fn _id(&self, a: PyObjectRef) -> usize {
        a.get_id()
    }

    // https://docs.python.org/3/reference/expressions.html#membership-test-operations
    fn _membership(&mut self, needle: PyObjectRef, haystack: &PyObjectRef) -> PyResult {
        self.call_method(&haystack, "__contains__", vec![needle])
        // TODO: implement __iter__ and __getitem__ cases when __contains__ is
        // not implemented.
    }

    fn _in(&mut self, needle: PyObjectRef, haystack: PyObjectRef) -> PyResult {
        match self._membership(needle, &haystack) {
            Ok(found) => Ok(found),
            Err(_) => Err(self.new_type_error(format!(
                "{} has no __contains__ method",
                objtype::get_type_name(&haystack.typ())
            ))),
        }
    }

    fn _not_in(&mut self, needle: PyObjectRef, haystack: PyObjectRef) -> PyResult {
        match self._membership(needle, &haystack) {
            Ok(found) => Ok(self.ctx.new_bool(!objbool::get_value(&found))),
            Err(_) => Err(self.new_type_error(format!(
                "{} has no __contains__ method",
                objtype::get_type_name(&haystack.typ())
            ))),
        }
    }

    fn _is(&self, a: PyObjectRef, b: PyObjectRef) -> bool {
        // Pointer equal:
        a.is(&b)
    }

    fn _is_not(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        let result_bool = !a.is(&b);
        let result = self.ctx.new_bool(result_bool);
        Ok(result)
    }

    fn execute_compare(&mut self, op: &bytecode::ComparisonOperator) -> Option<PyResult> {
        let b = self.pop_value();
        let a = self.pop_value();
        let result = match op {
            &bytecode::ComparisonOperator::Equal => self._eq(a, b),
            &bytecode::ComparisonOperator::NotEqual => self._ne(a, b),
            &bytecode::ComparisonOperator::Less => self._lt(a, b),
            &bytecode::ComparisonOperator::LessOrEqual => self._le(a, b),
            &bytecode::ComparisonOperator::Greater => self._gt(a, b),
            &bytecode::ComparisonOperator::GreaterOrEqual => self._ge(a, b),
            &bytecode::ComparisonOperator::Is => Ok(self.ctx.new_bool(self._is(a, b))),
            &bytecode::ComparisonOperator::IsNot => self._is_not(a, b),
            &bytecode::ComparisonOperator::In => self._in(a, b),
            &bytecode::ComparisonOperator::NotIn => self._not_in(a, b),
        };
        match result {
            Ok(value) => {
                self.push_value(value);
                None
            }
            Err(value) => Some(Err(value)),
        }
    }

    pub fn invoke(&mut self, func_ref: PyObjectRef, args: PyFuncArgs) -> PyResult {
        trace!("Invoke: {:?} {:?}", func_ref, args);
        match func_ref.borrow().kind {
            PyObjectKind::RustFunction { function } => function(self, args),
            PyObjectKind::Function {
                ref code,
                ref scope,
                ref defaults,
            } => self.invoke_python_function(code, scope, defaults, args),
            PyObjectKind::Class {
                name: _,
                dict: _,
                mro: _,
            } => objtype::call(self, func_ref.clone(), args),
            PyObjectKind::BoundMethod {
                ref function,
                ref object,
            } => self.invoke(function.clone(), args.insert(object.clone())),
            PyObjectKind::Instance { .. } => objobject::call(self, args.insert(func_ref.clone())),
            ref kind => {
                unimplemented!("invoke unimplemented for: {:?}", kind);
            }
        }
    }

    fn invoke_python_function(
        &mut self,
        code: &PyObjectRef,
        scope: &PyObjectRef,
        defaults: &PyObjectRef,
        args: PyFuncArgs,
    ) -> PyResult {
        let code_object = copy_code(code.clone());
        let scope = self.ctx.new_scope(Some(scope.clone()));
        self.fill_scope_from_args(&code_object, &scope, args, defaults)?;

        let frame = Frame::new(code.clone(), scope);
        self.run_frame(frame)
    }

    fn fill_scope_from_args(
        &mut self,
        code_object: &bytecode::CodeObject,
        scope: &PyObjectRef,
        args: PyFuncArgs,
        defaults: &PyObjectRef,
    ) -> Result<(), PyObjectRef> {
        let nargs = args.args.len();
        let nexpected_args = code_object.arg_names.len();

        // This parses the arguments from args and kwargs into
        // the proper variables keeping into account default values
        // and starargs and kwargs.
        // See also: PyEval_EvalCodeWithName in cpython:
        // https://github.com/python/cpython/blob/master/Python/ceval.c#L3681

        let n = if nargs > nexpected_args {
            nexpected_args
        } else {
            nargs
        };

        // Copy positional arguments into local variables
        for i in 0..n {
            let arg_name = &code_object.arg_names[i];
            let arg = &args.args[i];
            scope.set_item(arg_name, arg.clone());
        }

        // Pack other positional arguments in to *args:
        if let Some(vararg_name) = &code_object.varargs {
            let mut last_args = vec![];
            for i in n..nargs {
                let arg = &args.args[i];
                last_args.push(arg.clone());
            }
            let vararg_value = self.ctx.new_tuple(last_args);
            scope.set_item(vararg_name, vararg_value);
        } else {
            // Check the number of positional arguments
            if nargs > nexpected_args {
                return Err(self.new_type_error(format!(
                    "Expected {} arguments (got: {})",
                    nexpected_args, nargs
                )));
            }
        }

        // Do we support `**kwargs` ?
        let kwargs = if let Some(name) = &code_object.varkeywords {
            let d = self.new_dict();
            scope.set_item(&name, d.clone());
            Some(d)
        } else {
            None
        };

        // Handle keyword arguments
        for (name, value) in args.kwargs {
            // Check if we have a parameter with this name:
            if code_object.arg_names.contains(&name) || code_object.kwonlyarg_names.contains(&name)
            {
                if scope.contains_key(&name) {
                    return Err(
                        self.new_type_error(format!("Got multiple values for argument '{}'", name))
                    );
                }

                scope.set_item(&name, value);
            } else if let Some(d) = &kwargs {
                d.set_item(&name, value);
            } else {
                return Err(
                    self.new_type_error(format!("Got an unexpected keyword argument '{}'", name))
                );
            }
        }

        // Add missing positional arguments, if we have fewer positional arguments than the
        // function definition calls for
        if nargs < nexpected_args {
            let available_defaults = match defaults.borrow().kind {
                PyObjectKind::Tuple { ref elements } => elements.clone(),
                PyObjectKind::None => vec![],
                _ => panic!("function defaults not tuple or None"),
            };

            // Given the number of defaults available, check all the arguments for which we
            // _don't_ have defaults; if any are missing, raise an exception
            let required_args = nexpected_args - available_defaults.len();
            let mut missing = vec![];
            for i in 0..required_args {
                let variable_name = &code_object.arg_names[i];
                if !scope.contains_key(variable_name) {
                    missing.push(variable_name)
                }
            }
            if !missing.is_empty() {
                return Err(self.new_type_error(format!(
                    "Missing {} required positional arguments: {:?}",
                    missing.len(),
                    missing
                )));
            }

            // We have sufficient defaults, so iterate over the corresponding names and use
            // the default if we don't already have a value
            let mut default_index = 0;
            for i in required_args..nexpected_args {
                let arg_name = &code_object.arg_names[i];
                if !scope.contains_key(arg_name) {
                    scope.set_item(arg_name, available_defaults[default_index].clone());
                }
                default_index += 1;
            }
        };

        // Check if kw only arguments are all present:
        let kwdefs: HashMap<String, String> = HashMap::new();
        for arg_name in &code_object.kwonlyarg_names {
            if !scope.contains_key(arg_name) {
                if kwdefs.contains_key(arg_name) {
                    // If not yet specified, take the default value
                    unimplemented!();
                } else {
                    // No default value and not specified.
                    return Err(self.new_type_error(format!(
                        "Missing required kw only argument: '{}'",
                        arg_name
                    )));
                }
            }
        }

        Ok(())
    }

    fn import(&mut self, module: &str, symbol: &Option<String>) -> Option<PyResult> {
        let obj = match import(self, &module.to_string(), symbol) {
            Ok(value) => value,
            Err(value) => return Some(Err(value)),
        };

        // Push module on stack:
        self.push_value(obj);
        None
    }

    pub fn get_attribute(&mut self, obj: PyObjectRef, attr_name: &str) -> PyResult {
        objtype::get_attribute(self, obj.clone(), attr_name)
    }

    fn load_attr(&mut self, attr_name: &str) -> Option<PyResult> {
        let parent = self.pop_value();
        match self.get_attribute(parent, attr_name) {
            Ok(obj) => {
                self.push_value(obj);
                None
            }
            Err(err) => Some(Err(err)),
        }
    }

    fn store_attr(&mut self, attr_name: &str) -> Option<PyResult> {
        let parent = self.pop_value();
        let value = self.pop_value();
        parent.set_attr(attr_name, value);
        None
    }

    fn delete_attr(&mut self, attr_name: &str) -> Option<PyResult> {
        let parent = self.pop_value();
        let name = self.ctx.new_str(attr_name.to_string());
        match self.call_method(&parent, "__delattr__", vec![name]) {
            Ok(_) => None,
            err => Some(err),
        }
    }

    fn unwrap_constant(&self, value: &bytecode::Constant) -> PyObjectRef {
        match *value {
            bytecode::Constant::Integer { ref value } => self.ctx.new_int(*value),
            bytecode::Constant::Float { ref value } => self.ctx.new_float(*value),
            bytecode::Constant::String { ref value } => self.new_str(value.clone()),
            bytecode::Constant::Boolean { ref value } => self.new_bool(value.clone()),
            bytecode::Constant::Code { ref code } => {
                PyObject::new(PyObjectKind::Code { code: code.clone() }, self.get_type())
            }
            bytecode::Constant::Tuple { ref elements } => self.ctx.new_tuple(
                elements
                    .iter()
                    .map(|value| self.unwrap_constant(value))
                    .collect(),
            ),
            bytecode::Constant::None => self.ctx.none(),
        }
    }

    // Execute a single instruction:
    fn execute_instruction(&mut self) -> Option<PyResult> {
        let instruction = self.current_frame_mut().fetch_instruction();
        {
            trace!("=======");
            /* TODO:
            for frame in self.frames.iter() {
                trace!("  {:?}", frame);
            }
            */
            trace!("  {:?}", self.current_frame());
            trace!("  Executing op code: {:?}", instruction);
            trace!("=======");
        }
        match &instruction {
            bytecode::Instruction::LoadConst { ref value } => {
                let obj = self.unwrap_constant(value);
                self.push_value(obj);
                None
            }
            bytecode::Instruction::Import {
                ref name,
                ref symbol,
            } => self.import(name, symbol),
            bytecode::Instruction::LoadName { ref name } => self.load_name(name),
            bytecode::Instruction::StoreName { ref name } => self.store_name(name),
            bytecode::Instruction::DeleteName { ref name } => self.delete_name(name),
            bytecode::Instruction::StoreSubscript => self.execute_store_subscript(),
            bytecode::Instruction::DeleteSubscript => self.execute_delete_subscript(),
            bytecode::Instruction::Pop => {
                // Pop value from stack and ignore.
                self.pop_value();
                None
            }
            bytecode::Instruction::Duplicate => {
                // Duplicate top of stack
                let value = self.pop_value();
                self.push_value(value.clone());
                self.push_value(value);
                None
            }
            bytecode::Instruction::Rotate { amount } => {
                // Shuffles top of stack amount down
                if amount < &2 {
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
                None
            }
            bytecode::Instruction::BuildList { size } => {
                let elements = self.pop_multiple(*size);
                let list_obj = self.context().new_list(elements);
                self.push_value(list_obj);
                None
            }
            bytecode::Instruction::BuildSet { size } => {
                let elements = self.pop_multiple(*size);
                let py_obj = self.context().new_set(elements);
                self.push_value(py_obj);
                None
            }
            bytecode::Instruction::BuildTuple { size } => {
                let elements = self.pop_multiple(*size);
                let list_obj = self.context().new_tuple(elements);
                self.push_value(list_obj);
                None
            }
            bytecode::Instruction::BuildMap { size } => {
                let mut elements = HashMap::new();
                for _x in 0..*size {
                    let obj = self.pop_value();
                    // XXX: Currently, we only support String keys, so we have to unwrap the
                    // PyObject (and ensure it is a String).
                    let key_pyobj = self.pop_value();
                    let key = match key_pyobj.borrow().kind {
                        PyObjectKind::String { ref value } => value.clone(),
                        ref kind => unimplemented!(
                            "Only strings can be used as dict keys, we saw: {:?}",
                            kind
                        ),
                    };
                    elements.insert(key, obj);
                }
                let map_obj = PyObject::new(
                    PyObjectKind::Dict { elements: elements },
                    self.ctx.dict_type(),
                );
                self.push_value(map_obj);
                None
            }
            bytecode::Instruction::BuildSlice { size } => {
                assert!(*size == 2 || *size == 3);
                let elements = self.pop_multiple(*size);

                let mut out: Vec<Option<i32>> = elements
                    .into_iter()
                    .map(|x| match x.borrow().kind {
                        PyObjectKind::Integer { value } => Some(value),
                        PyObjectKind::None => None,
                        _ => panic!("Expect Int or None as BUILD_SLICE arguments, got {:?}", x),
                    })
                    .collect();

                let start = out[0];
                let stop = out[1];
                let step = if out.len() == 3 { out[2] } else { None };

                let obj = PyObject::new(
                    PyObjectKind::Slice { start, stop, step },
                    self.ctx.type_type(),
                );
                self.push_value(obj);
                None
            }
            bytecode::Instruction::ListAppend { i } => {
                let list_obj = self.nth_value(*i);
                let item = self.pop_value();
                // TODO: objlist::list_append()
                match self.call_method(&list_obj, "append", vec![item]) {
                    Ok(_) => None,
                    Err(err) => Some(Err(err)),
                }
            }
            bytecode::Instruction::SetAdd { i } => {
                let set_obj = self.nth_value(*i);
                let item = self.pop_value();
                match self.call_method(&set_obj, "add", vec![item]) {
                    Ok(_) => None,
                    Err(err) => Some(Err(err)),
                }
            }
            bytecode::Instruction::MapAdd { i } => {
                let dict_obj = self.nth_value(*i + 1);
                let key = self.pop_value();
                let value = self.pop_value();
                match self.call_method(&dict_obj, "__setitem__", vec![key, value]) {
                    Ok(_) => None,
                    Err(err) => Some(Err(err)),
                }
            }
            bytecode::Instruction::BinaryOperation { ref op } => self.execute_binop(op),
            bytecode::Instruction::LoadAttr { ref name } => self.load_attr(name),
            bytecode::Instruction::StoreAttr { ref name } => self.store_attr(name),
            bytecode::Instruction::DeleteAttr { ref name } => self.delete_attr(name),
            bytecode::Instruction::UnaryOperation { ref op } => self.execute_unop(op),
            bytecode::Instruction::CompareOperation { ref op } => self.execute_compare(op),
            bytecode::Instruction::ReturnValue => {
                let value = self.pop_value();
                if let Some(exc) = self.unwind_blocks() {
                    Some(Err(exc))
                } else {
                    Some(Ok(value))
                }
            }
            bytecode::Instruction::YieldValue => {
                let value = self.pop_value();
                unimplemented!("TODO: implement generators: {:?}", value);
            }
            bytecode::Instruction::SetupLoop { start, end } => {
                self.push_block(Block::Loop {
                    start: *start,
                    end: *end,
                });
                None
            }
            bytecode::Instruction::SetupExcept { handler } => {
                self.push_block(Block::TryExcept { handler: *handler });
                None
            }
            bytecode::Instruction::SetupWith { end } => {
                let context_manager = self.pop_value();
                // Call enter:
                match self.call_method(&context_manager, "__enter__", vec![]) {
                    Ok(obj) => {
                        self.push_block(Block::With {
                            end: *end,
                            context_manager: context_manager.clone(),
                        });
                        self.push_value(obj);
                        None
                    }
                    Err(err) => Some(Err(err)),
                }
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
                    match self.with_exit(context_manager, None) {
                        Ok(..) => None,
                        Err(exc) => Some(Err(exc)),
                    }
                } else {
                    panic!("Block stack is incorrect, expected a with block");
                }
            }
            bytecode::Instruction::PopBlock => {
                self.pop_block();
                None
            }
            bytecode::Instruction::GetIter => {
                let iterated_obj = self.pop_value();
                match objiter::get_iter(self, &iterated_obj) {
                    Ok(iter_obj) => {
                        self.push_value(iter_obj);
                        None
                    }
                    Err(err) => Some(Err(err)),
                }
            }
            bytecode::Instruction::ForIter => {
                // The top of stack contains the iterator, lets push it forward:
                let next_obj: PyResult = {
                    let top_of_stack = self.last_value();
                    self.call_method(&top_of_stack, "__next__", vec![])
                };

                // Check the next object:
                match next_obj {
                    Ok(value) => {
                        self.push_value(value);
                        None
                    }
                    Err(next_error) => {
                        // Check if we have stopiteration, or something else:
                        if objtype::isinstance(
                            &next_error,
                            self.ctx.exceptions.stop_iteration.clone(),
                        ) {
                            // Pop iterator from stack:
                            self.pop_value();

                            // End of for loop
                            let end_label = if let Block::Loop { start: _, end } = self.last_block()
                            {
                                *end
                            } else {
                                panic!("Wrong block type")
                            };
                            self.jump(end_label);
                            None
                        } else {
                            Some(Err(next_error))
                        }
                    }
                }
            }
            bytecode::Instruction::MakeFunction { flags } => {
                let _qualified_name = self.pop_value();
                let code_obj = self.pop_value();
                let defaults = if flags.contains(bytecode::FunctionOpArg::HAS_DEFAULTS) {
                    self.pop_value()
                } else {
                    self.get_none()
                };
                // pop argc arguments
                // argument: name, args, globals
                let scope = self.current_frame().locals.clone();
                let obj = self.ctx.new_function(code_obj, scope, defaults);
                self.push_value(obj);
                None
            }
            bytecode::Instruction::CallFunction { count } => {
                let args: Vec<PyObjectRef> = self.pop_multiple(*count);
                let args = PyFuncArgs {
                    args: args,
                    kwargs: vec![],
                };
                let func_ref = self.pop_value();

                // Call function:
                let func_result = self.invoke(func_ref, args);

                match func_result {
                    Ok(value) => {
                        self.push_value(value);
                        None
                    }
                    Err(value) => {
                        // Ripple exception upwards:
                        Some(Err(value))
                    }
                }
            }
            bytecode::Instruction::CallFunctionKw { count } => {
                let kwarg_names = self.pop_value();
                let args: Vec<PyObjectRef> = self.pop_multiple(*count);

                let kwarg_names = kwarg_names
                    .to_vec()
                    .unwrap()
                    .iter()
                    .map(|pyobj| objstr::get_value(pyobj))
                    .collect();
                let args = PyFuncArgs::new(args, kwarg_names);
                let func_ref = self.pop_value();

                // Call function:
                let func_result = self.invoke(func_ref, args);

                match func_result {
                    Ok(value) => {
                        self.push_value(value);
                        None
                    }
                    Err(value) => {
                        // Ripple exception upwards:
                        Some(Err(value))
                    }
                }
            }
            bytecode::Instruction::Jump { target } => {
                self.jump(*target);
                None
            }
            bytecode::Instruction::JumpIf { target } => {
                let obj = self.pop_value();
                match objbool::boolval(self, obj) {
                    Ok(value) => {
                        if value {
                            self.jump(*target);
                        }
                        None
                    }
                    Err(value) => Some(Err(value)),
                }
            }

            bytecode::Instruction::JumpIfFalse { target } => {
                let obj = self.pop_value();
                match objbool::boolval(self, obj) {
                    Ok(value) => {
                        if !value {
                            self.jump(*target);
                        }
                        None
                    }
                    Err(value) => Some(Err(value)),
                }
            }

            bytecode::Instruction::Raise { argc } => {
                let exception = match argc {
                    1 => self.pop_value(),
                    0 | 2 | 3 => panic!("Not implemented!"),
                    _ => panic!("Invalid paramter for RAISE_VARARGS, must be between 0 to 3"),
                };
                if objtype::isinstance(
                    &exception,
                    self.context().exceptions.base_exception_type.clone(),
                ) {
                    info!("Exception raised: {:?}", exception);
                    Some(Err(exception))
                } else {
                    let msg = format!(
                        "Can only raise BaseException derived types, not {:?}",
                        exception
                    );
                    let type_error_type = self.context().exceptions.type_error.clone();
                    let type_error = self.new_exception(type_error_type, msg);
                    Some(Err(type_error))
                }
            }

            bytecode::Instruction::Break => {
                let block = self.unwind_loop();
                if let Block::Loop { start: _, end } = block {
                    self.jump(end);
                }
                None
            }
            bytecode::Instruction::Pass => {
                // Ah, this is nice, just relax!
                None
            }
            bytecode::Instruction::Continue => {
                let block = self.unwind_loop();
                if let Block::Loop { start, end: _ } = block {
                    self.jump(start);
                } else {
                    assert!(false);
                }
                None
            }
            bytecode::Instruction::PrintExpr => {
                let expr = self.pop_value();
                match expr.borrow().kind {
                    PyObjectKind::None => (),
                    _ => {
                        let repr = self.to_repr(expr.clone()).unwrap();
                        builtins::builtin_print(
                            self,
                            PyFuncArgs {
                                args: vec![repr],
                                kwargs: vec![],
                            },
                        )
                        .unwrap();
                    }
                }
                None
            }
            bytecode::Instruction::LoadBuildClass => {
                let rustfunc = PyObject::new(
                    PyObjectKind::RustFunction {
                        function: builtins::builtin_build_class_,
                    },
                    self.ctx.type_type(),
                );
                self.push_value(rustfunc);
                None
            }
            bytecode::Instruction::StoreLocals => {
                let locals = self.pop_value();
                let ref mut frame = self.current_frame_mut();
                match frame.locals.borrow_mut().kind {
                    PyObjectKind::Scope { ref mut scope } => {
                        scope.locals = locals;
                    }
                    _ => panic!("We really expect our scope to be a scope!"),
                }
                None
            }
            bytecode::Instruction::UnpackSequence { size } => {
                let value = self.pop_value();

                let elements = objtuple::get_elements(&value);
                if elements.len() != *size {
                    Some(Err(self.new_value_error(
                        "Wrong number of values to unpack".to_string(),
                    )))
                } else {
                    for element in elements.into_iter().rev() {
                        self.push_value(element);
                    }
                    None
                }
            }
            bytecode::Instruction::UnpackEx { before, after } => {
                let value = self.pop_value();

                let elements = objtuple::get_elements(&value);
                let min_expected = *before + *after;
                if elements.len() < min_expected {
                    Some(Err(self.new_value_error(format!(
                        "Not enough values to unpack (expected at least {}, got {}",
                        min_expected,
                        elements.len()
                    ))))
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
                        .map(|x| x.clone())
                        .collect();
                    let t = self.ctx.new_list(middle_elements);
                    self.push_value(t);

                    // Lastly the first reversed values:
                    for element in elements[..*before].iter().rev() {
                        self.push_value(element.clone());
                    }

                    None
                }
            }
            bytecode::Instruction::Unpack => {
                let value = self.pop_value();

                let elements = objtuple::get_elements(&value);

                for element in elements.into_iter().rev() {
                    self.push_value(element);
                }
                None
            }
        }
    }

    fn jump(&mut self, label: bytecode::Label) {
        let current_frame = self.current_frame_mut();
        let target_pc = current_frame.code.label_map[&label];
        trace!(
            "program counter from {:?} to {:?}",
            current_frame.lasti,
            target_pc
        );
        current_frame.lasti = target_pc;
    }

    fn get_lineno(&self) -> ast::Location {
        self.current_frame().get_lineno()
    }
}

#[cfg(test)]
mod tests {
    use super::super::obj::objint;
    use super::objstr;
    use super::VirtualMachine;

    #[test]
    fn test_add_py_integers() {
        let mut vm = VirtualMachine::new();
        let a = vm.ctx.new_int(33);
        let b = vm.ctx.new_int(12);
        let res = vm._add(a, b).unwrap();
        let value = objint::get_value(&res);
        assert_eq!(value, 45);
    }

    #[test]
    fn test_multiply_str() {
        let mut vm = VirtualMachine::new();
        let a = vm.ctx.new_str(String::from("Hello "));
        let b = vm.ctx.new_int(4);
        let res = vm._mul(a, b).unwrap();
        let value = objstr::get_value(&res);
        assert_eq!(value, String::from("Hello Hello Hello Hello "))
    }
}
