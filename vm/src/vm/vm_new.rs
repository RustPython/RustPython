use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyRef,
    builtins::{
        PyBaseException, PyBaseExceptionRef, PyBytesRef, PyDictRef, PyModule, PyStrRef, PyType,
        PyTypeRef,
        builtin_func::PyNativeFunction,
        descriptor::PyMethodDescriptor,
        tuple::{IntoPyTuple, PyTupleRef},
    },
    convert::ToPyObject,
    function::{IntoPyNativeFn, PyMethodFlags},
    scope::Scope,
    vm::VirtualMachine,
};

/// Collection of object creation helpers
impl VirtualMachine {
    /// Create a new python object
    pub fn new_pyobj(&self, value: impl ToPyObject) -> PyObjectRef {
        value.to_pyobject(self)
    }

    pub fn new_tuple(&self, value: impl IntoPyTuple) -> PyTupleRef {
        value.into_pytuple(self)
    }

    pub fn new_module(
        &self,
        name: &str,
        dict: PyDictRef,
        doc: Option<PyStrRef>,
    ) -> PyRef<PyModule> {
        let module = PyRef::new_ref(
            PyModule::new(),
            self.ctx.types.module_type.to_owned(),
            Some(dict),
        );
        module.init_dict(self.ctx.intern_str(name), doc, self);
        module
    }

    pub fn new_scope_with_builtins(&self) -> Scope {
        Scope::with_builtins(None, self.ctx.new_dict(), self)
    }

