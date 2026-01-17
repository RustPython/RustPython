//! ExceptionGroup implementation for Python 3.11+
//!
//! This module implements BaseExceptionGroup and ExceptionGroup with multiple inheritance support.

use crate::builtins::{PyList, PyStrRef, PyTuple, PyTupleRef, PyType, PyTypeRef};
use crate::function::{ArgIterable, FuncArgs};
use crate::types::{PyTypeFlags, PyTypeSlots};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyRef, PyResult, TryFromObject, VirtualMachine,
};

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

    #[pyexception(name, base = PyBaseException, ctx = "base_exception_group")]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyBaseExceptionGroup(PyBaseException);

    #[pyexception(with(Constructor, Initializer))]
    impl PyBaseExceptionGroup {
        #[pyclassmethod]
        fn __class_getitem__(
            cls: PyTypeRef,
            args: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyGenericAlias {
            PyGenericAlias::from_args(cls, args, vm)
        }

        #[pymethod]
        fn derive(
            zelf: PyRef<PyBaseException>,
            excs: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult {
            let message = zelf.get_arg(0).unwrap_or_else(|| vm.ctx.new_str("").into());
            vm.invoke_exception(
                vm.ctx.exceptions.base_exception_group.to_owned(),
                vec![message, excs],
            )
            .map(|e| e.into())
        }

        #[pymethod]
        fn subgroup(
            zelf: PyRef<PyBaseException>,
            condition: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult {
            let matcher = get_condition_matcher(&condition, vm)?;

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
                    // Recursive call for nested groups
                    let subgroup_result = vm.call_method(&exc, "subgroup", (condition.clone(),))?;
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
                return Ok(zelf.clone().into());
            }

            if matching.is_empty() {
                return Ok(vm.ctx.none());
            }

            // Create new group with matching exceptions and copy metadata
            derive_and_copy_attributes(&zelf, matching, vm)
        }

        #[pymethod]
        fn split(
            zelf: PyRef<PyBaseException>,
            condition: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyTupleRef> {
            let matcher = get_condition_matcher(&condition, vm)?;

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
                    let result = vm.call_method(&exc, "split", (condition.clone(),))?;
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
        fn __str__(zelf: &Py<PyBaseException>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            let message = zelf
                .get_arg(0)
                .map(|m| m.str(vm))
                .transpose()?
                .map(|s| s.as_str().to_owned())
                .unwrap_or_default();

            let num_excs = zelf
                .get_arg(1)
                .and_then(|obj| obj.downcast_ref::<PyTuple>().map(|t| t.len()))
                .unwrap_or(0);

            let suffix = if num_excs == 1 { "" } else { "s" };
            Ok(vm.ctx.new_str(format!(
                "{} ({} sub-exception{})",
                message, num_excs, suffix
            )))
        }

        #[pyslot]
        fn slot_repr(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            let zelf = zelf
                .downcast_ref::<PyBaseException>()
                .expect("exception group must be BaseException");
            let class_name = zelf.class().name().to_owned();
            let message = zelf
                .get_arg(0)
                .map(|m| m.repr(vm))
                .transpose()?
                .map(|s| s.as_str().to_owned())
                .unwrap_or_else(|| "''".to_owned());

            // Format exceptions as list [exc1, exc2, ...] instead of tuple (exc1, exc2, ...)
            // CPython displays exceptions in list format even though they're stored as tuple
            let exceptions_str = if let Some(exceptions_obj) = zelf.get_arg(1) {
                // Get exceptions using ArgIterable for robustness
                let iter: ArgIterable<PyObjectRef> =
                    ArgIterable::try_from_object(vm, exceptions_obj.clone())?;
                let mut exc_repr_list = Vec::new();
                for exc in iter.iter(vm)? {
                    exc_repr_list.push(exc?.repr(vm)?.as_str().to_owned());
                }
                format!("[{}]", exc_repr_list.join(", "))
            } else {
                "[]".to_owned()
            };

            Ok(vm
                .ctx
                .new_str(format!("{}({}, {})", class_name, message, exceptions_str)))
        }
    }

    impl Constructor for PyBaseExceptionGroup {
        type Args = crate::function::PosArgs;

        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let args: Self::Args = args.bind(vm)?;
            let args = args.into_vec();
            // Validate exactly 2 positional arguments
            if args.len() != 2 {
                return Err(vm.new_type_error(format!(
                    "BaseExceptionGroup.__new__() takes exactly 2 positional arguments ({} given)",
                    args.len()
                )));
            }

            // Validate message is str
            let message = args[0].clone();
            if !message.fast_isinstance(vm.ctx.types.str_type) {
                return Err(vm.new_type_error(format!(
                    "argument 1 must be str, not {}",
                    message.class().name()
                )));
            }

            // Validate exceptions is a sequence (not set or None)
            let exceptions_arg = &args[1];

            // Check for set/frozenset (not a sequence - unordered)
            if exceptions_arg.fast_isinstance(vm.ctx.types.set_type)
                || exceptions_arg.fast_isinstance(vm.ctx.types.frozenset_type)
            {
                return Err(vm.new_type_error("second argument (exceptions) must be a sequence"));
            }

            // Check for None
            if exceptions_arg.is(&vm.ctx.none) {
                return Err(vm.new_type_error("second argument (exceptions) must be a sequence"));
            }

            let exceptions: Vec<PyObjectRef> = exceptions_arg.try_to_value(vm).map_err(|_| {
                vm.new_type_error("second argument (exceptions) must be a sequence")
            })?;

            // Validate non-empty
            if exceptions.is_empty() {
                return Err(vm.new_value_error(
                    "second argument (exceptions) must be a non-empty sequence".to_owned(),
                ));
            }

            // Validate all items are BaseException instances
            let mut has_non_exception = false;
            for (i, exc) in exceptions.iter().enumerate() {
                if !exc.fast_isinstance(vm.ctx.exceptions.base_exception_type) {
                    return Err(vm.new_value_error(format!(
                        "Item {} of second argument (exceptions) is not an exception",
                        i
                    )));
                }
                // Check if any exception is not an Exception subclass
                // With dynamic ExceptionGroup (inherits from both BaseExceptionGroup and Exception),
                // ExceptionGroup instances are automatically instances of Exception
                if !exc.fast_isinstance(vm.ctx.exceptions.exception_type) {
                    has_non_exception = true;
                }
            }

            // Get the dynamic ExceptionGroup type
            let exception_group_type = crate::exception_group::exception_group();

            // Determine the actual class to use
            let actual_cls = if cls.is(exception_group_type) {
                // ExceptionGroup cannot contain BaseExceptions that are not Exception
                if has_non_exception {
                    return Err(
                        vm.new_type_error("Cannot nest BaseExceptions in an ExceptionGroup")
                    );
                }
                cls
            } else if cls.is(vm.ctx.exceptions.base_exception_group) {
                // Auto-convert to ExceptionGroup if all are Exception subclasses
                if !has_non_exception {
                    exception_group_type.to_owned()
                } else {
                    cls
                }
            } else {
                // User-defined subclass
                if has_non_exception && cls.fast_issubclass(vm.ctx.exceptions.exception_type) {
                    return Err(vm.new_type_error(format!(
                        "Cannot nest BaseExceptions in '{}'",
                        cls.name()
                    )));
                }
                cls
            };

            // Create the exception with (message, exceptions_tuple) as args
            let exceptions_tuple = vm.ctx.new_tuple(exceptions);
            let init_args = vec![message, exceptions_tuple.into()];
            PyBaseException::new(init_args, vm)
                .into_ref_with_type(vm, actual_cls)
                .map(Into::into)
        }

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            unimplemented!("use slot_new")
        }
    }

    impl Initializer for PyBaseExceptionGroup {
        type Args = FuncArgs;

        fn slot_init(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            // BaseExceptionGroup_init: no kwargs allowed
            if !args.kwargs.is_empty() {
                return Err(vm.new_type_error(format!(
                    "{} does not take keyword arguments",
                    zelf.class().name()
                )));
            }
            // Do NOT call PyBaseException::slot_init here.
            // slot_new already set args to (message, exceptions_tuple).
            // Calling base init would overwrite with original args (message, exceptions_list).
            let _ = (zelf, args, vm);
            Ok(())
        }

        fn init(_zelf: PyRef<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
            unreachable!("slot_init is overridden")
        }
    }

    // Helper functions for ExceptionGroup
    fn is_base_exception_group(obj: &PyObject, vm: &VirtualMachine) -> bool {
        obj.fast_isinstance(vm.ctx.exceptions.base_exception_group)
    }

    fn get_exceptions_tuple(
        exc: &Py<PyBaseException>,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<PyObjectRef>> {
        let obj = exc
            .get_arg(1)
            .ok_or_else(|| vm.new_type_error("exceptions must be a tuple"))?;
        let tuple = obj
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
            for item in tuple.iter() {
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
                ConditionMatcher::Type(typ) => Ok(exc.fast_isinstance(typ)),
                ConditionMatcher::Types(types) => Ok(types.iter().any(|t| exc.fast_isinstance(t))),
                ConditionMatcher::Callable(func) => {
                    let result = func.call((exc.to_owned(),), vm)?;
                    result.try_to_bool(vm)
                }
            }
        }
    }

    fn derive_and_copy_attributes(
        orig: &Py<PyBaseException>,
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
        if let Some(tb) = orig.__traceback__() {
            new_group.set_attr("__traceback__", tb, vm)?;
        }

        // Copy context
        if let Some(ctx) = orig.__context__() {
            new_group.set_attr("__context__", ctx, vm)?;
        }

        // Copy cause
        if let Some(cause) = orig.__cause__() {
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
