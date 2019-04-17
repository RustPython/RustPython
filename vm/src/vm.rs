//! Implement virtual machine to run instructions.
//!
//! See also:
//!   https://github.com/ProgVal/pythonvm-rust/blob/master/src/processor/mod.rs
//!

extern crate rustpython_parser;

use std::cell::{Ref, RefCell};
use std::collections::hash_map::HashMap;
use std::collections::hash_set::HashSet;
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard};

use crate::builtins;
use crate::bytecode;
use crate::frame::{ExecutionResult, Frame, FrameRef, Scope};
use crate::function::PyFuncArgs;
use crate::obj::objbool;
use crate::obj::objbuiltinfunc::PyBuiltinFunction;
use crate::obj::objcode::PyCodeRef;
use crate::obj::objdict::PyDictRef;
use crate::obj::objfunction::{PyFunction, PyMethod};
use crate::obj::objgenerator::PyGenerator;
use crate::obj::objiter;
use crate::obj::objsequence;
use crate::obj::objstr::{PyString, PyStringRef};
use crate::obj::objtuple::PyTupleRef;
use crate::obj::objtype;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{
    IdProtocol, ItemProtocol, PyContext, PyObjectRef, PyResult, PyValue, TryFromObject, TryIntoRef,
    TypeProtocol,
};
use crate::stdlib;
use crate::sysmodule;
use num_bigint::BigInt;

// use objects::objects;

// Objects are live when they are on stack, or referenced by a name (for now)

/// Top level container of a python virtual machine. In theory you could
/// create more instances of this struct and have them operate fully isolated.
pub struct VirtualMachine {
    pub builtins: PyObjectRef,
    pub sys_module: PyObjectRef,
    pub stdlib_inits: RefCell<HashMap<String, stdlib::StdlibInitFunc>>,
    pub ctx: PyContext,
    pub frames: RefCell<Vec<FrameRef>>,
    pub wasm_id: Option<String>,
}

impl VirtualMachine {
    /// Create a new `VirtualMachine` structure.
    pub fn new() -> VirtualMachine {
        let ctx = PyContext::new();

        // Hard-core modules:
        let builtins = ctx.new_module("builtins", ctx.new_dict());
        let sysmod = ctx.new_module("sys", ctx.new_dict());

        let stdlib_inits = RefCell::new(stdlib::get_module_inits());
        let vm = VirtualMachine {
            builtins: builtins.clone(),
            sys_module: sysmod.clone(),
            stdlib_inits,
            ctx,
            frames: RefCell::new(vec![]),
            wasm_id: None,
        };

        builtins::make_module(&vm, builtins.clone());
        sysmodule::make_module(&vm, sysmod, builtins);
        vm
    }

    pub fn run_code_obj(&self, code: PyCodeRef, scope: Scope) -> PyResult {
        let frame = Frame::new(code, scope).into_ref(self);
        self.run_frame_full(frame)
    }

    pub fn run_frame_full(&self, frame: FrameRef) -> PyResult {
        match self.run_frame(frame)? {
            ExecutionResult::Return(value) => Ok(value),
            _ => panic!("Got unexpected result from function"),
        }
    }

    pub fn run_frame(&self, frame: FrameRef) -> PyResult<ExecutionResult> {
        self.frames.borrow_mut().push(frame.clone());
        let result = frame.run(self);
        self.frames.borrow_mut().pop();
        result
    }

    pub fn frame_throw(
        &self,
        frame: FrameRef,
        exception: PyObjectRef,
    ) -> PyResult<ExecutionResult> {
        self.frames.borrow_mut().push(frame.clone());
        let result = frame.throw(self, exception);
        self.frames.borrow_mut().pop();
        result
    }

    pub fn current_frame(&self) -> Option<Ref<FrameRef>> {
        let frames = self.frames.borrow();
        if frames.is_empty() {
            None
        } else {
            Some(Ref::map(self.frames.borrow(), |frames| {
                frames.last().unwrap()
            }))
        }
    }

