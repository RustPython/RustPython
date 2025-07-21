use super::{PyDictRef, PyList, PyStr, PyStrRef, PyType, PyTypeRef};
use crate::common::hash::PyHash;
use crate::types::PyTypeFlags;
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyResult, VirtualMachine,
    class::PyClassImpl,
    convert::ToPyResult,
    function::{Either, FuncArgs, PyArithmeticValue, PyComparisonValue, PySetterValue},
    types::{Constructor, PyComparisonOp},
};
use itertools::Itertools;

/// object()
/// --
///
/// The base class of the class hierarchy.
///
/// When called, it accepts no arguments and returns a new featureless
/// instance that has no instance attributes and cannot be given any.
#[pyclass(module = false, name = "object")]
#[derive(Debug)]
pub struct PyBaseObject;

impl PyPayload for PyBaseObject {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.object_type
    }
}

impl Constructor for PyBaseObject {
    type Args = FuncArgs;

    // = object_new
    fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        if !args.args.is_empty() || !args.kwargs.is_empty() {
            // Check if type's __new__ != object.__new__
            let tp_new = cls.get_attr(identifier!(vm, __new__));
            let object_new = vm.ctx.types.object_type.get_attr(identifier!(vm, __new__));

            if let (Some(tp_new), Some(object_new)) = (tp_new, object_new) {
                if !tp_new.is(&object_new) {
                    // Type has its own __new__, so object.__new__ is being called
                    // with excess args. This is the first error case in CPython
                    return Err(vm.new_type_error(
                        "object.__new__() takes exactly one argument (the type to instantiate)"
                            .to_owned(),
                    ));
                }

                // If we reach here, tp_new == object_new
                // Now check if type's __init__ == object.__init__
                let tp_init = cls.get_attr(identifier!(vm, __init__));
                let object_init = vm.ctx.types.object_type.get_attr(identifier!(vm, __init__));

                if let (Some(tp_init), Some(object_init)) = (tp_init, object_init) {
                    if tp_init.is(&object_init) {
                        // Both __new__ and __init__ are object's versions,
                        // so the type accepts no arguments
                        return Err(
                            vm.new_type_error(format!("{}() takes no arguments", cls.name()))
                        );
                    }
                }
                // If tp_init != object_init, then the type has custom __init__
                // which might accept arguments, so we allow it
            }
        }

        // more or less __new__ operator
        let dict = if cls.is(vm.ctx.types.object_type) {
            None
        } else {
            Some(vm.ctx.new_dict())
        };

        // Ensure that all abstract methods are implemented before instantiating instance.
        if let Some(abs_methods) = cls.get_attr(identifier!(vm, __abstractmethods__)) {
            if let Some(unimplemented_abstract_method_count) = abs_methods.length_opt(vm) {
                let methods: Vec<PyStrRef> = abs_methods.try_to_value(vm)?;
                let methods: String =
                    Itertools::intersperse(methods.iter().map(|name| name.as_str()), "', '")
                        .collect();

                let unimplemented_abstract_method_count = unimplemented_abstract_method_count?;
                let name = cls.name().to_string();

                match unimplemented_abstract_method_count {
                    0 => {}
                    1 => {
                        return Err(vm.new_type_error(format!(
                            "class {name} without an implementation for abstract method '{methods}'"
                        )));
                    }
                    2.. => {
                        return Err(vm.new_type_error(format!(
                            "class {name} without an implementation for abstract methods '{methods}'"
                        )));
                    }
                    // TODO: remove `allow` when redox build doesn't complain about it
                    #[allow(unreachable_patterns)]
                    _ => unreachable!(),
                }
            }
        }

        Ok(crate::PyRef::new_ref(Self, cls, dict).into())
    }
}

