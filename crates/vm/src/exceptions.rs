use self::types::{PyBaseException, PyBaseExceptionRef};
use crate::common::lock::PyRwLock;
use crate::object::{Traverse, TraverseFn};
use crate::{
    AsObject, Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject, VirtualMachine,
    builtins::{
        PyList, PyNone, PyStr, PyStrRef, PyTuple, PyTupleRef, PyType, PyTypeRef,
        traceback::{PyTraceback, PyTracebackRef},
    },
    class::{PyClassImpl, StaticType},
    convert::{ToPyException, ToPyObject},
    function::{ArgIterable, FuncArgs, IntoFuncArgs, PySetterValue},
    py_io::{self, Write},
    stdlib::sys,
    suggestion::offer_suggestions,
    types::{Callable, Constructor, Initializer, Representable},
};
use crossbeam_utils::atomic::AtomicCell;
use itertools::Itertools;
use std::{
    collections::HashSet,
    io::{self, BufRead, BufReader},
};

pub use super::exception_group::exception_group;

unsafe impl Traverse for PyBaseException {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.traceback.traverse(tracer_fn);
        self.cause.traverse(tracer_fn);
        self.context.traverse(tracer_fn);
        self.args.traverse(tracer_fn);
    }
}

impl core::fmt::Debug for PyBaseException {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // TODO: implement more detailed, non-recursive Debug formatter
        f.write_str("PyBaseException")
    }
}

impl PyPayload for PyBaseException {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.exceptions.base_exception_type
    }
}

impl VirtualMachine {
    // Why `impl VirtualMachine`?
    // These functions are natively free function in CPython - not methods of PyException

    /// Print exception chain by calling sys.excepthook
    pub fn print_exception(&self, exc: PyBaseExceptionRef) {
        let vm = self;
        let write_fallback = |exc, errstr| {
            if let Ok(stderr) = sys::get_stderr(vm) {
                let mut stderr = py_io::PyWriter(stderr, vm);
                // if this fails stderr might be closed -- ignore it
                let _ = writeln!(stderr, "{errstr}");
                let _ = self.write_exception(&mut stderr, exc);
            } else {
                eprintln!("{errstr}\nlost sys.stderr");
                let _ = self.write_exception(&mut py_io::IoWriter(io::stderr()), exc);
            }
        };
        if let Ok(excepthook) = vm.sys_module.get_attr("excepthook", vm) {
            let (exc_type, exc_val, exc_tb) = vm.split_exception(exc.clone());
            if let Err(eh_exc) = excepthook.call((exc_type, exc_val, exc_tb), vm) {
                write_fallback(&eh_exc, "Error in sys.excepthook:");
                write_fallback(&exc, "Original exception was:");
            }
        } else {
            write_fallback(&exc, "missing sys.excepthook");
        }
    }

    pub fn write_exception<W: Write>(
        &self,
        output: &mut W,
        exc: &Py<PyBaseException>,
    ) -> Result<(), W::Error> {
        let seen = &mut HashSet::<usize>::new();
        self.write_exception_recursive(output, exc, seen)
    }

    fn write_exception_recursive<W: Write>(
        &self,
        output: &mut W,
        exc: &Py<PyBaseException>,
        seen: &mut HashSet<usize>,
    ) -> Result<(), W::Error> {
        // This function should not be called directly,
        // use `wite_exception` as a public interface.
        // It is similar to `print_exception_recursive` from `CPython`.
        seen.insert(exc.get_id());

        #[allow(clippy::manual_map)]
        if let Some((cause_or_context, msg)) = if let Some(cause) = exc.__cause__() {
            // This can be a special case: `raise e from e`,
            // we just ignore it and treat like `raise e` without any extra steps.
            Some((
                cause,
                "\nThe above exception was the direct cause of the following exception:\n",
            ))
        } else if let Some(context) = exc.__context__() {
            // This can be a special case:
            //   e = ValueError('e')
            //   e.__context__ = e
            // In this case, we just ignore
            // `__context__` part from going into recursion.
            Some((
                context,
                "\nDuring handling of the above exception, another exception occurred:\n",
            ))
        } else {
            None
        } {
            if !seen.contains(&cause_or_context.get_id()) {
                self.write_exception_recursive(output, &cause_or_context, seen)?;
                writeln!(output, "{msg}")?;
            } else {
                seen.insert(cause_or_context.get_id());
            }
        }

        self.write_exception_inner(output, exc)
    }

    /// Print exception with traceback
    pub fn write_exception_inner<W: Write>(
        &self,
        output: &mut W,
        exc: &Py<PyBaseException>,
    ) -> Result<(), W::Error> {
        let vm = self;
        if let Some(tb) = exc.traceback.read().clone() {
            writeln!(output, "Traceback (most recent call last):")?;
            for tb in tb.iter() {
                write_traceback_entry(output, &tb)?;
            }
        }

        let varargs = exc.args();
        let args_repr = vm.exception_args_as_string(varargs, true);

        let exc_class = exc.class();

        if exc_class.fast_issubclass(vm.ctx.exceptions.syntax_error) {
            return self.write_syntaxerror(output, exc, exc_class, &args_repr);
        }

        let exc_name = exc_class.name();
        match args_repr.len() {
            0 => write!(output, "{exc_name}"),
            1 => write!(output, "{}: {}", exc_name, args_repr[0]),
            _ => write!(
                output,
                "{}: ({})",
                exc_name,
                args_repr.into_iter().format(", "),
            ),
        }?;

        match offer_suggestions(exc, vm) {
            Some(suggestions) => writeln!(output, ". Did you mean: '{suggestions}'?"),
            None => writeln!(output),
        }
    }

    /// Format and write a SyntaxError
    /// This logic is derived from TracebackException._format_syntax_error
    ///
    /// The logic has support for `end_offset` to highlight a range in the source code,
    /// but it looks like `end_offset` is not used yet when SyntaxErrors are created.
    fn write_syntaxerror<W: Write>(
        &self,
        output: &mut W,
        exc: &Py<PyBaseException>,
        exc_type: &Py<PyType>,
        args_repr: &[PyRef<PyStr>],
    ) -> Result<(), W::Error> {
        let vm = self;
        debug_assert!(exc_type.fast_issubclass(vm.ctx.exceptions.syntax_error));

        let getattr = |attr: &'static str| exc.as_object().get_attr(attr, vm).ok();

        let maybe_lineno = getattr("lineno").map(|obj| {
            obj.str(vm)
                .unwrap_or_else(|_| vm.ctx.new_str("<lineno str() failed>"))
        });
        let maybe_filename = getattr("filename").and_then(|obj| obj.str(vm).ok());

        let maybe_text = getattr("text").map(|obj| {
            obj.str(vm)
                .unwrap_or_else(|_| vm.ctx.new_str("<text str() failed>"))
        });

        let mut filename_suffix = String::new();

        if let Some(lineno) = maybe_lineno {
            let filename = match maybe_filename {
                Some(filename) => filename,
                None => vm.ctx.new_str("<string>"),
            };
            writeln!(output, r##"  File "{filename}", line {lineno}"##,)?;
        } else if let Some(filename) = maybe_filename {
            filename_suffix = format!(" ({filename})");
        }

        if let Some(text) = maybe_text {
            // if text ends with \n, remove it
            let r_text = text.as_str().trim_end_matches('\n');
            let l_text = r_text.trim_start_matches([' ', '\n', '\x0c']); // \x0c is \f
            let spaces = (r_text.len() - l_text.len()) as isize;

            writeln!(output, "    {l_text}")?;

            let maybe_offset: Option<isize> =
                getattr("offset").and_then(|obj| obj.try_to_value::<isize>(vm).ok());

            if let Some(offset) = maybe_offset {
                let maybe_end_offset: Option<isize> =
                    getattr("end_offset").and_then(|obj| obj.try_to_value::<isize>(vm).ok());
                let maybe_end_lineno: Option<isize> =
                    getattr("end_lineno").and_then(|obj| obj.try_to_value::<isize>(vm).ok());
                let maybe_lineno_int: Option<isize> =
                    getattr("lineno").and_then(|obj| obj.try_to_value::<isize>(vm).ok());

                // Only show caret if end_lineno is same as lineno (or not set)
                let same_line = match (maybe_lineno_int, maybe_end_lineno) {
                    (Some(lineno), Some(end_lineno)) => lineno == end_lineno,
                    _ => true,
                };

                if same_line {
                    let mut end_offset = match maybe_end_offset {
                        Some(0) | None => offset,
                        Some(end_offset) => end_offset,
                    };

                    if offset == end_offset || end_offset == -1 {
                        end_offset = offset + 1;
                    }

                    // Convert 1-based column offset to 0-based index into stripped text
                    let colno = offset - 1 - spaces;
                    let end_colno = end_offset - 1 - spaces;
                    if colno >= 0 {
                        let caret_space = l_text
                            .chars()
                            .take(colno as usize)
                            .map(|c| if c.is_whitespace() { c } else { ' ' })
                            .collect::<String>();

                        let mut error_width = end_colno - colno;
                        if error_width < 1 {
                            error_width = 1;
                        }

                        writeln!(
                            output,
                            "    {}{}",
                            caret_space,
                            "^".repeat(error_width as usize)
                        )?;
                    }
                }
            }
        }

        let exc_name = exc_type.name();

        match args_repr.len() {
            0 => write!(output, "{exc_name}{filename_suffix}"),
            1 => write!(output, "{}: {}{}", exc_name, args_repr[0], filename_suffix),
            _ => write!(
                output,
                "{}: ({}){}",
                exc_name,
                args_repr.iter().format(", "),
                filename_suffix
            ),
        }?;

        match offer_suggestions(exc, vm) {
            Some(suggestions) => writeln!(output, ". Did you mean: '{suggestions}'?"),
            None => writeln!(output),
        }
    }

    fn exception_args_as_string(&self, varargs: PyTupleRef, str_single: bool) -> Vec<PyStrRef> {
        let vm = self;
        match varargs.len() {
            0 => vec![],
            1 => {
                let args0_repr = if str_single {
                    varargs[0]
                        .str(vm)
                        .unwrap_or_else(|_| PyStr::from("<element str() failed>").into_ref(&vm.ctx))
                } else {
                    varargs[0].repr(vm).unwrap_or_else(|_| {
                        PyStr::from("<element repr() failed>").into_ref(&vm.ctx)
                    })
                };
                vec![args0_repr]
            }
            _ => varargs
                .iter()
                .map(|vararg| {
                    vararg.repr(vm).unwrap_or_else(|_| {
                        PyStr::from("<element repr() failed>").into_ref(&vm.ctx)
                    })
                })
                .collect(),
        }
    }

    pub fn split_exception(
        &self,
        exc: PyBaseExceptionRef,
    ) -> (PyObjectRef, PyObjectRef, PyObjectRef) {
        let tb = exc.__traceback__().to_pyobject(self);
        let class = exc.class().to_owned();
        (class.into(), exc.into(), tb)
    }

    /// Similar to PyErr_NormalizeException in CPython
    pub fn normalize_exception(
        &self,
        exc_type: PyObjectRef,
        exc_val: PyObjectRef,
        exc_tb: PyObjectRef,
    ) -> PyResult<PyBaseExceptionRef> {
        let ctor = ExceptionCtor::try_from_object(self, exc_type)?;
        let exc = ctor.instantiate_value(exc_val, self)?;
        if let Some(tb) = Option::<PyTracebackRef>::try_from_object(self, exc_tb)? {
            exc.set_traceback_typed(Some(tb));
        }
        Ok(exc)
    }

    pub fn invoke_exception(
        &self,
        cls: PyTypeRef,
        args: Vec<PyObjectRef>,
    ) -> PyResult<PyBaseExceptionRef> {
        // TODO: fast-path built-in exceptions by directly instantiating them? Is that really worth it?
        let res = PyType::call(&cls, args.into_args(self), self)?;
        PyBaseExceptionRef::try_from_object(self, res)
    }
}

fn print_source_line<W: Write>(
    output: &mut W,
    filename: &str,
    lineno: usize,
) -> Result<(), W::Error> {
    // TODO: use io.open() method instead, when available, according to https://github.com/python/cpython/blob/main/Python/traceback.c#L393
    // TODO: support different encodings
    let file = match std::fs::File::open(filename) {
        Ok(file) => file,
        Err(_) => return Ok(()),
    };
    let file = BufReader::new(file);

    for (i, line) in file.lines().enumerate() {
        if i + 1 == lineno {
            if let Ok(line) = line {
                // Indented with 4 spaces
                writeln!(output, "    {}", line.trim_start())?;
            }
            return Ok(());
        }
    }

    Ok(())
}

/// Print exception occurrence location from traceback element
fn write_traceback_entry<W: Write>(
    output: &mut W,
    tb_entry: &Py<PyTraceback>,
) -> Result<(), W::Error> {
    let filename = tb_entry.frame.code.source_path.as_str();
    writeln!(
        output,
        r##"  File "{}", line {}, in {}"##,
        filename.trim_start_matches(r"\\?\"),
        tb_entry.lineno,
        tb_entry.frame.code.obj_name
    )?;
    print_source_line(output, filename, tb_entry.lineno.get())?;

    Ok(())
}

#[derive(Clone)]
pub enum ExceptionCtor {
    Class(PyTypeRef),
    Instance(PyBaseExceptionRef),
}

impl TryFromObject for ExceptionCtor {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        obj.downcast::<PyType>()
            .and_then(|cls| {
                if cls.fast_issubclass(vm.ctx.exceptions.base_exception_type) {
                    Ok(Self::Class(cls))
                } else {
                    Err(cls.into())
                }
            })
            .or_else(|obj| obj.downcast::<PyBaseException>().map(Self::Instance))
            .map_err(|obj| {
                vm.new_type_error(format!(
                    "exceptions must be classes or instances deriving from BaseException, not {}",
                    obj.class().name()
                ))
            })
    }
}

impl ExceptionCtor {
    pub fn instantiate(self, vm: &VirtualMachine) -> PyResult<PyBaseExceptionRef> {
        match self {
            Self::Class(cls) => vm.invoke_exception(cls, vec![]),
            Self::Instance(exc) => Ok(exc),
        }
    }

