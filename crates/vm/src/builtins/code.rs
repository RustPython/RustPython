//! Infamous code object. The python class `code`

mod compile_cache;

use super::descriptor::{MemberKind, MemberLayout};
use super::{PyBytesRef, PyStrRef, PyTupleRef, PyType, set::PyFrozenSet};
use crate::common::lock::{OnceCell, PyMutex};
#[cfg(feature = "host_env")]
use crate::convert::ToPyException;
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::PyStrInterned,
    bytecode::{self, BorrowedConstant, CodeFlags, Constant, ConstantBag, Instruction},
    class::{PyClassImpl, StaticType},
    convert::ToPyObject,
    frozen,
    function::OptionalArg,
    types::{Comparable, Constructor, Hashable, Representable},
};
use alloc::fmt;
use core::{
    borrow::Borrow,
    ops::Deref,
    sync::atomic::{AtomicPtr, AtomicU64, Ordering},
};
use malachite_bigint::BigInt;
use num_traits::Zero;
use rustpython_compiler_core::{OneIndexed, bytecode::CodeUnits, bytecode::PyCodeLocationInfoKind};

/// State for iterating through code address ranges
struct PyCodeAddressRange<'a> {
    ar_start: i32,
    ar_end: i32,
    ar_line: i32,
    computed_line: i32,
    reader: LineTableReader<'a>,
}

impl<'a> PyCodeAddressRange<'a> {
    fn new(linetable: &'a [u8], first_line: i32) -> Self {
        PyCodeAddressRange {
            ar_start: 0,
            ar_end: 0,
            ar_line: -1,
            computed_line: first_line,
            reader: LineTableReader::new(linetable),
        }
    }

    /// Check if this is a NO_LINE marker (code 15)
    fn is_no_line_marker(byte: u8) -> bool {
        (byte >> 3) == 0x1f
    }

    /// Advance to next address range
    fn advance(&mut self) -> bool {
        if self.reader.at_end() {
            return false;
        }

        let first_byte = match self.reader.read_byte() {
            Some(b) => b,
            None => return false,
        };

        if (first_byte & 0x80) == 0 {
            return false; // Invalid linetable
        }

        let code = (first_byte >> 3) & 0x0f;
        let length = ((first_byte & 0x07) + 1) as i32;

        // Get line delta for this entry
        let line_delta = self.get_line_delta(code);

        // Update computed line
        self.computed_line += line_delta;

        // Check for NO_LINE marker
        if Self::is_no_line_marker(first_byte) {
            self.ar_line = -1;
        } else {
            self.ar_line = self.computed_line;
        }

        // Update address range
        self.ar_start = self.ar_end;
        self.ar_end += length * 2; // sizeof(_Py_CODEUNIT) = 2

        // Skip remaining bytes for this entry
        while !self.reader.at_end() {
            if let Some(b) = self.reader.peek_byte() {
                if (b & 0x80) != 0 {
                    break;
                }
                self.reader.read_byte();
            } else {
                break;
            }
        }

        true
    }

    fn get_line_delta(&mut self, code: u8) -> i32 {
        let kind = match PyCodeLocationInfoKind::from_code(code) {
            Some(k) => k,
            None => return 0,
        };

        match kind {
            PyCodeLocationInfoKind::None => 0, // NO_LINE marker
            PyCodeLocationInfoKind::Long => {
                let delta = self.reader.read_signed_varint();
                // Skip end_line, col, end_col
                self.reader.read_varint();
                self.reader.read_varint();
                self.reader.read_varint();
                delta
            }
            PyCodeLocationInfoKind::NoColumns => self.reader.read_signed_varint(),
            PyCodeLocationInfoKind::OneLine0 => {
                self.reader.read_byte(); // Skip column
                self.reader.read_byte(); // Skip end column
                0
            }
            PyCodeLocationInfoKind::OneLine1 => {
                self.reader.read_byte(); // Skip column
                self.reader.read_byte(); // Skip end column
                1
            }
            PyCodeLocationInfoKind::OneLine2 => {
                self.reader.read_byte(); // Skip column
                self.reader.read_byte(); // Skip end column
                2
            }
            _ if kind.is_short() => {
                self.reader.read_byte(); // Skip column byte
                0
            }
            _ => 0,
        }
    }
}

#[derive(FromArgs)]
pub struct ReplaceArgs {
    #[pyarg(named, optional)]
    co_posonlyargcount: OptionalArg<u32>,
    #[pyarg(named, optional)]
    co_argcount: OptionalArg<u32>,
    #[pyarg(named, optional)]
    co_kwonlyargcount: OptionalArg<u32>,
    #[pyarg(named, optional)]
    co_filename: OptionalArg<PyStrRef>,
    #[pyarg(named, optional)]
    co_firstlineno: OptionalArg<u32>,
    #[pyarg(named, optional)]
    co_consts: OptionalArg<PyTupleRef>,
    #[pyarg(named, optional)]
    co_name: OptionalArg<PyStrRef>,
    #[pyarg(named, optional)]
    co_names: OptionalArg<PyTupleRef>,
    #[pyarg(named, optional)]
    co_flags: OptionalArg<u32>,
    #[pyarg(named, optional)]
    co_varnames: OptionalArg<Vec<PyObjectRef>>,
    #[pyarg(named, optional)]
    co_nlocals: OptionalArg<u32>,
    #[pyarg(named, optional)]
    co_stacksize: OptionalArg<u32>,
    #[pyarg(named, optional)]
    co_code: OptionalArg<crate::builtins::PyBytesRef>,
    #[pyarg(named, optional)]
    co_linetable: OptionalArg<crate::builtins::PyBytesRef>,
    #[pyarg(named, optional)]
    co_exceptiontable: OptionalArg<crate::builtins::PyBytesRef>,
    #[pyarg(named, optional)]
    co_freevars: OptionalArg<Vec<PyObjectRef>>,
    #[pyarg(named, optional)]
    co_cellvars: OptionalArg<Vec<PyObjectRef>>,
    #[pyarg(named, optional)]
    co_qualname: OptionalArg<PyStrRef>,
}

#[derive(Clone)]
#[repr(transparent)]
pub struct Literal(PyObjectRef);

impl Borrow<PyObject> for Literal {
    fn borrow(&self) -> &PyObject {
        &self.0
    }
}

impl From<Literal> for PyObjectRef {
    fn from(obj: Literal) -> Self {
        obj.0
    }
}

impl From<PyObjectRef> for Literal {
    fn from(obj: PyObjectRef) -> Self {
        Self(obj)
    }
}

fn borrow_obj_constant(obj: &PyObject) -> BorrowedConstant<'_, Literal> {
    match_class!(match obj {
        ref i @ super::int::PyInt => {
            let value = i.as_bigint();
            if obj.class().is(super::bool_::PyBool::static_type()) {
                BorrowedConstant::Boolean {
                    value: !value.is_zero(),
                }
            } else {
                BorrowedConstant::Integer { value }
            }
        }
        ref f @ super::float::PyFloat => BorrowedConstant::Float { value: f.to_f64() },
        ref c @ super::complex::PyComplex => BorrowedConstant::Complex {
            value: c.as_complex()
        },
        ref s @ super::pystr::PyStr => BorrowedConstant::Str { value: s.as_wtf8() },
        ref b @ super::bytes::PyBytes => BorrowedConstant::Bytes {
            value: b.as_bytes()
        },
        ref c @ PyCode => {
            BorrowedConstant::Code { code: &c.code }
        }
        ref t @ super::tuple::PyTuple => {
            let elements = t.as_slice();
            // SAFETY: Literal is repr(transparent) over PyObjectRef, and a Literal tuple only ever
            //         has other literals as elements
            let elements = unsafe { &*(elements as *const [PyObjectRef] as *const [Literal]) };
            BorrowedConstant::Tuple { elements }
        }
        super::singletons::PyNone => BorrowedConstant::None,
        super::slice::PyEllipsis => BorrowedConstant::Ellipsis,
        ref s @ super::slice::PySlice => {
            // Constant pool slices always store Some() for start/step (even for None).
            // Box::leak the array so it outlives the borrow. Leak is acceptable since
            // constant pool objects live for the program's lifetime.
            let start = s.start.clone().unwrap();
            let stop = s.stop.clone();
            let step = s.step.clone().unwrap();
            let arr = Box::leak(Box::new([Literal(start), Literal(stop), Literal(step)]));
            BorrowedConstant::Slice { elements: arr }
        }
        ref fs @ super::set::PyFrozenSet => {
            // Box::leak the elements so they outlive the borrow. Leak is acceptable since
            // constant pool objects live for the program's lifetime.
            let elems: Vec<Literal> = fs.elements().into_iter().map(Literal).collect();
            let elements = Box::leak(elems.into_boxed_slice());
            BorrowedConstant::Frozenset { elements }
        }
        _ => panic!("unexpected payload for constant python value"),
    })
}

