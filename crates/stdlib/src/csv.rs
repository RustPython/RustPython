pub(crate) use _csv::module_def;

#[pymodule]
mod _csv {
    use crate::common::lock::PyMutex;
    use crate::vm::{
        AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
        VirtualMachine,
        builtins::{PyBaseExceptionRef, PyInt, PyStr, PyType, PyTypeRef, PyUtf8StrRef},
        function::{ArgIterable, ArgumentError, FromArgs, FuncArgs, OptionalArg, Param},
        protocol::{PyIter, PyIterReturn, PyNumber},
        types::{Callable, Constructor, IterNext, Iterable, SelfIter},
    };
    use alloc::fmt;
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
    #[pyclass(module = "_csv", name = "Dialect")]
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

    impl Constructor for PyDialect {
        type Args = FormatOptions;

        fn py_new(_cls: &Py<PyType>, opts: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            opts.result(vm)
        }
    }

    #[pyclass(with(Constructor))]
    impl PyDialect {
        #[pygetset]
        fn delimiter(zelf: &Py<Self>, vm: &VirtualMachine) -> PyRef<PyStr> {
            vm.ctx.new_str(format!("{}", zelf.delimiter as char))
        }

        #[pygetset]
        fn quotechar(zelf: &Py<Self>, vm: &VirtualMachine) -> Option<PyRef<PyStr>> {
            Some(vm.ctx.new_str(format!("{}", zelf.quotechar? as char)))
        }

        #[pygetset]
        fn doublequote(zelf: &Py<Self>) -> bool {
            zelf.doublequote
        }

        #[pygetset]
        fn skipinitialspace(zelf: &Py<Self>) -> bool {
            zelf.skipinitialspace
        }

        #[pygetset]
        fn lineterminator(zelf: &Py<Self>, vm: &VirtualMachine) -> PyRef<PyStr> {
            vm.ctx.new_str(zelf.lineterminator.clone())
        }

        #[pygetset]
        fn quoting(zelf: &Py<Self>) -> isize {
            zelf.quoting.into()
        }

        #[pygetset]
        fn escapechar(zelf: &Py<Self>, vm: &VirtualMachine) -> Option<PyRef<PyStr>> {
            Some(vm.ctx.new_str(format!("{}", zelf.escapechar? as char)))
        }

        #[pygetset(name = "strict")]
        fn get_strict(zelf: &Py<Self>) -> bool {
            zelf.strict
        }
    }

    fn parse_char(
        vm: &VirtualMachine,
        obj: &PyObject,
        name: &str,
        allow_none: bool,
    ) -> PyResult<Option<u8>> {
        if allow_none && vm.is_none(obj) {
            return Ok(None);
        }
        let expected = if allow_none {
            "a unicode character or None"
        } else {
            "a unicode character"
        };
        let value = obj.downcast_ref::<PyStr>().ok_or_else(|| {
            vm.new_type_error(format!(
                r#""{name}" must be {expected}, not {}"#,
                obj.class().name()
            ))
        })?;
        parse_single_char(value, |len| {
            vm.new_type_error(format!(
                r#""{name}" must be {expected}, not a string of length {len}"#
            ))
        })
        .map(Some)
    }

