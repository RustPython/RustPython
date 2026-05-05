// spell-checker:ignore abday abmon yesexpr noexpr CRNCYSTR RADIXCHAR AMPM THOUSEP

pub(crate) use _locale::module_def;

#[pymodule]
mod _locale {
    use alloc::ffi::CString;
    use rustpython_host_env::locale as host_locale;
    use rustpython_vm::{
        PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyDictRef, PyIntRef, PyListRef, PyTypeRef, PyUtf8StrRef},
        convert::ToPyException,
        function::OptionalArg,
    };

    #[cfg(all(
        unix,
        not(any(target_os = "ios", target_os = "android", target_os = "redox"))
    ))]
    #[pyattr]
    use libc::{
        ABDAY_1, ABDAY_2, ABDAY_3, ABDAY_4, ABDAY_5, ABDAY_6, ABDAY_7, ABMON_1, ABMON_2, ABMON_3,
        ABMON_4, ABMON_5, ABMON_6, ABMON_7, ABMON_8, ABMON_9, ABMON_10, ABMON_11, ABMON_12,
        ALT_DIGITS, AM_STR, CODESET, CRNCYSTR, D_FMT, D_T_FMT, DAY_1, DAY_2, DAY_3, DAY_4, DAY_5,
        DAY_6, DAY_7, ERA, ERA_D_FMT, ERA_D_T_FMT, ERA_T_FMT, MON_1, MON_2, MON_3, MON_4, MON_5,
        MON_6, MON_7, MON_8, MON_9, MON_10, MON_11, MON_12, NOEXPR, PM_STR, RADIXCHAR, T_FMT,
        T_FMT_AMPM, THOUSEP, YESEXPR,
    };

    #[cfg(all(unix, not(any(target_os = "ios", target_os = "redox"))))]
    #[pyattr]
    use libc::LC_MESSAGES;

    #[pyattr]
    use libc::{LC_ALL, LC_COLLATE, LC_CTYPE, LC_MONETARY, LC_NUMERIC, LC_TIME};

    #[pyattr(name = "CHAR_MAX")]
    fn char_max(vm: &VirtualMachine) -> PyIntRef {
        vm.ctx.new_int(libc::c_char::MAX)
    }

    fn copy_grouping(group: &[libc::c_char], vm: &VirtualMachine) -> PyListRef {
        let mut group_vec: Vec<PyObjectRef> = Vec::new();
        for &value in group {
            let val = vm.ctx.new_int(value);
            group_vec.push(val.into());
        }
        // https://github.com/python/cpython/blob/677320348728ce058fa3579017e985af74a236d4/Modules/_localemodule.c#L80
        if !group_vec.is_empty() {
            group_vec.push(vm.ctx.new_int(0).into());
        }
        vm.ctx.new_list(group_vec)
    }

    fn pystr_from_bytes(vm: &VirtualMachine, bytes: &[u8]) -> PyResult {
        // Fast path: ASCII/UTF-8
        if let Ok(s) = core::str::from_utf8(bytes) {
            return Ok(vm.new_pyobj(s));
        }

        // On Windows, locale strings use the ANSI code page encoding
        #[cfg(windows)]
        {
            if let Some(decoded) = host_locale::decode_ansi_bytes(bytes) {
                return Ok(vm.new_pyobj(decoded));
            }
        }

        Ok(vm.new_pyobj(String::from_utf8_lossy(bytes).into_owned()))
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
    fn strcoll(string1: PyUtf8StrRef, string2: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult {
        let cstr1 = CString::new(string1.as_str()).map_err(|e| e.to_pyexception(vm))?;
        let cstr2 = CString::new(string2.as_str()).map_err(|e| e.to_pyexception(vm))?;
        Ok(vm.new_pyobj(host_locale::strcoll(&cstr1, &cstr2)))
    }

    #[pyfunction]
    fn strxfrm(string: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult {
        // https://github.com/python/cpython/blob/eaae563b6878aa050b4ad406b67728b6b066220e/Modules/_localemodule.c#L390-L442
        let n1 = string.byte_len() + 1;
        let cstr = CString::new(string.as_str()).map_err(|e| e.to_pyexception(vm))?;
        let buff = host_locale::strxfrm(&cstr, n1);
        Ok(vm.new_pyobj(String::from_utf8(buff).expect("strxfrm returned invalid utf-8 string")))
    }

    #[pyfunction]
    fn localeconv(vm: &VirtualMachine) -> PyResult<PyDictRef> {
        let result = vm.ctx.new_dict();
        let lc = host_locale::localeconv_data();

        macro_rules! set_string_field {
            ($lc:expr, $field:ident) => {{ result.set_item(stringify!($field), pystr_from_bytes(vm, &$lc.$field)?, vm)? }};
        }

        macro_rules! set_int_field {
            ($lc:expr, $field:ident) => {{ result.set_item(stringify!($field), vm.new_pyobj($lc.$field), vm)? }};
        }

        macro_rules! set_group_field {
            ($lc:expr, $field:ident) => {{
                result.set_item(
                    stringify!($field),
                    copy_grouping(&$lc.$field, vm).into(),
                    vm,
                )?
            }};
        }

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
        Ok(result)
    }

    #[derive(FromArgs)]
    struct LocaleArgs {
        #[pyarg(any)]
        category: i32,
        #[pyarg(any, optional)]
        locale: OptionalArg<Option<PyUtf8StrRef>>,
    }

    /// Maximum code page encoding name length on Windows
    #[cfg(windows)]
    const MAX_CP_LEN: usize = 15;

    /// Check if the encoding part of a locale string is too long (Windows only)
    #[cfg(windows)]
    fn check_locale_name(locale: &str) -> bool {
        if let Some(dot_pos) = locale.find('.') {
            let encoding_part = &locale[dot_pos + 1..];
            // Find the end of encoding (could be followed by '@' modifier)
            let encoding_len = encoding_part.find('@').unwrap_or(encoding_part.len());
            encoding_len <= MAX_CP_LEN
        } else {
            true
        }
    }

    /// Check locale names for LC_ALL (handles semicolon-separated locales)
    #[cfg(windows)]
    fn check_locale_name_all(locale: &str) -> bool {
        for part in locale.split(';') {
            if !check_locale_name(part) {
                return false;
            }
        }
        true
    }

    #[pyfunction]
    fn setlocale(args: LocaleArgs, vm: &VirtualMachine) -> PyResult {
        let error = error(vm);
        if cfg!(windows) && (args.category < LC_ALL || args.category > LC_TIME) {
            return Err(vm.new_exception_msg(error, "unsupported locale setting".into()));
        }
        let result = match args.locale.flatten() {
            None => host_locale::setlocale(args.category, None),
            Some(locale) => {
                let locale_str = locale.as_str();
                #[cfg(windows)]
                {
                    let valid = if args.category == LC_ALL {
                        check_locale_name_all(locale_str)
                    } else {
                        check_locale_name(locale_str)
                    };
                    if !valid {
                        return Err(
                            vm.new_exception_msg(error, "unsupported locale setting".into())
                        );
                    }
                }
                let c_locale: CString =
                    CString::new(locale_str).map_err(|e| e.to_pyexception(vm))?;
                host_locale::setlocale(args.category, Some(&c_locale))
            }
        };
        let Some(result) = result else {
            return Err(vm.new_exception_msg(error, "unsupported locale setting".into()));
        };
        pystr_from_bytes(vm, &result)
    }

    /// Get the current locale encoding.
    #[pyfunction]
    fn getencoding() -> String {
        #[cfg(windows)]
        {
            format!("cp{}", host_locale::acp())
        }
        #[cfg(not(windows))]
        {
            #[cfg(all(
                unix,
                not(any(target_os = "ios", target_os = "android", target_os = "redox"))
            ))]
            {
                if let Some(codeset) = host_locale::nl_langinfo_codeset()
                    && let Ok(s) = core::str::from_utf8(&codeset)
                    && !s.is_empty()
                {
                    return s.to_string();
                }
                "UTF-8".to_string()
            }
            #[cfg(any(target_os = "ios", target_os = "android", target_os = "redox"))]
            {
                "UTF-8".to_string()
            }
        }
    }
}
