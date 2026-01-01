//! RustPython to CPython bridge via PyO3
//!
//! This crate provides interoperability between RustPython and CPython,
//! allowing RustPython code to execute functions in the CPython runtime.
//!
//! # Background
//!
//! RustPython does not implement all CPython C extension modules.
//! This crate enables calling into the real CPython runtime for functionality
//! that is not yet available in RustPython.
//!
//! # Architecture
//!
//! Communication between RustPython and CPython uses PyO3 for in-process calls.
//! Data is serialized using Python's `pickle` protocol:
//!
//! ```text
//! RustPython                         CPython
//!     │                                  │
//!     │  pickle.dumps(args, kwargs)      │
//!     │ ──────────────────────────────►  │
//!     │                                  │  exec(source)
//!     │                                  │  result = func(*args, **kwargs)
//!     │  pickle.dumps(result)            │
//!     │ ◄──────────────────────────────  │
//!     │                                  │
//!     │  pickle.loads(result)            │
//! ```
//!
//! # Limitations
//!
//! - **File-based functions only**: Functions defined in REPL or via `exec()` will fail
//!   (`inspect.getsource()` requires source file access)
//! - **Picklable data only**: Cannot pass functions, classes, file handles, etc.
//! - **Performance overhead**: pickle serialization + CPython GIL acquisition
//! - **CPython required**: System must have CPython installed (linked via PyO3)

#[macro_use]
extern crate rustpython_derive;

use rustpython_vm::{PyRef, VirtualMachine, builtins::PyModule};

/// Create the _cpython module
pub fn make_module(vm: &VirtualMachine) -> PyRef<PyModule> {
    _cpython::make_module(vm)
}

