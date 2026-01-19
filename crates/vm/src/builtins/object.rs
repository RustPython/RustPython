use super::{PyDictRef, PyList, PyStr, PyStrRef, PyType, PyTypeRef};
use crate::common::hash::PyHash;
use crate::types::PyTypeFlags;
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    class::PyClassImpl,
    convert::ToPyResult,
    function::{Either, FuncArgs, PyArithmeticValue, PyComparisonValue, PySetterValue},
    types::{Constructor, Initializer, PyComparisonOp},
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
    fn slot_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
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

                if let (Some(tp_init), Some(object_init)) = (tp_init, object_init)
                    && tp_init.is(&object_init)
                {
                    // Both __new__ and __init__ are object's versions,
                    // so the type accepts no arguments
                    return Err(vm.new_type_error(format!("{}() takes no arguments", cls.name())));
                }
                // If tp_init != object_init, then the type has custom __init__
                // which might accept arguments, so we allow it
            }
        }

        // more or less __new__ operator
        // Only create dict if the class has HAS_DICT flag (i.e., __slots__ was not defined
        // or __dict__ is in __slots__)
        let dict = if cls
            .slots
            .flags
            .has_feature(crate::types::PyTypeFlags::HAS_DICT)
        {
            Some(vm.ctx.new_dict())
        } else {
            None
        };

        // Ensure that all abstract methods are implemented before instantiating instance.
        if let Some(abs_methods) = cls.get_attr(identifier!(vm, __abstractmethods__))
            && let Some(unimplemented_abstract_method_count) = abs_methods.length_opt(vm)
        {
            let methods: Vec<PyStrRef> = abs_methods.try_to_value(vm)?;
            let methods: String =
                Itertools::intersperse(methods.iter().map(|name| name.as_str()), "', '").collect();

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

        Ok(crate::PyRef::new_ref(Self, cls, dict).into())
    }

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
    }
}

impl Initializer for PyBaseObject {
    type Args = FuncArgs;

    // object_init: excess_args validation
    fn slot_init(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        if args.is_empty() {
            return Ok(());
        }

        let typ = zelf.class();
        let object_type = &vm.ctx.types.object_type;

        let typ_init = typ.slots.init.load().map(|f| f as usize);
        let object_init = object_type.slots.init.load().map(|f| f as usize);

        // if (type->tp_init != object_init) → first error
        if typ_init != object_init {
            return Err(vm.new_type_error(
                "object.__init__() takes exactly one argument (the instance to initialize)"
                    .to_owned(),
            ));
        }

        let typ_new = typ.slots.new.load().map(|f| f as usize);
        let object_new = object_type.slots.new.load().map(|f| f as usize);

        // if (type->tp_new == object_new) → second error
        if typ_new == object_new {
            return Err(vm.new_type_error(format!(
                "{}.__init__() takes exactly one argument (the instance to initialize)",
                typ.name()
            )));
        }

        // Both conditions false → OK (e.g., tuple, dict with custom __new__)
        Ok(())
    }