    pub fn instantiate_value(
        self,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyBaseExceptionRef> {
        let exc_inst = value.clone().downcast::<PyBaseException>().ok();
        match (self, exc_inst) {
            // both are instances; which would we choose?
            (Self::Instance(_exc_a), Some(_exc_b)) => {
                Err(vm.new_type_error("instance exception may not have a separate value"))
            }
            // if the "type" is an instance and the value isn't, use the "type"
            (Self::Instance(exc), None) => Ok(exc),
            // if the value is an instance of the type, use the instance value
            (Self::Class(cls), Some(exc)) if exc.fast_isinstance(&cls) => Ok(exc),
            // otherwise; construct an exception of the type using the value as args
            (Self::Class(cls), _) => {
                let args = match_class!(match value {
                    PyNone => vec![],
                    tup @ PyTuple => tup.to_vec(),
                    exc @ PyBaseException => exc.args().to_vec(),
                    obj => vec![obj],
                });
                vm.invoke_exception(cls, args)
            }
        }
    }
}

#[derive(Debug)]
pub struct ExceptionZoo {
    pub base_exception_type: &'static Py<PyType>,
    pub base_exception_group: &'static Py<PyType>,
    pub system_exit: &'static Py<PyType>,
    pub keyboard_interrupt: &'static Py<PyType>,
    pub generator_exit: &'static Py<PyType>,
    pub exception_type: &'static Py<PyType>,
    pub stop_iteration: &'static Py<PyType>,
    pub stop_async_iteration: &'static Py<PyType>,
    pub arithmetic_error: &'static Py<PyType>,
    pub floating_point_error: &'static Py<PyType>,
    pub overflow_error: &'static Py<PyType>,
    pub zero_division_error: &'static Py<PyType>,
    pub assertion_error: &'static Py<PyType>,
    pub attribute_error: &'static Py<PyType>,
    pub buffer_error: &'static Py<PyType>,
    pub eof_error: &'static Py<PyType>,
    pub import_error: &'static Py<PyType>,
    pub module_not_found_error: &'static Py<PyType>,
    pub lookup_error: &'static Py<PyType>,
    pub index_error: &'static Py<PyType>,
    pub key_error: &'static Py<PyType>,
    pub memory_error: &'static Py<PyType>,
    pub name_error: &'static Py<PyType>,
    pub unbound_local_error: &'static Py<PyType>,
    pub os_error: &'static Py<PyType>,
    pub blocking_io_error: &'static Py<PyType>,
    pub child_process_error: &'static Py<PyType>,
    pub connection_error: &'static Py<PyType>,
    pub broken_pipe_error: &'static Py<PyType>,
    pub connection_aborted_error: &'static Py<PyType>,
    pub connection_refused_error: &'static Py<PyType>,
    pub connection_reset_error: &'static Py<PyType>,
    pub file_exists_error: &'static Py<PyType>,
    pub file_not_found_error: &'static Py<PyType>,
    pub interrupted_error: &'static Py<PyType>,
    pub is_a_directory_error: &'static Py<PyType>,
    pub not_a_directory_error: &'static Py<PyType>,
    pub permission_error: &'static Py<PyType>,
    pub process_lookup_error: &'static Py<PyType>,
    pub timeout_error: &'static Py<PyType>,
    pub reference_error: &'static Py<PyType>,
    pub runtime_error: &'static Py<PyType>,
    pub not_implemented_error: &'static Py<PyType>,
    pub recursion_error: &'static Py<PyType>,
    pub python_finalization_error: &'static Py<PyType>,
    pub syntax_error: &'static Py<PyType>,
    pub incomplete_input_error: &'static Py<PyType>,
    pub indentation_error: &'static Py<PyType>,
    pub tab_error: &'static Py<PyType>,
    pub system_error: &'static Py<PyType>,
    pub type_error: &'static Py<PyType>,
    pub value_error: &'static Py<PyType>,
    pub unicode_error: &'static Py<PyType>,
    pub unicode_decode_error: &'static Py<PyType>,
    pub unicode_encode_error: &'static Py<PyType>,
    pub unicode_translate_error: &'static Py<PyType>,

    #[cfg(feature = "jit")]
    pub jit_error: &'static Py<PyType>,

    pub warning: &'static Py<PyType>,
    pub deprecation_warning: &'static Py<PyType>,
    pub pending_deprecation_warning: &'static Py<PyType>,
    pub runtime_warning: &'static Py<PyType>,
    pub syntax_warning: &'static Py<PyType>,
    pub user_warning: &'static Py<PyType>,
    pub future_warning: &'static Py<PyType>,
    pub import_warning: &'static Py<PyType>,
    pub unicode_warning: &'static Py<PyType>,
    pub bytes_warning: &'static Py<PyType>,
    pub resource_warning: &'static Py<PyType>,
    pub encoding_warning: &'static Py<PyType>,
}

macro_rules! extend_exception {
    (
        $exc_struct:ident,
        $ctx:expr,
        $class:expr
    ) => {
        extend_exception!($exc_struct, $ctx, $class, {});
    };
    (
        $exc_struct:ident,
        $ctx:expr,
        $class:expr,
        { $($name:expr => $value:expr),* $(,)* }
    ) => {
        $exc_struct::extend_class($ctx, $class);
        extend_class!($ctx, $class, {
            $($name => $value,)*
        });
    };
}

impl PyBaseException {
    pub(crate) fn new(args: Vec<PyObjectRef>, vm: &VirtualMachine) -> Self {
        Self {
            traceback: PyRwLock::new(None),
            cause: PyRwLock::new(None),
            context: PyRwLock::new(None),
            suppress_context: AtomicCell::new(false),
            args: PyRwLock::new(PyTuple::new_ref(args, &vm.ctx)),
        }
    }

    pub fn get_arg(&self, idx: usize) -> Option<PyObjectRef> {
        self.args.read().get(idx).cloned()
    }
}

#[pyclass(
    with(Py, PyRef, Constructor, Initializer, Representable),
    flags(BASETYPE, HAS_DICT)
)]
impl PyBaseException {
    #[pygetset]
    pub fn args(&self) -> PyTupleRef {
        self.args.read().clone()
    }

    #[pygetset(setter)]
    fn set_args(&self, args: ArgIterable, vm: &VirtualMachine) -> PyResult<()> {
        let args = args.iter(vm)?.collect::<PyResult<Vec<_>>>()?;
        *self.args.write() = PyTuple::new_ref(args, &vm.ctx);
        Ok(())
    }

    #[pygetset]
    pub fn __traceback__(&self) -> Option<PyTracebackRef> {
        self.traceback.read().clone()
    }

    #[pygetset(setter)]
    pub fn set___traceback__(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let traceback = if vm.is_none(&value) {
            None
        } else {
            match value.downcast::<PyTraceback>() {
                Ok(tb) => Some(tb),
                Err(_) => {
                    return Err(vm.new_type_error("__traceback__ must be a traceback or None"));
                }
            }
        };
        self.set_traceback_typed(traceback);
        Ok(())
    }

    // Helper method for internal use that doesn't require PyObjectRef
    pub(crate) fn set_traceback_typed(&self, traceback: Option<PyTracebackRef>) {
        *self.traceback.write() = traceback;
    }

    #[pygetset]
    pub fn __cause__(&self) -> Option<PyRef<Self>> {
        self.cause.read().clone()
    }

    #[pygetset(setter)]
    pub fn set___cause__(&self, cause: Option<PyRef<Self>>) {
        let mut c = self.cause.write();
        self.set_suppress_context(true);
        *c = cause;
    }

    #[pygetset]
    pub fn __context__(&self) -> Option<PyRef<Self>> {
        self.context.read().clone()
    }

    #[pygetset(setter)]
    pub fn set___context__(&self, context: Option<PyRef<Self>>) {
        *self.context.write() = context;
    }

    #[pygetset]
    pub(super) fn __suppress_context__(&self) -> bool {
        self.suppress_context.load()
    }

    #[pygetset(name = "__suppress_context__", setter)]
    fn set_suppress_context(&self, suppress_context: bool) {
        self.suppress_context.store(suppress_context);
    }
}

#[pyclass]
impl Py<PyBaseException> {
    #[pymethod]
    pub(super) fn __str__(&self, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        let str_args = vm.exception_args_as_string(self.args(), true);
        Ok(match str_args.into_iter().exactly_one() {
            Err(i) if i.len() == 0 => vm.ctx.empty_str.to_owned(),
            Ok(s) => s,
            Err(i) => PyStr::from(format!("({})", i.format(", "))).into_ref(&vm.ctx),
        })
    }
}

#[pyclass]
impl PyRef<PyBaseException> {
    #[pymethod]
    fn with_traceback(self, tb: Option<PyTracebackRef>) -> PyResult<Self> {
        *self.traceback.write() = tb;
        Ok(self)
    }

    #[pymethod]
    fn add_note(self, note: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        let dict = self
            .as_object()
            .dict()
            .ok_or_else(|| vm.new_attribute_error("Exception object has no __dict__"))?;

        let notes = if let Ok(notes) = dict.get_item("__notes__", vm) {
            notes
        } else {
            let new_notes = vm.ctx.new_list(vec![]);
            dict.set_item("__notes__", new_notes.clone().into(), vm)?;
            new_notes.into()
        };

        let notes = notes
            .downcast::<PyList>()
            .map_err(|_| vm.new_type_error("__notes__ must be a list"))?;

        notes.borrow_vec_mut().push(note.into());
        Ok(())
    }

    #[pymethod]
    fn __reduce__(self, vm: &VirtualMachine) -> PyTupleRef {
        if let Some(dict) = self.as_object().dict().filter(|x| !x.is_empty()) {
            vm.new_tuple((self.class().to_owned(), self.args(), dict))
        } else {
            vm.new_tuple((self.class().to_owned(), self.args()))
        }
    }

