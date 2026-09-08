// spell-checker:ignore libmpdec mpdecimal pydecimal signaldict lsnprint decstate
pub(crate) use _decimal::module_def;

/// Native decimal arithmetic, the accelerator `Lib/decimal.py` imports in place
/// of `Lib/_pydecimal.py`.
///
/// The numeric engine lives in the `rustpython-decimal` crate; this module is
/// the Python surface: the `Decimal` and `Context` types, the signal hierarchy,
/// and the context variable that holds the current context.
#[pymodule]
mod _decimal {
    use crate::vm::{
        AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, PyWeakRef, VirtualMachine,
        builtins::{
            PyBaseExceptionRef, PyBytes, PyComplex, PyDict, PyFloat, PyInt, PyList, PyStr,
            PyStrRef, PyTuple, PyType, PyTypeRef,
        },
        common::{
            hash,
            lock::{LazyLock, PyMutex},
            static_cell,
            str::transform_decimal_and_space_to_ascii,
        },
        function::{FuncArgs, OptionalArg, PyComparisonValue, PySetterValue},
        protocol::{PyMappingMethods, PyNumberMethods, PySequenceMethods},
        stdlib::_warnings,
        types::{
            AsMapping, AsNumber, AsSequence, Comparable, Constructor, Hashable, Initializer,
            Iterable, PyComparisonOp, Representable,
        },
    };
    use malachite_bigint::{BigInt, BigUint, Sign};
    use num_traits::{One, ToPrimitive, Zero};
    use rustpython_decimal as dec;

    // ------------------------------------------------------------------ limits

    /// Largest allowed value of `Context.prec`.
    #[pyattr]
    const MAX_PREC: i64 = dec::MAX_PREC;
    /// Largest allowed value of `Context.Emax`.
    #[pyattr]
    const MAX_EMAX: i64 = dec::MAX_EMAX;
    /// Smallest allowed value of `Context.Emin`.
    #[pyattr]
    const MIN_EMIN: i64 = dec::MIN_EMIN;
    /// Smallest allowed value of `Context.Etiny()`.
    #[pyattr]
    const MIN_ETINY: i64 = dec::MIN_ETINY;
    /// Largest `bits` argument `IEEEContext` accepts.
    #[pyattr]
    const IEEE_CONTEXT_MAX_BITS: u32 = dec::IEEE_CONTEXT_MAX_BITS;
    /// Whether the current context is per-thread.
    #[pyattr]
    const HAVE_THREADS: bool = true;
    /// Whether the current context is held in a `contextvars.ContextVar`.
    #[pyattr]
    const HAVE_CONTEXTVAR: bool = true;

    /// Version of the General Decimal Arithmetic Specification implemented.
    #[pyattr(name = "__version__")]
    const VERSION: &str = "1.70";
    /// Version reported for the arithmetic backend. `Lib/decimal.py` re-exports
    /// it, and `_pydecimal` names the library it was modelled on the same way.
    #[pyattr(name = "__libmpdec_version__")]
    const LIBMPDEC_VERSION: &str = "4.0.0";

    // ---------------------------------------------------------------- rounding

    /// The rounding-mode names. They are interned so that
    /// `_decimal.ROUND_UP is _pydecimal.ROUND_UP`, which the test suite checks.
    #[pyattr(name = "ROUND_DOWN")]
    fn round_down(vm: &VirtualMachine) -> PyStrRef {
        vm.ctx.intern_str("ROUND_DOWN").to_owned()
    }

    #[pyattr(name = "ROUND_HALF_UP")]
    fn round_half_up(vm: &VirtualMachine) -> PyStrRef {
        vm.ctx.intern_str("ROUND_HALF_UP").to_owned()
    }

    #[pyattr(name = "ROUND_HALF_EVEN")]
    fn round_half_even(vm: &VirtualMachine) -> PyStrRef {
        vm.ctx.intern_str("ROUND_HALF_EVEN").to_owned()
    }

    #[pyattr(name = "ROUND_CEILING")]
    fn round_ceiling(vm: &VirtualMachine) -> PyStrRef {
        vm.ctx.intern_str("ROUND_CEILING").to_owned()
    }

    #[pyattr(name = "ROUND_FLOOR")]
    fn round_floor(vm: &VirtualMachine) -> PyStrRef {
        vm.ctx.intern_str("ROUND_FLOOR").to_owned()
    }

    #[pyattr(name = "ROUND_UP")]
    fn round_up(vm: &VirtualMachine) -> PyStrRef {
        vm.ctx.intern_str("ROUND_UP").to_owned()
    }

    #[pyattr(name = "ROUND_HALF_DOWN")]
    fn round_half_down(vm: &VirtualMachine) -> PyStrRef {
        vm.ctx.intern_str("ROUND_HALF_DOWN").to_owned()
    }

    #[pyattr(name = "ROUND_05UP")]
    fn round_05up(vm: &VirtualMachine) -> PyStrRef {
        vm.ctx.intern_str("ROUND_05UP").to_owned()
    }

    // -------------------------------------------------------------- exceptions

    /// The nine signals a context can flag or trap, in the order libmpdec's
    /// tables list them. `Context.flags` and `Context.traps` iterate in this
    /// order, and a trapped operation reports the first of these it hit.
    const SIGNALS: [(&str, u32); 9] = [
        ("InvalidOperation", dec::status::IEEE_INVALID_OPERATION),
        ("FloatOperation", dec::status::FLOAT_OPERATION),
        ("DivisionByZero", dec::status::DIVISION_BY_ZERO),
        ("Overflow", dec::status::OVERFLOW),
        ("Underflow", dec::status::UNDERFLOW),
        ("Subnormal", dec::status::SUBNORMAL),
        ("Inexact", dec::status::INEXACT),
        ("Rounded", dec::status::ROUNDED),
        ("Clamped", dec::status::CLAMPED),
    ];

    /// The conditions that surface as `InvalidOperation`, in the order they are
    /// listed in the argument of a trapped exception.
    const CONDITIONS: [(&str, u32); 5] = [
        ("InvalidOperation", dec::status::INVALID_OPERATION),
        ("ConversionSyntax", dec::status::CONVERSION_SYNTAX),
        ("DivisionImpossible", dec::status::DIVISION_IMPOSSIBLE),
        ("DivisionUndefined", dec::status::DIVISION_UNDEFINED),
        ("InvalidContext", dec::status::INVALID_CONTEXT),
    ];

    /// Condition name per status bit, used when a context prints its flags.
    /// Every invalid-operation condition prints as `InvalidOperation`, and a
    /// name is printed only once, which is what libmpdec's `mpd_lsnprint_signals`
    /// does and what `Context.__repr__` must reproduce.
    const SIGNAL_STRINGS: [&str; 15] = [
        "Clamped",
        "InvalidOperation",
        "DivisionByZero",
        "InvalidOperation",
        "InvalidOperation",
        "InvalidOperation",
        "Inexact",
        "InvalidOperation",
        "InvalidOperation",
        "InvalidOperation",
        "FloatOperation",
        "Overflow",
        "Rounded",
        "Subnormal",
        "Underflow",
    ];