    fn parse_lineterminator<'a>(vm: &VirtualMachine, s: &'a Py<PyStr>) -> PyResult<&'a str> {
        s.to_str()
            .ok_or_else(|| new_csv_error(vm, r#""lineterminator" must be a string"#))
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

    #[derive(FromArgs)]
    struct DialectName {
        #[pyarg(any)]
        name: PyObjectRef,
    }

    #[pyfunction]
    fn register_dialect(
        name: PyObjectRef,
        opts: FormatOptions,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let name = name
            .downcast::<PyStr>()
            .map_err(|_| vm.new_type_error("argument 0 must be a string"))?;

        let name: PyUtf8StrRef = name.try_into_utf8(vm)?;

        let dialect = opts.result(vm)?;
        validate_dialect(vm, &dialect)?;
        GLOBAL_HASHMAP
            .lock()
            .insert(name.as_str().to_owned(), dialect);

        Ok(())
    }

    #[pyfunction]
    fn get_dialect(DialectName { name }: DialectName, vm: &VirtualMachine) -> PyResult<PyDialect> {
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
    fn unregister_dialect(DialectName { name }: DialectName, vm: &VirtualMachine) -> PyResult<()> {
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
    fn list_dialects(vm: &VirtualMachine) -> rustpython_vm::builtins::PyListRef {
        let g = GLOBAL_HASHMAP.lock();
        let t = g
            .keys()
            .cloned()
            .map(|x| vm.ctx.new_str(x).into())
            .collect_vec();
        // .iter().map(|x| vm.ctx.new_str(x.clone()).into_pyobject(vm)).collect_vec();
        vm.ctx.new_list(t)
    }

    #[derive(FromArgs)]
    struct FieldSizeLimitArgs {
        #[pyarg(any, optional)]
        new_limit: OptionalArg<PyObjectRef>,
    }

    #[pyfunction]
    fn field_size_limit(args: FieldSizeLimitArgs, vm: &VirtualMachine) -> PyResult<isize> {
        let old_size = GLOBAL_FIELD_LIMIT.lock().to_owned();
        if let OptionalArg::Present(limit) = args.new_limit {
            let Ok(new_size) = limit.try_int(vm) else {
                return Err(vm.new_type_error("limit must be an integer"));
            };
            *GLOBAL_FIELD_LIMIT.lock() = new_size.try_to_primitive::<isize>(vm)?;
        }
        Ok(old_size)
    }

    #[pyfunction]
    fn reader(iterable: PyIter, options: FormatOptions, vm: &VirtualMachine) -> PyResult<Reader> {
        let dialect = options.result(vm)?;
        Ok(Reader {
            iter: iterable,
            state: PyMutex::new(ReadState {
                line_num: 0,
                generation: 0,
            }),
            dialect,
        })
    }

    #[pyfunction]
    fn writer(
        fileobj: PyObjectRef,
        options: FormatOptions,
        vm: &VirtualMachine,
    ) -> PyResult<Writer> {
        let write = match vm.get_attribute_opt(&fileobj, "write")? {
            Some(write_meth) => write_meth,
            None if fileobj.is_callable() => fileobj,
            None => {
                return Err(vm.new_type_error(r#"argument 1 must have a "write" method"#));
            }
        };
        let dialect = options.result(vm)?;

        Ok(Writer {
            write,
            state: PyMutex::new(()),
            dialect,
        })
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
    struct FormatOptions {
        dialect: Option<PyObjectRef>,
        delimiter: Option<PyObjectRef>,
        doublequote: Option<PyObjectRef>,
        escapechar: Option<PyObjectRef>,
        lineterminator: Option<PyObjectRef>,
        quotechar: Option<PyObjectRef>,
        quoting: Option<PyObjectRef>,
        skipinitialspace: Option<PyObjectRef>,
        strict: Option<PyObjectRef>,
    }

    impl FromArgs for FormatOptions {
        const PARAMS: Option<&'static [Param]> = Some(&[
            Param {
                name: "dialect",
                kind: rustpython_vm::function::ParamKind::PositionalOrKeyword,
                default: Some(rustpython_vm::function::DefaultRepr::Str("excel")),
            },
            Param::var_keyword("fmtparams"),
        ]);

        fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
            let res = Self {
                dialect: args.take_positional_keyword("dialect"),
                delimiter: args.kwargs.swap_remove("delimiter"),
                doublequote: args.kwargs.swap_remove("doublequote"),
                escapechar: args.kwargs.swap_remove("escapechar"),
                lineterminator: args.kwargs.swap_remove("lineterminator"),
                quotechar: args.kwargs.swap_remove("quotechar"),
                quoting: args.kwargs.swap_remove("quoting"),
                skipinitialspace: args.kwargs.swap_remove("skipinitialspace"),
                strict: args.kwargs.swap_remove("strict"),
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
        fn result(mut self, vm: &VirtualMachine) -> PyResult<PyDialect> {
            if let Some(obj) = self.dialect.take() {
                let obj = if obj.downcast_ref::<PyStr>().is_some() {
                    get_dialect(DialectName { name: obj }, vm)?
                        .into_ref(&vm.ctx)
                        .into()
                } else {
                    obj
                };
                // Read all non-overridden attributes before converting their values.
                macro_rules! fill_from_dialect {
                    ($($name:ident),* $(,)?) => {
                        $(if self.$name.is_none() {
                            self.$name = vm.get_attribute_opt(&obj, stringify!($name))?;
                        })*
                    };
                }
                fill_from_dialect!(
                    delimiter,
                    doublequote,
                    escapechar,
                    lineterminator,
                    quotechar,
                    quoting,
                    skipinitialspace,
                    strict,
                );
                self.dialect = Some(obj);
            }

            let delimiter = match &self.delimiter {
                Some(obj) => parse_char(vm, obj, "delimiter", false)?.unwrap(),
                None => b',',
            };
            let doublequote = match &self.doublequote {
                Some(obj) => obj.try_to_bool(vm)?,
                None => true,
            };
            let escapechar = match &self.escapechar {
                Some(obj) => parse_char(vm, obj, "escapechar", true)?,
                None => None,
            };
            let lineterminator = match &self.lineterminator {
                Some(obj) => {
                    let value = obj.downcast_ref::<PyStr>().ok_or_else(|| {
                        vm.new_type_error(format!(
                            r#""lineterminator" must be a string, not {}"#,
                            obj.class().name()
                        ))
                    })?;
                    // Store the full line terminator string. CPython accepts an
                    // arbitrary-length terminator; the manual writer paths emit it
                    // verbatim and the csv-core writer path appends it after a
                    // sentinel terminator (see `writerow`).
                    parse_lineterminator(vm, value)?.to_owned()
                }
                None => "\r\n".to_owned(),
            };
            let quotechar = match &self.quotechar {
                Some(obj) => parse_char(vm, obj, "quotechar", true)?,
                None => Some(b'"'),
            };
            let quoting = match &self.quoting {
                Some(obj) => {
                    if !obj.class().is(vm.ctx.types.int_type) {
                        return Err(vm.new_type_error(format!(
                            r#""quoting" must be an integer, not {}"#,
                            obj.class().name()
                        )));
                    }
                    obj.downcast_ref::<PyInt>()
                        .unwrap()
                        .try_to_primitive::<i32>(vm)?
                }
                None => QuoteStyle::Minimal as i32,
            };
            let skipinitialspace = match &self.skipinitialspace {
                Some(obj) => obj.try_to_bool(vm)?,
                None => false,
            };
            let strict = match &self.strict {
                Some(obj) => obj.try_to_bool(vm)?,
                None => false,
            };

            // Validate the quoting value only after all field conversions.
            let quoting = QuoteStyle::try_from(quoting as isize)
                .map_err(|_| vm.new_type_error(r#"bad "quoting" value"#))?;
            let quoting = if quotechar.is_none() && self.quoting.is_none() {
                QuoteStyle::None
            } else {
                quoting
            };
            if quotechar.is_none() && quoting != QuoteStyle::None {
                return Err(vm.new_type_error("quotechar must be set if quoting enabled"));
            }
            let dialect = PyDialect {
                delimiter,
                quotechar,
                escapechar,
                doublequote,
                skipinitialspace,
                lineterminator,
                quoting,
                strict,
            };
            validate_dialect(vm, &dialect)?;
            Ok(dialect)
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
        fn line_num(zelf: &Py<Self>) -> u64 {
            zelf.state.lock().line_num
        }

        #[pygetset]
        fn dialect(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyDialect {
            zelf.dialect.clone()
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

    #[pyclass(no_attr, module = "_csv", name = "writer", traverse)]
    #[derive(PyPayload)]
    pub(super) struct Writer {
        write: PyObjectRef,
        #[pytraverse(skip)]
        state: PyMutex<()>,
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
                || (dialect.doublequote && dialect.quotechar == Some(byte))
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
        fn get_dialect(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyDialect {
            zelf.dialect.clone()
        }

        #[pymethod]
        fn writerow(zelf: &Py<Self>, row: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let _state = zelf.state.lock();
            let row: ArgIterable =
                ArgIterable::try_from_object(vm, row.to_owned()).map_err(|_e| {
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
                    output.push(zelf.dialect.delimiter);
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

                if single_field
                    && data.is_empty()
                    && (zelf.dialect.quoting == QuoteStyle::None
                        || (is_none
                            && matches!(
                                zelf.dialect.quoting,
                                QuoteStyle::Strings | QuoteStyle::Notnull
                            )))
                {
                    return Err(new_csv_error(
                        vm,
                        "single empty field record must be quoted",
                    ));
                }

                if data.is_empty()
                    && zelf.dialect.delimiter == b' '
                    && zelf.dialect.skipinitialspace
                    && (zelf.dialect.quoting == QuoteStyle::None
                        || (is_none
                            && matches!(
                                zelf.dialect.quoting,
                                QuoteStyle::Strings | QuoteStyle::Notnull
                            )))
                {
                    return Err(new_csv_error(
                        vm,
                        "empty field must be quoted if delimiter is a space and skipinitialspace is true",
                    ));
                }

                let mut should_quote = match zelf.dialect.quoting {
                    QuoteStyle::All => true,
                    QuoteStyle::Nonnumeric => !PyNumber::check(&field),
                    QuoteStyle::Strings => is_str,
                    QuoteStyle::Notnull => !is_none,
                    QuoteStyle::Minimal | QuoteStyle::None => false,
                };

                if zelf.dialect.quoting != QuoteStyle::None
                    && ((data.is_empty()
                        && zelf.dialect.delimiter == b' '
                        && zelf.dialect.skipinitialspace)
                        || (single_field && data.is_empty())
                        || (!should_quote && field_needs_quotes(data, &zelf.dialect)))
                {
                    should_quote = true;
                }

                if should_quote {
                    write_quoted_field(&mut output, data, &zelf.dialect, vm)?;
                } else {
                    write_unquoted_field(&mut output, data, &zelf.dialect, vm)?;
                }
            }

            write_lineterminator(&mut output, &zelf.dialect.lineterminator);
            let s =
                core::str::from_utf8(&output).map_err(|e| new_not_utf8_error(vm, &output, e))?;
            zelf.write.call((s,), vm)
        }

        #[pymethod]
        fn writerows(zelf: &Py<Self>, rows: ArgIterable, vm: &VirtualMachine) -> PyResult<()> {
            for row in rows.iter(vm)? {
                Self::writerow(zelf, row?, vm)?;
            }
            Ok(())
        }
    }
}
