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
        raise_if_stop,
        types::{Constructor, IterNext, Iterable, SelfIter},
    };
    use alloc::fmt;
    use csv_core::Terminator;
    use itertools::Itertools;
    use parking_lot::Mutex;
    use rustpython_common::{lock::LazyLock, wtf8::Wtf8Buf};
    use rustpython_vm::{match_class, sliceable::SliceableSequenceOp};
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

    #[pyattr]
    #[pyclass(module = "csv", name = "Dialect")]
    #[derive(Debug, PyPayload, Clone, Copy)]
    struct PyDialect {
        delimiter: u8,
        quotechar: Option<u8>,
        escapechar: Option<u8>,
        doublequote: bool,
        skipinitialspace: bool,
        lineterminator: csv_core::Terminator,
        quoting: QuoteStyle,
        strict: bool,
    }

    impl Constructor for PyDialect {
        type Args = PyObjectRef;

        fn py_new(_cls: &Py<PyType>, ctx: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            Self::try_from_object(vm, ctx)
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
            match self.lineterminator {
                Terminator::CRLF => vm.ctx.new_str("\r\n".to_string()),
                Terminator::Any(t) => vm.ctx.new_str(format!("{}", t as char)),
                _ => unreachable!(),
            }
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
                    Ok(s.as_bytes().iter().copied().exactly_one().map_err(|_| {
                        vm.new_type_error(format!(
                            r#""delimiter" must be a unicode character, not a string of length {}"#,
                            s.len()
                        ))
                    })?)
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
                Ok(Some(s.as_bytes().iter().copied().exactly_one().map_err(|_| {
                    new_csv_error(vm, format!(r#""quotechar" must be a unicode character or None, not a string of length {}"#, s.len()))
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
                Ok(Some(s.as_bytes().iter().copied().exactly_one().map_err(|_| {
                    new_csv_error(
                        vm,
                        format!(r#""escapechar" must be a unicode character or None, not a string of length {}"#, s.len()),
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

    fn prase_lineterminator_from_obj(vm: &VirtualMachine, obj: &PyObject) -> PyResult<Terminator> {
        match_class!(match obj.get_attr("lineterminator", vm)? {
            s @ PyStr => {
                Ok(if s.as_bytes().eq(b"\r\n") {
                    csv_core::Terminator::CRLF
                } else if let Some(t) = s.as_bytes().first() {
                    // Due to limitations in the current implementation within csv_core
                    // the support for multiple characters in lineterminator is not complete.
                    // only capture the first character
                    csv_core::Terminator::Any(*t)
                } else {
                    return Err(new_csv_error(vm, r#""lineterminator" must be a string"#));
                })
            }
            attr => {
                Err(vm.new_type_error(format!(
                    r#""lineterminator" must be a string, not {}"#,
                    attr.class().name()
                )))
            }
        })
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
            return Ok(*dialect);
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
        Ok(Reader {
            iter,
            state: PyMutex::new(ReadState {
                buffer: vec![0; 1024],
                output_ends: vec![0; 16],
                reader: options.to_reader(),
                skipinitialspace: options.get_skipinitialspace(),
                line_num: 0,
                generation: 0,
            }),
            dialect: options.result(vm)?,
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

        Ok(Writer {
            write,
            state: PyMutex::new(WriteState {
                buffer: vec![0; 1024],
                writer: options.to_writer(),
            }),
            dialect: options.result(vm)?,
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
        escapechar: Option<u8>,
        doublequote: Option<bool>,
        skipinitialspace: Option<bool>,
        lineterminator: Option<csv_core::Terminator>,
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
                    s @ PyStr =>
                        Some(s.as_bytes().iter().copied().exactly_one().map_err(|_| {
                            vm.new_type_error(r#""escapechar" must be a 1-character string"#)
                        })?),
                    _ => None,
                })
            };

            if let Some(lineterminator) = args.kwargs.swap_remove("lineterminator") {
                res.lineterminator = Some(csv_core::Terminator::Any(
                    lineterminator
                        .try_to_value::<&str>(vm)?
                        .bytes()
                        .exactly_one()
                        .map_err(|_| {
                            vm.new_type_error(r#""lineterminator" must be a 1-character string"#)
                        })?,
                ))
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
                    s @ PyStr => Some(Some(s.as_bytes().iter().copied().exactly_one().map_err(
                        |_| { vm.new_type_error(r#""quotechar" must be a 1-character string"#) }
                    )?)),
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
                return Err(
                    rustpython_vm::function::ArgumentError::InvalidKeywordArgument(format!(
                        "'{}' is an invalid keyword argument for this function",
                        last_arg.0
                    )),
                );
            }

            Ok(res)
        }
    }

    impl FormatOptions {
        const fn update_py_dialect(&self, mut res: PyDialect) -> PyDialect {
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
                res.escapechar = Some(t);
            };

            if let Some(t) = self.quotechar {
                res.quotechar = t;
            };

            check_and_fill!(res, quoting);
            check_and_fill!(res, lineterminator);
            check_and_fill!(res, strict);
            res
        }

        fn result(&self, vm: &VirtualMachine) -> PyResult<PyDialect> {
            match &self.dialect {
                DialectItem::Str(name) => {
                    let g = GLOBAL_HASHMAP.lock();
                    if let Some(dialect) = g.get(name) {
                        Ok(self.update_py_dialect(*dialect))
                    } else {
                        Err(new_csv_error(vm, format!("{name} is not registered.")))
                    }
                    // TODO: Maybe need to update the obj from HashMap
                }
                DialectItem::Obj(o) => Ok(self.update_py_dialect(*o)),
                DialectItem::None => {
                    let g = GLOBAL_HASHMAP.lock();
                    let res = *g.get("excel").unwrap();
                    Ok(self.update_py_dialect(res))
                }
            }
        }

        fn get_skipinitialspace(&self) -> bool {
            let mut skipinitialspace = match &self.dialect {
                DialectItem::Str(name) => {
                    let g = GLOBAL_HASHMAP.lock();
                    if let Some(dialect) = g.get(name) {
                        dialect.skipinitialspace
                        // TODO: RUSTPYTHON; Perfecting the remaining attributes.
                    } else {
                        false
                    }
                }
                DialectItem::Obj(obj) => obj.skipinitialspace,
                _ => false,
            };

            if let Some(attr) = self.skipinitialspace {
                skipinitialspace = attr
            }

            skipinitialspace
        }

        fn get_lineterminator(&self) -> csv_core::Terminator {
            let mut lineterminator = match &self.dialect {
                DialectItem::Str(name) => {
                    let g = GLOBAL_HASHMAP.lock();
                    if let Some(dialect) = g.get(name) {
                        dialect.lineterminator
                    } else {
                        Terminator::CRLF
                    }
                }
                DialectItem::Obj(obj) => obj.lineterminator,
                _ => Terminator::CRLF,
            };

            if let Some(attr) = self.lineterminator {
                lineterminator = attr
            }

            lineterminator
        }

        fn get_quoting(&self) -> QuoteStyle {
            let mut quoting = match &self.dialect {
                DialectItem::Str(name) => {
                    let g = GLOBAL_HASHMAP.lock();
                    if let Some(dialect) = g.get(name) {
                        dialect.quoting
                    } else {
                        QuoteStyle::Minimal
                    }
                }
                DialectItem::Obj(obj) => obj.quoting,
                _ => QuoteStyle::Minimal,
            };

            if let Some(attr) = self.quoting {
                quoting = attr
            }

            quoting
        }

        fn to_reader(&self) -> csv_core::Reader {
            let dialect = match &self.dialect {
                DialectItem::Str(name) => GLOBAL_HASHMAP.lock().get(name).copied(),
                DialectItem::Obj(obj) => Some(*obj),
                DialectItem::None => {
                    let g = GLOBAL_HASHMAP.lock();
                    Some(*g.get("excel").unwrap())
                }
            };

            let mut builder = csv_core::ReaderBuilder::new();
            let mut reader = if let Some(dialect) = dialect {
                let mut builder = builder
                    .delimiter(dialect.delimiter)
                    .double_quote(dialect.doublequote)
                    .escape(dialect.escapechar);
                if let Some(quotechar) = dialect.quotechar {
                    builder = builder.quote(quotechar);
                }
                builder
            } else {
                &mut builder
            };

            if let Some(t) = self.delimiter {
                reader = reader.delimiter(t);
            }

            if let Some(t) = self.quotechar {
                reader = if let Some(u) = t {
                    reader.quote(u)
                } else {
                    reader.quoting(false)
                }
            } else {
                reader = reader.quoting(self.quoting != Some(QuoteStyle::None));
            }

            if let Some(t) = self.lineterminator {
                reader = reader.terminator(t);
            }

            if let Some(t) = self.doublequote {
                reader = reader.double_quote(t);
            }

            if self.escapechar.is_some() {
                reader = reader.escape(self.escapechar);
            }

            reader = reader.terminator(self.lineterminator.unwrap_or(Terminator::CRLF));

            reader.build()
        }

        fn to_writer(&self) -> csv_core::Writer {
            let mut builder = csv_core::WriterBuilder::new();
            let mut writer = match &self.dialect {
                DialectItem::Str(name) => {
                    let g = GLOBAL_HASHMAP.lock();
                    if let Some(dialect) = g.get(name) {
                        let mut builder = builder
                            .delimiter(dialect.delimiter)
                            .double_quote(dialect.doublequote)
                            .terminator(dialect.lineterminator);

                        if let Some(t) = dialect.quotechar {
                            builder = builder.quote(t);
                        }

                        builder

                        // TODO: RUSTPYTHON; Perfecting the remaining attributes.
                    } else {
                        &mut builder
                    }
                }
                DialectItem::Obj(obj) => {
                    let mut builder = builder
                        .delimiter(obj.delimiter)
                        .double_quote(obj.doublequote)
                        .terminator(obj.lineterminator);

                    if let Some(t) = obj.quotechar {
                        builder = builder.quote(t);
                    }

                    builder
                }
                _ => &mut builder,
            };

            if let Some(t) = self.delimiter {
                writer = writer.delimiter(t);
            }

            if let Some(Some(t)) = self.quotechar {
                writer = writer.quote(t);
            }

            if let Some(t) = self.doublequote {
                writer = writer.double_quote(t);
            }

            writer = writer.terminator(self.get_lineterminator());

            if let Some(e) = self.escapechar {
                writer = writer.escape(e);
            }

            writer = writer.quote_style(self.get_quoting().into());

            writer.build()
        }
    }

    struct ReadState {
        buffer: Vec<u8>,
        output_ends: Vec<usize>,
        reader: csv_core::Reader,
        skipinitialspace: bool,
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
        const fn dialect(&self, _vm: &VirtualMachine) -> PyDialect {
            self.dialect
        }
    }

    impl SelfIter for Reader {}

    enum QuoteScanEvent {
        InitialSpace,
        StartQuotedField,
        EndQuotedField,
        Escaped(Option<u8>),
        DoubleQuote(u8),
        Delimiter,
        RecordTerminator,
        Data(u8),
    }

    struct QuoteScanState {
        at_field_start: bool,
        in_quoted_field: bool,
    }

    impl QuoteScanState {
        const fn new() -> Self {
            Self {
                at_field_start: true,
                in_quoted_field: false,
            }
        }

        fn scan(
            &mut self,
            input: &[u8],
            index: usize,
            dialect: PyDialect,
            unquoted_escape: bool,
        ) -> (QuoteScanEvent, usize) {
            let byte = input[index];

            if (self.in_quoted_field || unquoted_escape) && dialect.escapechar == Some(byte) {
                self.at_field_start = false;
                return match input.get(index + 1).copied() {
                    Some(escaped) => (QuoteScanEvent::Escaped(Some(escaped)), 2),
                    None => (QuoteScanEvent::Escaped(None), 1),
                };
            }

            if self.in_quoted_field {
                if dialect.quotechar == Some(byte) {
                    if dialect.doublequote && input.get(index + 1) == Some(&byte) {
                        return (QuoteScanEvent::DoubleQuote(byte), 2);
                    }
                    self.in_quoted_field = false;
                    return (QuoteScanEvent::EndQuotedField, 1);
                }
                return (QuoteScanEvent::Data(byte), 1);
            }

            if self.at_field_start && dialect.skipinitialspace && byte == b' ' {
                return (QuoteScanEvent::InitialSpace, 1);
            }

            if self.at_field_start
                && dialect.quoting != QuoteStyle::None
                && dialect.quotechar == Some(byte)
            {
                self.at_field_start = false;
                self.in_quoted_field = true;
                return (QuoteScanEvent::StartQuotedField, 1);
            }

            if byte == dialect.delimiter {
                self.at_field_start = true;
                return (QuoteScanEvent::Delimiter, 1);
            }

            self.at_field_start = false;
            if matches!(byte, b'\r' | b'\n') {
                (QuoteScanEvent::RecordTerminator, 1)
            } else {
                (QuoteScanEvent::Data(byte), 1)
            }
        }
    }

    fn read_quote_record(
        input: &[u8],
        dialect: PyDialect,
        field_limit: isize,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<PyObjectRef>> {
        // QUOTE_NOTNULL and QUOTE_STRINGS map empty unquoted fields to None,
        // but preserve quoted empty fields as strings, so retain quote provenance.
        let mut fields = vec![(Vec::new(), false)];
        let mut scan_state = QuoteScanState::new();
        let mut dangling_escape = false;
        let mut index = 0;

        while index < input.len() {
            let (event, consumed) = scan_state.scan(input, index, dialect, true);
            match event {
                QuoteScanEvent::InitialSpace | QuoteScanEvent::EndQuotedField => {}
                QuoteScanEvent::StartQuotedField => fields.last_mut().unwrap().1 = true,
                QuoteScanEvent::Escaped(Some(byte)) | QuoteScanEvent::DoubleQuote(byte) => {
                    fields.last_mut().unwrap().0.push(byte);
                }
                QuoteScanEvent::Escaped(None) => dangling_escape = true,
                QuoteScanEvent::Delimiter => fields.push((Vec::new(), false)),
                QuoteScanEvent::RecordTerminator => {
                    if !input[index..]
                        .iter()
                        .all(|&byte| matches!(byte, b'\r' | b'\n'))
                    {
                        return Err(new_csv_error(
                            vm,
                            concat!(
                                "new-line character seen in unquoted field",
                                " - do you need to open the file in universal-newline mode?"
                            ),
                        ));
                    }
                    break;
                }
                QuoteScanEvent::Data(byte) => fields.last_mut().unwrap().0.push(byte),
            }
            index += consumed;
        }

        // CPython treats an escape character at the end of an iterator item
        // as escaping the implicit newline at the end of that item.
        if dangling_escape {
            fields.last_mut().unwrap().0.push(b'\n');
        }

        fields
            .into_iter()
            .map(|(field, was_quoted)| {
                if field.len() > field_limit as usize {
                    return Err(new_csv_error(vm, "filed too long to read"));
                }
                if matches!(dialect.quoting, QuoteStyle::Notnull | QuoteStyle::Strings)
                    && !was_quoted
                    && field.is_empty()
                {
                    return Ok(vm.ctx.none());
                }
                let field = core::str::from_utf8(&field)
                    .map_err(|_| vm.new_unicode_decode_error("csv not utf8"))?;
                Ok(vm.ctx.new_str(field).into())
            })
            .collect()
    }

    impl IterNext for Reader {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let generation = zelf.state.lock().generation;
            let string_obj = raise_if_stop!(zelf.iter.next(vm)?);
            let mut state = zelf.state.lock();
            if state.generation != generation {
                return Err(new_csv_error(
                    vm,
                    "iterator has already advanced the reader",
                ));
            }
            state.generation += 1;

            let string = string_obj.downcast::<PyStr>().map_err(|obj| {
                new_csv_error(
                    vm,
                    format!(
                "iterator should return strings, not {} (the file should be opened in text mode)",
                obj.class().name()
            ),
                )
            })?;
            let input = string.as_bytes();
            if input.is_empty() || input.starts_with(b"\n") {
                return Ok(PyIterReturn::Return(vm.ctx.new_list(vec![]).into()));
            }
            let ReadState {
                buffer,
                output_ends,
                reader,
                skipinitialspace,
                line_num,
                generation: _,
            } = &mut *state;

            let mut input_offset = 0;
            let mut output_offset = 0;
            let mut output_ends_offset = 0;
            let field_limit = GLOBAL_FIELD_LIMIT.lock().to_owned();

            let use_quote_record = matches!(
                zelf.dialect.quoting,
                QuoteStyle::Notnull | QuoteStyle::Strings
            ) || (zelf.dialect.quoting == QuoteStyle::None
                && zelf.dialect.escapechar.is_some());
            if use_quote_record {
                let out = read_quote_record(input, zelf.dialect, field_limit, vm)?;
                *line_num += 1;
                return Ok(PyIterReturn::Return(vm.ctx.new_list(out).into()));
            }

            #[inline]
            fn trim_initial_spaces(input: &[u8], dialect: PyDialect) -> Vec<u8> {
                let mut trimmed = Vec::with_capacity(input.len());
                let mut scan_state = QuoteScanState::new();
                let mut index = 0;

                // Delimiters inside quoted fields are data, so only skip spaces
                // after delimiters encountered outside quotes.
                while index < input.len() {
                    let (event, consumed) = scan_state.scan(input, index, dialect, false);
                    if !matches!(event, QuoteScanEvent::InitialSpace) {
                        trimmed.extend_from_slice(&input[index..index + consumed]);
                    }
                    index += consumed;
                }

                trimmed
            }

            #[inline]
            fn trim_spaces(input: &[u8]) -> &[u8] {
                let trimmed_start = input.iter().position(|&x| x != b' ').unwrap_or(input.len());
                let trimmed_end = input.iter().rposition(|&x| x != b' ').map_or(0, |i| i + 1);
                if trimmed_start >= trimmed_end {
                    &input[input.len()..]
                } else {
                    &input[trimmed_start..trimmed_end]
                }
            }

            let input = if *skipinitialspace {
                String::from_utf8(trim_initial_spaces(input, zelf.dialect)).unwrap()
            } else {
                String::from_utf8(input.to_vec()).unwrap()
            };

            loop {
                let (res, n_read, n_written, n_ends) = reader.read_record(
                    &input.as_bytes()[input_offset..],
                    &mut buffer[output_offset..],
                    &mut output_ends[output_ends_offset..],
                );
                input_offset += n_read;
                output_offset += n_written;
                output_ends_offset += n_ends;
                match res {
                    csv_core::ReadRecordResult::InputEmpty => {}
                    csv_core::ReadRecordResult::OutputFull => resize_buf(buffer),
                    csv_core::ReadRecordResult::OutputEndsFull => resize_buf(output_ends),
                    csv_core::ReadRecordResult::Record => break,
                    csv_core::ReadRecordResult::End => {
                        return Ok(PyIterReturn::StopIteration(None));
                    }
                }
            }

            let rest = &input.as_bytes()[input_offset..];
            if !rest.iter().all(|&c| matches!(c, b'\r' | b'\n')) {
                return Err(new_csv_error(
                    vm,
                    concat!(
                        "new-line character seen in unquoted field",
                        " - do you need to open the file in universal-newline mode?"
                    ),
                ));
            }

            let mut prev_end = 0;
            let out: Vec<PyObjectRef> = output_ends[..output_ends_offset]
                .iter()
                .map(|&end| {
                    let range = prev_end..end;
                    if range.len() > field_limit as usize {
                        return Err(new_csv_error(vm, "filed too long to read"));
                    }

                    prev_end = end;
                    let s = core::str::from_utf8(&buffer[range.clone()])
                        // not sure if this is possible - the input was all strings
                        .map_err(|_e| vm.new_unicode_decode_error("csv not utf8"))?;

                    // TODO: RUSTPYTHON; Incomplete implementation
                    if let QuoteStyle::Nonnumeric = zelf.dialect.quoting {
                        if let Ok(t) = String::from_utf8(trim_spaces(&buffer[range]).to_vec())
                            .unwrap()
                            .parse::<i64>()
                        {
                            Ok(vm.ctx.new_int(t).into())
                        } else {
                            Ok(vm.ctx.new_str(s).into())
                        }
                    } else {
                        Ok(vm.ctx.new_str(s).into())
                    }
                })
                .collect::<Result<_, _>>()?;
            // Removes the last null item before the line terminator, if there is a separator before the line terminator,
            // todo!
            // if out.last().unwrap().length(vm).unwrap() == 0 {
            //     out.pop();
            // }
            *line_num += 1;
            Ok(PyIterReturn::Return(vm.ctx.new_list(out).into()))
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
        dialect: PyDialect,
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
        dialect: PyDialect,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        for &byte in data {
            if field_needs_escape(byte, dialect) {
                let escapechar = dialect
                    .escapechar
                    .ok_or_else(|| new_csv_error(vm, "need to escape, but no escapechar set"))?;
                output.push(escapechar);
            }
            output.push(byte);
        }
        Ok(())
    }

    fn field_needs_quotes(data: &[u8], dialect: PyDialect) -> bool {
        data.iter().any(|&byte| {
            byte == dialect.delimiter
                || dialect.quotechar == Some(byte)
                || matches!(byte, b'\r' | b'\n')
                || matches!(dialect.lineterminator, Terminator::Any(t) if byte == t)
        })
    }

    fn field_needs_escape(byte: u8, dialect: PyDialect) -> bool {
        byte == dialect.delimiter
            || dialect.quotechar == Some(byte)
            || dialect.escapechar == Some(byte)
            || matches!(byte, b'\r' | b'\n')
            || matches!(dialect.lineterminator, Terminator::Any(t) if byte == t)
    }

    fn write_lineterminator(output: &mut Vec<u8>, terminator: Terminator) {
        match terminator {
            Terminator::CRLF => output.extend_from_slice(b"\r\n"),
            Terminator::Any(byte) => output.push(byte),
            _ => unreachable!(),
        }
    }

    #[pyclass(flags(DISALLOW_INSTANTIATION))]
    impl Writer {
        #[pygetset(name = "dialect")]
        const fn get_dialect(&self, _vm: &VirtualMachine) -> PyDialect {
            self.dialect
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
                    QuoteStyle::Strings => is_str || field_needs_quotes(data, self.dialect),
                    QuoteStyle::Notnull => !is_none,
                    _ => unreachable!(),
                };
                if should_quote {
                    write_quoted_field(&mut output, data, self.dialect, vm)?;
                } else if single_field && data.is_empty() {
                    return Err(new_csv_error(
                        vm,
                        "single empty field record must be quoted",
                    ));
                } else {
                    output.extend_from_slice(data);
                }
            }

            write_lineterminator(&mut output, self.dialect.lineterminator);
            let s = core::str::from_utf8(&output)
                .map_err(|_| vm.new_unicode_decode_error("csv not utf8"))?;
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

                write_unquoted_field(&mut output, data, self.dialect, vm)?;
            }

            write_lineterminator(&mut output, self.dialect.lineterminator);

            let s = core::str::from_utf8(&output)
                .map_err(|_| vm.new_unicode_decode_error("csv not utf8"))?;

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
                if field_needs_quotes(data, self.dialect) || (single_field && data.is_empty()) {
                    write_quoted_field(&mut output, data, self.dialect, vm)?;
                } else {
                    output.extend_from_slice(data);
                }
            }

            write_lineterminator(&mut output, self.dialect.lineterminator);

            let s = core::str::from_utf8(&output)
                .map_err(|_| vm.new_unicode_decode_error("csv not utf8"))?;

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

            let s = core::str::from_utf8(&buffer[..buffer_offset])
                .map_err(|_| vm.new_unicode_decode_error("csv not utf8"))?;

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
