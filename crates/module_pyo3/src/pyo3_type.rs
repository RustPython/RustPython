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
//! Built-in classes are not wrapped: `int` is the same class on both sides,
//! and so on.

pub(crate) use pyo3_type::{
    install_class_creation, instance_to_rustpython, is_cpython_backed, type_to_cpython,
    wrap_cpython_type,
};

#[pymodule(sub)]
pub(crate) mod pyo3_type {
    use core::cell::RefCell;

    use pyo3::types::{PyDict, PyTuple, PyType as CPythonType};
    use pyo3::{
        Bound, PyAny, PyErr, Python, ffi,
        types::{PyAnyMethods, PyDictMethods, PyStringMethods, PyTupleMethods, PyTypeMethods},
    };
    use rustpython_vm::{
        AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{
            PyClassMethod, PyDict as RustPyDict, PyStr, PyTuple as RustPyTuple, PyType, PyTypeRef,
        },
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
        /// The wrapped class's MRO, without the layout bases added on this side.
        #[pygetset]
        fn __mro__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            if let Some(mro) = published_mro(zelf.to_base(), vm)? {
                let items = mro.into_iter().map(Into::into).collect();
                return Ok(vm.ctx.new_tuple(items).into());
            }
            type_descriptor(zelf.to_base(), "__mro__", vm)
        }