#[pymodule]
mod _cpython {
    use crossbeam_utils::atomic::AtomicCell;
    use pyo3::PyErr;
    use pyo3::prelude::PyAnyMethods;
    use pyo3::types::PyBytes as Pyo3Bytes;
    use pyo3::types::PyBytesMethods;
    use pyo3::types::PyDictMethods;
    use rustpython_vm::{
        Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyBytes as RustPyBytes, PyBytesRef, PyDict, PyStr, PyStrRef, PyTypeRef},
        function::{FuncArgs, PyArithmeticValue, PyComparisonValue},
        protocol::{PyMappingMethods, PyNumberMethods, PySequenceMethods},
        types::{
            AsMapping, AsNumber, AsSequence, Callable, Comparable, Constructor, GetAttr, Iterable,
            PyComparisonOp, Representable,
        },
    };

    /// Wrapper class for executing functions in CPython.
    /// Used as a decorator: @cpython.wraps
    #[pyattr]
    #[pyclass(name = "wraps")]
    #[derive(Debug, PyPayload)]
    struct CPythonWraps {
        source: String,
        func_name: String,
    }

    impl Constructor for CPythonWraps {
        type Args = PyObjectRef;

        fn py_new(cls: PyTypeRef, func: Self::Args, vm: &VirtualMachine) -> PyResult {
            // Get function name
            let func_name = func
                .get_attr("__name__", vm)?
                .downcast::<rustpython_vm::builtins::PyStr>()
                .map_err(|_| vm.new_type_error("function must have __name__".to_owned()))?
                .as_str()
                .to_owned();

            // Get source using inspect.getsource(func)
            let inspect = vm.import("inspect", 0)?;
            let getsource = inspect.get_attr("getsource", vm)?;
            let source_obj = getsource.call((func.clone(),), vm)?;
            let source_full = source_obj
                .downcast::<rustpython_vm::builtins::PyStr>()
                .map_err(|_| vm.new_type_error("getsource did not return str".to_owned()))?
                .as_str()
                .to_owned();

            // Strip decorator lines from source (lines starting with @)
            // Find the first line that starts with 'def ' or 'async def '
            let source = strip_decorators(&source_full);

            Self { source, func_name }
                .into_ref_with_type(vm, cls)
                .map(Into::into)
        }
    }

    /// Serialize a RustPython object to pickle bytes.
    fn rustpython_pickle_dumps(
        obj: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<RustPyBytes>> {
        let pickle = vm.import("pickle", 0)?;
        let dumps = pickle.get_attr("dumps", vm)?;
        dumps
            .call((obj,), vm)?
            .downcast::<RustPyBytes>()
            .map_err(|_| vm.new_type_error("pickle.dumps did not return bytes".to_owned()))
    }

    /// Deserialize pickle bytes to a RustPython object.
    fn rustpython_pickle_loads(bytes: &[u8], vm: &VirtualMachine) -> PyResult {
        let pickle = vm.import("pickle", 0)?;
        let loads = pickle.get_attr("loads", vm)?;
        let bytes_obj = RustPyBytes::from(bytes.to_vec()).into_ref(&vm.ctx);
        loads.call((bytes_obj,), vm)
    }

    /// Strip decorator lines from function source code and dedent.
    /// Returns source starting from 'def' or 'async def', with common indentation removed.
    fn strip_decorators(source: &str) -> String {
        let lines: Vec<&str> = source.lines().collect();
        let mut result_lines = Vec::new();
        let mut found_def = false;
        let mut base_indent = 0;

        for line in &lines {
            let trimmed = line.trim_start();
            if !found_def {
                if trimmed.starts_with("def ") || trimmed.starts_with("async def ") {
                    found_def = true;
                    // Calculate base indentation from the def line
                    base_indent = line.len() - trimmed.len();
                    result_lines.push(*line);
                }
                // Skip decorator lines (starting with @) and blank lines before def
            } else {
                result_lines.push(*line);
            }
        }

        // Dedent all lines by base_indent
        result_lines
            .iter()
            .map(|line| {
                if line.len() >= base_indent
                    && line
                        .chars()
                        .take(base_indent)
                        .all(|c| c == ' ' || c == '\t')
                {
                    &line[base_indent..]
                } else if line.trim().is_empty() {
                    ""
                } else {
                    *line
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    impl Callable for CPythonWraps {
        type Args = FuncArgs;

        fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            // Pickle args and kwargs
            let args_tuple = vm.ctx.new_tuple(args.args);
            let kwargs_dict = PyDict::default().into_ref(&vm.ctx);
            for (key, value) in args.kwargs {
                kwargs_dict.set_item(&key, value, vm)?;
            }

            let pickled_args_bytes = rustpython_pickle_dumps(args_tuple.into(), vm)?;
            let pickled_kwargs_bytes = rustpython_pickle_dumps(kwargs_dict.into(), vm)?;

            // Call execute_impl()
            let result_bytes = execute_impl(
                &zelf.source,
                &zelf.func_name,
                pickled_args_bytes.as_bytes(),
                pickled_kwargs_bytes.as_bytes(),
                vm,
            )?;

            // Unpickle result
            rustpython_pickle_loads(&result_bytes, vm)
        }
    }

    impl Representable for CPythonWraps {
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok(format!("<_cpython.wraps wrapper for '{}'>", zelf.func_name))
        }
    }

    #[pyclass(with(Constructor, Callable, Representable))]
    impl CPythonWraps {}

    /// Internal implementation for executing Python code in CPython.
    fn execute_impl(
        source: &str,
        func_name: &str,
        args_bytes: &[u8],
        kwargs_bytes: &[u8],
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        // Build the CPython code to execute
        let pyo3_code = format!(
            r#"
import pickle as __pickle

# Unpickle arguments
__args__ = __pickle.loads(__pickled_args__)
__kwargs__ = __pickle.loads(__pickled_kwargs__)
# Execute the source code (defines the function)
{source}

# Call the function and pickle the result
__result__ = {func_name}(*__args__, **__kwargs__)
__pickled_result__ = __pickle.dumps(__result__, protocol=4)
"#,
            source = source,
            func_name = func_name,
        );

        // Execute in CPython via PyO3
        pyo3::Python::attach(|py| -> Result<Vec<u8>, PyErr> {
            // Create Python bytes for pickled data
            let py_args = Pyo3Bytes::new(py, args_bytes);
            let py_kwargs = Pyo3Bytes::new(py, kwargs_bytes);

            // Create globals dict with pickled args
            let globals = pyo3::types::PyDict::new(py);
            globals.set_item("__pickled_args__", &py_args)?;
            globals.set_item("__pickled_kwargs__", &py_kwargs)?;

            // Execute using compile + exec pattern
            let builtins = py.import("builtins")?;
            let compile = builtins.getattr("compile")?;
            let exec_fn = builtins.getattr("exec")?;

            // Compile the code
            let code = compile.call1((&pyo3_code, "<pyo3_bridge>", "exec"))?;

            // Execute with globals
            exec_fn.call1((code, &globals))?;

            // Get the pickled result
            let result = globals.get_item("__pickled_result__")?;
            let result = result.ok_or_else(|| {
                PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("No result returned")
            })?;
            let result_bytes: &pyo3::Bound<'_, Pyo3Bytes> = result.downcast()?;
            Ok(result_bytes.as_bytes().to_vec())
        })
        .map_err(|e| vm.new_runtime_error(format!("CPython exec error: {}", e)))
    }

    /// Execute a Python function in CPython runtime.
    ///
    /// # Arguments
    /// * `source` - The complete source code of the function
    /// * `func_name` - The name of the function to call
    /// * `pickled_args` - Pickled positional arguments (bytes)
    /// * `pickled_kwargs` - Pickled keyword arguments (bytes)
    ///
    /// # Returns
    /// Pickled result from CPython (bytes)
    #[pyfunction]
    fn execute(
        source: PyStrRef,
        func_name: PyStrRef,
        pickled_args: PyBytesRef,
        pickled_kwargs: PyBytesRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyBytesRef> {
        let result_bytes = execute_impl(
            source.as_str(),
            func_name.as_str(),
            pickled_args.as_bytes(),
            pickled_kwargs.as_bytes(),
            vm,
        )?;
        Ok(RustPyBytes::from(result_bytes).into_ref(&vm.ctx))
    }

    /// Execute arbitrary Python code in CPython and return pickled result.
    ///
    /// # Arguments
    /// * `code` - Python code to execute (should assign result to `__result__`)
    ///
    /// # Returns
    /// Pickled result from CPython (bytes)
    #[pyfunction]
    fn eval_code(code: PyStrRef, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
        let code_str = code.as_str();

        let wrapper_code = format!(
            r#"
import pickle
{code}
__pickled_result__ = pickle.dumps(__result__, protocol=4)
"#,
            code = code_str,
        );

        let result_bytes = pyo3::Python::attach(|py| -> Result<Vec<u8>, PyErr> {
            let globals = pyo3::types::PyDict::new(py);

            let builtins = py.import("builtins")?;
            let compile = builtins.getattr("compile")?;
            let exec_fn = builtins.getattr("exec")?;

            let code = compile.call1((&wrapper_code, "<pyo3_bridge>", "exec"))?;
            exec_fn.call1((code, &globals))?;

            let result = globals.get_item("__pickled_result__")?;
            let result = result.ok_or_else(|| {
                PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("No __result__ defined in code")
            })?;
            let result_bytes: &pyo3::Bound<'_, Pyo3Bytes> = result.downcast()?;
            Ok(result_bytes.as_bytes().to_vec())
        })
        .map_err(|e| vm.new_runtime_error(format!("CPython eval error: {}", e)))?;

        Ok(RustPyBytes::from(result_bytes).into_ref(&vm.ctx))
    }

    /// Pickle a CPython object to bytes.
    fn pickle_in_cpython(
        py: pyo3::Python<'_>,
        obj: &pyo3::Bound<'_, pyo3::PyAny>,
    ) -> Result<Vec<u8>, PyErr> {
        let pickle = py.import("pickle")?;
        let pickled = pickle.call_method1("dumps", (obj, 4i32))?;
        let bytes: &pyo3::Bound<'_, Pyo3Bytes> = pickled.downcast()?;
        Ok(bytes.as_bytes().to_vec())
    }

    /// Unpickle bytes in CPython.
    fn unpickle_in_cpython<'py>(
        py: pyo3::Python<'py>,
        bytes: &[u8],
    ) -> Result<pyo3::Bound<'py, pyo3::PyAny>, PyErr> {
        let pickle = py.import("pickle")?;
        pickle.call_method1("loads", (Pyo3Bytes::new(py, bytes),))
    }

    /// Create a Pyo3Ref from a pyo3 object, attempting to pickle it.
    fn create_pyo3_object(py: pyo3::Python<'_>, obj: &pyo3::Bound<'_, pyo3::PyAny>) -> Pyo3Ref {
        let pickled = pickle_in_cpython(py, obj).ok();
        Pyo3Ref {
            py_obj: obj.clone().unbind(),
            pickled,
        }
    }

    /// Convert a Pyo3Ref to RustPython object.
    /// If pickled bytes exist, tries to unpickle to native RustPython object.
    /// Falls back to returning the Pyo3Ref wrapper.
    fn pyo3_to_rustpython(pyo3_obj: Pyo3Ref, vm: &VirtualMachine) -> PyResult {
        if let Some(ref bytes) = pyo3_obj.pickled {
            if let Ok(unpickled) = rustpython_pickle_loads(bytes, vm) {
                return Ok(unpickled);
            }
            // Unpickle failed (e.g., numpy arrays need numpy module)
            // Fall through to return Pyo3Ref wrapper
        }
        Ok(pyo3_obj.into_ref(&vm.ctx).into())
    }

    /// Get attribute from a CPython object
    fn pyo3_getattr_impl(
        py_obj: &pyo3::Py<pyo3::PyAny>,
        name: &str,
        vm: &VirtualMachine,
    ) -> PyResult {
        let pyo3_obj = pyo3::Python::attach(|py| -> Result<Pyo3Ref, PyErr> {
            let obj = py_obj.bind(py);
            let attr = obj.getattr(name)?;
            Ok(create_pyo3_object(py, &attr))
        })
        .map_err(|e| vm.new_attribute_error(format!("CPython getattr error: {}", e)))?;

        pyo3_to_rustpython(pyo3_obj, vm)
    }

    /// Call a CPython object
    fn pyo3_call_impl(
        py_obj: &pyo3::Py<pyo3::PyAny>,
        args: FuncArgs,
        vm: &VirtualMachine,
    ) -> PyResult {
        // Pickle args and kwargs in RustPython
        let args_tuple = vm.ctx.new_tuple(args.args);
        let kwargs_dict = PyDict::default().into_ref(&vm.ctx);
        for (key, value) in args.kwargs {
            kwargs_dict.set_item(&key, value, vm)?;
        }

        let args_bytes = rustpython_pickle_dumps(args_tuple.into(), vm)?;
        let kwargs_bytes = rustpython_pickle_dumps(kwargs_dict.into(), vm)?;

        let pyo3_obj = pyo3::Python::attach(|py| -> Result<Pyo3Ref, PyErr> {
            let obj = py_obj.bind(py);

            // Unpickle args/kwargs in CPython
            let args_py = unpickle_in_cpython(py, args_bytes.as_bytes())?;
            let kwargs_py = unpickle_in_cpython(py, kwargs_bytes.as_bytes())?;

            // Call the object
            let call_result = obj.call(args_py.downcast()?, Some(kwargs_py.downcast()?))?;

            Ok(create_pyo3_object(py, &call_result))
        })
        .map_err(|e| vm.new_runtime_error(format!("CPython call error: {}", e)))?;

        pyo3_to_rustpython(pyo3_obj, vm)
    }

    /// Represents an object to be passed into CPython.
    /// Either already a CPython object (Native) or pickled RustPython object (Pickled).
    enum ToPyo3Ref<'a> {
        Native(&'a pyo3::Py<pyo3::PyAny>),
        Pickled(PyRef<RustPyBytes>),
    }

    impl ToPyo3Ref<'_> {
        fn to_pyo3<'py>(
            &self,
            py: pyo3::Python<'py>,
        ) -> Result<pyo3::Bound<'py, pyo3::PyAny>, PyErr> {
            match self {
                ToPyo3Ref::Native(obj) => Ok(obj.bind(py).clone()),
                ToPyo3Ref::Pickled(bytes) => unpickle_in_cpython(py, bytes.as_bytes()),
            }
        }
    }

    /// Convert a RustPython object to ToPyo3Ref for passing into CPython
    fn to_pyo3_object<'a>(obj: &'a PyObject, vm: &VirtualMachine) -> PyResult<ToPyo3Ref<'a>> {
        if let Some(pyo3_obj) = obj.downcast_ref::<Pyo3Ref>() {
            Ok(ToPyo3Ref::Native(&pyo3_obj.py_obj))
        } else {
            let pickled = rustpython_pickle_dumps(obj.to_owned(), vm)?;
            Ok(ToPyo3Ref::Pickled(pickled))
        }
    }

    /// Execute binary operation on CPython objects
    fn pyo3_binary_op(a: &PyObject, b: &PyObject, op: &str, vm: &VirtualMachine) -> PyResult {
        // If neither is Pyo3Ref, return NotImplemented
        if a.downcast_ref::<Pyo3Ref>().is_none() && b.downcast_ref::<Pyo3Ref>().is_none() {
            return Ok(vm.ctx.not_implemented());
        }

        let a_obj = to_pyo3_object(a, vm)?;
        let b_obj = to_pyo3_object(b, vm)?;

        let result = pyo3::Python::attach(|py| -> Result<PyArithmeticValue<Pyo3Ref>, PyErr> {
            let a_py = a_obj.to_pyo3(py)?;
            let b_py = b_obj.to_pyo3(py)?;

            let result_obj = a_py.call_method1(op, (&b_py,))?;

            if result_obj.is(&py.NotImplemented()) {
                return Ok(PyArithmeticValue::NotImplemented);
            }

            Ok(PyArithmeticValue::Implemented(create_pyo3_object(
                py,
                &result_obj,
            )))
        })
        .map_err(|e| vm.new_runtime_error(format!("CPython binary op error: {}", e)))?;

        match result {
            PyArithmeticValue::NotImplemented => Ok(vm.ctx.not_implemented()),
            PyArithmeticValue::Implemented(pyo3_obj) => pyo3_to_rustpython(pyo3_obj, vm),
        }
    }

    /// Wrapper for CPython objects
    #[pyattr]
    #[pyclass(name = "ref")]
    #[derive(PyPayload)]
    struct Pyo3Ref {
        py_obj: pyo3::Py<pyo3::PyAny>,
        /// Pickled bytes for potential unpickling to native RustPython object
        pickled: Option<Vec<u8>>,
    }

    impl std::fmt::Debug for Pyo3Ref {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Pyo3Ref")
                .field("py_obj", &"<CPython object>")
                .finish()
        }
    }

    impl GetAttr for Pyo3Ref {
        fn getattro(zelf: &Py<Self>, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
            pyo3_getattr_impl(&zelf.py_obj, name.as_str(), vm)
        }
    }

    impl Callable for Pyo3Ref {
        type Args = FuncArgs;

        fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            pyo3_call_impl(&zelf.py_obj, args, vm)
        }
    }

    impl Representable for Pyo3Ref {
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            // Get repr from CPython directly
            let result = pyo3::Python::attach(|py| -> Result<String, PyErr> {
                let obj = zelf.py_obj.bind(py);
                let builtins = py.import("builtins")?;
                let repr_fn = builtins.getattr("repr")?;
                let repr_result = repr_fn.call1((obj,))?;
                repr_result.extract()
            })
            .map_err(|e| vm.new_runtime_error(format!("CPython repr error: {}", e)))?;
            Ok(result)
        }
    }

    impl AsNumber for Pyo3Ref {
        fn as_number() -> &'static PyNumberMethods {
            static AS_NUMBER: PyNumberMethods = PyNumberMethods {
                add: Some(|a, b, vm| pyo3_binary_op(a, b, "__add__", vm)),
                subtract: Some(|a, b, vm| pyo3_binary_op(a, b, "__sub__", vm)),
                multiply: Some(|a, b, vm| pyo3_binary_op(a, b, "__mul__", vm)),
                remainder: Some(|a, b, vm| pyo3_binary_op(a, b, "__mod__", vm)),
                divmod: Some(|a, b, vm| pyo3_binary_op(a, b, "__divmod__", vm)),
                floor_divide: Some(|a, b, vm| pyo3_binary_op(a, b, "__floordiv__", vm)),
                true_divide: Some(|a, b, vm| pyo3_binary_op(a, b, "__truediv__", vm)),
                ..PyNumberMethods::NOT_IMPLEMENTED
            };
            &AS_NUMBER
        }
    }

    impl Comparable for Pyo3Ref {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            let method_name = match op {
                PyComparisonOp::Lt => "__lt__",
                PyComparisonOp::Le => "__le__",
                PyComparisonOp::Eq => "__eq__",
                PyComparisonOp::Ne => "__ne__",
                PyComparisonOp::Gt => "__gt__",
                PyComparisonOp::Ge => "__ge__",
            };

            let other_obj = to_pyo3_object(other, vm)?;

            let result = pyo3::Python::attach(|py| -> Result<PyComparisonValue, PyErr> {
                let obj = zelf.py_obj.bind(py);
                let other_py = other_obj.to_pyo3(py)?;

                let result = obj.call_method1(method_name, (&other_py,))?;

                if result.is(&py.NotImplemented()) {
                    return Ok(PyComparisonValue::NotImplemented);
                }

                // Try to extract bool; if it fails, return NotImplemented
                match result.extract::<bool>() {
                    Ok(bool_val) => Ok(PyComparisonValue::Implemented(bool_val)),
                    Err(_) => Ok(PyComparisonValue::NotImplemented),
                }
            })
            .map_err(|e| vm.new_runtime_error(format!("CPython comparison error: {}", e)))?;

            Ok(result)
        }
    }

    /// Helper to get len from CPython object
    fn pyo3_len(py_obj: &pyo3::Py<pyo3::PyAny>, vm: &VirtualMachine) -> PyResult<usize> {
        pyo3::Python::attach(|py| -> Result<usize, PyErr> {
            let obj = py_obj.bind(py);
            let builtins = py.import("builtins")?;
            let len_fn = builtins.getattr("len")?;
            let len_result = len_fn.call1((obj,))?;
            len_result.extract()
        })
        .map_err(|e| vm.new_runtime_error(format!("CPython len error: {}", e)))
    }

    /// Helper to get item by index from CPython object
    fn pyo3_getitem_by_index(
        py_obj: &pyo3::Py<pyo3::PyAny>,
        index: isize,
        vm: &VirtualMachine,
    ) -> PyResult {
        let pyo3_obj = pyo3::Python::attach(|py| -> Result<Pyo3Ref, PyErr> {
            let obj = py_obj.bind(py);
            let item = obj.get_item(index)?;
            Ok(create_pyo3_object(py, &item))
        })
        .map_err(|e| vm.new_index_error(format!("CPython getitem error: {}", e)))?;

        pyo3_to_rustpython(pyo3_obj, vm)
    }

    /// Helper to get item by key from CPython object
    fn pyo3_getitem(
        py_obj: &pyo3::Py<pyo3::PyAny>,
        key: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult {
        let key_obj = to_pyo3_object(key, vm)?;

        let pyo3_obj = pyo3::Python::attach(|py| -> Result<Pyo3Ref, PyErr> {
            let obj = py_obj.bind(py);
            let key_py = key_obj.to_pyo3(py)?;
            let item = obj.get_item(&key_py)?;
            Ok(create_pyo3_object(py, &item))
        })
        .map_err(|e| {
            vm.new_key_error(
                vm.ctx
                    .new_str(format!("CPython getitem error: {}", e))
                    .into(),
            )
        })?;

        pyo3_to_rustpython(pyo3_obj, vm)
    }

    /// Helper to set item in CPython object
    fn pyo3_setitem(
        py_obj: &pyo3::Py<pyo3::PyAny>,
        key: &PyObject,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let key_obj = to_pyo3_object(key, vm)?;
        let value_obj = value.as_ref().map(|v| to_pyo3_object(v, vm)).transpose()?;

        pyo3::Python::attach(|py| -> Result<(), PyErr> {
            let obj = py_obj.bind(py);
            let key_py = key_obj.to_pyo3(py)?;

            match value_obj {
                Some(ref val_obj) => {
                    let val_py = val_obj.to_pyo3(py)?;
                    obj.set_item(&key_py, &val_py)?;
                }
                None => {
                    obj.del_item(&key_py)?;
                }
            }
            Ok(())
        })
        .map_err(|e| vm.new_runtime_error(format!("CPython setitem error: {}", e)))
    }

    /// Helper to check if item is in CPython object
    fn pyo3_contains(
        py_obj: &pyo3::Py<pyo3::PyAny>,
        target: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        let target_obj = to_pyo3_object(target, vm)?;

        pyo3::Python::attach(|py| -> Result<bool, PyErr> {
            let obj = py_obj.bind(py);
            let target_py = target_obj.to_pyo3(py)?;
            obj.contains(&target_py)
        })
        .map_err(|e| vm.new_runtime_error(format!("CPython contains error: {}", e)))
    }

    impl AsSequence for Pyo3Ref {
        fn as_sequence() -> &'static PySequenceMethods {
            static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
                length: AtomicCell::new(Some(|seq, vm| {
                    let zelf = Pyo3Ref::sequence_downcast(seq);
                    pyo3_len(&zelf.py_obj, vm)
                })),
                concat: AtomicCell::new(None),
                repeat: AtomicCell::new(None),
                item: AtomicCell::new(Some(|seq, i, vm| {
                    let zelf = Pyo3Ref::sequence_downcast(seq);
                    pyo3_getitem_by_index(&zelf.py_obj, i, vm)
                })),
                ass_item: AtomicCell::new(None),
                contains: AtomicCell::new(Some(|seq, target, vm| {
                    let zelf = Pyo3Ref::sequence_downcast(seq);
                    pyo3_contains(&zelf.py_obj, target, vm)
                })),
                inplace_concat: AtomicCell::new(None),
                inplace_repeat: AtomicCell::new(None),
            };
            &AS_SEQUENCE
        }
    }

    impl AsMapping for Pyo3Ref {
        fn as_mapping() -> &'static PyMappingMethods {
            static AS_MAPPING: PyMappingMethods = PyMappingMethods {
                length: AtomicCell::new(Some(|mapping, vm| {
                    let zelf = Pyo3Ref::mapping_downcast(mapping);
                    pyo3_len(&zelf.py_obj, vm)
                })),
                subscript: AtomicCell::new(Some(|mapping, needle, vm| {
                    let zelf = Pyo3Ref::mapping_downcast(mapping);
                    pyo3_getitem(&zelf.py_obj, needle, vm)
                })),
                ass_subscript: AtomicCell::new(Some(|mapping, needle, value, vm| {
                    let zelf = Pyo3Ref::mapping_downcast(mapping);
                    pyo3_setitem(&zelf.py_obj, needle, value, vm)
                })),
            };
            &AS_MAPPING
        }
    }

    impl Iterable for Pyo3Ref {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let pyo3_obj = pyo3::Python::attach(|py| -> Result<Pyo3Ref, PyErr> {
                let obj = zelf.py_obj.bind(py);
                let builtins = py.import("builtins")?;
                let iter_fn = builtins.getattr("iter")?;
                let iter_result = iter_fn.call1((obj,))?;
                Ok(create_pyo3_object(py, &iter_result))
            })
            .map_err(|e| vm.new_type_error(format!("CPython iter error: {}", e)))?;

            // Iterators should stay as Pyo3Ref, don't try to unpickle
            Ok(pyo3_obj.into_ref(&vm.ctx).into())
        }
    }

    #[pyclass(with(
        GetAttr,
        Callable,
        Representable,
        AsNumber,
        Comparable,
        AsSequence,
        AsMapping,
        Iterable
    ))]
    impl Pyo3Ref {}

    /// Import a module from CPython and return a wrapper object.
    ///
    /// # Arguments
    /// * `name` - The name of the module to import
    ///
    /// # Returns
    /// A Pyo3Ref wrapping the imported module
    #[pyfunction]
    fn import_module(name: PyStrRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let module_name = name.as_str().to_owned();

        let pyo3_obj = pyo3::Python::attach(|py| -> Result<Pyo3Ref, PyErr> {
            let module = py.import(&*module_name)?;
            Ok(create_pyo3_object(py, module.as_any()))
        })
        .map_err(|e| {
            vm.new_import_error(
                format!("Cannot import '{}': {}", module_name, e),
                name.into(),
            )
        })?;

        // Modules should stay as Pyo3Ref, don't try to unpickle
        Ok(pyo3_obj.into_ref(&vm.ctx).into())
    }
}