    pub fn current_scope(&self) -> Ref<Scope> {
        let frame = self
            .current_frame()
            .expect("called current_scope but no frames on the stack");
        Ref::map(frame, |f| &f.scope)
    }

    pub fn try_class(&self, module: &str, class: &str) -> PyResult<PyClassRef> {
        let class = self
            .get_attribute(self.import(module)?, class)?
            .downcast()
            .expect("not a class");
        Ok(class)
    }

    pub fn class(&self, module: &str, class: &str) -> PyClassRef {
        let module = self
            .import(module)
            .unwrap_or_else(|_| panic!("unable to import {}", module));
        let class = self
            .get_attribute(module.clone(), class)
            .unwrap_or_else(|_| panic!("module {} has no class {}", module, class));
        class.downcast().expect("not a class")
    }

    /// Create a new python string object.
    pub fn new_str(&self, s: String) -> PyObjectRef {
        self.ctx.new_str(s)
    }

    /// Create a new python int object.
    pub fn new_int<T: Into<BigInt>>(&self, i: T) -> PyObjectRef {
        self.ctx.new_int(i)
    }

    /// Create a new python bool object.
    pub fn new_bool(&self, b: bool) -> PyObjectRef {
        self.ctx.new_bool(b)
    }

    pub fn new_empty_exception(&self, exc_type: PyClassRef) -> PyResult {
        info!("New exception created: no msg");
        let args = PyFuncArgs::default();
        self.invoke(exc_type.into_object(), args)
    }

    pub fn new_exception(&self, exc_type: PyClassRef, msg: String) -> PyObjectRef {
        // TODO: exc_type may be user-defined exception, so we should return PyResult
        // TODO: maybe there is a clearer way to create an instance:
        info!("New exception created: {}", msg);
        let pymsg = self.new_str(msg);
        let args: Vec<PyObjectRef> = vec![pymsg];

        // Call function:
        self.invoke(exc_type.into_object(), args).unwrap()
    }

    pub fn new_attribute_error(&self, msg: String) -> PyObjectRef {
        let attribute_error = self.ctx.exceptions.attribute_error.clone();
        self.new_exception(attribute_error, msg)
    }

    pub fn new_type_error(&self, msg: String) -> PyObjectRef {
        let type_error = self.ctx.exceptions.type_error.clone();
        self.new_exception(type_error, msg)
    }

    pub fn new_unsupported_operand_error(
        &self,
        a: PyObjectRef,
        b: PyObjectRef,
        op: &str,
    ) -> PyObjectRef {
        self.new_type_error(format!(
            "Unsupported operand types for '{}': '{}' and '{}'",
            op,
            a.class().name,
            b.class().name
        ))
    }

    pub fn new_os_error(&self, msg: String) -> PyObjectRef {
        let os_error = self.ctx.exceptions.os_error.clone();
        self.new_exception(os_error, msg)
    }

    /// Create a new python ValueError object. Useful for raising errors from
    /// python functions implemented in rust.
    pub fn new_value_error(&self, msg: String) -> PyObjectRef {
        let value_error = self.ctx.exceptions.value_error.clone();
        self.new_exception(value_error, msg)
    }

    pub fn new_key_error(&self, msg: String) -> PyObjectRef {
        let key_error = self.ctx.exceptions.key_error.clone();
        self.new_exception(key_error, msg)
    }

    pub fn new_index_error(&self, msg: String) -> PyObjectRef {
        let index_error = self.ctx.exceptions.index_error.clone();
        self.new_exception(index_error, msg)
    }

    pub fn new_not_implemented_error(&self, msg: String) -> PyObjectRef {
        let not_implemented_error = self.ctx.exceptions.not_implemented_error.clone();
        self.new_exception(not_implemented_error, msg)
    }

    pub fn new_zero_division_error(&self, msg: String) -> PyObjectRef {
        let zero_division_error = self.ctx.exceptions.zero_division_error.clone();
        self.new_exception(zero_division_error, msg)
    }

    pub fn new_overflow_error(&self, msg: String) -> PyObjectRef {
        let overflow_error = self.ctx.exceptions.overflow_error.clone();
        self.new_exception(overflow_error, msg)
    }

