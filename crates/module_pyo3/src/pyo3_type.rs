//! `pyo3.pyo3_type`: CPython classes as seen from RustPython.
//!
//! A CPython class reaches RustPython as a wrapper class. Its metaclass is
//! the wrapper of the CPython metaclass, or `pyo3.pyo3_type` when that
//! metaclass is `type`. The wrapper mirrors the class's bases and namespace,
//! so RustPython's own machinery — `super()`, `isinstance()`, attribute
//! lookup through the MRO — works on it, and the metaclass forwards the rest
//! to the CPython class: attribute access and assignment, operators, and
//! calls, which create the instances in CPython.
//!
//! An instance of a class that can carry a [`Pyo3Ref`](crate::pyo3_ref::Pyo3Ref)
//! is an instance of that wrapper, so `type(proxy)` is the wrapper and the
//! wrapper's slots are exactly the methods the class defines. Other instances
//! stay generic proxies.
//!
//! A class statement deriving from a wrapped class creates the class in
//! CPython, with its body converted, so CPython code sees a genuine subclass.
//! RustPython-only bases (mixins) cannot be CPython bases: they remain bases
//! of the RustPython wrapper, and their attributes are copied into the CPython
//! class.
//!
//! CPython's built-in classes are not wrapped: CPython's `int` is RustPython's
//! `int`, and so on.

pub(crate) use pyo3_type::{
    instance_to_rustpython, is_cpython_backed, type_to_cpython, wrap_cpython_type,
};

