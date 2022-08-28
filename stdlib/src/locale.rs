pub(crate) use _locale::make_module;

#[pymodule]
mod _locale {
    use std::{ptr, slice};

    use num_traits::ToPrimitive;
    use rustpython_vm::{
        builtins::{PyBaseExceptionRef, PyStrRef, PyTypeRef},
        function::OptionalArg,
        utils::ToCString,
        VirtualMachine,
    };

    use crate::vm::{builtins::PyIntRef, PyResult};

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
    #[derive(FromArgs)]
    struct LocaleArgs {
        #[pyarg(positional)]
        category: PyIntRef,
        #[pyarg(positional, default = "None")]
        locale: Option<PyStrRef>,
    }

    #[pyfunction]
    fn setlocale(args: LocaleArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let category = args.category;
        let locale = args.locale;
        match locale {
        /* set locale */
            Some(locale) => {
                let result = unsafe {

                    libc::setlocale(
                        category.as_bigint().to_i32().unwrap(),
                        locale.to_cstring(vm)?.as_ptr(),
                    )
                };
                if result.is_null() {
                    /* operation failed, no setting was changed */
                    return Err(new_locale_error(
                        "unsupported locale setting".to_owned(),
                        vm,
                    ));
                }
                Ok(unsafe {
                    slice::from_raw_parts(
                        result as *mut u8,
                        libc::strlen(result),
                    ).to_vec()
                })
            }
            None => {
                /* get locale */
                let result =
                    unsafe { libc::setlocale(category.as_bigint().to_i32().unwrap(), ptr::null()) };
                if result.is_null() {
                    return Err(new_locale_error("locale query failed".to_owned(), vm));
                }
                //let result_object = PyUnicode_DecodeLocale(result, NULL);
                Ok(unsafe {
                    slice::from_raw_parts(
                        result as *mut u8,
                        libc::strlen(result),
                    ).to_vec()
                })
            }
        }
    }
}