    #[pymethod]
    fn __setstate__(self, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        if !vm.is_none(&state) {
            let dict = state
                .downcast::<crate::builtins::PyDict>()
                .map_err(|_| vm.new_type_error("state is not a dictionary"))?;

            for (key, value) in &dict {
                let key_str = key.str(vm)?;
                if key_str.as_str().starts_with("__") {
                    continue;
                }
                self.as_object().set_attr(&key_str, value.clone(), vm)?;
            }
        }
        Ok(vm.ctx.none())
    }
}

impl Constructor for PyBaseException {
    type Args = FuncArgs;

    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        if cls.is(Self::class(&vm.ctx)) && !args.kwargs.is_empty() {
            return Err(vm.new_type_error("BaseException() takes no keyword arguments"));
        }
        Self::new(args.args, vm)
            .into_ref_with_type(vm, cls)
            .map(Into::into)
    }

    fn py_new(_cls: &Py<PyType>, _args: FuncArgs, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
    }
}

impl Initializer for PyBaseException {
    type Args = FuncArgs;

    fn init(zelf: PyRef<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        *zelf.args.write() = PyTuple::new_ref(args.args, &vm.ctx);
        Ok(())
    }
}

impl Representable for PyBaseException {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let repr_args = vm.exception_args_as_string(zelf.args(), false);
        let cls = zelf.class();
        Ok(format!("{}({})", cls.name(), repr_args.iter().format(", ")))
    }
}

impl ExceptionZoo {
    pub(crate) fn init() -> Self {
        use self::types::*;

        let base_exception_type = PyBaseException::init_builtin_type();

        // Sorted By Hierarchy then alphabetized.
        let base_exception_group = PyBaseExceptionGroup::init_builtin_type();
        let system_exit = PySystemExit::init_builtin_type();
        let keyboard_interrupt = PyKeyboardInterrupt::init_builtin_type();
        let generator_exit = PyGeneratorExit::init_builtin_type();

        let exception_type = PyException::init_builtin_type();
        let stop_iteration = PyStopIteration::init_builtin_type();
        let stop_async_iteration = PyStopAsyncIteration::init_builtin_type();
        let arithmetic_error = PyArithmeticError::init_builtin_type();
        let floating_point_error = PyFloatingPointError::init_builtin_type();
        let overflow_error = PyOverflowError::init_builtin_type();
        let zero_division_error = PyZeroDivisionError::init_builtin_type();

        let assertion_error = PyAssertionError::init_builtin_type();
        let attribute_error = PyAttributeError::init_builtin_type();
        let buffer_error = PyBufferError::init_builtin_type();
        let eof_error = PyEOFError::init_builtin_type();

        let import_error = PyImportError::init_builtin_type();
        let module_not_found_error = PyModuleNotFoundError::init_builtin_type();

        let lookup_error = PyLookupError::init_builtin_type();
        let index_error = PyIndexError::init_builtin_type();
        let key_error = PyKeyError::init_builtin_type();

        let memory_error = PyMemoryError::init_builtin_type();

        let name_error = PyNameError::init_builtin_type();
        let unbound_local_error = PyUnboundLocalError::init_builtin_type();

        // os errors
        let os_error = PyOSError::init_builtin_type();
        let blocking_io_error = PyBlockingIOError::init_builtin_type();
        let child_process_error = PyChildProcessError::init_builtin_type();

        let connection_error = PyConnectionError::init_builtin_type();
        let broken_pipe_error = PyBrokenPipeError::init_builtin_type();
        let connection_aborted_error = PyConnectionAbortedError::init_builtin_type();
        let connection_refused_error = PyConnectionRefusedError::init_builtin_type();
        let connection_reset_error = PyConnectionResetError::init_builtin_type();

        let file_exists_error = PyFileExistsError::init_builtin_type();
        let file_not_found_error = PyFileNotFoundError::init_builtin_type();
        let interrupted_error = PyInterruptedError::init_builtin_type();
        let is_a_directory_error = PyIsADirectoryError::init_builtin_type();
        let not_a_directory_error = PyNotADirectoryError::init_builtin_type();
        let permission_error = PyPermissionError::init_builtin_type();
        let process_lookup_error = PyProcessLookupError::init_builtin_type();
        let timeout_error = PyTimeoutError::init_builtin_type();

        let reference_error = PyReferenceError::init_builtin_type();

        let runtime_error = PyRuntimeError::init_builtin_type();
        let not_implemented_error = PyNotImplementedError::init_builtin_type();
        let recursion_error = PyRecursionError::init_builtin_type();
        let python_finalization_error = PyPythonFinalizationError::init_builtin_type();

        let syntax_error = PySyntaxError::init_builtin_type();
        let incomplete_input_error = PyIncompleteInputError::init_builtin_type();
        let indentation_error = PyIndentationError::init_builtin_type();
        let tab_error = PyTabError::init_builtin_type();

        let system_error = PySystemError::init_builtin_type();
        let type_error = PyTypeError::init_builtin_type();
        let value_error = PyValueError::init_builtin_type();
        let unicode_error = PyUnicodeError::init_builtin_type();
        let unicode_decode_error = PyUnicodeDecodeError::init_builtin_type();
        let unicode_encode_error = PyUnicodeEncodeError::init_builtin_type();
        let unicode_translate_error = PyUnicodeTranslateError::init_builtin_type();

        #[cfg(feature = "jit")]
        let jit_error = PyJitError::init_builtin_type();

        let warning = PyWarning::init_builtin_type();
        let deprecation_warning = PyDeprecationWarning::init_builtin_type();
        let pending_deprecation_warning = PyPendingDeprecationWarning::init_builtin_type();
        let runtime_warning = PyRuntimeWarning::init_builtin_type();
        let syntax_warning = PySyntaxWarning::init_builtin_type();
        let user_warning = PyUserWarning::init_builtin_type();
        let future_warning = PyFutureWarning::init_builtin_type();
        let import_warning = PyImportWarning::init_builtin_type();
        let unicode_warning = PyUnicodeWarning::init_builtin_type();
        let bytes_warning = PyBytesWarning::init_builtin_type();
        let resource_warning = PyResourceWarning::init_builtin_type();
        let encoding_warning = PyEncodingWarning::init_builtin_type();

        Self {
            base_exception_type,
            base_exception_group,
            system_exit,
            keyboard_interrupt,
            generator_exit,
            exception_type,
            stop_iteration,
            stop_async_iteration,
            arithmetic_error,
            floating_point_error,
            overflow_error,
            zero_division_error,
            assertion_error,
            attribute_error,
            buffer_error,
            eof_error,
            import_error,
            module_not_found_error,
            lookup_error,
            index_error,
            key_error,
            memory_error,
            name_error,
            unbound_local_error,
            os_error,
            blocking_io_error,
            child_process_error,
            connection_error,
            broken_pipe_error,
            connection_aborted_error,
            connection_refused_error,
            connection_reset_error,
            file_exists_error,
            file_not_found_error,
            interrupted_error,
            is_a_directory_error,
            not_a_directory_error,
            permission_error,
            process_lookup_error,
            timeout_error,
            reference_error,
            runtime_error,
            not_implemented_error,
            recursion_error,
            python_finalization_error,
            syntax_error,
            incomplete_input_error,
            indentation_error,
            tab_error,
            system_error,
            type_error,
            value_error,
            unicode_error,
            unicode_decode_error,
            unicode_encode_error,
            unicode_translate_error,

            #[cfg(feature = "jit")]
            jit_error,

            warning,
            deprecation_warning,
            pending_deprecation_warning,
            runtime_warning,
            syntax_warning,
            user_warning,
            future_warning,
            import_warning,
            unicode_warning,
            bytes_warning,
            resource_warning,
            encoding_warning,
        }
    }

    // TODO: remove it after fixing `errno` / `winerror` problem
    #[allow(clippy::redundant_clone)]
    pub fn extend(ctx: &Context) {
        use self::types::*;

        let excs = &ctx.exceptions;

        PyBaseException::extend_class(ctx, excs.base_exception_type);

        // Sorted By Hierarchy then alphabetized.
        extend_exception!(PyBaseExceptionGroup, ctx, excs.base_exception_group, {
            "message" => ctx.new_readonly_getset("message", excs.base_exception_group, make_arg_getter(0)),
            "exceptions" => ctx.new_readonly_getset("exceptions", excs.base_exception_group, make_arg_getter(1)),
        });

        extend_exception!(PySystemExit, ctx, excs.system_exit, {
            "code" => ctx.new_readonly_getset("code", excs.system_exit, system_exit_code),
        });
        extend_exception!(PyKeyboardInterrupt, ctx, excs.keyboard_interrupt);
        extend_exception!(PyGeneratorExit, ctx, excs.generator_exit);

        extend_exception!(PyException, ctx, excs.exception_type);

        extend_exception!(PyStopIteration, ctx, excs.stop_iteration, {
            "value" => ctx.none(),
        });
        extend_exception!(PyStopAsyncIteration, ctx, excs.stop_async_iteration);

        extend_exception!(PyArithmeticError, ctx, excs.arithmetic_error);
        extend_exception!(PyFloatingPointError, ctx, excs.floating_point_error);
        extend_exception!(PyOverflowError, ctx, excs.overflow_error);
        extend_exception!(PyZeroDivisionError, ctx, excs.zero_division_error);

        extend_exception!(PyAssertionError, ctx, excs.assertion_error);
        extend_exception!(PyAttributeError, ctx, excs.attribute_error, {
            "name" => ctx.none(),
            "obj" => ctx.none(),
        });
        extend_exception!(PyBufferError, ctx, excs.buffer_error);
        extend_exception!(PyEOFError, ctx, excs.eof_error);

        extend_exception!(PyImportError, ctx, excs.import_error, {
            "msg" => ctx.new_readonly_getset("msg", excs.import_error, make_arg_getter(0)),
        });
        extend_exception!(PyModuleNotFoundError, ctx, excs.module_not_found_error);

        extend_exception!(PyLookupError, ctx, excs.lookup_error);
        extend_exception!(PyIndexError, ctx, excs.index_error);

        extend_exception!(PyKeyError, ctx, excs.key_error);

        extend_exception!(PyMemoryError, ctx, excs.memory_error);
        extend_exception!(PyNameError, ctx, excs.name_error, {
            "name" => ctx.none(),
        });
        extend_exception!(PyUnboundLocalError, ctx, excs.unbound_local_error);

        // os errors:
        // PyOSError now uses struct fields with pygetset, no need for dynamic attributes
        extend_exception!(PyOSError, ctx, excs.os_error);

        extend_exception!(PyBlockingIOError, ctx, excs.blocking_io_error);
        extend_exception!(PyChildProcessError, ctx, excs.child_process_error);

        extend_exception!(PyConnectionError, ctx, excs.connection_error);
        extend_exception!(PyBrokenPipeError, ctx, excs.broken_pipe_error);
        extend_exception!(PyConnectionAbortedError, ctx, excs.connection_aborted_error);
        extend_exception!(PyConnectionRefusedError, ctx, excs.connection_refused_error);
        extend_exception!(PyConnectionResetError, ctx, excs.connection_reset_error);

        extend_exception!(PyFileExistsError, ctx, excs.file_exists_error);
        extend_exception!(PyFileNotFoundError, ctx, excs.file_not_found_error);
        extend_exception!(PyInterruptedError, ctx, excs.interrupted_error);
        extend_exception!(PyIsADirectoryError, ctx, excs.is_a_directory_error);
        extend_exception!(PyNotADirectoryError, ctx, excs.not_a_directory_error);
        extend_exception!(PyPermissionError, ctx, excs.permission_error);
        extend_exception!(PyProcessLookupError, ctx, excs.process_lookup_error);
        extend_exception!(PyTimeoutError, ctx, excs.timeout_error);

        extend_exception!(PyReferenceError, ctx, excs.reference_error);
        extend_exception!(PyRuntimeError, ctx, excs.runtime_error);
        extend_exception!(PyNotImplementedError, ctx, excs.not_implemented_error);
        extend_exception!(PyRecursionError, ctx, excs.recursion_error);
        extend_exception!(
            PyPythonFinalizationError,
            ctx,
            excs.python_finalization_error
        );

        extend_exception!(PySyntaxError, ctx, excs.syntax_error, {
            "msg" => ctx.new_static_getset(
                "msg",
                excs.syntax_error,
                make_arg_getter(0),
                syntax_error_set_msg,
            ),
            // TODO: members
            "filename" => ctx.none(),
            "lineno" => ctx.none(),
            "end_lineno" => ctx.none(),
            "offset" => ctx.none(),
            "end_offset" => ctx.none(),
            "text" => ctx.none(),
        });
        extend_exception!(PyIncompleteInputError, ctx, excs.incomplete_input_error);
        extend_exception!(PyIndentationError, ctx, excs.indentation_error);
        extend_exception!(PyTabError, ctx, excs.tab_error);

        extend_exception!(PySystemError, ctx, excs.system_error);
        extend_exception!(PyTypeError, ctx, excs.type_error);
        extend_exception!(PyValueError, ctx, excs.value_error);
        extend_exception!(PyUnicodeError, ctx, excs.unicode_error);
        extend_exception!(PyUnicodeDecodeError, ctx, excs.unicode_decode_error);
        extend_exception!(PyUnicodeEncodeError, ctx, excs.unicode_encode_error);
        extend_exception!(PyUnicodeTranslateError, ctx, excs.unicode_translate_error);

        #[cfg(feature = "jit")]
        extend_exception!(PyJitError, ctx, excs.jit_error);

        extend_exception!(PyWarning, ctx, excs.warning);
        extend_exception!(PyDeprecationWarning, ctx, excs.deprecation_warning);
        extend_exception!(
            PyPendingDeprecationWarning,
            ctx,
            excs.pending_deprecation_warning
        );
        extend_exception!(PyRuntimeWarning, ctx, excs.runtime_warning);
        extend_exception!(PySyntaxWarning, ctx, excs.syntax_warning);
        extend_exception!(PyUserWarning, ctx, excs.user_warning);
        extend_exception!(PyFutureWarning, ctx, excs.future_warning);
        extend_exception!(PyImportWarning, ctx, excs.import_warning);
        extend_exception!(PyUnicodeWarning, ctx, excs.unicode_warning);
        extend_exception!(PyBytesWarning, ctx, excs.bytes_warning);
        extend_exception!(PyResourceWarning, ctx, excs.resource_warning);
        extend_exception!(PyEncodingWarning, ctx, excs.encoding_warning);
    }
}

