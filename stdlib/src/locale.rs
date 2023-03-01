pub(crate) use _locale::make_module;

#[pymodule]
mod _locale {
    use rustpython_vm::{
        builtins::{PyDictRef, PyListRef, PyStrRef, PyTypeRef},
        convert::ToPyException,
        function::OptionalArg,
        PyObjectRef, PyResult, VirtualMachine,
    };
    
    use std::{
        ffi::{CStr, CString, c_char},
        ptr,
    };

    #[repr(C)]
    struct lconv {
        decimal_point: *mut c_char,
        thousands_sep: *mut c_char,
        grouping: *mut c_char,
        int_curr_symbol: *mut c_char,
        currency_symbol: *mut c_char,
        mon_decimal_point: *mut c_char,
        mon_thousands_sep: *mut c_char,
        mon_grouping: *mut c_char,
        positive_sign: *mut c_char,
        negative_sign: *mut c_char,
        int_frac_digits: c_char,
        frac_digits: c_char,
        p_cs_precedes: c_char,
        p_sep_by_space: c_char,
        n_cs_precedes: c_char,
        n_sep_by_space: c_char,
        p_sign_posn: c_char,
        n_sign_posn: c_char,
        int_p_cs_precedes: c_char,
        int_n_cs_precedes: c_char,
        int_p_sep_by_space: c_char,
        int_n_sep_by_space: c_char,
        int_p_sign_posn: c_char,
        int_n_sign_posn: c_char,
    }

    #[link(name = "liblocale", kind="static")]
    extern "C" {
        fn abday_1() -> i32;
        fn abday_2() -> i32;
        fn abday_3() -> i32;
        fn abday_4() -> i32;
        fn abday_5() -> i32;
        fn abday_6() -> i32;
        fn abday_7() -> i32;
        fn abmon_1() -> i32;
        fn abmon_10() -> i32;
        fn abmon_11() -> i32;
        fn abmon_12() -> i32;
        fn abmon_2() -> i32;
        fn abmon_3() -> i32;
        fn abmon_4() -> i32;
        fn abmon_5() -> i32;
        fn abmon_6() -> i32;
        fn abmon_7() -> i32;
        fn abmon_8() -> i32;
        fn abmon_9() -> i32;
        fn alt_digits() -> i32;
        fn am_str() -> i32;
        fn char_max() -> i32;
        fn codeset() -> i32;
        fn crncystr() -> i32;
        fn day_1() -> i32;
        fn day_2() -> i32;
        fn day_3() -> i32;
        fn day_4() -> i32;
        fn day_5() -> i32;
        fn day_6() -> i32;
        fn day_7() -> i32;
        fn d_fmt() -> i32;
        fn d_t_fmt() -> i32;
        fn era() -> i32;
        fn era_d_fmt() -> i32;
        fn era_d_t_fmt() -> i32;
        fn era_t_fmt() -> i32;
        fn lc_all() -> i32;
        fn lc_collate() -> i32;
        fn lc_ctype() -> i32;
        fn lc_messages() -> i32;
        fn lc_monetary() -> i32;
        fn lc_numeric() -> i32;
        fn lc_time() -> i32;
        fn mon_1() -> i32;
        fn mon_10() -> i32;
        fn mon_11() -> i32;
        fn mon_12() -> i32;
        fn mon_2() -> i32;
        fn mon_3() -> i32;
        fn mon_4() -> i32;
        fn mon_5() -> i32;
        fn mon_6() -> i32;
        fn mon_7() -> i32;
        fn mon_8() -> i32;
        fn mon_9() -> i32;
        fn noexpr() -> i32;
        fn pm_str() -> i32;
        fn radixchar() -> i32;
        fn thousep() -> i32;
        fn t_fmt() -> i32;
        fn t_fmt_ampm() -> i32;
        fn yesexpr() -> i32;
        fn _localeconv() -> *mut lconv;
    }

    
    #[pyattr(name = "ABDAY_1")]
    fn get_abday_1(_vm: &VirtualMachine) -> i32 {
        unsafe {
            abday_1()
        }
    }