    pub fn new_syntax_error<T: ToString>(&self, msg: &T) -> PyObjectRef {
        let syntax_error = self.ctx.exceptions.syntax_error.clone();
        self.new_exception(syntax_error, msg.to_string())
    }

    pub fn get_none(&self) -> PyObjectRef {
        self.ctx.none()
    }

    pub fn get_type(&self) -> PyClassRef {
        self.ctx.type_type()
    }

    pub fn get_object(&self) -> PyClassRef {
        self.ctx.object()
    }

    pub fn get_locals(&self) -> PyDictRef {
        self.current_scope().get_locals().clone()
    }

    pub fn context(&self) -> &PyContext {
        &self.ctx
    }

    // Container of the virtual machine state:
    pub fn to_str(&self, obj: &PyObjectRef) -> PyResult<PyStringRef> {
        let str = self.call_method(&obj, "__str__", vec![])?;
        TryFromObject::try_from_object(self, str)
    }

    pub fn to_pystr<'a, T: Into<&'a PyObjectRef>>(&'a self, obj: T) -> Result<String, PyObjectRef> {
        let py_str_obj = self.to_str(obj.into())?;
        Ok(py_str_obj.value.clone())
    }

    pub fn to_repr(&self, obj: &PyObjectRef) -> PyResult<PyStringRef> {
        let repr = self.call_method(obj, "__repr__", vec![])?;
        TryFromObject::try_from_object(self, repr)
    }

    pub fn import(&self, module: &str) -> PyResult {
        match self.get_attribute(self.builtins.clone(), "__import__") {
            Ok(func) => self.invoke(func, vec![self.ctx.new_str(module.to_string())]),
            Err(_) => Err(self.new_exception(
                self.ctx.exceptions.import_error.clone(),
                "__import__ not found".to_string(),
            )),
        }
    }

    /// Determines if `obj` is an instance of `cls`, either directly, indirectly or virtually via
    /// the __instancecheck__ magic method.
    pub fn isinstance(&self, obj: &PyObjectRef, cls: &PyClassRef) -> PyResult<bool> {
        // cpython first does an exact check on the type, although documentation doesn't state that
        // https://github.com/python/cpython/blob/a24107b04c1277e3c1105f98aff5bfa3a98b33a0/Objects/abstract.c#L2408
        if Rc::ptr_eq(&obj.class().into_object(), cls.as_object()) {
            Ok(true)
        } else {
            let ret = self.call_method(cls.as_object(), "__instancecheck__", vec![obj.clone()])?;
            objbool::boolval(self, ret)
        }
    }

    /// Determines if `subclass` is a subclass of `cls`, either directly, indirectly or virtually
    /// via the __subclasscheck__ magic method.
    pub fn issubclass(&self, subclass: &PyObjectRef, cls: &PyObjectRef) -> PyResult<bool> {
        let ret = self.call_method(cls, "__subclasscheck__", vec![subclass.clone()])?;
        objbool::boolval(self, ret)
    }

    pub fn call_get_descriptor(&self, attr: PyObjectRef, obj: PyObjectRef) -> PyResult {
        let attr_class = attr.class();
        if let Some(descriptor) = objtype::class_get_attr(&attr_class, "__get__") {
            let cls = obj.class();
            self.invoke(descriptor, vec![attr, obj.clone(), cls.into_object()])
        } else {
            Ok(attr)
        }
    }

