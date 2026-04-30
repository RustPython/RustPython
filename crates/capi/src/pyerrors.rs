use crate::{PyObject, with_vm};
use core::convert::Infallible;
use core::ffi::{CStr, c_char, c_int};
use core::mem::MaybeUninit;
use rustpython_vm::builtins::{PyTuple, PyType};
use rustpython_vm::convert::IntoObject;
use rustpython_vm::exceptions::ExceptionZoo;
use rustpython_vm::{AsObject, PyResult};

macro_rules! define_exception_statics {
    ($( $(#[$meta:meta])* $export:ident => $zoo:ident ),* $(,)?) => {
        $(
            $(#[$meta])*
            #[unsafe(no_mangle)]
            pub static mut $export: MaybeUninit<*mut PyObject> = MaybeUninit::uninit();
        )*

        #[allow(static_mut_refs)]
        pub(crate) unsafe fn init_exception_statics(exc: &ExceptionZoo) {
            unsafe {
                $(
                    $export.write(exc.$zoo.as_object().as_raw().cast_mut());
                )*
            }
        }
    };
}

define_exception_statics! {
    PyExc_BaseException => base_exception_type,
    PyExc_BaseExceptionGroup => base_exception_group,
    PyExc_SystemExit => system_exit,
    PyExc_KeyboardInterrupt => keyboard_interrupt,
    PyExc_GeneratorExit => generator_exit,
    PyExc_Exception => exception_type,
    PyExc_StopIteration => stop_iteration,
    PyExc_StopAsyncIteration => stop_async_iteration,
    PyExc_ArithmeticError => arithmetic_error,
    PyExc_FloatingPointError => floating_point_error,
    PyExc_SystemError => system_error,
    PyExc_TypeError => type_error,
    PyExc_OverflowError => overflow_error,
    PyExc_ZeroDivisionError => zero_division_error,
    PyExc_AssertionError => assertion_error,
    PyExc_IndexError => index_error,
    PyExc_KeyError => key_error,
    PyExc_LookupError => lookup_error,
    PyExc_AttributeError => attribute_error,
    PyExc_BufferError => buffer_error,
    PyExc_EOFError => eof_error,
    PyExc_ImportError => import_error,
    PyExc_ModuleNotFoundError => module_not_found_error,
    PyExc_MemoryError => memory_error,
    PyExc_NameError => name_error,
    PyExc_UnboundLocalError => unbound_local_error,
    PyExc_OSError => os_error,
    PyExc_BlockingIOError => blocking_io_error,
    PyExc_ChildProcessError => child_process_error,
    PyExc_ConnectionError => connection_error,
    PyExc_BrokenPipeError => broken_pipe_error,
    PyExc_ConnectionAbortedError => connection_aborted_error,
    PyExc_ConnectionRefusedError => connection_refused_error,
    PyExc_ConnectionResetError => connection_reset_error,
    PyExc_FileExistsError => file_exists_error,
    PyExc_FileNotFoundError => file_not_found_error,
    PyExc_InterruptedError => interrupted_error,
    PyExc_IsADirectoryError => is_a_directory_error,
    PyExc_NotADirectoryError => not_a_directory_error,
    PyExc_PermissionError => permission_error,
    PyExc_ProcessLookupError => process_lookup_error,
    PyExc_TimeoutError => timeout_error,
    PyExc_ReferenceError => reference_error,
    PyExc_RuntimeError => runtime_error,
    PyExc_NotImplementedError => not_implemented_error,
    PyExc_RecursionError => recursion_error,
    PyExc_SyntaxError => syntax_error,
    PyExc_IndentationError => indentation_error,
    PyExc_TabError => tab_error,
    PyExc_ValueError => value_error,
    PyExc_UnicodeError => unicode_error,
    PyExc_UnicodeDecodeError => unicode_decode_error,
    PyExc_UnicodeEncodeError => unicode_encode_error,
    PyExc_UnicodeTranslateError => unicode_translate_error,
    PyExc_Warning => warning,
    PyExc_DeprecationWarning => deprecation_warning,
    PyExc_PendingDeprecationWarning => pending_deprecation_warning,
    PyExc_RuntimeWarning => runtime_warning,
    PyExc_SyntaxWarning => syntax_warning,
    PyExc_UserWarning => user_warning,
    PyExc_FutureWarning => future_warning,
    PyExc_ImportWarning => import_warning,
    PyExc_UnicodeWarning => unicode_warning,
    PyExc_BytesWarning => bytes_warning,
    PyExc_ResourceWarning => resource_warning,
    PyExc_EncodingWarning => encoding_warning,
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_GetRaisedException() -> *mut PyObject {
    with_vm(|vm| vm.take_raised_exception())
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_SetRaisedException(exc: *mut PyObject) {
    with_vm::<PyResult<Infallible>, _>(|_vm| {
        let exception = unsafe { (&*exc).to_owned().downcast_unchecked() };
        Err(exception)
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_SetObject(exception: *mut PyObject, value: *mut PyObject) {
    with_vm::<PyResult<Infallible>, _>(|vm| {
        let exc_type = unsafe { (&*exception).to_owned() };
        let exc_val = unsafe { (&*value).to_owned() };

        let normalized = vm.normalize_exception(exc_type, exc_val, vm.ctx.none())?;

        Err(normalized)
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_SetString(exception: *mut PyObject, message: *const c_char) {
    with_vm::<PyResult<Infallible>, _>(|vm| {
        let exc_type = unsafe { &*exception }.try_downcast_ref::<PyType>(vm)?;

        let message = unsafe { CStr::from_ptr(message) }
            .to_str()
            .expect("Exception message is not valid UTF-8");

        let exc = vm.invoke_exception(
            exc_type.to_owned(),
            vec![vm.ctx.new_str(message).into_object()],
        )?;

        Err(exc)
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_PrintEx(_set_sys_last_vars: c_int) {
    with_vm(|vm| {
        let exception = vm
            .take_raised_exception()
            .expect("No exception set in PyErr_PrintEx");

        vm.print_exception(exception);
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_WriteUnraisable(obj: *mut PyObject) {
    with_vm(|vm| {
        let exception = vm
            .take_raised_exception()
            .expect("No exception set in PyErr_WriteUnraisable");

        let object = unsafe { vm.unwrap_or_none(obj.as_ref().map(|obj| obj.to_owned())) };

        vm.run_unraisable(exception, None, object)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_NewException(
    name: *const c_char,
    base: *mut PyObject,
    dict: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let (module, name) = unsafe {
            CStr::from_ptr(name)
                .to_str()
                .expect("Exception name is not valid UTF-8")
                .rsplit_once('.')
                .expect("Exception name must be of the form 'module.ExceptionName'")
        };

        let bases = unsafe { base.as_ref() }.map(|bases| {
            if let Some(ty) = bases.downcast_ref::<PyType>() {
                vec![ty.to_owned()]
            } else if let Some(tuple) = bases.downcast_ref::<PyTuple>() {
                tuple
                    .iter()
                    .map(|item| item.to_owned().downcast())
                    .collect::<Result<Vec<_>, _>>()
                    .expect("PyErr_NewException base tuple must contain only types")
            } else {
                panic!("PyErr_NewException base must be a type or a tuple of types");
            }
        });

        assert!(
            dict.is_null(),
            "PyErr_NewException with non-null dict is not supported yet"
        );

        vm.ctx.new_exception_type(module, name, bases)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_NewExceptionWithDoc(
    name: *const c_char,
    _doc: *const c_char,
    base: *mut PyObject,
    dict: *mut PyObject,
) -> *mut PyObject {
    PyErr_NewException(name, base, dict)
}

#[unsafe(no_mangle)]
pub extern "C" fn PyException_GetTraceback(exc: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let exc = unsafe { &*exc };
        exc.get_attr("__traceback__", vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyException_SetCause(exc: *mut PyObject, cause: *mut PyObject) {
    with_vm(|vm| {
        let exc = unsafe { &*exc };
        let cause = unsafe { cause.as_ref() }.map(|obj| obj.to_owned());
        exc.set_attr("__cause__", vm.unwrap_or_none(cause), vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyException_SetTraceback(exc: *mut PyObject, tb: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let exc = unsafe { &*exc };
        let traceback = unsafe { tb.as_ref() }.map(|obj| obj.to_owned());
        exc.set_attr("__traceback__", vm.unwrap_or_none(traceback), vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyErr_GivenExceptionMatches(given: *mut PyObject, exc: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let given = unsafe { &*given };
        let exc = unsafe { &*exc };

        if let Some(exc_type) = exc.downcast_ref::<PyType>() {
            given.is_subclass(exc_type.as_ref(), vm)
        } else if let Some(exc_tuple) = exc.downcast_ref::<PyTuple>() {
            Ok(exc_tuple
                .iter()
                .any(|ty| given.is_subclass(ty, vm).unwrap_or_default()))
        } else {
            Ok(false)
        }
    })
}

#[cfg(test)]
mod tests {
    use pyo3::PyTypeInfo;
    use pyo3::create_exception;
    use pyo3::exceptions::{PyException, PyTypeError};
    use pyo3::prelude::*;

    #[test]
    fn test_raised_exception() {
        Python::attach(|py| {
            PyTypeError::new_err("This is a type error").restore(py);
            assert!(PyErr::take(py).is_some());
        })
    }

    #[test]
    fn test_new_exception_type() {
        create_exception!(my_module, MyError, PyException, "Some description.");

        Python::attach(|py| {
            let exc = MyError::new_err("This is a new exception");
            assert!(exc.is_instance_of::<MyError>(py));
            let exc_type = MyError::type_object(py);
            assert_eq!(
                exc_type.fully_qualified_name().unwrap(),
                "my_module.MyError"
            );
        })
    }
}