    #[pyattr(name = "ABDAY_2")]
    fn get_abday_2(_vm: &VirtualMachine) -> i32 {
        unsafe {
            abday_2()
        }
    }


    #[pyattr(name = "ABDAY_3")]
    fn get_abday_3(_vm: &VirtualMachine) -> i32 {
        unsafe {
            abday_3()
        }
    }


    #[pyattr(name = "ABDAY_4")]
    fn get_abday_4(_vm: &VirtualMachine) -> i32 {
        unsafe {
            abday_4()
        }
    }


    #[pyattr(name = "ABDAY_5")]
    fn get_abday_5(_vm: &VirtualMachine) -> i32 {
        unsafe {
            abday_5()
        }
    }


    #[pyattr(name = "ABDAY_6")]
    fn get_abday_6(_vm: &VirtualMachine) -> i32 {
        unsafe {
            abday_6()
        }
    }


    #[pyattr(name = "ABDAY_7")]
    fn get_abday_7(_vm: &VirtualMachine) -> i32 {
        unsafe {
            abday_7()
        }
    }


    #[pyattr(name = "ABMON_1")]
    fn get_abmon_1(_vm: &VirtualMachine) -> i32 {
        unsafe {
            abmon_1()
        }
    }


    #[pyattr(name = "ABMON_10")]
    fn get_abmon_10(_vm: &VirtualMachine) -> i32 {
        unsafe {
            abmon_10()
        }
    }


    #[pyattr(name = "ABMON_11")]
    fn get_abmon_11(_vm: &VirtualMachine) -> i32 {
        unsafe {
            abmon_11()
        }
    }


    #[pyattr(name = "ABMON_12")]
    fn get_abmon_12(_vm: &VirtualMachine) -> i32 {
        unsafe {
            abmon_12()
        }
    }


    #[pyattr(name = "ABMON_2")]
    fn get_abmon_2(_vm: &VirtualMachine) -> i32 {
        unsafe {
            abmon_2()
        }
    }


    #[pyattr(name = "ABMON_3")]
    fn get_abmon_3(_vm: &VirtualMachine) -> i32 {
        unsafe {
            abmon_3()
        }
    }


    #[pyattr(name = "ABMON_4")]
    fn get_abmon_4(_vm: &VirtualMachine) -> i32 {
        unsafe {
            abmon_4()
        }
    }


    #[pyattr(name = "ABMON_5")]
    fn get_abmon_5(_vm: &VirtualMachine) -> i32 {
        unsafe {
            abmon_5()
        }
    }


    #[pyattr(name = "ABMON_6")]
    fn get_abmon_6(_vm: &VirtualMachine) -> i32 {
        unsafe {
            abmon_6()
        }
    }


    #[pyattr(name = "ABMON_7")]
    fn get_abmon_7(_vm: &VirtualMachine) -> i32 {
        unsafe {
            abmon_7()
        }
    }


    #[pyattr(name = "ABMON_8")]
    fn get_abmon_8(_vm: &VirtualMachine) -> i32 {
        unsafe {
            abmon_8()
        }
    }


    #[pyattr(name = "ABMON_9")]
    fn get_abmon_9(_vm: &VirtualMachine) -> i32 {
        unsafe {
            abmon_9()
        }
    }


    #[pyattr(name = "ALT_DIGITS")]
    fn get_alt_digits(_vm: &VirtualMachine) -> i32 {
        unsafe {
            alt_digits()
        }
    }


    #[pyattr(name = "AM_STR")]
    fn get_am_str(_vm: &VirtualMachine) -> i32 {
        unsafe {
            am_str()
        }
    }


    #[pyattr(name = "CHAR_MAX")]
    fn get_char_max(_vm: &VirtualMachine) -> i32 {
        unsafe {
            char_max()
        }
    }


    #[pyattr(name = "CODESET")]
    fn get_codeset(_vm: &VirtualMachine) -> i32 {
        unsafe {
            codeset()
        }
    }


    #[pyattr(name = "CRNCYSTR")]
    fn get_crncystr(_vm: &VirtualMachine) -> i32 {
        unsafe {
            crncystr()
        }
    }


