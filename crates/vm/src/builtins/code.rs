//! Infamous code object. The python class `code`

mod const_cache;

use super::descriptor::{MemberKind, MemberLayout};
use super::{PyBytesRef, PyStrRef, PyTupleRef, PyType, set::PyFrozenSet};
use crate::common::lock::{OnceCell, PyMutex};
#[cfg(feature = "host_env")]
use crate::convert::ToPyException;
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
    VirtualMachine,
    builtins::PyStrInterned,
    bytecode::{self, BorrowedConstant, CodeFlags, Constant, ConstantBag, Instruction},
    class::{PyClassImpl, StaticType},
    convert::ToPyObject,
    frozen,
    function::{
        ArgumentError, FromArgs, FuncArgs, KwArgs, NameChanges, OptionalArg, Param, PosArgs,
    },
    object::{Traverse, TraverseFn},
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
use rustpython_common::wtf8::Wtf8Buf;
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

#[derive(Clone, Copy)]
struct CodeInt(i32);

impl TryFromObject for CodeInt {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let value = obj.try_index(vm)?;
        i32::try_from(value.as_bigint())
            .map(Self)
            .map_err(|_| vm.new_overflow_error("Python int too large to convert to C int"))
    }
}

// Keep the caller's objects alive while derived conversion consumes its copy.
// An early __index__ argument must survive later conversions and the audit hook.
pub struct CodeArgs<T> {
    values: T,
    _owners: FuncArgs,
}

impl<T: FromArgs> FromArgs for CodeArgs<T> {
    const PARAMS: Option<&'static [Param]> = T::PARAMS;
    const BINDING_PARAMS: Option<&'static [Param]> = T::BINDING_PARAMS;

    fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
        let owners = args.clone();
        let values = T::from_args(vm, args)?;
        Ok(Self {
            values,
            _owners: owners,
        })
    }
}

#[derive(FromArgs)]
struct ReplaceArgs {
    #[pyarg(named, optional)]
    co_argcount: OptionalArg<CodeInt>,
    #[pyarg(named, optional)]
    co_posonlyargcount: OptionalArg<CodeInt>,
    #[pyarg(named, optional)]
    co_kwonlyargcount: OptionalArg<CodeInt>,
    #[pyarg(named, optional)]
    co_nlocals: OptionalArg<CodeInt>,
    #[pyarg(named, optional)]
    co_stacksize: OptionalArg<CodeInt>,
    #[pyarg(named, optional)]
    co_flags: OptionalArg<CodeInt>,
    #[pyarg(named, optional)]
    co_firstlineno: OptionalArg<CodeInt>,
    #[pyarg(named, optional)]
    co_code: OptionalArg<PyBytesRef>,
    #[pyarg(named, optional)]
    co_consts: OptionalArg<PyTupleRef>,
    #[pyarg(named, optional)]
    co_names: OptionalArg<PyTupleRef>,
    #[pyarg(named, optional)]
    co_varnames: OptionalArg<PyTupleRef>,
    #[pyarg(named, optional)]
    co_freevars: OptionalArg<PyTupleRef>,
    #[pyarg(named, optional)]
    co_cellvars: OptionalArg<PyTupleRef>,
    #[pyarg(named, optional)]
    co_filename: OptionalArg<PyStrRef>,
    #[pyarg(named, optional)]
    co_name: OptionalArg<PyStrRef>,
    #[pyarg(named, optional)]
    co_qualname: OptionalArg<PyStrRef>,
    #[pyarg(named, optional)]
    co_linetable: OptionalArg<PyBytesRef>,
    #[pyarg(named, optional)]
    co_exceptiontable: OptionalArg<PyBytesRef>,
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

/// `co_consts` viewed as constants.
#[derive(Clone)]
pub struct LiteralTuple(PyTupleRef);

impl LiteralTuple {
    pub const fn as_pytuple(&self) -> &PyTupleRef {
        &self.0
    }
}

impl Deref for LiteralTuple {
    type Target = [Literal];

    fn deref(&self) -> &[Literal] {
        let elements = self.0.as_slice();
        // SAFETY: Literal is repr(transparent) over PyObjectRef.
        unsafe { &*(elements as *const [PyObjectRef] as *const [Literal]) }
    }
}

impl FromIterator<Literal> for LiteralTuple {
    fn from_iter<T: IntoIterator<Item = Literal>>(iter: T) -> Self {
        Self(Context::genesis().new_tuple(iter.into_iter().map(|c| c.0).collect()))
    }
}

impl Constant for Literal {
    type Tuple = LiteralTuple;
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

    fn into_pycode(self, vm: &VirtualMachine) -> PyCode
    where
        Self: Sized,
    {
        let mut code = PyCode::new(self.into_code_object(vm), &vm.ctx);
        code.apply_debug_ranges(vm);
        code
    }
}

impl IntoCodeObject for CodeObject {
    fn into_code_object(self, _vm: &VirtualMachine) -> Self {
        self
    }
}

impl IntoCodeObject for bytecode::CodeObject {
    fn into_code_object(self, vm: &VirtualMachine) -> CodeObject {
        const_cache::into_code_object(self, vm)
    }