// TODO: implement _PyType_GetSlotNames properly
fn type_slot_names(typ: &Py<PyType>, vm: &VirtualMachine) -> PyResult<Option<super::PyListRef>> {
    // let attributes = typ.attributes.read();
    // if let Some(slot_names) = attributes.get(identifier!(vm.ctx, __slotnames__)) {
    //     return match_class!(match slot_names.clone() {
    //         l @ super::PyList => Ok(Some(l)),
    //         _n @ super::PyNone => Ok(None),
    //         _ => Err(vm.new_type_error(format!(
    //             "{:.200}.__slotnames__ should be a list or None, not {:.200}",
    //             typ.name(),
    //             slot_names.class().name()
    //         ))),
    //     });
    // }

    let copyreg = vm.import("copyreg", 0)?;
    let copyreg_slotnames = copyreg.get_attr("_slotnames", vm)?;
    let slot_names = copyreg_slotnames.call((typ.to_owned(),), vm)?;
    let result = match_class!(match slot_names {
        l @ super::PyList => Some(l),
        _n @ super::PyNone => None,
        _ => return Err(vm.new_type_error("copyreg._slotnames didn't return a list or None")),
    });
    Ok(result)
}

// object_getstate_default in CPython
fn object_getstate_default(obj: &PyObject, required: bool, vm: &VirtualMachine) -> PyResult {
    // TODO: itemsize
    // if required && obj.class().slots.itemsize > 0 {
    //     return vm.new_type_error(format!(
    //         "cannot pickle {:.200} objects",
    //         obj.class().name()
    //     ));
    // }

    let state = if obj.dict().is_none_or(|d| d.is_empty()) {
        vm.ctx.none()
    } else {
        // let state = object_get_dict(obj.clone(), obj.ctx()).unwrap();
        let Some(state) = obj.dict() else {
            return Ok(vm.ctx.none());
        };
        state.into()
    };

    let slot_names =
        type_slot_names(obj.class(), vm).map_err(|_| vm.new_type_error("cannot pickle object"))?;

    if required {
        let mut basicsize = obj.class().slots.basicsize;
        // if obj.class().slots.dict_offset > 0
        //     && !obj.class().slots.flags.has_feature(PyTypeFlags::MANAGED_DICT)
        // {
        //     basicsize += std::mem::size_of::<PyObjectRef>();
        // }
        // if obj.class().slots.weaklist_offset > 0 {
        //     basicsize += std::mem::size_of::<PyObjectRef>();
        // }
        if let Some(ref slot_names) = slot_names {
            basicsize += std::mem::size_of::<PyObjectRef>() * slot_names.__len__();
        }
        if obj.class().slots.basicsize > basicsize {
            return Err(
                vm.new_type_error(format!("cannot pickle {:.200} object", obj.class().name()))
            );
        }
    }

    if let Some(slot_names) = slot_names {
        let slot_names_len = slot_names.__len__();
        if slot_names_len > 0 {
            let slots = vm.ctx.new_dict();
            for i in 0..slot_names_len {
                let borrowed_names = slot_names.borrow_vec();
                let name = borrowed_names[i].downcast_ref::<PyStr>().unwrap();
                let Ok(value) = obj.get_attr(name, vm) else {
                    continue;
                };
                slots.set_item(name.as_str(), value, vm).unwrap();
            }

            if !slots.is_empty() {
                return (state, slots).to_pyresult(vm);
            }
        }
    }

    Ok(state)
}

// object_getstate in CPython
// fn object_getstate(
//     obj: &PyObject,
//     required: bool,
//     vm: &VirtualMachine,
// ) -> PyResult {
//     let getstate = obj.get_attr(identifier!(vm, __getstate__), vm)?;
//     if vm.is_none(&getstate) {
//         return Ok(None);
//     }

//     let getstate = match getstate.downcast_exact::<PyNativeFunction>(vm) {
//         Ok(getstate)
//             if getstate
//                 .get_self()
//                 .map_or(false, |self_obj| self_obj.is(obj))
//                 && std::ptr::addr_eq(
//                     getstate.as_func() as *const _,
//                     &PyBaseObject::__getstate__ as &dyn crate::function::PyNativeFn as *const _,
//                 ) =>
//         {
//             return object_getstate_default(obj, required, vm);
//         }
//         Ok(getstate) => getstate.into_pyref().into(),
//         Err(getstate) => getstate,
//     };
//     getstate.call((), vm)
// }

#[pyclass(with(Constructor), flags(BASETYPE))]
impl PyBaseObject {
    #[pymethod(raw)]
    fn __getstate__(vm: &VirtualMachine, args: FuncArgs) -> PyResult {
        let (zelf,): (PyObjectRef,) = args.bind(vm)?;
        object_getstate_default(&zelf, false, vm)
    }