    #[pyattr(name = "DAY_1")]
    fn get_day_1(_vm: &VirtualMachine) -> i32 {
        unsafe {
            day_1()
        }
    }


    #[pyattr(name = "DAY_2")]
    fn get_day_2(_vm: &VirtualMachine) -> i32 {
        unsafe {
            day_2()
        }
    }


    #[pyattr(name = "DAY_3")]
    fn get_day_3(_vm: &VirtualMachine) -> i32 {
        unsafe {
            day_3()
        }
    }


    #[pyattr(name = "DAY_4")]
    fn get_day_4(_vm: &VirtualMachine) -> i32 {
        unsafe {
            day_4()
        }
    }


    #[pyattr(name = "DAY_5")]
    fn get_day_5(_vm: &VirtualMachine) -> i32 {
        unsafe {
            day_5()
        }
    }


    #[pyattr(name = "DAY_6")]
    fn get_day_6(_vm: &VirtualMachine) -> i32 {
        unsafe {
            day_6()
        }
    }


    #[pyattr(name = "DAY_7")]
    fn get_day_7(_vm: &VirtualMachine) -> i32 {
        unsafe {
            day_7()
        }
    }


    #[pyattr(name = "D_FMT")]
    fn get_d_fmt(_vm: &VirtualMachine) -> i32 {
        unsafe {
            d_fmt()
        }
    }


    #[pyattr(name = "D_T_FMT")]
    fn get_d_t_fmt(_vm: &VirtualMachine) -> i32 {
        unsafe {
            d_t_fmt()
        }
    }


    #[pyattr(name = "ERA")]
    fn get_era(_vm: &VirtualMachine) -> i32 {
        unsafe {
            era()
        }
    }


    #[pyattr(name = "ERA_D_FMT")]
    fn get_era_d_fmt(_vm: &VirtualMachine) -> i32 {
        unsafe {
            era_d_fmt()
        }
    }


    #[pyattr(name = "ERA_D_T_FMT")]
    fn get_era_d_t_fmt(_vm: &VirtualMachine) -> i32 {
        unsafe {
            era_d_t_fmt()
        }
    }


    #[pyattr(name = "ERA_T_FMT")]
    fn get_era_t_fmt(_vm: &VirtualMachine) -> i32 {
        unsafe {
            era_t_fmt()
        }
    }


    #[pyattr(name = "LC_ALL")]
    fn get_lc_all(_vm: &VirtualMachine) -> i32 {
        unsafe {
            lc_all()
        }
    }


    #[pyattr(name = "LC_COLLATE")]
    fn get_lc_collate(_vm: &VirtualMachine) -> i32 {
        unsafe {
            lc_collate()
        }
    }


    #[pyattr(name = "LC_CTYPE")]
    fn get_lc_ctype(_vm: &VirtualMachine) -> i32 {
        unsafe {
            lc_ctype()
        }
    }


    #[pyattr(name = "LC_MESSAGES")]
    fn get_lc_messages(_vm: &VirtualMachine) -> i32 {
        unsafe {
            lc_messages()
        }
    }


    #[pyattr(name = "LC_MONETARY")]
    fn get_lc_monetary(_vm: &VirtualMachine) -> i32 {
        unsafe {
            lc_monetary()
        }
    }


    #[pyattr(name = "LC_NUMERIC")]
    fn get_lc_numeric(_vm: &VirtualMachine) -> i32 {
        unsafe {
            lc_numeric()
        }
    }


    #[pyattr(name = "LC_TIME")]
    fn get_lc_time(_vm: &VirtualMachine) -> i32 {
        unsafe {
            lc_time()
        }
    }


    #[pyattr(name = "MON_1")]
    fn get_mon_1(_vm: &VirtualMachine) -> i32 {
        unsafe {
            mon_1()
        }
    }


    #[pyattr(name = "MON_10")]
    fn get_mon_10(_vm: &VirtualMachine) -> i32 {
        unsafe {
            mon_10()
        }
    }


    #[pyattr(name = "MON_11")]
    fn get_mon_11(_vm: &VirtualMachine) -> i32 {
        unsafe {
            mon_11()
        }
    }


