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
use rustpython_compiler_core::SourceLocation;

macro_rules! define_exception_fn {
    (
        fn $fn_name:ident, $attr:ident, $python_repr:ident
    ) => {
        #[doc = concat!(
                    "Create a new python ",
                    stringify!($python_repr),
                    " object.\nUseful for raising errors from python functions implemented in rust."
                )]
        pub fn $fn_name(&self, msg: impl Into<String>) -> PyBaseExceptionRef
        {
            let err = self.ctx.exceptions.$attr.to_owned();
            self.new_exception_msg(err, msg.into())
        }
    };
}

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

    pub fn new_no_attribute_error(&self, obj: PyObjectRef, name: PyStrRef) -> PyBaseExceptionRef {
        let msg = format!(
            "'{}' object has no attribute '{}'",
            obj.class().name(),
            name
        );
        let attribute_error = self.new_attribute_error(msg);

        // Use existing set_attribute_error_context function
        self.set_attribute_error_context(&attribute_error, obj, name);

        attribute_error
    }

    pub fn new_name_error(&self, msg: impl Into<String>, name: PyStrRef) -> PyBaseExceptionRef {
        let name_error_type = self.ctx.exceptions.name_error.to_owned();
        let name_error = self.new_exception_msg(name_error_type, msg.into());
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

    pub fn new_errno_error(&self, errno: i32, msg: impl Into<String>) -> PyBaseExceptionRef {
        let vm = self;
        let exc_type =
            crate::exceptions::errno_to_exc_type(errno, vm).unwrap_or(vm.ctx.exceptions.os_error);

        let errno_obj = vm.new_pyobj(errno);
        vm.new_exception(
            exc_type.to_owned(),
            vec![errno_obj, vm.new_pyobj(msg.into())],
        )
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

    // TODO: don't take ownership should make the success path faster
    pub fn new_key_error(&self, obj: PyObjectRef) -> PyBaseExceptionRef {
        let key_error = self.ctx.exceptions.key_error.to_owned();
        self.new_exception(key_error, vec![obj])
    }

    #[cfg(any(feature = "parser", feature = "compiler"))]
    pub fn new_syntax_error_maybe_incomplete(
        &self,
        error: &crate::compiler::CompileError,
        source: Option<&str>,
        allow_incomplete: bool,
    ) -> PyBaseExceptionRef {
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
                error:
                    ruff_python_parser::ParseErrorType::Lexical(
                        ruff_python_parser::LexicalErrorType::Eof,
                    ),
                ..
            }) => {
                if allow_incomplete {
                    self.ctx.exceptions.incomplete_input_error
                } else {
                    self.ctx.exceptions.syntax_error
                }
            }
            #[cfg(feature = "parser")]
            crate::compiler::CompileError::Parse(rustpython_compiler::ParseError {
                error:
                    ruff_python_parser::ParseErrorType::Lexical(
                        ruff_python_parser::LexicalErrorType::FStringError(
                            ruff_python_parser::FStringErrorType::UnterminatedTripleQuotedString,
                        ),
                    ),
                ..
            }) => {
                if allow_incomplete {
                    self.ctx.exceptions.incomplete_input_error
                } else {
                    self.ctx.exceptions.syntax_error
                }
            }
            #[cfg(feature = "parser")]
            crate::compiler::CompileError::Parse(rustpython_compiler::ParseError {
                error:
                    ruff_python_parser::ParseErrorType::Lexical(
                        ruff_python_parser::LexicalErrorType::UnclosedStringError,
                    ),
                raw_location,
                ..
            }) => {
                if allow_incomplete {
                    let mut is_incomplete = false;

                    if let Some(source) = source {
                        let loc = raw_location.start().to_usize();
                        let mut iter = source.chars();
                        if let Some(quote) = iter.nth(loc) {
                            if iter.next() == Some(quote) && iter.next() == Some(quote) {
                                is_incomplete = true;
                            }
                        }
                    }

                    if is_incomplete {
                        self.ctx.exceptions.incomplete_input_error
                    } else {
                        self.ctx.exceptions.syntax_error
                    }
                } else {
                    self.ctx.exceptions.syntax_error
                }
            }
            #[cfg(feature = "parser")]
            crate::compiler::CompileError::Parse(rustpython_compiler::ParseError {
                error: ruff_python_parser::ParseErrorType::OtherError(s),
                raw_location,
                ..
            }) => {
                if s.starts_with("Expected an indented block after") {
                    if allow_incomplete {
                        // Check that all chars in the error are whitespace, if so, the source is
                        // incomplete. Otherwise, we've found code that might violates
                        // indentation rules.
                        let mut is_incomplete = true;
                        if let Some(source) = source {
                            let start = raw_location.start().to_usize();
                            let end = raw_location.end().to_usize();
                            let mut iter = source.chars();
                            iter.nth(start);
                            for _ in start..end {
                                if let Some(c) = iter.next() {
                                    if !c.is_ascii_whitespace() {
                                        is_incomplete = false;
                                    }
                                } else {
                                    break;
                                }
                            }
                        }

                        if is_incomplete {
                            self.ctx.exceptions.incomplete_input_error
                        } else {
                            self.ctx.exceptions.indentation_error
                        }
                    } else {
                        self.ctx.exceptions.indentation_error
                    }
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

    #[cfg(any(feature = "parser", feature = "compiler"))]
    pub fn new_syntax_error(
        &self,
        error: &crate::compiler::CompileError,
        source: Option<&str>,
    ) -> PyBaseExceptionRef {
        self.new_syntax_error_maybe_incomplete(error, source, false)
    }

    pub fn new_import_error(&self, msg: impl Into<String>, name: PyStrRef) -> PyBaseExceptionRef {
        let import_error = self.ctx.exceptions.import_error.to_owned();
        let exc = self.new_exception_msg(import_error, msg.into());
        exc.as_object().set_attr("name", name, self).unwrap();
        exc
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

    define_exception_fn!(fn new_lookup_error, lookup_error, LookupError);
    define_exception_fn!(fn new_eof_error, eof_error, EOFError);
    define_exception_fn!(fn new_attribute_error, attribute_error, AttributeError);
    define_exception_fn!(fn new_type_error, type_error, TypeError);
    define_exception_fn!(fn new_os_error, os_error, OSError);
    define_exception_fn!(fn new_system_error, system_error, SystemError);

    // TODO: remove & replace with new_unicode_decode_error_real
    define_exception_fn!(fn new_unicode_decode_error, unicode_decode_error, UnicodeDecodeError);

    // TODO: remove & replace with new_unicode_encode_error_real
    define_exception_fn!(fn new_unicode_encode_error, unicode_encode_error, UnicodeEncodeError);

    define_exception_fn!(fn new_value_error, value_error, ValueError);

    define_exception_fn!(fn new_buffer_error, buffer_error, BufferError);
    define_exception_fn!(fn new_index_error, index_error, IndexError);
    define_exception_fn!(
     fn new_not_implemented_error,
        not_implemented_error,
        NotImplementedError
    );
    define_exception_fn!(fn new_recursion_error, recursion_error, RecursionError);
    define_exception_fn!(fn new_zero_division_error, zero_division_error, ZeroDivisionError);
    define_exception_fn!(fn new_overflow_error, overflow_error, OverflowError);
    define_exception_fn!(fn new_runtime_error, runtime_error, RuntimeError);
    define_exception_fn!(fn new_memory_error, memory_error, MemoryError);
}