/// Immutable tuple backing for runtime constants. Construction is private so
/// the only exposed slice always borrows a real tuple owner.
#[derive(Clone)]
pub struct LiteralTupleOwner(PyTupleRef);

impl AsRef<[Literal]> for LiteralTupleOwner {
    fn as_ref(&self) -> &[Literal] {
        let elements = self.0.as_slice();
        // SAFETY: Literal is transparent over PyObjectRef and accepts every
        // Python object. The immutable slice cannot outlive this tuple owner.
        unsafe { &*(elements as *const [PyObjectRef] as *const [Literal]) }
    }
}

impl Constant for Literal {
    type TupleOwner = LiteralTupleOwner;
    type Name = &'static PyStrInterned;
    fn borrow_constant(&self) -> BorrowedConstant<'_, Self> {
        borrow_obj_constant(&self.0)
    }
}

/// Whether a string constant reads as a name. Those are the ones interned,
/// the way `all_name_chars` picks them out.
fn is_name_chars(value: &crate::common::wtf8::Wtf8) -> bool {
    value
        .as_bytes()
        .iter()
        .all(|&b| b.is_ascii_alphanumeric() || b == b'_')
}

#[derive(Clone, Copy)]
pub(crate) struct PyVmBag<'a>(pub &'a VirtualMachine);

impl ConstantBag for PyVmBag<'_> {
    type Constant = Literal;

    fn make_constant<C: Constant>(&self, constant: BorrowedConstant<'_, C>) -> Self::Constant {
        let vm = self.0;
        let ctx = &vm.ctx;
        let obj = match constant {
            BorrowedConstant::Integer { value } => ctx.new_bigint(value).into(),
            BorrowedConstant::Float { value } => ctx.new_float(value).into(),
            BorrowedConstant::Complex { value } => ctx.new_complex(value).into(),
            BorrowedConstant::Str { value } if is_name_chars(value) => {
                ctx.intern_str(value).to_object()
            }
            BorrowedConstant::Str { value } => ctx.new_str(value).into(),
            BorrowedConstant::Bytes { value } => ctx.new_bytes(value.to_vec()).into(),
            BorrowedConstant::Boolean { value } => ctx.new_bool(value).into(),
            BorrowedConstant::Code { code } => {
                PyCode::new_ref_with_bag(vm, code.map_clone_bag(self)).into()
            }
            BorrowedConstant::Tuple { elements } => {
                let elements = elements
                    .iter()
                    .map(|constant| self.make_constant(constant.borrow_constant()).0)
                    .collect();
                ctx.new_tuple(elements).into()
            }
            BorrowedConstant::Slice { elements } => {
                let [start, stop, step] = elements;
                let start_obj = self.make_constant(start.borrow_constant()).0;
                let stop_obj = self.make_constant(stop.borrow_constant()).0;
                let step_obj = self.make_constant(step.borrow_constant()).0;
                use crate::builtins::PySlice;
                PySlice {
                    start: Some(start_obj),
                    stop: stop_obj,
                    step: Some(step_obj),
                }
                .into_ref(ctx)
                .into()
            }
            BorrowedConstant::Frozenset { elements } => {
                let elements = elements
                    .iter()
                    .map(|constant| self.make_constant(constant.borrow_constant()).0);
                PyFrozenSet::from_iter(vm, elements)
                    .unwrap()
                    .into_ref(ctx)
                    .into()
            }
            BorrowedConstant::None => ctx.none(),
            BorrowedConstant::Ellipsis => ctx.ellipsis.clone().into(),
        };

        Literal(obj)
    }

    fn make_name(&self, name: &str) -> &'static PyStrInterned {
        self.0.ctx.intern_str(name)
    }

    fn make_int(&self, value: BigInt) -> Self::Constant {
        Literal(self.0.ctx.new_int(value).into())
    }

    fn make_tuple(&self, elements: impl Iterator<Item = Self::Constant>) -> Self::Constant {
        Literal(
            self.0
                .ctx
                .new_tuple(elements.map(|lit| lit.0).collect())
                .into(),
        )
    }

    fn make_code(&self, code: CodeObject) -> Self::Constant {
        Literal(PyCode::new_ref_with_bag(self.0, code).into())
    }
}

pub(crate) type CodeObject = bytecode::CodeObject<Literal>;

pub trait IntoCodeObject {
    fn into_code_object(self, vm: &VirtualMachine) -> CodeObject;

    /// Finalize runtime fields before publication. Basic compilation overrides
    /// this to share one private pool; frozen and Literal routes keep defaults.
    fn into_pycode(self, vm: &VirtualMachine) -> PyCode
    where
        Self: Sized,
    {
        PyCode::new(self.into_code_object(vm), &vm.ctx)
    }
}

impl IntoCodeObject for CodeObject {
    fn into_code_object(self, _vm: &VirtualMachine) -> Self {
        self
    }
}

impl IntoCodeObject for bytecode::CodeObject {
    fn into_code_object(self, vm: &VirtualMachine) -> CodeObject {
        // Raw callers receive pooled constants/nested bodies, but no promise
        // of root assembly-field identity; use into_pycode for that boundary.
        compile_cache::into_code_object(self, vm)
    }

    fn into_pycode(self, vm: &VirtualMachine) -> PyCode {
        compile_cache::into_pycode(self, vm)
    }
}

impl<B: AsRef<[u8]>> IntoCodeObject for frozen::FrozenCodeObject<B> {
    fn into_code_object(self, vm: &VirtualMachine) -> CodeObject {
        self.decode(PyVmBag(vm))
    }
}

/// Per-code-object monitoring data (_PyCoMonitoringData).
/// Stores original opcodes displaced by INSTRUMENTED_LINE / INSTRUMENTED_INSTRUCTION.
pub struct CoMonitoringData {
    /// Original opcodes at positions with INSTRUMENTED_LINE.
    /// Indexed by instruction index. 0 = not instrumented for LINE.
    pub line_opcodes: Vec<u8>,

    /// Original opcodes at positions with INSTRUMENTED_INSTRUCTION.
    /// Indexed by instruction index. 0 = not instrumented for INSTRUCTION.
    pub per_instruction_opcodes: Vec<u8>,
}

