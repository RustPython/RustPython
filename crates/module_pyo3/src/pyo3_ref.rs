//! `pyo3.ref`: CPython objects as seen from RustPython.
//!
//! A proxy forwards attribute access, calls, operators, item access,
//! iteration, hashing and the descriptor protocol to the CPython object it
//! wraps. `proxy.__class__` is the wrapped object's class, so `isinstance`
//! works against CPython classes, while `type(proxy)` stays the proxy class.
//!
//! As with `weakref.proxy`, callable objects get a subclass of their own,
//! `pyo3.callable_ref`, so `callable()` answers truthfully.

pub(crate) use pyo3_ref::{Pyo3Ref, binary_op};

#[pymodule(sub)]
pub(crate) mod pyo3_ref {
    use pyo3::exceptions::PyBufferError;
    use pyo3::pyclass::CompareOp;
    use pyo3::types::{PyIterator, PyTuple};
    use pyo3::{
        Bound, PyAny, PyErr, Python, ffi,
        types::{PyAnyMethods, PyTypeMethods},
    };
    use rustpython_vm::{
        AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        atomic_func,
        builtins::{PyStr, PyStrRef, PyType},
        common::{
            borrow::{BorrowedValue, BorrowedValueMut},
            hash::PyHash,
            lock::LazyLock,
        },
        function::{Either, FuncArgs, OptionalArg, PyComparisonValue, PySetterValue},
        protocol::{
            BufferDescriptor, BufferMethods, PyBuffer, PyIterReturn, PyMappingMethods,
            PyNumberMethods, PySequenceMethods,
        },
        types::{
            AsBuffer, AsMapping, AsNumber, AsSequence, Callable, Comparable, Constructor, GetAttr,
            GetDescriptor, Hashable, IterNext, Iterable, PyComparisonOp, Representable, SetAttr,
        },
    };

    use crate::convert::{call_cpython, str_to_cpython, to_cpython, to_rustpython};
    use crate::errors::Pyo3ResultExt;
    use crate::gil::{run_cpython, with_gil};
    use crate::pyo3_type::is_cpython_backed;

    /// Proxy for a CPython object.
    #[pyattr]
    #[pyclass(module = "pyo3", name = "ref")]
    #[derive(PyPayload)]
    pub(crate) struct Pyo3Ref {
        pub(crate) py_obj: pyo3::Py<pyo3::PyAny>,
    }

    /// Proxy for a callable CPython object.
    #[pyattr]
    #[pyclass(module = "pyo3", name = "callable_ref", base = Pyo3Ref)]
    pub(crate) struct Pyo3CallableRef {
        base: Pyo3Ref,
    }