#[pymodule(sub)]
pub(crate) mod pyo3_type {
    use pyo3::types::{PyDict, PyTuple, PyType as CPythonType};
    use pyo3::{
        Bound, PyAny, PyErr, Python, ffi,
        types::{PyAnyMethods, PyDictMethods, PyStringMethods, PyTupleMethods, PyTypeMethods},
    };
    use rustpython_vm::{
        AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyStr, PyTuple as RustPyTuple, PyType, PyTypeRef},
        class::{PyClassImpl, StaticType},
        function::{FuncArgs, KwArgs, PySetterValue},
        protocol::PyNumberMethods,
        types::{
            AsNumber, AtomicPyTypeFlags, Callable, GetAttr, PyTypeFlags, PyTypeSlots, SetAttr,
        },
    };

    use crate::_pyo3::module_state;
    use crate::convert::{
        call_cpython, call_cpython_with, str_to_cpython, to_cpython, to_rustpython,
    };
    use crate::errors::Pyo3ResultExt;
    use crate::gil::{run_cpython, with_gil};
    use crate::pyo3_ref::{Pyo3Ref, binary_op, buffer_method, call_method};
    use crate::rustpython_object::{RustPythonObject, unwrap_rustpython_object};

    /// Attribute of a wrapper class holding the CPython class it wraps.
    const CPYTHON_TYPE_ATTR: &str = "_pyo3_cpython_type";

    /// Metaclass of the classes wrapping CPython classes.
    #[pyattr]
    #[pyclass(module = "pyo3", name = "pyo3_type", base = PyType)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub(crate) struct Pyo3Type(PyType);

    #[pyclass(flags(BASETYPE), with(Callable, GetAttr, SetAttr, AsNumber))]
    impl Pyo3Type {
        /// Runs class statements deriving from wrapped classes.
        #[pyslot]
        fn slot_new(metatype: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let derives_from_cpython = args.args.len() == 3
                && args.args[1]
                    .downcast_ref::<RustPyTuple>()
                    .is_some_and(|bases| bases.iter().any(|base| is_cpython_backed(base, vm)));
            if derives_from_cpython {
                return with_gil(vm, |py| create_class(py, None, args, vm));
            }
            type_slot(|ty| ty.slots.new.load(), vm)(metatype, args, vm)
        }
    }

    /// Instantiates the CPython class.
    impl Callable for Pyo3Type {
        type Args = FuncArgs;

        fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let Some(wrapped) = wrapped_cpython_type(zelf.to_base(), vm) else {
                return type_slot(|ty| ty.slots.call.load(), vm)(zelf.as_object(), args, vm);
            };
            with_gil(vm, |py| {
                let cls = wrapped.py_obj.bind(py);
                let is_metaclass = cls
                    .cast::<CPythonType>()
                    .is_ok_and(|cls| cls.is_subclass_of::<CPythonType>().unwrap_or(false));
                if is_metaclass && args.args.len() == 3 {
                    return create_class(py, Some(cls.clone()), args, vm);
                }
                let result = call_cpython(cls, args, vm)?;
                to_rustpython(&result, vm)
            })
        }
    }

    impl GetAttr for Pyo3Type {
        fn getattro(zelf: &Py<Self>, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
            let wrapped = (!is_local_attr(name))
                .then(|| wrapped_cpython_type(zelf.to_base(), vm))
                .flatten();
            let Some(wrapped) = wrapped else {
                return type_slot(|ty| ty.slots.getattro.load(), vm)(zelf.as_object(), name, vm);
            };
            with_gil(vm, |py| {
                let name = str_to_cpython(py, name.as_wtf8()).to_pyresult(vm)?;
                let value = run_cpython(vm, || wrapped.py_obj.bind(py).getattr(name))?;
                to_rustpython(&value, vm)
            })
        }
    }

    impl SetAttr for Pyo3Type {
        fn setattro(
            zelf: &Py<Self>,
            name: &Py<PyStr>,
            value: PySetterValue,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let obj = zelf.as_object();
            let type_setattro = type_slot(|ty| ty.slots.setattro.load(), vm);
            let wrapped = (!is_local_attr(name))
                .then(|| wrapped_cpython_type(zelf.to_base(), vm))
                .flatten();
            let Some(wrapped) = wrapped else {
                return type_setattro(obj, name, value, vm);
            };
            // Assign in CPython, where the metaclass may act on it (ctypes
            // `_fields_`), then mirror what the class ended up with.
            let mirrored = with_gil(vm, |py| {
                let cls = wrapped.py_obj.bind(py);
                let cpython_name = str_to_cpython(py, name.as_wtf8()).to_pyresult(vm)?;
                let cpython_value = match &value {
                    PySetterValue::Assign(value) => Some(to_cpython(py, value, vm)?),
                    PySetterValue::Delete => None,
                };
                // A method copied from this interpreter must not run again:
                // `super().__setattr__` would swap `_fields_` twice and recurse.
                let bypass = if cpython_value.is_some() {
                    rustpython_setattr_bypass(cls).to_pyresult(vm)?
                } else {
                    None
                };
                run_cpython(vm, || match (&bypass, &cpython_value) {
                    (Some(descr), Some(value)) => {
                        let bound = descr.call_method1("__get__", (&cls, cls.get_type()))?;
                        bound.call1((&cpython_name, value)).map(|_| ())
                    }
                    (_, Some(value)) => cls.setattr(&cpython_name, value),
                    (_, None) => cls.delattr(&cpython_name),
                })?;
                cls.getattr(pyo3::intern!(py, "__dict__"))
                    .and_then(|namespace| namespace.get_item(&cpython_name))
                    .ok()
                    .map(|value| to_rustpython(&value, vm))
                    .transpose()
            })?;
            match mirrored {
                Some(value) => type_setattro(obj, name, PySetterValue::Assign(value), vm),
                None => match type_setattro(obj, name, PySetterValue::Delete, vm) {
                    Err(err) if err.fast_isinstance(vm.ctx.exceptions.attribute_error) => Ok(()),
                    result => result,
                },
            }
        }
    }

    /// `c_int * 4` and `Wrapped | None` are operators on classes.
    impl AsNumber for Pyo3Type {
        fn as_number() -> &'static PyNumberMethods {
            static AS_NUMBER: PyNumberMethods = PyNumberMethods {
                multiply: Some(|a, b, vm| binary_op(a, b, ("operator", "mul"), "mul", vm)),
                or: Some(|a, b, vm| binary_op(a, b, ("operator", "or_"), "or", vm)),
                ..PyNumberMethods::NOT_IMPLEMENTED
            };
            &AS_NUMBER
        }
    }

    fn metaclass() -> PyTypeRef {
        Pyo3Type::make_static_type()
    }

    /// The CPython class `ty` wraps. Subclasses do not inherit it.
    fn wrapped_cpython_type(ty: &PyType, vm: &VirtualMachine) -> Option<PyRef<Pyo3Ref>> {
        ty.get_direct_attr(vm.ctx.intern_str(CPYTHON_TYPE_ATTR))?
            .downcast::<Pyo3Ref>()
            .ok()
    }

    /// Whether `obj` lives in CPython: a proxy, or a class with a CPython
    /// counterpart.
    pub(crate) fn is_cpython_backed(obj: &PyObject, vm: &VirtualMachine) -> bool {
        obj.downcast_ref::<Pyo3Ref>().is_some()
            || obj.downcast_ref::<PyType>().is_some_and(|ty| {
                ty.mro_collect()
                    .iter()
                    .any(|cls| wrapped_cpython_type(cls, vm).is_some())
            })
    }

    /// `__setattr__` following methods copied from this interpreter.
    ///
    /// Those methods are on the metaclass so class creation invokes them, and
    /// `super().__setattr__` then lands here. Calling them again would swap
    /// `_fields_` a second time and recurse. `None` means the first
    /// `__setattr__` is already the one ordinary assignment should call.
    fn rustpython_setattr_bypass<'py>(
        cls: &Bound<'py, PyAny>,
    ) -> pyo3::PyResult<Option<Bound<'py, PyAny>>> {
        let py = cls.py();
        let mro = cls.get_type().mro();
        let mut passed_copied = false;
        for base in mro.iter() {
            let dict = base.getattr(pyo3::intern!(py, "__dict__"))?;
            let attr = dict.call_method1(
                pyo3::intern!(py, "get"),
                (pyo3::intern!(py, "__setattr__"),),
            )?;
            if attr.is_none() {
                continue;
            }
            if unwrap_rustpython_object(&attr).is_some() {
                passed_copied = true;
                continue;
            }
            if passed_copied {
                return Ok(Some(attr));
            }
            return Ok(None);
        }
        Ok(None)
    }

    /// Names a wrapper class answers itself, as a RustPython class, rather
    /// than forwarding to the CPython class.
    fn is_local_attr(name: &Py<PyStr>) -> bool {
        matches!(
            name.to_str(),
            Some(
                "__class__"
                    | "__bases__"
                    | "__base__"
                    | "__mro__"
                    | "__dict__"
                    | "mro"
                    | "__subclasses__"
                    | "__instancecheck__"
                    | "__subclasscheck__"
                    | CPYTHON_TYPE_ATTR
            )
        )
    }

    /// The CPython class standing for the RustPython class `ty`.
    pub(crate) fn type_to_cpython<'py>(
        py: Python<'py>,
        ty: &Py<PyType>,
        vm: &VirtualMachine,
    ) -> PyResult<Bound<'py, PyAny>> {
        if let Some(wrapped) = wrapped_cpython_type(ty, vm) {
            return Ok(wrapped.py_obj.bind(py).clone());
        }
        if is_cpython_backed(ty.as_object(), vm) {
            return cpython_subclass(py, ty, vm);
        }
        if let Some(builtin) = builtin_to_cpython(py, ty, vm) {
            return Ok(builtin);
        }
        Ok(RustPythonObject::wrap(py, ty.to_owned().into()))
    }

    /// The RustPython class standing for the CPython class `ty`.
    pub(crate) fn wrap_cpython_type(
        ty: &Bound<'_, CPythonType>,
        vm: &VirtualMachine,
    ) -> PyResult<PyTypeRef> {
        wrap_with_bases(ty, None, vm)
    }

    fn wrap_with_bases(
        ty: &Bound<'_, CPythonType>,
        bases: Option<Vec<PyTypeRef>>,
        vm: &VirtualMachine,
    ) -> PyResult<PyTypeRef> {
        let state = module_state(vm)?;
        if let Some(cached) = state.cached_type(ty) {
            return Ok(cached);
        }
        if let Some(builtin) = builtin_to_rustpython(ty, vm) {
            state.cache_type(ty, builtin.clone());
            return Ok(builtin);
        }

        let is_metaclass = ty.is_subclass_of::<CPythonType>().to_pyresult(vm)?;
        let meta_of_ty = ty.get_type();
        let metaclass = if is_exact_type(&meta_of_ty) {
            metaclass()
        } else {
            wrap_cpython_type(&meta_of_ty, vm)?
        };
        // Metaclasses are instances of `type`, not of `pyo3.ref`. A class whose
        // MRO names a built-in other than `object` cannot share that payload
        // either (`int`, `str`, ...).
        let ref_layout = !is_metaclass && allows_ref_payload(ty, vm);
        let bases = match bases {
            Some(bases) => bases,
            None => map_cpython_bases(ty, vm)?,
        };
        let bases = if ref_layout {
            ensure_ref_base(bases, vm)
        } else if is_metaclass {
            ensure_type_base(bases, vm)
        } else {
            bases
        };
        let basicsize = if ref_layout {
            Pyo3Ref::static_type().slots.basicsize
        } else {
            0
        };
        let name = ty.name().to_pyresult(vm)?.to_string();
        let slots = || {
            let mut slots = PyTypeSlots::default();
            slots.flags = AtomicPyTypeFlags::from_plain(PyTypeFlags::HEAP_TYPE);
            slots.basicsize = basicsize;
            slots
        };
        let fallback = if ref_layout {
            Pyo3Ref::static_type().to_owned()
        } else if is_metaclass {
            Pyo3Type::static_type().to_owned()
        } else {
            vm.ctx.types.object_type.to_owned()
        };
        // A mix of built-in bases RustPython cannot lay out together only
        // costs the wrapper those bases. The payload size stays `pyo3.ref`'s
        // when instances of the class are that payload.
        let wrapper = PyType::new_heap(
            &name,
            bases,
            Default::default(),
            slots(),
            metaclass.clone(),
            &vm.ctx,
        )
        .or_else(|_| {
            PyType::new_heap(
                &name,
                vec![fallback],
                Default::default(),
                slots(),
                metaclass,
                &vm.ctx,
            )
        })
        .map_err(|err| vm.new_type_error(err))?;
        wrapper.set_attr(
            vm.ctx.intern_str(CPYTHON_TYPE_ATTR),
            Pyo3Ref::new(ty.clone().into_any().unbind())
                .into_ref(&vm.ctx)
                .into(),
        );
        // Cached before the namespace is mirrored, which may refer back to it.
        state.cache_type(ty, wrapper.clone());
        mirror_namespace(ty, &wrapper, ref_layout, is_metaclass, vm)?;
        Ok(wrapper)
    }

    fn map_cpython_bases(
        ty: &Bound<'_, CPythonType>,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<PyTypeRef>> {
        ty.bases()
            .iter()
            .map(|base| {
                wrap_cpython_type(
                    base.cast::<CPythonType>()
                        .map_err(PyErr::from)
                        .to_pyresult(vm)?,
                    vm,
                )
            })
            .collect()
    }

    /// Drop plain `object` and make sure some base carries the proxy payload.
    fn ensure_ref_base(mut bases: Vec<PyTypeRef>, vm: &VirtualMachine) -> Vec<PyTypeRef> {
        let object = vm.ctx.types.object_type;
        let layout = Pyo3Ref::static_type();
        bases.retain(|base| !base.is(object));
        if !bases.iter().any(|base| base.fast_issubclass(layout)) {
            bases.push(layout.to_owned());
        }
        bases
    }

    /// A metaclass wrapper subclasses `pyo3.pyo3_type`, not the bare `type`.
    fn ensure_type_base(mut bases: Vec<PyTypeRef>, vm: &VirtualMachine) -> Vec<PyTypeRef> {
        let type_type = vm.ctx.types.type_type;
        let pyo3_type = Pyo3Type::static_type();
        for base in &mut bases {
            if base.is(type_type) {
                *base = pyo3_type.to_owned();
            }
        }
        if !bases.iter().any(|base| base.fast_issubclass(pyo3_type)) {
            bases.push(pyo3_type.to_owned());
        }
        bases
    }

    fn is_exact_type(ty: &Bound<'_, CPythonType>) -> bool {
        ty.is(ty.py().get_type::<CPythonType>())
    }

    /// `int`, `str` and the other built-ins RustPython lays out itself cannot
    /// share a payload with `pyo3.ref`. `object` can: it is what `pyo3.ref`
    /// extends.
    fn allows_ref_payload(ty: &Bound<'_, CPythonType>, vm: &VirtualMachine) -> bool {
        for base in ty.mro().iter() {
            let Ok(base) = base.cast::<CPythonType>() else {
                continue;
            };
            if let Some(rustpython) = builtin_to_rustpython(base, vm)
                && !rustpython.is(vm.ctx.types.object_type)
            {
                return false;
            }
        }
        true
    }

    fn is_heap_type(ty: &Bound<'_, CPythonType>) -> bool {
        // SAFETY: `ty` is a live type object. The call only reads its flag word.
        unsafe { ffi::PyType_HasFeature(ty.as_type_ptr(), ffi::Py_TPFLAGS_HEAPTYPE) != 0 }
    }

    /// Whether an instance of `ty` stays a generic proxy.
    ///
    /// Heap classes with a compatible layout become instances of their wrapper.
    /// A static type that owns `__get__` (`wrapper_descriptor`, `function`, ...)
    /// stays generic: binding those descriptors through a mirrored `__get__`
    /// would look `__get__` up again.
    fn use_generic_proxy(ty: &Bound<'_, CPythonType>, vm: &VirtualMachine) -> bool {
        !allows_ref_payload(ty, vm) || (!is_heap_type(ty) && owns_name(ty, "__get__"))
    }

    fn is_storage_descriptor(value: &Bound<'_, PyAny>) -> bool {
        let Ok(ty_name) = value.get_type().name() else {
            return false;
        };
        let Ok(ty_name) = ty_name.to_str() else {
            return false;
        };
        matches!(ty_name, "getset_descriptor" | "member_descriptor")
    }

    fn owns_name(ty: &Bound<'_, CPythonType>, name: &str) -> bool {
        let py = ty.py();
        ty.getattr(pyo3::intern!(py, "__dict__"))
            .and_then(|dict| dict.contains(name))
            .unwrap_or(false)
    }

    /// Copy the CPython class's `__dict__` into the wrapper, updating slots.
    fn mirror_namespace(
        ty: &Bound<'_, CPythonType>,
        wrapper: &Py<PyType>,
        ref_layout: bool,
        is_metaclass: bool,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let py = ty.py();
        let items = ty
            .getattr(pyo3::intern!(py, "__dict__"))
            .and_then(|namespace| namespace.call_method0(pyo3::intern!(py, "items")))
            .and_then(|items| items.try_iter())
            .to_pyresult(vm)?;
        for item in items {
            let (name, value): (String, Bound<'_, PyAny>) =
                item.and_then(|item| item.extract()).to_pyresult(vm)?;
            // getset and member descriptors describe the other interpreter's
            // layout. Installing them hits this type's own setters (`__name__`,
            // `__annotate__`) and would hide forwarded attribute access.
            if skip_mirror(&name, is_metaclass) || is_storage_descriptor(&value) {
                continue;
            }
            let mirrored = match (ref_layout, name.as_str()) {
                (true, "__buffer__") => buffer_method(vm),
                (true, "__call__") => call_method(vm),
                _ => to_rustpython(&value, vm)?,
            };
            let interned = vm.ctx.intern_str(name);
            PyType::setattro(wrapper, interned, PySetterValue::Assign(mirrored), vm)?;
        }
        Ok(())
    }

    /// Names that must not land in the wrapper dict.
    ///
    /// Attribute access stays the native forwarder, so `__getattribute__` and
    /// `__setattr__` are left alone. `__release_buffer__` is left alone too:
    /// the acquired buffer releases itself. On a metaclass, `__call__`,
    /// `__new__`, `__init__` and the class operators stay the metaclass's own
    /// slots. `type(name, bases, namespace)` runs that `__init__` again after
    /// the class has been created, and a ctypes metaclass refuses a second
    /// initialization.
    fn skip_mirror(name: &str, is_metaclass: bool) -> bool {
        matches!(
            name,
            "__dict__"
                | "__weakref__"
                | "__slots__"
                | "__class__"
                | "__bases__"
                | "__base__"
                | "__mro__"
                | "mro"
                | "__subclasses__"
                | "__instancecheck__"
                | "__subclasscheck__"
                | CPYTHON_TYPE_ATTR
                | "__flags__"
                | "__basicsize__"
                | "__itemsize__"
                | "__dictoffset__"
                | "__weakrefoffset__"
                | "__release_buffer__"
                | "__getattribute__"
                | "__getattr__"
                | "__setattr__"
                | "__delattr__"
        ) || (is_metaclass
            && matches!(
                name,
                "__call__" | "__init__" | "__mul__" | "__rmul__" | "__or__" | "__ror__" | "__new__"
            ))
    }

    /// An instance of `obj`'s class, when that class can carry the payload.
    pub(crate) fn instance_to_rustpython(obj: &Bound<'_, PyAny>, vm: &VirtualMachine) -> PyResult {
        let ty = obj.get_type();
        if use_generic_proxy(&ty, vm) {
            return Ok(Pyo3Ref::wrap(obj, vm));
        }
        let wrapper = wrap_cpython_type(&ty, vm)?;
        let layout = Pyo3Ref::static_type();
        if wrapper.fast_issubclass(layout) && wrapper.slots.basicsize == layout.slots.basicsize {
            return Pyo3Ref::new(obj.clone().unbind())
                .into_ref_with_type(vm, wrapper)
                .map(Into::into);
        }
        Ok(Pyo3Ref::wrap(obj, vm))
    }

    /// CPython's `None`, `int`, `ValueError`... are RustPython's.
    fn builtin_to_rustpython(
        ty: &Bound<'_, CPythonType>,
        vm: &VirtualMachine,
    ) -> Option<PyTypeRef> {
        let py = ty.py();
        let types = &vm.ctx.types;
        for (value, rustpython) in [
            (py.None(), types.none_type),
            (py.NotImplemented(), types.not_implemented_type),
            (py.Ellipsis(), types.ellipsis_type),
        ] {
            if ty.is(value.bind(py).get_type()) {
                return Some(rustpython.to_owned());
            }
        }
        if ty.module().ok()?.to_str().ok()? != "builtins" {
            return None;
        }
        let name = vm.ctx.new_str(ty.name().ok()?.to_str().ok()?);
        vm.builtins
            .get_attr(&name, vm)
            .ok()?
            .downcast::<PyType>()
            .ok()
    }

    fn builtin_to_cpython<'py>(
        py: Python<'py>,
        ty: &Py<PyType>,
        vm: &VirtualMachine,
    ) -> Option<Bound<'py, PyAny>> {
        let types = &vm.ctx.types;
        for (rustpython, value) in [
            (types.none_type, py.None()),
            (types.not_implemented_type, py.NotImplemented()),
            (types.ellipsis_type, py.Ellipsis()),
        ] {
            if ty.is(rustpython) {
                return Some(value.bind(py).get_type().into_any());
            }
        }
        let module = ty.__module__(vm).ok()?;
        if module.downcast_ref::<PyStr>()?.to_str()? != "builtins" {
            return None;
        }
        let builtin = py.import("builtins").ok()?.getattr(&*ty.name()).ok()?;
        builtin.is_instance_of::<CPythonType>().then_some(builtin)
    }

    /// A CPython class for a RustPython class deriving from a wrapped class
    /// without going through `pyo3.pyo3_type`, created on first use.
    fn cpython_subclass<'py>(
        py: Python<'py>,
        ty: &Py<PyType>,
        vm: &VirtualMachine,
    ) -> PyResult<Bound<'py, PyAny>> {
        let state = module_state(vm)?;
        if let Some(cpython) = state.cpython_type_of(py, ty) {
            return Ok(cpython);
        }
        let namespace = ty.as_object().get_attr("__dict__", vm)?;
        let bases = ty.bases.read().iter().map(|base| base.to_owned()).collect();
        let cpython = create_cpython_class(
            py,
            None,
            vm.ctx.new_str(ty.name().to_string()).into(),
            bases,
            &namespace,
            &KwArgs::default(),
            vm,
        )?;
        let cpython_type = cpython
            .cast::<CPythonType>()
            .map_err(PyErr::from)
            .to_pyresult(vm)?;
        state.cache_type(cpython_type, ty.to_owned());
        Ok(cpython)
    }

    /// Create a class in CPython from RustPython class-creation arguments.
    ///
    /// Bases with a CPython counterpart become its bases; the others are
    /// mixins, whose attributes are copied into the class namespace beneath
    /// the class body's own.
    fn create_cpython_class<'py>(
        py: Python<'py>,
        metaclass: Option<Bound<'py, PyAny>>,
        name: PyObjectRef,
        bases: Vec<PyTypeRef>,
        namespace: &PyObject,
        kwargs: &KwArgs,
        vm: &VirtualMachine,
    ) -> PyResult<Bound<'py, PyAny>> {
        let mut cpython_bases = Vec::new();
        let mut mixins = Vec::new();
        for base in bases {
            let backed = is_cpython_backed(base.as_object(), vm)
                || builtin_to_cpython(py, &base, vm).is_some();
            if backed {
                cpython_bases.push(type_to_cpython(py, &base, vm)?);
            } else {
                mixins.push(base);
            }
        }

        let cpython_namespace = PyDict::new(py);
        for mixin in mixins.iter().rev() {
            for cls in mixin.mro_collect().iter().rev() {
                if builtin_to_cpython(py, cls, vm).is_some() {
                    continue;
                }
                let namespace = cls.as_object().get_attr("__dict__", vm)?;
                for (key, value) in namespace_items(&namespace, vm)? {
                    let skip = key.downcast_ref::<PyStr>().is_some_and(|key| {
                        matches!(
                            key.to_str(),
                            Some(
                                "__dict__"
                                    | "__weakref__"
                                    | "__module__"
                                    | "__qualname__"
                                    | "__doc__"
                            )
                        )
                    });
                    if !skip {
                        cpython_namespace
                            .set_item(to_cpython(py, &key, vm)?, to_cpython(py, &value, vm)?)
                            .to_pyresult(vm)?;
                    }
                }
            }
        }
        for (key, value) in namespace_items(namespace, vm)? {
            // The cells are filled in on the RustPython side.
            let is_cell = key.downcast_ref::<PyStr>().is_some_and(|key| {
                matches!(key.to_str(), Some("__classcell__" | "__classdictcell__"))
            });
            if !is_cell {
                cpython_namespace
                    .set_item(to_cpython(py, &key, vm)?, to_cpython(py, &value, vm)?)
                    .to_pyresult(vm)?;
            }
        }

        let metaclass = match metaclass {
            Some(metaclass) => metaclass,
            None => most_derived_metaclass(py, &cpython_bases).to_pyresult(vm)?,
        };
        let cpython_bases = PyTuple::new(py, cpython_bases).to_pyresult(vm)?;
        let args = vec![
            to_cpython(py, &name, vm)?,
            cpython_bases.into_any(),
            cpython_namespace.into_any(),
        ];
        call_cpython_with(&metaclass, args, kwargs, vm)
    }

    fn most_derived_metaclass<'py>(
        py: Python<'py>,
        bases: &[Bound<'py, PyAny>],
    ) -> pyo3::PyResult<Bound<'py, PyAny>> {
        let mut winner = py.get_type::<CPythonType>();
        for base in bases {
            let metaclass = base.get_type();
            if metaclass.is_subclass(&winner)? {
                winner = metaclass;
            }
        }
        Ok(winner.into_any())
    }

    fn namespace_items(
        namespace: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<(PyObjectRef, PyObjectRef)>> {
        let keys = vm.call_method(namespace, "keys", ())?;
        keys.get_iter(vm)?
            .iter::<PyObjectRef>(vm)?
            .map(|key| {
                let key = key?;
                let value = namespace.get_item(&*key, vm)?;
                Ok((key, value))
            })
            .collect()
    }

    /// A class statement, or a `metaclass(name, bases, namespace)` call,
    /// creating a subclass of wrapped classes.
    fn create_class(
        py: Python<'_>,
        metaclass: Option<Bound<'_, PyAny>>,
        args: FuncArgs,
        vm: &VirtualMachine,
    ) -> PyResult {
        let [name, bases, namespace]: [PyObjectRef; 3] = args
            .args
            .try_into()
            .map_err(|_| vm.new_type_error("type() takes 1 or 3 arguments"))?;
        let bases = bases
            .downcast::<RustPyTuple>()
            .map_err(|_| vm.new_type_error("bases must be a tuple"))?
            .iter()
            .map(|base| {
                base.clone()
                    .downcast::<PyType>()
                    .map_err(|_| vm.new_type_error("bases must be classes"))
            })
            .collect::<PyResult<Vec<_>>>()?;
        let cpython = create_cpython_class(
            py,
            metaclass,
            name,
            bases.clone(),
            &namespace,
            &args.kwargs,
            vm,
        )?;
        let Ok(cpython_type) = cpython.cast::<CPythonType>() else {
            // The metaclass returned something other than a class.
            return to_rustpython(&cpython, vm);
        };
        let wrapper = wrap_with_bases(cpython_type, Some(bases), vm)?;

        let class_dict: Option<PyObjectRef> = wrapper
            .attributes
            .as_dict()
            .map(|dict| dict.to_owned().into());
        let cells = [
            ("__classcell__", Some(wrapper.clone().into())),
            ("__classdictcell__", class_dict),
        ];
        for (name, contents) in cells {
            if let Some(contents) = contents
                && let Ok(cell) = namespace.get_item(name, vm)
                && cell.class().is(vm.ctx.types.cell_type)
            {
                cell.set_attr("cell_contents", contents, vm)?;
            }
        }
        Ok(wrapper.into())
    }

    fn type_slot<T: Copy>(get: impl FnOnce(&PyType) -> Option<T>, vm: &VirtualMachine) -> T {
        get(vm.ctx.types.type_type).expect("type implements every slot pyo3_type overrides")
    }
}