#[pyclass(module = false, name = "code")]
pub struct PyCode {
    #[pymember(name = "co_argcount", path = "arg_count")]
    #[pymember(name = "co_posonlyargcount", path = "posonlyarg_count")]
    #[pymember(name = "co_kwonlyargcount", path = "kwonlyarg_count")]
    #[pymember(name = "co_stacksize", path = "max_stackdepth")]
    #[pymember(name = "co_qualname", path = "qualname")]
    #[pymember(name = "co_flags", path = "flags")]
    // Logical code fields are finalized by `new`. Execution may specialize or
    // instrument instructions, but must not change the canonical public bytes.
    // Callers replacing logical bytecode must construct a new PyCode instead.
    pub code: CodeObject,
    /// Canonical bytes captured before execution can instrument the code.
    canonical_code: Box<[u8]>,
    /// Preserve a public co_name subclass; exact strings use the interned owner.
    name_owner: Option<PyStrRef>,
    /// The stable Python object exposed by co_code, allocated on first access.
    code_bytes: OnceCell<PyBytesRef>,
    /// Slot-indexed names, equivalent to CPython's `co_localsplusnames`.
    /// Derived once so frame-local proxy operations do not repeatedly scan
    /// merged cell variables.
    localsplus_names: Box<[&'static PyStrInterned]>,
    names_tuple: PyTupleRef,
    localsplus_names_tuple: PyTupleRef,
    localsplus_kinds_bytes: PyBytesRef,
    linetable_bytes: PyBytesRef,
    exceptiontable_bytes: PyBytesRef,
    varnames_tuple: OnceCell<PyTupleRef>,
    cellvars_tuple: OnceCell<PyTupleRef>,
    freevars_tuple: OnceCell<PyTupleRef>,
    #[pymember(name = "co_filename")]
    source_path: AtomicPtr<PyStrInterned>,
    #[pymember(name = "co_nlocals")]
    nlocals: i32,
    /// Version counter for lazy re-instrumentation.
    /// Compared against `PyGlobalState::instrumentation_version` at RESUME.
    pub instrumentation_version: AtomicU64,
    /// Side-table for INSTRUMENTED_LINE / INSTRUMENTED_INSTRUCTION.
    pub monitoring_data: PyMutex<Option<CoMonitoringData>>,
    /// Whether adaptive counters have been initialized (lazy quickening).
    pub quickened: core::sync::atomic::AtomicBool,
    /// Whether the bytecode contains any instruction that mutates the current
    /// exc_info slot (`vm.set_exception`). When false, a normal frame call for
    /// this code cannot leave the slot unbalanced, so `with_frame` skips the
    /// exc_info save/restore. Computed once by scanning the instruction stream.
    pub has_exc_handling: bool,
}

impl Deref for PyCode {
    type Target = CodeObject;
    fn deref(&self) -> &Self::Target {
        &self.code
    }
}

fn build_localspluskinds(
    varnames: &[&'static PyStrInterned],
    cellvars: &[&'static PyStrInterned],
    freevars: &[&'static PyStrInterned],
    arg_counts: (u32, u32, u32),
    flags: CodeFlags,
    instructions: &CodeUnits,
) -> Result<Box<[u8]>, usize> {
    use rustpython_compiler_core::bytecode::{
        CO_FAST_ARG_KW, CO_FAST_ARG_POS, CO_FAST_ARG_VAR, CO_FAST_CELL, CO_FAST_FREE,
        CO_FAST_HIDDEN, CO_FAST_LOCAL, OpArgState,
    };

    let num_merged_cells = cellvars
        .iter()
        .filter(|cell| varnames.iter().any(|local| *local == **cell))
        .count();
    let mut kinds = vec![0; varnames.len() + cellvars.len() - num_merged_cells + freevars.len()];

    let (posonlyarg_count, arg_count, kwonlyarg_count) = arg_counts;
    let positional_only = posonlyarg_count as usize;
    let positional_or_keyword = arg_count.saturating_sub(posonlyarg_count) as usize;
    let argument_kinds = [
        (positional_only, CO_FAST_ARG_POS),
        (positional_or_keyword, CO_FAST_ARG_POS | CO_FAST_ARG_KW),
        (kwonlyarg_count as usize, CO_FAST_ARG_KW),
        (
            usize::from(flags.contains(CodeFlags::VARARGS)),
            CO_FAST_ARG_VAR | CO_FAST_ARG_POS,
        ),
        (
            usize::from(flags.contains(CodeFlags::VARKEYWORDS)),
            CO_FAST_ARG_VAR | CO_FAST_ARG_KW,
        ),
        (usize::MAX, 0),
    ];
    let mut local_index = 0;
    let mut argument_end = 0usize;
    for (count, argument_kind) in argument_kinds {
        argument_end = argument_end.saturating_add(count);
        while local_index < argument_end && local_index < varnames.len() {
            kinds[local_index] = CO_FAST_LOCAL | argument_kind;
            local_index += 1;
        }
    }

    let mut dropped_cells = 0;
    for (cell_index, cell) in cellvars.iter().enumerate() {
        if let Some(local_index) = varnames.iter().position(|local| *local == *cell) {
            kinds[local_index] |= CO_FAST_CELL;
            dropped_cells += 1;
        } else {
            kinds[varnames.len() + cell_index - dropped_cells] = CO_FAST_CELL;
        }
    }

    let free_start = varnames.len() + cellvars.len() - num_merged_cells;
    for kind in kinds.iter_mut().skip(free_start) {
        *kind = CO_FAST_FREE;
    }

    if !flags.contains(CodeFlags::OPTIMIZED) {
        let mut arg_state = OpArgState::default();
        for unit in instructions.iter().copied() {
            let (instruction, arg) = arg_state.get(unit);
            if matches!(instruction, Instruction::LoadFastAndClear { .. }) {
                let index = u32::from(arg) as usize;
                let Some(kind) = kinds.get_mut(index) else {
                    return Err(index);
                };
                *kind |= CO_FAST_HIDDEN;
            }
        }
    }

    Ok(kinds.into_boxed_slice())
}

impl MemberLayout for CodeFlags {
    const KIND: MemberKind = MemberKind::Int;
}

impl PyCode {
    /// Borrow the actual finalized owner, without reconstructing field values.
    /// The caller keeps its canonical bytecode getter owner through the write.
    pub(crate) fn marshal_field<'a>(
        &'a self,
        field: rustpython_compiler_core::marshal::CodeField,
        bytecode: &'a PyBytesRef,
    ) -> &'a PyObject {
        use rustpython_compiler_core::marshal::CodeField;
        match field {
            CodeField::Bytecode => bytecode.as_object(),
            CodeField::Constants => self
                .code
                .constants
                .tuple_owner()
                .expect("finalized constants")
                .0
                .as_object(),
            CodeField::Names => self.names_tuple.as_object(),
            CodeField::LocalsPlusNames => self.localsplus_names_tuple.as_object(),
            CodeField::LocalsPlusKinds => self.localsplus_kinds_bytes.as_object(),
            CodeField::Name => self.name_object(),
            CodeField::Qualname => self.code.qualname.as_object(),
            CodeField::LineTable => self.linetable_bytes.as_object(),
            CodeField::ExceptionTable => self.exceptiontable_bytes.as_object(),
        }
    }

    /// Finalize a pristine code object before publishing it to the runtime.
    ///
    /// The input must not be cloned from an already instrumented PyCode:
    /// generic monitoring wrappers need side tables that CodeObject does not
    /// carry. Python `replace` reconstructs from canonical bytes instead.
    pub fn new(mut code: CodeObject, ctx: &Context) -> Self {
        if code.constants.tuple_owner().is_none() {
            let boxed = core::mem::replace(&mut code.constants, core::iter::empty().collect());
            let tuple = ctx.new_tuple(
                boxed
                    .into_boxed()
                    .into_vec()
                    .into_iter()
                    .map(|x| x.0)
                    .collect(),
            );
            code.constants = bytecode::Constants::from_tuple(LiteralTupleOwner(tuple));
        }
        code.instructions.normalize_get_iter(code.flags);
        let canonical_code = code.instructions.original_bytes().into_boxed_slice();
        let sp = code.source_path as *const PyStrInterned as *mut PyStrInterned;
        let nlocals = i32::try_from(code.varnames.len()).unwrap_or(i32::MAX);
        let localsplus_names = {
            let varname_ids = code
                .varnames
                .iter()
                .map(|name| name.get_id())
                .collect::<std::collections::HashSet<_>>();
            let names = code
                .varnames
                .iter()
                .chain(
                    code.cellvars
                        .iter()
                        .filter(|name| !varname_ids.contains(&name.get_id())),
                )
                .chain(code.freevars.iter())
                .copied()
                .collect::<Box<[_]>>();
            debug_assert_eq!(names.len(), code.localspluskinds.len());
            names
        };
        // The only opcodes that call `vm.set_exception` (mutating the shared
        // exc_info slot); instrumented variants only replace these base opcodes
        // in place, so scanning the freshly-built stream is a sound predicate.
        let has_exc_handling = code.instructions.iter().any(|u| {
            matches!(
                u.op,
                Instruction::PushExcInfo
                    | Instruction::PopExcept
                    | Instruction::CheckEgMatch
                    | Instruction::EndAsyncFor
                    | Instruction::InstrumentedEndAsyncFor
            )
        });
        let names_tuple = ctx.new_tuple(code.names.iter().map(|name| name.to_object()).collect());
        let localsplus_names_tuple = ctx.new_tuple(
            localsplus_names
                .iter()
                .map(|name| name.to_object())
                .collect(),
        );
        let localsplus_kinds_bytes = ctx.new_bytes(code.localspluskinds.to_vec());
        let linetable_bytes = ctx.new_bytes(code.linetable.to_vec());
        let exceptiontable_bytes = ctx.new_bytes(code.exceptiontable.to_vec());
        Self {
            code,
            names_tuple,
            localsplus_names_tuple,
            localsplus_kinds_bytes,
            linetable_bytes,
            exceptiontable_bytes,
            varnames_tuple: OnceCell::new(),
            cellvars_tuple: OnceCell::new(),
            freevars_tuple: OnceCell::new(),
            canonical_code,
            name_owner: None,
            code_bytes: OnceCell::new(),
            localsplus_names,
            source_path: AtomicPtr::new(sp),
            nlocals,
            instrumentation_version: AtomicU64::new(0),
            monitoring_data: PyMutex::new(None),
            quickened: core::sync::atomic::AtomicBool::new(false),
            has_exc_handling,
        }
    }

    /// Attach validated public-reader fields before publication. Superseded
    /// boxed constants are dropped before the returned code can be observed.
    pub(crate) fn from_marshaled_fields(
        mut code: CodeObject,
        fields: rustpython_compiler_core::marshal::CodeFieldObjects<PyObjectRef>,
        ctx: &Context,
    ) -> Result<Self, rustpython_compiler_core::marshal::MarshalError> {
        use rustpython_compiler_core::marshal::MarshalError;
        let constants = fields
            .constants
            .downcast::<super::PyTuple>()
            .map_err(|_| MarshalError::BadType)?;
        let names = fields
            .names
            .downcast::<super::PyTuple>()
            .map_err(|_| MarshalError::BadType)?;
        let locals = fields
            .localsplus_names
            .downcast::<super::PyTuple>()
            .map_err(|_| MarshalError::BadType)?;
        let kinds = fields
            .localsplus_kinds
            .downcast::<super::PyBytes>()
            .map_err(|_| MarshalError::BadType)?;
        let lines = fields
            .linetable
            .downcast::<super::PyBytes>()
            .map_err(|_| MarshalError::BadType)?;
        let exceptions = fields
            .exceptiontable
            .downcast::<super::PyBytes>()
            .map_err(|_| MarshalError::BadType)?;
        code.constants = bytecode::Constants::from_tuple(LiteralTupleOwner(constants));
        let mut result = Self::new(code, ctx);
        result.retain_normalized_names(names, Some(locals));
        result.localsplus_kinds_bytes = kinds;
        result.linetable_bytes = lines;
        result.exceptiontable_bytes = exceptions;
        Ok(result)
    }

    fn retain_normalized_names(&mut self, names: PyTupleRef, locals: Option<PyTupleRef>) {
        let matches = |tuple: &PyTupleRef, canonical: &[&'static PyStrInterned]| {
            tuple.len() == canonical.len()
                && tuple
                    .iter()
                    .zip(canonical)
                    .all(|(item, name)| item.is(*name))
        };
        if matches(&names, &self.code.names) {
            self.names_tuple = names;
        }
        if let Some(locals) = locals
            && matches(&locals, &self.localsplus_names)
        {
            self.localsplus_names_tuple = locals;
        }
    }

    #[inline(always)]
    pub(crate) fn localsplus_name(&self, index: usize) -> &'static PyStrInterned {
        // Bytecode operands and frame-proxy indices are validated against
        // localspluskinds, whose length is kept equal to localsplus_names by
        // every CodeObject construction path.
        self.localsplus_names[index]
    }

    fn name_object(&self) -> &PyObject {
        self.name_owner
            .as_ref()
            .map_or_else(|| self.code.obj_name.as_object(), |name| name.as_object())
    }

    fn retain_name_subclass(&mut self, name: PyStrRef, ctx: &Context) {
        if !name.class().is(ctx.types.str_type) {
            self.name_owner = Some(name);
        }
    }

    pub fn source_path(&self) -> &'static PyStrInterned {
        // SAFETY: always points to a valid &'static PyStrInterned (interned strings are never deallocated)
        unsafe { &*self.source_path.load(Ordering::Relaxed) }
    }