    impl core::fmt::Debug for Pyo3Ref {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("Pyo3Ref").finish_non_exhaustive()
        }
    }

    impl core::fmt::Debug for Pyo3CallableRef {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("Pyo3CallableRef").finish_non_exhaustive()
        }
    }

    impl Pyo3Ref {
        pub(crate) const fn new(py_obj: pyo3::Py<pyo3::PyAny>) -> Self {
            Self { py_obj }
        }

        /// A proxy for `obj`, callable if `obj` is.
        pub(crate) fn wrap(obj: &Bound<'_, PyAny>, vm: &VirtualMachine) -> PyObjectRef {
            let base = Self::new(obj.clone().unbind());
            if obj.is_callable() {
                Pyo3CallableRef { base }.into_ref(&vm.ctx).into()
            } else {
                base.into_ref(&vm.ctx).into()
            }
        }

        /// Apply `f` to the wrapped object and bring the result back.
        fn apply(
            &self,
            vm: &VirtualMachine,
            f: impl for<'py> FnOnce(&Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>>,
        ) -> PyResult {
            with_gil(vm, |py| {
                let result = f(self.py_obj.bind(py))?;
                to_rustpython(&result, vm)
            })
        }

        /// Call the wrapped object's `name` method with converted `args`.
        fn call_method(&self, name: &str, args: &[&PyObject], vm: &VirtualMachine) -> PyResult {
            self.apply(vm, |obj| {
                let args = args
                    .iter()
                    .map(|arg| to_cpython(obj.py(), arg, vm))
                    .collect::<PyResult<Vec<_>>>()?;
                let args = PyTuple::new(obj.py(), args).to_pyresult(vm)?;
                run_cpython(vm, || obj.call_method1(name, args))
            })
        }

        fn call(&self, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            self.apply(vm, |obj| call_cpython(obj, args, vm))
        }
    }

    #[pyclass(
        with(
            Constructor,
            GetAttr,
            SetAttr,
            Representable,
            Comparable,
            Hashable,
            AsNumber,
            AsSequence,
            AsMapping,
            AsBuffer,
            Iterable,
            IterNext,
            GetDescriptor
        ),
        flags(BASETYPE)
    )]
    impl Pyo3Ref {
        // Special methods looked up on the type rather than through
        // `__getattribute__`; each forwards to the wrapped object's own.

        #[pymethod]
        fn __str__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            zelf.apply(vm, |obj| Ok(run_cpython(vm, || obj.str())?.into_any()))
        }

        #[pymethod]
        fn __format__(zelf: &Py<Self>, spec: PyStrRef, vm: &VirtualMachine) -> PyResult {
            zelf.call_method("__format__", &[spec.as_object()], vm)
        }

        #[pymethod]
        fn __dir__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            zelf.call_method("__dir__", &[], vm)
        }

        #[pymethod]
        fn __bytes__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            zelf.call_method("__bytes__", &[], vm)
        }

        #[pymethod]
        fn __reversed__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            zelf.call_method("__reversed__", &[], vm)
        }

        #[pymethod]
        fn __round__(zelf: &Py<Self>, ndigits: OptionalArg, vm: &VirtualMachine) -> PyResult {
            match ndigits {
                OptionalArg::Present(ndigits) => zelf.call_method("__round__", &[&ndigits], vm),
                OptionalArg::Missing => zelf.call_method("__round__", &[], vm),
            }
        }

        #[pymethod]
        fn __trunc__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            zelf.call_method("__trunc__", &[], vm)
        }

        #[pymethod]
        fn __floor__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            zelf.call_method("__floor__", &[], vm)
        }

        #[pymethod]
        fn __ceil__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            zelf.call_method("__ceil__", &[], vm)
        }

        #[pymethod]
        fn __fspath__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            zelf.call_method("__fspath__", &[], vm)
        }

        #[pymethod]
        fn __enter__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            zelf.call_method("__enter__", &[], vm)
        }

        #[pymethod]
        fn __exit__(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let args: Vec<&PyObject> = args.args.iter().map(|arg| &**arg).collect();
            zelf.call_method("__exit__", &args, vm)
        }
    }

    #[pyclass(with(Callable))]
    impl Pyo3CallableRef {}

    impl Constructor for Pyo3Ref {
        type Args = FuncArgs;

        fn py_new(cls: &Py<PyType>, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
            Err(vm.new_type_error(format!("cannot create '{}' instances", cls.slot_name())))
        }
    }

    impl Callable for Pyo3CallableRef {
        type Args = FuncArgs;

        fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            zelf.base.call(args, vm)
        }
    }

    impl GetAttr for Pyo3Ref {
        fn getattro(zelf: &Py<Self>, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
            zelf.apply(vm, |obj| {
                let name = str_to_cpython(obj.py(), name.as_wtf8()).to_pyresult(vm)?;
                run_cpython(vm, || obj.getattr(name))
            })
        }
    }

    impl SetAttr for Pyo3Ref {
        fn setattro(
            zelf: &Py<Self>,
            name: &Py<PyStr>,
            value: PySetterValue,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            with_gil(vm, |py| {
                let obj = zelf.py_obj.bind(py);
                let name = str_to_cpython(py, name.as_wtf8()).to_pyresult(vm)?;
                let value = match value {
                    PySetterValue::Assign(value) => Some(to_cpython(py, &value, vm)?),
                    PySetterValue::Delete => None,
                };
                run_cpython(vm, || match value {
                    Some(value) => obj.setattr(name, value),
                    None => obj.delattr(name),
                })
            })
        }
    }

    impl Representable for Pyo3Ref {
        fn repr(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyRef<PyStr>> {
            zelf.apply(vm, |obj| Ok(run_cpython(vm, || obj.repr())?.into_any()))?
                .downcast::<PyStr>()
                .map_err(|_| vm.new_type_error("__repr__ returned non-string"))
        }

        fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            unreachable!("repr() is implemented directly")
        }
    }

    impl Comparable for Pyo3Ref {
        fn slot_richcompare(
            zelf: &PyObject,
            other: &PyObject,
            op: PyComparisonOp,
            vm: &VirtualMachine,
        ) -> PyResult<Either<PyObjectRef, PyComparisonValue>> {
            let zelf = zelf
                .downcast_ref::<Self>()
                .ok_or_else(|| vm.new_type_error("expected a pyo3.ref"))?;
            let op = match op {
                PyComparisonOp::Lt => CompareOp::Lt,
                PyComparisonOp::Le => CompareOp::Le,
                PyComparisonOp::Eq => CompareOp::Eq,
                PyComparisonOp::Ne => CompareOp::Ne,
                PyComparisonOp::Gt => CompareOp::Gt,
                PyComparisonOp::Ge => CompareOp::Ge,
            };
            // Rich comparisons may return anything, e.g. a numpy array.
            zelf.apply(vm, |obj| {
                let other = to_cpython(obj.py(), other, vm)?;
                run_cpython(vm, || obj.rich_compare(other, op))
            })
            .map(Either::A)
        }

        fn cmp(
            _zelf: &Py<Self>,
            _other: &PyObject,
            _op: PyComparisonOp,
            _vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            unreachable!("slot_richcompare is implemented directly")
        }
    }

    impl Hashable for Pyo3Ref {
        fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
            with_gil(vm, |py| run_cpython(vm, || zelf.py_obj.bind(py).hash())).map(|h| h as PyHash)
        }
    }

    impl Iterable for Pyo3Ref {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            zelf.apply(vm, |obj| Ok(run_cpython(vm, || obj.try_iter())?.into_any()))
        }
    }

    impl IterNext for Pyo3Ref {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            with_gil(vm, |py| {
                let obj = zelf.py_obj.bind(py);
                let Ok(iterator) = obj.cast::<PyIterator>() else {
                    let name = obj
                        .get_type()
                        .name()
                        .map_or_else(|_| "?".to_owned(), |name| name.to_string());
                    return Err(vm.new_type_error(format!("'{name}' object is not an iterator")));
                };
                match run_cpython(vm, || iterator.clone().next().transpose())? {
                    Some(item) => Ok(PyIterReturn::Return(to_rustpython(&item, vm)?)),
                    None => Ok(PyIterReturn::StopIteration(None)),
                }
            })
        }
    }

    impl GetDescriptor for Pyo3Ref {
        fn descr_get(
            zelf: &PyObject,
            obj: Option<&PyObject>,
            cls: Option<&PyObject>,
            vm: &VirtualMachine,
        ) -> PyResult {
            let zelf_ref = Self::_as_pyref(zelf, vm)?;
            with_gil(vm, |py| {
                let descr = zelf_ref.py_obj.bind(py);
                // The descriptor protocol looks `__get__` up on the type.
                let Ok(get) = descr.get_type().getattr(pyo3::intern!(py, "__get__")) else {
                    return Ok(zelf.to_owned());
                };
                let instance = match obj {
                    Some(obj) if !vm.is_none(obj) => to_cpython(py, obj, vm)?,
                    _ => py.None().into_bound(py),
                };
                let owner = match cls {
                    Some(cls) => to_cpython(py, cls, vm)?,
                    None => py.None().into_bound(py),
                };
                let bound = run_cpython(vm, || get.call1((descr, instance, owner)))?;
                to_rustpython(&bound, vm)
            })
        }
    }

    impl AsBuffer for Pyo3Ref {
        fn as_buffer(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyBuffer> {
            let (export, desc) = with_gil(vm, |py| {
                run_cpython(vm, || CPythonBuffer::get(zelf.py_obj.bind(py)))
            })?;
            Ok(PyBuffer::new(
                export.into_ref(&vm.ctx).into(),
                desc,
                &CPYTHON_BUFFER_METHODS,
            ))
        }
    }

    /// The memory of a CPython buffer, exported to RustPython without a copy.
    ///
    /// The CPython buffer is held, and so its memory stays in place, until
    /// the last RustPython view of it is gone.
    #[pyclass(no_attr, module = "pyo3", name = "buffer")]
    #[derive(PyPayload)]
    struct CPythonBuffer {
        view: Box<ffi::Py_buffer>,
        /// Where the memory the elements span begins, relative to `buf`.
        start: isize,
        /// Length of the memory the elements span.
        len: usize,
    }

    // SAFETY: the view is plain data describing memory the exporter keeps in
    // place until the view is released, which takes the GIL.
    unsafe impl Send for CPythonBuffer {}
    unsafe impl Sync for CPythonBuffer {}

    #[pyclass]
    impl CPythonBuffer {}

    impl core::fmt::Debug for CPythonBuffer {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("CPythonBuffer")
                .field("start", &self.start)
                .field("len", &self.len)
                .finish_non_exhaustive()
        }
    }

    impl Drop for CPythonBuffer {
        fn drop(&mut self) {
            let view: *mut ffi::Py_buffer = &mut *self.view;
            let release = move || {
                Python::try_attach(|_| {
                    // SAFETY: the view was filled by `PyObject_GetBuffer` and
                    // is released once.
                    unsafe { ffi::PyBuffer_Release(view) }
                });
            };
            // Wait for the GIL detached from RustPython.
            rustpython_vm::vm::thread::try_with_current_vm(|vm| vm.allow_threads(release))
                .unwrap_or_else(release);
        }
    }

    impl CPythonBuffer {
        fn get(obj: &Bound<'_, PyAny>) -> pyo3::PyResult<(Self, BufferDescriptor)> {
            let py = obj.py();
            let mut view = Box::new(ffi::Py_buffer::new());
            // SAFETY: `view` is a fresh Py_buffer for the exporter to fill.
            if unsafe { ffi::PyObject_GetBuffer(obj.as_ptr(), &mut *view, ffi::PyBUF_FULL_RO) }
                == -1
            {
                return Err(PyErr::fetch(py));
            }
            let (start, len) = (0, 0);
            let mut export = Self { view, start, len };
            let view = &*export.view;
            if !view.suboffsets.is_null() {
                return Err(PyBufferError::new_err(
                    "buffers with suboffsets are not supported",
                ));
            }
            let ndim = view.ndim as usize;
            let itemsize = view.itemsize as usize;
            // SAFETY: the exporter provides `ndim` entries for each array it
            // does not leave NULL.
            let shape: Vec<usize> = if view.shape.is_null() {
                // Only a one-dimensional buffer may omit its shape.
                if ndim == 0 {
                    vec![]
                } else {
                    vec![view.len as usize / itemsize.max(1)]
                }
            } else {
                unsafe { core::slice::from_raw_parts(view.shape, ndim) }
                    .iter()
                    .map(|&n| n as usize)
                    .collect()
            };
            // A buffer without strides is C-contiguous.
            let strides: Vec<isize> = if view.strides.is_null() {
                let mut strides = vec![0; shape.len()];
                let mut stride = itemsize as isize;
                for (dim, &n) in strides.iter_mut().zip(&shape).rev() {
                    *dim = stride;
                    stride *= n as isize;
                }
                strides
            } else {
                unsafe { core::slice::from_raw_parts(view.strides, ndim) }.to_vec()
            };
            let dim_desc: Vec<_> = shape
                .iter()
                .zip(&strides)
                .map(|(&shape, &stride)| (shape, stride, 0))
                .collect();
            // The memory the elements span; strides may be negative.
            let (mut start, mut end) = (0, itemsize as isize);
            if view.len == 0 {
                end = 0;
            } else {
                for &(shape, stride, _) in &dim_desc {
                    let extent = stride * (shape as isize - 1);
                    if extent < 0 {
                        start += extent;
                    } else {
                        end += extent;
                    }
                }
            }
            let format = if view.format.is_null() {
                "B".into()
            } else {
                // SAFETY: a non-NULL format is a NUL-terminated string.
                unsafe { core::ffi::CStr::from_ptr(view.format) }
                    .to_string_lossy()
                    .into_owned()
                    .into()
            };
            let desc = BufferDescriptor::from_dim_desc(
                view.len as usize,
                -start,
                view.readonly != 0,
                itemsize,
                format,
                dim_desc,
            );
            export.start = start;
            export.len = (end - start) as usize;
            Ok((export, desc))
        }

        fn memory(&self) -> *mut u8 {
            self.view.buf.cast::<u8>().wrapping_offset(self.start)
        }
    }

    static CPYTHON_BUFFER_METHODS: BufferMethods = BufferMethods {
        obj_bytes: |buffer| {
            let export = buffer.obj_as::<CPythonBuffer>();
            if export.len == 0 {
                return BorrowedValue::Ref(&[]);
            }
            // SAFETY: the held CPython buffer keeps the memory valid.
            BorrowedValue::Ref(unsafe { core::slice::from_raw_parts(export.memory(), export.len) })
        },
        obj_bytes_mut: |buffer| {
            let export = buffer.obj_as::<CPythonBuffer>();
            if export.len == 0 {
                return BorrowedValueMut::RefMut(&mut []);
            }
            // SAFETY: the held CPython buffer keeps the memory valid, and it
            // is only written through when the exporter made it writable.
            BorrowedValueMut::RefMut(unsafe {
                core::slice::from_raw_parts_mut(export.memory(), export.len)
            })
        },
        release: |_| {},
        retain: |_| {},
    };

    fn cpython_function<'py>(
        py: Python<'py>,
        (module, name): (&str, &str),
    ) -> pyo3::PyResult<Bound<'py, PyAny>> {
        py.import(module)?.getattr(name)
    }

    /// `type(obj).name(obj, *args)`, or `None` when the type has no `name`,
    /// following how the interpreter itself looks special methods up.
    fn call_special<'py>(
        obj: &Bound<'py, PyAny>,
        name: &str,
        args: Vec<Bound<'py, PyAny>>,
    ) -> pyo3::PyResult<Option<Bound<'py, PyAny>>> {
        let Ok(method) = obj.get_type().getattr(name) else {
            return Ok(None);
        };
        let mut call_args = Vec::with_capacity(args.len() + 1);
        call_args.push(obj.clone());
        call_args.extend(args);
        let args = PyTuple::new(obj.py(), call_args)?;
        method.call1(args).map(Some)
    }

    /// A binary operator on two operands, at least one of them living in
    /// CPython.
    ///
    /// With both operands in CPython the operator runs there in full.
    /// Otherwise only the proxy's side is tried — `__op__` on the left,
    /// `__rop__` on the right — and `NotImplemented` hands the operation back
    /// to RustPython to try the other operand's side.
    pub(crate) fn binary_op(
        a: &PyObject,
        b: &PyObject,
        function: (&str, &str),
        method: &str,
        vm: &VirtualMachine,
    ) -> PyResult {
        let a_proxy = is_cpython_backed(a, vm);
        let b_proxy = is_cpython_backed(b, vm);
        if !a_proxy && !b_proxy {
            return Ok(vm.ctx.not_implemented());
        }
        with_gil(vm, |py| {
            let (a, b) = (to_cpython(py, a, vm)?, to_cpython(py, b, vm)?);
            let result = run_cpython(vm, || match (a_proxy, b_proxy) {
                (true, true) => cpython_function(py, function)?.call1((a, b)).map(Some),
                (true, _) => call_special(&a, &format!("__{method}__"), vec![b]),
                (false, _) => call_special(&b, &format!("__r{method}__"), vec![a]),
            })?;
            match result {
                Some(result) if !result.is(py.NotImplemented()) => to_rustpython(&result, vm),
                _ => Ok(vm.ctx.not_implemented()),
            }
        })
    }

    /// An augmented assignment. Falls back to the binary operator through
    /// `NotImplemented` when the proxy has no in-place method.
    fn inplace_op(a: &PyObject, b: &PyObject, method: &str, vm: &VirtualMachine) -> PyResult {
        with_gil(vm, |py| {
            let (a, b) = (to_cpython(py, a, vm)?, to_cpython(py, b, vm)?);
            match run_cpython(vm, || call_special(&a, &format!("__i{method}__"), vec![b]))? {
                Some(result) if !result.is(py.NotImplemented()) => to_rustpython(&result, vm),
                _ => Ok(vm.ctx.not_implemented()),
            }
        })
    }

    fn unary_op(obj: &PyObject, operator_name: &str, vm: &VirtualMachine) -> PyResult {
        proxy_of(obj, vm)?.apply(vm, |obj| {
            run_cpython(vm, || {
                cpython_function(obj.py(), ("operator", operator_name))?.call1((obj,))
            })
        })
    }

    fn conversion(obj: &PyObject, builtin: &str, vm: &VirtualMachine) -> PyResult {
        proxy_of(obj, vm)?.apply(vm, |obj| {
            run_cpython(vm, || {
                cpython_function(obj.py(), ("builtins", builtin))?.call1((obj,))
            })
        })
    }

    macro_rules! binary_slot {
        ($module:ident $function:literal, $method:literal) => {
            atomic_func!(|a, b, vm| {
                binary_op(a, b, (stringify!($module), $function), $method, vm)
            })
        };
    }

    macro_rules! inplace_slot {
        ($method:literal) => {
            atomic_func!(|a, b, vm| inplace_op(a, b, $method, vm))
        };
    }

    macro_rules! unary_slot {
        ($operator:literal) => {
            atomic_func!(|number, vm| unary_op(number.obj, $operator, vm))
        };
    }

    impl AsNumber for Pyo3Ref {
        fn as_number() -> &'static PyNumberMethods {
            static AS_NUMBER: LazyLock<PyNumberMethods> = LazyLock::new(|| PyNumberMethods {
                add: binary_slot!(operator "add", "add"),
                subtract: binary_slot!(operator "sub", "sub"),
                multiply: binary_slot!(operator "mul", "mul"),
                remainder: binary_slot!(operator "mod", "mod"),
                divmod: binary_slot!(builtins "divmod", "divmod"),
                power: atomic_func!(|a, b, modulus, vm| {
                    if vm.is_none(modulus) {
                        return binary_op(a, b, ("builtins", "pow"), "pow", vm);
                    }
                    with_gil(vm, |py| {
                        let args = (
                            to_cpython(py, a, vm)?,
                            to_cpython(py, b, vm)?,
                            to_cpython(py, modulus, vm)?,
                        );
                        let result = run_cpython(vm, || {
                            cpython_function(py, ("builtins", "pow"))?.call1(args)
                        })?;
                        to_rustpython(&result, vm)
                    })
                }),
                negative: unary_slot!("neg"),
                positive: unary_slot!("pos"),
                absolute: unary_slot!("abs"),
                boolean: atomic_func!(|number, vm| {
                    let zelf = proxy_of(number.obj, vm)?;
                    with_gil(vm, |py| {
                        run_cpython(vm, || zelf.py_obj.bind(py).is_truthy())
                    })
                }),
                invert: unary_slot!("invert"),
                lshift: binary_slot!(operator "lshift", "lshift"),
                rshift: binary_slot!(operator "rshift", "rshift"),
                and: binary_slot!(operator "and_", "and"),
                xor: binary_slot!(operator "xor", "xor"),
                or: binary_slot!(operator "or_", "or"),
                int: atomic_func!(|number, vm| conversion(number.obj, "int", vm)),
                float: atomic_func!(|number, vm| conversion(number.obj, "float", vm)),
                inplace_add: inplace_slot!("add"),
                inplace_subtract: inplace_slot!("sub"),
                inplace_multiply: inplace_slot!("mul"),
                inplace_remainder: inplace_slot!("mod"),
                inplace_power: atomic_func!(|a, b, _modulus, vm| inplace_op(a, b, "pow", vm)),
                inplace_lshift: inplace_slot!("lshift"),
                inplace_rshift: inplace_slot!("rshift"),
                inplace_and: inplace_slot!("and"),
                inplace_xor: inplace_slot!("xor"),
                inplace_or: inplace_slot!("or"),
                floor_divide: binary_slot!(operator "floordiv", "floordiv"),
                true_divide: binary_slot!(operator "truediv", "truediv"),
                inplace_floor_divide: inplace_slot!("floordiv"),
                inplace_true_divide: inplace_slot!("truediv"),
                index: unary_slot!("index"),
                matrix_multiply: binary_slot!(operator "matmul", "matmul"),
                inplace_matrix_multiply: inplace_slot!("matmul"),
            });
            &AS_NUMBER
        }
    }

    fn proxy_of<'a>(obj: &'a PyObject, vm: &VirtualMachine) -> PyResult<&'a Py<Pyo3Ref>> {
        obj.downcast_ref()
            .ok_or_else(|| vm.new_type_error("expected a pyo3.ref"))
    }

    fn len(zelf: &Py<Pyo3Ref>, vm: &VirtualMachine) -> PyResult<usize> {
        with_gil(vm, |py| run_cpython(vm, || zelf.py_obj.bind(py).len()))
    }

    fn set_item(
        zelf: &Py<Pyo3Ref>,
        key: &PyObject,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        with_gil(vm, |py| {
            let obj = zelf.py_obj.bind(py);
            let key = to_cpython(py, key, vm)?;
            let value = value.map(|value| to_cpython(py, &value, vm)).transpose()?;
            run_cpython(vm, || match value {
                Some(value) => obj.set_item(key, value),
                None => obj.del_item(key),
            })
        })
    }

    impl AsSequence for Pyo3Ref {
        fn as_sequence() -> &'static PySequenceMethods {
            static AS_SEQUENCE: LazyLock<PySequenceMethods> = LazyLock::new(|| PySequenceMethods {
                length: atomic_func!(|seq, vm| len(proxy_of(seq.obj, vm)?, vm)),
                item: atomic_func!(|seq, i, vm| {
                    proxy_of(seq.obj, vm)?.apply(vm, |obj| run_cpython(vm, || obj.get_item(i)))
                }),
                ass_item: atomic_func!(|seq, i, value, vm| {
                    set_item(
                        proxy_of(seq.obj, vm)?,
                        vm.ctx.new_int(i).as_object(),
                        value,
                        vm,
                    )
                }),
                contains: atomic_func!(|seq, target, vm| {
                    with_gil(vm, |py| {
                        let obj = proxy_of(seq.obj, vm)?.py_obj.bind(py);
                        let target = to_cpython(py, target, vm)?;
                        run_cpython(vm, || obj.contains(target))
                    })
                }),
                ..PySequenceMethods::NOT_IMPLEMENTED
            });
            &AS_SEQUENCE
        }
    }

    impl AsMapping for Pyo3Ref {
        fn as_mapping() -> &'static PyMappingMethods {
            static AS_MAPPING: LazyLock<PyMappingMethods> = LazyLock::new(|| PyMappingMethods {
                length: atomic_func!(|mapping, vm| len(proxy_of(mapping.obj, vm)?, vm)),
                subscript: atomic_func!(|mapping, key, vm| {
                    proxy_of(mapping.obj, vm)?.apply(vm, |obj| {
                        let key = to_cpython(obj.py(), key, vm)?;
                        run_cpython(vm, || obj.get_item(key))
                    })
                }),
                ass_subscript: atomic_func!(|mapping, key, value, vm| {
                    set_item(proxy_of(mapping.obj, vm)?, key, value, vm)
                }),
            });
            &AS_MAPPING
        }
    }
}
