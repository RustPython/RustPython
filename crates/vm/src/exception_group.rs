//! ExceptionGroup implementation for Python 3.11+
//!
//! This module implements BaseExceptionGroup and ExceptionGroup with multiple inheritance support.

use crate::builtins::{PyList, PyStrRef, PyTuple, PyTupleRef, PyType, PyTypeRef};
use crate::function::FuncArgs;
use crate::object::{Traverse, TraverseFn};
use crate::types::{PyTypeFlags, PyTypeSlots};
use crate::{
    AsObject, Context, Py, PyAtomicRef, PyObject, PyObjectRef, PyRef, PyResult, VirtualMachine,
};
use core::fmt::Write;
use rustpython_common::wtf8::Wtf8Buf;

use crate::exceptions::types::PyBaseException;

/// Create dynamic ExceptionGroup type with multiple inheritance
fn create_exception_group(ctx: &Context) -> PyRef<PyType> {
    let excs = &ctx.exceptions;
    let exception_group_slots = PyTypeSlots {
        flags: PyTypeFlags::heap_type_flags() | PyTypeFlags::HAS_DICT,
        ..Default::default()
    };
    PyType::new_heap(
        "ExceptionGroup",
        vec![
            excs.base_exception_group.to_owned(),
            excs.exception_type.to_owned(),
        ],
        Default::default(),
        exception_group_slots,
        ctx.types.type_type.to_owned(),
        ctx,
    )
    .expect("Failed to create ExceptionGroup type with multiple inheritance")
}

#[must_use]
pub fn exception_group() -> &'static Py<PyType> {
    ::rustpython_vm::common::static_cell! {
        static CELL: ::rustpython_vm::builtins::PyTypeRef;
    }
    CELL.get_or_init(|| create_exception_group(Context::genesis()))
}

pub(super) mod types {
    use super::*;
    use crate::PyPayload;
    use crate::builtins::PyGenericAlias;
    use crate::types::{Constructor, Initializer};

    #[pyexception(name, base = PyBaseException, ctx = "base_exception_group", traverse = "manual")]
    #[repr(C)]
    pub struct PyBaseExceptionGroup {
        base: PyBaseException,
        msg: PyAtomicRef<PyObject>,
        excs: PyAtomicRef<PyObject>,
        excs_str: PyAtomicRef<Option<PyObject>>,
    }

    impl crate::class::PySubclass for PyBaseExceptionGroup {
        type Base = PyBaseException;
        fn as_base(&self) -> &Self::Base {
            &self.base
        }
    }