    pub fn set_source_path(&self, new: &'static PyStrInterned) {
        self.source_path.store(
            new as *const PyStrInterned as *mut PyStrInterned,
            Ordering::Relaxed,
        );
    }

    pub fn new_ref_with_bag(vm: &VirtualMachine, code: CodeObject) -> PyRef<Self> {
        PyRef::new_ref(
            Self::new(code, &vm.ctx),
            vm.ctx.types.code_type.to_owned(),
            None,
        )
    }

    pub fn new_ref_from_bytecode(vm: &VirtualMachine, code: bytecode::CodeObject) -> PyRef<Self> {
        PyRef::new_ref(
            code.into_pycode(vm),
            vm.ctx.types.code_type.to_owned(),
            None,
        )
    }

    pub fn new_ref_from_frozen<B: AsRef<[u8]>>(
        vm: &VirtualMachine,
        code: frozen::FrozenCodeObject<B>,
    ) -> PyRef<Self> {
        let py_code = Self::new_ref_with_bag(vm, code.decode(PyVmBag(vm)));
        apply_frozen_co_filename(&py_code, vm);
        py_code
    }

    #[cfg(feature = "host_env")]
    pub fn from_pyc_path(path: &std::path::Path, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        let name = match path.file_stem() {
            Some(stem) => stem.display().to_string(),
            None => "".to_owned(),
        };
        let content = crate::host_env::fs::read(path).map_err(|e| e.to_pyexception(vm))?;
        Self::from_pyc(
            &content,
            Some(&name),
            Some(&path.display().to_string()),
            Some("<source>"),
            vm,
        )
    }
    #[cfg(not(feature = "host_env"))]
    pub fn from_pyc_path(_path: &std::path::Path, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        Err(vm.new_runtime_error("loading a pyc file requires the `host_env` feature"))
    }
    pub fn from_pyc(
        pyc_bytes: &[u8],
        name: Option<&str>,
        bytecode_path: Option<&str>,
        source_path: Option<&str>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        if !crate::import::check_pyc_magic_number_bytes(pyc_bytes) {
            return Err(vm.new_value_error("pyc bytes has wrong MAGIC"));
        }
        let bootstrap_external = vm.import("_frozen_importlib_external", 0)?;
        let compile_bytecode = bootstrap_external.get_attr("_compile_bytecode", vm)?;
        // 16 is the pyc header length
        let Some((_, code_bytes)) = pyc_bytes.split_at_checked(16) else {
            return Err(vm.new_value_error(format!(
                "pyc_bytes header is broken. 16 bytes expected but {} bytes given.",
                pyc_bytes.len()
            )));
        };
        let code_bytes_obj = vm.ctx.new_bytes(code_bytes.to_vec());
        let compiled =
            compile_bytecode.call((code_bytes_obj, name, bytecode_path, source_path), vm)?;
        compiled.try_downcast(vm)
    }
}

fn frozen_co_filename(path: &str) -> Option<&'static str> {
    match path {
        "_frozen_importlib" => Some("<frozen importlib._bootstrap>"),
        "_frozen_importlib_external" => Some("<frozen importlib._bootstrap_external>"),
        _ => None,
    }
}

fn apply_frozen_co_filename(code: &PyCode, vm: &VirtualMachine) {
    let Some(new) = frozen_co_filename(code.source_path().as_str()) else {
        return;
    };
    set_source_path_tree(code, vm.ctx.intern_str(new));
}

fn set_source_path_tree(code: &PyCode, interned: &'static PyStrInterned) {
    code.set_source_path(interned);
    for constant in code.constants.iter() {
        set_source_path_in_const(&constant.0, interned);
    }
}

fn set_source_path_in_const(obj: &PyObject, interned: &'static PyStrInterned) {
    if let Some(inner) = obj.downcast_ref::<PyCode>() {
        set_source_path_tree(inner, interned);
    } else if let Some(tup) = obj.downcast_ref::<super::PyTuple>() {
        for item in tup {
            set_source_path_in_const(item, interned);
        }
    }
}

impl fmt::Debug for PyCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "code: {:?}", self.code)
    }
}

impl PyCode {
    /// Line number for a byte offset, or -1 when the linetable has no line.
    pub fn addr2line(&self, lasti_bytes: i32) -> i32 {
        if lasti_bytes < 0 {
            return self.code.first_line_number.map_or(-1, |n| n.get() as i32);
        }
        let linetable = self.code.linetable.as_ref();
        if linetable.is_empty() {
            return self.code.first_line_number.map_or(-1, |n| n.get() as i32);
        }
        let first_line = self.code.first_line_number.map_or(0, |n| n.get() as i32);
        let mut range = PyCodeAddressRange::new(linetable, first_line);
        while range.ar_end <= lasti_bytes {
            if !range.advance() {
                return -1;
            }
        }
        range.ar_line
    }
}