    #[pyattr(name = "MON_12")]
    fn get_mon_12(_vm: &VirtualMachine) -> i32 {
        unsafe {
            mon_12()
        }
    }


    #[pyattr(name = "MON_2")]
    fn get_mon_2(_vm: &VirtualMachine) -> i32 {
        unsafe {
            mon_2()
        }
    }


    #[pyattr(name = "MON_3")]
    fn get_mon_3(_vm: &VirtualMachine) -> i32 {
        unsafe {
            mon_3()
        }
    }


    #[pyattr(name = "MON_4")]
    fn get_mon_4(_vm: &VirtualMachine) -> i32 {
        unsafe {
            mon_4()
        }
    }


    #[pyattr(name = "MON_5")]
    fn get_mon_5(_vm: &VirtualMachine) -> i32 {
        unsafe {
            mon_5()
        }
    }


    #[pyattr(name = "MON_6")]
    fn get_mon_6(_vm: &VirtualMachine) -> i32 {
        unsafe {
            mon_6()
        }
    }


    #[pyattr(name = "MON_7")]
    fn get_mon_7(_vm: &VirtualMachine) -> i32 {
        unsafe {
            mon_7()
        }
    }


    #[pyattr(name = "MON_8")]
    fn get_mon_8(_vm: &VirtualMachine) -> i32 {
        unsafe {
            mon_8()
        }
    }


    #[pyattr(name = "MON_9")]
    fn get_mon_9(_vm: &VirtualMachine) -> i32 {
        unsafe {
            mon_9()
        }
    }


    #[pyattr(name = "NOEXPR")]
    fn get_noexpr(_vm: &VirtualMachine) -> i32 {
        unsafe {
            noexpr()
        }
    }


    #[pyattr(name = "PM_STR")]
    fn get_pm_str(_vm: &VirtualMachine) -> i32 {
        unsafe {
            pm_str()
        }
    }


    #[pyattr(name = "RADIXCHAR")]
    fn get_radixchar(_vm: &VirtualMachine) -> i32 {
        unsafe {
            radixchar()
        }
    }


    #[pyattr(name = "THOUSEP")]
    fn get_thousep(_vm: &VirtualMachine) -> i32 {
        unsafe {
            thousep()
        }
    }


    #[pyattr(name = "T_FMT")]
    fn get_t_fmt(_vm: &VirtualMachine) -> i32 {
        unsafe {
            t_fmt()
        }
    }


    #[pyattr(name = "T_FMT_AMPM")]
    fn get_t_fmt_ampm(_vm: &VirtualMachine) -> i32 {
        unsafe {
            t_fmt_ampm()
        }
    }


    #[pyattr(name = "YESEXPR")]
    fn get_yesexpr(_vm: &VirtualMachine) -> i32 {
        unsafe {
            yesexpr()
        }
    }

    unsafe fn copy_grouping(group: *const libc::c_char, vm: &VirtualMachine) -> PyListRef {
        let mut group_vec: Vec<PyObjectRef> = Vec::new();
        if group.is_null() {
            return vm.ctx.new_list(group_vec);
        }

        let mut ptr = group;
        while ![0_i8, libc::c_char::MAX].contains(&*ptr) {
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
    fn localeconv(vm: &VirtualMachine) -> PyResult<PyDictRef> {
        let result = vm.ctx.new_dict();

        unsafe {
            let lc = _localeconv();

            macro_rules! set_string_field {
                ($field:ident) => {{
                    result.set_item(
                        stringify!($field),
                        pystr_from_raw_cstr(vm, (*lc).$field)?,
                        vm,
                    )?
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

    #[derive(FromArgs)]
    struct LocaleArgs {
        #[pyarg(any)]
        category: i32,
        #[pyarg(any, optional)]
        locale: OptionalArg<Option<PyStrRef>>,
    }

    #[pyfunction]
    fn setlocale(args: LocaleArgs, vm: &VirtualMachine) -> PyResult {
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
                let error = error(vm);
                return Err(vm.new_exception_msg(error, String::from("unsupported locale setting")));
            }
            pystr_from_raw_cstr(vm, result)
        }
    }
}
