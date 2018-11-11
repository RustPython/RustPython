//! Implement virtual machine to run instructions.
//!
//! See also:
//!   https://github.com/ProgVal/pythonvm-rust/blob/master/src/processor/mod.rs
//!

extern crate rustpython_parser;

use std::collections::hash_map::HashMap;

use super::builtins;
use super::bytecode;
use super::frame::{ExecutionResult, Frame};
use super::obj::objcode::copy_code;
use super::obj::objframe;
use super::obj::objgenerator;
use super::obj::objiter;
use super::obj::objsequence;
use super::obj::objstr;
use super::obj::objtype;
use super::pyobject::{
    AttributeProtocol, DictProtocol, PyContext, PyFuncArgs, PyObjectKind, PyObjectRef, PyResult,
    TypeProtocol,
};
use super::stdlib;
use super::sysmodule;

// use objects::objects;

// Objects are live when they are on stack, or referenced by a name (for now)

/// Top level container of a python virtual machine. In theory you could
/// create more instances of this struct and have them operate fully isolated.
pub struct VirtualMachine {
    builtins: PyObjectRef,
    pub sys_module: PyObjectRef,
    pub stdlib_inits: HashMap<String, stdlib::StdlibInitFunc>,
    pub ctx: PyContext,
    pub frames: Vec<PyObjectRef>,
}

impl VirtualMachine {
    /// Create a new `VirtualMachine` structure.
    pub fn new() -> VirtualMachine {
        let ctx = PyContext::new();
        let builtins = builtins::make_module(&ctx);
        let sysmod = sysmodule::mk_module(&ctx);
        // Add builtins as builtins module:
        // sysmod.get_attr("modules").unwrap().set_item("builtins", builtins.clone());
        let stdlib_inits = stdlib::get_module_inits();
        VirtualMachine {
            builtins: builtins,
            sys_module: sysmod,
            stdlib_inits,
            ctx: ctx,
            frames: vec![],
        }
    }

    pub fn run_code_obj(&mut self, code: PyObjectRef, scope: PyObjectRef) -> PyResult {
        self.run_frame_full(Frame::new(code, scope))
    }

    pub fn run_frame_full(&mut self, frame: Frame) -> PyResult {
        match self.run_frame(frame)? {
            ExecutionResult::Return(value) => Ok(value),
            _ => panic!("Got unexpected result from function"),
        }
    }

    pub fn run_frame(&mut self, frame: Frame) -> Result<ExecutionResult, PyObjectRef> {
        let frame = self.ctx.new_frame(frame);
        self.frames.push(frame.clone());
        let mut frame = objframe::get_value(&frame);
        let result = frame.run(self);
        self.frames.pop();
        result
    }

    /// Create a new python string object.
    pub fn new_str(&self, s: String) -> PyObjectRef {
        self.ctx.new_str(s)
    }

    /// Create a new python bool object.
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

    /// Create a new python ValueError object. Useful for raising errors from
    /// python functions implemented in rust.
    pub fn new_value_error(&mut self, msg: String) -> PyObjectRef {
        let value_error = self.ctx.exceptions.value_error.clone();
        self.new_exception(value_error, msg)
    }

    pub fn new_not_implemented_error(&mut self, msg: String) -> PyObjectRef {
        let value_error = self.ctx.exceptions.not_implemented_error.clone();
        self.new_exception(value_error, msg)
    }

    pub fn new_scope(&mut self, parent_scope: Option<PyObjectRef>) -> PyObjectRef {
        // let parent_scope = self.current_frame_mut().locals.clone();
        self.ctx.new_scope(parent_scope)
    }

    pub fn get_none(&self) -> PyObjectRef {
        self.ctx.none()
    }

    pub fn get_type(&self) -> PyObjectRef {
        self.ctx.type_type()
    }

    pub fn get_object(&self) -> PyObjectRef {
        self.ctx.object()
    }