impl PyPayload for PyCode {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.code_type
    }
}

impl Representable for PyCode {
    #[inline]
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        let code = &zelf.code;
        Ok(format!(
            "<code object {} at {:#x}, file \"{}\", line {}>",
            code.obj_name,
            zelf.get_id(),
            zelf.source_path().as_str(),
            code.first_line_number.map_or(-1, |n| n.get() as i32)
        ))
    }
}

impl Comparable for PyCode {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: crate::types::PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<crate::function::PyComparisonValue> {
        op.eq_only(|| {
            let other = class_or_notimplemented!(Self, other);
            let a = &zelf.code;
            let b = &other.code;
            let eq = vm.bool_eq(zelf.name_object(), other.name_object())?
                && a.arg_count == b.arg_count
                && a.posonlyarg_count == b.posonlyarg_count
                && a.kwonlyarg_count == b.kwonlyarg_count
                && a.flags == b.flags
                && a.first_line_number == b.first_line_number
                && a.instructions.original_bytes() == b.instructions.original_bytes()
                && a.linetable == b.linetable
                && a.exceptiontable == b.exceptiontable
                && a.names == b.names
                && a.varnames == b.varnames
                && a.freevars == b.freevars
                && a.cellvars == b.cellvars
                && {
                    let a_consts: Vec<_> = a.constants.iter().map(|c| c.0.clone()).collect();
                    let b_consts: Vec<_> = b.constants.iter().map(|c| c.0.clone()).collect();
                    if a_consts.len() != b_consts.len() {
                        false
                    } else {
                        let mut eq = true;
                        for (ac, bc) in a_consts.iter().zip(b_consts.iter()) {
                            if !vm.bool_eq(ac, bc)? {
                                eq = false;
                                break;
                            }
                        }
                        eq
                    }
                };
            Ok(eq.into())
        })
    }
}

impl Hashable for PyCode {
    fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<crate::common::hash::PyHash> {
        let code = &zelf.code;
        // Hash a tuple of key attributes, matching CPython's code_hash
        let tuple = vm.ctx.new_tuple(vec![
            zelf.name_object().to_owned(),
            vm.ctx.new_int(code.arg_count).into(),
            vm.ctx.new_int(code.posonlyarg_count).into(),
            vm.ctx.new_int(code.kwonlyarg_count).into(),
            vm.ctx.new_int(code.varnames.len()).into(),
            vm.ctx.new_int(code.flags.bits()).into(),
            vm.ctx
                .new_int(code.first_line_number.map_or(0, |n| n.get()) as i64)
                .into(),
            vm.ctx.new_bytes(code.instructions.original_bytes()).into(),
            {
                let consts: Vec<_> = code.constants.iter().map(|c| c.0.clone()).collect();
                vm.ctx.new_tuple(consts).into()
            },
        ]);
        tuple.as_object().hash(vm)
    }
}

// Arguments for code object constructor
#[derive(FromArgs)]
pub struct PyCodeNewArgs {
    argcount: u32,
    posonlyargcount: u32,
    kwonlyargcount: u32,
    nlocals: u32,
    stacksize: u32,
    flags: u32,
    co_code: PyBytesRef,
    consts: PyTupleRef,
    names: PyTupleRef,
    varnames: PyTupleRef,
    filename: PyStrRef,
    name: PyStrRef,
    qualname: PyStrRef,
    firstlineno: i32,
    linetable: PyBytesRef,
    exceptiontable: PyBytesRef,
    freevars: PyTupleRef,
    cellvars: PyTupleRef,
}

impl Constructor for PyCode {
    type Args = PyCodeNewArgs;

    fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
        // Convert names tuple to vector of interned strings
        let names: Box<[&'static PyStrInterned]> = args
            .names
            .as_slice()
            .iter()
            .map(|obj| {
                let s = obj
                    .downcast_ref::<super::pystr::PyStr>()
                    .ok_or_else(|| vm.new_type_error("names must be tuple of strings"))?;
                Ok(vm.ctx.intern_str(s.as_wtf8()))
            })
            .collect::<PyResult<Vec<_>>>()?
            .into_boxed_slice();

        let varnames: Box<[&'static PyStrInterned]> = args
            .varnames
            .as_slice()
            .iter()
            .map(|obj| {
                let s = obj
                    .downcast_ref::<super::pystr::PyStr>()
                    .ok_or_else(|| vm.new_type_error("varnames must be tuple of strings"))?;
                Ok(vm.ctx.intern_str(s.as_wtf8()))
            })
            .collect::<PyResult<Vec<_>>>()?
            .into_boxed_slice();

        let cellvars: Box<[&'static PyStrInterned]> = args
            .cellvars
            .as_slice()
            .iter()
            .map(|obj| {
                let s = obj
                    .downcast_ref::<super::pystr::PyStr>()
                    .ok_or_else(|| vm.new_type_error("cellvars must be tuple of strings"))?;
                Ok(vm.ctx.intern_str(s.as_wtf8()))
            })
            .collect::<PyResult<Vec<_>>>()?
            .into_boxed_slice();

        let freevars: Box<[&'static PyStrInterned]> = args
            .freevars
            .as_slice()
            .iter()
            .map(|obj| {
                let s = obj
                    .downcast_ref::<super::pystr::PyStr>()
                    .ok_or_else(|| vm.new_type_error("freevars must be tuple of strings"))?;
                Ok(vm.ctx.intern_str(s.as_wtf8()))
            })
            .collect::<PyResult<Vec<_>>>()?
            .into_boxed_slice();

        // Check nlocals matches varnames length
        if args.nlocals as usize != varnames.len() {
            return Err(vm.new_value_error(format!(
                "nlocals ({}) != len(varnames) ({})",
                args.nlocals,
                varnames.len()
            )));
        }

        // Parse and validate bytecode from bytes
        let bytecode_bytes = args.co_code.as_bytes();
        let instructions = CodeUnits::try_from(bytecode_bytes)
            .map_err(|e| vm.new_value_error(format!("invalid bytecode: {e}")))?;

        let constants = bytecode::Constants::from_tuple(LiteralTupleOwner(args.consts));

        // Create locations (start and end pairs)
        let row = if args.firstlineno > 0 {
            OneIndexed::new(args.firstlineno as usize).unwrap_or(OneIndexed::MIN)
        } else {
            OneIndexed::MIN
        };
        let loc = rustpython_compiler_core::SourceLocation {
            line: row,
            character_offset: OneIndexed::from_zero_indexed(0),
        };
        let locations: Box<
            [(
                rustpython_compiler_core::SourceLocation,
                rustpython_compiler_core::SourceLocation,
            )],
        > = vec![(loc, loc); instructions.len()].into_boxed_slice();

        let flags = CodeFlags::from_bits_truncate(args.flags);
        let localspluskinds = build_localspluskinds(
            &varnames,
            &cellvars,
            &freevars,
            (args.posonlyargcount, args.argcount, args.kwonlyargcount),
            flags,
            &instructions,
        )
        .map_err(|index| {
            vm.new_value_error(format!(
                "code: LOAD_FAST_AND_CLEAR oparg {index} out of range"
            ))
        })?;

        // Build the CodeObject
        let code = CodeObject {
            instructions,
            locations,
            flags,
            posonlyarg_count: args.posonlyargcount,
            arg_count: args.argcount,
            kwonlyarg_count: args.kwonlyargcount,
            source_path: vm.ctx.intern_str(args.filename.as_wtf8()),
            first_line_number: if args.firstlineno > 0 {
                OneIndexed::new(args.firstlineno as usize)
            } else {
                None
            },
            // Room for one value is always reserved, even where nothing is pushed.
            max_stackdepth: args.stacksize.max(1),
            obj_name: vm.ctx.intern_str(args.name.as_wtf8()),
            qualname: vm.ctx.intern_str(args.qualname.as_wtf8()),
            constants,
            names,
            varnames,
            cellvars,
            freevars,
            localspluskinds,
            linetable: args.linetable.as_bytes().to_vec().into_boxed_slice(),
            exceptiontable: args.exceptiontable.as_bytes().to_vec().into_boxed_slice(),
        };

        let mut result = Self::new(code, &vm.ctx);
        result.retain_name_subclass(args.name, &vm.ctx);
        result.linetable_bytes = args.linetable;
        result.exceptiontable_bytes = args.exceptiontable;
        Ok(result)
    }
}

impl PyCode {
    pub fn co_filename(&self) -> PyStrRef {
        self.source_path().to_owned()
    }
}

#[pyclass(
    itemsize = core::mem::size_of::<u16>(),
    with(Representable, Constructor, Comparable, Hashable),
    flags(HAS_WEAKREF)
)]
impl Py<PyCode> {
    #[pygetset]
    pub fn co_cellvars(&self, vm: &VirtualMachine) -> PyTupleRef {
        self.cellvars_tuple
            .get_or_init(|| {
                vm.ctx
                    .new_tuple(self.cellvars.iter().map(|name| name.to_object()).collect())
            })
            .clone()
    }

