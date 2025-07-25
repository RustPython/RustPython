pub(crate) use _csv::make_module;

#[pymodule]
mod _csv {
    use crate::common::lock::PyMutex;
    use crate::vm::{
        AsObject, Py, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyInt, PyNone, PyStr, PyType, PyTypeError, PyTypeRef},
        function::{ArgIterable, ArgumentError, FromArgs, FuncArgs, OptionalArg},
        protocol::{PyIter, PyIterReturn},
        raise_if_stop,
        types::{Constructor, IterNext, Iterable, SelfIter},
    };
    use csv_core::Terminator;
    use itertools::{self, Itertools};
    use parking_lot::Mutex;
    use rustpython_vm::match_class;
    use std::sync::LazyLock;
    use std::{collections::HashMap, fmt};

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

    fn new_csv_error(vm: &VirtualMachine, msg: String) -> PyBaseExceptionRef {
        vm.new_exception_msg(super::_csv::error(vm), msg)
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

        fn py_new(cls: PyTypeRef, ctx: Self::Args, vm: &VirtualMachine) -> PyResult {
            Self::try_from_object(vm, ctx)?
                .into_ref_with_type(vm, cls)
                .map(Into::into)
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
                Terminator::CRLF => vm.ctx.new_str("\r\n".to_string()).to_owned(),
                Terminator::Any(t) => vm.ctx.new_str(format!("{}", t as char)).to_owned(),
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
    fn parse_delimiter_from_obj(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult<u8> {
        if let Ok(attr) = obj.get_attr("delimiter", vm) {
            parse_delimiter_from_obj(vm, &attr)
        } else {
            match_class!(match obj.clone() {
                s @ PyStr => {
                    Ok(s.as_str().bytes().exactly_one().map_err(|_| {
                        let msg = r#""delimiter" must be a 1-character string"#;
                        vm.new_type_error(msg.to_owned())
                    })?)
                }
                attr => {
                    let msg = format!("\"delimiter\" must be string, not {}", attr.class());
                    Err(vm.new_type_error(msg))
                }
            })
        }
    }
    fn parse_quotechar_from_obj(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult<Option<u8>> {
        match_class!(match obj.get_attr("quotechar", vm)? {
            s @ PyStr => {
                Ok(Some(s.as_str().bytes().exactly_one().map_err(|_| {
                    vm.new_exception_msg(
                        super::_csv::error(vm),
                        r#""quotechar" must be a 1-character string"#.to_owned(),
                    )
                })?))
            }
            _n @ PyNone => {
                Ok(None)
            }
            _ => {
                Err(vm.new_exception_msg(
                    super::_csv::error(vm),
                    r#""quotechar" must be string or None, not int"#.to_owned(),
                ))
            }
        })
    }
    fn parse_escapechar_from_obj(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult<Option<u8>> {
        match_class!(match obj.get_attr("escapechar", vm)? {
            s @ PyStr => {
                Ok(Some(s.as_str().bytes().exactly_one().map_err(|_| {
                    vm.new_exception_msg(
                        super::_csv::error(vm),
                        r#""escapechar" must be a 1-character string"#.to_owned(),
                    )
                })?))
            }
            _n @ PyNone => {
                Ok(None)
            }
            attr => {
                let msg = format!(
                    "\"escapechar\" must be string or None, not {}",
                    attr.class()
                );
                Err(vm.new_type_error(msg.to_owned()))
            }
        })
    }
    fn prase_lineterminator_from_obj(
        vm: &VirtualMachine,
        obj: &PyObjectRef,
    ) -> PyResult<Terminator> {
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
                    return Err(vm.new_exception_msg(
                        super::_csv::error(vm),
                        r#""lineterminator" must be a string"#.to_owned(),
                    ));
                })
            }
            _ => {
                let msg = "\"lineterminator\" must be a string".to_string();
                Err(vm.new_type_error(msg.to_owned()))
            }
        })
    }
    fn prase_quoting_from_obj(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult<QuoteStyle> {
        match_class!(match obj.get_attr("quoting", vm)? {
            i @ PyInt => {
                Ok(i.try_to_primitive::<isize>(vm)?.try_into().map_err(|_| {
                    let msg = r#"bad "quoting" value"#;
                    vm.new_type_error(msg.to_owned())
                })?)
            }
            attr => {
                let msg = format!("\"quoting\" must be string or None, not {}", attr.class());
                Err(vm.new_type_error(msg.to_owned()))
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
        let Some(name) = name.downcast_ref::<PyStr>() else {
            return Err(vm.new_type_error("argument 0 must be a string"));
        };
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
        let Some(name) = name.downcast_ref::<PyStr>() else {
            return Err(vm.new_exception_msg(
                super::_csv::error(vm),
                format!("argument 0 must be a string, not '{}'", name.class()),
            ));
        };
        let g = GLOBAL_HASHMAP.lock();
        if let Some(dialect) = g.get(name.as_str()) {
            return Ok(*dialect);
        }
        Err(vm.new_exception_msg(super::_csv::error(vm), "unknown dialect".to_string()))
    }

    #[pyfunction]
    fn unregister_dialect(
        name: PyObjectRef,
        mut _rest: FuncArgs,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let Some(name) = name.downcast_ref::<PyStr>() else {
            return Err(vm.new_exception_msg(
                super::_csv::error(vm),
                format!("argument 0 must be a string, not '{}'", name.class()),
            ));
        };
        let mut g = GLOBAL_HASHMAP.lock();
        if let Some(_removed) = g.remove(name.as_str()) {
            return Ok(());
        }
        Err(vm.new_exception_msg(super::_csv::error(vm), "unknown dialect".to_string()))
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
                delimiter: options.get_delimiter(),
                line_num: 0,
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
                return Err(vm.new_type_error("argument 1 must have a \"write\" method"));
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
    #[derive(Debug, Clone, Copy)]
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
                QuoteStyle::Minimal => Self::Always,
                QuoteStyle::All => Self::Always,
                QuoteStyle::Nonnumeric => Self::NonNumeric,
                QuoteStyle::None => Self::Never,
                QuoteStyle::Strings => todo!(),
                QuoteStyle::Notnull => todo!(),
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
        type Error = PyTypeError;
        fn try_from(num: isize) -> Result<Self, PyTypeError> {
            match num {
                0 => Ok(Self::Minimal),
                1 => Ok(Self::All),
                2 => Ok(Self::Nonnumeric),
                3 => Ok(Self::None),
                4 => Ok(Self::Strings),
                5 => Ok(Self::Notnull),
                _ => Err(PyTypeError {}),
            }
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

    enum DialectItem {
        Str(String),
        Obj(PyDialect),
        None,
    }

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
    impl Default for FormatOptions {
        fn default() -> Self {
            Self {
                dialect: DialectItem::None,
                delimiter: None,
                quotechar: None,
                escapechar: None,
                doublequote: None,
                skipinitialspace: None,
                lineterminator: None,
                quoting: None,
                strict: None,
            }
        }
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
                Ok(DialectItem::Str(s.as_str().to_string()))
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
                    let msg = "dialect".to_string();
                    Err(ArgumentError::InvalidKeywordArgument(msg))
                }
            }
        })
    }

    impl FromArgs for FormatOptions {
        fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
            let mut res = Self::default();
            if let Some(dialect) = args.kwargs.swap_remove("dialect") {
                res.dialect = prase_dialect_item_from_arg(vm, dialect)?;
            } else if let Some(dialect) = args.args.first() {
                res.dialect = prase_dialect_item_from_arg(vm, dialect.clone())?;
            } else {
                res.dialect = DialectItem::None;
            };

            if let Some(delimiter) = args.kwargs.swap_remove("delimiter") {
                res.delimiter = Some(parse_delimiter_from_obj(vm, &delimiter)?);
            }

            if let Some(escapechar) = args.kwargs.swap_remove("escapechar") {
                res.escapechar = match_class!(match escapechar {
                    s @ PyStr => Some(s.as_str().bytes().exactly_one().map_err(|_| {
                        let msg = r#""escapechar" must be a 1-character string"#;
                        vm.new_type_error(msg.to_owned())
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
                            let msg = r#""lineterminator" must be a 1-character string"#;
                            vm.new_type_error(msg.to_owned())
                        })?,
                ))
            };
            if let Some(doublequote) = args.kwargs.swap_remove("doublequote") {
                res.doublequote = Some(doublequote.try_to_bool(vm).map_err(|_| {
                    let msg = r#""doublequote" must be a bool"#;
                    vm.new_type_error(msg.to_owned())
                })?)
            };
            if let Some(skipinitialspace) = args.kwargs.swap_remove("skipinitialspace") {
                res.skipinitialspace = Some(skipinitialspace.try_to_bool(vm).map_err(|_| {
                    let msg = r#""skipinitialspace" must be a bool"#;
                    vm.new_type_error(msg.to_owned())
                })?)
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
                    s @ PyStr => Some(Some(s.as_str().bytes().exactly_one().map_err(|_| {
                        let msg = r#""quotechar" must be a 1-character string"#;
                        vm.new_type_error(msg.to_owned())
                    })?)),
                    PyNone => {
                        if let Some(QuoteStyle::All) = res.quoting {
                            let msg = "quotechar must be set if quoting enabled";
                            return Err(ArgumentError::Exception(
                                vm.new_type_error(msg.to_owned()),
                            ));
                        }
                        Some(None)
                    }
                    _o => {
                        let msg = r#"quotechar"#;
                        return Err(
                            rustpython_vm::function::ArgumentError::InvalidKeywordArgument(
                                msg.to_string(),
                            ),
                        );
                    }
                })
            };
            if let Some(strict) = args.kwargs.swap_remove("strict") {
                res.strict = Some(strict.try_to_bool(vm).map_err(|_| {
                    let msg = r#""strict" must be a int enum"#;
                    vm.new_type_error(msg.to_owned())
                })?)
            };

            if let Some(last_arg) = args.kwargs.pop() {
                let msg = format!(
                    r#"'{}' is an invalid keyword argument for this function"#,
                    last_arg.0
                );
                return Err(rustpython_vm::function::ArgumentError::InvalidKeywordArgument(msg));
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
                if let Some(u) = t {
                    res.quotechar = Some(u);
                } else {
                    res.quotechar = None;
                }
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
                    // TODO
                    // Maybe need to update the obj from HashMap
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
                        // RustPython todo
                        // todo! Perfecting the remaining attributes.
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
        fn get_delimiter(&self) -> u8 {
            let mut delimiter = match &self.dialect {
                DialectItem::Str(name) => {
                    let g = GLOBAL_HASHMAP.lock();
                    if let Some(dialect) = g.get(name) {
                        dialect.delimiter
                        // RustPython todo
                        // todo! Perfecting the remaining attributes.
                    } else {
                        b','
                    }
                }
                DialectItem::Obj(obj) => obj.delimiter,
                _ => b',',
            };
            if let Some(attr) = self.delimiter {
                delimiter = attr
            }
            delimiter
        }
        fn to_reader(&self) -> csv_core::Reader {
            let mut builder = csv_core::ReaderBuilder::new();
            let mut reader = match &self.dialect {
                DialectItem::Str(name) => {
                    let g = GLOBAL_HASHMAP.lock();
                    if let Some(dialect) = g.get(name) {
                        let mut builder = builder
                            .delimiter(dialect.delimiter)
                            .double_quote(dialect.doublequote);
                        if let Some(t) = dialect.quotechar {
                            builder = builder.quote(t);
                        }
                        builder
                        // RustPython todo
                        // todo! Perfecting the remaining attributes.
                    } else {
                        &mut builder
                    }
                }
                DialectItem::Obj(obj) => {
                    let mut builder = builder
                        .delimiter(obj.delimiter)
                        .double_quote(obj.doublequote);
                    if let Some(t) = obj.quotechar {
                        builder = builder.quote(t);
                    }
                    builder
                }
                _ => {
                    let name = "excel";
                    let g = GLOBAL_HASHMAP.lock();
                    let dialect = g.get(name).unwrap();
                    let mut builder = builder
                        .delimiter(dialect.delimiter)
                        .double_quote(dialect.doublequote);
                    if let Some(quotechar) = dialect.quotechar {
                        builder = builder.quote(quotechar);
                    }
                    builder
                }
            };

            if let Some(t) = self.delimiter {
                reader = reader.delimiter(t);
            }
            if let Some(t) = self.quotechar {
                if let Some(u) = t {
                    reader = reader.quote(u);
                } else {
                    reader = reader.quoting(false);
                }
            } else {
                match self.quoting {
                    Some(QuoteStyle::None) => {
                        reader = reader.quoting(false);
                    }
                    // None => reader = reader.quoting(true),
                    _ => reader = reader.quoting(true),
                }
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
            reader = match self.lineterminator {
                Some(u) => reader.terminator(u),
                None => reader.terminator(Terminator::CRLF),
            };
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

                        // RustPython todo
                        // todo! Perfecting the remaining attributes.
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
            if let Some(t) = self.quotechar {
                if let Some(u) = t {
                    writer = writer.quote(u);
                } else {
                    todo!()
                }
            }
            if let Some(t) = self.doublequote {
                writer = writer.double_quote(t);
            }
            writer = match self.lineterminator {
                Some(u) => writer.terminator(u),
                None => writer.terminator(Terminator::CRLF),
            };
            if let Some(e) = self.escapechar {
                writer = writer.escape(e);
            }
            if let Some(e) = self.quoting {
                writer = writer.quote_style(e.into());
            }
            writer.build()
        }
    }

    struct ReadState {
        buffer: Vec<u8>,
        output_ends: Vec<usize>,
        reader: csv_core::Reader,
        skipinitialspace: bool,
        delimiter: u8,
        line_num: u64,
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

    #[pyclass(with(IterNext, Iterable))]
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
    impl IterNext for Reader {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let string = raise_if_stop!(zelf.iter.next(vm)?);
            let string = string.downcast::<PyStr>().map_err(|obj| {
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
            let mut state = zelf.state.lock();
            let ReadState {
                buffer,
                output_ends,
                reader,
                skipinitialspace,
                delimiter,
                line_num,
            } = &mut *state;

            let mut input_offset = 0;
            let mut output_offset = 0;
            let mut output_ends_offset = 0;
            let field_limit = GLOBAL_FIELD_LIMIT.lock().to_owned();
            #[inline]
            fn trim_spaces(input: &[u8]) -> &[u8] {
                let trimmed_start = input.iter().position(|&x| x != b' ').unwrap_or(input.len());
                let trimmed_end = input
                    .iter()
                    .rposition(|&x| x != b' ')
                    .map(|i| i + 1)
                    .unwrap_or(0);
                &input[trimmed_start..trimmed_end]
            }
            let input = if *skipinitialspace {
                let t = input.split(|x| x == delimiter);
                t.map(|x| {
                    let trimmed = trim_spaces(x);
                    String::from_utf8(trimmed.to_vec()).unwrap()
                })
                .join(format!("{}", *delimiter as char).as_str())
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
                    "new-line character seen in unquoted field - \
                    do you need to open the file in universal-newline mode?"
                        .to_owned(),
                ));
            }

            let mut prev_end = 0;
            let out: Vec<PyObjectRef> = output_ends[..output_ends_offset]
                .iter()
                .map(|&end| {
                    let range = prev_end..end;
                    if range.len() > field_limit as usize {
                        return Err(new_csv_error(vm, "filed too long to read".to_string()));
                    }
                    prev_end = end;
                    let s = std::str::from_utf8(&buffer[range.clone()])
                        // not sure if this is possible - the input was all strings
                        .map_err(|_e| vm.new_unicode_decode_error("csv not utf8"))?;
                    // Rustpython TODO!
                    // Incomplete implementation
                    if let QuoteStyle::Nonnumeric = zelf.dialect.quoting {
                        if let Ok(t) =
                            String::from_utf8(trim_spaces(&buffer[range.clone()]).to_vec())
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

    #[pyclass]
    impl Writer {
        #[pygetset(name = "dialect")]
        const fn get_dialect(&self, _vm: &VirtualMachine) -> PyDialect {
            self.dialect
        }
        #[pymethod]
        fn writerow(&self, row: PyObjectRef, vm: &VirtualMachine) -> PyResult {
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
                new_csv_error(vm, format!("\'{}\' object is not iterable", row.class()))
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
            let s = std::str::from_utf8(&buffer[..buffer_offset])
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