    #[pyslot]
    fn slot_richcompare(
        zelf: &PyObject,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<Either<PyObjectRef, PyComparisonValue>> {
        Self::cmp(zelf, other, op, vm).map(Either::B)
    }

    #[inline(always)]
    fn cmp(
        zelf: &PyObject,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        let res = match op {
            PyComparisonOp::Eq => {
                if zelf.is(other) {
                    PyComparisonValue::Implemented(true)
                } else {
                    PyComparisonValue::NotImplemented
                }
            }
            PyComparisonOp::Ne => {
                let cmp = zelf
                    .class()
                    .mro_find_map(|cls| cls.slots.richcompare.load())
                    .unwrap();
                let value = match cmp(zelf, other, PyComparisonOp::Eq, vm)? {
                    Either::A(obj) => PyArithmeticValue::from_object(vm, obj)
                        .map(|obj| obj.try_to_bool(vm))
                        .transpose()?,
                    Either::B(value) => value,
                };
                value.map(|v| !v)
            }
            _ => PyComparisonValue::NotImplemented,
        };
        Ok(res)
    }

    /// Return self==value.
    #[pymethod]
    fn __eq__(
        zelf: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &value, PyComparisonOp::Eq, vm)
    }

    /// Return self!=value.
    #[pymethod]
    fn __ne__(
        zelf: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &value, PyComparisonOp::Ne, vm)
    }

    /// Return self<value.
    #[pymethod]
    fn __lt__(
        zelf: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &value, PyComparisonOp::Lt, vm)
    }

    /// Return self<=value.
    #[pymethod]
    fn __le__(
        zelf: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &value, PyComparisonOp::Le, vm)
    }

    /// Return self>=value.
    #[pymethod]
    fn __ge__(
        zelf: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &value, PyComparisonOp::Ge, vm)
    }

    /// Return self>value.
    #[pymethod]
    fn __gt__(
        zelf: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &value, PyComparisonOp::Gt, vm)
    }

    /// Implement setattr(self, name, value).
    #[pymethod]
    fn __setattr__(
        obj: PyObjectRef,
        name: PyStrRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        obj.generic_setattr(&name, PySetterValue::Assign(value), vm)
    }

    /// Implement delattr(self, name).
    #[pymethod]
    fn __delattr__(obj: PyObjectRef, name: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        obj.generic_setattr(&name, PySetterValue::Delete, vm)
    }

    #[pyslot]
    fn slot_setattro(
        obj: &PyObject,
        attr_name: &Py<PyStr>,
        value: PySetterValue,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        obj.generic_setattr(attr_name, value, vm)
    }

    /// Return str(self).
    #[pymethod]
    fn __str__(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        // FIXME: try tp_repr first and fallback to object.__repr__
        zelf.repr(vm)
    }

    #[pyslot]
    fn slot_repr(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        let class = zelf.class();
        match (
            class
                .__qualname__(vm)
                .downcast_ref::<PyStr>()
                .map(|n| n.as_str()),
            class
                .__module__(vm)
                .downcast_ref::<PyStr>()
                .map(|m| m.as_str()),
        ) {
            (None, _) => Err(vm.new_type_error("Unknown qualified name")),
            (Some(qualname), Some(module)) if module != "builtins" => Ok(PyStr::from(format!(
                "<{}.{} object at {:#x}>",
                module,
                qualname,
                zelf.get_id()
            ))
            .into_ref(&vm.ctx)),
            _ => Ok(PyStr::from(format!(
                "<{} object at {:#x}>",
                class.slot_name(),
                zelf.get_id()
            ))
            .into_ref(&vm.ctx)),
        }
    }

    /// Return repr(self).
    #[pymethod]
    fn __repr__(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        Self::slot_repr(&zelf, vm)
    }

    #[pyclassmethod]
    fn __subclasshook__(_args: FuncArgs, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.not_implemented()
    }

    #[pyclassmethod]
    fn __init_subclass__(_cls: PyTypeRef) {}

    #[pymethod]
    pub fn __dir__(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyList> {
        obj.dir(vm)
    }

    #[pymethod]
    fn __format__(
        obj: PyObjectRef,
        format_spec: PyStrRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyStrRef> {
        if !format_spec.is_empty() {
            return Err(vm.new_type_error(format!(
                "unsupported format string passed to {}.__format__",
                obj.class().name()
            )));
        }
        obj.str(vm)
    }

    #[pyslot]
    #[pymethod]
    fn __init__(_zelf: PyObjectRef, _args: FuncArgs, _vm: &VirtualMachine) -> PyResult<()> {
        Ok(())
    }

    #[pygetset(name = "__class__")]
    fn get_class(obj: PyObjectRef) -> PyTypeRef {
        obj.class().to_owned()
    }

    #[pygetset(name = "__class__", setter)]
    fn set_class(instance: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        match value.downcast::<PyType>() {
            Ok(cls) => {
                let both_module = instance.class().fast_issubclass(vm.ctx.types.module_type)
                    && cls.fast_issubclass(vm.ctx.types.module_type);
                let both_mutable = !instance
                    .class()
                    .slots
                    .flags
                    .has_feature(PyTypeFlags::IMMUTABLETYPE)
                    && !cls.slots.flags.has_feature(PyTypeFlags::IMMUTABLETYPE);
                // FIXME(#1979) cls instances might have a payload
                if both_mutable || both_module {
                    instance.set_class(cls, vm);
                    Ok(())
                } else {
                    Err(vm.new_type_error(
                        "__class__ assignment only supported for mutable types or ModuleType subclasses",
                    ))
                }
            }
            Err(value) => {
                let value_class = value.class();
                let type_repr = &value_class.name();
                Err(vm.new_type_error(format!(
                    "__class__ must be set to a class, not '{type_repr}' object"
                )))
            }
        }
    }

    /// Return getattr(self, name).
    #[pyslot]
    pub(crate) fn getattro(obj: &PyObject, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
        vm_trace!("object.__getattribute__({:?}, {:?})", obj, name);
        obj.as_object().generic_getattr(name, vm)
    }

    #[pymethod]
    fn __getattribute__(obj: PyObjectRef, name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        Self::getattro(&obj, &name, vm)
    }

    #[pymethod]
    fn __reduce__(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        common_reduce(obj, 0, vm)
    }

    #[pymethod]
    fn __reduce_ex__(obj: PyObjectRef, proto: usize, vm: &VirtualMachine) -> PyResult {
        let __reduce__ = identifier!(vm, __reduce__);
        if let Some(reduce) = vm.get_attribute_opt(obj.clone(), __reduce__)? {
            let object_reduce = vm.ctx.types.object_type.get_attr(__reduce__).unwrap();
            let typ_obj: PyObjectRef = obj.class().to_owned().into();
            let class_reduce = typ_obj.get_attr(__reduce__, vm)?;
            if !class_reduce.is(&object_reduce) {
                return reduce.call((), vm);
            }
        }
        common_reduce(obj, proto, vm)
    }

    #[pyslot]
    fn slot_hash(zelf: &PyObject, _vm: &VirtualMachine) -> PyResult<PyHash> {
        Ok(zelf.get_id() as _)
    }

    /// Return hash(self).
    #[pymethod]
    fn __hash__(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyHash> {
        Self::slot_hash(&zelf, vm)
    }

    #[pymethod]
    fn __sizeof__(zelf: PyObjectRef) -> usize {
        zelf.class().slots.basicsize
    }
}

pub fn object_get_dict(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyDictRef> {
    obj.dict()
        .ok_or_else(|| vm.new_attribute_error("This object has no __dict__"))
}
pub fn object_set_dict(obj: PyObjectRef, dict: PyDictRef, vm: &VirtualMachine) -> PyResult<()> {
    obj.set_dict(dict)
        .map_err(|_| vm.new_attribute_error("This object has no __dict__"))
}

pub fn init(ctx: &Context) {
    PyBaseObject::extend_class(ctx, ctx.types.object_type);
}

fn common_reduce(obj: PyObjectRef, proto: usize, vm: &VirtualMachine) -> PyResult {
    if proto >= 2 {
        let reducelib = vm.import("__reducelib", 0)?;
        let reduce_2 = reducelib.get_attr("reduce_2", vm)?;
        reduce_2.call((obj,), vm)
    } else {
        let copyreg = vm.import("copyreg", 0)?;
        let reduce_ex = copyreg.get_attr("_reduce_ex", vm)?;
        reduce_ex.call((obj, proto), vm)
    }
}