    pub fn get_locals(&self) -> PyObjectRef {
        // let scope = &self.frames.last().unwrap().locals;
        // scope.clone()
        // TODO: fix this!
        self.get_none()
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
    pub fn to_str(&mut self, obj: &PyObjectRef) -> PyResult {
        self.call_method(&obj, "__str__", vec![])
    }

    pub fn to_pystr(&mut self, obj: &PyObjectRef) -> Result<String, PyObjectRef> {
        let py_str_obj = self.to_str(obj)?;
        Ok(objstr::get_value(&py_str_obj))
    }

    pub fn to_repr(&mut self, obj: &PyObjectRef) -> PyResult {
        self.call_method(obj, "__repr__", vec![])
    }

    pub fn call_get_descriptor(&mut self, attr: PyObjectRef, obj: PyObjectRef) -> PyResult {
        let attr_class = attr.typ();
        if let Some(descriptor) = attr_class.get_attr("__get__") {
            let cls = obj.typ();
            self.invoke(
                descriptor,
                PyFuncArgs {
                    args: vec![attr, obj.clone(), cls],
                    kwargs: vec![],
                },
            )
        } else {
            Ok(attr)
        }
    }

    pub fn call_method(
        &mut self,
        obj: &PyObjectRef,
        method_name: &str,
        args: Vec<PyObjectRef>,
    ) -> PyResult {
        self.call_method_pyargs(
            obj,
            method_name,
            PyFuncArgs {
                args: args,
                kwargs: vec![],
            },
        )
    }

    pub fn call_method_pyargs(
        &mut self,
        obj: &PyObjectRef,
        method_name: &str,
        args: PyFuncArgs,
    ) -> PyResult {
        // This is only used in the vm for magic methods, which use a greatly simplified attribute lookup.
        let cls = obj.typ();
        match cls.get_attr(method_name) {
            Some(func) => {
                trace!(
                    "vm.call_method {:?} {:?} {:?} -> {:?}",
                    obj,
                    cls,
                    method_name,
                    func
                );
                let wrapped = self.call_get_descriptor(func, obj.clone())?;
                self.invoke(wrapped, args)
            }
            None => Err(self.new_type_error(format!("Unsupported method: {}", method_name))),
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
            } => self.call_method_pyargs(&func_ref, "__call__", args),
            PyObjectKind::BoundMethod {
                ref function,
                ref object,
            } => self.invoke(function.clone(), args.insert(object.clone())),
            PyObjectKind::Instance { .. } => self.call_method_pyargs(&func_ref, "__call__", args),
            ref kind => {
                // TODO: is it safe to just invoke __call__ otherwise?
                trace!("invoke __call__ for: {:?}", kind);
                self.call_method_pyargs(&func_ref, "__call__", args)
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
        let code_object = copy_code(code);
        let scope = self.ctx.new_scope(Some(scope.clone()));
        self.fill_scope_from_args(&code_object, &scope, args, defaults)?;

        // Construct frame:
        let frame = Frame::new(code.clone(), scope);

        // If we have a generator, create a new generator
        if code_object.is_generator {
            objgenerator::new_generator(self, frame)
        } else {
            self.run_frame_full(frame)
        }
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
        if let Some(vararg) = &code_object.varargs {
            let mut last_args = vec![];
            for i in n..nargs {
                let arg = &args.args[i];
                last_args.push(arg.clone());
            }
            let vararg_value = self.ctx.new_tuple(last_args);

            // If we have a name (not '*' only) then store it:
            if let Some(vararg_name) = vararg {
                scope.set_item(vararg_name, vararg_value);
            }
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
        let kwargs = if let Some(kwargs) = &code_object.varkeywords {
            let d = self.new_dict();

            // Store when we have a name:
            if let Some(kwargs_name) = kwargs {
                scope.set_item(&kwargs_name, d.clone());
            }

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
                PyObjectKind::Sequence { ref elements } => elements.clone(),
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

    pub fn extract_elements(
        &mut self,
        value: &PyObjectRef,
    ) -> Result<Vec<PyObjectRef>, PyObjectRef> {
        // Extract elements from item, if possible:
        let elements = if objtype::isinstance(value, &self.ctx.tuple_type()) {
            objsequence::get_elements(value).to_vec()
        } else if objtype::isinstance(value, &self.ctx.list_type()) {
            objsequence::get_elements(value).to_vec()
        } else {
            let iter = objiter::get_iter(self, value)?;
            objiter::get_all(self, &iter)?
        };
        Ok(elements)
    }

    // get_attribute should be used for full attribute access (usually from user code).
    pub fn get_attribute(&mut self, obj: PyObjectRef, attr_name: PyObjectRef) -> PyResult {
        trace!("vm.__getattribute__: {:?} {:?}", obj, attr_name);
        self.call_method(&obj, "__getattribute__", vec![attr_name])
    }

    pub fn del_attr(&mut self, obj: &PyObjectRef, attr_name: PyObjectRef) -> PyResult {
        self.call_method(&obj, "__delattr__", vec![attr_name])
    }

    // get_method should be used for internal access to magic methods (by-passing
    // the full getattribute look-up.
    pub fn get_method(&mut self, obj: PyObjectRef, method_name: &str) -> PyResult {
        let cls = obj.typ();
        match cls.get_attr(method_name) {
            Some(method) => self.call_get_descriptor(method, obj.clone()),
            None => {
                Err(self
                    .new_type_error(format!("{:?} object has no method {:?}", obj, method_name)))
            }
        }
    }

    pub fn _sub(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        // 1. Try __sub__, next __rsub__, next, give up
        if let Ok(method) = self.get_method(a.clone(), "__sub__") {
            match self.invoke(
                method,
                PyFuncArgs {
                    args: vec![b.clone()],
                    kwargs: vec![],
                },
            ) {
                Ok(value) => return Ok(value),
                Err(err) => {
                    if !objtype::isinstance(&err, &self.ctx.exceptions.not_implemented_error) {
                        return Err(err);
                    }
                }
            }
        }

        // 2. try __rsub__
        if let Ok(method) = self.get_method(b.clone(), "__rsub__") {
            match self.invoke(
                method,
                PyFuncArgs {
                    args: vec![a.clone()],
                    kwargs: vec![],
                },
            ) {
                Ok(value) => return Ok(value),
                Err(err) => {
                    if !objtype::isinstance(&err, &self.ctx.exceptions.not_implemented_error) {
                        return Err(err);
                    }
                }
            }
        }

        // 3. It all failed :(
        // Cannot sub a and b
        let a_type_name = objtype::get_type_name(&a.typ());
        let b_type_name = objtype::get_type_name(&b.typ());
        Err(self.new_type_error(format!(
            "Unsupported operand types for '-': '{}' and '{}'",
            a_type_name, b_type_name
        )))
    }

    pub fn _add(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_method(&a, "__add__", vec![b])
    }

    pub fn _mul(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_method(&a, "__mul__", vec![b])
    }

    pub fn _div(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_method(&a, "__truediv__", vec![b])
    }

    pub fn _pow(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_method(&a, "__pow__", vec![b])
    }

    pub fn _modulo(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_method(&a, "__mod__", vec![b])
    }

    pub fn _xor(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_method(&a, "__xor__", vec![b])
    }

    pub fn _or(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_method(&a, "__or__", vec![b])
    }

    pub fn _and(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        // 1. Try __and__, next __rand__, next, give up
        if let Ok(method) = self.get_method(a.clone(), "__and__") {
            match self.invoke(
                method,
                PyFuncArgs {
                    args: vec![b.clone()],
                    kwargs: vec![],
                },
            ) {
                Ok(value) => return Ok(value),
                Err(err) => {
                    if !objtype::isinstance(&err, &self.ctx.exceptions.not_implemented_error) {
                        return Err(err);
                    }
                }
            }
        }

        // 2. try __rand__
        if let Ok(method) = self.get_method(b.clone(), "__rand__") {
            match self.invoke(
                method,
                PyFuncArgs {
                    args: vec![a.clone()],
                    kwargs: vec![],
                },
            ) {
                Ok(value) => return Ok(value),
                Err(err) => {
                    if !objtype::isinstance(&err, &self.ctx.exceptions.not_implemented_error) {
                        return Err(err);
                    }
                }
            }
        }

        // 3. It all failed :(
        // Cannot and a and b
        let a_type_name = objtype::get_type_name(&a.typ());
        let b_type_name = objtype::get_type_name(&b.typ());
        Err(self.new_type_error(format!(
            "Unsupported operand types for '&': '{}' and '{}'",
            a_type_name, b_type_name
        )))
    }

    pub fn _eq(&mut self, a: &PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_method(a, "__eq__", vec![b])
    }

    pub fn _ne(&mut self, a: &PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_method(a, "__ne__", vec![b])
    }

    pub fn _lt(&mut self, a: &PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_method(a, "__lt__", vec![b])
    }

    pub fn _le(&mut self, a: &PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_method(a, "__le__", vec![b])
    }

    pub fn _gt(&mut self, a: &PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_method(a, "__gt__", vec![b])
    }

    pub fn _ge(&mut self, a: &PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_method(a, "__ge__", vec![b])
    }
}

#[cfg(test)]
mod tests {
    use super::super::obj::{objint, objstr};
    use super::VirtualMachine;
    use num_bigint::ToBigInt;

    #[test]
    fn test_add_py_integers() {
        let mut vm = VirtualMachine::new();
        let a = vm.ctx.new_int(33_i32.to_bigint().unwrap());
        let b = vm.ctx.new_int(12_i32.to_bigint().unwrap());
        let res = vm._add(a, b).unwrap();
        let value = objint::get_value(&res);
        assert_eq!(value, 45_i32.to_bigint().unwrap());
    }

    #[test]
    fn test_multiply_str() {
        let mut vm = VirtualMachine::new();
        let a = vm.ctx.new_str(String::from("Hello "));
        let b = vm.ctx.new_int(4_i32.to_bigint().unwrap());
        let res = vm._mul(a, b).unwrap();
        let value = objstr::get_value(&res);
        assert_eq!(value, String::from("Hello Hello Hello Hello "))
    }
}