    #[pygetset]
    fn co_firstlineno(&self) -> u32 {
        self.code.first_line_number.map_or(0, |n| n.get() as _)
    }

    #[pygetset]
    fn co_name(&self) -> PyStrRef {
        self.name_owner
            .clone()
            .unwrap_or_else(|| self.code.obj_name.to_owned())
    }

    #[pygetset]
    fn co_consts(&self) -> PyTupleRef {
        self.code
            .constants
            .tuple_owner()
            .expect("finalized constants")
            .0
            .clone()
    }

    #[pygetset]
    fn co_names(&self) -> PyTupleRef {
        self.names_tuple.clone()
    }

    #[pygetset]
    pub fn co_varnames(&self, vm: &VirtualMachine) -> PyTupleRef {
        self.varnames_tuple
            .get_or_init(|| {
                vm.ctx.new_tuple(
                    self.code
                        .varnames
                        .iter()
                        .map(|name| name.to_object())
                        .collect(),
                )
            })
            .clone()
    }

    #[pygetset]
    pub fn co_code(&self, vm: &VirtualMachine) -> crate::builtins::PyBytesRef {
        self.code_bytes
            .get_or_init(|| vm.ctx.new_bytes(self.canonical_code.to_vec()))
            .clone()
    }

    #[pygetset]
    pub fn _co_code_adaptive(&self, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
        let incompatible = || vm.new_value_error("incompatible adaptive bytecode layout");
        let legacy =
            || vm.new_value_error("legacy private GET_YIELD_FROM_ITER has no public 3.15 encoding");
        let (canonical, []) = self.canonical_code.as_chunks::<2>() else {
            return Err(incompatible());
        };
        let instructions = &self.code.instructions;
        if canonical.len() != instructions.unit_count() {
            return Err(incompatible());
        }
        let mut bytes = Vec::with_capacity(self.canonical_code.len());
        let mut index = 0;
        while index < canonical.len() {
            let base = Instruction::try_from(canonical[index][0]).map_err(|_| incompatible())?;
            if matches!(base, Instruction::GetYieldFromIter) {
                return Err(legacy());
            }
            let next = index
                .checked_add(1 + base.cache_entries())
                .filter(|&next| next <= canonical.len())
                .ok_or_else(incompatible)?;
            // Boundaries come only from this code's immutable canonical backing.
            // Never decode a cache payload or use a live monitoring wrapper's width.
            let op = match instructions.read_op(index) {
                op @ (Instruction::LoadConstImmortal | Instruction::LoadConstMortal) => {
                    op.deoptimize()
                }
                Instruction::GetYieldFromIter => return Err(legacy()),
                op => op,
            };
            bytes.push(u8::from(op));
            bytes.push(u8::from(instructions.read_arg(index)));
            for cache in index + 1..next {
                bytes.extend_from_slice(&instructions.read_cache_u16(cache).to_ne_bytes());
            }
            index = next;
        }
        // Per-cell atomic observation, not a coherent cross-thread snapshot.
        // This does not repair the interpreter's other live-storage access paths.
        Ok(vm.ctx.new_bytes(bytes))
    }

    #[pygetset]
    pub fn co_freevars(&self, vm: &VirtualMachine) -> PyTupleRef {
        self.freevars_tuple
            .get_or_init(|| {
                vm.ctx.new_tuple(
                    self.code
                        .freevars
                        .iter()
                        .map(|name| name.to_object())
                        .collect(),
                )
            })
            .clone()
    }

    #[pygetset]
    pub fn co_linetable(&self) -> PyBytesRef {
        self.linetable_bytes.clone()
    }

    #[pygetset]
    pub fn co_exceptiontable(&self) -> PyBytesRef {
        self.exceptiontable_bytes.clone()
    }

    // spell-checker: ignore lnotab
    // co_lnotab is intentionally not implemented.
    // It was deprecated since 3.12 and scheduled for removal in 3.14.
    // Use co_lines() or co_linetable instead.

    #[pymethod]
    pub fn co_lines(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        // TODO: Implement lazy iterator (lineiterator) like CPython for better performance
        // Currently returns eager list for simplicity

        // Return an iterator over (start_offset, end_offset, lineno) tuples
        let linetable = self.code.linetable.as_ref();
        let mut lines = Vec::new();

        if !linetable.is_empty() {
            let first_line = self.code.first_line_number.map_or(0, |n| n.get() as i32);
            let mut range = PyCodeAddressRange::new(linetable, first_line);

            // Process all address ranges and merge consecutive entries with same line
            let mut pending_entry: Option<(i32, i32, i32)> = None;

            while range.advance() {
                let start = range.ar_start;
                let end = range.ar_end;
                let line = range.ar_line;

                if let Some((prev_start, _, prev_line)) = pending_entry {
                    if prev_line == line {
                        // Same line, extend the range
                        pending_entry = Some((prev_start, end, prev_line));
                    } else {
                        // Different line, emit the previous entry
                        let tuple = if prev_line == -1 {
                            vm.ctx.new_tuple(vec![
                                vm.ctx.new_int(prev_start).into(),
                                vm.ctx.new_int(start).into(),
                                vm.ctx.none(),
                            ])
                        } else {
                            vm.ctx.new_tuple(vec![
                                vm.ctx.new_int(prev_start).into(),
                                vm.ctx.new_int(start).into(),
                                vm.ctx.new_int(prev_line).into(),
                            ])
                        };
                        lines.push(tuple.into());
                        pending_entry = Some((start, end, line));
                    }
                } else {
                    // First entry
                    pending_entry = Some((start, end, line));
                }
            }

            // Emit the last pending entry
            if let Some((start, end, line)) = pending_entry {
                let tuple = if line == -1 {
                    vm.ctx.new_tuple(vec![
                        vm.ctx.new_int(start).into(),
                        vm.ctx.new_int(end).into(),
                        vm.ctx.none(),
                    ])
                } else {
                    vm.ctx.new_tuple(vec![
                        vm.ctx.new_int(start).into(),
                        vm.ctx.new_int(end).into(),
                        vm.ctx.new_int(line).into(),
                    ])
                };
                lines.push(tuple.into());
            }
        }

        let list = vm.ctx.new_list(lines);
        vm.call_method(list.as_object(), "__iter__", ())
    }