    #[pyattr(name = "DecimalException", once)]
    fn decimal_exception(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "decimal",
            "DecimalException",
            Some(vec![vm.ctx.exceptions.arithmetic_error.to_owned()]),
        )
    }

    #[pyattr(name = "Clamped", once)]
    fn clamped(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx
            .new_exception_type("decimal", "Clamped", Some(vec![decimal_exception(vm)]))
    }

    #[pyattr(name = "Rounded", once)]
    fn rounded(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx
            .new_exception_type("decimal", "Rounded", Some(vec![decimal_exception(vm)]))
    }

    #[pyattr(name = "Inexact", once)]
    fn inexact(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx
            .new_exception_type("decimal", "Inexact", Some(vec![decimal_exception(vm)]))
    }

    #[pyattr(name = "Subnormal", once)]
    fn subnormal(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx
            .new_exception_type("decimal", "Subnormal", Some(vec![decimal_exception(vm)]))
    }

    #[pyattr(name = "Underflow", once)]
    fn underflow(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "decimal",
            "Underflow",
            Some(vec![inexact(vm), rounded(vm), subnormal(vm)]),
        )
    }

    #[pyattr(name = "Overflow", once)]
    fn overflow(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx
            .new_exception_type("decimal", "Overflow", Some(vec![inexact(vm), rounded(vm)]))
    }

    #[pyattr(name = "DivisionByZero", once)]
    fn division_by_zero(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "decimal",
            "DivisionByZero",
            Some(vec![
                decimal_exception(vm),
                vm.ctx.exceptions.zero_division_error.to_owned(),
            ]),
        )
    }

    #[pyattr(name = "FloatOperation", once)]
    fn float_operation(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "decimal",
            "FloatOperation",
            Some(vec![
                decimal_exception(vm),
                vm.ctx.exceptions.type_error.to_owned(),
            ]),
        )
    }

    #[pyattr(name = "InvalidOperation", once)]
    fn invalid_operation(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "decimal",
            "InvalidOperation",
            Some(vec![decimal_exception(vm)]),
        )
    }

    #[pyattr(name = "ConversionSyntax", once)]
    fn conversion_syntax(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "decimal",
            "ConversionSyntax",
            Some(vec![invalid_operation(vm)]),
        )
    }

    #[pyattr(name = "DivisionImpossible", once)]
    fn division_impossible(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "decimal",
            "DivisionImpossible",
            Some(vec![invalid_operation(vm)]),
        )
    }

    #[pyattr(name = "DivisionUndefined", once)]
    fn division_undefined(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "decimal",
            "DivisionUndefined",
            Some(vec![
                invalid_operation(vm),
                vm.ctx.exceptions.zero_division_error.to_owned(),
            ]),
        )
    }

    #[pyattr(name = "InvalidContext", once)]
    fn invalid_context(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "decimal",
            "InvalidContext",
            Some(vec![invalid_operation(vm)]),
        )
    }

    /// The exception class for a signal name, used when building the flag and
    /// trap dictionaries and the argument of a trapped exception.
    fn signal_class(name: &str, vm: &VirtualMachine) -> PyTypeRef {
        match name {
            "InvalidOperation" => invalid_operation(vm),
            "FloatOperation" => float_operation(vm),
            "DivisionByZero" => division_by_zero(vm),
            "Overflow" => overflow(vm),
            "Underflow" => underflow(vm),
            "Subnormal" => subnormal(vm),
            "Inexact" => inexact(vm),
            "Rounded" => rounded(vm),
            "Clamped" => clamped(vm),
            "ConversionSyntax" => conversion_syntax(vm),
            "DivisionImpossible" => division_impossible(vm),
            "DivisionUndefined" => division_undefined(vm),
            "InvalidContext" => invalid_context(vm),
            _ => unreachable!("unknown decimal signal name"),
        }
    }

    /// libmpdec's `flags_as_exception`: the class a trapped operation raises,
    /// which is the first signal in `SIGNALS` order that the trap caught.
    fn flags_as_exception(flags: u32, vm: &VirtualMachine) -> PyTypeRef {
        for (name, bit) in SIGNALS {
            if flags & bit != 0 {
                return signal_class(name, vm);
            }
        }
        invalid_operation(vm)
    }

    /// libmpdec's `flags_as_list`: the argument a trapped exception carries,
    /// naming every condition the operation raised.
    fn flags_as_list(flags: u32, vm: &VirtualMachine) -> PyObjectRef {
        let mut items: Vec<PyObjectRef> = Vec::new();
        for (name, bit) in CONDITIONS {
            if flags & bit != 0 {
                items.push(signal_class(name, vm).into());
            }
        }
        for (name, bit) in &SIGNALS[1..] {
            if flags & bit != 0 {
                items.push(signal_class(name, vm).into());
            }
        }
        vm.ctx.new_list(items).into()
    }

    /// libmpdec's `mpd_lsnprint_signals`: the flag names a context prints.
    fn signal_names(flags: u32) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = Vec::new();
        for (bit, name) in SIGNAL_STRINGS.iter().enumerate() {
            if flags & (1 << bit) != 0 && !names.contains(name) {
                names.push(name);
            }
        }
        names
    }

    /// Raises the exception a trapped status word calls for.
    fn status_exception(trapped: u32, vm: &VirtualMachine) -> PyBaseExceptionRef {
        if trapped & dec::status::MALLOC_ERROR != 0 {
            return vm.new_memory_error("not enough memory for decimal operation");
        }
        let class = flags_as_exception(trapped, vm);
        let args = flags_as_list(trapped, vm);
        vm.new_exception(class, vec![args])
    }

    // ----------------------------------------------------------- signal dicts

    /// The live view a context exposes as `Context.flags` or `Context.traps`.
    ///
    /// It is not a `dict`: writing through it changes the context's status or
    /// trap word directly. A default-constructed one has no context behind it,
    /// which every operation reports rather than dereferencing nothing.
    #[pyclass(no_attr, module = "decimal", name = "SignalDict")]
    #[derive(Debug, PyPayload)]
    struct SignalDict {
        binding: Option<Binding>,
    }

    /// A signal dictionary borrows its context, as `_decimal`'s does: the
    /// context owns the two views, so a strong reference the other way would
    /// make a cycle. A view that outlives its context reports itself invalid
    /// rather than reading freed state.
    struct Binding {
        context: PyWeakRef<PyDecContext>,
        traps: bool,
    }

    impl core::fmt::Debug for Binding {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("Binding")
                .field("traps", &self.traps)
                .finish()
        }
    }

    impl Constructor for SignalDict {
        type Args = FuncArgs;

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self { binding: None })
        }
    }

    impl SignalDict {
        fn bound(&self, vm: &VirtualMachine) -> PyResult<(PyRef<PyDecContext>, bool)> {
            let binding = self
                .binding
                .as_ref()
                .and_then(|binding| Some((binding.context.upgrade()?, binding.traps)));
            binding.ok_or_else(|| vm.new_value_error("invalid signal dict"))
        }

        fn word(&self, vm: &VirtualMachine) -> PyResult<u32> {
            let (context, traps) = self.bound(vm)?;
            let state = context.state.lock();
            Ok(if traps { state.traps } else { state.status })
        }

        fn set_word(&self, value: u32, vm: &VirtualMachine) -> PyResult<()> {
            let (context, traps) = self.bound(vm)?;
            let mut state = context.state.lock();
            if traps {
                state.traps = value;
            } else {
                state.status = value;
            }
            Ok(())
        }

        /// The status bit a signal class stands for, or `None` when the key is
        /// not one of the nine signals.
        fn key_flag(key: &PyObject, vm: &VirtualMachine) -> Option<u32> {
            SIGNALS
                .iter()
                .find(|(name, _)| signal_class(name, vm).is(key))
                .map(|(_, bit)| *bit)
        }
    }

    #[pyclass(with(
        Constructor,
        AsMapping,
        AsSequence,
        Iterable,
        Representable,
        Comparable
    ))]
    impl SignalDict {
        fn __len__(&self, vm: &VirtualMachine) -> PyResult<usize> {
            self.word(vm)?;
            Ok(SIGNALS.len())
        }

        #[pymethod]
        fn keys(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            self.word(vm)?;
            let items = SIGNALS
                .iter()
                .map(|(name, _)| signal_class(name, vm).into())
                .collect();
            Ok(vm.ctx.new_list(items).into())
        }

        #[pymethod]
        fn values(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let word = self.word(vm)?;
            let items = SIGNALS
                .iter()
                .map(|(_, bit)| vm.ctx.new_bool(word & bit != 0).into())
                .collect();
            Ok(vm.ctx.new_list(items).into())
        }

        #[pymethod]
        fn items(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let word = self.word(vm)?;
            let items = SIGNALS
                .iter()
                .map(|(name, bit)| {
                    vm.ctx
                        .new_tuple(vec![
                            signal_class(name, vm).into(),
                            vm.ctx.new_bool(word & bit != 0).into(),
                        ])
                        .into()
                })
                .collect();
            Ok(vm.ctx.new_list(items).into())
        }

        fn __getitem__(&self, key: &PyObject, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let word = self.word(vm)?;
            let bit = Self::key_flag(key, vm).ok_or_else(|| invalid_signal_key(vm))?;
            Ok(vm.ctx.new_bool(word & bit != 0).into())
        }

        fn __setitem__(
            &self,
            key: &PyObject,
            value: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let word = self.word(vm)?;
            let Some(value) = value else {
                return Err(vm.new_value_error("signal keys cannot be deleted"));
            };
            let bit = Self::key_flag(key, vm).ok_or_else(|| invalid_signal_key(vm))?;
            let word = if value.try_to_bool(vm)? {
                word | bit
            } else {
                word & !bit
            };
            self.set_word(word, vm)
        }

        fn __contains__(&self, key: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
            self.word(vm)?;
            Ok(Self::key_flag(key, vm).is_some())
        }

        #[pymethod]
        fn get(
            &self,
            key: PyObjectRef,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            let word = self.word(vm)?;
            match Self::key_flag(&key, vm) {
                Some(bit) => Ok(vm.ctx.new_bool(word & bit != 0).into()),
                None => Ok(default.unwrap_or_none(vm)),
            }
        }

        #[pymethod]
        fn copy(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let word = self.word(vm)?;
            Ok(signal_word_as_dict(word, vm))
        }
    }

    impl AsMapping for SignalDict {
        fn as_mapping() -> &'static PyMappingMethods {
            static AS_MAPPING: LazyLock<PyMappingMethods> = LazyLock::new(|| PyMappingMethods {
                length: Some(|mapping, vm| SignalDict::mapping_downcast(mapping).__len__(vm)),
                subscript: Some(|mapping, needle, vm| {
                    SignalDict::mapping_downcast(mapping).__getitem__(needle, vm)
                }),
                ass_subscript: Some(|mapping, needle, value, vm| {
                    SignalDict::mapping_downcast(mapping).__setitem__(needle, value, vm)
                }),
            });
            &AS_MAPPING
        }
    }

    impl AsSequence for SignalDict {
        fn as_sequence() -> &'static PySequenceMethods {
            static AS_SEQUENCE: LazyLock<PySequenceMethods> = LazyLock::new(|| PySequenceMethods {
                contains: Some(|seq, target, vm| {
                    SignalDict::sequence_downcast(seq).__contains__(target, vm)
                }),
                ..PySequenceMethods::NOT_IMPLEMENTED
            });
            &AS_SEQUENCE
        }
    }

    impl Iterable for SignalDict {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let keys = zelf.keys(vm)?;
            keys.get_iter(vm).map(Into::into)
        }
    }

    impl Representable for SignalDict {
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let word = zelf.word(vm)?;
            let body = SIGNALS
                .iter()
                .map(|(name, bit)| {
                    format!(
                        "<class 'decimal.{name}'>:{}",
                        if word & bit != 0 { "True" } else { "False" }
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            Ok(format!("{{{body}}}"))
        }
    }

    impl Comparable for SignalDict {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            op.eq_only(|| {
                let word = zelf.word(vm)?;
                if let Some(other) = other.downcast_ref::<Self>() {
                    return Ok(PyComparisonValue::Implemented(word == other.word(vm)?));
                }
                let mine = signal_word_as_dict(word, vm);
                let equal = vm.bool_eq(&mine, other)?;
                Ok(PyComparisonValue::Implemented(equal))
            })
        }
    }

    /// The `KeyError` a signal dictionary raises for anything that is not one
    /// of the nine signals.
    fn invalid_signal_key(vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_exception_msg(vm.ctx.exceptions.key_error.to_owned(), "invalid key".into())
    }

    /// The plain dictionary a signal word corresponds to.
    fn signal_word_as_dict(word: u32, vm: &VirtualMachine) -> PyObjectRef {
        let dict = vm.ctx.new_dict();
        for (name, bit) in SIGNALS {
            let key = signal_class(name, vm);
            dict.set_item(key.as_object(), vm.ctx.new_bool(word & bit != 0).into(), vm)
                .expect("a fresh dict accepts any hashable key");
        }
        dict.into()
    }

    /// Reads a `flags=`/`traps=` constructor argument, which may be a mapping of
    /// signals to truth values or a list of the signals to enable.
    fn signal_word_from_object(obj: &PyObject, vm: &VirtualMachine) -> PyResult<u32> {
        if let Ok(list) = obj.to_owned().downcast::<PyList>() {
            let mut word = 0u32;
            for item in list.borrow_vec().iter() {
                let bit = SignalDict::key_flag(item, vm).ok_or_else(|| {
                    vm.new_exception_msg(
                        vm.ctx.exceptions.key_error.to_owned(),
                        "invalid signal list".into(),
                    )
                })?;
                word |= bit;
            }
            return Ok(word);
        }
        signal_word_from_mapping(obj, vm)
    }

    /// Reads a signal mapping, which is what assigning to `Context.flags` or
    /// `Context.traps` accepts: a full dictionary, or another live view.
    fn signal_word_from_mapping(obj: &PyObject, vm: &VirtualMachine) -> PyResult<u32> {
        if let Some(signals) = obj.downcast_ref::<SignalDict>() {
            return signals.word(vm);
        }
        if let Ok(dict) = obj.to_owned().downcast::<PyDict>() {
            let mut word = 0u32;
            let mut seen = 0usize;
            for (key, value) in &dict {
                let bit = SignalDict::key_flag(&key, vm).ok_or_else(|| {
                    vm.new_exception_msg(
                        vm.ctx.exceptions.key_error.to_owned(),
                        "invalid signal dict".into(),
                    )
                })?;
                seen += 1;
                if value.try_to_bool(vm)? {
                    word |= bit;
                }
            }
            if seen != SIGNALS.len() {
                return Err(vm.new_exception_msg(
                    vm.ctx.exceptions.key_error.to_owned(),
                    "invalid signal dict".into(),
                ));
            }
            return Ok(word);
        }
        Err(vm.new_type_error("invalid signal dict"))
    }

    // ------------------------------------------------------------------ context

    /// The mutable state a `Context` carries: the engine's arithmetic
    /// parameters plus the status and trap words the signal dictionaries view.
    #[derive(Debug, Clone, Copy)]
    struct CtxState {
        ctx: dec::Context,
        status: u32,
        traps: u32,
    }

    impl Default for CtxState {
        fn default() -> Self {
            Self {
                ctx: dec::Context::default(),
                status: 0,
                traps: dec::status::IEEE_INVALID_OPERATION
                    | dec::status::DIVISION_BY_ZERO
                    | dec::status::OVERFLOW,
            }
        }
    }

    /// Context(prec=None, rounding=None, Emin=None, Emax=None, capitals=None, clamp=None, flags=None, traps=None)
    /// --
    ///
    /// The context affects almost all operations and controls rounding,
    /// Over/Underflow, raising of exceptions and much more. A new context can be
    /// constructed by creating a context and modifying the attributes.
    #[pyattr]
    #[pyclass(module = "decimal", name = "Context")]
    #[derive(Debug, Default, PyPayload)]
    struct PyDecContext {
        state: PyMutex<CtxState>,
        /// The `flags` and `traps` views, created on first use and then kept,
        /// so that `context.flags is context.flags`.
        views: PyMutex<Option<(PyObjectRef, PyObjectRef)>>,
    }

    impl PyDecContext {
        fn from_state(state: CtxState) -> Self {
            Self {
                state: PyMutex::new(state),
                views: PyMutex::new(None),
            }
        }

        fn dec_context(&self) -> dec::Context {
            self.state.lock().ctx
        }

        /// The pair of live signal views, created on first use.
        fn views(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<(PyObjectRef, PyObjectRef)> {
            let cached = zelf.views.lock().clone();
            if let Some(views) = cached {
                return Ok(views);
            }
            let view = |traps: bool| -> PyResult<PyObjectRef> {
                Ok(SignalDict {
                    binding: Some(Binding {
                        context: zelf.downgrade(None, vm)?,
                        traps,
                    }),
                }
                .into_ref(&vm.ctx)
                .into())
            };
            let views = (view(false)?, view(true)?);
            let mut slot = zelf.views.lock();
            if let Some(existing) = slot.as_ref() {
                return Ok(existing.clone());
            }
            *slot = Some(views.clone());
            Ok(views)
        }

        /// libmpdec's `dec_addstatus`: record the conditions an operation
        /// raised and, if any of them is trapped, raise the exception naming
        /// them all.
        fn add_status(&self, status: u32, vm: &VirtualMachine) -> PyResult<()> {
            if status == 0 {
                return Ok(());
            }
            let traps = {
                let mut state = self.state.lock();
                state.status |= status;
                state.traps
            };
            let trapped = status & (traps | dec::status::MALLOC_ERROR);
            if trapped == 0 {
                return Ok(());
            }
            Err(status_exception(trapped, vm))
        }

        /// An operand of a `Context` method, which must be a `Decimal` or an
        /// exact integer.
        fn convert_arg(&self, obj: &PyObject, vm: &VirtualMachine) -> PyResult<dec::Decimal> {
            if let Some(d) = obj.downcast_ref::<PyDecimal>() {
                return Ok(d.value.clone());
            }
            if let Some(int) = obj.downcast_ref::<PyInt>() {
                return Ok(decimal_from_bigint(int.as_bigint()));
            }
            Err(vm.new_type_error(format!(
                "conversion from {} to Decimal is not supported",
                obj.class().name()
            )))
        }

        fn unary_op<F>(
            &self,
            a: &PyObject,
            op: F,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>>
        where
            F: FnOnce(&dec::Decimal, &dec::Context, &mut u32) -> dec::Decimal,
        {
            let a = self.convert_arg(a, vm)?;
            let ctx = self.dec_context();
            let mut status = 0;
            let value = op(&a, &ctx, &mut status);
            self.add_status(status, vm)?;
            Ok(PyDecimal::new_ref(value, vm))
        }

        fn binary_op<F>(
            &self,
            a: &PyObject,
            b: &PyObject,
            op: F,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>>
        where
            F: FnOnce(&dec::Decimal, &dec::Decimal, &dec::Context, &mut u32) -> dec::Decimal,
        {
            let a = self.convert_arg(a, vm)?;
            let b = self.convert_arg(b, vm)?;
            let ctx = self.dec_context();
            let mut status = 0;
            let value = op(&a, &b, &ctx, &mut status);
            self.add_status(status, vm)?;
            Ok(PyDecimal::new_ref(value, vm))
        }
    }

    fn value_error_range(name: &str, lo: i64, hi: &str, vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_value_error(format!("valid range for {name} is [{lo}, {hi}]"))
    }

    /// Reads a context field that must be an exact integer in `Py_ssize_t`
    /// range, the way `_decimal` does with `PyLong_AsSsize_t`.
    fn context_ssize(obj: &PyObject, vm: &VirtualMachine) -> PyResult<i64> {
        let int = obj
            .downcast_ref::<PyInt>()
            .ok_or_else(|| vm.new_type_error("integer argument expected"))?;
        int.try_to_primitive::<i64>(vm)
    }

    #[derive(FromArgs)]
    struct ContextArgs {
        #[pyarg(any, optional)]
        prec: Option<PyObjectRef>,
        #[pyarg(any, optional)]
        rounding: Option<PyObjectRef>,
        #[pyarg(any, name = "Emin", optional)]
        emin: Option<PyObjectRef>,
        #[pyarg(any, name = "Emax", optional)]
        emax: Option<PyObjectRef>,
        #[pyarg(any, optional)]
        capitals: Option<PyObjectRef>,
        #[pyarg(any, optional)]
        clamp: Option<PyObjectRef>,
        #[pyarg(any, optional)]
        flags: Option<PyObjectRef>,
        #[pyarg(any, optional)]
        traps: Option<PyObjectRef>,
    }

    impl Constructor for PyDecContext {
        type Args = FuncArgs;

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            // A fresh context starts from the module's template, as `_decimal`
            // does, but never inherits its flags.
            let mut state = *default_context(vm).state.lock();
            state.status = 0;
            Ok(Self::from_state(state))
        }
    }

    impl Initializer for PyDecContext {
        type Args = ContextArgs;

        fn init(zelf: PyRef<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
            let template = *default_context(vm).state.lock();
            let mut state = CtxState {
                status: 0,
                ..template
            };
            if let Some(value) = args.prec {
                state.ctx.prec = check_prec(context_ssize(&value, vm)?, vm)?;
            }
            if let Some(value) = args.rounding {
                state.ctx.round = check_rounding(&value, vm)?;
            }
            if let Some(value) = args.emin {
                state.ctx.emin = check_emin(context_ssize(&value, vm)?, vm)?;
            }
            if let Some(value) = args.emax {
                state.ctx.emax = check_emax(context_ssize(&value, vm)?, vm)?;
            }
            if let Some(value) = args.capitals {
                state.ctx.capitals = check_bit("capitals", context_ssize(&value, vm)?, vm)?;
            }
            if let Some(value) = args.clamp {
                state.ctx.clamp = check_bit("clamp", context_ssize(&value, vm)?, vm)?;
            }
            if let Some(value) = args.flags {
                state.status = signal_word_from_object(&value, vm)?;
            }
            if let Some(value) = args.traps {
                state.traps = signal_word_from_object(&value, vm)?;
            }
            *zelf.state.lock() = state;
            Ok(())
        }
    }

    fn check_prec(value: i64, vm: &VirtualMachine) -> PyResult<i64> {
        if (1..=dec::MAX_PREC).contains(&value) {
            Ok(value)
        } else {
            Err(value_error_range("prec", 1, "MAX_PREC", vm))
        }
    }

    fn check_emin(value: i64, vm: &VirtualMachine) -> PyResult<i64> {
        if (dec::MIN_EMIN..=0).contains(&value) {
            Ok(value)
        } else {
            Err(vm.new_value_error("valid range for Emin is [MIN_EMIN, 0]"))
        }
    }

    fn check_emax(value: i64, vm: &VirtualMachine) -> PyResult<i64> {
        if (0..=dec::MAX_EMAX).contains(&value) {
            Ok(value)
        } else {
            Err(value_error_range("Emax", 0, "MAX_EMAX", vm))
        }
    }

    fn check_bit(name: &str, value: i64, vm: &VirtualMachine) -> PyResult<bool> {
        match value {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(vm.new_value_error(format!("valid values for {name} are 0 or 1"))),
        }
    }

    fn check_rounding(obj: &PyObject, vm: &VirtualMachine) -> PyResult<dec::RoundMode> {
        let name = obj
            .downcast_ref::<PyStr>()
            .and_then(|s| s.to_str())
            .and_then(dec::RoundMode::from_name);
        name.ok_or_else(|| {
            vm.new_type_error(
                "valid values for rounding are:\n  \
                 [ROUND_CEILING, ROUND_FLOOR, ROUND_UP, ROUND_DOWN,\n   \
                 ROUND_HALF_UP, ROUND_HALF_DOWN, ROUND_HALF_EVEN,\n   \
                 ROUND_05UP]",
            )
        })
    }

    #[pyclass(
        flags(BASETYPE, HAS_WEAKREF),
        with(Constructor, Initializer, Representable)
    )]
    impl PyDecContext {
        #[pymethod]
        fn abs(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<PyDecimal>> {
            self.unary_op(
                &x,
                |a, ctx, status| dec::ops::arith::abs(a, true, ctx, status),
                vm,
            )
        }
        #[pymethod]
        fn exp(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<PyDecimal>> {
            self.unary_op(&x, dec::transcendental::exp, vm)
        }
        #[pymethod]
        fn ln(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<PyDecimal>> {
            self.unary_op(&x, dec::transcendental::ln, vm)
        }
        #[pymethod]
        fn log10(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<PyDecimal>> {
            self.unary_op(&x, dec::transcendental::log10, vm)
        }
        #[pymethod]
        fn logb(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<PyDecimal>> {
            self.unary_op(&x, dec::ops::misc::logb, vm)
        }
        #[pymethod]
        fn logical_invert(
            &self,
            x: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            self.unary_op(&x, dec::ops::misc::logical_invert, vm)
        }
        #[pymethod]
        fn minus(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<PyDecimal>> {
            self.unary_op(&x, dec::ops::arith::neg, vm)
        }
        #[pymethod]
        fn next_minus(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<PyDecimal>> {
            self.unary_op(&x, dec::ops::misc::next_minus, vm)
        }
        #[pymethod]
        fn next_plus(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<PyDecimal>> {
            self.unary_op(&x, dec::ops::misc::next_plus, vm)
        }
        #[pymethod]
        fn normalize(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<PyDecimal>> {
            self.unary_op(&x, dec::ops::arith::normalize, vm)
        }
        #[pymethod]
        fn plus(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<PyDecimal>> {
            self.unary_op(&x, dec::ops::arith::pos, vm)
        }
        #[pymethod]
        fn sqrt(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<PyDecimal>> {
            self.unary_op(&x, dec::transcendental::sqrt, vm)
        }
        #[pymethod]
        fn add(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            self.binary_op(&x, &y, dec::ops::arith::add, vm)
        }
        #[pymethod]
        fn compare(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            self.binary_op(&x, &y, dec::ops::compare::compare, vm)
        }
        #[pymethod]
        fn compare_signal(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            self.binary_op(&x, &y, dec::ops::compare::compare_signal, vm)
        }
        #[pymethod]
        fn divide(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            self.binary_op(&x, &y, dec::ops::arith::div, vm)
        }
        #[pymethod]
        fn divide_int(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            self.binary_op(&x, &y, dec::ops::arith::floordiv, vm)
        }
        #[pymethod]
        fn logical_and(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            self.binary_op(&x, &y, dec::ops::misc::logical_and, vm)
        }
        #[pymethod]
        fn logical_or(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            self.binary_op(&x, &y, dec::ops::misc::logical_or, vm)
        }
        #[pymethod]
        fn logical_xor(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            self.binary_op(&x, &y, dec::ops::misc::logical_xor, vm)
        }
        #[pymethod]
        fn max(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            self.binary_op(&x, &y, dec::ops::compare::max, vm)
        }
        #[pymethod]
        fn max_mag(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            self.binary_op(&x, &y, dec::ops::compare::max_mag, vm)
        }
        #[pymethod]
        fn min(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            self.binary_op(&x, &y, dec::ops::compare::min, vm)
        }
        #[pymethod]
        fn min_mag(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            self.binary_op(&x, &y, dec::ops::compare::min_mag, vm)
        }
        #[pymethod]
        fn multiply(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            self.binary_op(&x, &y, dec::ops::arith::mul, vm)
        }
        #[pymethod]
        fn next_toward(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            self.binary_op(&x, &y, dec::ops::misc::next_toward, vm)
        }
        #[pymethod]
        fn remainder(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            self.binary_op(&x, &y, dec::ops::arith::rem, vm)
        }
        #[pymethod]
        fn remainder_near(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            self.binary_op(&x, &y, dec::ops::arith::remainder_near, vm)
        }
        #[pymethod]
        fn rotate(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            self.binary_op(&x, &y, dec::ops::misc::rotate, vm)
        }
        #[pymethod]
        fn scaleb(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            self.binary_op(&x, &y, dec::ops::misc::scaleb, vm)
        }
        #[pymethod]
        fn shift(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            self.binary_op(&x, &y, dec::ops::misc::shift, vm)
        }
        #[pymethod]
        fn subtract(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            self.binary_op(&x, &y, dec::ops::arith::sub, vm)
        }
        #[pymethod]
        fn is_canonical(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
            decimal_arg(&x, vm)?;
            Ok(true)
        }
        #[pymethod]
        fn is_finite(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
            let value = self.convert_arg(&x, vm)?;
            let _ = &value;
            Ok(value.is_finite())
        }
        #[pymethod]
        fn is_infinite(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
            let value = self.convert_arg(&x, vm)?;
            let _ = &value;
            Ok(value.is_infinite())
        }
        #[pymethod]
        fn is_nan(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
            let value = self.convert_arg(&x, vm)?;
            let _ = &value;
            Ok(value.is_nan())
        }
        #[pymethod]
        fn is_qnan(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
            let value = self.convert_arg(&x, vm)?;
            let _ = &value;
            Ok(value.is_qnan())
        }
        #[pymethod]
        fn is_snan(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
            let value = self.convert_arg(&x, vm)?;
            let _ = &value;
            Ok(value.is_snan())
        }
        #[pymethod]
        fn is_signed(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
            let value = self.convert_arg(&x, vm)?;
            let _ = &value;
            Ok(value.sign() != 0)
        }
        #[pymethod]
        fn is_zero(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
            let value = self.convert_arg(&x, vm)?;
            let _ = &value;
            Ok(value.is_zero())
        }
        #[pymethod]
        fn is_normal(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
            let value = self.convert_arg(&x, vm)?;
            Ok(dec::ops::misc::is_normal(&value, &self.dec_context()))
        }
        #[pymethod]
        fn is_subnormal(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
            let value = self.convert_arg(&x, vm)?;
            Ok(dec::ops::misc::is_subnormal(&value, &self.dec_context()))
        }

        #[pymethod]
        fn canonical(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<PyDecimal>> {
            let value = decimal_arg(&x, vm)?;
            Ok(PyDecimal::new_ref(value, vm))
        }

        #[pymethod]
        fn copy_decimal(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<PyDecimal>> {
            let value = self.convert_arg(&x, vm)?;
            Ok(PyDecimal::new_ref(value, vm))
        }

        #[pymethod]
        fn copy_abs(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<PyDecimal>> {
            let value = self.convert_arg(&x, vm)?;
            Ok(PyDecimal::new_ref(dec::ops::compare::copy_abs(&value), vm))
        }

        #[pymethod]
        fn copy_negate(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<PyDecimal>> {
            let value = self.convert_arg(&x, vm)?;
            Ok(PyDecimal::new_ref(
                dec::ops::compare::copy_negate(&value),
                vm,
            ))
        }

        #[pymethod]
        fn copy_sign(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            let a = self.convert_arg(&x, vm)?;
            let b = self.convert_arg(&y, vm)?;
            Ok(PyDecimal::new_ref(dec::ops::compare::copy_sign(&a, &b), vm))
        }

        #[pymethod]
        fn compare_total(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            let a = self.convert_arg(&x, vm)?;
            let b = self.convert_arg(&y, vm)?;
            let ordering = dec::ops::compare::compare_total(&a, &b);
            Ok(PyDecimal::new_ref(ordering_as_decimal(ordering), vm))
        }

        #[pymethod]
        fn compare_total_mag(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            let a = self.convert_arg(&x, vm)?;
            let b = self.convert_arg(&y, vm)?;
            let ordering = dec::ops::compare::compare_total_mag(&a, &b);
            Ok(PyDecimal::new_ref(ordering_as_decimal(ordering), vm))
        }

        #[pymethod]
        fn same_quantum(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<bool> {
            let a = self.convert_arg(&x, vm)?;
            let b = self.convert_arg(&y, vm)?;
            Ok(dec::ops::compare::same_quantum(&a, &b))
        }

        #[pymethod]
        fn quantize(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            let round = self.dec_context().round;
            self.binary_op(
                &x,
                &y,
                |a, b, ctx, status| dec::ops::arith::quantize(a, b, round, ctx, status),
                vm,
            )
        }

        #[pymethod]
        fn to_integral(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<PyDecimal>> {
            self.to_integral_value(x, vm)
        }

        #[pymethod]
        fn to_integral_value(
            &self,
            x: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            let round = self.dec_context().round;
            self.unary_op(
                &x,
                |a, ctx, status| dec::ops::arith::to_integral_value(a, round, ctx, status),
                vm,
            )
        }

        #[pymethod]
        fn to_integral_exact(
            &self,
            x: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            let round = self.dec_context().round;
            self.unary_op(
                &x,
                |a, ctx, status| dec::ops::arith::to_integral_exact(a, round, ctx, status),
                vm,
            )
        }

        #[pymethod]
        fn divmod(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            let a = self.convert_arg(&x, vm)?;
            let b = self.convert_arg(&y, vm)?;
            let ctx = self.dec_context();
            let mut status = 0;
            let (quotient, remainder) = dec::ops::arith::divmod(&a, &b, &ctx, &mut status);
            self.add_status(status, vm)?;
            Ok(vm
                .ctx
                .new_tuple(vec![
                    PyDecimal::new_ref(quotient, vm).into(),
                    PyDecimal::new_ref(remainder, vm).into(),
                ])
                .into())
        }

        /// power($self, /, a, b, modulo=None)
        /// --
        ///
        /// Compute a**b, or a**b modulo the third argument when it is given.
        #[pymethod]
        fn power(
            &self,
            ContextPowerArgs { a, b, modulo }: ContextPowerArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            let base = self.convert_arg(&a, vm)?;
            let exponent = self.convert_arg(&b, vm)?;
            let modulo = match modulo {
                OptionalArg::Present(obj) if !vm.is_none(&obj) => Some(self.convert_arg(&obj, vm)?),
                _ => None,
            };
            let ctx = self.dec_context();
            let mut status = 0;
            let value = match &modulo {
                Some(modulo) => {
                    dec::transcendental::power_modulo(&base, &exponent, modulo, &ctx, &mut status)
                }
                None => dec::transcendental::power(&base, &exponent, &ctx, &mut status),
            };
            self.add_status(status, vm)?;
            Ok(PyDecimal::new_ref(value, vm))
        }

        #[pymethod]
        fn fma(
            &self,
            x: PyObjectRef,
            y: PyObjectRef,
            z: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            let a = self.convert_arg(&x, vm)?;
            let b = self.convert_arg(&y, vm)?;
            let c = self.convert_arg(&z, vm)?;
            let ctx = self.dec_context();
            let mut status = 0;
            let value = dec::ops::arith::fma(&a, &b, &c, &ctx, &mut status);
            self.add_status(status, vm)?;
            Ok(PyDecimal::new_ref(value, vm))
        }

        #[pymethod]
        fn number_class(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
            let value = self.convert_arg(&x, vm)?;
            Ok(dec::ops::misc::number_class(&value, &self.dec_context()).to_owned())
        }

        #[pymethod]
        fn to_eng_string(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
            let value = self.convert_arg(&x, vm)?;
            Ok(value.to_eng_string(self.dec_context().capitals))
        }

        #[pymethod]
        fn to_sci_string(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
            let value = self.convert_arg(&x, vm)?;
            Ok(value.to_sci_string(self.dec_context().capitals))
        }

        #[pymethod]
        fn radix(&self, vm: &VirtualMachine) -> PyRef<PyDecimal> {
            PyDecimal::new_ref(dec::Decimal::from_i64(10), vm)
        }

        /// `Context.create_decimal`: convert and then round into this context,
        /// unlike the exact conversion `Decimal()` performs.
        #[pymethod]
        fn create_decimal(
            zelf: &Py<Self>,
            num: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            let value = match num {
                OptionalArg::Present(num) => decimal_from_object(&num, zelf, false, vm)?,
                OptionalArg::Missing => dec::Decimal::zero(0, 0),
            };
            let ctx = zelf.dec_context();
            if value.is_nan() && value.digits() > ctx.prec - i64::from(ctx.clamp) {
                // Unlike `_fix`, which quietly truncates, a conversion refuses
                // a diagnostic that the context cannot hold.
                zelf.add_status(dec::status::CONVERSION_SYNTAX, vm)?;
                return Ok(PyDecimal::new_ref(quiet_nan(), vm));
            }
            let mut status = 0;
            let value = dec::ops::fix(&value, &ctx, &mut status);
            zelf.add_status(status, vm)?;
            Ok(PyDecimal::new_ref(value, vm))
        }

        #[pymethod]
        fn create_decimal_from_float(
            &self,
            f: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyDecimal>> {
            let value = if let Some(int) = f.downcast_ref::<PyInt>() {
                decimal_from_bigint(int.as_bigint())
            } else if let Some(float) = f.downcast_ref::<PyFloat>() {
                // An explicit float conversion is not "mixing"; only an
                // implicit one signals.
                decimal_from_f64(float.to_f64())
            } else {
                return Err(vm.new_type_error("argument must be int or float"));
            };
            let ctx = self.dec_context();
            let mut status = 0;
            let value = dec::ops::fix(&value, &ctx, &mut status);
            self.add_status(status, vm)?;
            Ok(PyDecimal::new_ref(value, vm))
        }

        /// `Context._apply`, which the IBM test suite drives directly.
        #[pymethod(name = "_apply")]
        fn apply(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<PyDecimal>> {
            self.unary_op(&x, dec::ops::fix, vm)
        }
        #[pygetset]
        fn prec(&self) -> i64 {
            self.state.lock().ctx.prec
        }

        #[pygetset(setter)]
        fn set_prec(&self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
            let value = setter_value("prec", value, vm)?;
            self.state.lock().ctx.prec = check_prec(context_ssize(&value, vm)?, vm)?;
            Ok(())
        }

        #[pygetset]
        fn rounding(&self, vm: &VirtualMachine) -> PyStrRef {
            vm.ctx
                .intern_str(self.state.lock().ctx.round.name())
                .to_owned()
        }

        #[pygetset(setter)]
        fn set_rounding(&self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
            let value = setter_value("rounding", value, vm)?;
            self.state.lock().ctx.round = check_rounding(&value, vm)?;
            Ok(())
        }

        #[pygetset(name = "Emin")]
        fn emin(&self) -> i64 {
            self.state.lock().ctx.emin
        }

        #[pygetset(setter, name = "Emin")]
        fn set_emin(&self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
            let value = setter_value("Emin", value, vm)?;
            self.state.lock().ctx.emin = check_emin(context_ssize(&value, vm)?, vm)?;
            Ok(())
        }

        #[pygetset(name = "Emax")]
        fn emax(&self) -> i64 {
            self.state.lock().ctx.emax
        }

        #[pygetset(setter, name = "Emax")]
        fn set_emax(&self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
            let value = setter_value("Emax", value, vm)?;
            self.state.lock().ctx.emax = check_emax(context_ssize(&value, vm)?, vm)?;
            Ok(())
        }

        #[pygetset]
        fn capitals(&self) -> i64 {
            i64::from(self.state.lock().ctx.capitals)
        }

        #[pygetset(setter)]
        fn set_capitals(&self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
            let value = setter_value("capitals", value, vm)?;
            self.state.lock().ctx.capitals = check_bit("capitals", context_ssize(&value, vm)?, vm)?;
            Ok(())
        }

        #[pygetset]
        fn clamp(&self) -> i64 {
            i64::from(self.state.lock().ctx.clamp)
        }

        #[pygetset(setter)]
        fn set_clamp(&self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
            let value = setter_value("clamp", value, vm)?;
            self.state.lock().ctx.clamp = check_bit("clamp", context_ssize(&value, vm)?, vm)?;
            Ok(())
        }

        #[pygetset]
        fn flags(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            Ok(Self::views(zelf, vm)?.0)
        }

        #[pygetset(setter)]
        fn set_flags(&self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
            let value = setter_value("flags", value, vm)?;
            self.state.lock().status = signal_word_from_mapping(&value, vm)?;
            Ok(())
        }

        #[pygetset]
        fn traps(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            Ok(Self::views(zelf, vm)?.1)
        }

        #[pygetset(setter)]
        fn set_traps(&self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
            let value = setter_value("traps", value, vm)?;
            self.state.lock().traps = signal_word_from_mapping(&value, vm)?;
            Ok(())
        }

        #[pymethod(name = "Etiny")]
        fn etiny(&self) -> i64 {
            self.state.lock().ctx.etiny()
        }

        #[pymethod(name = "Etop")]
        fn etop(&self) -> i64 {
            self.state.lock().ctx.etop()
        }

        #[pymethod]
        fn clear_flags(&self) {
            self.state.lock().status = 0;
        }

        #[pymethod]
        fn clear_traps(&self) {
            self.state.lock().traps = 0;
        }

        #[pymethod]
        #[pymethod(name = "__copy__")]
        fn copy(&self, vm: &VirtualMachine) -> PyRef<Self> {
            Self::from_state(*self.state.lock()).into_ref(&vm.ctx)
        }

        #[pymethod]
        fn __reduce__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyObjectRef {
            let state = *zelf.state.lock();
            let flags = signal_word_as_list(state.status, vm);
            let traps = signal_word_as_list(state.traps, vm);
            let args = vm.ctx.new_tuple(vec![
                vm.ctx.new_int(state.ctx.prec).into(),
                vm.ctx.intern_str(state.ctx.round.name()).to_owned().into(),
                vm.ctx.new_int(state.ctx.emin).into(),
                vm.ctx.new_int(state.ctx.emax).into(),
                vm.ctx.new_int(i64::from(state.ctx.capitals)).into(),
                vm.ctx.new_int(i64::from(state.ctx.clamp)).into(),
                flags,
                traps,
            ]);
            vm.ctx
                .new_tuple(vec![zelf.class().to_owned().into(), args.into()])
                .into()
        }
    }

    impl Representable for PyDecContext {
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            let state = *zelf.state.lock();
            Ok(format!(
                "Context(prec={}, rounding={}, Emin={}, Emax={}, capitals={}, clamp={}, flags=[{}], traps=[{}])",
                state.ctx.prec,
                state.ctx.round.name(),
                state.ctx.emin,
                state.ctx.emax,
                i64::from(state.ctx.capitals),
                i64::from(state.ctx.clamp),
                signal_names(state.status).join(", "),
                signal_names(state.traps).join(", "),
            ))
        }
    }

    /// The signal classes a word turns on, as a list, which is the form
    /// `Context.__reduce__` hands back to the constructor.
    fn signal_word_as_list(word: u32, vm: &VirtualMachine) -> PyObjectRef {
        let items = SIGNALS
            .iter()
            .filter(|(_, bit)| word & bit != 0)
            .map(|(name, _)| signal_class(name, vm).into())
            .collect();
        vm.ctx.new_list(items).into()
    }

    /// Rejects `del context.field`, which `_decimal` does not allow.
    fn setter_value(
        name: &str,
        value: PySetterValue,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        match value {
            PySetterValue::Assign(value) => Ok(value),
            PySetterValue::Delete => {
                Err(vm.new_attribute_error(format!("cannot delete attribute {name}")))
            }
        }
    }

    // ------------------------------------------------------------------ decimal

    /// Decimal(value="0", context=None)
    /// --
    ///
    /// Construct a new Decimal object. `value` can be an integer, string, tuple,
    /// float, or another Decimal object. If no value is given, return
    /// Decimal('0'). The context does not affect the conversion and is only
    /// passed to determine if the InvalidOperation trap is active.
    #[pyattr]
    #[pyclass(module = "decimal", name = "Decimal")]
    #[derive(Debug, PyPayload)]
    struct PyDecimal {
        value: dec::Decimal,
    }

    impl PyDecimal {
        fn new_ref(value: dec::Decimal, vm: &VirtualMachine) -> PyRef<Self> {
            Self { value }.into_ref(&vm.ctx)
        }
    }

    /// An operand that must already be a `Decimal`, which is what the few
    /// `Context` methods that inspect a value's representation demand.
    fn decimal_arg(obj: &PyObject, vm: &VirtualMachine) -> PyResult<dec::Decimal> {
        obj.downcast_ref::<PyDecimal>()
            .map(|d| d.value.clone())
            .ok_or_else(|| {
                vm.new_type_error(format!(
                    "argument must be a Decimal, not {}",
                    obj.class().name()
                ))
            })
    }

    /// A Python integer as an exact decimal.
    fn decimal_from_bigint(value: &BigInt) -> dec::Decimal {
        let sign = u8::from(value.sign() == Sign::Minus);
        dec::Decimal::new_finite(sign, value.magnitude().clone(), 0)
    }

    /// CPython's `numeric_as_ascii`: fold non-ASCII decimal digits to ASCII so
    /// the engine only ever sees the specification's numeric-string grammar.
    ///
    /// `strict` is what `Context.create_decimal` wants: unlike the `Decimal`
    /// constructor it refuses surrounding whitespace and underscores rather
    /// than ignoring them.
    fn numeric_as_ascii(s: &str, strict: bool) -> Option<Vec<u8>> {
        let ascii = transform_decimal_and_space_to_ascii(s);
        if strict {
            let trimmed = ascii.trim_matches(|c: char| c.is_ascii_whitespace());
            if trimmed.len() != ascii.len() || ascii.contains('_') {
                return None;
            }
            return Some(trimmed.as_bytes().to_vec());
        }
        Some(
            ascii
                .trim_matches(|c: char| c.is_ascii_whitespace())
                .bytes()
                .filter(|b| *b != b'_')
                .collect(),
        )
    }

    // --------------------------------------------------------- current context

    /// The `ContextVar` holding the current context, created on first use.
    fn context_var(vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        static_cell! {
            static CONTEXT_VAR: PyObjectRef;
        }
        CONTEXT_VAR
            .get_or_try_init(|| {
                let module = vm.import("_contextvars", 0)?;
                let class = module.get_attr("ContextVar", vm)?;
                class.call(("decimal_context",), vm)
            })
            .cloned()
    }

    /// The template new contexts and the thread's first context are copied
    /// from. It is the module's `DefaultContext`.
    fn default_context(vm: &VirtualMachine) -> PyRef<PyDecContext> {
        static_cell! {
            static DEFAULT_CONTEXT: PyRef<PyDecContext>;
        }
        DEFAULT_CONTEXT
            .get_or_init(|| PyDecContext::default().into_ref(&vm.ctx))
            .clone()
    }

    #[pyattr(name = "DefaultContext")]
    fn default_context_attr(vm: &VirtualMachine) -> PyRef<PyDecContext> {
        default_context(vm)
    }

    #[pyattr(name = "BasicContext", once)]
    fn basic_context(vm: &VirtualMachine) -> PyRef<PyDecContext> {
        let state = CtxState {
            ctx: dec::Context {
                prec: 9,
                round: dec::RoundMode::HalfUp,
                ..dec::Context::default()
            },
            status: 0,
            traps: dec::status::IEEE_INVALID_OPERATION
                | dec::status::DIVISION_BY_ZERO
                | dec::status::OVERFLOW
                | dec::status::UNDERFLOW
                | dec::status::CLAMPED,
        };
        PyDecContext::from_state(state).into_ref(&vm.ctx)
    }

    #[pyattr(name = "ExtendedContext", once)]
    fn extended_context(vm: &VirtualMachine) -> PyRef<PyDecContext> {
        let state = CtxState {
            ctx: dec::Context {
                prec: 9,
                round: dec::RoundMode::HalfEven,
                ..dec::Context::default()
            },
            status: 0,
            traps: 0,
        };
        PyDecContext::from_state(state).into_ref(&vm.ctx)
    }

    /// The context every operation uses when the caller does not name one.
    fn current_context(vm: &VirtualMachine) -> PyResult<PyRef<PyDecContext>> {
        let var = context_var(vm)?;
        let current = vm.call_method(&var, "get", (vm.ctx.none(),))?;
        if let Ok(context) = current.downcast::<PyDecContext>() {
            return Ok(context);
        }
        let mut state = *default_context(vm).state.lock();
        state.status = 0;
        let fresh = PyDecContext::from_state(state).into_ref(&vm.ctx);
        vm.call_method(&var, "set", (fresh.clone(),))?;
        Ok(fresh)
    }

    #[pyfunction]
    fn getcontext(vm: &VirtualMachine) -> PyResult<PyRef<PyDecContext>> {
        current_context(vm)
    }

    #[pyfunction]
    fn setcontext(context: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let mut context = context
            .downcast::<PyDecContext>()
            .map_err(|_| vm.new_type_error("setcontext() argument must be a context"))?;
        // Installing one of the module's templates installs a copy, so that
        // arithmetic never writes flags back into the template.
        if context.is(&default_context(vm))
            || context.is(&basic_context(vm))
            || context.is(&extended_context(vm))
        {
            context = context.copy(vm);
        }
        let var = context_var(vm)?;
        vm.call_method(&var, "set", (context,))?;
        Ok(())
    }

    /// The `context=None` argument every `Decimal` method accepts.
    #[derive(FromArgs)]
    struct CtxArgs {
        #[pyarg(any, optional)]
        context: OptionalArg<PyObjectRef>,
    }

    /// An `(other, context=None)` argument pair.
    #[derive(FromArgs)]
    struct OtherArgs {
        #[pyarg(any)]
        other: PyObjectRef,
        #[pyarg(any, optional)]
        context: OptionalArg<PyObjectRef>,
    }

    /// `Decimal.quantize`'s arguments.
    #[derive(FromArgs)]
    struct QuantizeArgs {
        #[pyarg(any)]
        exp: PyObjectRef,
        #[pyarg(any, optional)]
        rounding: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        context: OptionalArg<PyObjectRef>,
    }

    /// The arguments of the `to_integral` family.
    #[derive(FromArgs)]
    struct IntegralArgs {
        #[pyarg(any, optional)]
        rounding: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        context: OptionalArg<PyObjectRef>,
    }

    /// `Decimal.fma`'s arguments.
    #[derive(FromArgs)]
    struct FmaArgs {
        #[pyarg(any)]
        other: PyObjectRef,
        #[pyarg(any)]
        third: PyObjectRef,
        #[pyarg(any, optional)]
        context: OptionalArg<PyObjectRef>,
    }

    /// `Context.power`'s arguments, the one `Context` method that takes
    /// keywords.
    #[derive(FromArgs)]
    struct ContextPowerArgs {
        #[pyarg(any)]
        a: PyObjectRef,
        #[pyarg(any)]
        b: PyObjectRef,
        #[pyarg(any, optional)]
        modulo: OptionalArg<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct DecimalArgs {
        #[pyarg(any, optional)]
        value: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        context: OptionalArg<PyObjectRef>,
    }

    impl Constructor for PyDecimal {
        type Args = DecimalArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let context = context_arg(args.context, vm)?;
            let value = match args.value {
                OptionalArg::Present(value) => value,
                OptionalArg::Missing => {
                    return Ok(Self {
                        value: dec::Decimal::zero(0, 0),
                    });
                }
            };
            Ok(Self {
                value: decimal_from_object(&value, &context, true, vm)?,
            })
        }
    }

    /// The context an optional `context` argument names, or the current one.
    fn context_arg(
        context: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<PyDecContext>> {
        match context {
            OptionalArg::Present(obj) if !vm.is_none(&obj) => obj
                .downcast::<PyDecContext>()
                .map_err(|_| vm.new_type_error("optional argument must be a context")),
            _ => current_context(vm),
        }
    }

    /// `Decimal.__new__`: an exact conversion, signalling in `context` when the
    /// value cannot be represented. An untrapped signal yields a quiet NaN,
    /// which is what `_decimal` hands back.
    fn decimal_from_object(
        value: &PyObject,
        context: &Py<PyDecContext>,
        exact: bool,
        vm: &VirtualMachine,
    ) -> PyResult<dec::Decimal> {
        if let Some(d) = value.downcast_ref::<PyDecimal>() {
            return Ok(d.value.clone());
        }
        if let Some(s) = value.downcast_ref::<PyStr>() {
            let parsed = s
                .to_str()
                .and_then(|text| numeric_as_ascii(text, !exact))
                .map_or(Err(dec::ParseError::Syntax), |text| {
                    if exact {
                        dec::Decimal::parse_ascii(&text)
                    } else {
                        dec::Decimal::parse_ascii_raw(&text)
                    }
                });
            return match parsed {
                Ok(value) => Ok(value),
                Err(error) => {
                    let status = match error {
                        dec::ParseError::Syntax => dec::status::CONVERSION_SYNTAX,
                        dec::ParseError::Range => dec::status::INVALID_OPERATION,
                    };
                    context.add_status(status, vm)?;
                    Ok(quiet_nan())
                }
            };
        }
        if let Some(int) = value.downcast_ref::<PyInt>() {
            return Ok(decimal_from_bigint(int.as_bigint()));
        }
        if let Some(f) = value.downcast_ref::<PyFloat>() {
            context.add_status(dec::status::FLOAT_OPERATION, vm)?;
            return Ok(decimal_from_f64(f.to_f64()));
        }
        if let Some(items) = tuple_items(value) {
            return decimal_from_tuple(&items, context, exact, vm);
        }
        Err(vm.new_type_error(format!(
            "conversion from {} to Decimal is not supported",
            value.class().name()
        )))
    }

    fn quiet_nan() -> dec::Decimal {
        dec::Decimal::nan(0, BigUint::zero(), false)
    }

    /// The elements of a `list` or `tuple` argument, which `Decimal` accepts as
    /// a `(sign, digits, exponent)` triple.
    fn tuple_items(value: &PyObject) -> Option<Vec<PyObjectRef>> {
        if let Some(tuple) = value.downcast_ref::<PyTuple>() {
            return Some(tuple.as_slice().to_vec());
        }
        value
            .downcast_ref::<PyList>()
            .map(|list| list.borrow_vec().to_vec())
    }

    /// `Decimal((sign, digits, exponent))`.
    fn decimal_from_tuple(
        items: &[PyObjectRef],
        context: &Py<PyDecContext>,
        exact: bool,
        vm: &VirtualMachine,
    ) -> PyResult<dec::Decimal> {
        if items.len() != 3 {
            return Err(vm.new_value_error(
                "Invalid tuple size in creation of Decimal from list or tuple. \
                 The list or tuple should have exactly three elements.",
            ));
        }
        let sign = items[0]
            .downcast_ref::<PyInt>()
            .and_then(|i| i.try_to_primitive::<u8>(vm).ok())
            .filter(|s| *s <= 1)
            .ok_or_else(|| {
                vm.new_value_error(
                    "Invalid sign. The first value in the tuple should be an integer; \
                     either 0 for a positive number or 1 for a negative number.",
                )
            })?;

        // The exponent selects the kind: 'F', 'n' and 'N' name the specials.
        let special = items[2].downcast_ref::<PyStr>().and_then(|s| s.to_str());
        if special == Some("F") {
            return Ok(dec::Decimal::infinity(sign));
        }

        let digits = collect_tuple_digits(&items[1], vm)?;
        match special {
            Some(kind @ ("n" | "N")) => {
                let payload = if digits.is_empty() {
                    BigUint::zero()
                } else {
                    BigUint::parse_bytes(&digits, 10).expect("ASCII decimal digits")
                };
                Ok(dec::Decimal::nan(sign, payload, kind == "N"))
            }
            Some(_) => Err(vm.new_value_error(
                "The third value in the tuple must be an integer, or one of the strings \
                 'F', 'n', 'N'.",
            )),
            None => {
                let exp = items[2]
                    .downcast_ref::<PyInt>()
                    .ok_or_else(|| {
                        vm.new_value_error(
                            "The third value in the tuple must be an integer, or one of the \
                             strings 'F', 'n', 'N'.",
                        )
                    })?
                    .try_to_primitive::<i64>(vm)?;
                let coeff = if digits.is_empty() {
                    BigUint::zero()
                } else {
                    BigUint::parse_bytes(&digits, 10).expect("ASCII decimal digits")
                };
                let value = dec::Decimal::new_finite(sign, coeff, exp);
                if !exact || value.is_exactly_representable() {
                    Ok(value)
                } else {
                    context.add_status(dec::status::INVALID_OPERATION, vm)?;
                    Ok(quiet_nan())
                }
            }
        }
    }

    /// The digit tuple of a `Decimal` triple, with leading zeros dropped the
    /// way `_pydecimal` drops them.
    fn collect_tuple_digits(obj: &PyObject, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let mut digits = Vec::new();
        let iter = obj.to_owned().get_iter(vm)?;
        for item in iter.iter::<PyObjectRef>(vm)? {
            let item = item?;
            let digit = item
                .downcast_ref::<PyInt>()
                .and_then(|i| i.try_to_primitive::<u8>(vm).ok())
                .filter(|d| *d <= 9)
                .ok_or_else(|| {
                    vm.new_value_error(
                        "The second value in the tuple must be composed of integers in the \
                         range 0 through 9.",
                    )
                })?;
            if !digits.is_empty() || digit != 0 {
                digits.push(b'0' + digit);
            }
        }
        Ok(digits)
    }

    /// `Decimal.from_float`: the exact value of a binary float.
    fn decimal_from_f64(value: f64) -> dec::Decimal {
        let sign = u8::from(value.is_sign_negative());
        if value.is_nan() {
            return dec::Decimal::nan(sign, BigUint::zero(), false);
        }
        if value.is_infinite() {
            return dec::Decimal::infinity(sign);
        }
        if value == 0.0 {
            return dec::Decimal::zero(sign, 0);
        }
        // Decompose |value| as mantissa * 2**exp exactly, then trade the
        // remaining powers of two for powers of five so the exponent is decimal.
        let bits = value.abs().to_bits();
        let raw_exponent = ((bits >> 52) & 0x7ff) as i32;
        let (mut mantissa, mut exponent) = if raw_exponent == 0 {
            (bits & 0x000f_ffff_ffff_ffff, -1074_i32)
        } else {
            (
                (bits & 0x000f_ffff_ffff_ffff) | 0x0010_0000_0000_0000,
                raw_exponent - 1075,
            )
        };
        while mantissa != 0 && mantissa % 2 == 0 && exponent < 0 {
            mantissa /= 2;
            exponent += 1;
        }
        let coeff = BigUint::from(mantissa);
        if exponent >= 0 {
            let coeff = coeff * BigUint::from(2u32).pow(exponent as u32);
            dec::Decimal::new_finite(sign, coeff, 0)
        } else {
            let k = exponent.unsigned_abs();
            let coeff = coeff * BigUint::from(5u32).pow(k);
            dec::Decimal::new_finite(sign, coeff, -i64::from(k))
        }
    }

    /// `numbers.Rational`, imported on first use so that a comparison against a
    /// `Fraction` can be made exactly, as `_pydecimal._convert_for_comparison`
    /// does.
    fn numbers_rational(vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        static_cell! {
            static RATIONAL: PyObjectRef;
        }
        RATIONAL
            .get_or_try_init(|| vm.import("numbers", 0)?.get_attr("Rational", vm))
            .cloned()
    }

    /// The `DecimalTuple` named tuple `as_tuple` returns.
    fn decimal_tuple_type(vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        static_cell! {
            static DECIMAL_TUPLE: PyObjectRef;
        }
        DECIMAL_TUPLE
            .get_or_try_init(|| {
                let collections = vm.import("collections", 0)?;
                let namedtuple = collections.get_attr("namedtuple", vm)?;
                let result = namedtuple.call(("DecimalTuple", "sign digits exponent"), vm)?;
                result.set_attr("__module__", vm.ctx.new_str("decimal"), vm)?;
                Ok(result)
            })
            .cloned()
    }

    /// The operand of a `Decimal` operator: another `Decimal` or an exact
    /// integer. Anything else makes the operator return `NotImplemented`.
    fn operand(obj: &PyObject) -> Option<dec::Decimal> {
        if let Some(d) = obj.downcast_ref::<PyDecimal>() {
            return Some(d.value.clone());
        }
        obj.downcast_ref::<PyInt>()
            .map(|int| decimal_from_bigint(int.as_bigint()))
    }

    /// Runs a binary operator, deferring to the other operand's reflected
    /// method when this type cannot handle it.
    fn number_binop<F>(a: &PyObject, b: &PyObject, op: F, vm: &VirtualMachine) -> PyResult
    where
        F: FnOnce(&dec::Decimal, &dec::Decimal, &dec::Context, &mut u32) -> dec::Decimal,
    {
        let (Some(a), Some(b)) = (operand(a), operand(b)) else {
            return Ok(vm.ctx.not_implemented());
        };
        let context = current_context(vm)?;
        let ctx = context.dec_context();
        let mut status = 0;
        let value = op(&a, &b, &ctx, &mut status);
        context.add_status(status, vm)?;
        Ok(PyDecimal::new_ref(value, vm).into())
    }

    impl PyDecimal {
        /// Runs a unary operation under the context an optional argument names.
        fn unary<F>(
            &self,
            context: OptionalArg<PyObjectRef>,
            op: F,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>>
        where
            F: FnOnce(&dec::Decimal, &dec::Context, &mut u32) -> dec::Decimal,
        {
            let context = context_arg(context, vm)?;
            let ctx = context.dec_context();
            let mut status = 0;
            let value = op(&self.value, &ctx, &mut status);
            context.add_status(status, vm)?;
            Ok(Self::new_ref(value, vm))
        }

        /// Runs a binary method, whose operand must convert.
        fn binary<F>(
            &self,
            other: &PyObject,
            context: OptionalArg<PyObjectRef>,
            op: F,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>>
        where
            F: FnOnce(&dec::Decimal, &dec::Decimal, &dec::Context, &mut u32) -> dec::Decimal,
        {
            let other = operand(other).ok_or_else(|| {
                vm.new_type_error(format!(
                    "conversion from {} to Decimal is not supported",
                    other.class().name()
                ))
            })?;
            let context = context_arg(context, vm)?;
            let ctx = context.dec_context();
            let mut status = 0;
            let value = op(&self.value, &other, &ctx, &mut status);
            context.add_status(status, vm)?;
            Ok(Self::new_ref(value, vm))
        }

        /// The value the current context would print, honouring `capitals`.
        fn to_str(&self, vm: &VirtualMachine) -> PyResult<String> {
            let capitals = current_context(vm)?.dec_context().capitals;
            Ok(self.value.to_sci_string(capitals))
        }
    }

    #[pyclass(
        flags(BASETYPE),
        with(Constructor, Comparable, Hashable, AsNumber, Representable)
    )]
    impl PyDecimal {
        #[pymethod]
        fn __str__(&self, vm: &VirtualMachine) -> PyResult<String> {
            self.to_str(vm)
        }

        /// to_eng_string($self, /, context=None)
        /// --
        ///
        /// Convert to a string, using engineering notation if an exponent is needed.
        #[pymethod]
        fn to_eng_string(
            &self,
            CtxArgs { context }: CtxArgs,
            vm: &VirtualMachine,
        ) -> PyResult<String> {
            let context = context_arg(context, vm)?;
            Ok(self.value.to_eng_string(context.dec_context().capitals))
        }

        #[pymethod]
        fn as_tuple(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let digits: Vec<PyObjectRef> = self
                .value
                .coeff_digits()
                .into_iter()
                .map(|d| vm.ctx.new_int(d - b'0').into())
                .collect();
            let exponent: PyObjectRef = match self.value.special() {
                dec::Special::Finite => vm.ctx.new_int(self.value.exponent()).into(),
                dec::Special::Inf => vm.ctx.new_str("F").into(),
                dec::Special::Nan => vm.ctx.new_str("n").into(),
                dec::Special::Snan => vm.ctx.new_str("N").into(),
            };
            decimal_tuple_type(vm)?.call(
                (
                    vm.ctx.new_int(self.value.sign()),
                    vm.ctx.new_tuple(digits),
                    exponent,
                ),
                vm,
            )
        }

        #[pymethod]
        fn as_integer_ratio(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            if self.value.is_nan() {
                return Err(vm.new_value_error("cannot convert NaN to integer ratio"));
            }
            if self.value.is_infinite() {
                return Err(vm.new_overflow_error("cannot convert Infinity to integer ratio"));
            }
            let (num, den) = decimal_as_ratio(&self.value);
            Ok(vm
                .ctx
                .new_tuple(vec![
                    vm.ctx.new_bigint(&num).into(),
                    vm.ctx.new_bigint(&den).into(),
                ])
                .into())
        }

        #[pymethod]
        fn adjusted(&self) -> i64 {
            self.value.adjusted()
        }

        #[pymethod]
        fn canonical(&self, vm: &VirtualMachine) -> PyRef<Self> {
            Self::new_ref(self.value.clone(), vm)
        }

        #[pymethod]
        fn radix(&self, vm: &VirtualMachine) -> PyRef<Self> {
            Self::new_ref(dec::Decimal::from_i64(10), vm)
        }

        #[pymethod]
        fn copy_abs(&self, vm: &VirtualMachine) -> PyRef<Self> {
            Self::new_ref(dec::ops::compare::copy_abs(&self.value), vm)
        }

        #[pymethod]
        fn copy_negate(&self, vm: &VirtualMachine) -> PyRef<Self> {
            Self::new_ref(dec::ops::compare::copy_negate(&self.value), vm)
        }

        /// copy_sign($self, /, other, context=None)
        /// --
        ///
        /// Return a copy of this number with the sign of the other one.
        #[pymethod]
        fn copy_sign(
            &self,
            OtherArgs { other, context }: OtherArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let context = context_arg(context, vm)?;
            let other = operand(&other).ok_or_else(|| {
                vm.new_type_error(format!(
                    "conversion from {} to Decimal is not supported",
                    other.class().name()
                ))
            })?;
            let _ = &context;
            Ok(Self::new_ref(
                dec::ops::compare::copy_sign(&self.value, &other),
                vm,
            ))
        }

        /// same_quantum($self, /, other, context=None)
        /// --
        ///
        /// Return True if the two numbers have the same exponent.
        #[pymethod]
        fn same_quantum(
            &self,
            OtherArgs { other, context }: OtherArgs,
            vm: &VirtualMachine,
        ) -> PyResult<bool> {
            let _ = context_arg(context, vm)?;
            let other = operand(&other).ok_or_else(|| {
                vm.new_type_error(format!(
                    "conversion from {} to Decimal is not supported",
                    other.class().name()
                ))
            })?;
            Ok(dec::ops::compare::same_quantum(&self.value, &other))
        }

        /// compare_total($self, /, other, context=None)
        /// --
        ///
        /// Compare two numbers using their abstract representation.
        #[pymethod]
        fn compare_total(
            &self,
            OtherArgs { other, context }: OtherArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let _ = context_arg(context, vm)?;
            let other = operand(&other).ok_or_else(|| {
                vm.new_type_error(format!(
                    "conversion from {} to Decimal is not supported",
                    other.class().name()
                ))
            })?;
            let ordering = dec::ops::compare::compare_total(&self.value, &other);
            Ok(Self::new_ref(ordering_as_decimal(ordering), vm))
        }

        /// compare_total_mag($self, /, other, context=None)
        /// --
        ///
        /// Compare two numbers using their abstract representation, ignoring signs.
        #[pymethod]
        fn compare_total_mag(
            &self,
            OtherArgs { other, context }: OtherArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let _ = context_arg(context, vm)?;
            let other = operand(&other).ok_or_else(|| {
                vm.new_type_error(format!(
                    "conversion from {} to Decimal is not supported",
                    other.class().name()
                ))
            })?;
            let ordering = dec::ops::compare::compare_total_mag(&self.value, &other);
            Ok(Self::new_ref(ordering_as_decimal(ordering), vm))
        }

        /// number_class($self, /, context=None)
        /// --
        ///
        /// Return a string describing the class of this number.
        #[pymethod]
        fn number_class(
            &self,
            CtxArgs { context }: CtxArgs,
            vm: &VirtualMachine,
        ) -> PyResult<String> {
            let context = context_arg(context, vm)?;
            Ok(dec::ops::misc::number_class(&self.value, &context.dec_context()).to_owned())
        }

        /// quantize($self, /, exp, rounding=None, context=None)
        /// --
        ///
        /// Return a value equal to this number after rounding to the exponent of `exp`.
        #[pymethod]
        fn quantize(
            &self,
            QuantizeArgs {
                exp,
                rounding,
                context,
            }: QuantizeArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let context = context_arg(context, vm)?;
            let ctx = context.dec_context();
            let round = rounding_arg(rounding, ctx.round, vm)?;
            let exp = operand(&exp).ok_or_else(|| {
                vm.new_type_error(format!(
                    "conversion from {} to Decimal is not supported",
                    exp.class().name()
                ))
            })?;
            let mut status = 0;
            let value = dec::ops::arith::quantize(&self.value, &exp, round, &ctx, &mut status);
            context.add_status(status, vm)?;
            Ok(Self::new_ref(value, vm))
        }

        /// to_integral_exact($self, /, rounding=None, context=None)
        /// --
        ///
        /// Round to the nearest integer, signalling Inexact or Rounded as appropriate.
        #[pymethod]
        fn to_integral_exact(
            &self,
            IntegralArgs { rounding, context }: IntegralArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let context = context_arg(context, vm)?;
            let ctx = context.dec_context();
            let round = rounding_arg(rounding, ctx.round, vm)?;
            let mut status = 0;
            let value = dec::ops::arith::to_integral_exact(&self.value, round, &ctx, &mut status);
            context.add_status(status, vm)?;
            Ok(Self::new_ref(value, vm))
        }

        /// to_integral($self, /, rounding=None, context=None)
        /// --
        ///
        /// Identical to to_integral_value().
        #[pymethod]
        fn to_integral(
            &self,
            IntegralArgs { rounding, context }: IntegralArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            self.to_integral_value(IntegralArgs { rounding, context }, vm)
        }

        /// to_integral_value($self, /, rounding=None, context=None)
        /// --
        ///
        /// Round to the nearest integer without signalling Inexact or Rounded.
        #[pymethod]
        fn to_integral_value(
            &self,
            IntegralArgs { rounding, context }: IntegralArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let context = context_arg(context, vm)?;
            let ctx = context.dec_context();
            let round = rounding_arg(rounding, ctx.round, vm)?;
            let mut status = 0;
            let value = dec::ops::arith::to_integral_value(&self.value, round, &ctx, &mut status);
            context.add_status(status, vm)?;
            Ok(Self::new_ref(value, vm))
        }

        /// fma($self, /, other, third, context=None)
        /// --
        ///
        /// Return self*other+third with no rounding of the intermediate product.
        #[pymethod]
        fn fma(
            &self,
            FmaArgs {
                other,
                third,
                context,
            }: FmaArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let context = context_arg(context, vm)?;
            let convert = |obj: &PyObject| {
                operand(obj).ok_or_else(|| {
                    vm.new_type_error(format!(
                        "conversion from {} to Decimal is not supported",
                        obj.class().name()
                    ))
                })
            };
            let other = convert(&other)?;
            let third = convert(&third)?;
            let ctx = context.dec_context();
            let mut status = 0;
            let value = dec::ops::arith::fma(&self.value, &other, &third, &ctx, &mut status);
            context.add_status(status, vm)?;
            Ok(Self::new_ref(value, vm))
        }

        #[pyclassmethod]
        fn from_float(
            cls: PyTypeRef,
            f: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            let value = if let Some(int) = f.downcast_ref::<PyInt>() {
                decimal_from_bigint(int.as_bigint())
            } else if let Some(float) = f.downcast_ref::<PyFloat>() {
                decimal_from_f64(float.to_f64())
            } else {
                return Err(vm.new_type_error("argument must be int or float"));
            };
            let exact: PyObjectRef = Self::new_ref(value, vm).into();
            if cls.is(Self::class(&vm.ctx)) {
                return Ok(exact);
            }
            // A subclass builds through its own constructor, so that its
            // `__init__` runs on the converted value.
            cls.as_object().call((exact,), vm)
        }

        #[pyclassmethod]
        fn from_number(
            cls: PyTypeRef,
            number: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            if number.downcast_ref::<Self>().is_some()
                || number.downcast_ref::<PyInt>().is_some()
                || number.downcast_ref::<PyFloat>().is_some()
            {
                let context = current_context(vm)?;
                let value = decimal_from_object(&number, &context, true, vm)?;
                return Self { value }.into_ref_with_type(vm, cls).map(Into::into);
            }
            Err(vm.new_type_error(format!(
                "argument must be int, float or Decimal, not {}",
                number.class().name()
            )))
        }

        #[pymethod]
        fn __reduce__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let text = zelf.to_str(vm)?;
            Ok(vm
                .ctx
                .new_tuple(vec![
                    zelf.class().to_owned().into(),
                    vm.ctx.new_tuple(vec![vm.ctx.new_str(text).into()]).into(),
                ])
                .into())
        }

        #[pymethod]
        fn __copy__(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }

        #[pymethod]
        fn __deepcopy__(zelf: PyRef<Self>, _memo: OptionalArg<PyObjectRef>) -> PyRef<Self> {
            zelf
        }

        #[pymethod]
        fn __sizeof__(&self) -> usize {
            // Report the coefficient the way libmpdec stores it, in words of
            // nineteen digits, so that growth is visible at the same steps.
            let digits = self.value.digits().max(1) as usize;
            let words = digits.div_ceil(19);
            core::mem::size_of::<Self>() + 8 * words
        }

        #[pymethod]
        fn __trunc__(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            self.__int__(vm)
        }

        #[pymethod]
        fn __floor__(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            self.round_to_integer(dec::RoundMode::Floor, vm)
        }

        #[pymethod]
        fn __ceil__(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            self.round_to_integer(dec::RoundMode::Ceiling, vm)
        }

        #[pymethod]
        fn __round__(
            &self,
            ndigits: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            let Some(ndigits) = ndigits.into_option().filter(|n| !vm.is_none(n)) else {
                return self.round_to_integer(dec::RoundMode::HalfEven, vm);
            };
            let ndigits = ndigits
                .downcast_ref::<PyInt>()
                .ok_or_else(|| vm.new_type_error("optional arg must be an integer"))?
                .try_to_primitive::<i64>(vm)?;
            let exp = dec::Decimal::new_finite(0, BigUint::one(), ndigits.saturating_neg());
            let context = current_context(vm)?;
            let ctx = context.dec_context();
            let mut status = 0;
            let value = dec::ops::arith::quantize(
                &self.value,
                &exp,
                dec::RoundMode::HalfEven,
                &ctx,
                &mut status,
            );
            context.add_status(status, vm)?;
            Ok(Self::new_ref(value, vm).into())
        }

        fn __int__(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            if self.value.is_nan() {
                return Err(vm.new_value_error("cannot convert NaN to integer"));
            }
            if self.value.is_infinite() {
                return Err(vm.new_overflow_error("cannot convert Infinity to integer"));
            }
            Ok(vm.ctx.new_bigint(&decimal_truncate(&self.value)).into())
        }

        fn __float__(&self, vm: &VirtualMachine) -> PyResult<f64> {
            if self.value.is_snan() {
                return Err(vm.new_value_error("cannot convert signaling NaN to float"));
            }
            Ok(decimal_to_f64(&self.value))
        }

        #[pymethod]
        fn __complex__(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let real = self.__float__(vm)?;
            Ok(vm
                .ctx
                .new_complex(num_complex::Complex64::new(real, 0.0))
                .into())
        }

        #[pymethod]
        fn conjugate(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }

        #[pygetset]
        fn real(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }

        #[pygetset]
        fn imag(&self, vm: &VirtualMachine) -> PyRef<Self> {
            Self::new_ref(dec::Decimal::zero(0, 0), vm)
        }

        #[pymethod]
        fn __format__(
            &self,
            specifier: PyObjectRef,
            override_locale: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<String> {
            let spec = specifier
                .downcast_ref::<PyStr>()
                .and_then(|s| s.to_str())
                .ok_or_else(|| vm.new_type_error("format spec must be a string"))?;
            // libmpdec still accepts a deprecated 'N' type, which formats like
            // 'n' and then upper-cases the result.
            let deprecated_n = spec.ends_with('N');
            let effective = if deprecated_n {
                format!("{}n", &spec[..spec.len() - 1])
            } else {
                spec.to_owned()
            };
            let locale = format_locale(&effective, override_locale, vm)?;
            let context = current_context(vm)?;
            let ctx = context.dec_context();
            let mut status = 0;
            let result =
                dec::fmt::format(&self.value, &effective, &ctx, locale.as_ref(), &mut status);
            context.add_status(status, vm)?;
            match result {
                Ok(text) if deprecated_n => {
                    _warnings::warn(
                        vm.ctx.exceptions.deprecation_warning,
                        "Format specifier 'N' is deprecated".to_owned(),
                        1,
                        vm,
                    )?;
                    Ok(text.to_uppercase())
                }
                Ok(text) => Ok(text),
                Err(err) => Err(vm.new_value_error(err.0.replace(&effective, spec))),
            }
        }
        /// exp($self, /, context=None)
        /// --
        ///
        /// Return the value of the (natural) exponential function e**x at this number.
        #[pymethod]
        fn exp(&self, CtxArgs { context }: CtxArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            self.unary(context, dec::transcendental::exp, vm)
        }
        /// ln($self, /, context=None)
        /// --
        ///
        /// Return the natural (base e) logarithm of this number.
        #[pymethod]
        fn ln(&self, CtxArgs { context }: CtxArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            self.unary(context, dec::transcendental::ln, vm)
        }
        /// log10($self, /, context=None)
        /// --
        ///
        /// Return the base ten logarithm of this number.
        #[pymethod]
        fn log10(
            &self,
            CtxArgs { context }: CtxArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            self.unary(context, dec::transcendental::log10, vm)
        }
        /// logb($self, /, context=None)
        /// --
        ///
        /// Return the adjusted exponent of this number as a Decimal.
        #[pymethod]
        fn logb(&self, CtxArgs { context }: CtxArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            self.unary(context, dec::ops::misc::logb, vm)
        }
        /// logical_invert($self, /, context=None)
        /// --
        ///
        /// Return the digit-wise inversion of this number.
        #[pymethod]
        fn logical_invert(
            &self,
            CtxArgs { context }: CtxArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            self.unary(context, dec::ops::misc::logical_invert, vm)
        }
        /// next_minus($self, /, context=None)
        /// --
        ///
        /// Return the largest representable number smaller than this one.
        #[pymethod]
        fn next_minus(
            &self,
            CtxArgs { context }: CtxArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            self.unary(context, dec::ops::misc::next_minus, vm)
        }
        /// next_plus($self, /, context=None)
        /// --
        ///
        /// Return the smallest representable number larger than this one.
        #[pymethod]
        fn next_plus(
            &self,
            CtxArgs { context }: CtxArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            self.unary(context, dec::ops::misc::next_plus, vm)
        }
        /// normalize($self, /, context=None)
        /// --
        ///
        /// Return this number with its trailing zeros removed.
        #[pymethod]
        fn normalize(
            &self,
            CtxArgs { context }: CtxArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            self.unary(context, dec::ops::arith::normalize, vm)
        }
        /// sqrt($self, /, context=None)
        /// --
        ///
        /// Return the square root of this number, correctly rounded.
        #[pymethod]
        fn sqrt(&self, CtxArgs { context }: CtxArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            self.unary(context, dec::transcendental::sqrt, vm)
        }
        /// compare($self, /, other, context=None)
        /// --
        ///
        /// Compare this number with the other one, returning -1, 0, 1 or NaN.
        #[pymethod]
        fn compare(
            &self,
            OtherArgs { other, context }: OtherArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            self.binary(&other, context, dec::ops::compare::compare, vm)
        }
        /// compare_signal($self, /, other, context=None)
        /// --
        ///
        /// Compare like compare(), but signal on a quiet NaN operand.
        #[pymethod]
        fn compare_signal(
            &self,
            OtherArgs { other, context }: OtherArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            self.binary(&other, context, dec::ops::compare::compare_signal, vm)
        }
        /// logical_and($self, /, other, context=None)
        /// --
        ///
        /// Return the digit-wise and of this number and the other one.
        #[pymethod]
        fn logical_and(
            &self,
            OtherArgs { other, context }: OtherArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            self.binary(&other, context, dec::ops::misc::logical_and, vm)
        }
        /// logical_or($self, /, other, context=None)
        /// --
        ///
        /// Return the digit-wise or of this number and the other one.
        #[pymethod]
        fn logical_or(
            &self,
            OtherArgs { other, context }: OtherArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            self.binary(&other, context, dec::ops::misc::logical_or, vm)
        }
        /// logical_xor($self, /, other, context=None)
        /// --
        ///
        /// Return the digit-wise exclusive or of this number and the other one.
        #[pymethod]
        fn logical_xor(
            &self,
            OtherArgs { other, context }: OtherArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            self.binary(&other, context, dec::ops::misc::logical_xor, vm)
        }
        /// max($self, /, other, context=None)
        /// --
        ///
        /// Return the larger of this number and the other one.
        #[pymethod]
        fn max(
            &self,
            OtherArgs { other, context }: OtherArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            self.binary(&other, context, dec::ops::compare::max, vm)
        }
        /// max_mag($self, /, other, context=None)
        /// --
        ///
        /// Return the larger of this number and the other one, ignoring signs.
        #[pymethod]
        fn max_mag(
            &self,
            OtherArgs { other, context }: OtherArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            self.binary(&other, context, dec::ops::compare::max_mag, vm)
        }
        /// min($self, /, other, context=None)
        /// --
        ///
        /// Return the smaller of this number and the other one.
        #[pymethod]
        fn min(
            &self,
            OtherArgs { other, context }: OtherArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            self.binary(&other, context, dec::ops::compare::min, vm)
        }
        /// min_mag($self, /, other, context=None)
        /// --
        ///
        /// Return the smaller of this number and the other one, ignoring signs.
        #[pymethod]
        fn min_mag(
            &self,
            OtherArgs { other, context }: OtherArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            self.binary(&other, context, dec::ops::compare::min_mag, vm)
        }
        /// next_toward($self, /, other, context=None)
        /// --
        ///
        /// Return the number closest to this one, in the direction of the other.
        #[pymethod]
        fn next_toward(
            &self,
            OtherArgs { other, context }: OtherArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            self.binary(&other, context, dec::ops::misc::next_toward, vm)
        }
        /// remainder_near($self, /, other, context=None)
        /// --
        ///
        /// Return the remainder of dividing by the other number, nearest to zero.
        #[pymethod]
        fn remainder_near(
            &self,
            OtherArgs { other, context }: OtherArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            self.binary(&other, context, dec::ops::arith::remainder_near, vm)
        }
        /// rotate($self, /, other, context=None)
        /// --
        ///
        /// Return this number rotated by the other number of digits.
        #[pymethod]
        fn rotate(
            &self,
            OtherArgs { other, context }: OtherArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            self.binary(&other, context, dec::ops::misc::rotate, vm)
        }
        /// scaleb($self, /, other, context=None)
        /// --
        ///
        /// Return this number with its exponent adjusted by the other number.
        #[pymethod]
        fn scaleb(
            &self,
            OtherArgs { other, context }: OtherArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            self.binary(&other, context, dec::ops::misc::scaleb, vm)
        }
        /// shift($self, /, other, context=None)
        /// --
        ///
        /// Return this number shifted by the other number of digits.
        #[pymethod]
        fn shift(
            &self,
            OtherArgs { other, context }: OtherArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            self.binary(&other, context, dec::ops::misc::shift, vm)
        }
        /// Return True, since every Decimal is canonical.
        #[pymethod]
        fn is_canonical(&self) -> bool {
            true
        }
        /// Return True if this number is neither infinite nor a NaN.
        #[pymethod]
        fn is_finite(&self) -> bool {
            self.value.is_finite()
        }
        /// Return True if this number is infinite.
        #[pymethod]
        fn is_infinite(&self) -> bool {
            self.value.is_infinite()
        }
        /// Return True if this number is a quiet or signaling NaN.
        #[pymethod]
        fn is_nan(&self) -> bool {
            self.value.is_nan()
        }
        /// Return True if this number is a quiet NaN.
        #[pymethod]
        fn is_qnan(&self) -> bool {
            self.value.is_qnan()
        }
        /// Return True if this number is a signaling NaN.
        #[pymethod]
        fn is_snan(&self) -> bool {
            self.value.is_snan()
        }
        /// Return True if this number carries a minus sign.
        #[pymethod]
        fn is_signed(&self) -> bool {
            self.value.sign() != 0
        }
        /// Return True if this number is zero.
        #[pymethod]
        fn is_zero(&self) -> bool {
            self.value.is_zero()
        }
        /// is_normal($self, /, context=None)
        /// --
        ///
        /// Return True if this number is a normal finite non-zero number.
        #[pymethod]
        fn is_normal(&self, CtxArgs { context }: CtxArgs, vm: &VirtualMachine) -> PyResult<bool> {
            let context = context_arg(context, vm)?;
            Ok(dec::ops::misc::is_normal(
                &self.value,
                &context.dec_context(),
            ))
        }
        /// is_subnormal($self, /, context=None)
        /// --
        ///
        /// Return True if this number is subnormal.
        #[pymethod]
        fn is_subnormal(
            &self,
            CtxArgs { context }: CtxArgs,
            vm: &VirtualMachine,
        ) -> PyResult<bool> {
            let context = context_arg(context, vm)?;
            Ok(dec::ops::misc::is_subnormal(
                &self.value,
                &context.dec_context(),
            ))
        }
    }

    impl PyDecimal {
        /// `int(self._rescale(0, mode))`: round to an integer, refusing the
        /// specials the way `Decimal.__floor__` and friends do.
        fn round_to_integer(
            &self,
            mode: dec::RoundMode,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            if self.value.is_nan() {
                return Err(vm.new_value_error("cannot round a NaN"));
            }
            if self.value.is_infinite() {
                return Err(vm.new_overflow_error("cannot round an infinity"));
            }
            let rounded = dec::ops::arith::rescale(&self.value, 0, mode);
            Ok(vm.ctx.new_bigint(&decimal_truncate(&rounded)).into())
        }
    }

    /// `int(self)`: the value with everything after the point dropped.
    fn decimal_truncate(value: &dec::Decimal) -> BigInt {
        let magnitude = if value.exponent() >= 0 {
            dec_bigops_mul(value.coefficient(), value.exponent().unsigned_abs())
        } else {
            dec_bigops_div(value.coefficient(), value.exponent().unsigned_abs())
        };
        let sign = if value.sign() == 0 {
            Sign::Plus
        } else {
            Sign::Minus
        };
        BigInt::from_biguint(sign, magnitude)
    }

    fn dec_bigops_mul(coeff: &BigUint, shift: u64) -> BigUint {
        dec::bigops::mul_pow10(coeff, shift)
    }

    fn dec_bigops_div(coeff: &BigUint, shift: u64) -> BigUint {
        dec::bigops::div_pow10(coeff, shift)
    }

    /// `Decimal.as_integer_ratio`, in lowest terms.
    fn decimal_as_ratio(value: &dec::Decimal) -> (BigInt, BigInt) {
        if value.is_zero() {
            return (BigInt::from(0), BigInt::from(1));
        }
        let mut n = value.coefficient().clone();
        let den = if value.exponent() >= 0 {
            n = dec::bigops::mul_pow10(&n, value.exponent().unsigned_abs());
            BigUint::one()
        } else {
            let scale = value.exponent().unsigned_abs();
            // Cancel the fives and twos the coefficient already carries, which
            // is what reducing `n / 10**scale` amounts to.
            let mut fives = scale;
            let five = BigUint::from(5u32);
            while fives > 0 && (&n % &five).is_zero() {
                n /= &five;
                fives -= 1;
            }
            let mut twos = scale;
            let shift = n.trailing_zeros().unwrap_or(0).min(twos);
            if shift > 0 {
                n >>= shift;
                twos -= shift;
            }
            five.pow(u32::try_from(fives).expect("scale fits in a u32")) << twos
        };
        let sign = if value.sign() == 0 {
            Sign::Plus
        } else {
            Sign::Minus
        };
        (
            BigInt::from_biguint(sign, n),
            BigInt::from_biguint(Sign::Plus, den),
        )
    }

    /// `float(self)`.
    fn decimal_to_f64(value: &dec::Decimal) -> f64 {
        let magnitude = if value.is_nan() {
            f64::NAN
        } else if value.is_infinite() {
            f64::INFINITY
        } else {
            value
                .to_sci_string(true)
                .trim_start_matches('-')
                .parse()
                .unwrap_or(f64::NAN)
        };
        if value.sign() == 0 {
            magnitude
        } else {
            -magnitude
        }
    }

    /// The `-1`, `0` or `1` the total-ordering comparisons return.
    fn ordering_as_decimal(ordering: core::cmp::Ordering) -> dec::Decimal {
        dec::Decimal::from_i64(match ordering {
            core::cmp::Ordering::Less => -1,
            core::cmp::Ordering::Equal => 0,
            core::cmp::Ordering::Greater => 1,
        })
    }

    /// An optional `rounding=` argument, defaulting to the context's mode.
    fn rounding_arg(
        rounding: OptionalArg<PyObjectRef>,
        default: dec::RoundMode,
        vm: &VirtualMachine,
    ) -> PyResult<dec::RoundMode> {
        match rounding {
            OptionalArg::Present(obj) if !vm.is_none(&obj) => check_rounding(&obj, vm),
            _ => Ok(default),
        }
    }

    /// The locale conventions a format specification needs: the `'n'` type and
    /// an explicit override both ask for them, and nothing else does.
    fn format_locale(
        spec: &str,
        override_locale: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<Option<dec::fmt::LocaleInfo>> {
        if let OptionalArg::Present(obj) = override_locale
            && !vm.is_none(&obj)
        {
            let dict = obj
                .downcast_ref::<PyDict>()
                .ok_or_else(|| vm.new_type_error("optional argument must be a dict"))?;
            let text = |key: &str| -> PyResult<String> {
                let value = dict
                    .get_item_opt(key, vm)?
                    .ok_or_else(|| vm.new_value_error(format!("missing key {key}")))?;
                let text = value
                    .downcast_ref::<PyStr>()
                    .and_then(|s| s.to_str())
                    .map(str::to_owned)
                    .ok_or_else(|| vm.new_type_error("optional argument must be a dict"))?;
                // libmpdec stores these in a fixed buffer of four bytes plus a
                // terminator, so a longer separator is not usable.
                if text.len() > 4 {
                    return Err(vm.new_value_error(format!("{key} too long")));
                }
                Ok(text)
            };
            let grouping = match dict.get_item_opt("grouping", vm)? {
                Some(value) => locale_grouping(&value, vm)?,
                None => Vec::new(),
            };
            return Ok(Some(dec::fmt::LocaleInfo {
                decimal_point: text("decimal_point")?,
                thousands_sep: text("thousands_sep")?,
                grouping,
            }));
        }
        if !spec.ends_with('n') {
            return Ok(None);
        }
        let localeconv = vm.import("_locale", 0)?.get_attr("localeconv", vm)?;
        let conv = localeconv.call((), vm)?;
        let dict = conv
            .downcast_ref::<PyDict>()
            .ok_or_else(|| vm.new_type_error("localeconv() did not return a dict"))?;
        let text = |key: &str| -> PyResult<String> {
            Ok(dict
                .get_item_opt(key, vm)?
                .and_then(|v| {
                    v.downcast_ref::<PyStr>()
                        .and_then(|s| s.to_str())
                        .map(str::to_owned)
                })
                .unwrap_or_default())
        };
        let grouping = match dict.get_item_opt("grouping", vm)? {
            Some(value) => locale_grouping(&value, vm)?,
            None => Vec::new(),
        };
        Ok(Some(dec::fmt::LocaleInfo {
            decimal_point: text("decimal_point")?,
            thousands_sep: text("thousands_sep")?,
            grouping,
        }))
    }

    fn locale_grouping(value: &PyObject, vm: &VirtualMachine) -> PyResult<Vec<i32>> {
        let mut out = Vec::new();
        // `_decimal` takes the grouping as a string whose code points are the
        // group sizes, which is how the test suite and `localeconv` spell it.
        // They are `char` values, so anything above 127 is a negative size,
        // which libmpdec rejects.
        if let Some(text) = value.downcast_ref::<PyStr>() {
            for ch in text.as_wtf8().code_points() {
                let raw = ch.to_u32();
                let size = if raw < 256 {
                    i32::from(raw as u8 as i8)
                } else {
                    i32::try_from(raw).unwrap_or(i32::MAX)
                };
                if size < 0 {
                    return Err(vm.new_value_error("invalid grouping"));
                }
                out.push(size);
            }
            return Ok(out);
        }
        if let Some(bytes) = value.downcast_ref::<PyBytes>() {
            for byte in bytes.as_bytes() {
                out.push(i32::from(*byte));
            }
            return Ok(out);
        }
        let iter = value.to_owned().get_iter(vm)?;
        for item in iter.iter::<PyObjectRef>(vm)? {
            let item = item?;
            let size = item
                .downcast_ref::<PyInt>()
                .ok_or_else(|| vm.new_type_error("invalid grouping"))?
                .try_to_primitive::<i32>(vm)?;
            out.push(size);
        }
        Ok(out)
    }

    impl Representable for PyDecimal {
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            Ok(format!("Decimal('{}')", zelf.to_str(vm)?))
        }
    }

    /// `_pydecimal._convert_for_comparison`: the pair of decimals a comparison
    /// should actually compare, or `None` when the operand is not a number this
    /// type knows how to compare against.
    fn convert_for_comparison(
        zelf: &dec::Decimal,
        other: &PyObject,
        equality: bool,
        vm: &VirtualMachine,
    ) -> PyResult<Option<(dec::Decimal, dec::Decimal)>> {
        if let Some(d) = other.downcast_ref::<PyDecimal>() {
            return Ok(Some((zelf.clone(), d.value.clone())));
        }
        if let Some(int) = other.downcast_ref::<PyInt>() {
            return Ok(Some((zelf.clone(), decimal_from_bigint(int.as_bigint()))));
        }
        if let Some(f) = other.downcast_ref::<PyFloat>() {
            let context = current_context(vm)?;
            if equality {
                // Equality only records the mixing; it never raises.
                context.state.lock().status |= dec::status::FLOAT_OPERATION;
            } else {
                context.add_status(dec::status::FLOAT_OPERATION, vm)?;
            }
            return Ok(Some((zelf.clone(), decimal_from_f64(f.to_f64()))));
        }
        // An equality test against a complex number with no imaginary part
        // compares against its real part.
        if equality && let Some(complex) = other.downcast_ref::<PyComplex>() {
            let value = complex.to_complex64();
            if value.im != 0.0 {
                return Ok(None);
            }
            let context = current_context(vm)?;
            context.state.lock().status |= dec::status::FLOAT_OPERATION;
            return Ok(Some((zelf.clone(), decimal_from_f64(value.re))));
        }
        // A `Fraction` (or any other Rational) compares exactly, by scaling this
        // value by the other's denominator. A special is left as it is: scaling
        // cannot change how it compares.
        let rational = numbers_rational(vm)?;
        if other.is_instance(&rational, vm)? {
            let numerator = other.get_attr("numerator", vm)?;
            let denominator = other.get_attr("denominator", vm)?;
            let (Some(numerator), Some(denominator)) = (
                numerator.downcast_ref::<PyInt>(),
                denominator.downcast_ref::<PyInt>(),
            ) else {
                return Ok(None);
            };
            let left = if zelf.is_special() {
                zelf.clone()
            } else {
                let scaled = zelf.coefficient() * denominator.as_bigint().magnitude();
                dec::Decimal::new_finite(zelf.sign(), scaled, zelf.exponent())
            };
            return Ok(Some((left, decimal_from_bigint(numerator.as_bigint()))));
        }
        Ok(None)
    }

    impl Comparable for PyDecimal {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            let equality = matches!(op, PyComparisonOp::Eq | PyComparisonOp::Ne);
            let Some((a, b)) = convert_for_comparison(&zelf.value, other, equality, vm)? else {
                return Ok(PyComparisonValue::NotImplemented);
            };
            if equality {
                // Only a signaling NaN is invalid here; a quiet NaN simply
                // compares unequal to everything.
                let context = current_context(vm)?;
                let mut status = 0;
                if dec::ops::check_nans(&a, Some(&b), &context.dec_context(), &mut status).is_some()
                {
                    context.add_status(status, vm)?;
                    return Ok(PyComparisonValue::Implemented(op == PyComparisonOp::Ne));
                }
            } else if a.is_nan() || b.is_nan() {
                let context = current_context(vm)?;
                context.add_status(dec::status::INVALID_OPERATION, vm)?;
                return Ok(PyComparisonValue::Implemented(false));
            }
            let ordering = dec::ops::cmp_values(&a, &b);
            Ok(PyComparisonValue::Implemented(op.eval_ord(ordering)))
        }
    }

    impl Hashable for PyDecimal {
        fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<hash::PyHash> {
            let value = &zelf.value;
            if value.is_snan() {
                return Err(vm.new_type_error("cannot hash a signaling NaN value"));
            }
            if value.is_nan() {
                // Every NaN hashes by identity, so that distinct NaNs can live
                // in the same dictionary.
                return Ok(hash::hash_object_id(zelf.get_id()));
            }
            if value.is_infinite() {
                return Ok(if value.sign() == 0 {
                    hash::INF
                } else {
                    -hash::INF
                });
            }
            let modulus = BigUint::from(hash::MODULUS);
            let exp = value.exponent();
            let exp_hash = if exp >= 0 {
                BigUint::from(10u32).modpow(&BigUint::from(exp.unsigned_abs()), &modulus)
            } else {
                hash_ten_inverse().modpow(&BigUint::from(exp.unsigned_abs()), &modulus)
            };
            let magnitude = (value.coefficient() * exp_hash) % modulus;
            let magnitude = magnitude
                .to_i64()
                .expect("a residue modulo 2**61 - 1 fits in i64");
            let hashed = if value.sign() == 0 {
                magnitude
            } else {
                -magnitude
            };
            Ok(if hashed == -1 { -2 } else { hashed })
        }
    }

    /// The modular inverse of ten, which turns a negative exponent into a
    /// multiplication when hashing.
    fn hash_ten_inverse() -> BigUint {
        static INVERSE: LazyLock<BigUint> = LazyLock::new(|| {
            let modulus = BigUint::from(hash::MODULUS);
            BigUint::from(10u32).modpow(&(&modulus - BigUint::from(2u32)), &modulus)
        });
        INVERSE.clone()
    }

    impl AsNumber for PyDecimal {
        fn as_number() -> &'static PyNumberMethods {
            static AS_NUMBER: PyNumberMethods = PyNumberMethods {
                add: Some(|a, b, vm| number_binop(a, b, dec::ops::arith::add, vm)),
                subtract: Some(|a, b, vm| number_binop(a, b, dec::ops::arith::sub, vm)),
                multiply: Some(|a, b, vm| number_binop(a, b, dec::ops::arith::mul, vm)),
                true_divide: Some(|a, b, vm| number_binop(a, b, dec::ops::arith::div, vm)),
                floor_divide: Some(|a, b, vm| number_binop(a, b, dec::ops::arith::floordiv, vm)),
                remainder: Some(|a, b, vm| number_binop(a, b, dec::ops::arith::rem, vm)),
                divmod: Some(decimal_divmod),
                power: Some(decimal_power),
                negative: Some(|number, vm| {
                    let zelf = PyDecimal::number_downcast(number);
                    zelf.unary(OptionalArg::Missing, dec::ops::arith::neg, vm)
                        .map(Into::into)
                }),
                positive: Some(|number, vm| {
                    let zelf = PyDecimal::number_downcast(number);
                    zelf.unary(OptionalArg::Missing, dec::ops::arith::pos, vm)
                        .map(Into::into)
                }),
                absolute: Some(|number, vm| {
                    let zelf = PyDecimal::number_downcast(number);
                    zelf.unary(
                        OptionalArg::Missing,
                        |a, ctx, status| dec::ops::arith::abs(a, true, ctx, status),
                        vm,
                    )
                    .map(Into::into)
                }),
                boolean: Some(|number, _vm| {
                    Ok(PyDecimal::number_downcast(number).value.is_nonzero())
                }),
                int: Some(|number, vm| PyDecimal::number_downcast(number).__int__(vm)),
                float: Some(|number, vm| {
                    let value = PyDecimal::number_downcast(number).__float__(vm)?;
                    Ok(vm.ctx.new_float(value).into())
                }),
                ..PyNumberMethods::NOT_IMPLEMENTED
            };
            &AS_NUMBER
        }
    }

    fn decimal_divmod(a: &PyObject, b: &PyObject, vm: &VirtualMachine) -> PyResult {
        let (Some(a), Some(b)) = (operand(a), operand(b)) else {
            return Ok(vm.ctx.not_implemented());
        };
        let context = current_context(vm)?;
        let ctx = context.dec_context();
        let mut status = 0;
        let (quotient, remainder) = dec::ops::arith::divmod(&a, &b, &ctx, &mut status);
        context.add_status(status, vm)?;
        Ok(vm
            .ctx
            .new_tuple(vec![
                PyDecimal::new_ref(quotient, vm).into(),
                PyDecimal::new_ref(remainder, vm).into(),
            ])
            .into())
    }

    fn decimal_power(a: &PyObject, b: &PyObject, c: &PyObject, vm: &VirtualMachine) -> PyResult {
        let (Some(a), Some(b)) = (operand(a), operand(b)) else {
            return Ok(vm.ctx.not_implemented());
        };
        let context = current_context(vm)?;
        let ctx = context.dec_context();
        let mut status = 0;
        let value = if vm.is_none(c) {
            dec::transcendental::power(&a, &b, &ctx, &mut status)
        } else {
            let Some(modulo) = operand(c) else {
                return Ok(vm.ctx.not_implemented());
            };
            dec::transcendental::power_modulo(&a, &b, &modulo, &ctx, &mut status)
        };
        context.add_status(status, vm)?;
        Ok(PyDecimal::new_ref(value, vm).into())
    }

    // ---------------------------------------------------------- local context

    /// The object `localcontext()` returns: a `with` block that installs a
    /// context for its body and puts the previous one back afterwards.
    #[pyclass(no_attr, module = "decimal", name = "ContextManager")]
    #[derive(Debug, PyPayload)]
    struct DecContextManager {
        local: PyRef<PyDecContext>,
        saved: PyMutex<Option<PyRef<PyDecContext>>>,
    }

    #[pyclass(flags(DISALLOW_INSTANTIATION))]
    impl DecContextManager {
        #[pymethod]
        fn __enter__(&self, vm: &VirtualMachine) -> PyResult<PyRef<PyDecContext>> {
            *self.saved.lock() = Some(current_context(vm)?);
            let var = context_var(vm)?;
            vm.call_method(&var, "set", (self.local.clone(),))?;
            Ok(self.local.clone())
        }

        #[pymethod]
        fn __exit__(&self, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            let Some(saved) = self.saved.lock().take() else {
                return Ok(());
            };
            let var = context_var(vm)?;
            vm.call_method(&var, "set", (saved,))?;
            Ok(())
        }
    }

    #[derive(FromArgs)]
    struct LocalContextArgs {
        #[pyarg(any, optional)]
        ctx: OptionalArg<PyObjectRef>,
        #[pyarg(named, optional)]
        prec: Option<PyObjectRef>,
        #[pyarg(named, optional)]
        rounding: Option<PyObjectRef>,
        #[pyarg(named, name = "Emin", optional)]
        emin: Option<PyObjectRef>,
        #[pyarg(named, name = "Emax", optional)]
        emax: Option<PyObjectRef>,
        #[pyarg(named, optional)]
        capitals: Option<PyObjectRef>,
        #[pyarg(named, optional)]
        clamp: Option<PyObjectRef>,
        #[pyarg(named, optional)]
        flags: Option<PyObjectRef>,
        #[pyarg(named, optional)]
        traps: Option<PyObjectRef>,
    }

    /// localcontext(ctx=None, **kwargs)
    /// --
    ///
    /// Return a context manager that will set the default context to a copy of
    /// ctx on entry to the with-statement and restore the previous default
    /// context when exiting the with-statement.
    #[pyfunction]
    fn localcontext(
        LocalContextArgs {
            ctx,
            prec,
            rounding,
            emin,
            emax,
            capitals,
            clamp,
            flags,
            traps,
        }: LocalContextArgs,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<DecContextManager>> {
        let base = match ctx {
            OptionalArg::Present(obj) if !vm.is_none(&obj) => obj
                .downcast::<PyDecContext>()
                .map_err(|_| vm.new_type_error("optional argument must be a context"))?,
            _ => current_context(vm)?,
        };
        let mut state = *base.state.lock();
        if let Some(value) = prec {
            state.ctx.prec = check_prec(context_ssize(&value, vm)?, vm)?;
        }
        if let Some(value) = rounding {
            state.ctx.round = check_rounding(&value, vm)?;
        }
        if let Some(value) = emin {
            state.ctx.emin = check_emin(context_ssize(&value, vm)?, vm)?;
        }
        if let Some(value) = emax {
            state.ctx.emax = check_emax(context_ssize(&value, vm)?, vm)?;
        }
        if let Some(value) = capitals {
            state.ctx.capitals = check_bit("capitals", context_ssize(&value, vm)?, vm)?;
        }
        if let Some(value) = clamp {
            state.ctx.clamp = check_bit("clamp", context_ssize(&value, vm)?, vm)?;
        }
        if let Some(value) = flags {
            state.status = signal_word_from_object(&value, vm)?;
        }
        if let Some(value) = traps {
            state.traps = signal_word_from_object(&value, vm)?;
        }
        Ok(DecContextManager {
            local: PyDecContext::from_state(state).into_ref(&vm.ctx),
            saved: PyMutex::new(None),
        }
        .into_ref(&vm.ctx))
    }

    /// `IEEEContext(bits)`: the interchange format of the given width.
    #[pyfunction(name = "IEEEContext")]
    fn ieee_context(bits: i64, vm: &VirtualMachine) -> PyResult<PyRef<PyDecContext>> {
        if bits <= 0 || bits > i64::from(dec::IEEE_CONTEXT_MAX_BITS) || bits % 32 != 0 {
            return Err(vm.new_value_error(
                "argument must be a multiple of 32, with a value from 32 to \
                 IEEE_CONTEXT_MAX_BITS.",
            ));
        }
        let emax = 3 * (1i64 << (bits / 16 + 3));
        let state = CtxState {
            ctx: dec::Context {
                prec: 9 * bits / 32 - 2,
                emax,
                emin: 1 - emax,
                round: dec::RoundMode::HalfEven,
                capitals: true,
                clamp: true,
            },
            status: 0,
            traps: 0,
        };
        Ok(PyDecContext::from_state(state).into_ref(&vm.ctx))
    }

    /// Extra module setup: `Decimal` must be a `numbers.Number`, and
    /// `DecimalTuple` is a named tuple built from `collections`.
    pub(crate) fn module_exec(
        vm: &VirtualMachine,
        module: &Py<crate::vm::builtins::PyModule>,
    ) -> PyResult<()> {
        __module_exec(vm, module);
        let tuple_type = decimal_tuple_type(vm)?;
        module.set_attr("DecimalTuple", tuple_type, vm)?;
        let number = vm.import("numbers", 0)?.get_attr("Number", vm)?;
        vm.call_method(&number, "register", (PyDecimal::class(&vm.ctx).to_owned(),))?;
        Ok(())
    }
}