fn make_arg_getter(idx: usize) -> impl Fn(PyBaseExceptionRef) -> Option<PyObjectRef> {
    move |exc| exc.get_arg(idx)
}

fn syntax_error_set_msg(
    exc: PyBaseExceptionRef,
    value: PySetterValue,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let mut args = exc.args.write();
    let mut new_args = args.as_slice().to_vec();
    // Ensure the message slot at index 0 always exists for SyntaxError.args.
    if new_args.is_empty() {
        new_args.push(vm.ctx.none());
    }
    match value {
        PySetterValue::Assign(value) => new_args[0] = value,
        PySetterValue::Delete => new_args[0] = vm.ctx.none(),
    }
    *args = PyTuple::new_ref(new_args, &vm.ctx);
    Ok(())
}

fn system_exit_code(exc: PyBaseExceptionRef) -> Option<PyObjectRef> {
    // SystemExit.code based on args length:
    // - size == 0: code is None
    // - size == 1: code is args[0]
    // - size > 1: code is args (the whole tuple)
    let args = exc.args.read();
    match args.len() {
        0 => None,
        1 => Some(args.first().unwrap().clone()),
        _ => Some(args.as_object().to_owned()),
    }
}

#[cfg(feature = "serde")]
pub struct SerializeException<'vm, 's> {
    vm: &'vm VirtualMachine,
    exc: &'s Py<PyBaseException>,
}

#[cfg(feature = "serde")]
impl<'vm, 's> SerializeException<'vm, 's> {
    pub fn new(vm: &'vm VirtualMachine, exc: &'s Py<PyBaseException>) -> Self {
        SerializeException { vm, exc }
    }
}

#[cfg(feature = "serde")]
pub struct SerializeExceptionOwned<'vm> {
    vm: &'vm VirtualMachine,
    exc: PyBaseExceptionRef,
}

#[cfg(feature = "serde")]
impl serde::Serialize for SerializeExceptionOwned<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let Self { vm, exc } = self;
        SerializeException::new(vm, exc).serialize(s)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for SerializeException<'_, '_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::*;

        let mut struc = s.serialize_struct("PyBaseException", 7)?;
        struc.serialize_field("exc_type", &*self.exc.class().name())?;
        let tbs = {
            struct Tracebacks(PyTracebackRef);
            impl serde::Serialize for Tracebacks {
                fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                    let mut s = s.serialize_seq(None)?;
                    for tb in self.0.iter() {
                        s.serialize_element(&**tb)?;
                    }
                    s.end()
                }
            }
            self.exc.__traceback__().map(Tracebacks)
        };
        struc.serialize_field("traceback", &tbs)?;
        struc.serialize_field(
            "cause",
            &self
                .exc
                .__cause__()
                .map(|exc| SerializeExceptionOwned { vm: self.vm, exc }),
        )?;
        struc.serialize_field(
            "context",
            &self
                .exc
                .__context__()
                .map(|exc| SerializeExceptionOwned { vm: self.vm, exc }),
        )?;
        struc.serialize_field("suppress_context", &self.exc.__suppress_context__())?;

        let args = {
            struct Args<'vm>(&'vm VirtualMachine, PyTupleRef);
            impl serde::Serialize for Args<'_> {
                fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                    s.collect_seq(
                        self.1
                            .iter()
                            .map(|arg| crate::py_serde::PyObjectSerializer::new(self.0, arg)),
                    )
                }
            }
            Args(self.vm, self.exc.args())
        };
        struc.serialize_field("args", &args)?;

        let rendered = {
            let mut rendered = String::new();
            self.vm
                .write_exception(&mut rendered, self.exc)
                .map_err(S::Error::custom)?;
            rendered
        };
        struc.serialize_field("rendered", &rendered)?;

        struc.end()
    }
}

pub fn cstring_error(vm: &VirtualMachine) -> PyBaseExceptionRef {
    vm.new_value_error("embedded null character")
}

impl ToPyException for alloc::ffi::NulError {
    fn to_pyexception(&self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        cstring_error(vm)
    }
}

#[cfg(windows)]
impl<C> ToPyException for widestring::error::ContainsNul<C> {
    fn to_pyexception(&self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        cstring_error(vm)
    }
}

#[cfg(any(unix, windows, target_os = "wasi"))]
pub(crate) fn errno_to_exc_type(errno: i32, vm: &VirtualMachine) -> Option<&'static Py<PyType>> {
    use crate::stdlib::errno::errors;
    let excs = &vm.ctx.exceptions;
    match errno {
        #[allow(unreachable_patterns)] // EAGAIN is sometimes the same as EWOULDBLOCK
        errors::EWOULDBLOCK | errors::EAGAIN => Some(excs.blocking_io_error),
        errors::EALREADY => Some(excs.blocking_io_error),
        errors::EINPROGRESS => Some(excs.blocking_io_error),
        errors::EPIPE => Some(excs.broken_pipe_error),
        #[cfg(not(target_os = "wasi"))]
        errors::ESHUTDOWN => Some(excs.broken_pipe_error),
        errors::ECHILD => Some(excs.child_process_error),
        errors::ECONNABORTED => Some(excs.connection_aborted_error),
        errors::ECONNREFUSED => Some(excs.connection_refused_error),
        errors::ECONNRESET => Some(excs.connection_reset_error),
        errors::EEXIST => Some(excs.file_exists_error),
        errors::ENOENT => Some(excs.file_not_found_error),
        errors::EISDIR => Some(excs.is_a_directory_error),
        errors::ENOTDIR => Some(excs.not_a_directory_error),
        errors::EINTR => Some(excs.interrupted_error),
        errors::EACCES => Some(excs.permission_error),
        errors::EPERM => Some(excs.permission_error),
        errors::ESRCH => Some(excs.process_lookup_error),
        errors::ETIMEDOUT => Some(excs.timeout_error),
        _ => None,
    }
}

#[cfg(all(target_arch = "wasm32", not(target_os = "wasi")))]
pub(crate) fn errno_to_exc_type(_errno: i32, _vm: &VirtualMachine) -> Option<&'static Py<PyType>> {
    None
}

pub(crate) use types::{OSErrorBuilder, ToOSErrorBuilder};

pub(super) mod types {
    use crate::common::lock::PyRwLock;
    use crate::object::{MaybeTraverse, Traverse, TraverseFn};
    #[cfg_attr(target_arch = "wasm32", allow(unused_imports))]
    use crate::{
        AsObject, Py, PyAtomicRef, PyObject, PyObjectRef, PyPayload, PyRef, PyResult,
        VirtualMachine,
        builtins::{
            PyInt, PyStrRef, PyTupleRef, PyType, PyTypeRef, traceback::PyTracebackRef,
            tuple::IntoPyTuple,
        },
        convert::ToPyObject,
        convert::ToPyResult,
        function::{ArgBytesLike, FuncArgs, KwArgs},
        types::{Constructor, Initializer},
    };
    use crossbeam_utils::atomic::AtomicCell;
    use itertools::Itertools;
    use rustpython_common::str::UnicodeEscapeCodepoint;

    pub(crate) trait ToOSErrorBuilder {
        fn to_os_error_builder(&self, vm: &VirtualMachine) -> OSErrorBuilder;
    }

    pub struct OSErrorBuilder {
        exc_type: PyTypeRef,
        errno: Option<i32>,
        strerror: Option<PyObjectRef>,
        filename: Option<PyObjectRef>,
        #[cfg(windows)]
        winerror: Option<PyObjectRef>,
        filename2: Option<PyObjectRef>,
    }

    impl OSErrorBuilder {
        #[must_use]
        pub fn with_subtype(
            exc_type: PyTypeRef,
            errno: Option<i32>,
            strerror: impl ToPyObject,
            vm: &VirtualMachine,
        ) -> Self {
            let strerror = strerror.to_pyobject(vm);
            Self {
                exc_type,
                errno,
                strerror: Some(strerror),
                filename: None,
                #[cfg(windows)]
                winerror: None,
                filename2: None,
            }
        }
        #[must_use]
        pub fn with_errno(errno: i32, strerror: impl ToPyObject, vm: &VirtualMachine) -> Self {
            let exc_type = crate::exceptions::errno_to_exc_type(errno, vm)
                .unwrap_or(vm.ctx.exceptions.os_error)
                .to_owned();
            Self::with_subtype(exc_type, Some(errno), strerror, vm)
        }

        // #[must_use]
        // pub(crate) fn errno(mut self, errno: i32) -> Self {
        //     self.errno.replace(errno);
        //     self
        // }

        #[must_use]
        pub(crate) fn filename(mut self, filename: PyObjectRef) -> Self {
            self.filename.replace(filename);
            self
        }

        #[must_use]
        pub(crate) fn filename2(mut self, filename: PyObjectRef) -> Self {
            self.filename2.replace(filename);
            self
        }

        #[must_use]
        #[cfg(windows)]
        pub(crate) fn winerror(mut self, winerror: PyObjectRef) -> Self {
            self.winerror.replace(winerror);
            self
        }

