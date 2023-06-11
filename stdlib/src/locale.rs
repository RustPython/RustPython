pub(crate) use _locale::make_module;

#[cfg(windows)]
#[repr(C)]
struct lconv {
    decimal_point: *mut libc::c_char,
    thousands_sep: *mut libc::c_char,
    grouping: *mut libc::c_char,
    int_curr_symbol: *mut libc::c_char,
    currency_symbol: *mut libc::c_char,
    mon_decimal_point: *mut libc::c_char,
    mon_thousands_sep: *mut libc::c_char,
    mon_grouping: *mut libc::c_char,
    positive_sign: *mut libc::c_char,
    negative_sign: *mut libc::c_char,
    int_frac_digits: libc::c_char,
    frac_digits: libc::c_char,
    p_cs_precedes: libc::c_char,
    p_sep_by_space: libc::c_char,
    n_cs_precedes: libc::c_char,
    n_sep_by_space: libc::c_char,
    p_sign_posn: libc::c_char,
    n_sign_posn: libc::c_char,
    int_p_cs_precedes: libc::c_char,
    int_n_cs_precedes: libc::c_char,
    int_p_sep_by_space: libc::c_char,
    int_n_sep_by_space: libc::c_char,
    int_p_sign_posn: libc::c_char,
    int_n_sign_posn: libc::c_char,
}

#[cfg(windows)]
extern "C" {
    fn localeconv() -> *mut lconv;
}

#[cfg(unix)]
use libc::localeconv;

#[pymodule]
mod _locale {
    use rustpython_vm::{
        builtins::{PyDictRef, PyIntRef, PyListRef, PyStrRef, PyTypeRef},
        convert::ToPyException,
        function::OptionalArg,
        PyObjectRef, PyResult, VirtualMachine,
    };
    use std::{
        ffi::{CStr, CString},
        ptr,
    };