    pub fn call_method<T>(&self, obj: &PyObjectRef, method_name: &str, args: T) -> PyResult
    where
        T: Into<PyFuncArgs>,
    {
        // This is only used in the vm for magic methods, which use a greatly simplified attribute lookup.
        let cls = obj.class();
        match objtype::class_get_attr(&cls, method_name) {
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

    pub fn invoke<T>(&self, func_ref: PyObjectRef, args: T) -> PyResult
    where
        T: Into<PyFuncArgs>,
    {
        let args = args.into();
        trace!("Invoke: {:?} {:?}", func_ref, args);
        if let Some(PyFunction {
            ref code,
            ref scope,
            ref defaults,
            ref kw_only_defaults,
        }) = func_ref.payload()
        {
            return self.invoke_python_function(code, scope, defaults, kw_only_defaults, args);
        }
        if let Some(PyMethod {
            ref function,
            ref object,
        }) = func_ref.payload()
        {
            return self.invoke(function.clone(), args.insert(object.clone()));
        }
        if let Some(PyBuiltinFunction { ref value }) = func_ref.payload() {
            return value(self, args);
        }

        // TODO: is it safe to just invoke __call__ otherwise?
        trace!("invoke __call__ for: {:?}", &func_ref.payload);
        self.call_method(&func_ref, "__call__", args)
    }

    fn invoke_python_function(
        &self,
        code: &PyCodeRef,
        scope: &Scope,
        defaults: &Option<PyTupleRef>,
        kw_only_defaults: &Option<PyDictRef>,
        args: PyFuncArgs,
    ) -> PyResult {
        let scope = scope.child_scope(&self.ctx);
        self.fill_locals_from_args(
            &code.code,
            &scope.get_locals(),
            args,
            defaults,
            kw_only_defaults,
        )?;

        // Construct frame:
        let frame = Frame::new(code.clone(), scope).into_ref(self);

        // If we have a generator, create a new generator
        if code.code.is_generator {
            Ok(PyGenerator::new(frame, self).into_object())
        } else {
            self.run_frame_full(frame)
        }
    }

    pub fn invoke_with_locals(
        &self,
        function: PyObjectRef,
        cells: PyDictRef,
        locals: PyDictRef,
    ) -> PyResult {
        if let Some(PyFunction { code, scope, .. }) = &function.payload() {
            let scope = scope
                .child_scope_with_locals(cells)
                .child_scope_with_locals(locals);
            let frame = Frame::new(code.clone(), scope).into_ref(self);
            return self.run_frame_full(frame);
        }
        panic!(
            "invoke_with_locals: expected python function, got: {:?}",
            function
        );
    }

    fn fill_locals_from_args(
        &self,
        code_object: &bytecode::CodeObject,
        locals: &PyDictRef,
        args: PyFuncArgs,
        defaults: &Option<PyTupleRef>,
        kw_only_defaults: &Option<PyDictRef>,
    ) -> PyResult<()> {
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
            locals.set_item(arg_name, arg.clone(), self)?;
        }

        // Pack other positional arguments in to *args:
        match code_object.varargs {
            bytecode::Varargs::Named(ref vararg_name) => {
                let mut last_args = vec![];
                for i in n..nargs {
                    let arg = &args.args[i];
                    last_args.push(arg.clone());
                }
                let vararg_value = self.ctx.new_tuple(last_args);

                locals.set_item(vararg_name, vararg_value, self)?;
            }
            bytecode::Varargs::Unnamed | bytecode::Varargs::None => {
                // Check the number of positional arguments
                if nargs > nexpected_args {
                    return Err(self.new_type_error(format!(
                        "Expected {} arguments (got: {})",
                        nexpected_args, nargs
                    )));
                }
            }
        }

        // Do we support `**kwargs` ?
        let kwargs = match code_object.varkeywords {
            bytecode::Varargs::Named(ref kwargs_name) => {
                let d = self.ctx.new_dict();
                locals.set_item(kwargs_name, d.as_object().clone(), self)?;
                Some(d)
            }
            bytecode::Varargs::Unnamed => Some(self.ctx.new_dict()),
            bytecode::Varargs::None => None,
        };

        // Handle keyword arguments
        for (name, value) in args.kwargs {
            // Check if we have a parameter with this name:
            if code_object.arg_names.contains(&name) || code_object.kwonlyarg_names.contains(&name)
            {
                if locals.contains_key(&name, self) {
                    return Err(
                        self.new_type_error(format!("Got multiple values for argument '{}'", name))
                    );
                }

                locals.set_item(&name, value, self)?;
            } else if let Some(d) = &kwargs {
                d.set_item(&name, value, self)?;
            } else {
                return Err(
                    self.new_type_error(format!("Got an unexpected keyword argument '{}'", name))
                );
            }
        }

        // Add missing positional arguments, if we have fewer positional arguments than the
        // function definition calls for
        if nargs < nexpected_args {
            let num_defaults_available = defaults.as_ref().map_or(0, |d| d.elements.borrow().len());

            // Given the number of defaults available, check all the arguments for which we
            // _don't_ have defaults; if any are missing, raise an exception
            let required_args = nexpected_args - num_defaults_available;
            let mut missing = vec![];
            for i in 0..required_args {
                let variable_name = &code_object.arg_names[i];
                if !locals.contains_key(variable_name, self) {
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
            if let Some(defaults) = defaults {
                let defaults = defaults.elements.borrow();
                // We have sufficient defaults, so iterate over the corresponding names and use
                // the default if we don't already have a value
                for (default_index, i) in (required_args..nexpected_args).enumerate() {
                    let arg_name = &code_object.arg_names[i];
                    if !locals.contains_key(arg_name, self) {
                        locals.set_item(arg_name, defaults[default_index].clone(), self)?;
                    }
                }
            }
        };

        // Check if kw only arguments are all present:
        for arg_name in &code_object.kwonlyarg_names {
            if !locals.contains_key(arg_name, self) {
                if let Some(kw_only_defaults) = kw_only_defaults {
                    if let Some(default) = kw_only_defaults.get_item_option(arg_name, self)? {
                        locals.set_item(arg_name, default, self)?;
                        continue;
                    }
                }

                // No default value and not specified.
                return Err(self
                    .new_type_error(format!("Missing required kw only argument: '{}'", arg_name)));
            }
        }

        Ok(())
    }

    pub fn extract_elements(&self, value: &PyObjectRef) -> PyResult<Vec<PyObjectRef>> {
        // Extract elements from item, if possible:
        let elements = if objtype::isinstance(value, &self.ctx.tuple_type())
            || objtype::isinstance(value, &self.ctx.list_type())
        {
            objsequence::get_elements(value).to_vec()
        } else {
            let iter = objiter::get_iter(self, value)?;
            objiter::get_all(self, &iter)?
        };
        Ok(elements)
    }

    // get_attribute should be used for full attribute access (usually from user code).
    pub fn get_attribute<T>(&self, obj: PyObjectRef, attr_name: T) -> PyResult
    where
        T: TryIntoRef<PyString>,
    {
        let attr_name = attr_name.try_into_ref(self)?;
        trace!("vm.__getattribute__: {:?} {:?}", obj, attr_name);
        self.call_method(&obj, "__getattribute__", vec![attr_name.into_object()])
    }

    pub fn set_attr<K, V>(&self, obj: &PyObjectRef, attr_name: K, attr_value: V) -> PyResult
    where
        K: TryIntoRef<PyString>,
        V: Into<PyObjectRef>,
    {
        let attr_name = attr_name.try_into_ref(self)?;
        self.call_method(
            obj,
            "__setattr__",
            vec![attr_name.into_object(), attr_value.into()],
        )
    }

    pub fn del_attr(&self, obj: &PyObjectRef, attr_name: PyObjectRef) -> PyResult<()> {
        self.call_method(&obj, "__delattr__", vec![attr_name])?;
        Ok(())
    }

    // get_method should be used for internal access to magic methods (by-passing
    // the full getattribute look-up.
    pub fn get_method(&self, obj: PyObjectRef, method_name: &str) -> PyResult {
        let cls = obj.class();
        match objtype::class_get_attr(&cls, method_name) {
            Some(method) => self.call_get_descriptor(method, obj.clone()),
            None => Err(self.new_type_error(format!("{} has no method {:?}", obj, method_name))),
        }
    }

    /// Calls a method on `obj` passing `arg`, if the method exists.
    ///
    /// Otherwise, or if the result is the special `NotImplemented` built-in constant,
    /// calls `unsupported` to determine fallback value.
    pub fn call_or_unsupported<F>(
        &self,
        obj: PyObjectRef,
        arg: PyObjectRef,
        method: &str,
        unsupported: F,
    ) -> PyResult
    where
        F: Fn(&VirtualMachine, PyObjectRef, PyObjectRef) -> PyResult,
    {
        if let Ok(method) = self.get_method(obj.clone(), method) {
            let result = self.invoke(method, vec![arg.clone()])?;
            if !result.is(&self.ctx.not_implemented()) {
                return Ok(result);
            }
        }

        unsupported(self, obj, arg)
    }

    /// Calls a method, falling back to its reflection with the operands
    /// reversed, and then to the value provided by `unsupported`.
    ///
    /// For example: the following:
    ///
    /// `call_or_reflection(lhs, rhs, "__and__", "__rand__", unsupported)`
    ///
    /// 1. Calls `__and__` with `lhs` and `rhs`.
    /// 2. If above is not implemented, calls `__rand__` with `rhs` and `lhs`.
    /// 3. If above is not implemented, invokes `unsupported` for the result.
    pub fn call_or_reflection(
        &self,
        lhs: PyObjectRef,
        rhs: PyObjectRef,
        default: &str,
        reflection: &str,
        unsupported: fn(&VirtualMachine, PyObjectRef, PyObjectRef) -> PyResult,
    ) -> PyResult {
        // Try to call the default method
        self.call_or_unsupported(lhs, rhs, default, move |vm, lhs, rhs| {
            // Try to call the reflection method
            vm.call_or_unsupported(rhs, lhs, reflection, unsupported)
        })
    }

    pub fn serialize(&self, obj: &PyObjectRef) -> PyResult<String> {
        crate::stdlib::json::ser_pyobject(self, obj)
    }

    pub fn deserialize(&self, s: &str) -> PyResult {
        crate::stdlib::json::de_pyobject(self, s)
    }

    pub fn _sub(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__sub__", "__rsub__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "-"))
        })
    }

    pub fn _isub(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__isub__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__sub__", "__rsub__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, "-="))
            })
        })
    }

    pub fn _add(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__add__", "__radd__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "+"))
        })
    }

    pub fn _iadd(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__iadd__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__add__", "__radd__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, "+="))
            })
        })
    }

    pub fn _mul(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__mul__", "__rmul__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "*"))
        })
    }

    pub fn _imul(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__imul__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__mul__", "__rmul__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, "*="))
            })
        })
    }

    pub fn _matmul(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__matmul__", "__rmatmul__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "@"))
        })
    }

    pub fn _imatmul(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__imatmul__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__matmul__", "__rmatmul__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, "@="))
            })
        })
    }

    pub fn _truediv(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__truediv__", "__rtruediv__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "/"))
        })
    }

    pub fn _itruediv(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__itruediv__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__truediv__", "__rtruediv__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, "/="))
            })
        })
    }

    pub fn _floordiv(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__floordiv__", "__rfloordiv__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "//"))
        })
    }

    pub fn _ifloordiv(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__ifloordiv__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__floordiv__", "__rfloordiv__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, "//="))
            })
        })
    }

    pub fn _pow(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__pow__", "__rpow__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "**"))
        })
    }

    pub fn _ipow(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__ipow__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__pow__", "__rpow__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, "**="))
            })
        })
    }

    pub fn _mod(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__mod__", "__rmod__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "%"))
        })
    }

    pub fn _imod(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__imod__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__mod__", "__rmod__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, "%="))
            })
        })
    }

    pub fn _lshift(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__lshift__", "__rlshift__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "<<"))
        })
    }

    pub fn _ilshift(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__ilshift__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__lshift__", "__rlshift__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, "<<="))
            })
        })
    }

    pub fn _rshift(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__rshift__", "__rrshift__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, ">>"))
        })
    }

    pub fn _irshift(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__irshift__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__rshift__", "__rrshift__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, ">>="))
            })
        })
    }

    pub fn _xor(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__xor__", "__rxor__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "^"))
        })
    }

    pub fn _ixor(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__ixor__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__xor__", "__rxor__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, "^="))
            })
        })
    }

    pub fn _or(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__or__", "__ror__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "|"))
        })
    }

    pub fn _ior(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__ior__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__or__", "__ror__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, "|="))
            })
        })
    }

    pub fn _and(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__and__", "__rand__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "&"))
        })
    }

    pub fn _iand(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__iand__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__and__", "__rand__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, "&="))
            })
        })
    }

    pub fn _eq(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__eq__", "__eq__", |vm, a, b| {
            Ok(vm.new_bool(a.is(&b)))
        })
    }

    pub fn _ne(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__ne__", "__ne__", |vm, a, b| {
            let eq = vm._eq(a, b)?;
            objbool::not(vm, &eq)
        })
    }

    pub fn _lt(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__lt__", "__gt__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "<"))
        })
    }

    pub fn _le(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__le__", "__ge__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "<="))
        })
    }

    pub fn _gt(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__gt__", "__lt__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, ">"))
        })
    }

    pub fn _ge(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__ge__", "__le__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, ">="))
        })
    }

    // https://docs.python.org/3/reference/expressions.html#membership-test-operations
    fn _membership_iter_search(&self, haystack: PyObjectRef, needle: PyObjectRef) -> PyResult {
        let iter = objiter::get_iter(self, &haystack)?;
        loop {
            if let Some(element) = objiter::get_next_object(self, &iter)? {
                let equal = self._eq(needle.clone(), element.clone())?;
                if objbool::get_value(&equal) {
                    return Ok(self.new_bool(true));
                } else {
                    continue;
                }
            } else {
                return Ok(self.new_bool(false));
            }
        }
    }

    pub fn _membership(&self, haystack: PyObjectRef, needle: PyObjectRef) -> PyResult {
        if let Ok(method) = self.get_method(haystack.clone(), "__contains__") {
            self.invoke(method, vec![needle])
        } else {
            self._membership_iter_search(haystack, needle)
        }
    }
}

