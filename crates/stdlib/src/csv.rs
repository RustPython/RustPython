pub(crate) use _csv::module_def;

#[pymodule]
mod _csv {
    use crate::common::lock::PyMutex;
    use crate::vm::{
        AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
        VirtualMachine,
        builtins::{PyBaseExceptionRef, PyInt, PyNone, PyStr, PyType, PyTypeRef, PyUtf8StrRef},
        function::{ArgIterable, ArgumentError, FromArgs, FuncArgs, OptionalArg},
        protocol::{PyIter, PyIterReturn},
        types::{Callable, Constructor, IterNext, Iterable, SelfIter},
    };
    use alloc::fmt;
    use csv_core::Terminator;
    use itertools::Itertools;
    use parking_lot::Mutex;
    use rustpython_common::{lock::LazyLock, wtf8::Wtf8Buf};
    use rustpython_vm::match_class;
    use std::collections::HashMap;

    #[pyattr]
    const QUOTE_MINIMAL: i32 = QuoteStyle::Minimal as i32;

    #[pyattr]
    const QUOTE_ALL: i32 = QuoteStyle::All as i32;

    #[pyattr]
    const QUOTE_NONNUMERIC: i32 = QuoteStyle::Nonnumeric as i32;

    #[pyattr]
    const QUOTE_NONE: i32 = QuoteStyle::None as i32;

    #[pyattr]
    const QUOTE_STRINGS: i32 = QuoteStyle::Strings as i32;

    #[pyattr]
    const QUOTE_NOTNULL: i32 = QuoteStyle::Notnull as i32;

    #[pyattr(name = "__version__")]
    const __VERSION__: &str = "1.0";