        pub fn build(self, vm: &VirtualMachine) -> PyRef<PyOSError> {
            let OSErrorBuilder {
                exc_type,
                errno,
                strerror,
                filename,
                #[cfg(windows)]
                winerror,
                filename2,
            } = self;

            let args = if let Some(errno) = errno {
                #[cfg(windows)]
                let winerror = winerror.to_pyobject(vm);
                #[cfg(not(windows))]
                let winerror = vm.ctx.none();

                vec![
                    errno.to_pyobject(vm),
                    strerror.to_pyobject(vm),
                    filename.to_pyobject(vm),
                    winerror,
                    filename2.to_pyobject(vm),
                ]
            } else {
                vec![strerror.to_pyobject(vm)]
            };

            let payload = PyOSError::py_new(&exc_type, args.clone().into(), vm)
                .expect("new_os_error usage error");
            let os_error = payload
                .into_ref_with_type(vm, exc_type)
                .expect("new_os_error usage error");
            PyOSError::slot_init(os_error.as_object().to_owned(), args.into(), vm)
                .expect("new_os_error usage error");
            os_error
        }
    }

    impl crate::convert::IntoPyException for OSErrorBuilder {
        fn into_pyexception(self, vm: &VirtualMachine) -> PyBaseExceptionRef {
            self.build(vm).upcast()
        }
    }

    // Re-export exception group types from dedicated module
    pub use crate::exception_group::types::PyBaseExceptionGroup;

    // This module is designed to be used as `use builtins::*;`.
    // Do not add any pub symbols not included in builtins module.
    // `PyBaseExceptionRef` is the only exception.

    pub type PyBaseExceptionRef = PyRef<PyBaseException>;

    // Sorted By Hierarchy then alphabetized.

    #[pyclass(module = false, name = "BaseException", traverse = "manual")]
    pub struct PyBaseException {
        pub(super) traceback: PyRwLock<Option<PyTracebackRef>>,
        pub(super) cause: PyRwLock<Option<PyRef<Self>>>,
        pub(super) context: PyRwLock<Option<PyRef<Self>>>,
        pub(super) suppress_context: AtomicCell<bool>,
        pub(super) args: PyRwLock<PyTupleRef>,
    }

    #[pyexception(name, base = PyBaseException, ctx = "system_exit")]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PySystemExit(PyBaseException);

    // SystemExit_init: has its own __init__ that sets the code attribute
    #[pyexception(with(Initializer))]
    impl PySystemExit {}

    impl Initializer for PySystemExit {
        type Args = FuncArgs;
        fn slot_init(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            // Call BaseException_init first (handles args)
            PyBaseException::slot_init(zelf, args, vm)
            // Note: code is computed dynamically via system_exit_code getter
            // so we don't need to set it here explicitly
        }

        fn init(_zelf: PyRef<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
            unreachable!("slot_init is defined")
        }
    }

    #[pyexception(name, base = PyBaseException, ctx = "generator_exit", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyGeneratorExit(PyBaseException);

    #[pyexception(name, base = PyBaseException, ctx = "keyboard_interrupt", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyKeyboardInterrupt(PyBaseException);

    #[pyexception(name, base = PyBaseException, ctx = "exception_type", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyException(PyBaseException);

    #[pyexception(name, base = PyException, ctx = "stop_iteration")]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyStopIteration(PyException);

    #[pyexception(with(Initializer))]
    impl PyStopIteration {}

    impl Initializer for PyStopIteration {
        type Args = FuncArgs;
        fn slot_init(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            zelf.set_attr("value", vm.unwrap_or_none(args.args.first().cloned()), vm)?;
            Ok(())
        }

        fn init(_zelf: PyRef<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
            unreachable!("slot_init is defined")
        }
    }

    #[pyexception(name, base = PyException, ctx = "stop_async_iteration", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyStopAsyncIteration(PyException);

    #[pyexception(name, base = PyException, ctx = "arithmetic_error", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyArithmeticError(PyException);

    #[pyexception(name, base = PyArithmeticError, ctx = "floating_point_error", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyFloatingPointError(PyArithmeticError);
    #[pyexception(name, base = PyArithmeticError, ctx = "overflow_error", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyOverflowError(PyArithmeticError);

    #[pyexception(name, base = PyArithmeticError, ctx = "zero_division_error", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyZeroDivisionError(PyArithmeticError);

    #[pyexception(name, base = PyException, ctx = "assertion_error", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyAssertionError(PyException);

    #[pyexception(name, base = PyException, ctx = "attribute_error")]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyAttributeError(PyException);

    #[pyexception(with(Initializer))]
    impl PyAttributeError {}

    impl Initializer for PyAttributeError {
        type Args = FuncArgs;

        fn slot_init(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            // Only 'name' and 'obj' kwargs are allowed
            let mut kwargs = args.kwargs.clone();
            let name = kwargs.swap_remove("name");
            let obj = kwargs.swap_remove("obj");

            // Reject unknown kwargs
            if let Some(invalid_key) = kwargs.keys().next() {
                return Err(vm.new_type_error(format!(
                    "AttributeError() got an unexpected keyword argument '{invalid_key}'"
                )));
            }

            // Pass args without kwargs to BaseException_init
            let base_args = FuncArgs::new(args.args.clone(), KwArgs::default());
            PyBaseException::slot_init(zelf.clone(), base_args, vm)?;

            // Set attributes
            zelf.set_attr("name", vm.unwrap_or_none(name), vm)?;
            zelf.set_attr("obj", vm.unwrap_or_none(obj), vm)?;
            Ok(())
        }

        fn init(_zelf: PyRef<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
            unreachable!("slot_init is defined")
        }
    }

    #[pyexception(name, base = PyException, ctx = "buffer_error", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyBufferError(PyException);

    #[pyexception(name, base = PyException, ctx = "eof_error", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyEOFError(PyException);

    #[pyexception(name, base = PyException, ctx = "import_error")]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyImportError(PyException);

    #[pyexception(with(Initializer))]
    impl PyImportError {
        #[pymethod]
        fn __reduce__(exc: PyBaseExceptionRef, vm: &VirtualMachine) -> PyTupleRef {
            let obj = exc.as_object().to_owned();
            let mut result: Vec<PyObjectRef> = vec![
                obj.class().to_owned().into(),
                vm.new_tuple((exc.get_arg(0).unwrap(),)).into(),
            ];

            if let Some(dict) = obj.dict().filter(|x| !x.is_empty()) {
                result.push(dict.into());
            }

            result.into_pytuple(vm)
        }
    }

    impl Initializer for PyImportError {
        type Args = FuncArgs;

        fn slot_init(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            // Only 'name', 'path', 'name_from' kwargs are allowed
            let mut kwargs = args.kwargs.clone();
            let name = kwargs.swap_remove("name");
            let path = kwargs.swap_remove("path");
            let name_from = kwargs.swap_remove("name_from");

            // Check for any remaining invalid keyword arguments
            if let Some(invalid_key) = kwargs.keys().next() {
                return Err(vm.new_type_error(format!(
                    "'{invalid_key}' is an invalid keyword argument for ImportError"
                )));
            }

            let dict = zelf.dict().unwrap();
            dict.set_item("name", vm.unwrap_or_none(name), vm)?;
            dict.set_item("path", vm.unwrap_or_none(path), vm)?;
            dict.set_item("name_from", vm.unwrap_or_none(name_from), vm)?;
            PyBaseException::slot_init(zelf, args, vm)
        }

        fn init(_zelf: PyRef<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
            unreachable!("slot_init is defined")
        }
    }

    #[pyexception(name, base = PyImportError, ctx = "module_not_found_error", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyModuleNotFoundError(PyImportError);

    #[pyexception(name, base = PyException, ctx = "lookup_error", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyLookupError(PyException);

    #[pyexception(name, base = PyLookupError, ctx = "index_error", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyIndexError(PyLookupError);

    #[pyexception(name, base = PyLookupError, ctx = "key_error")]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyKeyError(PyLookupError);

    #[pyexception]
    impl PyKeyError {
        #[pymethod]
        fn __str__(zelf: &Py<PyBaseException>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            let args = zelf.args();
            Ok(if args.len() == 1 {
                vm.exception_args_as_string(args, false)
                    .into_iter()
                    .exactly_one()
                    .unwrap()
            } else {
                zelf.__str__(vm)?
            })
        }
    }

    #[pyexception(name, base = PyException, ctx = "memory_error", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyMemoryError(PyException);

    #[pyexception(name, base = PyException, ctx = "name_error")]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyNameError(PyException);

    // NameError_init: handles the .name. kwarg
    #[pyexception(with(Initializer))]
    impl PyNameError {}

    impl Initializer for PyNameError {
        type Args = FuncArgs;
        fn slot_init(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            // Only 'name' kwarg is allowed
            let mut kwargs = args.kwargs.clone();
            let name = kwargs.swap_remove("name");

            // Reject unknown kwargs
            if let Some(invalid_key) = kwargs.keys().next() {
                return Err(vm.new_type_error(format!(
                    "NameError() got an unexpected keyword argument '{invalid_key}'"
                )));
            }

            // Pass args without kwargs to BaseException_init
            let base_args = FuncArgs::new(args.args.clone(), KwArgs::default());
            PyBaseException::slot_init(zelf.clone(), base_args, vm)?;

            // Set name attribute if provided
            if let Some(name) = name {
                zelf.set_attr("name", name, vm)?;
            }
            Ok(())
        }

        fn init(_zelf: PyRef<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
            unreachable!("slot_init is defined")
        }
    }

    #[pyexception(name, base = PyNameError, ctx = "unbound_local_error", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyUnboundLocalError(PyNameError);

    #[pyexception(name, base = PyException, ctx = "os_error")]
    #[repr(C)]
    pub struct PyOSError {
        base: PyException,
        errno: PyAtomicRef<Option<PyObject>>,
        strerror: PyAtomicRef<Option<PyObject>>,
        filename: PyAtomicRef<Option<PyObject>>,
        filename2: PyAtomicRef<Option<PyObject>>,
        #[cfg(windows)]
        winerror: PyAtomicRef<Option<PyObject>>,
        // For BlockingIOError: characters written before blocking occurred
        // -1 means not set (AttributeError when accessed)
        written: AtomicCell<isize>,
    }

    impl crate::class::PySubclass for PyOSError {
        type Base = PyException;
        fn as_base(&self) -> &Self::Base {
            &self.base
        }
    }

