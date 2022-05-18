#[cfg(feature = "rustpython-compiler")]
use crate::compile::{CompileError, CompileErrorType};
use crate::{
    builtins::{
        pystr::IntoPyStrRef,
        tuple::{IntoPyTuple, PyTupleRef},
        PyBaseException, PyBaseExceptionRef, PyDictRef, PyModule, PyStrRef, PyType, PyTypeRef,
    },
    convert::ToPyObject,
    scope::Scope,
    vm::VirtualMachine,
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef,
};

/// Collection of object creation helpers
impl VirtualMachine {
    /// Create a new python object
    pub fn new_pyobj(&self, value: impl ToPyObject) -> PyObjectRef {
        value.to_pyobject(self)
    }

    pub fn new_pyref<T, P>(&self, value: T) -> PyRef<P>
    where
        T: Into<P>,
        P: PyPayload,
    {
        value.into().into_ref(self)
    }

    pub fn new_tuple(&self, value: impl IntoPyTuple) -> PyTupleRef {
        value.into_pytuple(self)
    }

    pub fn new_module(&self, name: &str, dict: PyDictRef, doc: Option<&str>) -> PyObjectRef {
        let module = PyRef::new_ref(
            PyModule {},
            self.ctx.types.module_type.to_owned(),
            Some(dict),
        );
        module.init_module_dict(
            self.ctx.intern_str(name),
            doc.map(|doc| self.new_pyobj(doc.to_owned()))
                .unwrap_or_else(|| self.ctx.none()),
            self,
        );
        module.into()
    }

    pub fn new_scope_with_builtins(&self) -> Scope {
        Scope::with_builtins(None, self.ctx.new_dict(), self)
    }

    /// Instantiate an exception with arguments.
    /// This function should only be used with builtin exception types; if a user-defined exception
    /// type is passed in, it may not be fully initialized; try using
    /// [`vm.invoke_exception()`][Self::invoke_exception] or
    /// [`exceptions::ExceptionCtor`][crate::exceptions::ExceptionCtor] instead.
    pub fn new_exception(&self, exc_type: PyTypeRef, args: Vec<PyObjectRef>) -> PyBaseExceptionRef {
        // TODO: add repr of args into logging?

        PyRef::new_ref(
            // TODO: this costructor might be invalid, because multiple
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

    pub fn new_unsupported_binop_error(
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

    pub fn new_unsupported_ternop_error(
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

    pub fn new_unicode_decode_error(&self, msg: String) -> PyBaseExceptionRef {
        let unicode_decode_error = self.ctx.exceptions.unicode_decode_error.to_owned();
        self.new_exception_msg(unicode_decode_error, msg)
    }

    pub fn new_unicode_encode_error(&self, msg: String) -> PyBaseExceptionRef {
        let unicode_encode_error = self.ctx.exceptions.unicode_encode_error.to_owned();
        self.new_exception_msg(unicode_encode_error, msg)
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

    #[cfg(feature = "rustpython-compiler")]
    pub fn new_syntax_error(&self, error: &CompileError) -> PyBaseExceptionRef {
        let syntax_error_type = match &error.error {
            CompileErrorType::Parse(p) if p.is_indentation_error() => {
                self.ctx.exceptions.indentation_error
            }
            CompileErrorType::Parse(p) if p.is_tab_error() => self.ctx.exceptions.tab_error,
            _ => self.ctx.exceptions.syntax_error,
        }
        .to_owned();
        let syntax_error = self.new_exception_msg(syntax_error_type, error.to_string());
        let lineno = self.ctx.new_int(error.location.row());
        let offset = self.ctx.new_int(error.location.column());
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
            .set_attr("text", error.statement.clone().to_pyobject(self), self)
            .unwrap();
        syntax_error
            .as_object()
            .set_attr(
                "filename",
                self.ctx.new_str(error.source_path.clone()),
                self,
            )
            .unwrap();
        syntax_error
    }

    pub fn new_import_error(&self, msg: String, name: impl IntoPyStrRef) -> PyBaseExceptionRef {
        let import_error = self.ctx.exceptions.import_error.to_owned();
        let exc = self.new_exception_msg(import_error, msg);
        exc.as_object()
            .set_attr("name", name.into_pystr_ref(self), self)
            .unwrap();
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
        let args = if let Some(value) = value {
            vec![value]
        } else {
            Vec::new()
        };
        self.new_exception(self.ctx.exceptions.stop_iteration.to_owned(), args)
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
        let msg = format!("Expected {msg} '{expected_type}' but '{actual_type}' found");
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
}
