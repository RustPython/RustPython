pub(crate) use _locale::make_module;

#[pymodule]
mod _locale {
    extern crate libc;
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

    use libc::c_char;
    use std::ffi::CStr;

    #[pyattr(name = "CHAR_MAX")]
    fn char_max(vm: &VirtualMachine) -> PyIntRef {
        vm.ctx.new_int(libc::c_char::MAX)
    }

    unsafe fn _get_grouping(group: *mut c_char, vm: &VirtualMachine) -> PyListRef {
        let mut group_vec: Vec<PyObjectRef> = Vec::new();
        let mut ptr = group;

        while *ptr != (u8::MIN as i8) && *ptr != i8::MAX {
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
            .map(|s| s.to_owned())
            .map_err(|e| vm.new_unicode_decode_error(format!("unable to decode: {e}")))?;

        Ok(vm.new_pyobj(cstr))
    }

    #[pyfunction]
    fn localeconv(vm: &VirtualMachine) -> PyResult<PyDictRef> {
        let result = vm.ctx.new_dict();
        unsafe {
            let lc = libc::localeconv();

            let mon_grouping = (*lc).mon_grouping;
            let int_frac_digits = vm.ctx.new_int((*lc).int_frac_digits).into();
            let frac_digits = vm.ctx.new_int((*lc).frac_digits).into();
            let p_cs_precedes = vm.ctx.new_int((*lc).p_cs_precedes).into();
            let p_sep_by_space = vm.ctx.new_int((*lc).p_sep_by_space).into();
            let n_cs_precedes = vm.ctx.new_int((*lc).n_cs_precedes).into();
            let p_sign_posn = vm.ctx.new_int((*lc).p_sign_posn).into();
            let n_sign_posn = vm.ctx.new_int((*lc).n_sign_posn).into();
            let grouping = (*lc).grouping;
            let decimal_point = _parse_ptr_to_str(vm, (*lc).decimal_point)?;
            let thousands_sep = _parse_ptr_to_str(vm, (*lc).thousands_sep)?;
            let int_curr_symbol = _parse_ptr_to_str(vm, (*lc).int_curr_symbol)?;
            let currency_symbol = _parse_ptr_to_str(vm, (*lc).currency_symbol)?;
            let mon_decimal_point = _parse_ptr_to_str(vm, (*lc).mon_decimal_point)?;
            let mon_thousands_sep = _parse_ptr_to_str(vm, (*lc).mon_thousands_sep)?;
            let n_sep_by_space = vm.ctx.new_int((*lc).n_sep_by_space).into();
            let positive_sign = _parse_ptr_to_str(vm, (*lc).positive_sign)?;
            let negative_sign = _parse_ptr_to_str(vm, (*lc).negative_sign)?;

            result.set_item(
                stringify!(mon_grouping),
                _get_grouping(mon_grouping, vm).into(),
                vm,
            )?;
            result.set_item(stringify!(int_frac_digits), int_frac_digits, vm)?;
            result.set_item(stringify!(frac_digits), frac_digits, vm)?;
            result.set_item(stringify!(p_cs_precedes), p_cs_precedes, vm)?;
            result.set_item(stringify!(p_sep_by_space), p_sep_by_space, vm)?;
            result.set_item(stringify!(n_cs_precedes), n_cs_precedes, vm)?;
            result.set_item(stringify!(p_sign_posn), p_sign_posn, vm)?;
            result.set_item(stringify!(n_sign_posn), n_sign_posn, vm)?;
            result.set_item(stringify!(grouping), _get_grouping(grouping, vm).into(), vm)?;
            result.set_item(stringify!(decimal_point), decimal_point, vm)?;
            result.set_item(stringify!(thousands_sep), thousands_sep, vm)?;
            result.set_item(stringify!(int_curr_symbol), int_curr_symbol, vm)?;
            result.set_item(stringify!(currency_symbol), currency_symbol, vm)?;
            result.set_item(stringify!(mon_decimal_point), mon_decimal_point, vm)?;
            result.set_item(stringify!(mon_thousands_sep), mon_thousands_sep, vm)?;
            result.set_item(stringify!(n_sep_by_space), n_sep_by_space, vm)?;
            result.set_item(stringify!(positive_sign), positive_sign, vm)?;
            result.set_item(stringify!(negative_sign), negative_sign, vm)?;
        }
        Ok(result)
    }
}
