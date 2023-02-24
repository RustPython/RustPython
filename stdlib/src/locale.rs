#[cfg(unix)]
pub(crate) use _locale::make_module;

#[cfg(unix)]
#[pymodule]
mod _locale {
    use rustpython_vm::{
        builtins::{PyDictRef, PyIntRef, PyListRef},
        PyObjectRef, PyResult, VirtualMachine,
    };

    #[pyattr]
    use libc::{
        ABDAY_1, ABDAY_2, ABDAY_3, ABDAY_4, ABDAY_5, ABDAY_6, ABDAY_7, ABMON_1, ABMON_10, ABMON_11,
        ABMON_12, ABMON_2, ABMON_3, ABMON_4, ABMON_5, ABMON_6, ABMON_7, ABMON_8, ABMON_9,
        ALT_DIGITS, AM_STR, CODESET, CRNCYSTR, DAY_1, DAY_2, DAY_3, DAY_4, DAY_5, DAY_6, DAY_7,
        D_FMT, D_T_FMT, ERA, ERA_D_FMT, ERA_D_T_FMT, ERA_T_FMT, LC_ALL, LC_COLLATE, LC_CTYPE,
        LC_MESSAGES, LC_MONETARY, LC_NUMERIC, LC_TIME, MON_1, MON_10, MON_11, MON_12, MON_2, MON_3,
        MON_4, MON_5, MON_6, MON_7, MON_8, MON_9, NOEXPR, PM_STR, RADIXCHAR, THOUSEP, T_FMT,
        T_FMT_AMPM, YESEXPR,
    };

    use std::ffi::{c_char, CStr};

    #[pyattr(name = "CHAR_MAX")]
    fn char_max(vm: &VirtualMachine) -> PyIntRef {
        vm.ctx.new_int(libc::c_char::MAX)
    }

    unsafe fn copy_grouping(group: *mut c_char, vm: &VirtualMachine) -> PyListRef {
        let mut group_vec: Vec<PyObjectRef> = Vec::new();
        let mut ptr = group;

        while ![0 as i8, c_char::MAX].contains(&*ptr) {
            let val = vm.ctx.new_int(*ptr);
            group_vec.push(val.into());
            ptr = ptr.offset(1);
        }
        vm.ctx.new_list(group_vec)
    }

    unsafe fn _parse_ptr_to_str(vm: &VirtualMachine, raw_ptr: *mut c_char) -> PyResult {
        let slice = unsafe { CStr::from_ptr(raw_ptr) };
        let cstr = slice
            .to_str()
            .expect("localeconv always return decodable string");

        Ok(vm.new_pyobj(cstr))
    }

    #[pyfunction]
    fn localeconv(vm: &VirtualMachine) -> PyResult<PyDictRef> {
        let result = vm.ctx.new_dict();

        unsafe {
            let lc = libc::localeconv();

            macro_rules! set_string_field {
                ($field:ident) => {{
                    result.set_item(stringify!($field), _parse_ptr_to_str(vm, (*lc).$field)?, vm)?
                }};
            }

            macro_rules! set_int_field {
                ($field:ident) => {{
                    result.set_item(stringify!($field), vm.new_pyobj((*lc).$field), vm)?
                }};
            }

            macro_rules! set_group_field {
                ($field:ident) => {{
                    result.set_item(
                        stringify!($field),
                        copy_grouping((*lc).$field, vm).into(),
                        vm,
                    )?
                }};
            }

            set_group_field!(mon_grouping);
            set_group_field!(grouping);
            set_int_field!(int_frac_digits);
            set_int_field!(frac_digits);
            set_int_field!(p_cs_precedes);
            set_int_field!(p_sep_by_space);
            set_int_field!(n_cs_precedes);
            set_int_field!(p_sign_posn);
            set_int_field!(n_sign_posn);
            set_string_field!(decimal_point);
            set_string_field!(thousands_sep);
            set_string_field!(int_curr_symbol);
            set_string_field!(currency_symbol);
            set_string_field!(mon_decimal_point);
            set_string_field!(mon_thousands_sep);
            set_int_field!(n_sep_by_space);
            set_string_field!(positive_sign);
            set_string_field!(negative_sign);
        }
        Ok(result)
    }
}