    pub fn new_function<F, FKind>(&self, name: &'static str, f: F) -> PyRef<PyNativeFunction>
    where
        F: IntoPyNativeFn<FKind>,
    {
        let def = self
            .ctx
            .new_method_def(name, f, PyMethodFlags::empty(), None);
        def.build_function(self)
    }

    pub fn new_method<F, FKind>(
        &self,
        name: &'static str,
        class: &'static Py<PyType>,
        f: F,
    ) -> PyRef<PyMethodDescriptor>
    where
        F: IntoPyNativeFn<FKind>,
    {
        let def = self
            .ctx
            .new_method_def(name, f, PyMethodFlags::METHOD, None);
        def.build_method(class, self)
    }

    /// Instantiate an exception with arguments.
    /// This function should only be used with builtin exception types; if a user-defined exception
    /// type is passed in, it may not be fully initialized; try using
    /// [`vm.invoke_exception()`][Self::invoke_exception] or
    /// [`exceptions::ExceptionCtor`][crate::exceptions::ExceptionCtor] instead.
    pub fn new_exception(&self, exc_type: PyTypeRef, args: Vec<PyObjectRef>) -> PyBaseExceptionRef {
        // TODO: add repr of args into logging?

        PyRef::new_ref(
            // TODO: this constructor might be invalid, because multiple
            // exception (even builtin ones) are using custom constructors,
            // see `OSError` as an example:
            PyBaseException::new(args, self),
            exc_type,
            Some(self.ctx.new_dict()),
        )
    }

    /// Instantiate an exception with no arguments.
    /// This function should only be used with builtin exception types; if a user-defined exception
    /// type is passed in, it may not be fully initialized; try using
    /// [`vm.invoke_exception()`][Self::invoke_exception] or
    /// [`exceptions::ExceptionCtor`][crate::exceptions::ExceptionCtor] instead.
    pub fn new_exception_empty(&self, exc_type: PyTypeRef) -> PyBaseExceptionRef {
        self.new_exception(exc_type, vec![])
    }

    /// Instantiate an exception with `msg` as the only argument.
    /// This function should only be used with builtin exception types; if a user-defined exception
    /// type is passed in, it may not be fully initialized; try using
    /// [`vm.invoke_exception()`][Self::invoke_exception] or
    /// [`exceptions::ExceptionCtor`][crate::exceptions::ExceptionCtor] instead.
    pub fn new_exception_msg(&self, exc_type: PyTypeRef, msg: String) -> PyBaseExceptionRef {
        self.new_exception(exc_type, vec![self.ctx.new_str(msg).into()])
    }

    /// Instantiate an exception with `msg` as the only argument and `dict` for object
    /// This function should only be used with builtin exception types; if a user-defined exception
    /// type is passed in, it may not be fully initialized; try using
    /// [`vm.invoke_exception()`][Self::invoke_exception] or
    /// [`exceptions::ExceptionCtor`][crate::exceptions::ExceptionCtor] instead.
    pub fn new_exception_msg_dict(
        &self,
        exc_type: PyTypeRef,
        msg: String,
        dict: PyDictRef,
    ) -> PyBaseExceptionRef {
        PyRef::new_ref(
            // TODO: this constructor might be invalid, because multiple
            // exception (even builtin ones) are using custom constructors,
            // see `OSError` as an example:
            PyBaseException::new(vec![self.ctx.new_str(msg).into()], self),
            exc_type,
            Some(dict),
        )
    }

    pub fn new_lookup_error(&self, msg: String) -> PyBaseExceptionRef {
        let lookup_error = self.ctx.exceptions.lookup_error.to_owned();
        self.new_exception_msg(lookup_error, msg)
    }

    pub fn new_attribute_error(&self, msg: String) -> PyBaseExceptionRef {
        let attribute_error = self.ctx.exceptions.attribute_error.to_owned();
        self.new_exception_msg(attribute_error, msg)
    }

    pub fn new_type_error(&self, msg: String) -> PyBaseExceptionRef {
        let type_error = self.ctx.exceptions.type_error.to_owned();
        self.new_exception_msg(type_error, msg)
    }

    pub fn new_name_error(&self, msg: String, name: PyStrRef) -> PyBaseExceptionRef {
        let name_error_type = self.ctx.exceptions.name_error.to_owned();
        let name_error = self.new_exception_msg(name_error_type, msg);
        name_error.as_object().set_attr("name", name, self).unwrap();
        name_error
    }

    pub fn new_unsupported_unary_error(&self, a: &PyObject, op: &str) -> PyBaseExceptionRef {
        self.new_type_error(format!(
            "bad operand type for {}: '{}'",
            op,
            a.class().name()
        ))
    }

    pub fn new_unsupported_bin_op_error(
        &self,
        a: &PyObject,
        b: &PyObject,
        op: &str,
    ) -> PyBaseExceptionRef {
        self.new_type_error(format!(
            "'{}' not supported between instances of '{}' and '{}'",
            op,
            a.class().name(),
            b.class().name()
        ))
    }

    pub fn new_unsupported_ternary_op_error(
        &self,
        a: &PyObject,
        b: &PyObject,
        c: &PyObject,
        op: &str,
    ) -> PyBaseExceptionRef {
        self.new_type_error(format!(
            "Unsupported operand types for '{}': '{}', '{}', and '{}'",
            op,
            a.class().name(),
            b.class().name(),
            c.class().name()
        ))
    }

    pub fn new_os_error(&self, msg: String) -> PyBaseExceptionRef {
        let os_error = self.ctx.exceptions.os_error.to_owned();
        self.new_exception_msg(os_error, msg)
    }

    pub fn new_errno_error(&self, errno: i32, msg: String) -> PyBaseExceptionRef {
        let vm = self;
        let exc_type =
            crate::exceptions::errno_to_exc_type(errno, vm).unwrap_or(vm.ctx.exceptions.os_error);

        let errno_obj = vm.new_pyobj(errno);
        vm.new_exception(exc_type.to_owned(), vec![errno_obj, vm.new_pyobj(msg)])
    }

    pub fn new_system_error(&self, msg: String) -> PyBaseExceptionRef {
        let sys_error = self.ctx.exceptions.system_error.to_owned();
        self.new_exception_msg(sys_error, msg)
    }

    // TODO: remove & replace with new_unicode_decode_error_real
    pub fn new_unicode_decode_error(&self, msg: String) -> PyBaseExceptionRef {
        let unicode_decode_error = self.ctx.exceptions.unicode_decode_error.to_owned();
        self.new_exception_msg(unicode_decode_error, msg)
    }

    pub fn new_unicode_decode_error_real(
        &self,
        encoding: PyStrRef,
        object: PyBytesRef,
        start: usize,
        end: usize,
        reason: PyStrRef,
    ) -> PyBaseExceptionRef {
        let start = self.ctx.new_int(start);
        let end = self.ctx.new_int(end);
        let exc = self.new_exception(
            self.ctx.exceptions.unicode_decode_error.to_owned(),
            vec![
                encoding.clone().into(),
                object.clone().into(),
                start.clone().into(),
                end.clone().into(),
                reason.clone().into(),
            ],
        );
        exc.as_object()
            .set_attr("encoding", encoding, self)
            .unwrap();
        exc.as_object().set_attr("object", object, self).unwrap();
        exc.as_object().set_attr("start", start, self).unwrap();
        exc.as_object().set_attr("end", end, self).unwrap();
        exc.as_object().set_attr("reason", reason, self).unwrap();
        exc
    }

    // TODO: remove & replace with new_unicode_encode_error_real
    pub fn new_unicode_encode_error(&self, msg: String) -> PyBaseExceptionRef {
        let unicode_encode_error = self.ctx.exceptions.unicode_encode_error.to_owned();
        self.new_exception_msg(unicode_encode_error, msg)
    }

    pub fn new_unicode_encode_error_real(
        &self,
        encoding: PyStrRef,
        object: PyStrRef,
        start: usize,
        end: usize,
        reason: PyStrRef,
    ) -> PyBaseExceptionRef {
        let start = self.ctx.new_int(start);
        let end = self.ctx.new_int(end);
        let exc = self.new_exception(
            self.ctx.exceptions.unicode_encode_error.to_owned(),
            vec![
                encoding.clone().into(),
                object.clone().into(),
                start.clone().into(),
                end.clone().into(),
                reason.clone().into(),
            ],
        );
        exc.as_object()
            .set_attr("encoding", encoding, self)
            .unwrap();
        exc.as_object().set_attr("object", object, self).unwrap();
        exc.as_object().set_attr("start", start, self).unwrap();
        exc.as_object().set_attr("end", end, self).unwrap();
        exc.as_object().set_attr("reason", reason, self).unwrap();
        exc
    }

    /// Create a new python ValueError object. Useful for raising errors from
    /// python functions implemented in rust.
    pub fn new_value_error(&self, msg: String) -> PyBaseExceptionRef {
        let value_error = self.ctx.exceptions.value_error.to_owned();
        self.new_exception_msg(value_error, msg)
    }

    pub fn new_buffer_error(&self, msg: String) -> PyBaseExceptionRef {
        let buffer_error = self.ctx.exceptions.buffer_error.to_owned();
        self.new_exception_msg(buffer_error, msg)
    }

    // TODO: don't take ownership should make the success path faster
    pub fn new_key_error(&self, obj: PyObjectRef) -> PyBaseExceptionRef {
        let key_error = self.ctx.exceptions.key_error.to_owned();
        self.new_exception(key_error, vec![obj])
    }

    pub fn new_index_error(&self, msg: String) -> PyBaseExceptionRef {
        let index_error = self.ctx.exceptions.index_error.to_owned();
        self.new_exception_msg(index_error, msg)
    }

    pub fn new_not_implemented_error(&self, msg: String) -> PyBaseExceptionRef {
        let not_implemented_error = self.ctx.exceptions.not_implemented_error.to_owned();
        self.new_exception_msg(not_implemented_error, msg)
    }

    pub fn new_recursion_error(&self, msg: String) -> PyBaseExceptionRef {
        let recursion_error = self.ctx.exceptions.recursion_error.to_owned();
        self.new_exception_msg(recursion_error, msg)
    }

    pub fn new_zero_division_error(&self, msg: String) -> PyBaseExceptionRef {
        let zero_division_error = self.ctx.exceptions.zero_division_error.to_owned();
        self.new_exception_msg(zero_division_error, msg)
    }

    pub fn new_overflow_error(&self, msg: String) -> PyBaseExceptionRef {
        let overflow_error = self.ctx.exceptions.overflow_error.to_owned();
        self.new_exception_msg(overflow_error, msg)
    }

    #[cfg(any(feature = "parser", feature = "compiler"))]
    pub fn new_syntax_error(
        &self,
        error: &crate::compiler::CompileError,
        source: Option<&str>,
    ) -> PyBaseExceptionRef {
        use crate::source::SourceLocation;

        let syntax_error_type = match &error {
            #[cfg(feature = "parser")]
            // FIXME: this condition will cause TabError even when the matching actual error is IndentationError
            crate::compiler::CompileError::Parse(rustpython_compiler::ParseError {
                error:
                    ruff_python_parser::ParseErrorType::Lexical(
                        ruff_python_parser::LexicalErrorType::IndentationError,
                    ),
                ..
            }) => self.ctx.exceptions.tab_error,
            #[cfg(feature = "parser")]
            crate::compiler::CompileError::Parse(rustpython_compiler::ParseError {
                error: ruff_python_parser::ParseErrorType::UnexpectedIndentation,
                ..
            }) => self.ctx.exceptions.indentation_error,
            #[cfg(feature = "parser")]
            crate::compiler::CompileError::Parse(rustpython_compiler::ParseError {
                error: ruff_python_parser::ParseErrorType::OtherError(s),
                ..
            }) => {
                if s.starts_with("Expected an indented block after") {
                    self.ctx.exceptions.indentation_error
                } else {
                    self.ctx.exceptions.syntax_error
                }
            }
            _ => self.ctx.exceptions.syntax_error,
        }
        .to_owned();

        // TODO: replace to SourceCode
        fn get_statement(source: &str, loc: Option<SourceLocation>) -> Option<String> {
            let line = source
                .split('\n')
                .nth(loc?.row.to_zero_indexed())?
                .to_owned();
            Some(line + "\n")
        }

        let statement = if let Some(source) = source {
            get_statement(source, error.location())
        } else {
            None
        };

        let mut msg = error.to_string();
        if let Some(msg) = msg.get_mut(..1) {
            msg.make_ascii_lowercase();
        }
        match error {
            #[cfg(feature = "parser")]
            crate::compiler::CompileError::Parse(rustpython_compiler::ParseError {
                error:
                    ruff_python_parser::ParseErrorType::FStringError(_)
                    | ruff_python_parser::ParseErrorType::UnexpectedExpressionToken,
                ..
            }) => msg.insert_str(0, "invalid syntax: "),
            _ => {}
        }
        let syntax_error = self.new_exception_msg(syntax_error_type, msg);
        let (lineno, offset) = error.python_location();
        let lineno = self.ctx.new_int(lineno);
        let offset = self.ctx.new_int(offset);
        syntax_error
            .as_object()
            .set_attr("lineno", lineno, self)
            .unwrap();
        syntax_error
            .as_object()
            .set_attr("offset", offset, self)
            .unwrap();

        syntax_error
            .as_object()
            .set_attr("text", statement.to_pyobject(self), self)
            .unwrap();
        syntax_error
            .as_object()
            .set_attr("filename", self.ctx.new_str(error.source_path()), self)
            .unwrap();
        syntax_error
    }

    pub fn new_import_error(&self, msg: String, name: PyStrRef) -> PyBaseExceptionRef {
        let import_error = self.ctx.exceptions.import_error.to_owned();
        let exc = self.new_exception_msg(import_error, msg);
        exc.as_object().set_attr("name", name, self).unwrap();
        exc
    }

    pub fn new_runtime_error(&self, msg: String) -> PyBaseExceptionRef {
        let runtime_error = self.ctx.exceptions.runtime_error.to_owned();
        self.new_exception_msg(runtime_error, msg)
    }

    pub fn new_memory_error(&self, msg: String) -> PyBaseExceptionRef {
        let memory_error_type = self.ctx.exceptions.memory_error.to_owned();
        self.new_exception_msg(memory_error_type, msg)
    }

    pub fn new_stop_iteration(&self, value: Option<PyObjectRef>) -> PyBaseExceptionRef {
        let dict = self.ctx.new_dict();
        let args = if let Some(value) = value {
            // manually set `value` attribute like StopIteration.__init__
            dict.set_item("value", value.clone(), self)
                .expect("dict.__setitem__ never fails");
            vec![value]
        } else {
            Vec::new()
        };

        PyRef::new_ref(
            PyBaseException::new(args, self),
            self.ctx.exceptions.stop_iteration.to_owned(),
            Some(dict),
        )
    }

    fn new_downcast_error(
        &self,
        msg: &'static str,
        error_type: &'static Py<PyType>,
        class: &Py<PyType>,
        obj: &PyObject, // the impl Borrow allows to pass PyObjectRef or &PyObject
    ) -> PyBaseExceptionRef {
        let actual_class = obj.class();
        let actual_type = &*actual_class.name();
        let expected_type = &*class.name();
        let msg = format!("Expected {msg} '{expected_type}' but '{actual_type}' found.");
        #[cfg(debug_assertions)]
        let msg = if class.get_id() == actual_class.get_id() {
            let mut msg = msg;
            msg += " Did you forget to add `#[pyclass(with(Constructor))]`?";
            msg
        } else {
            msg
        };
        self.new_exception_msg(error_type.to_owned(), msg)
    }

    pub(crate) fn new_downcast_runtime_error(
        &self,
        class: &Py<PyType>,
        obj: &impl AsObject,
    ) -> PyBaseExceptionRef {
        self.new_downcast_error(
            "payload",
            self.ctx.exceptions.runtime_error,
            class,
            obj.as_object(),
        )
    }

    pub(crate) fn new_downcast_type_error(
        &self,
        class: &Py<PyType>,
        obj: &impl AsObject,
    ) -> PyBaseExceptionRef {
        self.new_downcast_error(
            "type",
            self.ctx.exceptions.type_error,
            class,
            obj.as_object(),
        )
    }

    pub fn new_eof_error(&self, msg: String) -> PyBaseExceptionRef {
        let eof_error = self.ctx.exceptions.eof_error.to_owned();
        self.new_exception_msg(eof_error, msg)
    }
}