    #[cfg(all(
        unix,
        not(any(target_os = "ios", target_os = "android", target_os = "redox"))
    ))]
    #[pyattr]
    use libc::{
        ABDAY_1, ABDAY_2, ABDAY_3, ABDAY_4, ABDAY_5, ABDAY_6, ABDAY_7, ABMON_1, ABMON_10, ABMON_11,
        ABMON_12, ABMON_2, ABMON_3, ABMON_4, ABMON_5, ABMON_6, ABMON_7, ABMON_8, ABMON_9,
        ALT_DIGITS, AM_STR, CODESET, CRNCYSTR, DAY_1, DAY_2, DAY_3, DAY_4, DAY_5, DAY_6, DAY_7,
        D_FMT, D_T_FMT, ERA, ERA_D_FMT, ERA_D_T_FMT, ERA_T_FMT, LC_MESSAGES, MON_1, MON_10, MON_11,
        MON_12, MON_2, MON_3, MON_4, MON_5, MON_6, MON_7, MON_8, MON_9, NOEXPR, PM_STR, RADIXCHAR,
        THOUSEP, T_FMT, T_FMT_AMPM, YESEXPR,
    };

    #[pyattr]
    use libc::{LC_ALL, LC_COLLATE, LC_CTYPE, LC_MONETARY, LC_NUMERIC, LC_TIME};

    #[pyattr(name = "CHAR_MAX")]
    fn char_max(vm: &VirtualMachine) -> PyIntRef {
        vm.ctx.new_int(libc::c_char::MAX)
    }

    unsafe fn copy_grouping(group: *const libc::c_char, vm: &VirtualMachine) -> PyListRef {
        let mut group_vec: Vec<PyObjectRef> = Vec::new();
        if group.is_null() {
            return vm.ctx.new_list(group_vec);
        }

        let mut ptr = group;
        while ![0, libc::c_char::MAX].contains(&*ptr) {
            let val = vm.ctx.new_int(*ptr);
            group_vec.push(val.into());
            ptr = ptr.add(1);
        }
        // https://github.com/python/cpython/blob/677320348728ce058fa3579017e985af74a236d4/Modules/_localemodule.c#L80
        if !group_vec.is_empty() {
            group_vec.push(vm.ctx.new_int(0).into());
        }
        vm.ctx.new_list(group_vec)
    }

    unsafe fn pystr_from_raw_cstr(vm: &VirtualMachine, raw_ptr: *const libc::c_char) -> PyResult {
        let slice = unsafe { CStr::from_ptr(raw_ptr) };
        let string = slice
            .to_str()
            .expect("localeconv always return decodable string");
        Ok(vm.new_pyobj(string))
    }

    #[pyattr(name = "Error", once)]
    fn error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "locale",
            "Error",
            Some(vec![vm.ctx.exceptions.exception_type.to_owned()]),
        )
    }

    #[pyfunction]
    fn strcoll(string1: PyStrRef, string2: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let cstr1 = CString::new(string1.as_str()).map_err(|e| e.to_pyexception(vm))?;
        let cstr2 = CString::new(string2.as_str()).map_err(|e| e.to_pyexception(vm))?;
        Ok(vm.new_pyobj(unsafe { libc::strcoll(cstr1.as_ptr(), cstr2.as_ptr()) }))
    }

    #[pyfunction]
    fn strxfrm(string: PyStrRef, vm: &VirtualMachine) -> PyResult {
        // https://github.com/python/cpython/blob/eaae563b6878aa050b4ad406b67728b6b066220e/Modules/_localemodule.c#L390-L442
        let n1 = string.byte_len() + 1;
        let mut buff = vec![0u8; n1];

        let cstr = CString::new(string.as_str()).map_err(|e| e.to_pyexception(vm))?;
        let n2 = unsafe { libc::strxfrm(buff.as_mut_ptr() as _, cstr.as_ptr(), n1) };
        buff = vec![0u8; n2 + 1];
        unsafe {
            libc::strxfrm(buff.as_mut_ptr() as _, cstr.as_ptr(), n2 + 1);
        }
        Ok(vm.new_pyobj(String::from_utf8(buff).expect("strxfrm returned invalid utf-8 string")))
    }

    #[pyfunction]
    fn localeconv(vm: &VirtualMachine) -> PyResult<PyDictRef> {
        let result = vm.ctx.new_dict();

        unsafe {
            macro_rules! set_string_field {
                ($lc:expr, $field:ident) => {{
                    result.set_item(
                        stringify!($field),
                        pystr_from_raw_cstr(vm, (*$lc).$field)?,
                        vm,
                    )?
                }};
            }

            macro_rules! set_int_field {
                ($lc:expr, $field:ident) => {{
                    result.set_item(stringify!($field), vm.new_pyobj((*$lc).$field), vm)?
                }};
            }

            macro_rules! set_group_field {
                ($lc:expr, $field:ident) => {{
                    result.set_item(
                        stringify!($field),
                        copy_grouping((*$lc).$field, vm).into(),
                        vm,
                    )?
                }};
            }

            let lc = super::localeconv();
            set_group_field!(lc, mon_grouping);
            set_group_field!(lc, grouping);
            set_int_field!(lc, int_frac_digits);
            set_int_field!(lc, frac_digits);
            set_int_field!(lc, p_cs_precedes);
            set_int_field!(lc, p_sep_by_space);
            set_int_field!(lc, n_cs_precedes);
            set_int_field!(lc, p_sign_posn);
            set_int_field!(lc, n_sign_posn);
            set_string_field!(lc, decimal_point);
            set_string_field!(lc, thousands_sep);
            set_string_field!(lc, int_curr_symbol);
            set_string_field!(lc, currency_symbol);
            set_string_field!(lc, mon_decimal_point);
            set_string_field!(lc, mon_thousands_sep);
            set_int_field!(lc, n_sep_by_space);
            set_string_field!(lc, positive_sign);
            set_string_field!(lc, negative_sign);
        }
        Ok(result)
    }

    #[derive(FromArgs)]
    struct LocaleArgs {
        #[pyarg(any)]
        category: i32,
        #[pyarg(any, optional)]
        locale: OptionalArg<Option<PyStrRef>>,
    }

    #[pyfunction]
    fn setlocale(args: LocaleArgs, vm: &VirtualMachine) -> PyResult {
        let error = error(vm);
        if cfg!(windows) && (args.category < LC_ALL || args.category > LC_TIME) {
            return Err(vm.new_exception_msg(error, String::from("unsupported locale setting")));
        }
        unsafe {
            let result = match args.locale.flatten() {
                None => libc::setlocale(args.category, ptr::null()),
                Some(locale) => {
                    let c_locale: CString =
                        CString::new(locale.as_str()).map_err(|e| e.to_pyexception(vm))?;
                    libc::setlocale(args.category, c_locale.as_ptr())
                }
            };
            if result.is_null() {
                return Err(vm.new_exception_msg(error, String::from("unsupported locale setting")));
            }
            pystr_from_raw_cstr(vm, result)
        }
    }
}
