//! ExceptionGroup implementation for Python 3.11+
//!
//! This module implements BaseExceptionGroup and ExceptionGroup with multiple inheritance support.

use crate::builtins::{PyList, PyListRef, PyStrRef, PyTuple, PyTupleRef, PyType, PyTypeRef};
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
        flags: crate::types::AtomicPyTypeFlags::from_plain(PyTypeFlags::HEAP_TYPE_WITH_DICT),
        ..Default::default()
    };
    let mut attrs = crate::builtins::type_::PyAttributes::default();
    attrs.insert(
        crate::identifier!(ctx, __module__),
        ctx.intern_str("builtins").to_object(),
    );
    PyType::new_heap(
        "ExceptionGroup",
        vec![
            excs.base_exception_group.to_owned(),
            excs.exception_type.to_owned(),
        ],
        attrs,
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
        #[pymember(name = "message")]
        msg: PyAtomicRef<PyObject>,
        #[pymember(name = "exceptions")]
        excs: PyAtomicRef<PyObject>,
        excs_str: PyAtomicRef<Option<PyObject>>,
    }

    // SAFETY: pyexception checks the base field offset, payload offset, and alignment.
    unsafe impl crate::class::PySubclass for PyBaseExceptionGroup {
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
            let result = split_recursive(zelf.as_object(), &matcher, false, vm)?;
            Ok(result.matching.unwrap_or_else(|| vm.ctx.none()))
        }

        #[pymethod]
        fn split(
            zelf: PyRef<Self>,
            matcher_value: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyTupleRef> {
            let matcher = get_condition_matcher(&matcher_value, vm)?;
            let result = split_recursive(zelf.as_object(), &matcher, true, vm)?;
            Ok(vm.ctx.new_tuple(vec![
                result.matching.unwrap_or_else(|| vm.ctx.none()),
                result.rest.unwrap_or_else(|| vm.ctx.none()),
            ]))
        }

        #[pyslot]
        fn slot_str(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            let zelf: &Py<Self> = zelf
                .downcast_ref()
                .expect("slot wrapper checked BaseExceptionGroup");
            let message = zelf.msg.str(vm)?;
            let num_excs = zelf
                .excs
                .downcast_ref::<PyTuple>()
                .map_or(0, |t| t.as_slice().len());

            let suffix = if num_excs == 1 { "" } else { "s" };
            let mut result = message.as_wtf8().to_owned();
            write!(result, " ({num_excs} sub-exception{suffix})")
                .expect("formatting into a string buffer cannot fail");
            Ok(vm.ctx.new_str(result))
        }

        #[pyslot]
        fn slot_repr(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            let zelf = zelf
                .downcast_ref::<PyBaseExceptionGroup>()
                .expect("exception group must be BaseExceptionGroup");
            let exceptions_str = if let Some(saved) = zelf.excs_str.load_owned() {
                saved
                    .downcast::<crate::builtins::PyStr>()
                    .map_err(|_| vm.new_type_error("__repr__ returned non-string"))?
            } else {
                let use_list = {
                    let args = zelf.base.args();
                    args.as_slice().len() == 2
                        && args.as_slice()[1].downcast_ref::<PyList>().is_some()
                };
                let exceptions_obj = if use_list {
                    let list = match zelf.excs.downcast_ref::<PyTuple>() {
                        Some(tuple) => vm.ctx.new_list(tuple.as_slice().to_vec()),
                        None => vm.ctx.new_list(vec![]),
                    };
                    list.into()
                } else {
                    zelf.excs.to_owned()
                };
                exceptions_obj.repr(vm)?
            };

            // Child repr callbacks can rename the type; message repr callbacks
            // run only after its short name has been captured.
            let class_name = {
                let name = zelf.class().slot_name();
                name.rsplit_once('.')
                    .map_or(&*name, |(_, name)| name)
                    .to_owned()
            };
            let message = zelf.msg.repr(vm)?;
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
            // Bind only raw positional objects, retaining the caller's owners.
            // __new__ ignores keywords; __init__ rejects them after construction.
            let (message, exceptions_arg): (PyObjectRef, PyObjectRef) =
                FuncArgs::from(args.args.clone()).bind(vm).map_err(|_| {
                    // Raw-object binding can fail only on the positional count.
                    vm.new_type_error(format!(
                        "BaseExceptionGroup.__new__() takes exactly 2 arguments ({} given)",
                        args.args.len()
                    ))
                })?;

            if !message.fast_isinstance(vm.ctx.types.str_type) {
                return Err(vm.new_type_error(format!(
                    "argument 1 must be str, not {}",
                    message.class().name()
                )));
            }

            if exceptions_arg.fast_isinstance(vm.ctx.types.dict_type)
                || !exceptions_arg.sequence_unchecked().check()
            {
                return Err(vm.new_type_error("second argument (exceptions) must be a sequence"));
            }

            let is_list = exceptions_arg.downcast_ref::<PyList>().is_some();
            let is_tuple = exceptions_arg.downcast_ref::<PyTuple>().is_some();
            let excs_str = if !is_list && !is_tuple {
                Some(exceptions_arg.repr(vm)?.into())
            } else {
                None
            };

            let exceptions =
                if let Some(tuple) = exceptions_arg.downcast_ref_if_exact::<PyTuple>(vm) {
                    tuple.to_owned()
                } else if let Some(list) = exceptions_arg.downcast_ref_if_exact::<PyList>(vm) {
                    vm.ctx.new_tuple(list.borrow_vec().to_vec())
                } else {
                    // Sequence conversion uses iteration without a length hint and
                    // must preserve the original exception from any callback.
                    vm.ctx
                        .new_tuple(exceptions_arg.try_to_value::<Vec<PyObjectRef>>(vm)?)
                };

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
                if !exc.is_instance(vm.ctx.exceptions.exception_type.as_object(), vm)? {
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

            let payload = Self {
                base: PyBaseException::new(args.args.clone(), vm),
                msg: message.into(),
                excs: PyObjectRef::from(exceptions).into(),
                excs_str: excs_str.into(),
            };
            let result = payload
                .into_ref_with_type_lazy_dict(vm, actual_cls)
                .map(Into::into);
            // Retain the original owners even if native allocation rejects the
            // payload, then release them in their normal scope order.
            drop(exceptions_arg);
            drop(args);
            result
        }

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            unimplemented!("use slot_new")
        }
    }

    impl Initializer for PyBaseExceptionGroup {
        type Args = FuncArgs;

        fn slot_init(zelf: &PyObject, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            if !args.kwargs.is_empty() {
                let name = {
                    let name = zelf.class().slot_name();
                    name[..name.floor_char_boundary(200.min(name.len()))].to_owned()
                };
                return Err(vm.new_type_error(format!("{name}() takes no keyword arguments")));
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

    enum ConditionMatcher {
        Type(PyTypeRef),
        Types(Vec<PyTypeRef>),
        Callable(PyObjectRef),
    }

    fn get_condition_matcher(
        condition: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<ConditionMatcher> {
        // A callable tuple subclass is a predicate, not a tuple of types.
        if condition.is_callable() && condition.downcast_ref::<PyType>().is_none() {
            return Ok(ConditionMatcher::Callable(condition.to_owned()));
        }

        // If it's a type and subclass of BaseException
        if let Some(typ) = condition.downcast_ref::<PyType>()
            && typ.fast_issubclass(vm.ctx.exceptions.base_exception_type)
        {
            return Ok(ConditionMatcher::Type(typ.to_owned()));
        }

        // Only exact tuples are accepted, including the empty tuple.
        if let Some(tuple) = condition.downcast_ref_if_exact::<PyTuple>(vm) {
            let mut types = Vec::new();
            for item in tuple {
                let Some(typ) = item.downcast_ref::<PyType>() else {
                    return Err(invalid_condition(vm));
                };
                if !typ.fast_issubclass(vm.ctx.exceptions.base_exception_type) {
                    return Err(invalid_condition(vm));
                }
                types.push(typ.to_owned());
            }
            return Ok(ConditionMatcher::Types(types));
        }

        Err(invalid_condition(vm))
    }

    fn invalid_condition(vm: &VirtualMachine) -> crate::exceptions::types::PyBaseExceptionRef {
        vm.new_type_error(
            "expected an exception type, a tuple of exception types, or a callable (other than a class)",
        )
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

    struct SplitResult {
        matching: Option<PyObjectRef>,
        rest: Option<PyObjectRef>,
    }

    struct SplitLists {
        // Both lists outlive the derive calls, and field order releases the
        // matching input first, including when a callback raises.
        matching: PyListRef,
        rest: Option<PyListRef>,
    }

    fn split_recursive(
        exc: &PyObject,
        matcher: &ConditionMatcher,
        construct_rest: bool,
        vm: &VirtualMachine,
    ) -> PyResult<SplitResult> {
        if matcher.check(exc, vm)? {
            return Ok(SplitResult {
                matching: Some(exc.to_owned()),
                rest: None,
            });
        }
        let Some(group) = exc.downcast_ref::<PyBaseExceptionGroup>() else {
            return Ok(SplitResult {
                matching: None,
                rest: construct_rest.then(|| exc.to_owned()),
            });
        };

        let exceptions = group
            .excs
            .downcast_ref::<PyTuple>()
            .expect("exception group constructor stored an exact tuple");
        let lists = SplitLists {
            matching: vm.ctx.new_list(Vec::new()),
            rest: construct_rest.then(|| vm.ctx.new_list(Vec::new())),
        };
        for child in exceptions {
            // Recurse internally, without dispatching overridden subgroup or
            // split methods. No Python frame otherwise bounds this recursion.
            let result = vm.with_recursion(" in exceptiongroup_split_recursive", || {
                split_recursive(child, matcher, construct_rest, vm)
            })?;
            if let Some(child) = result.matching {
                lists.matching.borrow_vec_mut().push(child);
            }
            if let Some(child) = result.rest {
                lists
                    .rest
                    .as_ref()
                    .expect("rest is collected only for split")
                    .borrow_vec_mut()
                    .push(child);
            }
        }

        // Only a group-level match preserves identity. Matching every leaf
        // still invokes derive, even for subgroup(), and derive stays dynamic.
        let matching = if lists.matching.borrow_vec().is_empty() {
            None
        } else {
            Some(derive_and_copy_attributes_from_list(
                group,
                &lists.matching,
                vm,
            )?)
        };
        let rest = match &lists.rest {
            Some(rest) if !rest.borrow_vec().is_empty() => {
                Some(derive_and_copy_attributes_from_list(group, rest, vm)?)
            }
            _ => None,
        };
        Ok(SplitResult { matching, rest })
    }

    pub(crate) fn derive_and_copy_attributes(
        orig: &Py<PyBaseExceptionGroup>,
        excs: Vec<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let excs_seq = vm.ctx.new_list(excs);
        derive_and_copy_attributes_from_list(orig, &excs_seq, vm)
    }

    fn derive_and_copy_attributes_from_list(
        orig: &Py<PyBaseExceptionGroup>,
        excs: &PyListRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        // Call derive method to create new group while the caller retains the
        // exact list, whose elements the override may replace.
        let new_group = vm.call_method(orig.as_object(), "derive", (excs.clone(),))?;

        // Verify derive returned a BaseExceptionGroup
        if !is_base_exception_group(&new_group, vm) {
            return Err(vm.new_type_error("derive must return an instance of BaseExceptionGroup"));
        }

        let derived = new_group.try_downcast_ref::<PyBaseExceptionGroup>(vm)?;
        if let Some(tb) = orig.base.__traceback__() {
            derived.base.set_traceback(Some(tb));
        }
        derived.base.set_context(orig.base.__context__());
        derived.base.set_cause(orig.base.__cause__());

        // CPython copies notes only when they are a sequence, excluding dicts.
        if let Some(notes) = vm.get_attribute_opt(orig.as_object(), "__notes__")?
            && !notes.fast_isinstance(vm.ctx.types.dict_type)
            && notes.sequence_unchecked().check()
        {
            let notes_copy = vm
                .ctx
                .new_list(vm.extract_elements_sized(&notes, &|| 0, Ok)?);
            drop(notes);
            new_group.set_attr("__notes__", notes_copy, vm)?;
        }

        Ok(new_group)
    }
}