    impl core::fmt::Debug for PyOSError {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("PyOSError").finish_non_exhaustive()
        }
    }

    unsafe impl Traverse for PyOSError {
        fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
            self.base.try_traverse(tracer_fn);
            if let Some(obj) = self.errno.deref() {
                tracer_fn(obj);
            }
            if let Some(obj) = self.strerror.deref() {
                tracer_fn(obj);
            }
            if let Some(obj) = self.filename.deref() {
                tracer_fn(obj);
            }
            if let Some(obj) = self.filename2.deref() {
                tracer_fn(obj);
            }
            #[cfg(windows)]
            if let Some(obj) = self.winerror.deref() {
                tracer_fn(obj);
            }
        }
    }

    // OS Errors:
    impl Constructor for PyOSError {
        type Args = FuncArgs;

        fn py_new(_cls: &Py<PyType>, args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
            let len = args.args.len();
            // CPython only sets errno/strerror when args len is 2-5
            let (errno, strerror) = if (2..=5).contains(&len) {
                (Some(args.args[0].clone()), Some(args.args[1].clone()))
            } else {
                (None, None)
            };
            let filename = if (3..=5).contains(&len) {
                Some(args.args[2].clone())
            } else {
                None
            };
            let filename2 = if len == 5 {
                args.args.get(4).cloned()
            } else {
                None
            };
            // Truncate args for base exception when 3-5 args
            let base_args = if (3..=5).contains(&len) {
                args.args[..2].to_vec()
            } else {
                args.args.to_vec()
            };
            let base_exception = PyBaseException::new(base_args, vm);
            Ok(Self {
                base: PyException(base_exception),
                errno: errno.into(),
                strerror: strerror.into(),
                filename: filename.into(),
                filename2: filename2.into(),
                #[cfg(windows)]
                winerror: None.into(),
                written: AtomicCell::new(-1),
            })
        }

        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            // We need this method, because of how `CPython` copies `init`
            // from `BaseException` in `SimpleExtendsException` macro.
            // See: `BaseException_new`
            if *cls.name() == *vm.ctx.exceptions.os_error.name() {
                let args_vec = args.args.to_vec();
                let len = args_vec.len();
                if (2..=5).contains(&len) {
                    let errno = &args_vec[0];
                    if let Some(error) = errno
                        .downcast_ref::<PyInt>()
                        .and_then(|errno| errno.try_to_primitive::<i32>(vm).ok())
                        .and_then(|errno| super::errno_to_exc_type(errno, vm))
                        .and_then(|typ| vm.invoke_exception(typ.to_owned(), args_vec).ok())
                    {
                        return error.to_pyresult(vm);
                    }
                }
            }
            let payload = Self::py_new(&cls, args, vm)?;
            payload.into_ref_with_type(vm, cls).map(Into::into)
        }
    }

    impl Initializer for PyOSError {
        type Args = FuncArgs;

        fn slot_init(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            let len = args.args.len();
            let mut new_args = args;

            // All OSError subclasses use #[repr(transparent)] wrapping PyOSError,
            // so we can safely access the PyOSError fields through pointer cast
            // SAFETY: All OSError subclasses (FileNotFoundError, etc.) are
            // #[repr(transparent)] wrappers around PyOSError with identical memory layout
            #[allow(deprecated)]
            let exc: &Py<PyOSError> = zelf.downcast_ref::<PyOSError>().unwrap();

            // Check if this is BlockingIOError - need to handle characters_written
            let is_blocking_io_error =
                zelf.class()
                    .is(vm.ctx.exceptions.blocking_io_error.as_ref());

            // SAFETY: slot_init is called during object initialization,
            // so fields are None and swap result can be safely ignored
            let mut set_filename = true;
            if len <= 5 {
                // Only set errno/strerror when args len is 2-5
                if 2 <= len {
                    let _ = unsafe { exc.errno.swap(Some(new_args.args[0].clone())) };
                    let _ = unsafe { exc.strerror.swap(Some(new_args.args[1].clone())) };
                }
                if 3 <= len {
                    let third_arg = &new_args.args[2];
                    // BlockingIOError's 3rd argument can be the number of characters written
                    if is_blocking_io_error
                        && !vm.is_none(third_arg)
                        && crate::protocol::PyNumber::check(third_arg)
                        && let Ok(written) = third_arg.try_index(vm)
                        && let Ok(n) = written.try_to_primitive::<isize>(vm)
                    {
                        exc.written.store(n);
                        set_filename = false;
                        // Clear filename that was set in py_new
                        let _ = unsafe { exc.filename.swap(None) };
                    }
                    if set_filename {
                        let _ = unsafe { exc.filename.swap(Some(third_arg.clone())) };
                    }
                }
                #[cfg(windows)]
                if 4 <= len {
                    let winerror = new_args.args.get(3).cloned();
                    // Store original winerror
                    let _ = unsafe { exc.winerror.swap(winerror.clone()) };

                    // Convert winerror to errno and update errno + args[0]
                    if let Some(errno) = winerror
                        .as_ref()
                        .and_then(|w| w.downcast_ref::<crate::builtins::PyInt>())
                        .and_then(|w| w.try_to_primitive::<i32>(vm).ok())
                        .map(crate::common::os::winerror_to_errno)
                    {
                        let errno_obj = vm.new_pyobj(errno);
                        let _ = unsafe { exc.errno.swap(Some(errno_obj.clone())) };
                        new_args.args[0] = errno_obj;
                    }
                }
                if len == 5 {
                    let _ = unsafe { exc.filename2.swap(new_args.args.get(4).cloned()) };
                }
            }

            // args are truncated to 2 for compatibility (only when 2-5 args and filename is not None)
            // truncation happens inside "if (filename && filename != Py_None)" block
            let has_filename = exc.filename.to_owned().filter(|f| !vm.is_none(f)).is_some();
            if (3..=5).contains(&len) && has_filename {
                new_args.args.truncate(2);
            }
            PyBaseException::slot_init(zelf, new_args, vm)
        }

        fn init(_zelf: PyRef<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
            unreachable!("slot_init is defined")
        }
    }

    #[pyexception(with(Constructor, Initializer))]
    impl PyOSError {
        #[pymethod]
        fn __str__(zelf: &Py<PyBaseException>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            let obj = zelf.as_object();

            // Get OSError fields directly
            let errno_field = obj.get_attr("errno", vm).ok().filter(|v| !vm.is_none(v));
            let strerror = obj.get_attr("strerror", vm).ok().filter(|v| !vm.is_none(v));
            let filename = obj.get_attr("filename", vm).ok().filter(|v| !vm.is_none(v));
            let filename2 = obj
                .get_attr("filename2", vm)
                .ok()
                .filter(|v| !vm.is_none(v));
            #[cfg(windows)]
            let winerror = obj.get_attr("winerror", vm).ok().filter(|v| !vm.is_none(v));

            // Windows: winerror takes priority over errno
            #[cfg(windows)]
            if let Some(ref win_err) = winerror {
                let code = win_err.str(vm)?;
                if let Some(ref f) = filename {
                    let msg = strerror
                        .as_ref()
                        .map(|s| s.str(vm))
                        .transpose()?
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "None".to_owned());
                    if let Some(ref f2) = filename2 {
                        return Ok(vm.ctx.new_str(format!(
                            "[WinError {}] {}: {} -> {}",
                            code,
                            msg,
                            f.repr(vm)?,
                            f2.repr(vm)?
                        )));
                    }
                    return Ok(vm.ctx.new_str(format!(
                        "[WinError {}] {}: {}",
                        code,
                        msg,
                        f.repr(vm)?
                    )));
                }
                // winerror && strerror (no filename)
                if let Some(ref s) = strerror {
                    return Ok(vm
                        .ctx
                        .new_str(format!("[WinError {}] {}", code, s.str(vm)?)));
                }
            }

            // Non-Windows or fallback: use errno
            if let Some(ref f) = filename {
                let errno_str = errno_field
                    .as_ref()
                    .map(|e| e.str(vm))
                    .transpose()?
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "None".to_owned());
                let msg = strerror
                    .as_ref()
                    .map(|s| s.str(vm))
                    .transpose()?
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "None".to_owned());
                if let Some(ref f2) = filename2 {
                    return Ok(vm.ctx.new_str(format!(
                        "[Errno {}] {}: {} -> {}",
                        errno_str,
                        msg,
                        f.repr(vm)?,
                        f2.repr(vm)?
                    )));
                }
                return Ok(vm.ctx.new_str(format!(
                    "[Errno {}] {}: {}",
                    errno_str,
                    msg,
                    f.repr(vm)?
                )));
            }

            // errno && strerror (no filename)
            if let (Some(e), Some(s)) = (&errno_field, &strerror) {
                return Ok(vm
                    .ctx
                    .new_str(format!("[Errno {}] {}", e.str(vm)?, s.str(vm)?)));
            }

            // fallback to BaseException.__str__
            zelf.__str__(vm)
        }

        #[pymethod]
        fn __reduce__(exc: PyBaseExceptionRef, vm: &VirtualMachine) -> PyTupleRef {
            let args = exc.args();
            let obj = exc.as_object().to_owned();
            let mut result: Vec<PyObjectRef> = vec![obj.class().to_owned().into()];

            if args.len() >= 2 && args.len() <= 5 {
                // SAFETY: len() == 2 is checked so get_arg 1 or 2 won't panic
                let errno = exc.get_arg(0).unwrap();
                let msg = exc.get_arg(1).unwrap();

                if let Ok(filename) = obj.get_attr("filename", vm) {
                    if !vm.is_none(&filename) {
                        let mut args_reduced: Vec<PyObjectRef> = vec![errno, msg, filename];

                        if let Ok(filename2) = obj.get_attr("filename2", vm)
                            && !vm.is_none(&filename2)
                        {
                            args_reduced.push(filename2);
                        }
                        result.push(args_reduced.into_pytuple(vm).into());
                    } else {
                        result.push(vm.new_tuple((errno, msg)).into());
                    }
                } else {
                    result.push(vm.new_tuple((errno, msg)).into());
                }
            } else {
                result.push(args.into());
            }

            if let Some(dict) = obj.dict().filter(|x| !x.is_empty()) {
                result.push(dict.into());
            }
            result.into_pytuple(vm)
        }

        // Getters and setters for OSError fields
        #[pygetset]
        fn errno(&self) -> Option<PyObjectRef> {
            self.errno.to_owned()
        }

        #[pygetset(setter)]
        fn set_errno(&self, value: Option<PyObjectRef>, vm: &VirtualMachine) {
            self.errno.swap_to_temporary_refs(value, vm);
        }

        #[pygetset]
        fn strerror(&self) -> Option<PyObjectRef> {
            self.strerror.to_owned()
        }

        #[pygetset(setter, name = "strerror")]
        fn set_strerror(&self, value: Option<PyObjectRef>, vm: &VirtualMachine) {
            self.strerror.swap_to_temporary_refs(value, vm);
        }

        #[pygetset]
        fn filename(&self) -> Option<PyObjectRef> {
            self.filename.to_owned()
        }

        #[pygetset(setter)]
        fn set_filename(&self, value: Option<PyObjectRef>, vm: &VirtualMachine) {
            self.filename.swap_to_temporary_refs(value, vm);
        }

        #[pygetset]
        fn filename2(&self) -> Option<PyObjectRef> {
            self.filename2.to_owned()
        }

        #[pygetset(setter)]
        fn set_filename2(&self, value: Option<PyObjectRef>, vm: &VirtualMachine) {
            self.filename2.swap_to_temporary_refs(value, vm);
        }

        #[cfg(windows)]
        #[pygetset]
        fn winerror(&self) -> Option<PyObjectRef> {
            self.winerror.to_owned()
        }

        #[cfg(windows)]
        #[pygetset(setter)]
        fn set_winerror(&self, value: Option<PyObjectRef>, vm: &VirtualMachine) {
            self.winerror.swap_to_temporary_refs(value, vm);
        }

        #[pygetset]
        fn characters_written(&self, vm: &VirtualMachine) -> PyResult<isize> {
            let written = self.written.load();
            if written == -1 {
                Err(vm.new_attribute_error("characters_written".to_owned()))
            } else {
                Ok(written)
            }
        }

        #[pygetset(setter)]
        fn set_characters_written(
            &self,
            value: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            match value {
                None => {
                    // Deleting the attribute
                    if self.written.load() == -1 {
                        Err(vm.new_attribute_error("characters_written".to_owned()))
                    } else {
                        self.written.store(-1);
                        Ok(())
                    }
                }
                Some(v) => {
                    let n = v
                        .try_index(vm)?
                        .try_to_primitive::<isize>(vm)
                        .map_err(|_| {
                            vm.new_value_error(
                                "cannot convert characters_written value to isize".to_owned(),
                            )
                        })?;
                    self.written.store(n);
                    Ok(())
                }
            }
        }
    }

    #[pyexception(name, base = PyOSError, ctx = "blocking_io_error", impl)]
    #[repr(transparent)]
    #[derive(Debug)]
    pub struct PyBlockingIOError(PyOSError);

    #[pyexception(name, base = PyOSError, ctx = "child_process_error", impl)]
    #[repr(transparent)]
    #[derive(Debug)]
    pub struct PyChildProcessError(PyOSError);

    #[pyexception(name, base = PyOSError, ctx = "connection_error", impl)]
    #[repr(transparent)]
    #[derive(Debug)]
    pub struct PyConnectionError(PyOSError);

    #[pyexception(name, base = PyConnectionError, ctx = "broken_pipe_error", impl)]
    #[repr(transparent)]
    #[derive(Debug)]
    pub struct PyBrokenPipeError(PyConnectionError);

    #[pyexception(
        name,
        base = PyConnectionError,
        ctx = "connection_aborted_error",
        impl
    )]
    #[repr(transparent)]
    #[derive(Debug)]
    pub struct PyConnectionAbortedError(PyConnectionError);

    #[pyexception(
        name,
        base = PyConnectionError,
        ctx = "connection_refused_error",
        impl
    )]
    #[repr(transparent)]
    #[derive(Debug)]
    pub struct PyConnectionRefusedError(PyConnectionError);

    #[pyexception(name, base = PyConnectionError, ctx = "connection_reset_error", impl)]
    #[repr(transparent)]
    #[derive(Debug)]
    pub struct PyConnectionResetError(PyConnectionError);

    #[pyexception(name, base = PyOSError, ctx = "file_exists_error", impl)]
    #[repr(transparent)]
    #[derive(Debug)]
    pub struct PyFileExistsError(PyOSError);

    #[pyexception(name, base = PyOSError, ctx = "file_not_found_error", impl)]
    #[repr(transparent)]
    #[derive(Debug)]
    pub struct PyFileNotFoundError(PyOSError);

    #[pyexception(name, base = PyOSError, ctx = "interrupted_error", impl)]
    #[repr(transparent)]
    #[derive(Debug)]
    pub struct PyInterruptedError(PyOSError);

    #[pyexception(name, base = PyOSError, ctx = "is_a_directory_error", impl)]
    #[repr(transparent)]
    #[derive(Debug)]
    pub struct PyIsADirectoryError(PyOSError);

    #[pyexception(name, base = PyOSError, ctx = "not_a_directory_error", impl)]
    #[repr(transparent)]
    #[derive(Debug)]
    pub struct PyNotADirectoryError(PyOSError);

    #[pyexception(name, base = PyOSError, ctx = "permission_error", impl)]
    #[repr(transparent)]
    #[derive(Debug)]
    pub struct PyPermissionError(PyOSError);

    #[pyexception(name, base = PyOSError, ctx = "process_lookup_error", impl)]
    #[repr(transparent)]
    #[derive(Debug)]
    pub struct PyProcessLookupError(PyOSError);

    #[pyexception(name, base = PyOSError, ctx = "timeout_error", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyTimeoutError(PyOSError);

    #[pyexception(name, base = PyException, ctx = "reference_error", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyReferenceError(PyException);

    #[pyexception(name, base = PyException, ctx = "runtime_error", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyRuntimeError(PyException);

    #[pyexception(name, base = PyRuntimeError, ctx = "not_implemented_error", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyNotImplementedError(PyRuntimeError);

    #[pyexception(name, base = PyRuntimeError, ctx = "recursion_error", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyRecursionError(PyRuntimeError);

    #[pyexception(name, base = PyRuntimeError, ctx = "python_finalization_error", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyPythonFinalizationError(PyRuntimeError);

    #[pyexception(name, base = PyException, ctx = "syntax_error")]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PySyntaxError(PyException);

    #[pyexception(with(Initializer))]
    impl PySyntaxError {
        #[pymethod]
        fn __str__(zelf: &Py<PyBaseException>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            fn basename(filename: &str) -> &str {
                let splitted = if cfg!(windows) {
                    filename.rsplit(&['/', '\\']).next()
                } else {
                    filename.rsplit('/').next()
                };
                splitted.unwrap_or(filename)
            }

            let maybe_lineno = zelf.as_object().get_attr("lineno", vm).ok().map(|obj| {
                obj.str(vm)
                    .unwrap_or_else(|_| vm.ctx.new_str("<lineno str() failed>"))
            });
            let maybe_filename = zelf.as_object().get_attr("filename", vm).ok().map(|obj| {
                obj.str(vm)
                    .unwrap_or_else(|_| vm.ctx.new_str("<filename str() failed>"))
            });

            let msg = match zelf.as_object().get_attr("msg", vm) {
                Ok(obj) => obj
                    .str(vm)
                    .unwrap_or_else(|_| vm.ctx.new_str("<msg str() failed>")),
                Err(_) => {
                    // Fallback to the base formatting if the msg attribute was deleted or attribute lookup fails for any reason.
                    return Py::<PyBaseException>::__str__(zelf, vm);
                }
            };

            let msg_with_location_info: String = match (maybe_lineno, maybe_filename) {
                (Some(lineno), Some(filename)) => {
                    format!("{} ({}, line {})", msg, basename(filename.as_str()), lineno)
                }
                (Some(lineno), None) => {
                    format!("{msg} (line {lineno})")
                }
                (None, Some(filename)) => {
                    format!("{} ({})", msg, basename(filename.as_str()))
                }
                (None, None) => msg.to_string(),
            };

            Ok(vm.ctx.new_str(msg_with_location_info))
        }
    }

    impl Initializer for PySyntaxError {
        type Args = FuncArgs;

        fn slot_init(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            let len = args.args.len();
            let new_args = args;

            zelf.set_attr("print_file_and_line", vm.ctx.none(), vm)?;

            if len == 2
                && let Ok(location_tuple) = new_args.args[1]
                    .clone()
                    .downcast::<crate::builtins::PyTuple>()
            {
                let location_tup_len = location_tuple.len();
                for (i, &attr) in [
                    "filename",
                    "lineno",
                    "offset",
                    "text",
                    "end_lineno",
                    "end_offset",
                ]
                .iter()
                .enumerate()
                {
                    if location_tup_len > i {
                        zelf.set_attr(attr, location_tuple[i].to_owned(), vm)?;
                    } else {
                        break;
                    }
                }
            }

            PyBaseException::slot_init(zelf, new_args, vm)
        }

        fn init(_zelf: PyRef<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
            unreachable!("slot_init is defined")
        }
    }

    // MiddlingExtendsException: inherits __init__ from SyntaxError via MRO
    #[pyexception(
        name = "_IncompleteInputError",
        base = PySyntaxError,
        ctx = "incomplete_input_error",
        impl
    )]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyIncompleteInputError(PySyntaxError);

    #[pyexception(name, base = PySyntaxError, ctx = "indentation_error", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyIndentationError(PySyntaxError);

    #[pyexception(name, base = PyIndentationError, ctx = "tab_error", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyTabError(PyIndentationError);

    #[pyexception(name, base = PyException, ctx = "system_error", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PySystemError(PyException);

    #[pyexception(name, base = PyException, ctx = "type_error", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyTypeError(PyException);

    #[pyexception(name, base = PyException, ctx = "value_error", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyValueError(PyException);

    #[pyexception(name, base = PyValueError, ctx = "unicode_error", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyUnicodeError(PyValueError);

    #[pyexception(name, base = PyUnicodeError, ctx = "unicode_decode_error")]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyUnicodeDecodeError(PyUnicodeError);

    #[pyexception(with(Initializer))]
    impl PyUnicodeDecodeError {
        #[pymethod]
        fn __str__(zelf: &Py<PyBaseException>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            let Ok(object) = zelf.as_object().get_attr("object", vm) else {
                return Ok(vm.ctx.empty_str.to_owned());
            };
            let object: ArgBytesLike = object.try_into_value(vm)?;
            let encoding: PyStrRef = zelf
                .as_object()
                .get_attr("encoding", vm)?
                .try_into_value(vm)?;
            let start: usize = zelf.as_object().get_attr("start", vm)?.try_into_value(vm)?;
            let end: usize = zelf.as_object().get_attr("end", vm)?.try_into_value(vm)?;
            let reason: PyStrRef = zelf
                .as_object()
                .get_attr("reason", vm)?
                .try_into_value(vm)?;
            Ok(vm.ctx.new_str(if start < object.len() && end <= object.len() && end == start + 1 {
                let b = object.borrow_buf()[start];
                format!(
                    "'{encoding}' codec can't decode byte {b:#02x} in position {start}: {reason}"
                )
            } else {
                format!(
                    "'{encoding}' codec can't decode bytes in position {start}-{}: {reason}",
                    end - 1,
                )
            }))
        }
    }

    impl Initializer for PyUnicodeDecodeError {
        type Args = FuncArgs;

        fn slot_init(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            type Args = (PyStrRef, ArgBytesLike, isize, isize, PyStrRef);
            let (encoding, object, start, end, reason): Args = args.bind(vm)?;
            zelf.set_attr("encoding", encoding, vm)?;
            let object_as_bytes = vm.ctx.new_bytes(object.borrow_buf().to_vec());
            zelf.set_attr("object", object_as_bytes, vm)?;
            zelf.set_attr("start", vm.ctx.new_int(start), vm)?;
            zelf.set_attr("end", vm.ctx.new_int(end), vm)?;
            zelf.set_attr("reason", reason, vm)?;
            Ok(())
        }

        fn init(_zelf: PyRef<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
            unreachable!("slot_init is defined")
        }
    }

    #[pyexception(name, base = PyUnicodeError, ctx = "unicode_encode_error")]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyUnicodeEncodeError(PyUnicodeError);

    #[pyexception(with(Initializer))]
    impl PyUnicodeEncodeError {
        #[pymethod]
        fn __str__(zelf: &Py<PyBaseException>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            let Ok(object) = zelf.as_object().get_attr("object", vm) else {
                return Ok(vm.ctx.empty_str.to_owned());
            };
            let object: PyStrRef = object.try_into_value(vm)?;
            let encoding: PyStrRef = zelf
                .as_object()
                .get_attr("encoding", vm)?
                .try_into_value(vm)?;
            let start: usize = zelf.as_object().get_attr("start", vm)?.try_into_value(vm)?;
            let end: usize = zelf.as_object().get_attr("end", vm)?.try_into_value(vm)?;
            let reason: PyStrRef = zelf
                .as_object()
                .get_attr("reason", vm)?
                .try_into_value(vm)?;
            Ok(vm.ctx.new_str(if start < object.char_len() && end <= object.char_len() && end == start + 1 {
                let ch = object.as_wtf8().code_points().nth(start).unwrap();
                format!(
                    "'{encoding}' codec can't encode character '{}' in position {start}: {reason}",
                    UnicodeEscapeCodepoint(ch)
                )
            } else {
                format!(
                    "'{encoding}' codec can't encode characters in position {start}-{}: {reason}",
                    end - 1,
                )
            }))
        }
    }

    impl Initializer for PyUnicodeEncodeError {
        type Args = FuncArgs;

        fn slot_init(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            type Args = (PyStrRef, PyStrRef, isize, isize, PyStrRef);
            let (encoding, object, start, end, reason): Args = args.bind(vm)?;
            zelf.set_attr("encoding", encoding, vm)?;
            zelf.set_attr("object", object, vm)?;
            zelf.set_attr("start", vm.ctx.new_int(start), vm)?;
            zelf.set_attr("end", vm.ctx.new_int(end), vm)?;
            zelf.set_attr("reason", reason, vm)?;
            Ok(())
        }

        fn init(_zelf: PyRef<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
            unreachable!("slot_init is defined")
        }
    }

    #[pyexception(name, base = PyUnicodeError, ctx = "unicode_translate_error")]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyUnicodeTranslateError(PyUnicodeError);

    #[pyexception(with(Initializer))]
    impl PyUnicodeTranslateError {
        #[pymethod]
        fn __str__(zelf: &Py<PyBaseException>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            let Ok(object) = zelf.as_object().get_attr("object", vm) else {
                return Ok(vm.ctx.empty_str.to_owned());
            };
            let object: PyStrRef = object.try_into_value(vm)?;
            let start: usize = zelf.as_object().get_attr("start", vm)?.try_into_value(vm)?;
            let end: usize = zelf.as_object().get_attr("end", vm)?.try_into_value(vm)?;
            let reason: PyStrRef = zelf
                .as_object()
                .get_attr("reason", vm)?
                .try_into_value(vm)?;
            Ok(vm.ctx.new_str(
                if start < object.char_len() && end <= object.char_len() && end == start + 1 {
                    let ch = object.as_wtf8().code_points().nth(start).unwrap();
                    format!(
                        "can't translate character '{}' in position {start}: {reason}",
                        UnicodeEscapeCodepoint(ch)
                    )
                } else {
                    format!(
                        "can't translate characters in position {start}-{}: {reason}",
                        end - 1,
                    )
                },
            ))
        }
    }

    impl Initializer for PyUnicodeTranslateError {
        type Args = FuncArgs;

        fn slot_init(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            type Args = (PyStrRef, isize, isize, PyStrRef);
            let (object, start, end, reason): Args = args.bind(vm)?;
            zelf.set_attr("object", object, vm)?;
            zelf.set_attr("start", vm.ctx.new_int(start), vm)?;
            zelf.set_attr("end", vm.ctx.new_int(end), vm)?;
            zelf.set_attr("reason", reason, vm)?;
            Ok(())
        }

        fn init(_zelf: PyRef<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
            unreachable!("slot_init is defined")
        }
    }

    /// JIT error.
    #[cfg(feature = "jit")]
    #[pyexception(name, base = PyException, ctx = "jit_error", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyJitError(PyException);

    // Warnings
    #[pyexception(name, base = PyException, ctx = "warning", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyWarning(PyException);

    #[pyexception(name, base = PyWarning, ctx = "deprecation_warning", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyDeprecationWarning(PyWarning);

    #[pyexception(name, base = PyWarning, ctx = "pending_deprecation_warning", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyPendingDeprecationWarning(PyWarning);

    #[pyexception(name, base = PyWarning, ctx = "runtime_warning", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyRuntimeWarning(PyWarning);

    #[pyexception(name, base = PyWarning, ctx = "syntax_warning", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PySyntaxWarning(PyWarning);

    #[pyexception(name, base = PyWarning, ctx = "user_warning", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyUserWarning(PyWarning);

    #[pyexception(name, base = PyWarning, ctx = "future_warning", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyFutureWarning(PyWarning);

    #[pyexception(name, base = PyWarning, ctx = "import_warning", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyImportWarning(PyWarning);

    #[pyexception(name, base = PyWarning, ctx = "unicode_warning", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyUnicodeWarning(PyWarning);

    #[pyexception(name, base = PyWarning, ctx = "bytes_warning", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyBytesWarning(PyWarning);

    #[pyexception(name, base = PyWarning, ctx = "resource_warning", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyResourceWarning(PyWarning);

    #[pyexception(name, base = PyWarning, ctx = "encoding_warning", impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct PyEncodingWarning(PyWarning);
}