    #[pymethod]
    pub fn co_positions(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        // Return an iterator over (line, end_line, column, end_column) tuples for each instruction
        let linetable = self.code.linetable.as_ref();
        let mut positions = Vec::new();

        if !linetable.is_empty() {
            let mut reader = LineTableReader::new(linetable);
            let mut line = self.code.first_line_number.map_or(0, |n| n.get() as i32);

            while !reader.at_end() {
                let first_byte = match reader.read_byte() {
                    Some(b) => b,
                    None => break,
                };

                if (first_byte & 0x80) == 0 {
                    break; // Invalid linetable
                }

                let code = (first_byte >> 3) & 0x0f;
                let length = ((first_byte & 0x07) + 1) as i32;

                let kind = match PyCodeLocationInfoKind::from_code(code) {
                    Some(k) => k,
                    None => break, // Invalid code
                };

                let (line_delta, end_line_delta, column, end_column): (
                    i32,
                    i32,
                    Option<i32>,
                    Option<i32>,
                ) = match kind {
                    PyCodeLocationInfoKind::None => {
                        // No location - all values are None
                        (0, 0, None, None)
                    }
                    PyCodeLocationInfoKind::Long => {
                        // Long form
                        let delta = reader.read_signed_varint();
                        let end_line_delta = reader.read_varint() as i32;

                        let col = reader.read_varint();
                        let column = if col == 0 {
                            None
                        } else {
                            Some((col - 1) as i32)
                        };

                        let end_col = reader.read_varint();
                        let end_column = if end_col == 0 {
                            None
                        } else {
                            Some((end_col - 1) as i32)
                        };

                        // endline = line + end_line_delta (will be computed after line update)
                        (delta, end_line_delta, column, end_column)
                    }
                    PyCodeLocationInfoKind::NoColumns => {
                        // No column form
                        let delta = reader.read_signed_varint();
                        (delta, 0, None, None) // endline will be same as line (delta = 0)
                    }
                    PyCodeLocationInfoKind::OneLine0
                    | PyCodeLocationInfoKind::OneLine1
                    | PyCodeLocationInfoKind::OneLine2 => {
                        // One-line form - endline = line
                        let col = reader.read_byte().unwrap_or(0) as i32;
                        let end_col = reader.read_byte().unwrap_or(0) as i32;
                        let delta = kind.one_line_delta().unwrap_or(0);
                        (delta, 0, Some(col), Some(end_col)) // endline = line (delta = 0)
                    }
                    _ if kind.is_short() => {
                        // Short form - endline = line
                        let col_data = reader.read_byte().unwrap_or(0);
                        let col_group = kind.short_column_group().unwrap_or(0);
                        let col = ((col_group as i32) << 3) | ((col_data >> 4) as i32);
                        let end_col = col + (col_data & 0x0f) as i32;
                        (0, 0, Some(col), Some(end_col)) // endline = line (delta = 0)
                    }
                    _ => (0, 0, None, None),
                };

                // Update line number
                line += line_delta;

                // Generate position tuples for each instruction covered by this entry
                for _ in 0..length {
                    // Handle special case for no location (code 15)
                    let final_line = if kind == PyCodeLocationInfoKind::None {
                        None
                    } else {
                        Some(line)
                    };

                    let final_endline = if kind == PyCodeLocationInfoKind::None {
                        None
                    } else {
                        Some(line + end_line_delta)
                    };

                    let line_obj = final_line.to_pyobject(vm);
                    let end_line_obj = final_endline.to_pyobject(vm);
                    let column_obj = column.to_pyobject(vm);
                    let end_column_obj = end_column.to_pyobject(vm);

                    let tuple =
                        vm.ctx
                            .new_tuple(vec![line_obj, end_line_obj, column_obj, end_column_obj]);
                    positions.push(tuple.into());
                }
            }
        }

        let list = vm.ctx.new_list(positions);
        vm.call_method(list.as_object(), "__iter__", ())
    }

    #[pymethod]
    pub fn co_branches(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let instructions = &self.code.instructions;
        let mut branches = Vec::new();
        let mut extended_arg: u32 = 0;

        for (i, unit) in instructions.iter().enumerate() {
            // De-instrument: use base opcode for instrumented variants
            let op = unit.op.to_base().unwrap_or(unit.op);
            let raw_arg = u32::from(u8::from(unit.arg));

            if matches!(op, Instruction::ExtendedArg) {
                extended_arg = (extended_arg | raw_arg) << 8;
                continue;
            }

            let oparg = extended_arg | raw_arg;
            extended_arg = 0;

            let caches = op.cache_entries();
            let (src, left, right) = match op {
                Instruction::ForIter { .. } => {
                    // left = fall-through past CACHE entries (continue iteration)
                    // right = next_offset + oparg + 2 (skip END_FOR and POP_ITER)
                    let after_cache = i + 1 + caches;
                    let target = after_cache + oparg as usize;
                    let right = if matches!(
                        instructions.get(target).map(|u| u.op),
                        Some(Instruction::EndFor | Instruction::InstrumentedEndFor)
                    ) {
                        (target + 2) * 2
                    } else {
                        target * 2
                    };
                    (i * 2, after_cache * 2, right)
                }
                Instruction::PopJumpIfFalse { .. }
                | Instruction::PopJumpIfTrue { .. }
                | Instruction::PopJumpIfNone { .. }
                | Instruction::PopJumpIfNotNone { .. } => {
                    // left = fall-through past CACHE entries (skip NOT_TAKEN if present)
                    // right = jump target (relative forward from after instruction+caches)
                    let after_cache = i + 1 + caches;
                    let next_op = instructions
                        .get(after_cache)
                        .map(|u| u.op.to_base().unwrap_or(u.op));
                    let fallthrough = if matches!(next_op, Some(Instruction::NotTaken)) {
                        (after_cache + 1) * 2
                    } else {
                        after_cache * 2
                    };
                    let right_target = after_cache + oparg as usize;
                    (i * 2, fallthrough, right_target * 2)
                }
                Instruction::EndAsyncFor => {
                    // src = END_SEND position (next_i - oparg)
                    let next_i = i + 1;
                    let Some(src_i) = next_i.checked_sub(oparg as usize) else {
                        continue;
                    };
                    // left = fall-through past NOT_TAKEN
                    (src_i * 2, (src_i + 2) * 2, next_i * 2)
                }
                _ => continue,
            };

            let tuple = vm.ctx.new_tuple(vec![
                vm.ctx.new_int(src).into(),
                vm.ctx.new_int(left).into(),
                vm.ctx.new_int(right).into(),
            ]);
            branches.push(tuple.into());
        }

        let list = vm.ctx.new_list(branches);
        vm.call_method(list.as_object(), "__iter__", ())
    }

    #[pymethod]
    pub fn __replace__(&self, args: ReplaceArgs, vm: &VirtualMachine) -> PyResult<PyCode> {
        self.replace(args, vm)
    }

