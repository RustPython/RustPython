//! The `pyo3` module: running code in an embedded CPython through PyO3.
//!
//! RustPython does not implement every C extension module. This module loads
//! CPython into the process and lets RustPython code use CPython modules
//! directly:
//!
//! ```python
//! import pyo3
//! np = pyo3.import_module("numpy")
//! a = np.array([1, 2, 3])
//! print(a * 2, a.sum())
//! ```
//!
//! CPython objects appear in RustPython as instances of the wrapper of their
//! class. `pyo3.ref` carries the proxy payload and forwards attribute access;
//! slots come from the methods the class actually defines. Classes that
//! cannot be laid out that way stay generic proxies, and `proxy.__class__`
//! is still the wrapped class. A wrapper's metaclass is the wrapper of the
//! CPython metaclass. When that metaclass is `type`, the class is itself a
//! metaclass, and the class is immutable, the wrapper's metaclass is `type`
//! too and the immutable-type flag is copied from the wrapped class. A
//! mutable metaclass keeps `pyo3.pyo3_type`, so reads and writes on the
//! wrapper reach the wrapped class. Every other class whose metaclass is
//! `type` uses `pyo3.pyo3_type`.
//! See [`convert`] for what crosses by value and what by reference,
//! [`pyo3_type`] for classes, and [`errors`] for exceptions.
//!
//! `copy.copy` and `copy.deepcopy` of an embedded object call the embedded
//! `copy` module. `pickle` stores `pyo3.load_embedded` and the bytes the
//! embedded `pickle` produced; loading unpickles there and converts the
//! result back. See [`pyo3_ref`].
//!
//! A RustPython object handed across publishes the non-slot dunder methods
//! its class defines. The first definition in the MRO wins, and a name only
//! `object` defines stays unpublished. Its sequence length is its `__len__`.
//! See [`rustpython_object`]. Embedded `warnings.showwarning` is delivered to
//! this interpreter while a thread is running it. See [`cpython_init`].
//!
//! `pyo3.wraps` instead runs a whole function in CPython:
//!
//! ```python
//! @pyo3.wraps
//! def max_prec():
//!     import _decimal
//!     return _decimal.MAX_PREC
//! ```
//!
//! [`borrow_module`] makes a CPython module importable under its own name,
//! e.g. to use the embedded `_ctypes` in place of this interpreter's.

#![allow(clippy::module_inception)]

extern crate alloc;
#[macro_use]
extern crate rustpython_derive;

mod convert;
mod cpython_init;
mod errors;
mod gil;
mod pyo3_ref;
mod pyo3_type;
mod rustpython_object;

use rustpython_vm::{Context, PyResult, VirtualMachine, builtins::PyModuleDef};

/// Definition of the `pyo3` module, for registering it in an interpreter.
pub fn module_def(ctx: &Context) -> &'static PyModuleDef {
    _pyo3::module_def(ctx)
}

/// Import the CPython module `name` and register it in `sys.modules`, so
/// that RustPython code importing `name` gets the embedded implementation.
/// An existing entry is replaced.
pub fn borrow_module(name: &str, vm: &VirtualMachine) -> PyResult<()> {
    vm.import("pyo3", 0)?;
    let module = _pyo3::import_module_impl(name, vm)?;
    vm.sys_module
        .get_attr("modules", vm)?
        .set_item(name, module, vm)
}

#[pymodule(name = "pyo3", with(pyo3_ref::pyo3_ref, pyo3_type::pyo3_type))]
mod _pyo3 {
    use std::collections::HashMap;

