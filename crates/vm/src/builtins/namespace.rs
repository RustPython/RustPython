use super::{PyStr, PyTupleRef, PyType, tuple::IntoPyTuple};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::PyDict,
    class::PyClassImpl,
    function::{KwArgs, NameChanges, OptionalArg, PyComparisonValue},
    recursion::ReprGuard,
    types::{
        Comparable, Constructor, DefaultConstructor, Initializer, PyComparisonOp, Representable,
    },
};
use rustpython_common::wtf8::Wtf8Buf;

#[pyclass(module = "types", name = "SimpleNamespace", unhashable = true)]
// The dict lives in the object extension, not the payload. The offset is
// the dict cell at the front of that extension.
#[pymember(name = "__dict__", offset = ::rustpython_vm::object::dict_member_offset())]
#[derive(Debug, Default)]
pub struct PyNamespace {}

impl PyPayload for PyNamespace {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.namespace_type
    }
}

impl DefaultConstructor for PyNamespace {}

#[pyclass(
    flags(BASETYPE, HAS_DICT),
    with(Constructor, Initializer, Comparable, Representable)
)]
impl PyNamespace {
    #[pymethod]
    fn __reduce__(zelf: PyObjectRef, vm: &VirtualMachine) -> PyTupleRef {
        let dict = zelf.as_object().dict().unwrap();
        let obj = zelf.as_object().to_owned();
        let result: (PyObjectRef, PyObjectRef, PyObjectRef) = (
            obj.class().to_owned().into(),
            vm.new_tuple(()).into(),
            dict.into(),
        );
        result.into_pytuple(vm)
    }

    #[pymethod]
    fn __replace__(
        zelf: PyObjectRef,
        changes: KwArgs<PyObjectRef, NameChanges>,
        vm: &VirtualMachine,
    ) -> PyResult {
        // Create a new instance of the same type
        let cls: PyObjectRef = zelf.class().to_owned().into();
        let result = cls.call((), vm)?;

        if !result.downcastable::<Self>() {
            return Err(vm.new_type_error(format!(
                "expect {} type, but {}() returned '{}' object",
                Self::class(&vm.ctx).slot_name(),
                zelf.class()
                    .__qualname__(vm)
                    .downcast_ref::<PyStr>()
                    .map_or_else(
                        || zelf.class().name().to_string(),
                        |n| n.as_wtf8().to_string()
                    ),
                result.class().name(),
            )));
        }

        // Copy the current namespace dict to the new instance
        let src_dict = zelf.dict().unwrap();
        let dst_dict = result.dict().unwrap();
        dst_dict.merge_dict(&src_dict, true, vm)?;

        // Bypass attribute setters and retain all argument owners during updates.
        for (name, value) in &changes {
            dst_dict.inner_setitem(name, value.clone(), vm)?;
        }

        Ok(result)
    }
}

#[derive(FromArgs)]
pub struct NamespaceArgs {
    #[pyarg(positional, default, py_default = "()")]
    mapping_or_iterable: OptionalArg<PyObjectRef>,
    #[pyarg(flatten)]
    kwargs: KwArgs,
}

impl Initializer for PyNamespace {
    type Args = NamespaceArgs;

    fn init(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        // Keep the original input alive through conversion and both updates.
        if let OptionalArg::Present(mapping) = &args.mapping_or_iterable {
            let dict: PyRef<PyDict> = if mapping.class().is(vm.ctx.types.dict_type) {
                mapping.downcast_ref::<PyDict>().unwrap().to_owned()
            } else {
                // Dict subclasses may override the mapping conversion protocol.
                let dict_type: PyObjectRef = vm.ctx.types.dict_type.to_owned().into();
                dict_type
                    .call((mapping.clone(),), vm)?
                    .downcast()
                    .map_err(|_| vm.new_type_error("dict() did not return a dict"))?
            };

            // Validate every key before publishing any of the supplied values.
            if dict
                .keys_vec()
                .iter()
                .any(|key| !key.downcastable::<PyStr>())
            {
                return Err(vm.new_type_error("keywords must be strings"));
            }
            zelf.as_object()
                .dict()
                .unwrap()
                .merge_dict(&dict, true, vm)?;
        }

        // Apply keywords directly to the namespace, without invoking descriptors.
        let dict = zelf.as_object().dict().unwrap();
        for (name, value) in &args.kwargs {
            dict.inner_setitem(name, value.clone(), vm)?;
        }
        Ok(())
    }
}

impl Comparable for PyNamespace {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        let other = class_or_notimplemented!(Self, other);
        let (d1, d2) = (
            zelf.as_object().dict().unwrap(),
            other.as_object().dict().unwrap(),
        );
        PyDict::cmp(&d1, d2.as_object(), op, vm)
    }
}

impl Representable for PyNamespace {
    #[inline]
    fn repr_wtf8(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Wtf8Buf> {
        let o = zelf.as_object();
        let name = if o.class().is(vm.ctx.types.namespace_type) {
            "namespace".to_owned()
        } else {
            o.class().slot_name().to_owned()
        };

        let repr = if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
            let dict = zelf.as_object().dict().unwrap();
            // Retain the entire key snapshot, but look up each value after any
            // preceding repr callback has had a chance to change the dictionary.
            let keys = dict.keys_vec();
            let result: PyResult<Wtf8Buf> = (|| {
                let mut result = Wtf8Buf::from(format!("{name}("));
                let mut first = true;
                for key in &keys {
                    let Some(key_str) = key.downcast_ref::<PyStr>() else {
                        continue;
                    };
                    if key_str.as_wtf8().is_empty() {
                        continue;
                    }
                    let Some(value) = dict.get_item_opt(&**key, vm)? else {
                        continue;
                    };
                    if !first {
                        result.push_str(", ");
                    }
                    first = false;
                    result.push_wtf8(key_str.as_wtf8());
                    result.push_char('=');
                    result.push_wtf8(value.repr(vm)?.as_wtf8());
                }
                result.push_char(')');
                Ok(result)
            })();
            // The key list releases its elements backwards, on success and
            // failure, while recursive calls from finalizers still see the guard.
            for _key in keys.into_iter().rev() {}
            result?
        } else {
            Wtf8Buf::from(format!("{name}(...)"))
        };
        Ok(repr)
    }
}

pub(crate) fn init(context: &'static Context) {
    PyNamespace::extend_class(context, context.types.namespace_type);
}