    fn into_pycode(self, vm: &VirtualMachine) -> PyCode {
        const_cache::into_pycode(self, vm)
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

impl CoMonitoringData {
    fn original_opcode(&self, i: usize, op: Instruction) -> Instruction {
        let op = match op {
            Instruction::InstrumentedLine => Instruction::try_from(self.line_opcodes[i])
                .expect("INSTRUMENTED_LINE without original opcode"),
            op => op,
        };
        match op {
            Instruction::InstrumentedInstruction => {
                Instruction::try_from(self.per_instruction_opcodes[i])
                    .expect("INSTRUMENTED_INSTRUCTION without original opcode")
            }
            op => op,
        }
    }
}

/// `_PyCoCached`: objects created on first use.
#[derive(Default)]
struct CoCached {
    code: OnceCell<PyBytesRef>,
    varnames: OnceCell<PyTupleRef>,
    cellvars: OnceCell<PyTupleRef>,
    freevars: OnceCell<PyTupleRef>,
}

#[pyclass(module = false, name = "code", traverse = "manual", clear = false)]
pub struct PyCode {
    #[pymember(name = "co_argcount", path = "arg_count")]
    #[pymember(name = "co_posonlyargcount", path = "posonlyarg_count")]
    #[pymember(name = "co_kwonlyargcount", path = "kwonlyarg_count")]
    #[pymember(name = "co_stacksize", path = "max_stackdepth")]
    #[pymember(name = "co_flags", path = "flags")]
    pub code: CodeObject,
    /// Slot-indexed names, equivalent to CPython's `co_localsplusnames`.
    /// Derived once so frame-local proxy operations do not repeatedly scan
    /// merged cell variables.
    localsplus_names: Box<[&'static PyStrInterned]>,
    pub(crate) co_name: PyStrRef,
    #[pymember(name = "co_qualname")]
    qualified_name: PyStrRef,
    pub(crate) co_names: PyTupleRef,
    pub(crate) co_localsplusnames: PyTupleRef,
    pub(crate) co_localspluskinds: PyBytesRef,
    pub(crate) co_linetable: PyBytesRef,
    pub(crate) co_exceptiontable: PyBytesRef,
    co_cached: CoCached,
    /// `_co_firsttraceable`: the index of the first `RESUME` that does not
    /// start a generator expression, or the code length if there is none.
    pub(crate) first_traceable: usize,
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

// SAFETY: Visit each owned reference once, including initialized cache cells.
// Constant elements belong to the constants tuple and are not direct edges.
// Interned names and source paths are borrowed static references.
unsafe impl Traverse for PyCode {
    fn traverse(&self, tracer: &mut TraverseFn<'_>) {
        self.co_consts().traverse(tracer);
        self.co_name.traverse(tracer);
        self.qualified_name.traverse(tracer);
        self.co_names.traverse(tracer);
        self.co_localsplusnames.traverse(tracer);
        self.co_localspluskinds.traverse(tracer);
        self.co_linetable.traverse(tracer);
        self.co_exceptiontable.traverse(tracer);
        if let Some(value) = self.co_cached.code.get() {
            value.traverse(tracer);
        }
        if let Some(value) = self.co_cached.varnames.get() {
            value.traverse(tracer);
        }
        if let Some(value) = self.co_cached.cellvars.get() {
            value.traverse(tracer);
        }
        if let Some(value) = self.co_cached.freevars.get() {
            value.traverse(tracer);
        }
    }
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

fn holds_names(tuple: &super::PyTuple, names: &[&'static PyStrInterned]) -> bool {
    tuple.len() == names.len() && tuple.iter().zip(names).all(|(item, name)| item.is(*name))
}

impl MemberLayout for CodeFlags {
    const KIND: MemberKind = MemberKind::Int;
}

impl PyCode {
    pub fn new(mut code: CodeObject, ctx: &Context) -> Self {
        code.instructions.fixup_get_iter(code.flags);
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
        let first_traceable = code
            .instructions
            .iter()
            .position(|unit| {
                matches!(unit.op, Instruction::Resume { .. })
                    && u32::from(u8::from(unit.arg)) & bytecode::oparg::ResumeContext::LOCATION_MASK
                        != bytecode::oparg::ResumeLocation::AtGenExprStart.as_u32()
            })
            .unwrap_or(code.instructions.len());
        let names_tuple = |names: &[&'static PyStrInterned]| {
            ctx.new_tuple(names.iter().map(|name| name.to_object()).collect())
        };
        Self {
            first_traceable,
            co_name: code.obj_name.to_owned(),
            qualified_name: code.qualname.to_owned(),
            co_names: names_tuple(&code.names),
            co_localsplusnames: names_tuple(&localsplus_names),
            co_localspluskinds: ctx.new_bytes(code.localspluskinds.to_vec()),
            co_linetable: ctx.new_bytes(code.linetable.to_vec()),
            co_exceptiontable: ctx.new_bytes(code.exceptiontable.to_vec()),
            co_cached: CoCached::default(),
            code,
            localsplus_names,
            source_path: AtomicPtr::new(sp),
            nlocals,
            instrumentation_version: AtomicU64::new(0),
            monitoring_data: PyMutex::new(None),
            quickened: core::sync::atomic::AtomicBool::new(false),
            has_exc_handling,
        }
    }

    /// The code object `r_object` creates from a TYPE_CODE record. It keeps
    /// the objects read for the object-valued fields.
    pub(crate) fn from_marshal(
        mut code: CodeObject,
        fields: rustpython_compiler_core::marshal::CodeFields<PyObjectRef>,
        ctx: &Context,
    ) -> Result<Self, rustpython_compiler_core::marshal::MarshalError> {
        use rustpython_compiler_core::marshal::MarshalError;
        fn downcast<T: PyPayload>(obj: PyObjectRef) -> Result<PyRef<T>, MarshalError> {
            obj.downcast().map_err(|_| MarshalError::BadType)
        }
        code.constants = bytecode::Constants::new(LiteralTuple(downcast(fields.consts)?));
        // The compiler's string conversion is lossy for surrogates; retain the
        // decoded value and apply code construction's exact-string interning.
        let qualname = downcast::<super::PyStr>(fields.qualname)?;
        code.qualname = ctx.intern_str(qualname.as_wtf8());
        let mut code = Self::new(code, ctx);
        code.set_names(downcast(fields.names)?);
        code.set_localsplusnames(downcast(fields.localsplusnames)?);
        code.co_localspluskinds = downcast(fields.localspluskinds)?;
        code.co_linetable = downcast(fields.linetable)?;
        code.co_exceptiontable = downcast(fields.exceptiontable)?;
        Ok(code)
    }

    /// Keep `names` as `co_names` when it holds the interned names, the way
    /// `intern_strings` interns a names tuple in place.
    fn set_names(&mut self, names: PyTupleRef) {
        if holds_names(&names, &self.code.names) {
            self.co_names = names;
        }
    }

    /// Keep `localsplusnames` as `co_localsplusnames`; see [`Self::set_names`].
    fn set_localsplusnames(&mut self, localsplusnames: PyTupleRef) {
        if holds_names(&localsplusnames, &self.localsplus_names) {
            self.co_localsplusnames = localsplusnames;
        }
    }

    pub(crate) fn apply_debug_ranges(&mut self, vm: &VirtualMachine) {
        if !vm.state.config.settings.code_debug_ranges
            && let Some(linetable) = remove_column_info(&self.code.linetable)
        {
            self.code.linetable = linetable;
            self.co_linetable = vm.ctx.new_bytes(self.code.linetable.to_vec());
        }
    }

    pub(crate) fn co_qualname(&self) -> PyStrRef {
        self.qualified_name.clone()
    }

    #[inline(always)]
    pub(crate) fn localsplus_name(&self, index: usize) -> &'static PyStrInterned {
        // Bytecode operands and frame-proxy indices are validated against
        // localspluskinds, whose length is kept equal to localsplus_names by
        // every CodeObject construction path.
        self.localsplus_names[index]
    }

    pub(crate) fn co_consts(&self) -> &PyTupleRef {
        self.code.constants.as_tuple().as_pytuple()
    }

    /// `_PyCode_GetCode`
    pub(crate) fn get_code(&self, vm: &VirtualMachine) -> PyBytesRef {
        self.co_cached
            .code
            .get_or_init(|| vm.ctx.new_bytes(self.deopt_code()))
            .clone()
    }

    /// `deopt_code`: the instructions with each opcode replaced by its base
    /// opcode (`_Py_GetBaseCodeUnit`) and the inline caches zeroed.
    fn deopt_code(&self) -> Vec<u8> {
        let monitoring_data = self.monitoring_data.lock();
        self.code.instructions.original_bytes_with(|i, op| {
            monitoring_data
                .as_ref()
                .map_or(op, |data| data.original_opcode(i, op))
        })
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
        let mut code = Self::new(code, &vm.ctx);
        code.apply_debug_ranges(vm);
        PyRef::new_ref(code, vm.ctx.types.code_type.to_owned(), None)
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
    let Some(new) = code.source_path().to_str().and_then(frozen_co_filename) else {
        return;
    };
    set_source_path_tree(code, vm.ctx.intern_str(new));
}

pub(crate) fn set_source_path_tree(code: &PyCode, interned: &'static PyStrInterned) {
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
    fn repr_wtf8(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<Wtf8Buf> {
        let code = &zelf.code;
        let mut repr = Wtf8Buf::from(format!(
            "<code object {} at {:#x}, file \"",
            code.obj_name,
            zelf.get_id(),
        ));
        repr.push_wtf8(zelf.source_path().as_wtf8());
        repr.push_str(&format!(
            "\", line {}>",
            code.first_line_number.map_or(-1, |n| n.get() as i32)
        ));
        Ok(repr)
    }
}

/// `_PyCode_ConstantKey`
pub(crate) fn constant_key(op: &PyObject, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
    use super::{PyBool, PyBytes, PyComplex, PyFloat, PyInt, PySlice, PyStr, PyTuple};

    let ctx = &vm.ctx;
    let class = op.class();
    let pair = |first: PyObjectRef| ctx.new_tuple(vec![first, op.to_owned()]).into();
    let triple = |tag: PyObjectRef| {
        ctx.new_tuple(vec![class.to_owned().into(), op.to_owned(), tag])
            .into()
    };
    let is_neg_zero = |d: f64| d == 0.0 && d.is_sign_negative();

    let key = if vm.is_none(op)
        || op.is(&ctx.ellipsis)
        || class.is(PyInt::static_type())
        || class.is(PyStr::static_type())
        || op.downcast_ref::<PyCode>().is_some()
    {
        // Objects of these types are always different from object of other
        // type and from tuples.
        op.to_owned()
    } else if class.is(PyBool::static_type()) || class.is(PyBytes::static_type()) {
        // Make booleans different from integers 0 and 1.
        pair(class.to_owned().into())
    } else if let Some(float) = op.downcast_ref_if_exact::<PyFloat>(vm) {
        if is_neg_zero(float.to_f64()) {
            triple(ctx.none())
        } else {
            pair(class.to_owned().into())
        }
    } else if let Some(complex) = op.downcast_ref_if_exact::<PyComplex>(vm) {
        // All four complex zeros must be distinguished.
        let z = complex.to_complex();
        match (is_neg_zero(z.re), is_neg_zero(z.im)) {
            (true, true) => triple(ctx.new_bool(true).into()),
            (false, true) => triple(ctx.new_bool(false).into()),
            (true, false) => triple(ctx.none()),
            (false, false) => pair(class.to_owned().into()),
        }
    } else if let Some(tuple) = op.downcast_ref_if_exact::<PyTuple>(vm) {
        let keys = tuple
            .iter()
            .map(|item| constant_key(item, vm))
            .collect::<PyResult<_>>()?;
        pair(ctx.new_tuple(keys).into())
    } else if let Some(set) = op.downcast_ref_if_exact::<PyFrozenSet>(vm) {
        let keys = set
            .elements()
            .iter()
            .map(|item| constant_key(item, vm))
            .collect::<PyResult<Vec<_>>>()?;
        pair(PyFrozenSet::from_iter(vm, keys)?.into_ref(ctx).into())
    } else if let Some(slice) = op.downcast_ref::<PySlice>() {
        let key = |item: Option<&PyObjectRef>| match item {
            Some(item) => constant_key(item, vm),
            None => Ok(ctx.none()),
        };
        let slice_key = PySlice {
            start: Some(key(slice.start.as_ref())?),
            stop: key(Some(&slice.stop))?,
            step: Some(key(slice.step.as_ref())?),
        };
        pair(slice_key.into_ref(ctx).into())
    } else {
        // For other types, use the object identifier as a unique identifier
        // to ensure that they are seen as unequal.
        pair(ctx.new_int(op.get_id()).into())
    };
    Ok(key)
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
            let eq = vm.bool_eq(zelf.co_name.as_object(), other.co_name.as_object())?
                && a.arg_count == b.arg_count
                && a.posonlyarg_count == b.posonlyarg_count
                && a.kwonlyarg_count == b.kwonlyarg_count
                && a.flags == b.flags
                && a.first_line_number == b.first_line_number
                && a.instructions.len() == b.instructions.len()
                && zelf.get_code(vm).as_bytes() == other.get_code(vm).as_bytes()
                && {
                    let left = constant_key(Self::co_consts(zelf).as_object(), vm)?;
                    let right = constant_key(Self::co_consts(other).as_object(), vm)?;
                    let eq = vm.bool_eq(&left, &right);
                    // Release both temporary keys in comparison order, including
                    // on errors, before comparing the remaining code fields.
                    drop(left);
                    eq?
                }
                && vm.bool_eq(zelf.co_names.as_object(), other.co_names.as_object())?
                && vm.bool_eq(
                    zelf.co_localsplusnames.as_object(),
                    other.co_localsplusnames.as_object(),
                )?
                && vm.bool_eq(
                    zelf.co_linetable.as_object(),
                    other.co_linetable.as_object(),
                )?
                && vm.bool_eq(
                    zelf.co_exceptiontable.as_object(),
                    other.co_exceptiontable.as_object(),
                )?;
            Ok(eq.into())
        })
    }
}

impl Hashable for PyCode {
    fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<crate::common::hash::PyHash> {
        use crate::common::hash::{self, PyHash, PyUHash};

        let code = &zelf.code;
        let mut hash: PyUHash = 20_221_211;
        let mut scramble = |value: PyUHash| {
            hash = (hash ^ value).wrapping_mul(hash::MULTIPLIER as PyUHash);
        };

        // Hash the retained Python objects in code_hash order: subclass hooks
        // and exceptions must be observed before reading the instruction stream.
        for field in [
            zelf.co_name.as_object(),
            Self::co_consts(zelf).as_object(),
            zelf.co_names.as_object(),
            zelf.co_localsplusnames.as_object(),
            zelf.co_linetable.as_object(),
            zelf.co_exceptiontable.as_object(),
        ] {
            scramble(field.hash(vm)? as PyUHash);
        }
        for value in [
            code.arg_count as PyUHash,
            code.posonlyarg_count as PyUHash,
            code.kwonlyarg_count as PyUHash,
            code.flags.bits() as PyUHash,
            code.first_line_number.map_or(0, |n| n.get()) as PyUHash,
            code.instructions.len() as PyUHash,
        ] {
            scramble(value);
        }

        // Share co_code's original instruction and cache traversal, without hashing
        // adaptive counters or holding the monitoring lock across Python calls.
        let monitoring_data = zelf.monitoring_data.lock();
        let instructions = code.instructions.original_instructions_with(|i, op| {
            monitoring_data
                .as_ref()
                .map_or(op, |data| data.original_opcode(i, op))
        });
        for unit in instructions {
            scramble(u8::from(unit.op) as PyUHash);
            scramble(u8::from(unit.arg) as PyUHash);
        }
        Ok(hash::fix_sentinel(hash as PyHash))
    }
}

// Arguments for code object constructor
#[derive(FromArgs)]
pub struct PyCodeNewArgs {
    #[pyarg(positional)]
    argcount: CodeInt,
    #[pyarg(positional)]
    posonlyargcount: CodeInt,
    #[pyarg(positional)]
    kwonlyargcount: CodeInt,
    #[pyarg(positional)]
    nlocals: CodeInt,
    #[pyarg(positional)]
    stacksize: CodeInt,
    #[pyarg(positional)]
    flags: CodeInt,
    #[pyarg(positional)]
    codestring: PyBytesRef,
    #[pyarg(positional)]
    constants: PyTupleRef,
    #[pyarg(positional)]
    names: PyTupleRef,
    #[pyarg(positional)]
    varnames: PyTupleRef,
    #[pyarg(positional)]
    filename: PyStrRef,
    #[pyarg(positional)]
    name: PyStrRef,
    #[pyarg(positional)]
    qualname: PyStrRef,
    #[pyarg(positional)]
    firstlineno: CodeInt,
    #[pyarg(positional)]
    linetable: PyBytesRef,
    #[pyarg(positional)]
    exceptiontable: PyBytesRef,
    #[pyarg(positional, optional, py_default = "()")]
    freevars: OptionalArg<PyTupleRef>,
    #[pyarg(positional, optional, py_default = "()")]
    cellvars: OptionalArg<PyTupleRef>,
}

struct CodeConstructor {
    argcount: i32,
    posonlyargcount: i32,
    kwonlyargcount: i32,
    nlocals: i32,
    stacksize: i32,
    flags: i32,
    co_code: PyBytesRef,
    consts: PyTupleRef,
    names: PyTupleRef,
    varnames: PyTupleRef,
    freevars: PyTupleRef,
    cellvars: PyTupleRef,
    filename: PyStrRef,
    name: PyStrRef,
    qualname: PyStrRef,
    firstlineno: i32,
    linetable: PyBytesRef,
    exceptiontable: PyBytesRef,
}

fn copy_name_tuple(tuple: &super::PyTuple, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
    let values = tuple
        .as_slice()
        .iter()
        .map(|obj| {
            let string = obj.downcast_ref::<super::PyStr>().ok_or_else(|| {
                let class = obj.class();
                let name = class.slot_name();
                let name = &name[..name.floor_char_boundary(500)];
                vm.new_type_error(format!(
                    "name tuples must contain only strings, not '{name}'"
                ))
            })?;
            Ok(if obj.class().is(vm.ctx.types.str_type) {
                obj.clone()
            } else {
                vm.ctx.new_str(string.as_wtf8()).into()
            })
        })
        .collect::<PyResult<Vec<_>>>()?;
    Ok(vm.ctx.new_tuple(values))
}

fn intern_name_tuple(
    tuple: &super::PyTuple,
    vm: &VirtualMachine,
) -> PyResult<Box<[&'static PyStrInterned]>> {
    // CPython's intern_strings works backwards and requires exact strings.
    let mut names = tuple
        .as_slice()
        .iter()
        .rev()
        .map(|obj| {
            let string = obj
                .downcast_ref_if_exact::<super::PyStr>(vm)
                .ok_or_else(|| vm.new_system_error("non-string found in code slot"))?;
            Ok(vm.ctx.intern_str(string.as_wtf8()))
        })
        .collect::<PyResult<Vec<_>>>()?;
    names.reverse();
    Ok(names.into_boxed_slice())
}

fn audit_code_new(
    code: &PyBytesRef,
    filename: &PyStrRef,
    name: &PyStrRef,
    integers: [i32; 6],
    vm: &VirtualMachine,
) -> PyResult<()> {
    vm.audit("code.__new__", || {
        let mut values = vec![
            code.clone().into(),
            filename.clone().into(),
            name.clone().into(),
        ];
        values.extend(integers.map(|value| value.to_pyobject(vm)));
        PosArgs::new(values)
    })
}

impl CodeConstructor {
    fn build(&self, previous: Option<&CodeObject>, vm: &VirtualMachine) -> PyResult<PyCode> {
        if self.argcount < self.posonlyargcount
            || self.posonlyargcount < 0
            || self.kwonlyargcount < 0
            || self.stacksize < 0
            || self.flags < 0
        {
            return Err(vm.new_system_error("bad argument to internal function"));
        }
        let bytecode = self.co_code.as_bytes();
        if bytecode.len() > i32::MAX as usize {
            return Err(vm.new_overflow_error("code: co_code larger than INT_MAX"));
        }
        if !bytecode.len().is_multiple_of(2) {
            return Err(vm.new_value_error("code: co_code is malformed"));
        }
        let instructions = CodeUnits::try_from(bytecode)
            .map_err(|e| vm.new_value_error(format!("invalid bytecode: {e}")))?;
        let flags = CodeFlags::from_bits_retain(self.flags as u32);
        let nargs = self.argcount as u64
            + self.kwonlyargcount as u64
            + u64::from(flags.contains(CodeFlags::VARARGS))
            + u64::from(flags.contains(CodeFlags::VARKEYWORDS));
        if (self.varnames.len() as u64) < nargs {
            return Err(vm.new_value_error("code: co_varnames is too small"));
        }
        if self.nlocals as usize != self.varnames.len() {
            return Err(vm.new_value_error("code: co_nlocals != len(co_varnames)"));
        }

        // Constructor inputs have already been copied to exact strings;
        // replace inputs reach this check only after structural validation.
        let names = intern_name_tuple(&self.names, vm)?;
        let varnames = intern_name_tuple(&self.varnames, vm)?;
        let cellvars = intern_name_tuple(&self.cellvars, vm)?;
        let freevars = intern_name_tuple(&self.freevars, vm)?;
        let localspluskinds = build_localspluskinds(
            &varnames,
            &cellvars,
            &freevars,
            (
                self.posonlyargcount as u32,
                self.argcount as u32,
                self.kwonlyargcount as u32,
            ),
            flags,
            &instructions,
        )
        .map_err(|index| {
            vm.new_value_error(format!(
                "code: LOAD_FAST_AND_CLEAR oparg {index} out of range"
            ))
        })?;
        let first_line_number = if self.firstlineno > 0 {
            OneIndexed::new(self.firstlineno as usize)
        } else {
            None
        };
        let locations = if let Some(previous) = previous {
            // FIXME: invalid locations. Actually locations is a duplication of linetable.
            // It can be removed once we move every other code to use linetable only.
            previous.locations.clone()
        } else {
            let loc = rustpython_compiler_core::SourceLocation {
                line: first_line_number.unwrap_or(OneIndexed::MIN),
                character_offset: OneIndexed::from_zero_indexed(0),
            };
            vec![(loc, loc); instructions.len()].into_boxed_slice()
        };
        let code = CodeObject {
            instructions,
            locations,
            flags,
            posonlyarg_count: self.posonlyargcount as u32,
            arg_count: self.argcount as u32,
            kwonlyarg_count: self.kwonlyargcount as u32,
            source_path: vm.ctx.intern_str(self.filename.as_wtf8()),
            first_line_number,
            // Room for one value is always reserved, even where nothing is pushed.
            max_stackdepth: self.stacksize.max(1) as u32,
            obj_name: vm.ctx.intern_str(self.name.as_wtf8()),
            qualname: vm.ctx.intern_str(self.qualname.as_wtf8()),
            constants: bytecode::Constants::new(LiteralTuple(self.consts.clone())),
            names,
            varnames,
            cellvars,
            freevars,
            localspluskinds,
            linetable: self.linetable.as_bytes().into(),
            exceptiontable: self.exceptiontable.as_bytes().into(),
        };
        let mut code = PyCode::new(code, &vm.ctx);
        code.co_name = self.name.clone();
        if !self.qualname.class().is(vm.ctx.types.str_type) {
            code.qualified_name = self.qualname.clone();
        }
        code.set_names(self.names.clone());
        code.co_linetable = self.linetable.clone();
        code.co_exceptiontable = self.exceptiontable.clone();
        code.apply_debug_ranges(vm);
        Ok(code)
    }
}

impl Constructor for PyCode {
    type Args = CodeArgs<PyCodeNewArgs>;

    fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
        let CodeArgs {
            values: args,
            _owners,
        } = args;
        let mut constructor = CodeConstructor {
            argcount: args.argcount.0,
            posonlyargcount: args.posonlyargcount.0,
            kwonlyargcount: args.kwonlyargcount.0,
            nlocals: args.nlocals.0,
            stacksize: args.stacksize.0,
            flags: args.flags.0,
            co_code: args.codestring,
            consts: args.constants,
            names: args.names,
            varnames: args.varnames,
            freevars: args
                .freevars
                .unwrap_or_else(|| vm.ctx.new_tuple(Vec::new())),
            cellvars: args
                .cellvars
                .unwrap_or_else(|| vm.ctx.new_tuple(Vec::new())),
            filename: args.filename,
            name: args.name,
            qualname: args.qualname,
            firstlineno: args.firstlineno.0,
            linetable: args.linetable,
            exceptiontable: args.exceptiontable,
        };
        audit_code_new(
            &constructor.co_code,
            &constructor.filename,
            &constructor.name,
            [
                constructor.argcount,
                constructor.posonlyargcount,
                constructor.kwonlyargcount,
                constructor.nlocals,
                constructor.stacksize,
                constructor.flags,
            ],
            vm,
        )?;
        for (name, value) in [
            ("argcount", constructor.argcount),
            ("posonlyargcount", constructor.posonlyargcount),
            ("kwonlyargcount", constructor.kwonlyargcount),
            ("nlocals", constructor.nlocals),
        ] {
            if value < 0 {
                return Err(vm.new_value_error(format!("code: {name} must not be negative")));
            }
        }
        constructor.names = copy_name_tuple(&constructor.names, vm)?;
        constructor.varnames = copy_name_tuple(&constructor.varnames, vm)?;
        constructor.freevars = copy_name_tuple(&constructor.freevars, vm)?;
        constructor.cellvars = copy_name_tuple(&constructor.cellvars, vm)?;
        constructor.build(None, vm)
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
        self.co_cached
            .cellvars
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
        self.co_name.clone()
    }

    #[pygetset]
    fn co_consts(&self) -> PyTupleRef {
        PyCode::co_consts(self).clone()
    }

    #[pygetset]
    fn co_names(&self) -> PyTupleRef {
        self.co_names.clone()
    }

    #[pygetset]
    pub fn co_varnames(&self, vm: &VirtualMachine) -> PyTupleRef {
        self.co_cached
            .varnames
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
    pub fn co_code(&self, vm: &VirtualMachine) -> PyBytesRef {
        self.get_code(vm)
    }

    #[pygetset]
    pub fn _co_code_adaptive(&self, vm: &VirtualMachine) -> PyBytesRef {
        // Return current (possibly quickened/specialized) bytecode
        let bytes = unsafe {
            core::slice::from_raw_parts(
                self.code.instructions.as_ptr() as *const u8,
                self.code.instructions.len() * 2,
            )
        };
        vm.ctx.new_bytes(bytes.to_vec())
    }

    #[pygetset]
    pub fn co_freevars(&self, vm: &VirtualMachine) -> PyTupleRef {
        self.co_cached
            .freevars
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
        self.co_linetable.clone()
    }

    #[pygetset]
    pub fn co_exceptiontable(&self) -> PyBytesRef {
        self.co_exceptiontable.clone()
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
    pub fn __replace__(
        &self,
        args: KwArgs<PyObjectRef, NameChanges>,
        vm: &VirtualMachine,
    ) -> PyResult<PyCode> {
        self.replace(args, vm)
    }

    #[pymethod]
    pub fn replace(
        &self,
        args: KwArgs<PyObjectRef, NameChanges>,
        vm: &VirtualMachine,
    ) -> PyResult<PyCode> {
        // _fix_co_filename can change the source path from an __index__ hook.
        // Capture the default before conversion, as the Clinic wrapper does.
        let default_filename = self.source_path().to_owned();
        let CodeArgs {
            values: args,
            _owners,
        } = FuncArgs::from(args).bind_for::<CodeArgs<ReplaceArgs>>(vm, "replace")?;
        let argcount = args
            .co_argcount
            .map(|value| value.0)
            .unwrap_or(self.code.arg_count as i32);
        let posonlyargcount = args
            .co_posonlyargcount
            .map(|value| value.0)
            .unwrap_or(self.code.posonlyarg_count as i32);
        let kwonlyargcount = args
            .co_kwonlyargcount
            .map(|value| value.0)
            .unwrap_or(self.code.kwonlyarg_count as i32);
        let nlocals = args.co_nlocals.map(|value| value.0).unwrap_or(self.nlocals);
        let stacksize = args
            .co_stacksize
            .map(|value| value.0)
            .unwrap_or(self.code.max_stackdepth as i32);
        let flags = args
            .co_flags
            .map(|value| value.0)
            .unwrap_or(self.code.flags.bits() as i32);
        let firstlineno = args.co_firstlineno.map(|value| value.0).unwrap_or_else(|| {
            self.code
                .first_line_number
                .map_or(0, |line| line.get() as i32)
        });
        for (name, value) in [
            ("co_argcount", argcount),
            ("co_posonlyargcount", posonlyargcount),
            ("co_kwonlyargcount", kwonlyargcount),
            ("co_nlocals", nlocals),
            ("co_stacksize", stacksize),
            ("co_flags", flags),
            ("co_firstlineno", firstlineno),
        ] {
            if value < 0 {
                return Err(vm.new_value_error(format!("{name} must be a positive integer")));
            }
        }
        let co_code = args.co_code.unwrap_or_else(|| self.get_code(vm));
        let filename = args.co_filename.unwrap_or(default_filename);
        let name = args.co_name.unwrap_or_else(|| self.co_name.clone());
        audit_code_new(
            &co_code,
            &filename,
            &name,
            [
                argcount,
                posonlyargcount,
                kwonlyargcount,
                nlocals,
                stacksize,
                flags,
            ],
            vm,
        )?;
        let names_tuple = |names: &[&'static PyStrInterned]| {
            vm.ctx
                .new_tuple(names.iter().map(|name| name.to_object()).collect())
        };
        let constructor = CodeConstructor {
            argcount,
            posonlyargcount,
            kwonlyargcount,
            nlocals,
            stacksize,
            flags,
            co_code,
            consts: args
                .co_consts
                .unwrap_or_else(|| PyCode::co_consts(self).clone()),
            names: args.co_names.unwrap_or_else(|| self.co_names.clone()),
            varnames: args
                .co_varnames
                .unwrap_or_else(|| names_tuple(&self.code.varnames)),
            cellvars: args
                .co_cellvars
                .unwrap_or_else(|| names_tuple(&self.code.cellvars)),
            freevars: args
                .co_freevars
                .unwrap_or_else(|| names_tuple(&self.code.freevars)),
            filename,
            name,
            qualname: args
                .co_qualname
                .unwrap_or_else(|| self.qualified_name.clone()),
            firstlineno,
            linetable: args
                .co_linetable
                .unwrap_or_else(|| self.co_linetable.clone()),
            exceptiontable: args
                .co_exceptiontable
                .unwrap_or_else(|| self.co_exceptiontable.clone()),
        };
        constructor.build(Some(&self.code), vm)
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

// Preserve entry lengths and line deltas while dropping columns, as code
// construction does under -X no_debug_ranges in CPython. Decode and re-encode
// signed deltas to normalize redundant encodings, including negative zero.
// Leave unconvertible tables unchanged rather than inventing line deltas or
// introducing new constructor errors. This is not a general table validator.
fn remove_column_info(linetable: &[u8]) -> Option<Box<[u8]>> {
    let mut result = Vec::with_capacity(linetable.len());
    let mut reader = LineTableReader::new(linetable);
    while let Some(header) = reader.read_byte() {
        if header & 0x80 == 0 {
            return None;
        }
        let kind = (header >> 3) & 15;
        if kind == PyCodeLocationInfoKind::None as u8 {
            result.push(header);
        } else {
            result.push(0x80 | ((PyCodeLocationInfoKind::NoColumns as u8) << 3) | (header & 7));
            let delta = match PyCodeLocationInfoKind::from_code(kind) {
                Some(PyCodeLocationInfoKind::Long | PyCodeLocationInfoKind::NoColumns) => {
                    read_location_delta(&mut reader)?
                }
                Some(PyCodeLocationInfoKind::OneLine1) => 1,
                Some(PyCodeLocationInfoKind::OneLine2) => 2,
                _ => 0,
            };
            let mut encoded = (delta.unsigned_abs() << 1) | u32::from(delta < 0);
            while encoded >= 64 {
                result.push(0x40 | (encoded & 0x3f) as u8);
                encoded >>= 6;
            }
            result.push(encoded as u8);
        }
        while reader.peek_byte().is_some_and(|byte| byte & 0x80 == 0) {
            reader.read_byte();
        }
    }
    Some(result.into_boxed_slice())
}

// Location deltas use an unsigned 32-bit sign/magnitude representation. Allow
// redundant zero groups, but reject overflow and truncation without unchecked
// shifts or consuming the next header. This does not validate absolute lines.
fn read_location_delta(reader: &mut LineTableReader<'_>) -> Option<i32> {
    let mut value = 0u32;
    let mut shift = 0;
    loop {
        let byte = reader.peek_byte()?;
        if byte & 0x80 != 0 {
            return None;
        }
        reader.read_byte();
        let digit = u32::from(byte & 0x3f);
        if shift < u32::BITS {
            if digit > (u32::MAX >> shift) {
                return None;
            }
            value |= digit << shift;
        } else if digit != 0 {
            return None;
        }
        if byte & 0x40 == 0 {
            let magnitude = (value >> 1) as i32;
            return Some(if value & 1 != 0 {
                -magnitude
            } else {
                magnitude
            });
        }
        shift = (shift + 6).min(u32::BITS);
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