    impl core::fmt::Debug for PyBaseExceptionGroup {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("PyBaseExceptionGroup")
                .finish_non_exhaustive()
        }
    }

    unsafe impl Traverse for PyBaseExceptionGroup {
        fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
            self.base.traverse(tracer_fn);
            tracer_fn(&self.msg);
            tracer_fn(&self.excs);
            if let Some(obj) = self.excs_str.deref() {
                tracer_fn(obj);
            }
        }
    }

    #[pyexception(with(Constructor, Initializer))]
    impl PyBaseExceptionGroup {
        #[pygetset]
        fn message(&self) -> PyObjectRef {
            self.msg.to_owned()
        }

        #[pygetset]
        fn exceptions(&self) -> PyObjectRef {
            self.excs.to_owned()
        }

        #[pyclassmethod]
        fn __class_getitem__(
            cls: PyTypeRef,
            object: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyGenericAlias> {
            PyGenericAlias::from_args(cls, object, vm)
        }

        #[pymethod]
        fn derive(zelf: PyRef<Self>, excs: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let message = zelf.msg.to_owned();
            vm.invoke_exception(vm.ctx.exceptions.base_exception_group, vec![message, excs])
                .map(|e| e.into())
        }

        #[pymethod]
        fn subgroup(
            zelf: PyRef<Self>,
            matcher_value: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult {
            let matcher = get_condition_matcher(&matcher_value, vm)?;

            // If self matches the condition entirely, return self
            let zelf_obj: PyObjectRef = zelf.clone().into();
            if matcher.check(&zelf_obj, vm)? {
                return Ok(zelf_obj);
            }

            let exceptions = get_exceptions_tuple(&zelf, vm)?;
            let mut matching: Vec<PyObjectRef> = Vec::new();
            let mut modified = false;

            for exc in exceptions {
                if is_base_exception_group(&exc, vm) {
                    // Recursive call for nested groups. It pushes no Python
                    // frame, so a deep enough group runs off the native stack
                    // unless this guard is here.
                    let subgroup_result = vm
                        .with_recursion("in exception group subgroup", || {
                            vm.call_method(&exc, "subgroup", (matcher_value.clone(),))
                        })?;
                    if !vm.is_none(&subgroup_result) {
                        matching.push(subgroup_result.clone());
                    }
                    if !subgroup_result.is(&exc) {
                        modified = true;
                    }
                } else if matcher.check(&exc, vm)? {
                    matching.push(exc);
                } else {
                    modified = true;
                }
            }

            if !modified {
                return Ok(zelf.into());
            }

            if matching.is_empty() {
                return Ok(vm.ctx.none());
            }

            // Create new group with matching exceptions and copy metadata
            derive_and_copy_attributes(&zelf, matching, vm)
        }

        #[pymethod]
        fn split(
            zelf: PyRef<Self>,
            matcher_value: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyTupleRef> {
            let matcher = get_condition_matcher(&matcher_value, vm)?;

            // If self matches the condition entirely
            let zelf_obj: PyObjectRef = zelf.clone().into();
            if matcher.check(&zelf_obj, vm)? {
                return Ok(vm.ctx.new_tuple(vec![zelf_obj, vm.ctx.none()]));
            }

            let exceptions = get_exceptions_tuple(&zelf, vm)?;
            let mut matching: Vec<PyObjectRef> = Vec::new();
            let mut rest: Vec<PyObjectRef> = Vec::new();

            for exc in exceptions {
                if is_base_exception_group(&exc, vm) {
                    // Same as in subgroup: nothing else bounds this recursion
                    // against the native stack.
                    let result = vm.with_recursion("in exception group split", || {
                        vm.call_method(&exc, "split", (matcher_value.clone(),))
                    })?;
                    let result_tuple: PyTupleRef = result.try_into_value(vm)?;
                    let match_part = result_tuple
                        .first()
                        .cloned()
                        .unwrap_or_else(|| vm.ctx.none());
                    let rest_part = result_tuple
                        .get(1)
                        .cloned()
                        .unwrap_or_else(|| vm.ctx.none());

                    if !vm.is_none(&match_part) {
                        matching.push(match_part);
                    }
                    if !vm.is_none(&rest_part) {
                        rest.push(rest_part);
                    }
                } else if matcher.check(&exc, vm)? {
                    matching.push(exc);
                } else {
                    rest.push(exc);
                }
            }

            let match_group = if matching.is_empty() {
                vm.ctx.none()
            } else {
                derive_and_copy_attributes(&zelf, matching, vm)?
            };

            let rest_group = if rest.is_empty() {
                vm.ctx.none()
            } else {
                derive_and_copy_attributes(&zelf, rest, vm)?
            };

            Ok(vm.ctx.new_tuple(vec![match_group, rest_group]))
        }

        #[pymethod]
        fn __str__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            let message = zelf.msg.str(vm)?;
            let num_excs = zelf.excs.downcast_ref::<PyTuple>().map_or(0, |t| t.len());

            let suffix = if num_excs == 1 { "" } else { "s" };
            let mut result = message.as_wtf8().to_owned();
            write!(result, " ({num_excs} sub-exception{suffix})").unwrap();
            Ok(vm.ctx.new_str(result))
        }

        #[pyslot]
        fn slot_repr(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            let zelf = zelf
                .downcast_ref::<PyBaseExceptionGroup>()
                .expect("exception group must be BaseExceptionGroup");
            let class_name = zelf.class().name().to_owned();
            let message = zelf.msg.repr(vm)?;

            let exceptions_str = if let Some(saved) = zelf.excs_str.to_owned() {
                saved
                    .downcast::<crate::builtins::PyStr>()
                    .map_err(|_| vm.new_type_error("__repr__ returned non-string"))?
            } else {
                let args = zelf.base.args();
                let exceptions_obj =
                    if args.len() == 2 && args[1].downcast_ref::<PyList>().is_some() {
                        let list = match zelf.excs.downcast_ref::<PyTuple>() {
                            Some(tuple) => vm.ctx.new_list(tuple.to_vec()),
                            None => vm.ctx.new_list(vec![]),
                        };
                        list.into()
                    } else {
                        zelf.excs.to_owned()
                    };
                exceptions_obj.repr(vm)?
            };

            let mut result = Wtf8Buf::new();
            write!(result, "{class_name}(").unwrap();
            result.push_wtf8(message.as_wtf8());
            result.push_str(", ");
            result.push_wtf8(exceptions_str.as_wtf8());
            result.push_str(")");

            Ok(vm.ctx.new_str(result))
        }
    }

    impl Constructor for PyBaseExceptionGroup {
        type Args = FuncArgs;

        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            if args.args.len() != 2 {
                return Err(vm.new_type_error(format!(
                    "BaseExceptionGroup.__new__() takes exactly 2 arguments ({} given)",
                    args.args.len()
                )));
            }

            let message = args.args[0].clone();
            if !message.fast_isinstance(vm.ctx.types.str_type) {
                return Err(vm.new_type_error(format!(
                    "argument 1 must be str, not {}",
                    message.class().name()
                )));
            }

            let exceptions_arg = &args.args[1];
            exceptions_arg.try_sequence(vm).map_err(|_| {
                vm.new_type_error("second argument (exceptions) must be a sequence")
            })?;

            let is_list = exceptions_arg.downcast_ref::<PyList>().is_some();
            let is_tuple = exceptions_arg.downcast_ref::<PyTuple>().is_some();
            let excs_str = if !is_list && !is_tuple {
                Some(exceptions_arg.repr(vm)?.into())
            } else {
                None
            };

            let exceptions: Vec<PyObjectRef> = exceptions_arg.try_to_value(vm).map_err(|_| {
                vm.new_type_error("second argument (exceptions) must be a sequence")
            })?;

            if exceptions.is_empty() {
                return Err(
                    vm.new_value_error("second argument (exceptions) must be a non-empty sequence")
                );
            }

            let mut has_non_exception = false;
            for (i, exc) in exceptions.iter().enumerate() {
                if !exc.fast_isinstance(vm.ctx.exceptions.base_exception_type) {
                    return Err(vm.new_value_error(format!(
                        "Item {i} of second argument (exceptions) is not an exception"
                    )));
                }
                if !exc.fast_isinstance(vm.ctx.exceptions.exception_type) {
                    has_non_exception = true;
                }
            }

            let exception_group_type = crate::exception_group::exception_group();

            let actual_cls = if cls.is(exception_group_type) {
                if has_non_exception {
                    return Err(
                        vm.new_type_error("Cannot nest BaseExceptions in an ExceptionGroup")
                    );
                }
                cls
            } else if cls.is(vm.ctx.exceptions.base_exception_group) {
                if !has_non_exception {
                    exception_group_type.to_owned()
                } else {
                    cls
                }
            } else {
                if has_non_exception && cls.fast_issubclass(vm.ctx.exceptions.exception_type) {
                    return Err(vm.new_type_error(format!(
                        "Cannot nest BaseExceptions in '{}'",
                        cls.name()
                    )));
                }
                cls
            };

            // Keep an exact tuple as-is so `.exceptions is original` for tuples.
            let exceptions_tuple = if exceptions_arg.class().is(vm.ctx.types.tuple_type) {
                exceptions_arg
                    .clone()
                    .downcast::<PyTuple>()
                    .expect("exact tuple")
            } else {
                vm.ctx.new_tuple(exceptions)
            };

            let payload = Self {
                base: PyBaseException::new(args.args.clone(), vm),
                msg: message.into(),
                excs: PyObjectRef::from(exceptions_tuple).into(),
                excs_str: excs_str.into(),
            };
            payload
                .into_ref_with_type_lazy_dict(vm, actual_cls)
                .map(Into::into)
        }

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            unimplemented!("use slot_new")
        }
    }

    impl Initializer for PyBaseExceptionGroup {
        type Args = FuncArgs;

        fn slot_init(zelf: &PyObject, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            if !args.kwargs.is_empty() {
                return Err(vm.new_type_error(format!(
                    "{} does not take keyword arguments",
                    zelf.class().name()
                )));
            }
            PyBaseException::slot_init(zelf, args, vm)
        }

        fn init(_zelf: &Py<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
            unreachable!("slot_init is overridden")
        }
    }

    // Helper functions for ExceptionGroup
    fn is_base_exception_group(obj: &PyObject, vm: &VirtualMachine) -> bool {
        obj.fast_isinstance(vm.ctx.exceptions.base_exception_group)
    }

    fn get_exceptions_tuple(
        exc: &Py<PyBaseExceptionGroup>,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<PyObjectRef>> {
        let tuple = exc
            .excs
            .downcast_ref::<PyTuple>()
            .ok_or_else(|| vm.new_type_error("exceptions must be a tuple"))?;
        Ok(tuple.to_vec())
    }

    enum ConditionMatcher {
        Type(PyTypeRef),
        Types(Vec<PyTypeRef>),
        Callable(PyObjectRef),
    }

    fn get_condition_matcher(
        condition: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<ConditionMatcher> {
        // If it's a type and subclass of BaseException
        if let Some(typ) = condition.downcast_ref::<PyType>()
            && typ.fast_issubclass(vm.ctx.exceptions.base_exception_type)
        {
            return Ok(ConditionMatcher::Type(typ.to_owned()));
        }

        // If it's a tuple of types
        if let Some(tuple) = condition.downcast_ref::<PyTuple>() {
            let mut types = Vec::new();
            for item in tuple {
                let typ: PyTypeRef = item.clone().try_into_value(vm).map_err(|_| {
                    vm.new_type_error(
                        "expected a function, exception type or tuple of exception types",
                    )
                })?;
                if !typ.fast_issubclass(vm.ctx.exceptions.base_exception_type) {
                    return Err(vm.new_type_error(
                        "expected a function, exception type or tuple of exception types",
                    ));
                }
                types.push(typ);
            }
            if !types.is_empty() {
                return Ok(ConditionMatcher::Types(types));
            }
        }

        // If it's callable (but not a type)
        if condition.is_callable() && condition.downcast_ref::<PyType>().is_none() {
            return Ok(ConditionMatcher::Callable(condition.to_owned()));
        }

        Err(vm.new_type_error("expected a function, exception type or tuple of exception types"))
    }

    impl ConditionMatcher {
        fn check(&self, exc: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
            match self {
                Self::Type(typ) => Ok(exc.fast_isinstance(typ)),
                Self::Types(types) => Ok(types.iter().any(|t| exc.fast_isinstance(t))),
                Self::Callable(func) => {
                    let result = func.call((exc.to_owned(),), vm)?;
                    result.try_to_bool(vm)
                }
            }
        }
    }

    pub(crate) fn derive_and_copy_attributes(
        orig: &Py<PyBaseExceptionGroup>,
        excs: Vec<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        // Call derive method to create new group
        let excs_seq = vm.ctx.new_list(excs);
        let new_group = vm.call_method(orig.as_object(), "derive", (excs_seq,))?;

        // Verify derive returned a BaseExceptionGroup
        if !is_base_exception_group(&new_group, vm) {
            return Err(vm.new_type_error("derive must return an instance of BaseExceptionGroup"));
        }

        // Copy traceback
        if let Some(tb) = orig.base.__traceback__() {
            new_group.set_attr("__traceback__", tb, vm)?;
        }

        // Copy context
        if let Some(ctx) = orig.base.__context__() {
            new_group.set_attr("__context__", ctx, vm)?;
        }

        // Copy cause
        if let Some(cause) = orig.base.__cause__() {
            new_group.set_attr("__cause__", cause, vm)?;
        }

        // Copy notes (if present) - make a copy of the list
        if let Ok(notes) = orig.as_object().get_attr("__notes__", vm)
            && let Some(notes_list) = notes.downcast_ref::<PyList>()
        {
            let notes_copy = vm.ctx.new_list(notes_list.borrow_vec().to_vec());
            new_group.set_attr("__notes__", notes_copy, vm)?;
        }

        Ok(new_group)
    }
}