    #[pyattr(name = "Error", once)]
    fn error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "_csv",
            "Error",
            Some(vec![vm.ctx.exceptions.exception_type.to_owned()]),
        )
    }

    static GLOBAL_HASHMAP: LazyLock<Mutex<HashMap<String, PyDialect>>> = LazyLock::new(|| {
        let m = HashMap::new();
        Mutex::new(m)
    });
    static GLOBAL_FIELD_LIMIT: LazyLock<Mutex<isize>> = LazyLock::new(|| Mutex::new(131072));

    fn new_csv_error(vm: &VirtualMachine, msg: impl Into<Wtf8Buf>) -> PyBaseExceptionRef {
        vm.new_exception_msg(super::_csv::error(vm), msg.into())
    }

    fn new_not_utf8_error(
        vm: &VirtualMachine,
        bytes: &[u8],
        err: core::str::Utf8Error,
    ) -> PyBaseExceptionRef {
        vm.new_unicode_decode_error(
            vm.ctx.new_str("utf-8"),
            vm.ctx.new_bytes(bytes.to_vec()),
            err.valid_up_to(),
            err.error_len()
                .map_or(bytes.len(), |n| err.valid_up_to() + n),
            vm.ctx.new_str("csv not utf8"),
        )
    }

    #[pyattr]
    #[pyclass(module = "csv", name = "Dialect")]
    #[derive(Debug, PyPayload, Clone)]
    struct PyDialect {
        delimiter: u8,
        quotechar: Option<u8>,
        escapechar: Option<u8>,
        doublequote: bool,
        skipinitialspace: bool,
        lineterminator: String,
        quoting: QuoteStyle,
        strict: bool,
    }

    /// Placeholder single-byte terminator for the csv-core writer paths
    /// (`QUOTE_ALL` / `QUOTE_NONNUMERIC`). csv-core can only emit a single byte
    /// for the record terminator, but its `terminator()` call also performs
    /// essential bookkeeping — closing the final quote and emitting `""` for an
    /// empty record — that must not be bypassed. So the writer emits this
    /// sentinel byte, and `writerow` strips it and appends the real (possibly
    /// multi-character) line terminator afterwards.
    const CSV_CORE_TERMINATOR_SENTINEL: u8 = b'\n';

    impl Constructor for PyDialect {
        type Args = PyObjectRef;

        fn py_new(_cls: &Py<PyType>, obj: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let dialect = Self::try_from_object(vm, obj)?;
            validate_dialect(vm, &dialect)?;
            Ok(dialect)
        }
    }

    #[pyclass(with(Constructor))]
    impl PyDialect {
        #[pygetset]
        fn delimiter(&self, vm: &VirtualMachine) -> PyRef<PyStr> {
            vm.ctx.new_str(format!("{}", self.delimiter as char))
        }

        #[pygetset]
        fn quotechar(&self, vm: &VirtualMachine) -> Option<PyRef<PyStr>> {
            Some(vm.ctx.new_str(format!("{}", self.quotechar? as char)))
        }

        #[pygetset]
        const fn doublequote(&self) -> bool {
            self.doublequote
        }

        #[pygetset]
        const fn skipinitialspace(&self) -> bool {
            self.skipinitialspace
        }

        #[pygetset]
        fn lineterminator(&self, vm: &VirtualMachine) -> PyRef<PyStr> {
            vm.ctx.new_str(self.lineterminator.clone())
        }

        #[pygetset]
        fn quoting(&self) -> isize {
            self.quoting.into()
        }

        #[pygetset]
        fn escapechar(&self, vm: &VirtualMachine) -> Option<PyRef<PyStr>> {
            Some(vm.ctx.new_str(format!("{}", self.escapechar? as char)))
        }

        #[pygetset(name = "strict")]
        const fn get_strict(&self) -> bool {
            self.strict
        }
    }

    /// Parses the delimiter from a Python object and returns its ASCII value.
    ///
    /// This function attempts to extract the 'delimiter' attribute from the given Python object and ensures that the attribute is a single-character string. If successful, it returns the ASCII value of the character. If the attribute is not a single-character string, an error is returned.
    ///
    /// # Arguments
    ///
    /// * `vm` - A reference to the VirtualMachine, used for executing Python code and manipulating Python objects.
    /// * `obj` - A reference to the PyObjectRef from which the 'delimiter' attribute is to be parsed.
    ///
    /// # Returns
    ///
    /// If successful, returns a `PyResult<u8>` representing the ASCII value of the 'delimiter' attribute. If unsuccessful, returns a `PyResult` containing an error message.
    ///
    /// # Errors
    ///
    /// This function can return the following errors:
    ///
    /// * If the 'delimiter' attribute is not a single-character string, a type error is returned.
    /// * If the 'obj' is not of string type and does not have a 'delimiter' attribute, a type error is returned.
    fn parse_delimiter_from_obj(vm: &VirtualMachine, obj: &PyObject) -> PyResult<u8> {
        if let Ok(attr) = obj.get_attr("delimiter", vm) {
            parse_delimiter_from_obj(vm, &attr)
        } else {
            match_class!(match obj.to_owned() {
                s @ PyStr => {
                    parse_single_char(&s, |len| {
                        vm.new_type_error(format!(
                            r#""delimiter" must be a unicode character, not a string of length {len}"#
                        ))
                    })
                }
                attr => {
                    Err(vm.new_type_error(format!(
                        r#""delimiter" must be a unicode character, not {}"#,
                        attr.class().name()
                    )))
                }
            })
        }
    }

    fn parse_quotechar_from_obj(vm: &VirtualMachine, obj: &PyObject) -> PyResult<Option<u8>> {
        match_class!(match obj.get_attr("quotechar", vm)? {
            s @ PyStr => {
                Ok(Some(parse_single_char(&s, |len| {
                    new_csv_error(
                        vm,
                        format!(
                            r#""quotechar" must be a unicode character or None, not a string of length {len}"#
                        ),
                    )
                })?))
            }
            _n @ PyNone => {
                Ok(None)
            }
            attr => {
                Err(new_csv_error(
                    vm,
                    format!(
                        r#""quotechar" must be a unicode character or None, not {}"#,
                        attr.class().name()
                    ),
                ))
            }
        })
    }

    fn parse_escapechar_from_obj(vm: &VirtualMachine, obj: &PyObject) -> PyResult<Option<u8>> {
        match_class!(match obj.get_attr("escapechar", vm)? {
            s @ PyStr => {
                Ok(Some(parse_single_char(&s, |len| {
                    new_csv_error(
                        vm,
                        format!(
                            r#""escapechar" must be a unicode character or None, not a string of length {len}"#
                        ),
                    )
                })?))
            }
            _n @ PyNone => {
                Ok(None)
            }
            attr => {
                Err(vm.new_type_error(format!(
                    r#""escapechar" must be a unicode character or None, not {}"#,
                    attr.class().name()
                )))
            }
        })
    }

    fn parse_lineterminator<'a>(vm: &VirtualMachine, s: &'a PyStr) -> PyResult<&'a str> {
        s.to_str()
            .ok_or_else(|| new_csv_error(vm, r#""lineterminator" must be a string"#))
    }

    fn prase_lineterminator_from_obj(vm: &VirtualMachine, obj: &PyObject) -> PyResult<String> {
        match_class!(match obj.get_attr("lineterminator", vm)? {
            s @ PyStr => {
                // Store the full line terminator string. CPython accepts an
                // arbitrary-length terminator; the manual writer paths emit it
                // verbatim and the csv-core writer path appends it after a
                // sentinel terminator (see `writerow`).
                let value = parse_lineterminator(vm, &s)?;
                Ok(value.to_owned())
            }
            attr => {
                Err(vm.new_type_error(format!(
                    r#""lineterminator" must be a string, not {}"#,
                    attr.class().name()
                )))
            }
        })
    }

    fn parse_single_char(
        s: &Py<PyStr>,
        error: impl Fn(usize) -> PyBaseExceptionRef,
    ) -> PyResult<u8> {
        let ch = s
            .as_wtf8()
            .code_points()
            .exactly_one()
            .map_err(|_| error(s.char_len()))?;
        u8::try_from(ch.to_u32()).map_err(|_| error(s.char_len()))
    }

    fn prase_quoting_from_obj(vm: &VirtualMachine, obj: &PyObject) -> PyResult<QuoteStyle> {
        match_class!(match obj.get_attr("quoting", vm)? {
            i @ PyInt => {
                Ok(i.try_to_primitive::<isize>(vm)?
                    .try_into()
                    .map_err(|_| vm.new_type_error(r#"bad "quoting" value"#))?)
            }
            attr => {
                Err(vm.new_type_error(format!(
                    r#""quoting" must be string or None, not {}"#,
                    attr.class().name()
                )))
            }
        })
    }

    impl TryFromObject for PyDialect {
        fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            let delimiter = parse_delimiter_from_obj(vm, &obj)?;
            let quotechar = parse_quotechar_from_obj(vm, &obj)?;
            let escapechar = parse_escapechar_from_obj(vm, &obj)?;
            let doublequote = obj.get_attr("doublequote", vm)?.try_to_bool(vm)?;
            let skipinitialspace = obj.get_attr("skipinitialspace", vm)?.try_to_bool(vm)?;
            let lineterminator = prase_lineterminator_from_obj(vm, &obj)?;
            let quoting = prase_quoting_from_obj(vm, &obj)?;

            let strict = if let Ok(t) = obj.get_attr("strict", vm) {
                t.try_to_bool(vm).unwrap_or(false)
            } else {
                false
            };

            Ok(Self {
                delimiter,
                quotechar,
                escapechar,
                doublequote,
                skipinitialspace,
                lineterminator,
                quoting,
                strict,
            })
        }
    }

    #[pyfunction]
    fn register_dialect(
        name: PyObjectRef,
        dialect: OptionalArg<PyObjectRef>,
        opts: FormatOptions,
        // TODO: handle quote style, etc
        mut _rest: FuncArgs,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let name = name
            .downcast::<PyStr>()
            .map_err(|_| vm.new_type_error("argument 0 must be a string"))?;

        let name: PyUtf8StrRef = name.try_into_utf8(vm)?;

        let dialect = match dialect {
            OptionalArg::Present(d) => PyDialect::try_from_object(vm, d)
                .map_err(|_| vm.new_type_error("argument 1 must be a dialect object"))?,
            OptionalArg::Missing => opts.result(vm)?,
        };

        let dialect = opts.update_py_dialect(dialect);
        validate_dialect(vm, &dialect)?;
        GLOBAL_HASHMAP
            .lock()
            .insert(name.as_str().to_owned(), dialect);

        Ok(())
    }

    #[pyfunction]
    fn get_dialect(
        name: PyObjectRef,
        mut _rest: FuncArgs,
        vm: &VirtualMachine,
    ) -> PyResult<PyDialect> {
        let name = name.downcast::<PyStr>().map_err(|obj| {
            new_csv_error(
                vm,
                format!("argument 0 must be a string, not '{}'", obj.class().name()),
            )
        })?;

        let name: PyUtf8StrRef = name.try_into_utf8(vm)?;
        let g = GLOBAL_HASHMAP.lock();

        if let Some(dialect) = g.get(name.as_str()) {
            return Ok(dialect.clone());
        }

        Err(new_csv_error(vm, "unknown dialect"))
    }

    #[pyfunction]
    fn unregister_dialect(
        name: PyObjectRef,
        mut _rest: FuncArgs,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let name = name.downcast::<PyStr>().map_err(|obj| {
            new_csv_error(
                vm,
                format!("argument 0 must be a string, not '{}'", obj.class().name()),
            )
        })?;

        let name: PyUtf8StrRef = name.try_into_utf8(vm)?;
        let mut g = GLOBAL_HASHMAP.lock();

        if let Some(_removed) = g.remove(name.as_str()) {
            return Ok(());
        }

        Err(new_csv_error(vm, "unknown dialect"))
    }

    #[pyfunction]
    fn list_dialects(
        rest: FuncArgs,
        vm: &VirtualMachine,
    ) -> PyResult<rustpython_vm::builtins::PyListRef> {
        if !rest.args.is_empty() || !rest.kwargs.is_empty() {
            return Err(vm.new_type_error("too many argument"));
        }
        let g = GLOBAL_HASHMAP.lock();
        let t = g
            .keys()
            .cloned()
            .map(|x| vm.ctx.new_str(x).into())
            .collect_vec();
        // .iter().map(|x| vm.ctx.new_str(x.clone()).into_pyobject(vm)).collect_vec();
        Ok(vm.ctx.new_list(t))
    }

    #[pyfunction]
    fn field_size_limit(rest: FuncArgs, vm: &VirtualMachine) -> PyResult<isize> {
        let old_size = GLOBAL_FIELD_LIMIT.lock().to_owned();
        if !rest.args.is_empty() {
            let arg_len = rest.args.len();
            if arg_len != 1 {
                return Err(vm.new_type_error(format!(
                    "field_size_limit() takes at most 1 argument ({arg_len} given)"
                )));
            }
            let Ok(new_size) = rest.args.first().unwrap().try_int(vm) else {
                return Err(vm.new_type_error("limit must be an integer"));
            };
            *GLOBAL_FIELD_LIMIT.lock() = new_size.try_to_primitive::<isize>(vm)?;
        }
        Ok(old_size)
    }

    #[pyfunction]
    fn reader(
        iter: PyIter,
        options: FormatOptions,
        // TODO: handle quote style, etc
        _rest: FuncArgs,
        vm: &VirtualMachine,
    ) -> PyResult<Reader> {
        let dialect = options.result(vm)?;
        Ok(Reader {
            iter,
            state: PyMutex::new(ReadState {
                line_num: 0,
                generation: 0,
            }),
            dialect,
        })
    }

    #[pyfunction]
    fn writer(
        file: PyObjectRef,
        options: FormatOptions,
        // TODO: handle quote style, etc
        _rest: FuncArgs,
        vm: &VirtualMachine,
    ) -> PyResult<Writer> {
        let write = match vm.get_attribute_opt(file.clone(), "write")? {
            Some(write_meth) => write_meth,
            None if file.is_callable() => file,
            None => {
                return Err(vm.new_type_error(r#"argument 1 must have a "write" method"#));
            }
        };
        let dialect = options.result(vm)?;

        Ok(Writer {
            write,
            state: PyMutex::new(WriteState {
                buffer: vec![0; 1024],
                writer: FormatOptions::to_writer(&dialect),
            }),
            dialect,
        })
    }

    #[inline]
    fn resize_buf<T: num_traits::PrimInt>(buf: &mut Vec<T>) {
        let new_size = buf.len() * 2;
        buf.resize(new_size, T::zero());
    }

    #[repr(i32)]
    #[derive(Debug, Clone, Copy, Eq, PartialEq)]
    pub enum QuoteStyle {
        Minimal = 0,
        All = 1,
        Nonnumeric = 2,
        None = 3,
        Strings = 4,
        Notnull = 5,
    }

    impl From<QuoteStyle> for csv_core::QuoteStyle {
        fn from(val: QuoteStyle) -> Self {
            match val {
                QuoteStyle::Minimal => Self::Necessary,
                QuoteStyle::All => Self::Always,
                QuoteStyle::Nonnumeric => Self::NonNumeric,
                QuoteStyle::None => Self::Never,
                QuoteStyle::Strings | QuoteStyle::Notnull => Self::Necessary,
            }
        }
    }

    impl TryFromObject for QuoteStyle {
        fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            let num = obj.try_int(vm)?.try_to_primitive::<isize>(vm)?;
            num.try_into().map_err(|_| {
                vm.new_value_error("can not convert to QuoteStyle enum from input argument")
            })
        }
    }

    impl TryFrom<isize> for QuoteStyle {
        type Error = ();

        fn try_from(num: isize) -> Result<Self, Self::Error> {
            Ok(match num {
                0 => Self::Minimal,
                1 => Self::All,
                2 => Self::Nonnumeric,
                3 => Self::None,
                4 => Self::Strings,
                5 => Self::Notnull,
                _ => return Err(()),
            })
        }
    }

    impl From<QuoteStyle> for isize {
        fn from(val: QuoteStyle) -> Self {
            match val {
                QuoteStyle::Minimal => 0,
                QuoteStyle::All => 1,
                QuoteStyle::Nonnumeric => 2,
                QuoteStyle::None => 3,
                QuoteStyle::Strings => 4,
                QuoteStyle::Notnull => 5,
            }
        }
    }

    #[derive(Default)]
    enum DialectItem {
        Str(String),
        Obj(PyDialect),
        #[default]
        None,
    }

    #[derive(Default)]
    struct FormatOptions {
        dialect: DialectItem,
        delimiter: Option<u8>,
        quotechar: Option<Option<u8>>,
        escapechar: Option<Option<u8>>,
        doublequote: Option<bool>,
        skipinitialspace: Option<bool>,
        lineterminator: Option<String>,
        quoting: Option<QuoteStyle>,
        strict: Option<bool>,
    }

    /// prase a dialect item from a Python argument and returns a `DialectItem` or an `ArgumentError`.
    ///
    /// This function takes a reference to the VirtualMachine and a PyObjectRef as input and attempts to parse a dialect item from the provided Python argument. It returns a `DialectItem` if successful, or an `ArgumentError` if unsuccessful.
    ///
    /// # Arguments
    ///
    /// * `vm` - A reference to the VirtualMachine, used for executing Python code and manipulating Python objects.
    /// * `obj` - The PyObjectRef from which the dialect item is to be parsed.
    ///
    /// # Returns
    ///
    /// If successful, returns a `Result<DialectItem, ArgumentError>` representing the parsed dialect item. If unsuccessful, returns an `ArgumentError`.
    ///
    /// # Errors
    ///
    /// This function can return the following errors:
    ///
    /// * If the provided object is a PyStr, it returns a `DialectItem::Str` containing the string value.
    /// * If the provided object is PyNone, it returns an `ArgumentError` with the message "InvalidKeywordArgument('dialect')".
    /// * If the provided object is a PyType, it attempts to create a PyDialect from the object and returns a `DialectItem::Obj` containing the PyDialect if successful. If unsuccessful, it returns an `ArgumentError` with the message "InvalidKeywordArgument('dialect')".
    /// * If the provided object is none of the above types, it attempts to create a PyDialect from the object and returns a `DialectItem::Obj` containing the PyDialect if successful. If unsuccessful, it returns an `ArgumentError` with the message "InvalidKeywordArgument('dialect')".
    fn prase_dialect_item_from_arg(
        vm: &VirtualMachine,
        obj: PyObjectRef,
    ) -> Result<DialectItem, ArgumentError> {
        match_class!(match obj {
            s @ PyStr => {
                let s = s.try_into_utf8(vm).map_err(ArgumentError::Exception)?;
                Ok(DialectItem::Str(s.as_str().to_owned()))
            }
            PyNone => {
                Err(ArgumentError::InvalidKeywordArgument("dialect".to_string()))
            }
            t @ PyType => {
                let temp = t
                    .as_object()
                    .call(vec![], vm)
                    .map_err(|_e| ArgumentError::InvalidKeywordArgument("dialect".to_string()))?;
                Ok(DialectItem::Obj(
                    PyDialect::try_from_object(vm, temp).map_err(|_| {
                        ArgumentError::InvalidKeywordArgument("dialect".to_string())
                    })?,
                ))
            }
            obj => {
                if let Ok(cur_dialect_item) = PyDialect::try_from_object(vm, obj) {
                    Ok(DialectItem::Obj(cur_dialect_item))
                } else {
                    Err(ArgumentError::InvalidKeywordArgument("dialect".to_string()))
                }
            }
        })
    }

    impl FromArgs for FormatOptions {
        fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
            let dialect = if let Some(dialect) = args.kwargs.swap_remove("dialect") {
                prase_dialect_item_from_arg(vm, dialect)?
            } else if let Some(dialect) = args.args.first() {
                prase_dialect_item_from_arg(vm, dialect.clone())?
            } else {
                DialectItem::None
            };

            let mut res = Self {
                dialect,
                ..Default::default()
            };

            if let Some(delimiter) = args.kwargs.swap_remove("delimiter") {
                res.delimiter = Some(parse_delimiter_from_obj(vm, &delimiter)?);
            }

            if let Some(escapechar) = args.kwargs.swap_remove("escapechar") {
                res.escapechar = match_class!(match escapechar {
                    s @ PyStr => Some(Some(parse_single_char(&s, |_| {
                        vm.new_type_error(r#""escapechar" must be a 1-character string"#)
                    })?)),
                    PyNone => Some(None),
                    _ => {
                        return Err(ArgumentError::Exception(
                            vm.new_type_error(r#""escapechar" must be a 1-character string"#),
                        ));
                    }
                })
            };

            if let Some(lineterminator) = args.kwargs.swap_remove("lineterminator") {
                let s = lineterminator.downcast_ref::<PyStr>().ok_or_else(|| {
                    vm.new_type_error(format!(
                        r#""lineterminator" must be a string, not {}"#,
                        lineterminator.class().name()
                    ))
                })?;
                let value = parse_lineterminator(vm, s)?;
                res.lineterminator = Some(value.to_owned());
            };

            if let Some(doublequote) = args.kwargs.swap_remove("doublequote") {
                res.doublequote = Some(
                    doublequote
                        .try_to_bool(vm)
                        .map_err(|_| vm.new_type_error(r#""doublequote" must be a bool"#))?,
                )
            };

            if let Some(skipinitialspace) = args.kwargs.swap_remove("skipinitialspace") {
                res.skipinitialspace = Some(
                    skipinitialspace
                        .try_to_bool(vm)
                        .map_err(|_| vm.new_type_error(r#""skipinitialspace" must be a bool"#))?,
                )
            };

            if let Some(quoting) = args.kwargs.swap_remove("quoting") {
                res.quoting = match_class!(match quoting {
                    i @ PyInt =>
                        Some(i.try_to_primitive::<isize>(vm)?.try_into().map_err(|_e| {
                            ArgumentError::InvalidKeywordArgument("quoting".to_string())
                        })?),
                    _ => {
                        // let msg = r#""quoting" must be a int enum"#;
                        return Err(ArgumentError::InvalidKeywordArgument("quoting".to_string()));
                    }
                });
            };

            if let Some(quotechar) = args.kwargs.swap_remove("quotechar") {
                res.quotechar = match_class!(match quotechar {
                    s @ PyStr => Some(Some(parse_single_char(&s, |_| {
                        vm.new_type_error(r#""quotechar" must be a 1-character string"#)
                    })?)),
                    PyNone => {
                        if res
                            .quoting
                            .is_some_and(|quoting| quoting != QuoteStyle::None)
                        {
                            return Err(ArgumentError::Exception(
                                vm.new_type_error("quotechar must be set if quoting enabled"),
                            ));
                        }
                        Some(None)
                    }
                    _o => {
                        return Err(
                            rustpython_vm::function::ArgumentError::InvalidKeywordArgument(
                                "quotechar".to_string(),
                            ),
                        );
                    }
                })
            };

            if let Some(strict) = args.kwargs.swap_remove("strict") {
                res.strict = Some(
                    strict
                        .try_to_bool(vm)
                        .map_err(|_| vm.new_type_error(r#""strict" must be a int enum"#))?,
                )
            };

            if let Some(last_arg) = args.kwargs.pop() {
                // The dialect is parsed by a parser of its own, which has no
                // name to give the message.
                return Err(ArgumentError::Exception(
                    vm.new_unexpected_keyword_type_error(None, &last_arg.0.to_string()),
                ));
            }

            Ok(res)
        }
    }

    fn validate_dialect(vm: &VirtualMachine, dialect: &PyDialect) -> PyResult<()> {
        let special = |name: &str, value: u8| {
            if matches!(value, b'\r' | b'\n') {
                Err(vm.new_value_error(format!(
                    "{name} must be a single character, not a line break"
                )))
            } else {
                Ok(())
            }
        };

        special("delimiter", dialect.delimiter)?;
        if let Some(quotechar) = dialect.quotechar {
            special("quotechar", quotechar)?;
        }
        if let Some(escapechar) = dialect.escapechar {
            special("escapechar", escapechar)?;
        }

        if dialect.skipinitialspace
            && (matches!(dialect.escapechar, Some(b' ')) || matches!(dialect.quotechar, Some(b' ')))
        {
            return Err(vm.new_value_error(
                "escapechar or quotechar cannot be a space when skipinitialspace is enabled",
            ));
        }

        let values: [(&str, Option<u8>); 3] = [
            ("delimiter", Some(dialect.delimiter)),
            ("quotechar", dialect.quotechar),
            ("escapechar", dialect.escapechar),
        ];
        for (index, (left_name, left)) in values.iter().enumerate() {
            for (right_name, right) in values.iter().skip(index + 1) {
                if left.is_some() && left == right {
                    return Err(vm.new_value_error(format!(
                        "{left_name} and {right_name} cannot be the same"
                    )));
                }
            }
            if left.is_some_and(|value| {
                dialect
                    .lineterminator
                    .chars()
                    .any(|character| character == value as char)
            }) {
                return Err(vm.new_value_error(format!(
                    "{left_name} and lineterminator cannot be the same"
                )));
            }
        }
        Ok(())
    }

    impl FormatOptions {
        fn update_py_dialect(&self, mut res: PyDialect) -> PyDialect {
            macro_rules! check_and_fill {
                ($res:ident, $e:ident) => {{
                    if let Some(t) = self.$e {
                        $res.$e = t;
                    }
                }};
            }

            check_and_fill!(res, delimiter);
            // check_and_fill!(res, quotechar);
            check_and_fill!(res, delimiter);
            check_and_fill!(res, doublequote);
            check_and_fill!(res, skipinitialspace);

            if let Some(t) = self.escapechar {
                res.escapechar = t;
            };

            if let Some(t) = self.quotechar {
                res.quotechar = t;
            };

            check_and_fill!(res, quoting);
            if let Some(t) = &self.lineterminator {
                res.lineterminator.clone_from(t);
            };
            check_and_fill!(res, strict);
            res
        }

        fn result(&self, vm: &VirtualMachine) -> PyResult<PyDialect> {
            let dialect = match &self.dialect {
                DialectItem::Str(name) => {
                    let g = GLOBAL_HASHMAP.lock();
                    if let Some(dialect) = g.get(name) {
                        Ok(self.update_py_dialect(dialect.clone()))
                    } else {
                        Err(new_csv_error(vm, format!("{name} is not registered.")))
                    }
                    // TODO: Maybe need to update the obj from HashMap
                }
                DialectItem::Obj(o) => Ok(self.update_py_dialect(o.clone())),
                DialectItem::None => Ok(self.update_py_dialect(PyDialect {
                    delimiter: b',',
                    quotechar: Some(b'"'),
                    escapechar: None,
                    doublequote: true,
                    skipinitialspace: false,
                    lineterminator: "\r\n".to_owned(),
                    quoting: QuoteStyle::Minimal,
                    strict: false,
                })),
            }?;
            validate_dialect(vm, &dialect)?;
            Ok(dialect)
        }

        fn to_writer(dialect: &PyDialect) -> csv_core::Writer {
            let mut builder = csv_core::WriterBuilder::new();
            let mut writer = builder
                .delimiter(dialect.delimiter)
                .double_quote(dialect.doublequote);

            if let Some(t) = dialect.quotechar {
                writer = writer.quote(t);
            }

            writer = writer.terminator(Terminator::Any(CSV_CORE_TERMINATOR_SENTINEL));

            if let Some(e) = dialect.escapechar {
                writer = writer.escape(e);
            }

            writer = writer.quote_style(dialect.quoting.into());

            writer.build()
        }
    }

    struct ReadState {
        line_num: u64,
        generation: u64,
    }

    #[pyclass(no_attr, module = "_csv", name = "reader", traverse)]
    #[derive(PyPayload)]
    pub(super) struct Reader {
        iter: PyIter,
        #[pytraverse(skip)]
        state: PyMutex<ReadState>,
        #[pytraverse(skip)]
        dialect: PyDialect,
    }

    impl fmt::Debug for Reader {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "_csv.reader")
        }
    }

    #[pyclass(with(IterNext, Iterable), flags(DISALLOW_INSTANTIATION))]
    impl Reader {
        #[pygetset]
        fn line_num(&self) -> u64 {
            self.state.lock().line_num
        }

        #[pygetset]
        fn dialect(&self, _vm: &VirtualMachine) -> PyDialect {
            self.dialect.clone()
        }
    }

    impl SelfIter for Reader {}

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum ParserState {
        StartRecord,
        StartField,
        EscapedChar,
        InField,
        InQuotedField,
        EscapeInQuotedField,
        QuoteInQuotedField,
        EatCrnl,
        AfterEscapedCrnl,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum ParserInput {
        Byte(u8),
        Eol,
    }

    const EOL: ParserInput = ParserInput::Eol;

    struct CsvParser {
        state: ParserState,
        fields: Vec<PyObjectRef>,
        field: Vec<u8>,
        unquoted_field: bool,
        field_limit: isize,
    }

    impl CsvParser {
        fn new(field_limit: isize) -> Self {
            Self {
                state: ParserState::StartRecord,
                fields: Vec::new(),
                field: Vec::new(),
                unquoted_field: false,
                field_limit,
            }
        }

        fn into_result(self, vm: &VirtualMachine) -> PyIterReturn {
            PyIterReturn::Return(vm.ctx.new_list(self.fields).into())
        }

        fn add_byte(&mut self, byte: u8, vm: &VirtualMachine) -> PyResult<()> {
            if self.field_limit < 0 || self.field.len() >= self.field_limit as usize {
                return Err(new_csv_error(
                    vm,
                    format!("field larger than field limit ({})", self.field_limit),
                ));
            }
            self.field.push(byte);
            Ok(())
        }

        fn save_field(&mut self, quoting: QuoteStyle, vm: &VirtualMachine) -> PyResult<()> {
            let field = if self.unquoted_field
                && self.field.is_empty()
                && matches!(quoting, QuoteStyle::Notnull | QuoteStyle::Strings)
            {
                vm.ctx.none()
            } else {
                let value = core::str::from_utf8(&self.field)
                    .map_err(|e| new_not_utf8_error(vm, &self.field, e))?;
                let field: PyObjectRef = vm.ctx.new_str(value).into();
                if self.unquoted_field
                    && !self.field.is_empty()
                    && matches!(quoting, QuoteStyle::Nonnumeric | QuoteStyle::Strings)
                {
                    PyType::call(vm.ctx.types.float_type, vec![field].into(), vm)?
                } else {
                    field
                }
            };
            self.fields.push(field);
            self.field.clear();
            Ok(())
        }

        fn process_parser_input(
            &mut self,
            input: ParserInput,
            dialect: &PyDialect,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            match self.state {
                ParserState::StartRecord => match input {
                    ParserInput::Eol => {}
                    ParserInput::Byte(b'\r' | b'\n') => self.state = ParserState::EatCrnl,
                    _ => {
                        self.state = ParserState::StartField;
                        return self.process_parser_input(input, dialect, vm);
                    }
                },
                ParserState::StartField => {
                    self.unquoted_field = true;
                    match input {
                        ParserInput::Eol | ParserInput::Byte(b'\r' | b'\n') => {
                            self.save_field(dialect.quoting, vm)?;
                            self.state = state_after_record_end(input);
                        }
                        ParserInput::Byte(byte)
                            if dialect.quoting != QuoteStyle::None
                                && dialect.quotechar == Some(byte) =>
                        {
                            self.unquoted_field = false;
                            self.state = ParserState::InQuotedField;
                        }
                        ParserInput::Byte(byte) if dialect.escapechar == Some(byte) => {
                            self.state = ParserState::EscapedChar;
                        }
                        ParserInput::Byte(b' ') if dialect.skipinitialspace => {}
                        ParserInput::Byte(byte) if byte == dialect.delimiter => {
                            self.save_field(dialect.quoting, vm)?;
                        }
                        ParserInput::Byte(byte) => {
                            self.add_byte(byte, vm)?;
                            self.state = ParserState::InField;
                        }
                    }
                }
                ParserState::EscapedChar => match input {
                    ParserInput::Byte(byte @ (b'\r' | b'\n')) => {
                        self.add_byte(byte, vm)?;
                        self.state = ParserState::AfterEscapedCrnl;
                    }
                    ParserInput::Eol => {
                        self.add_byte(b'\n', vm)?;
                        self.state = ParserState::InField;
                    }
                    ParserInput::Byte(byte) => {
                        self.add_byte(byte, vm)?;
                        self.state = ParserState::InField;
                    }
                },
                ParserState::AfterEscapedCrnl => {
                    if input != ParserInput::Eol {
                        self.state = ParserState::InField;
                        return self.process_parser_input(input, dialect, vm);
                    }
                }
                ParserState::InField => match input {
                    ParserInput::Eol | ParserInput::Byte(b'\r' | b'\n') => {
                        self.save_field(dialect.quoting, vm)?;
                        self.state = state_after_record_end(input);
                    }
                    ParserInput::Byte(byte) if dialect.escapechar == Some(byte) => {
                        self.state = ParserState::EscapedChar;
                    }
                    ParserInput::Byte(byte) if byte == dialect.delimiter => {
                        self.save_field(dialect.quoting, vm)?;
                        self.state = ParserState::StartField;
                    }
                    ParserInput::Byte(byte) => self.add_byte(byte, vm)?,
                },
                ParserState::InQuotedField => match input {
                    ParserInput::Eol => {}
                    ParserInput::Byte(byte) if dialect.escapechar == Some(byte) => {
                        self.state = ParserState::EscapeInQuotedField;
                    }
                    ParserInput::Byte(byte)
                        if dialect.quoting != QuoteStyle::None
                            && dialect.quotechar == Some(byte) =>
                    {
                        self.state = if dialect.doublequote {
                            ParserState::QuoteInQuotedField
                        } else {
                            ParserState::InField
                        };
                    }
                    ParserInput::Byte(byte) => self.add_byte(byte, vm)?,
                },
                ParserState::EscapeInQuotedField => {
                    let byte = match input {
                        ParserInput::Eol => b'\n',
                        ParserInput::Byte(byte) => byte,
                    };
                    self.add_byte(byte, vm)?;
                    self.state = ParserState::InQuotedField;
                }
                ParserState::QuoteInQuotedField => match input {
                    ParserInput::Byte(byte)
                        if dialect.quoting != QuoteStyle::None
                            && dialect.quotechar == Some(byte) =>
                    {
                        self.add_byte(byte, vm)?;
                        self.state = ParserState::InQuotedField;
                    }
                    ParserInput::Byte(byte) if byte == dialect.delimiter => {
                        self.save_field(dialect.quoting, vm)?;
                        self.state = ParserState::StartField;
                    }
                    ParserInput::Eol | ParserInput::Byte(b'\r' | b'\n') => {
                        self.save_field(dialect.quoting, vm)?;
                        self.state = state_after_record_end(input);
                    }
                    ParserInput::Byte(byte) if !dialect.strict => {
                        self.add_byte(byte, vm)?;
                        self.state = ParserState::InField;
                    }
                    ParserInput::Byte(_) => {
                        return Err(new_csv_error(
                            vm,
                            format!(
                                "'{}' expected after '{}'",
                                dialect.delimiter as char,
                                dialect.quotechar.unwrap_or_default() as char,
                            ),
                        ));
                    }
                },
                ParserState::EatCrnl => match input {
                    ParserInput::Byte(b'\r' | b'\n') => {}
                    ParserInput::Eol => self.state = ParserState::StartRecord,
                    ParserInput::Byte(_) => {
                        return Err(new_csv_error(
                            vm,
                            concat!(
                                "new-line character seen in unquoted field - ",
                                "do you need to open the file with newline=''?"
                            ),
                        ));
                    }
                },
            }
            Ok(())
        }
    }

    fn state_after_record_end(input: ParserInput) -> ParserState {
        if input == ParserInput::Eol {
            ParserState::StartRecord
        } else {
            ParserState::EatCrnl
        }
    }

    fn next_input_item(zelf: &Py<Reader>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        let generation = zelf.state.lock().generation;
        // Advancing user code may re-enter this reader, so do not hold its lock here.
        let result = zelf.iter.next(vm)?;
        let mut state = zelf.state.lock();
        if state.generation != generation {
            return Err(new_csv_error(
                vm,
                "iterator has already advanced the reader",
            ));
        }
        if matches!(result, PyIterReturn::Return(_)) {
            state.generation += 1;
        }
        Ok(result)
    }

    fn finish_at_true_eof(
        mut parser: CsvParser,
        dialect: &PyDialect,
        vm: &VirtualMachine,
    ) -> PyResult<PyIterReturn> {
        let has_unfinished_record =
            !parser.field.is_empty() || parser.state == ParserState::InQuotedField;
        if !has_unfinished_record {
            return Ok(PyIterReturn::StopIteration(None));
        }
        if dialect.strict {
            return Err(new_csv_error(vm, "unexpected end of data"));
        }
        parser.save_field(dialect.quoting, vm)?;
        Ok(parser.into_result(vm))
    }

    impl IterNext for Reader {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let mut parser = CsvParser::new(*GLOBAL_FIELD_LIMIT.lock());

            loop {
                match next_input_item(zelf, vm)? {
                    PyIterReturn::Return(obj) => {
                        let string = obj.downcast::<PyStr>().map_err(|obj| {
                            new_csv_error(
                                vm,
                                format!(
                                    concat!(
                                        "iterator should return strings, not {} ",
                                        "(the file should be opened in text mode)"
                                    ),
                                    obj.class().name()
                                ),
                            )
                        })?;

                        zelf.state.lock().line_num += 1;
                        parser.field_limit = *GLOBAL_FIELD_LIMIT.lock();
                        for &byte in string.as_bytes() {
                            parser.process_parser_input(
                                ParserInput::Byte(byte),
                                &zelf.dialect,
                                vm,
                            )?;
                        }

                        // Virtual EOL marks an iterator-item boundary, not true EOF.
                        parser.process_parser_input(EOL, &zelf.dialect, vm)?;
                        if parser.state == ParserState::StartRecord {
                            return Ok(parser.into_result(vm));
                        }
                    }
                    PyIterReturn::StopIteration(_) => {
                        return finish_at_true_eof(parser, &zelf.dialect, vm);
                    }
                }
            }
        }
    }

    struct WriteState {
        buffer: Vec<u8>,
        writer: csv_core::Writer,
    }

    #[pyclass(no_attr, module = "_csv", name = "writer", traverse)]
    #[derive(PyPayload)]
    pub(super) struct Writer {
        write: PyObjectRef,
        #[pytraverse(skip)]
        state: PyMutex<WriteState>,
        #[pytraverse(skip)]
        dialect: PyDialect,
    }

    impl fmt::Debug for Writer {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "_csv.writer")
        }
    }

    fn write_quoted_field(
        output: &mut Vec<u8>,
        data: &[u8],
        dialect: &PyDialect,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let quotechar = dialect
            .quotechar
            .ok_or_else(|| vm.new_type_error("quotechar must be set if quoting enabled"))?;
        output.push(quotechar);
        for &byte in data {
            if byte == quotechar {
                if dialect.doublequote {
                    output.push(quotechar);
                    output.push(quotechar);
                } else if let Some(escapechar) = dialect.escapechar {
                    output.push(escapechar);
                    output.push(byte);
                } else {
                    return Err(new_csv_error(vm, "need to escape, but no escapechar set"));
                }
            } else {
                if dialect.escapechar == Some(byte) {
                    output.push(byte);
                }
                output.push(byte);
            }
        }
        output.push(quotechar);
        Ok(())
    }

    fn write_unquoted_field(
        output: &mut Vec<u8>,
        data: &[u8],
        dialect: &PyDialect,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let mut data = data;
        while let Some((&byte, rest)) = data.split_first() {
            if field_needs_escape(data, dialect) {
                let escapechar = dialect
                    .escapechar
                    .ok_or_else(|| new_csv_error(vm, "need to escape, but no escapechar set"))?;
                output.push(escapechar);
            }
            output.push(byte);
            data = rest;
        }
        Ok(())
    }

    fn data_contains_lineterminator_char(data: &[u8], dialect: &PyDialect) -> bool {
        dialect.lineterminator.chars().any(|character| {
            let mut encoded = [0; 4];
            let character = character.encode_utf8(&mut encoded).as_bytes();
            data.windows(character.len())
                .any(|window| window == character)
        })
    }

    fn data_starts_with_lineterminator_char(data: &[u8], dialect: &PyDialect) -> bool {
        dialect.lineterminator.chars().any(|character| {
            let mut encoded = [0; 4];
            let character = character.encode_utf8(&mut encoded).as_bytes();
            data.starts_with(character)
        })
    }

    fn field_needs_quotes(data: &[u8], dialect: &PyDialect) -> bool {
        data.iter().any(|&byte| {
            byte == dialect.delimiter
                || dialect.quotechar == Some(byte)
                || matches!(byte, b'\r' | b'\n')
        }) || data_contains_lineterminator_char(data, dialect)
    }

    fn field_needs_escape(data: &[u8], dialect: &PyDialect) -> bool {
        let byte = data[0];
        byte == dialect.delimiter
            || dialect.quotechar == Some(byte)
            || dialect.escapechar == Some(byte)
            || matches!(byte, b'\r' | b'\n')
            || data_starts_with_lineterminator_char(data, dialect)
    }

    fn write_lineterminator(output: &mut Vec<u8>, terminator: &str) {
        output.extend_from_slice(terminator.as_bytes());
    }

    #[pyclass(flags(DISALLOW_INSTANTIATION))]
    impl Writer {
        #[pygetset(name = "dialect")]
        fn get_dialect(&self, _vm: &VirtualMachine) -> PyDialect {
            self.dialect.clone()
        }

        fn writerow_quoted_strings(&self, row: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let _state = self.state.lock();
            let row: ArgIterable = ArgIterable::try_from_object(vm, row.clone()).map_err(|_e| {
                new_csv_error(
                    vm,
                    format!("'{}' object is not iterable", row.class().name()),
                )
            })?;
            let fields = row.iter(vm)?.collect::<PyResult<Vec<_>>>()?;
            let single_field = fields.len() == 1;
            let mut output = Vec::new();

            for (index, field) in fields.into_iter().enumerate() {
                if index > 0 {
                    output.push(self.dialect.delimiter);
                }

                let stringified;
                let (data, is_str, is_none): (&[u8], bool, bool) = match_class!(match field {
                    ref s @ PyStr => (s.as_bytes(), true, false),
                    crate::builtins::PyNone => (b"", false, true),
                    ref obj => {
                        stringified = obj.str(vm)?;
                        (stringified.as_bytes(), false, false)
                    }
                });

                let should_quote = match self.dialect.quoting {
                    QuoteStyle::Strings => is_str || field_needs_quotes(data, &self.dialect),
                    QuoteStyle::Notnull => !is_none,
                    _ => unreachable!(),
                };
                if should_quote {
                    write_quoted_field(&mut output, data, &self.dialect, vm)?;
                } else if single_field && data.is_empty() {
                    return Err(new_csv_error(
                        vm,
                        "single empty field record must be quoted",
                    ));
                } else {
                    output.extend_from_slice(data);
                }
            }

            write_lineterminator(&mut output, &self.dialect.lineterminator);
            let s =
                core::str::from_utf8(&output).map_err(|e| new_not_utf8_error(vm, &output, e))?;
            self.write.call((s,), vm)
        }

        fn writerow_quote_none(&self, row: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let _state = self.state.lock();

            let row: ArgIterable = ArgIterable::try_from_object(vm, row.clone()).map_err(|_e| {
                new_csv_error(
                    vm,
                    format!("'{}' object is not iterable", row.class().name()),
                )
            })?;

            let fields = row.iter(vm)?.collect::<PyResult<Vec<_>>>()?;
            let single_field = fields.len() == 1;
            let mut output = Vec::new();

            for (index, field) in fields.into_iter().enumerate() {
                if index > 0 {
                    output.push(self.dialect.delimiter);
                }

                let stringified;
                let data: &[u8] = match_class!(match field {
                    ref s @ PyStr => s.as_bytes(),
                    crate::builtins::PyNone => b"",
                    ref obj => {
                        stringified = obj.str(vm)?;
                        stringified.as_bytes()
                    }
                });

                if single_field && data.is_empty() {
                    return Err(new_csv_error(
                        vm,
                        "single empty field record must be quoted",
                    ));
                }

                write_unquoted_field(&mut output, data, &self.dialect, vm)?;
            }

            write_lineterminator(&mut output, &self.dialect.lineterminator);

            let s =
                core::str::from_utf8(&output).map_err(|e| new_not_utf8_error(vm, &output, e))?;

            self.write.call((s,), vm)
        }

        fn writerow_minimal(&self, row: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let _state = self.state.lock();

            let row: ArgIterable = ArgIterable::try_from_object(vm, row.clone()).map_err(|_e| {
                new_csv_error(
                    vm,
                    format!("'{}' object is not iterable", row.class().name()),
                )
            })?;

            let fields = row.iter(vm)?.collect::<PyResult<Vec<_>>>()?;
            let single_field = fields.len() == 1;
            let mut output = Vec::new();

            for (index, field) in fields.into_iter().enumerate() {
                if index > 0 {
                    output.push(self.dialect.delimiter);
                }

                let stringified;
                let data: &[u8] = match_class!(match field {
                    ref s @ PyStr => s.as_bytes(),
                    crate::builtins::PyNone => b"",
                    ref obj => {
                        stringified = obj.str(vm)?;
                        stringified.as_bytes()
                    }
                });

                // CPython quotes a QUOTE_MINIMAL field if it contains the
                // delimiter, the quote character, '\r', '\n', or the line
                // terminator, regardless of which line terminator is
                // configured. A row with a single empty field is also quoted
                // so that it is not read back as an empty line.
                if field_needs_quotes(data, &self.dialect) || (single_field && data.is_empty()) {
                    write_quoted_field(&mut output, data, &self.dialect, vm)?;
                } else {
                    output.extend_from_slice(data);
                }
            }

            write_lineterminator(&mut output, &self.dialect.lineterminator);

            let s =
                core::str::from_utf8(&output).map_err(|e| new_not_utf8_error(vm, &output, e))?;

            self.write.call((s,), vm)
        }

        #[pymethod]
        fn writerow(&self, row: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            match self.dialect.quoting {
                QuoteStyle::None => return self.writerow_quote_none(row, vm),
                QuoteStyle::Strings | QuoteStyle::Notnull => {
                    return self.writerow_quoted_strings(row, vm);
                }
                QuoteStyle::Minimal => return self.writerow_minimal(row, vm),
                _ => {}
            }

            let mut state = self.state.lock();
            let WriteState { buffer, writer } = &mut *state;

            let mut buffer_offset = 0;

            macro_rules! handle_res {
                ($x:expr) => {{
                    let (res, n_written) = $x;
                    buffer_offset += n_written;
                    match res {
                        csv_core::WriteResult::InputEmpty => break,
                        csv_core::WriteResult::OutputFull => resize_buf(buffer),
                    }
                }};
            }

            let row = ArgIterable::try_from_object(vm, row.clone()).map_err(|_e| {
                new_csv_error(
                    vm,
                    format!("'{}' object is not iterable", row.class().name()),
                )
            })?;

            let mut first_flag = true;
            for field in row.iter(vm)? {
                let field: PyObjectRef = field?;
                let stringified;
                let data: &[u8] = match_class!(match field {
                    ref s @ PyStr => s.as_bytes(),
                    crate::builtins::PyNone => b"",
                    ref obj => {
                        stringified = obj.str(vm)?;
                        stringified.as_bytes()
                    }
                });
                let mut input_offset = 0;

                if first_flag {
                    first_flag = false;
                } else {
                    loop {
                        handle_res!(writer.delimiter(&mut buffer[buffer_offset..]));
                    }
                }

                loop {
                    let (res, n_read, n_written) =
                        writer.field(&data[input_offset..], &mut buffer[buffer_offset..]);
                    input_offset += n_read;
                    handle_res!((res, n_written));
                }
            }

            loop {
                handle_res!(writer.terminator(&mut buffer[buffer_offset..]));
            }

            // csv-core just emitted the single-byte sentinel terminator (after
            // closing the final quote / emitting an empty record as needed).
            // Drop that sentinel byte and append the real, possibly
            // multi-character, line terminator.
            let emitted = &buffer[..buffer_offset];
            let body = emitted
                .strip_suffix(&[CSV_CORE_TERMINATOR_SENTINEL])
                .ok_or_else(|| new_csv_error(vm, "internal error: missing record terminator"))?;
            let mut output = body.to_vec();
            output.extend_from_slice(self.dialect.lineterminator.as_bytes());

            let s =
                core::str::from_utf8(&output).map_err(|e| new_not_utf8_error(vm, &output, e))?;

            self.write.call((s,), vm)
        }

        #[pymethod]
        fn writerows(&self, rows: ArgIterable, vm: &VirtualMachine) -> PyResult<()> {
            for row in rows.iter(vm)? {
                self.writerow(row?, vm)?;
            }
            Ok(())
        }
    }
}