    #[pymethod]
    pub fn replace(&self, args: ReplaceArgs, vm: &VirtualMachine) -> PyResult<PyCode> {
        let ReplaceArgs {
            co_posonlyargcount,
            co_argcount,
            co_kwonlyargcount,
            co_filename,
            co_firstlineno,
            co_consts,
            co_name,
            co_names,
            co_flags,
            co_varnames,
            co_nlocals,
            co_stacksize,
            co_code,
            co_linetable,
            co_exceptiontable,
            co_freevars,
            co_cellvars,
            co_qualname,
        } = args;
        let posonlyarg_count = match co_posonlyargcount {
            OptionalArg::Present(posonlyarg_count) => posonlyarg_count,
            OptionalArg::Missing => self.code.posonlyarg_count,
        };

        let arg_count = match co_argcount {
            OptionalArg::Present(arg_count) => arg_count,
            OptionalArg::Missing => self.code.arg_count,
        };

        let source_path = match co_filename {
            OptionalArg::Present(source_path) => source_path,
            OptionalArg::Missing => self.source_path().to_owned(),
        };

        let first_line_number = match co_firstlineno {
            OptionalArg::Present(first_line_number) => OneIndexed::new(first_line_number as _),
            OptionalArg::Missing => self.code.first_line_number,
        };

        let kwonlyarg_count = match co_kwonlyargcount {
            OptionalArg::Present(kwonlyarg_count) => kwonlyarg_count,
            OptionalArg::Missing => self.code.kwonlyarg_count,
        };

        let constants = match co_consts {
            OptionalArg::Present(constants) => constants,
            OptionalArg::Missing => self
                .code
                .constants
                .tuple_owner()
                .expect("finalized constants")
                .0
                .clone(),
        };

        let obj_name = match co_name {
            OptionalArg::Present(obj_name) => obj_name,
            OptionalArg::Missing => self.co_name(),
        };

        let names = match co_names {
            OptionalArg::Present(names) => names,
            OptionalArg::Missing => self.names_tuple.clone(),
        };

        let flags = CodeFlags::from_bits_truncate(match co_flags {
            OptionalArg::Present(flags) => flags,
            OptionalArg::Missing => self.code.flags.bits(),
        });

        let varname_objects = match co_varnames {
            OptionalArg::Present(varnames) => varnames,
            OptionalArg::Missing => self.code.varnames.iter().map(|s| s.to_object()).collect(),
        };

        let qualname = match co_qualname {
            OptionalArg::Present(qualname) => qualname,
            OptionalArg::Missing => self.code.qualname.to_owned(),
        };

        // Room for one value is always reserved, even where nothing is pushed.
        let max_stackdepth = match co_stacksize {
            OptionalArg::Present(stacksize) => stacksize,
            OptionalArg::Missing => self.code.max_stackdepth,
        }
        .max(1);

        let instructions = match co_code {
            OptionalArg::Present(code_bytes) => {
                // Parse and validate bytecode from bytes
                CodeUnits::try_from(code_bytes.as_bytes())
                    .map_err(|e| vm.new_value_error(format!("invalid bytecode: {e}")))?
            }
            // Live instructions may contain monitoring wrappers whose original
            // opcodes exist only in the source code object's side tables.
            OptionalArg::Missing => CodeUnits::try_from(self.canonical_code.as_ref())
                .map_err(|e| vm.new_value_error(format!("invalid bytecode: {e}")))?,
        };

        let intern_all = |objs: Vec<PyObjectRef>, field: &str| -> PyResult<Box<[_]>> {
            objs.into_iter()
                .map(|o| {
                    let s = o.downcast_ref::<super::pystr::PyStr>().ok_or_else(|| {
                        vm.new_type_error(format!("{field} must be a tuple of strings"))
                    })?;
                    Ok(vm.ctx.intern_str(s.as_wtf8()))
                })
                .collect::<PyResult<Vec<_>>>()
                .map(Vec::into_boxed_slice)
        };

        let varnames = intern_all(varname_objects, "co_varnames")?;

        let cellvars = match co_cellvars {
            OptionalArg::Present(cellvars) => intern_all(cellvars, "co_cellvars")?,
            OptionalArg::Missing => self.code.cellvars.clone(),
        };

        let freevars = match co_freevars {
            OptionalArg::Present(freevars) => intern_all(freevars, "co_freevars")?,
            OptionalArg::Missing => self.code.freevars.clone(),
        };

        let nlocals = match co_nlocals {
            OptionalArg::Present(nlocals) => nlocals as usize,
            OptionalArg::Missing => self.code.varnames.len(),
        };
        if nlocals != varnames.len() {
            return Err(vm.new_value_error(format!(
                "co_nlocals ({}) != len(co_varnames) ({})",
                nlocals,
                varnames.len()
            )));
        }

        let localspluskinds = build_localspluskinds(
            &varnames,
            &cellvars,
            &freevars,
            (posonlyarg_count, arg_count, kwonlyarg_count),
            flags,
            &instructions,
        )
        .map_err(|index| {
            vm.new_value_error(format!(
                "code: LOAD_FAST_AND_CLEAR oparg {index} out of range"
            ))
        })?;

        // Handle linetable and exceptiontable
        let linetable = match co_linetable {
            OptionalArg::Present(linetable) => linetable,
            OptionalArg::Missing => self.linetable_bytes.clone(),
        };

        let exceptiontable = match co_exceptiontable {
            OptionalArg::Present(exceptiontable) => exceptiontable,
            OptionalArg::Missing => self.exceptiontable_bytes.clone(),
        };

        let new_code = CodeObject {
            flags,
            posonlyarg_count,
            arg_count,
            kwonlyarg_count,
            source_path: vm.ctx.intern_str(source_path.as_wtf8()),
            first_line_number,
            obj_name: vm.ctx.intern_str(obj_name.as_wtf8()),
            qualname: vm.ctx.intern_str(qualname.as_wtf8()),

            max_stackdepth,
            instructions,
            // FIXME: invalid locations. Actually locations is a duplication of linetable.
            // It can be removed once we move every other code to use linetable only.
            locations: self.code.locations.clone(),
            constants: bytecode::Constants::from_tuple(LiteralTupleOwner(constants)),
            names: intern_all(names.as_slice().to_vec(), "co_names")?,
            varnames,
            cellvars,
            freevars,
            localspluskinds,
            linetable: linetable.as_bytes().into(),
            exceptiontable: exceptiontable.as_bytes().into(),
        };

        let mut result = PyCode::new(new_code, &vm.ctx);
        result.retain_name_subclass(obj_name, &vm.ctx);
        result.retain_normalized_names(names, None);
        result.linetable_bytes = linetable;
        result.exceptiontable_bytes = exceptiontable;
        Ok(result)
    }

    #[pymethod]
    fn _varname_from_oparg(&self, opcode: i32, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let idx_err = |vm: &VirtualMachine| vm.new_index_error("tuple index out of range");

        let idx = usize::try_from(opcode).map_err(|_| idx_err(vm))?;

        let varnames_len = self.code.varnames.len();
        // Non-parameter cells: cellvars that are NOT also in varnames
        let nonparam_cellvars: Vec<_> = self
            .code
            .cellvars
            .iter()
            .filter(|s| {
                let s_str: &str = s.as_ref();
                !self.code.varnames.iter().any(|v| {
                    let v_str: &str = v.as_ref();
                    v_str == s_str
                })
            })
            .collect();
        let nonparam_len = nonparam_cellvars.len();

        let name = if idx < varnames_len {
            // Index in varnames (includes parameter cells)
            self.code.varnames.get(idx).ok_or_else(|| idx_err(vm))?
        } else if idx < varnames_len + nonparam_len {
            // Index in non-parameter cellvars
            *nonparam_cellvars
                .get(idx - varnames_len)
                .ok_or_else(|| idx_err(vm))?
        } else {
            // Index in freevars
            self.code
                .freevars
                .get(idx - varnames_len - nonparam_len)
                .ok_or_else(|| idx_err(vm))?
        };
        Ok(name.to_object())
    }
}

impl ToPyObject for CodeObject {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.new_code(self).into()
    }
}

impl ToPyObject for bytecode::CodeObject {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        PyCode::new_ref_from_bytecode(vm, self).into()
    }
}

// Helper struct for reading linetable
struct LineTableReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> LineTableReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn read_byte(&mut self) -> Option<u8> {
        if self.pos < self.data.len() {
            let byte = self.data[self.pos];
            self.pos += 1;
            Some(byte)
        } else {
            None
        }
    }

    fn peek_byte(&self) -> Option<u8> {
        if self.pos < self.data.len() {
            Some(self.data[self.pos])
        } else {
            None
        }
    }

    fn read_varint(&mut self) -> u32 {
        if let Some(first) = self.read_byte() {
            let mut val = (first & 0x3f) as u32;
            let mut shift = 0;
            let mut byte = first;
            while (byte & 0x40) != 0 {
                if let Some(next) = self.read_byte() {
                    shift += 6;
                    val |= ((next & 0x3f) as u32) << shift;
                    byte = next;
                } else {
                    break;
                }
            }
            val
        } else {
            0
        }
    }

    fn read_signed_varint(&mut self) -> i32 {
        let uval = self.read_varint();
        if uval & 1 != 0 {
            -((uval >> 1) as i32)
        } else {
            (uval >> 1) as i32
        }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.data.len()
    }
}

pub(crate) fn init(ctx: &'static Context) {
    PyCode::extend_class(ctx, ctx.types.code_type);
}

#[cfg(test)]
mod storage_tests {
    use super::*;

    #[test]
    fn tuple_constants_share_owner_and_copy_on_write() {
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let child: PyObjectRef = vm.ctx.new_list(vec![]).into();
            assert_eq!(child.strong_count(), 1);
            let owner = vm.ctx.new_tuple(vec![child.clone()]);
            let mut constants = bytecode::Constants::from_tuple(LiteralTupleOwner(owner.clone()));
            assert_eq!(child.strong_count(), 2);
            let shared = constants.clone();
            assert_eq!(child.strong_count(), 2);
            constants[bytecode::oparg::ConstIdx::from_u32(0)] = Literal(vm.ctx.none());
            assert!(constants.tuple_owner().is_none());
            assert!(shared[bytecode::oparg::ConstIdx::from_u32(0)].0.is(&child));
            assert!(owner.as_slice()[0].is(&child));
            assert_eq!(child.strong_count(), 2);
            drop(shared);
            drop(owner);
            assert_eq!(child.strong_count(), 1);
        });
    }

    #[cfg(feature = "compiler")]
    #[test]
    fn published_code_has_one_constant_owner_representation() {
        crate::Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let compiled = vm
                .compile("pass", crate::compiler::Mode::Exec, "code_storage.py")
                .unwrap();
            let child: PyObjectRef = vm.ctx.new_list(vec![]).into();
            let mut raw = compiled.code.clone();
            raw.constants = core::iter::once(Literal(child.clone())).collect();
            assert_eq!(child.strong_count(), 2);
            let code = PyCode::new(raw, &vm.ctx);
            assert_eq!(child.strong_count(), 2);
            let owner = code.code.constants.tuple_owner().unwrap().0.clone();
            let cloned = code.code.clone();
            assert_eq!(child.strong_count(), 2);
            drop(code);
            drop(cloned);
            assert_eq!(child.strong_count(), 2);
            drop(owner);
            assert_eq!(child.strong_count(), 1);
        });
    }
}
