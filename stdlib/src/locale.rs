pub(crate) use locale::make_module;

#[pymodule]
mod locale {
    use std::ptr;

    use num_traits::ToPrimitive;
    use rustpython_vm::{PyObjectRef, VirtualMachine, builtins::{PyTypeRef, PyBaseExceptionRef, PyStr}, utils::ToCString};

    use crate::vm::{
        builtins::PyIntRef,
        PyResult,
    };

    struct LocaleState {
        error: PyObjectRef,
    }

    fn new_locale_error(msg: String, vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_exception_msg(error_type(vm), msg)
    }

    #[pyattr(once)]
    fn error_type(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "locale",
            "error",
            Some(vec![vm.ctx.exceptions.value_error.to_owned()]),
        )
    }

    #[pyfunction]
    fn setlocale(category: PyIntRef, locale: Option<PyStr>, vm: &VirtualMachine) -> PyResult<*mut i8> {
        match locale {
            /* set locale */
            Some(locale) => {
                let result = unsafe { libc::setlocale(category.as_bigint().to_i32().unwrap(), locale.to_cstring(vm).unwrap().as_ptr()) };
                if result == 0 as *mut i8 {
                    /* operation failed, no setting was changed */
                    return Err(new_locale_error("unsupported locale setting".to_owned(), vm));
                }
                Ok(result)
            },
            None => {
                /* get locale */
                let result = unsafe { libc::setlocale(category.as_bigint().to_i32().unwrap(), ptr::null()) };
                if result == 0 as *mut i8 {
                    return Err(new_locale_error("locale query failed".to_owned(), vm));
                }
                //let result_object = PyUnicode_DecodeLocale(result, NULL);
                Ok(result)
            }
        }
    }
}