    fn init(_zelf: PyRef<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
        unreachable!("slot_init is defined")
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

// object_getstate_default
fn object_getstate_default(obj: &PyObject, required: bool, vm: &VirtualMachine) -> PyResult {
    // Check itemsize
    if required && obj.class().slots.itemsize > 0 {
        return Err(vm.new_type_error(format!("cannot pickle {:.200} objects", obj.class().name())));
    }

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
        // Start with PyBaseObject_Type's basicsize
        let mut basicsize = vm.ctx.types.object_type.slots.basicsize;

        // Add __dict__ size if type has dict
        if obj.class().slots.flags.has_feature(PyTypeFlags::HAS_DICT) {
            basicsize += core::mem::size_of::<PyObjectRef>();
        }

        // Add __weakref__ size if type has weakref support
        let has_weakref = if let Some(ref ext) = obj.class().heaptype_ext {
            match &ext.slots {
                None => true, // Heap type without __slots__ has automatic weakref
                Some(slots) => slots.iter().any(|s| s.as_str() == "__weakref__"),
            }
        } else {
            let weakref_name = vm.ctx.intern_str("__weakref__");
            obj.class().attributes.read().contains_key(weakref_name)
        };
        if has_weakref {
            basicsize += core::mem::size_of::<PyObjectRef>();
        }

        // Add slots size
        if let Some(ref slot_names) = slot_names {
            basicsize += core::mem::size_of::<PyObjectRef>() * slot_names.__len__();
        }

        // Fail if actual type's basicsize > expected basicsize
        if obj.class().slots.basicsize > basicsize {
            return Err(vm.new_type_error(format!("cannot pickle '{}' object", obj.class().name())));
        }
    }

    if let Some(slot_names) = slot_names {
        let slot_names_len = slot_names.__len__();
        if slot_names_len > 0 {
            let slots = vm.ctx.new_dict();
            for i in 0..slot_names_len {
                let borrowed_names = slot_names.borrow_vec();
                // Check if slotnames changed during iteration
                if borrowed_names.len() != slot_names_len {
                    return Err(vm.new_runtime_error(
                        "__slotnames__ changed size during iteration".to_owned(),
                    ));
                }
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

// object_getstate
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

#[pyclass(with(Constructor, Initializer), flags(BASETYPE))]
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
                let cmp = zelf.class().slots.richcompare.load().unwrap();
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
    #[pyslot]
    fn slot_str(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyStrRef> {
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

    #[pygetset]
    fn __class__(obj: PyObjectRef) -> PyTypeRef {
        obj.class().to_owned()
    }

    #[pygetset(setter)]
    fn set___class__(
        instance: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match value.downcast::<PyType>() {
            Ok(cls) => {
                let current_cls = instance.class();
                let both_module = current_cls.fast_issubclass(vm.ctx.types.module_type)
                    && cls.fast_issubclass(vm.ctx.types.module_type);
                let both_mutable = !current_cls
                    .slots
                    .flags
                    .has_feature(PyTypeFlags::IMMUTABLETYPE)
                    && !cls.slots.flags.has_feature(PyTypeFlags::IMMUTABLETYPE);
                // FIXME(#1979) cls instances might have a payload
                if both_mutable || both_module {
                    let has_dict =
                        |typ: &Py<PyType>| typ.slots.flags.has_feature(PyTypeFlags::HAS_DICT);
                    // Compare slots tuples
                    let slots_equal = match (
                        current_cls
                            .heaptype_ext
                            .as_ref()
                            .and_then(|e| e.slots.as_ref()),
                        cls.heaptype_ext.as_ref().and_then(|e| e.slots.as_ref()),
                    ) {
                        (Some(a), Some(b)) => {
                            a.len() == b.len()
                                && a.iter()
                                    .zip(b.iter())
                                    .all(|(x, y)| x.as_str() == y.as_str())
                        }
                        (None, None) => true,
                        _ => false,
                    };
                    if current_cls.slots.basicsize != cls.slots.basicsize
                        || !slots_equal
                        || has_dict(current_cls) != has_dict(&cls)
                    {
                        return Err(vm.new_type_error(format!(
                            "__class__ assignment: '{}' object layout differs from '{}'",
                            cls.name(),
                            current_cls.name()
                        )));
                    }
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
    // Manually set init slot - derive macro doesn't generate extend_slots
    // for trait impl that overrides #[pyslot] method
    ctx.types
        .object_type
        .slots
        .init
        .store(Some(<PyBaseObject as Initializer>::slot_init));
    PyBaseObject::extend_class(ctx, ctx.types.object_type);
}

/// Get arguments for __new__ from __getnewargs_ex__ or __getnewargs__
/// Returns (args, kwargs) tuple where either can be None
fn get_new_arguments(
    obj: &PyObject,
    vm: &VirtualMachine,
) -> PyResult<(Option<super::PyTupleRef>, Option<super::PyDictRef>)> {
    // First try __getnewargs_ex__
    if let Some(getnewargs_ex) = vm.get_special_method(obj, identifier!(vm, __getnewargs_ex__))? {
        let newargs = getnewargs_ex.invoke((), vm)?;

        let newargs_tuple: PyRef<super::PyTuple> = newargs.downcast().map_err(|obj| {
            vm.new_type_error(format!(
                "__getnewargs_ex__ should return a tuple, not '{}'",
                obj.class().name()
            ))
        })?;

        if newargs_tuple.len() != 2 {
            return Err(vm.new_value_error(format!(
                "__getnewargs_ex__ should return a tuple of length 2, not {}",
                newargs_tuple.len()
            )));
        }

        let args = newargs_tuple.as_slice()[0].clone();
        let kwargs = newargs_tuple.as_slice()[1].clone();

        let args_tuple: PyRef<super::PyTuple> = args.downcast().map_err(|obj| {
            vm.new_type_error(format!(
                "first item of the tuple returned by __getnewargs_ex__ must be a tuple, not '{}'",
                obj.class().name()
            ))
        })?;

        let kwargs_dict: PyRef<super::PyDict> = kwargs.downcast().map_err(|obj| {
            vm.new_type_error(format!(
                "second item of the tuple returned by __getnewargs_ex__ must be a dict, not '{}'",
                obj.class().name()
            ))
        })?;

        return Ok((Some(args_tuple), Some(kwargs_dict)));
    }

    // Fall back to __getnewargs__
    if let Some(getnewargs) = vm.get_special_method(obj, identifier!(vm, __getnewargs__))? {
        let args = getnewargs.invoke((), vm)?;

        let args_tuple: PyRef<super::PyTuple> = args.downcast().map_err(|obj| {
            vm.new_type_error(format!(
                "__getnewargs__ should return a tuple, not '{}'",
                obj.class().name()
            ))
        })?;

        return Ok((Some(args_tuple), None));
    }

    // No __getnewargs_ex__ or __getnewargs__
    Ok((None, None))
}

/// Check if __getstate__ is overridden by comparing with object.__getstate__
fn is_getstate_overridden(obj: &PyObject, vm: &VirtualMachine) -> bool {
    let obj_cls = obj.class();
    let object_type = vm.ctx.types.object_type;

    // If the class is object itself, not overridden
    if obj_cls.is(object_type) {
        return false;
    }

    // Check if __getstate__ in the MRO comes from object or elsewhere
    // If the type has its own __getstate__, it's overridden
    if let Some(getstate) = obj_cls.get_attr(identifier!(vm, __getstate__))
        && let Some(obj_getstate) = object_type.get_attr(identifier!(vm, __getstate__))
    {
        return !getstate.is(&obj_getstate);
    }
    false
}

/// object_getstate - calls __getstate__ method or default implementation
fn object_getstate(obj: &PyObject, required: bool, vm: &VirtualMachine) -> PyResult {
    // If __getstate__ is not overridden, use the default implementation with required flag
    if !is_getstate_overridden(obj, vm) {
        return object_getstate_default(obj, required, vm);
    }

    // __getstate__ is overridden, call it without required
    let getstate = obj.get_attr(identifier!(vm, __getstate__), vm)?;
    getstate.call((), vm)
}

/// Get list items iterator if obj is a list (or subclass), None iterator otherwise
fn get_items_iter(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<(PyObjectRef, PyObjectRef)> {
    let listitems: PyObjectRef = if obj.fast_isinstance(vm.ctx.types.list_type) {
        obj.get_iter(vm)?.into()
    } else {
        vm.ctx.none()
    };

    let dictitems: PyObjectRef = if obj.fast_isinstance(vm.ctx.types.dict_type) {
        let items = vm.call_method(obj, "items", ())?;
        items.get_iter(vm)?.into()
    } else {
        vm.ctx.none()
    };

    Ok((listitems, dictitems))
}

/// reduce_newobj - creates reduce tuple for protocol >= 2
fn reduce_newobj(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    // Check if type has tp_new
    let cls = obj.class();
    if cls.slots.new.load().is_none() {
        return Err(vm.new_type_error(format!("cannot pickle '{}' object", cls.name())));
    }

    let (args, kwargs) = get_new_arguments(&obj, vm)?;

    let copyreg = vm.import("copyreg", 0)?;

    let has_args = args.is_some();

    let (newobj, newargs): (PyObjectRef, PyObjectRef) = if kwargs.is_none()
        || kwargs.as_ref().is_some_and(|k| k.is_empty())
    {
        // Use copyreg.__newobj__
        let newobj = copyreg.get_attr("__newobj__", vm)?;

        let args_vec: Vec<PyObjectRef> = args.map(|a| a.as_slice().to_vec()).unwrap_or_default();

        // Create (cls, *args) tuple
        let mut newargs_vec: Vec<PyObjectRef> = vec![cls.to_owned().into()];
        newargs_vec.extend(args_vec);
        let newargs = vm.ctx.new_tuple(newargs_vec);

        (newobj, newargs.into())
    } else {
        // args == NULL with non-empty kwargs is BadInternalCall
        let Some(args) = args else {
            return Err(vm.new_system_error("bad internal call".to_owned()));
        };
        // Use copyreg.__newobj_ex__
        let newobj = copyreg.get_attr("__newobj_ex__", vm)?;
        let args_tuple: PyObjectRef = args.into();
        let kwargs_dict: PyObjectRef = kwargs
            .map(|k| k.into())
            .unwrap_or_else(|| vm.ctx.new_dict().into());

        let newargs = vm
            .ctx
            .new_tuple(vec![cls.to_owned().into(), args_tuple, kwargs_dict]);
        (newobj, newargs.into())
    };

    // Determine if state is required
    // required = !(has_args || is_list || is_dict)
    let is_list = obj.fast_isinstance(vm.ctx.types.list_type);
    let is_dict = obj.fast_isinstance(vm.ctx.types.dict_type);
    let required = !(has_args || is_list || is_dict);

    let state = object_getstate(&obj, required, vm)?;

    let (listitems, dictitems) = get_items_iter(&obj, vm)?;

    let result = vm
        .ctx
        .new_tuple(vec![newobj, newargs, state, listitems, dictitems]);
    Ok(result.into())
}

fn common_reduce(obj: PyObjectRef, proto: usize, vm: &VirtualMachine) -> PyResult {
    if proto >= 2 {
        reduce_newobj(obj, vm)
    } else {
        let copyreg = vm.import("copyreg", 0)?;
        let reduce_ex = copyreg.get_attr("_reduce_ex", vm)?;
        reduce_ex.call((obj, proto), vm)
    }
}