    use pyo3::types::{PyDict, PyList, PyType as CPythonType};
    use pyo3::{
        Bound, PyAny, Python,
        types::{PyAnyMethods, PyDictMethods},
    };
    use rustpython_vm::{
        AsObject, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyModule, PyStrRef, PyType, PyTypeRef, PyUtf8StrRef},
        common::lock::PyMutex,
        function::FuncArgs,
        types::{Callable, Constructor, Representable},
    };

    use crate::convert::{call_cpython, str_to_cpython, to_cpython, to_rustpython};
    use crate::errors::Pyo3ResultExt;
    use crate::gil::{run_cpython, with_gil};

    /// Per-interpreter state of the module.
    #[pyclass(no_attr, module = "pyo3", name = "_pyo3_state")]
    #[derive(Debug, Default, PyPayload)]
    pub(crate) struct Pyo3State {
        /// Classes of either interpreter standing for each other, by the
        /// address of the CPython class. An entry keeps the CPython class
        /// alive, so the address cannot be reused by another class.
        types: PyMutex<HashMap<usize, (pyo3::Py<pyo3::PyAny>, PyTypeRef)>>,
        /// Tuple subclass passed to `sys.unraisablehook`. One per interpreter.
        unraisable_args: PyMutex<Option<PyTypeRef>>,
        /// `mro` of metaclass wrappers whose own metaclass is `type`.
        /// One descriptor, shared; it binds the class it is read from.
        published_mro_method: PyMutex<Option<PyObjectRef>>,
    }

    #[pyclass(flags(DISALLOW_INSTANTIATION))]
    impl Pyo3State {}

    impl Pyo3State {
        pub(crate) fn cached_type(&self, ty: &Bound<'_, CPythonType>) -> Option<PyTypeRef> {
            let key = ty.as_ptr() as usize;
            self.types.lock().get(&key).map(|(_, ty)| ty.clone())
        }

        pub(crate) fn cache_type(&self, ty: &Bound<'_, CPythonType>, rustpython: PyTypeRef) {
            let key = ty.as_ptr() as usize;
            self.types
                .lock()
                .insert(key, (ty.clone().into_any().unbind(), rustpython));
        }

        /// The CPython class cached for the RustPython class `ty`.
        pub(crate) fn cpython_type_of<'py>(
            &self,
            py: Python<'py>,
            ty: &Py<PyType>,
        ) -> Option<Bound<'py, PyAny>> {
            self.types
                .lock()
                .values()
                .find(|(_, rustpython)| rustpython.is(ty))
                .map(|(cpython, _)| cpython.bind(py).clone())
        }

        pub(crate) fn cached_unraisable_args(&self) -> Option<PyTypeRef> {
            self.unraisable_args.lock().clone()
        }

        /// Remember `ty` unless another thread stored one first.
        pub(crate) fn store_unraisable_args(&self, ty: PyTypeRef) -> PyTypeRef {
            let mut slot = self.unraisable_args.lock();
            if let Some(existing) = slot.clone() {
                return existing;
            }
            *slot = Some(ty.clone());
            ty
        }

        pub(crate) fn cached_published_mro_method(&self) -> Option<PyObjectRef> {
            self.published_mro_method.lock().clone()
        }

        /// Remember `method` unless another thread stored one first.
        pub(crate) fn store_published_mro_method(&self, method: PyObjectRef) -> PyObjectRef {
            let mut slot = self.published_mro_method.lock();
            if let Some(existing) = slot.clone() {
                return existing;
            }
            *slot = Some(method.clone());
            method
        }
    }

    pub(crate) fn module_state(vm: &VirtualMachine) -> PyResult<PyRef<Pyo3State>> {
        let modules = vm.sys_module.get_attr("modules", vm)?;
        let module = match modules.get_item("pyo3", vm) {
            Ok(module) => module,
            Err(_) => vm.import("pyo3", 0)?,
        };
        module
            .get_attr("_state", vm)?
            .downcast::<Pyo3State>()
            .map_err(|_| vm.new_runtime_error("pyo3 module state is corrupted"))
    }

    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        __module_exec(vm, module);
        // Before any wrapped class is created: its `tp_new` must be `type.__new__`.
        crate::pyo3_type::install_class_creation(vm);
        module.set_attr("_state", Pyo3State::default().into_ref(&vm.ctx), vm)?;
        Ok(())
    }

    /// Import the CPython module `name`.
    #[pyfunction]
    fn import_module(name: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult {
        import_module_impl(name.as_str(), vm)
    }

    /// Unpickle `data` with the embedded interpreter's `pickle` and convert the result.
    ///
    /// Embedded objects reduce to this function plus the bytes that
    /// interpreter's `pickle.dumps` produced. Protocol 5 buffers are included
    /// in those bytes. The failure of an object that cannot be pickled is the
    /// embedded failure, converted.
    #[pyfunction]
    fn load_embedded(data: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        with_gil(vm, |py| {
            let pickle = run_cpython(vm, || py.import("pickle"))?;
            let data = to_cpython(py, &data, vm)?;
            let result = run_cpython(vm, || pickle.call_method1("loads", (data,)))?;
            to_rustpython(&result, vm)
        })
    }

    pub(crate) fn import_module_impl(name: &str, vm: &VirtualMachine) -> PyResult {
        with_gil(vm, |py| {
            let module = run_cpython(vm, || py.import(name))?;
            to_rustpython(module.as_any(), vm)
        })
    }

    /// `@pyo3.wraps`: run the decorated function in CPython.
    ///
    /// The function's source is compiled in CPython once, when decorating, as
    /// a function of its own module-less namespace: it sees the embedded
    /// interpreter's built-ins, not the globals of the module defining it.
    /// Arguments and the result cross as in any other call into CPython.
    #[pyattr]
    #[pyclass(module = "pyo3", name = "wraps", traverse)]
    #[derive(Debug, PyPayload)]
    struct Pyo3Wraps {
        func: PyObjectRef,
        #[pytraverse(skip)]
        cpython_func: pyo3::Py<pyo3::PyAny>,
    }

    impl Constructor for Pyo3Wraps {
        type Args = PyObjectRef;

        fn py_new(_cls: &Py<PyType>, func: PyObjectRef, vm: &VirtualMachine) -> PyResult<Self> {
            let source: PyStrRef = vm
                .import("inspect", 0)?
                .get_attr("getsource", vm)?
                .call((func.clone(),), vm)?
                .try_into_value(vm)?;
            let code = func.get_attr("__code__", vm)?;
            let filename: PyStrRef = code.get_attr("co_filename", vm)?.try_into_value(vm)?;
            let first_line: usize = code.get_attr("co_firstlineno", vm)?.try_into_value(vm)?;
            let module = func.get_attr("__module__", vm)?;

            let cpython_func = with_gil(vm, |py| {
                let module = to_cpython(py, &module, vm)?;
                define_in_cpython(py, &source, &filename, first_line, module).to_pyresult(vm)
            })?;
            Ok(Self { func, cpython_func })
        }
    }

    /// Define the function whose source is `source` in CPython, without its
    /// decorators, and return it.
    fn define_in_cpython(
        py: Python<'_>,
        source: &Py<rustpython_vm::builtins::PyStr>,
        filename: &Py<rustpython_vm::builtins::PyStr>,
        first_line: usize,
        module: Bound<'_, PyAny>,
    ) -> pyo3::PyResult<pyo3::Py<pyo3::PyAny>> {
        let ast = py.import("ast")?;
        let source = str_to_cpython(py, source.as_wtf8())?;
        let source = py.import("textwrap")?.call_method1("dedent", (source,))?;
        let tree = ast.call_method1("parse", (source,))?;
        let definition = tree.getattr("body")?.get_item(0)?;
        definition.setattr("decorator_list", PyList::empty(py))?;
        // `inspect.getsource` starts at the first decorator, as does
        // `co_firstlineno`, so tracebacks point into the original file.
        ast.call_method1("increment_lineno", (&tree, first_line.saturating_sub(1)))?;
        let filename = str_to_cpython(py, filename.as_wtf8())?;
        let builtins = py.import("builtins")?;
        let code = builtins
            .getattr("compile")?
            .call1((tree, filename, "exec"))?;
        let namespace = PyDict::new(py);
        namespace.set_item("__name__", module)?;
        builtins.getattr("exec")?.call1((code, &namespace))?;
        let name = definition.getattr("name")?;
        Ok(namespace.as_any().get_item(name)?.unbind())
    }

    impl Callable for Pyo3Wraps {
        type Args = FuncArgs;

        fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            with_gil(vm, |py| {
                let result = call_cpython(zelf.cpython_func.bind(py), args, vm)?;
                to_rustpython(&result, vm)
            })
        }
    }

    impl Representable for Pyo3Wraps {
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            Ok(format!("<pyo3.wraps {}>", zelf.func.repr(vm)?))
        }
    }

    #[pyclass(with(Constructor, Callable, Representable))]
    impl Pyo3Wraps {
        #[pygetset]
        fn __wrapped__(zelf: &Py<Self>) -> PyObjectRef {
            zelf.func.clone()
        }

        #[pygetset]
        fn __name__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            zelf.func.get_attr("__name__", vm)
        }

        #[pygetset]
        fn __qualname__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            zelf.func.get_attr("__qualname__", vm)
        }

        #[pygetset]
        fn __module__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            zelf.func.get_attr("__module__", vm)
        }

        #[pygetset]
        fn __doc__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            zelf.func.get_attr("__doc__", vm)
        }
    }
}