impl Default for VirtualMachine {
    fn default() -> Self {
        VirtualMachine::new()
    }
}

lazy_static! {
    static ref REPR_GUARDS: Mutex<HashSet<usize>> = { Mutex::new(HashSet::new()) };
}

pub struct ReprGuard {
    id: usize,
}

/// A guard to protect repr methods from recursion into itself,
impl ReprGuard {
    fn get_guards<'a>() -> MutexGuard<'a, HashSet<usize>> {
        REPR_GUARDS.lock().expect("ReprGuard lock poisoned")
    }

    /// Returns None if the guard against 'obj' is still held otherwise returns the guard. The guard
    /// which is released if dropped.
    pub fn enter(obj: &PyObjectRef) -> Option<ReprGuard> {
        let mut guards = ReprGuard::get_guards();

        // Should this be a flag on the obj itself? putting it in a global variable for now until it
        // decided the form of the PyObject. https://github.com/RustPython/RustPython/issues/371
        let id = obj.get_id();
        if guards.contains(&id) {
            return None;
        }
        guards.insert(id);
        Some(ReprGuard { id })
    }
}

impl Drop for ReprGuard {
    fn drop(&mut self) {
        ReprGuard::get_guards().remove(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::VirtualMachine;
    use crate::obj::{objint, objstr};
    use num_bigint::ToBigInt;

    #[test]
    fn test_add_py_integers() {
        let vm = VirtualMachine::new();
        let a = vm.ctx.new_int(33_i32);
        let b = vm.ctx.new_int(12_i32);
        let res = vm._add(a, b).unwrap();
        let value = objint::get_value(&res);
        assert_eq!(*value, 45_i32.to_bigint().unwrap());
    }

    #[test]
    fn test_multiply_str() {
        let vm = VirtualMachine::new();
        let a = vm.ctx.new_str(String::from("Hello "));
        let b = vm.ctx.new_int(4_i32);
        let res = vm._mul(a, b).unwrap();
        let value = objstr::get_value(&res);
        assert_eq!(value, String::from("Hello Hello Hello Hello "))
    }
}