        #[pymethod]
        fn mro(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            if let Some(mro) = published_mro(zelf.to_base(), vm)? {
                let items = mro.into_iter().map(Into::into).collect();
                return Ok(vm.ctx.new_list(items).into());
            }
            type_descriptor(zelf.to_base(), "mro", vm)?.call((), vm)
        }
    }

    /// Instantiates the wrapped class, or creates a subclass of one.
    impl Callable for Pyo3Type {
        type Args = FuncArgs;

        fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let ty = zelf.to_base();
            let wrapped = wrapped_cpython_type(ty, vm);
            let metaclass = ty.fast_issubclass(vm.ctx.types.type_type);
            // `type.__new__` takes a dict. `__prepare__` on a wrapped metaclass
            // would otherwise hand back a proxy of the other interpreter's dict.
            let args = if metaclass {
                ensure_dict_namespace(args, vm)?
            } else {
                args
            };
            if let Some(wrapped) = wrapped.as_ref()
                && metaclass
                && args.args.len() == 3
                && !defines_python_new(ty, vm)
            {
                // No Python `__new__` runs for this metaclass, so the class can
                // be created directly. `tp_new` stays `type.__new__`: the safety
                // check compares those pointers.
                return with_gil(vm, |py| {
                    create_class(py, Some(wrapped.py_obj.bind(py).clone()), args, vm, true)
                });
            }
            if let Some(wrapped) = wrapped.as_ref()
                && !metaclass
            {
                return with_gil(vm, |py| {
                    let result = call_cpython(wrapped.py_obj.bind(py), args, vm)?;
                    to_rustpython(&result, vm)
                });
            }
            // A heap type allocated by `type.__new__` has no wrapped class yet.
            // Instantiating it still has to reach the class that owns storage.
            if wrapped.is_none() && !metaclass && is_cpython_backed(zelf.as_object(), vm) {
                return with_gil(vm, |py| {
                    let cls = cpython_subclass(py, ty, vm)?;
                    let result = call_cpython(&cls, args, vm)?;
                    to_rustpython(&result, vm)
                });
            }
            // A Python `__new__` writes into the namespace object it received
            // (`namespace["_type_"] = ...`). Calling the wrapped metaclass
            // copies that dict, so the write never reaches `tp_init`.
            // `type.__call__` runs `__new__` and `__init__` on the original.
            type_slot(|ty| ty.slots.call.load(), vm)(zelf.as_object(), args, vm)
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
                // The copied method runs once, so it can rewrite the value.
                // `super().__setattr__` lands here again and must call the next
                // setter directly; running the copied method twice swaps
                // `_fields_` twice and recurses.
                let bypass = if cpython_value.is_some() {
                    rustpython_setattr_bypass(cls).to_pyresult(vm)?
                } else {
                    None
                };
                run_cpython(vm, || match (&bypass, &cpython_value) {
                    (Some(bypass), Some(value)) => {
                        call_bypassed_setattr(cls, bypass, &cpython_name, value)
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

    /// A function `__new__` anywhere in `ty`'s MRO.
    ///
    /// `type.__new__` is not one: it is the builtin slot. A staticmethod
    /// wrapped around a function is the form a class body stores.
    fn defines_python_new(ty: &Py<PyType>, vm: &VirtualMachine) -> bool {
        let name = vm.ctx.intern_str("__new__");
        ty.mro_collect().iter().any(|cls| {
            cls.get_direct_attr(name)
                .is_some_and(|value| is_python_function(&value, vm))
        })
    }

    /// Class creation stores the body in a real dict.
    ///
    /// A mapping that is not a dict is copied. `type.__new__` accepts only a
    /// dict, and later reads (`namespace["_type_"]`) have to see the same
    /// object `__new__` and `__init__` received.
    fn ensure_dict_namespace(mut args: FuncArgs, vm: &VirtualMachine) -> PyResult<FuncArgs> {
        if args.args.len() < 3 {
            return Ok(args);
        }
        let namespace = &args.args[2];
        if namespace.downcast_ref::<RustPyDict>().is_some() {
            return Ok(args);
        }
        if namespace.get_attr(vm.ctx.intern_str("keys"), vm).is_err() {
            return Ok(args);
        }
        let copied = vm.ctx.new_dict();
        copied.merge_object(namespace.clone(), vm)?;
        args.args[2] = copied.into();
        Ok(args)
    }

    fn is_python_function(value: &PyObject, vm: &VirtualMachine) -> bool {
        let function = vm.ctx.types.function_type;
        if value.class().is(function) {
            return true;
        }
        value.class().is(vm.ctx.types.staticmethod_type)
            && value
                .get_attr("__func__", vm)
                .is_ok_and(|func| func.class().is(function))
    }

    /// Wrapped metaclasses use the same `tp_new` as `type`.
    ///
    /// `type.__new__(PyCSimpleType)` is safe only when the two pointers match.
    /// A different pointer reports `type.__new__(X) is not safe, use Y.__new__()`.
    pub(crate) fn install_class_creation(vm: &VirtualMachine) {
        if let Some(type_new) = vm.ctx.types.type_type.slots.new.load() {
            Pyo3Type::static_type().slots.new.store(Some(type_new));
        }
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

    /// A `__setattr__` copied from this interpreter, and the next one after it.
    struct CopiedSetattr<'py> {
        copied: Bound<'py, PyAny>,
        next: Bound<'py, PyAny>,
    }

    std::thread_local! {
        static SETATTR_REENTRY: core::cell::RefCell<Vec<usize>> =
            const { core::cell::RefCell::new(Vec::new()) };
    }

    /// `__setattr__` copied from this interpreter, followed by the next one.
    ///
    /// The copied method rewrites `_fields_` and then calls
    /// `super().__setattr__`, which assigns on this same class again. `None`
    /// means the first `__setattr__` is already the one ordinary assignment
    /// should call.
    fn rustpython_setattr_bypass<'py>(
        cls: &Bound<'py, PyAny>,
    ) -> pyo3::PyResult<Option<CopiedSetattr<'py>>> {
        let py = cls.py();
        let mro = cls.get_type().mro();
        let mut copied = None;
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
                if copied.is_none() {
                    copied = Some(attr);
                }
                continue;
            }
            if let Some(copied) = copied {
                return Ok(Some(CopiedSetattr { copied, next: attr }));
            }
            return Ok(None);
        }
        Ok(None)
    }

    /// Call the copied setter, or on re-entry for this class the one after it.
    fn call_bypassed_setattr(
        cls: &Bound<'_, PyAny>,
        bypass: &CopiedSetattr<'_>,
        name: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> pyo3::PyResult<()> {
        let key = cls.as_ptr() as usize;
        let outer = SETATTR_REENTRY.with(|stack| {
            let mut stack = stack.borrow_mut();
            if stack.contains(&key) {
                false
            } else {
                stack.push(key);
                true
            }
        });
        struct Guard {
            key: usize,
            outer: bool,
        }
        impl Drop for Guard {
            fn drop(&mut self) {
                if self.outer {
                    SETATTR_REENTRY.with(|stack| {
                        let mut stack = stack.borrow_mut();
                        if let Some(pos) = stack.iter().rposition(|item| *item == self.key) {
                            stack.remove(pos);
                        }
                    });
                }
            }
        }
        let _guard = Guard { key, outer };
        let descr = if outer { &bypass.copied } else { &bypass.next };
        let bound = descr.call_method1("__get__", (cls, cls.get_type()))?;
        bound.call1((name, value)).map(|_| ())
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
                    | "__prepare__"
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
        RustPythonObject::wrap(py, ty.to_owned().into(), vm).to_pyresult(vm)
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
            // Re-entered allocation was given only the C bases. Mixins are
            // on the class statement's list, recorded outside this call.
            Some(bases) => restore_mixin_bases(bases),
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
        // Managed weak references live in a prefix, so the payload size stays
        // `pyo3.ref`'s. `__dict__` stays forwarded and is not added here.
        let weakref = ref_layout && cpython_type_supports_weakref(ty);
        let slots = || {
            let mut flags = PyTypeFlags::HEAP_TYPE;
            if weakref {
                flags.insert(PyTypeFlags::HAS_WEAKREF);
                flags.insert(PyTypeFlags::MANAGED_WEAKREF);
            }
            let mut slots = PyTypeSlots::default();
            slots.flags = AtomicPyTypeFlags::from_plain(flags);
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
        // `type(name, bases, namespace)` allocates with `tp_new` and then
        // calls this metaclass's `tp_init` slot. `super()` reads `__init__`
        // from the dict and binds it, so the dict entry is the C initializer
        // itself: a function that does not bind drops the instance and the
        // base initializer never runs.
        if is_metaclass {
            let init = ty
                .getattr(pyo3::intern!(ty.py(), "__init__"))
                .to_pyresult(vm)?;
            let init = to_rustpython(&init, vm)?;
            wrapper.set_attr(vm.ctx.intern_str("__init__"), init);
            wrapper.slots.init.store(Some(metaclass_init));
        }
        Ok(wrapper)
    }

    fn cpython_type_supports_weakref(ty: &Bound<'_, CPythonType>) -> bool {
        // SAFETY: `ty` is a live type object. Only its weak-reference offset
        // and flag word are read.
        unsafe {
            let ptr = ty.as_type_ptr();
            (*ptr).tp_weaklistoffset != 0
                || pyo3::ffi::PyType_HasFeature(ptr, pyo3::ffi::Py_TPFLAGS_MANAGED_WEAKREF) != 0
        }
    }

    /// Classes of the wrapped type, in its own MRO order.
    fn published_mro(zelf: &Py<PyType>, vm: &VirtualMachine) -> PyResult<Option<Vec<PyTypeRef>>> {
        let Some(wrapped) = wrapped_cpython_type(zelf, vm) else {
            return Ok(None);
        };
        with_gil(vm, |py| {
            let obj = wrapped.py_obj.bind(py);
            let Ok(cls) = obj.cast::<CPythonType>() else {
                return Ok(None);
            };
            let mut mro = Vec::new();
            for base in cls.mro().iter() {
                let base = base
                    .cast::<CPythonType>()
                    .map_err(PyErr::from)
                    .to_pyresult(vm)?;
                mro.push(wrap_cpython_type(base, vm)?);
            }
            Ok(Some(mro))
        })
    }

    /// `type`'s own `__mro__` or `mro`, bound to `zelf`, without looking the
    /// name up on `zelf` again.
    fn type_descriptor(zelf: &Py<PyType>, name: &str, vm: &VirtualMachine) -> PyResult {
        let type_type = vm.ctx.types.type_type;
        let Some(descr) = type_type.get_direct_attr(vm.ctx.intern_str(name)) else {
            return Err(
                vm.new_attribute_error(format!("type object 'type' has no attribute '{name}'"))
            );
        };
        let Some(descr_get) = descr.class().slots.descr_get.load() else {
            return Ok(descr);
        };
        descr_get(
            descr.as_object(),
            Some(zelf.as_object()),
            Some(type_type.as_object()),
            vm,
        )
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
        if let Some(cached) = Pyo3Ref::cached(obj) {
            return Ok(cached);
        }
        if use_generic_proxy(&ty, vm) {
            return Ok(Pyo3Ref::wrap(obj, vm));
        }
        let wrapper = wrap_cpython_type(&ty, vm)?;
        let layout = Pyo3Ref::static_type();
        if wrapper.fast_issubclass(layout) && wrapper.slots.basicsize == layout.slots.basicsize {
            let proxy = Pyo3Ref::tracked(obj.clone().unbind())
                .into_ref_with_type(vm, wrapper)
                .map(Into::into)?;
            return Ok(Pyo3Ref::publish(proxy));
        }
        Ok(Pyo3Ref::wrap(obj, vm))
    }

    /// `None`, `int`, `ValueError`... are this interpreter's.
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

    /// The class already paired with `ty`, if one exists.
    fn bound_cpython_type<'py>(
        py: Python<'py>,
        ty: &Py<PyType>,
        vm: &VirtualMachine,
    ) -> PyResult<Option<Bound<'py, PyAny>>> {
        if let Some(wrapped) = wrapped_cpython_type(ty, vm) {
            return Ok(Some(wrapped.py_obj.bind(py).clone()));
        }
        let Some(found) = module_state(vm)?.cpython_type_of(py, ty) else {
            return Ok(None);
        };
        if let Ok(c_ty) = found.cast::<CPythonType>() {
            bind_cpython_type(ty, c_ty, vm)?;
        }
        Ok(Some(found))
    }

    /// Remember that `ty` is the class standing for `cpython`.
    ///
    /// The attribute is what attribute access forwards through. The cache is
    /// what a later lookup finds before that attribute is read.
    fn bind_cpython_type(
        ty: &Py<PyType>,
        cpython: &Bound<'_, CPythonType>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if wrapped_cpython_type(ty, vm).is_none() {
            ty.set_attr(
                vm.ctx.intern_str(CPYTHON_TYPE_ATTR),
                Pyo3Ref::new(cpython.clone().into_any().unbind())
                    .into_ref(&vm.ctx)
                    .into(),
            );
        }
        module_state(vm)?.cache_type(cpython, ty.to_owned());
        Ok(())
    }

    std::thread_local! {
        static MATERIALIZING: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    }

    struct Materializing;

    impl Materializing {
        /// `None` when `key` is already on the stack.
        fn enter(key: usize) -> Option<Self> {
            let entered = MATERIALIZING.with(|stack| {
                let mut stack = stack.borrow_mut();
                if stack.contains(&key) {
                    false
                } else {
                    stack.push(key);
                    true
                }
            });
            entered.then_some(Self)
        }
    }

    impl Drop for Materializing {
        fn drop(&mut self) {
            MATERIALIZING.with(|stack| {
                stack.borrow_mut().pop();
            });
        }
    }

    /// A class for `ty`, which already exists on this side.
    ///
    /// The metaclass initializes only when nothing else will. A backed
    /// metaclass's `tp_init` runs after `__new__` returns, or from
    /// [`metaclass_init`]. Calling it here as well raises `already initialized`.
    fn cpython_subclass<'py>(
        py: Python<'py>,
        ty: &Py<PyType>,
        vm: &VirtualMachine,
    ) -> PyResult<Bound<'py, PyAny>> {
        if let Some(existing) = bound_cpython_type(py, ty, vm)? {
            return Ok(existing);
        }
        let key = ty.as_object().as_raw() as usize;
        let Some(_guard) = Materializing::enter(key) else {
            return Err(
                vm.new_runtime_error("reentered conversion of a class that is still being created")
            );
        };
        // The check above can miss a class published by the reentered call.
        if let Some(existing) = bound_cpython_type(py, ty, vm)? {
            return Ok(existing);
        }
        let metaclass_ty = ty.class().to_owned();
        let metaclass_backed = is_cpython_backed(metaclass_ty.as_object(), vm);
        let metaclass = if metaclass_backed {
            Some(type_to_cpython(py, &metaclass_ty, vm)?)
        } else {
            None
        };
        let initialize = !metaclass_backed;
        let namespace = ty.as_object().get_attr("__dict__", vm)?;
        let bases = ty.bases.read().iter().map(|base| base.to_owned()).collect();
        let (metaclass, args) = create_cpython_class(
            py,
            metaclass,
            vm.ctx.new_str(ty.name().to_string()).into(),
            bases,
            &namespace,
            vm,
        )?;
        let cpython = call_new_class(py, metaclass, args, &KwArgs::default(), vm, initialize)?;
        let cpython_type = cpython
            .cast::<CPythonType>()
            .map_err(PyErr::from)
            .to_pyresult(vm)?;
        bind_cpython_type(ty, cpython_type, vm)?;
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
        vm: &VirtualMachine,
    ) -> PyResult<(Bound<'py, PyAny>, Vec<Bound<'py, PyAny>>)> {
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
            // A metaclass mixin contributes `__new__` but not `__init__`.
            // Copying `__init__` would replace the base metaclass initializer.
            let skip_init = mixin.fast_issubclass(vm.ctx.types.type_type);
            for cls in mixin.mro_collect().iter().rev() {
                if builtin_to_cpython(py, cls, vm).is_some() {
                    continue;
                }
                let namespace = cls.as_object().get_attr("__dict__", vm)?;
                for (key, value) in namespace_items(&namespace, vm)? {
                    let skip = key
                        .downcast_ref::<PyStr>()
                        .is_some_and(|key| match key.to_str() {
                            Some(
                                "__dict__" | "__weakref__" | "__module__" | "__qualname__"
                                | "__doc__",
                            ) => true,
                            Some("__init__") => skip_init,
                            _ => false,
                        });
                    if !skip {
                        let value = namespace_value_to_cpython(py, &key, value, vm)?;
                        cpython_namespace
                            .set_item(to_cpython(py, &key, vm)?, value)
                            .to_pyresult(vm)?;
                    }
                }
            }
        }
        for (key, value) in namespace_items(namespace, vm)? {
            // The cells are filled in on this side. Layout descriptors belong
            // to the class that was already created; copying them installs the
            // other interpreter's slots on the new class.
            let skip = is_layout_descriptor(&value)
                || key.downcast_ref::<PyStr>().is_some_and(|key| {
                    matches!(
                        key.to_str(),
                        Some(
                            "__classcell__"
                                | "__classdictcell__"
                                | "__dict__"
                                | "__weakref__"
                                | CPYTHON_TYPE_ATTR
                        )
                    )
                });
            if !skip {
                let value = namespace_value_to_cpython(py, &key, value, vm)?;
                cpython_namespace
                    .set_item(to_cpython(py, &key, vm)?, value)
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
        Ok((metaclass, args))
    }

    /// Class statements call the metaclass (`tp_new` and `tp_init`).
    /// `type.__new__` only allocates; the following `slots.init` initializes.
    fn call_new_class<'py>(
        py: Python<'py>,
        metaclass: Bound<'py, PyAny>,
        args: Vec<Bound<'py, PyAny>>,
        kwargs: &KwArgs,
        vm: &VirtualMachine,
        initialize: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let (callable, positional) = if initialize {
            (metaclass, args)
        } else {
            let type_new = py
                .get_type::<CPythonType>()
                .getattr("__new__")
                .to_pyresult(vm)?;
            let mut positional = Vec::with_capacity(args.len() + 1);
            positional.push(metaclass);
            positional.extend(args);
            (type_new, positional)
        };
        call_cpython_with(&callable, positional, kwargs, vm)
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

    /// A plain class used as `_type_` has no storage of its own. An empty
    /// heap type is enough for the type check, and it is not cached: nothing
    /// else must keep a placeholder for every class that was named once.
    fn is_plain_class(py: Python<'_>, ty: &Py<PyType>, vm: &VirtualMachine) -> bool {
        !is_cpython_backed(ty.as_object(), vm) && builtin_to_cpython(py, ty, vm).is_none()
    }

    fn plain_type_placeholder<'py>(
        py: Python<'py>,
        ty: &Py<PyType>,
        vm: &VirtualMachine,
    ) -> PyResult<Bound<'py, PyAny>> {
        let name = to_cpython(py, vm.ctx.new_str(ty.name().to_string()).as_object(), vm)?;
        let bases = PyTuple::new(py, [py.get_type::<PyAny>()]).to_pyresult(vm)?;
        let namespace = PyDict::new(py);
        run_cpython(vm, || {
            py.get_type::<CPythonType>().call1((name, bases, namespace))
        })
    }

    fn convert_namespace_arg<'py>(
        py: Python<'py>,
        arg: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<Bound<'py, PyAny>> {
        if arg
            .downcast_ref::<rustpython_vm::builtins::PyDict>()
            .is_none()
        {
            return to_cpython(py, arg, vm);
        }
        let converted = PyDict::new(py);
        for (key, value) in namespace_items(arg, vm)? {
            let value = namespace_value_to_cpython(py, &key, value, vm)?;
            converted
                .set_item(to_cpython(py, &key, vm)?, value)
                .to_pyresult(vm)?;
        }
        Ok(converted.into_any())
    }

    fn namespace_value_to_cpython<'py>(
        py: Python<'py>,
        key: &PyObject,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<Bound<'py, PyAny>> {
        let value = promote_class_method(key, value, vm);
        if key
            .downcast_ref::<PyStr>()
            .is_some_and(|key| matches!(key.to_str(), Some("_type_")))
            && let Some(ty) = value.downcast_ref::<PyType>()
            && is_plain_class(py, ty, vm)
        {
            return plain_type_placeholder(py, ty, vm);
        }
        to_cpython(py, &value, vm)
    }

    /// Plain functions named `__init_subclass__` or `__class_getitem__` are
    /// class methods. A value that is already a classmethod stays as it is.
    fn promote_class_method(
        key: &PyObject,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyObjectRef {
        let promote = key.downcast_ref::<PyStr>().is_some_and(|key| {
            matches!(
                key.to_str(),
                Some("__init_subclass__" | "__class_getitem__")
            )
        }) && value.class().is(vm.ctx.types.function_type);
        if promote {
            PyClassMethod::from(value).into_pyobject(vm)
        } else {
            value
        }
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

    // Bases of the class statement currently running.
    //
    // Re-entering allocation sees only the bases that became C bases.
    // Mixins stay on the statement's own list, so use that list when it
    // still names every base the inner call received.
    thread_local! {
        static PENDING_CLASS_BASES: RefCell<Vec<Vec<PyTypeRef>>> = const { RefCell::new(Vec::new()) };
    }

    struct PendingClassBases;

    impl PendingClassBases {
        fn enter(bases: Vec<PyTypeRef>) -> Self {
            PENDING_CLASS_BASES.with(|stack| stack.borrow_mut().push(bases));
            Self
        }
    }

    impl Drop for PendingClassBases {
        fn drop(&mut self) {
            PENDING_CLASS_BASES.with(|stack| {
                stack.borrow_mut().pop();
            });
        }
    }

    fn restore_mixin_bases(explicit: Vec<PyTypeRef>) -> Vec<PyTypeRef> {
        PENDING_CLASS_BASES.with(|stack| {
            let stack = stack.borrow();
            for pending in stack.iter().rev() {
                let covers = explicit
                    .iter()
                    .all(|base| pending.iter().any(|kept| kept.is(base)));
                if covers && pending.len() > explicit.len() {
                    return pending.clone();
                }
            }
            explicit
        })
    }

    /// A class statement, or a `metaclass(name, bases, namespace)` call,
    /// creating a subclass of wrapped classes.
    fn create_class(
        py: Python<'_>,
        metaclass: Option<Bound<'_, PyAny>>,
        args: FuncArgs,
        vm: &VirtualMachine,
        initialize: bool,
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
        // Held across the C call: `__new__` allocates again through `type.__new__`.
        let _pending_bases = PendingClassBases::enter(bases.clone());
        let (metaclass, c_args) =
            create_cpython_class(py, metaclass, name, bases.clone(), &namespace, vm)?;
        let cpython = call_new_class(py, metaclass, c_args, &args.kwargs, vm, initialize)?;
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

    fn is_layout_descriptor(value: &PyObject) -> bool {
        matches!(
            &*value.class().slot_name(),
            "getset_descriptor" | "member_descriptor" | "wrapper_descriptor"
        )
    }

    /// Run the metaclass initializer on the class allocated by `slot_new`.
    ///
    /// `type(name, bases, namespace)` allocates on this side first. The
    /// matching class is created uninitialized, then this initializer runs
    /// once. A second run raises `already initialized`.
    fn metaclass_init(obj: &PyObject, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        let Some(ty) = obj.downcast_ref::<PyType>() else {
            return Ok(());
        };
        let needs_class = wrapped_cpython_type(ty, vm).is_none()
            && (is_cpython_backed(ty.as_object(), vm)
                || is_cpython_backed(ty.class().as_object(), vm));
        if needs_class {
            with_gil(vm, |py| {
                cpython_subclass(py, ty, vm)?;
                Ok(())
            })?;
        }
        let Some(wrapped) = wrapped_cpython_type(ty, vm) else {
            return Ok(());
        };
        with_gil(vm, |py| {
            let c_type = wrapped.py_obj.bind(py);
            let init = run_cpython(vm, || c_type.get_type().getattr("__init__"))?;
            // The namespace is the third argument. `_type_` in it is read by
            // the metaclass initializer, so it has to be converted the same
            // way as when the class dict was filled.
            let converted = args
                .args
                .iter()
                .enumerate()
                .map(|(index, arg)| {
                    if index == 2 {
                        convert_namespace_arg(py, arg, vm)
                    } else {
                        to_cpython(py, arg, vm)
                    }
                })
                .collect::<PyResult<Vec<_>>>()?;
            let mut positional = Vec::with_capacity(converted.len() + 1);
            positional.push(c_type.clone().into_any());
            positional.extend(converted);
            call_cpython_with(&init, positional, &args.kwargs, vm)?;
            Ok(())
        })
    }
}