/// Check if match_type is valid for except* (must be exception type, not ExceptionGroup).
fn check_except_star_type_valid(match_type: &PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
    let base_exc: PyObjectRef = vm.ctx.exceptions.base_exception_type.to_owned().into();
    let base_eg: PyObjectRef = vm.ctx.exceptions.base_exception_group.to_owned().into();

    // Helper to check a single type
    let check_one = |exc_type: &PyObjectRef| -> PyResult<()> {
        // Must be a subclass of BaseException
        if !exc_type.is_subclass(&base_exc, vm)? {
            return Err(vm.new_type_error(
                "catching classes that do not inherit from BaseException is not allowed".to_owned(),
            ));
        }
        // Must not be a subclass of BaseExceptionGroup
        if exc_type.is_subclass(&base_eg, vm)? {
            return Err(vm.new_type_error(
                "catching ExceptionGroup with except* is not allowed. Use except instead."
                    .to_owned(),
            ));
        }
        Ok(())
    };

    // If it's a tuple, check each element
    if let Ok(tuple) = match_type.clone().downcast::<PyTuple>() {
        for item in tuple.iter() {
            check_one(item)?;
        }
    } else {
        check_one(match_type)?;
    }
    Ok(())
}

/// Match exception against except* handler type.
/// Returns (rest, match) tuple.
pub fn exception_group_match(
    exc_value: &PyObjectRef,
    match_type: &PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<(PyObjectRef, PyObjectRef)> {
    // Implements _PyEval_ExceptionGroupMatch

    // If exc_value is None, return (None, None)
    if vm.is_none(exc_value) {
        return Ok((vm.ctx.none(), vm.ctx.none()));
    }

    // Validate match_type and reject ExceptionGroup/BaseExceptionGroup
    check_except_star_type_valid(match_type, vm)?;

    // Check if exc_value matches match_type
    if exc_value.is_instance(match_type, vm)? {
        // Full match of exc itself
        let is_eg = exc_value.fast_isinstance(vm.ctx.exceptions.base_exception_group);
        let matched = if is_eg {
            exc_value.clone()
        } else {
            // Naked exception - wrap it in ExceptionGroup
            let excs = vm.ctx.new_tuple(vec![exc_value.clone()]);
            let eg_type: PyObjectRef = crate::exception_group::exception_group().to_owned().into();
            let wrapped = eg_type.call((vm.ctx.new_str(""), excs), vm)?;
            // Copy traceback from original exception
            if let Ok(exc) = exc_value.clone().downcast::<types::PyBaseException>()
                && let Some(tb) = exc.__traceback__()
                && let Ok(wrapped_exc) = wrapped.clone().downcast::<types::PyBaseException>()
            {
                let _ = wrapped_exc.set___traceback__(tb.into(), vm);
            }
            wrapped
        };
        return Ok((vm.ctx.none(), matched));
    }

    // Check for partial match if it's an exception group
    if exc_value.fast_isinstance(vm.ctx.exceptions.base_exception_group) {
        let pair = vm.call_method(exc_value, "split", (match_type.clone(),))?;
        if !pair.class().is(vm.ctx.types.tuple_type) {
            return Err(vm.new_type_error(format!(
                "{}.split must return a tuple, not {}",
                exc_value.class().name(),
                pair.class().name()
            )));
        }
        let pair_tuple: PyTupleRef = pair.try_into_value(vm)?;
        if pair_tuple.len() < 2 {
            return Err(vm.new_type_error(format!(
                "{}.split must return a 2-tuple, got tuple of size {}",
                exc_value.class().name(),
                pair_tuple.len()
            )));
        }
        let matched = pair_tuple[0].clone();
        let rest = pair_tuple[1].clone();
        return Ok((rest, matched));
    }

    // No match
    Ok((exc_value.clone(), vm.ctx.none()))
}

/// Prepare exception for reraise in except* block.
/// Implements _PyExc_PrepReraiseStar
pub fn prep_reraise_star(orig: PyObjectRef, excs: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    use crate::builtins::PyList;

    let excs_list = excs
        .downcast::<PyList>()
        .map_err(|_| vm.new_type_error("expected list for prep_reraise_star"))?;

    let excs_vec: Vec<PyObjectRef> = excs_list.borrow_vec().to_vec();

    // If no exceptions to process, return None
    if excs_vec.is_empty() {
        return Ok(vm.ctx.none());
    }

    // Special case: naked exception (not an ExceptionGroup)
    // Only one except* clause could have executed, so there's at most one exception to raise
    if !orig.fast_isinstance(vm.ctx.exceptions.base_exception_group) {
        // Find first non-None exception
        let first = excs_vec.into_iter().find(|e| !vm.is_none(e));
        return Ok(first.unwrap_or_else(|| vm.ctx.none()));
    }

    // Split excs into raised (new) and reraised (from original) by comparing metadata
    let mut raised: Vec<PyObjectRef> = Vec::new();
    let mut reraised: Vec<PyObjectRef> = Vec::new();

    for exc in excs_vec {
        if vm.is_none(&exc) {
            continue;
        }
        // Check if this exception came from the original group
        if is_exception_from_orig(&exc, &orig, vm) {
            reraised.push(exc);
        } else {
            raised.push(exc);
        }
    }

    // If no exceptions to reraise, return None
    if raised.is_empty() && reraised.is_empty() {
        return Ok(vm.ctx.none());
    }

    // Project reraised exceptions onto original structure to preserve nesting
    let reraised_eg = exception_group_projection(&orig, &reraised, vm)?;

    // If no new raised exceptions, just return the reraised projection
    if raised.is_empty() {
        return Ok(reraised_eg);
    }

    // Combine raised with reraised_eg
    if !vm.is_none(&reraised_eg) {
        raised.push(reraised_eg);
    }

    // If only one exception, return it directly
    if raised.len() == 1 {
        return Ok(raised.into_iter().next().unwrap());
    }

    // Create new ExceptionGroup for multiple exceptions
    let excs_tuple = vm.ctx.new_tuple(raised);
    let eg_type: PyObjectRef = crate::exception_group::exception_group().to_owned().into();
    eg_type.call((vm.ctx.new_str(""), excs_tuple), vm)
}

/// Check if an exception came from the original group (for reraise detection).
/// Instead of comparing metadata (which can be modified when caught), we compare
/// leaf exception object IDs. split() preserves leaf exception identity.
fn is_exception_from_orig(exc: &PyObjectRef, orig: &PyObjectRef, vm: &VirtualMachine) -> bool {
    // Collect leaf exception IDs from exc
    let mut exc_leaf_ids = HashSet::new();
    collect_exception_group_leaf_ids(exc, &mut exc_leaf_ids, vm);

    if exc_leaf_ids.is_empty() {
        return false;
    }

    // Collect leaf exception IDs from orig
    let mut orig_leaf_ids = HashSet::new();
    collect_exception_group_leaf_ids(orig, &mut orig_leaf_ids, vm);

    // If ALL of exc's leaves are in orig's leaves, it's a reraise
    exc_leaf_ids.iter().all(|id| orig_leaf_ids.contains(id))
}

/// Collect all leaf exception IDs from an exception (group).
fn collect_exception_group_leaf_ids(
    exc: &PyObjectRef,
    leaf_ids: &mut HashSet<usize>,
    vm: &VirtualMachine,
) {
    if vm.is_none(exc) {
        return;
    }

    // If not an exception group, it's a leaf - add its ID
    if !exc.fast_isinstance(vm.ctx.exceptions.base_exception_group) {
        leaf_ids.insert(exc.get_id());
        return;
    }

    // Recurse into exception group's exceptions
    if let Ok(excs_attr) = exc.get_attr("exceptions", vm)
        && let Ok(tuple) = excs_attr.downcast::<PyTuple>()
    {
        for e in tuple.iter() {
            collect_exception_group_leaf_ids(e, leaf_ids, vm);
        }
    }
}

/// Project orig onto keep list, preserving nested structure.
/// Returns an exception group containing only the exceptions from orig
/// that are also in the keep list.
fn exception_group_projection(
    orig: &PyObjectRef,
    keep: &[PyObjectRef],
    vm: &VirtualMachine,
) -> PyResult {
    if keep.is_empty() {
        return Ok(vm.ctx.none());
    }

    // Collect all leaf IDs from keep list
    let mut leaf_ids = HashSet::new();
    for e in keep {
        collect_exception_group_leaf_ids(e, &mut leaf_ids, vm);
    }

    // Split orig by matching leaf IDs, preserving structure
    split_by_leaf_ids(orig, &leaf_ids, vm)
}

/// Recursively split an exception (group) by leaf IDs.
/// Returns the projection containing only matching leaves with preserved structure.
fn split_by_leaf_ids(
    exc: &PyObjectRef,
    leaf_ids: &HashSet<usize>,
    vm: &VirtualMachine,
) -> PyResult {
    if vm.is_none(exc) {
        return Ok(vm.ctx.none());
    }

    // If not an exception group, check if it's in our set
    if !exc.fast_isinstance(vm.ctx.exceptions.base_exception_group) {
        if leaf_ids.contains(&exc.get_id()) {
            return Ok(exc.clone());
        }
        return Ok(vm.ctx.none());
    }

    // Exception group - recurse and reconstruct
    let excs_attr = exc.get_attr("exceptions", vm)?;
    let tuple: PyTupleRef = excs_attr.try_into_value(vm)?;

    let mut matched = Vec::new();
    for e in tuple.iter() {
        let m = split_by_leaf_ids(e, leaf_ids, vm)?;
        if !vm.is_none(&m) {
            matched.push(m);
        }
    }

    if matched.is_empty() {
        return Ok(vm.ctx.none());
    }

    // Reconstruct using derive() to preserve the structure (not necessarily the subclass type)
    let matched_tuple = vm.ctx.new_tuple(matched);
    vm.call_method(exc, "derive", (matched_tuple,))
}
