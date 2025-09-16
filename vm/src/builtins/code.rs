/*! Infamous code object. The python class `code`

*/

use super::{PyStrRef, PyTupleRef, PyType, PyTypeRef};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyResult, VirtualMachine,
    builtins::PyStrInterned,
    bytecode::{self, AsBag, BorrowedConstant, CodeFlags, Constant, ConstantBag},
    class::{PyClassImpl, StaticType},
    convert::ToPyObject,
    frozen,
    function::{FuncArgs, OptionalArg},
    types::Representable,
};
use malachite_bigint::BigInt;
use num_traits::Zero;
use rustpython_compiler_core::OneIndexed;
use rustpython_compiler_core::bytecode::PyCodeLocationInfoKind;
use std::{borrow::Borrow, fmt, ops::Deref};

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
    co_consts: OptionalArg<Vec<PyObjectRef>>,
    #[pyarg(named, optional)]
    co_name: OptionalArg<PyStrRef>,
    #[pyarg(named, optional)]
    co_names: OptionalArg<Vec<PyObjectRef>>,
    #[pyarg(named, optional)]
    co_flags: OptionalArg<u16>,
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
            value: c.to_complex()
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
        _ => panic!("unexpected payload for constant python value"),
    })
}

impl Constant for Literal {
    type Name = &'static PyStrInterned;
    fn borrow_constant(&self) -> BorrowedConstant<'_, Self> {
        borrow_obj_constant(&self.0)
    }
}

impl<'a> AsBag for &'a Context {
    type Bag = PyObjBag<'a>;
    fn as_bag(self) -> PyObjBag<'a> {
        PyObjBag(self)
    }
}

impl<'a> AsBag for &'a VirtualMachine {
    type Bag = PyObjBag<'a>;
    fn as_bag(self) -> PyObjBag<'a> {
        PyObjBag(&self.ctx)
    }
}

#[derive(Clone, Copy)]
pub struct PyObjBag<'a>(pub &'a Context);

impl ConstantBag for PyObjBag<'_> {
    type Constant = Literal;

    fn make_constant<C: Constant>(&self, constant: BorrowedConstant<'_, C>) -> Self::Constant {
        let ctx = self.0;
        let obj = match constant {
            BorrowedConstant::Integer { value } => ctx.new_bigint(value).into(),
            BorrowedConstant::Float { value } => ctx.new_float(value).into(),
            BorrowedConstant::Complex { value } => ctx.new_complex(value).into(),
            BorrowedConstant::Str { value } if value.len() <= 20 => {
                ctx.intern_str(value).to_object()
            }
            BorrowedConstant::Str { value } => ctx.new_str(value).into(),
            BorrowedConstant::Bytes { value } => ctx.new_bytes(value.to_vec()).into(),
            BorrowedConstant::Boolean { value } => ctx.new_bool(value).into(),
            BorrowedConstant::Code { code } => ctx.new_code(code.map_clone_bag(self)).into(),
            BorrowedConstant::Tuple { elements } => {
                let elements = elements
                    .iter()
                    .map(|constant| self.make_constant(constant.borrow_constant()).0)
                    .collect();
                ctx.new_tuple(elements).into()
            }
            BorrowedConstant::None => ctx.none(),
            BorrowedConstant::Ellipsis => ctx.ellipsis.clone().into(),
        };

        Literal(obj)
    }

    fn make_name(&self, name: &str) -> &'static PyStrInterned {
        self.0.intern_str(name)
    }

    fn make_int(&self, value: BigInt) -> Self::Constant {
        Literal(self.0.new_int(value).into())
    }

    fn make_tuple(&self, elements: impl Iterator<Item = Self::Constant>) -> Self::Constant {
        Literal(self.0.new_tuple(elements.map(|lit| lit.0).collect()).into())
    }

    fn make_code(&self, code: CodeObject) -> Self::Constant {
        Literal(self.0.new_code(code).into())
    }
}

pub type CodeObject = bytecode::CodeObject<Literal>;

pub trait IntoCodeObject {
    fn into_code_object(self, ctx: &Context) -> CodeObject;
}

impl IntoCodeObject for CodeObject {
    fn into_code_object(self, _ctx: &Context) -> Self {
        self
    }
}

impl IntoCodeObject for bytecode::CodeObject {
    fn into_code_object(self, ctx: &Context) -> CodeObject {
        self.map_bag(PyObjBag(ctx))
    }
}

impl<B: AsRef<[u8]>> IntoCodeObject for frozen::FrozenCodeObject<B> {
    fn into_code_object(self, ctx: &Context) -> CodeObject {
        self.decode(ctx)
    }
}

#[pyclass(module = false, name = "code")]
pub struct PyCode {
    pub code: CodeObject,
}

impl Deref for PyCode {
    type Target = CodeObject;
    fn deref(&self) -> &Self::Target {
        &self.code
    }
}

impl PyCode {
    pub const fn new(code: CodeObject) -> Self {
        Self { code }
    }
}

impl fmt::Debug for PyCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "code: {:?}", self.code)
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
            "<code object {} at {:#x} file {:?}, line {}>",
            code.obj_name,
            zelf.get_id(),
            code.source_path.as_str(),
            code.first_line_number.map_or(-1, |n| n.get() as i32)
        ))
    }
}

#[pyclass(with(Representable))]
impl PyCode {
    #[pyslot]
    fn slot_new(_cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error("Cannot directly create code object"))
    }

    #[pygetset]
    const fn co_posonlyargcount(&self) -> usize {
        self.code.posonlyarg_count as usize
    }

    #[pygetset]
    const fn co_argcount(&self) -> usize {
        self.code.arg_count as usize
    }

    #[pygetset]
    const fn co_stacksize(&self) -> u32 {
        self.code.max_stackdepth
    }

    #[pygetset]
    pub fn co_filename(&self) -> PyStrRef {
        self.code.source_path.to_owned()
    }

    #[pygetset]
    pub fn co_cellvars(&self, vm: &VirtualMachine) -> PyTupleRef {
        let cellvars = self
            .code
            .cellvars
            .deref()
            .iter()
            .map(|name| name.to_pyobject(vm))
            .collect();
        vm.ctx.new_tuple(cellvars)
    }

    #[pygetset]
    fn co_nlocals(&self) -> usize {
        self.varnames.len()
    }

    #[pygetset]
    fn co_firstlineno(&self) -> u32 {
        self.code.first_line_number.map_or(0, |n| n.get() as _)
    }

    #[pygetset]
    const fn co_kwonlyargcount(&self) -> usize {
        self.code.kwonlyarg_count as usize
    }

    #[pygetset]
    fn co_consts(&self, vm: &VirtualMachine) -> PyTupleRef {
        let consts = self.code.constants.iter().map(|x| x.0.clone()).collect();
        vm.ctx.new_tuple(consts)
    }

    #[pygetset]
    fn co_name(&self) -> PyStrRef {
        self.code.obj_name.to_owned()
    }
    #[pygetset]
    fn co_qualname(&self) -> PyStrRef {
        self.code.qualname.to_owned()
    }

    #[pygetset]
    fn co_names(&self, vm: &VirtualMachine) -> PyTupleRef {
        let names = self
            .code
            .names
            .deref()
            .iter()
            .map(|name| name.to_pyobject(vm))
            .collect();
        vm.ctx.new_tuple(names)
    }

    #[pygetset]
    const fn co_flags(&self) -> u16 {
        self.code.flags.bits()
    }

    #[pygetset]
    pub fn co_varnames(&self, vm: &VirtualMachine) -> PyTupleRef {
        let varnames = self.code.varnames.iter().map(|s| s.to_object()).collect();
        vm.ctx.new_tuple(varnames)
    }

    #[pygetset]
    pub fn co_code(&self, vm: &VirtualMachine) -> crate::builtins::PyBytesRef {
        // SAFETY: CodeUnit is #[repr(C)] with size 2, so we can safely transmute to bytes
        let bytes = unsafe {
            std::slice::from_raw_parts(
                self.code.instructions.as_ptr() as *const u8,
                self.code.instructions.len() * 2,
            )
        };
        vm.ctx.new_bytes(bytes.to_vec())
    }

    #[pygetset]
    pub fn co_freevars(&self, vm: &VirtualMachine) -> PyTupleRef {
        let names = self
            .code
            .freevars
            .deref()
            .iter()
            .map(|name| name.to_pyobject(vm))
            .collect();
        vm.ctx.new_tuple(names)
    }

    #[pygetset]
    pub fn co_linetable(&self, vm: &VirtualMachine) -> crate::builtins::PyBytesRef {
        // Return the actual linetable from the code object
        vm.ctx.new_bytes(self.code.linetable.to_vec())
    }

    #[pygetset]
    pub fn co_exceptiontable(&self, vm: &VirtualMachine) -> crate::builtins::PyBytesRef {
        // Return the actual exception table from the code object
        vm.ctx.new_bytes(self.code.exceptiontable.to_vec())
    }

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

                    // Convert Option to PyObject (None or int)
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
    pub fn replace(&self, args: ReplaceArgs, vm: &VirtualMachine) -> PyResult<Self> {
        let posonlyarg_count = match args.co_posonlyargcount {
            OptionalArg::Present(posonlyarg_count) => posonlyarg_count,
            OptionalArg::Missing => self.code.posonlyarg_count,
        };

        let arg_count = match args.co_argcount {
            OptionalArg::Present(arg_count) => arg_count,
            OptionalArg::Missing => self.code.arg_count,
        };

        let source_path = match args.co_filename {
            OptionalArg::Present(source_path) => source_path,
            OptionalArg::Missing => self.code.source_path.to_owned(),
        };

        let first_line_number = match args.co_firstlineno {
            OptionalArg::Present(first_line_number) => OneIndexed::new(first_line_number as _),
            OptionalArg::Missing => self.code.first_line_number,
        };

        let kwonlyarg_count = match args.co_kwonlyargcount {
            OptionalArg::Present(kwonlyarg_count) => kwonlyarg_count,
            OptionalArg::Missing => self.code.kwonlyarg_count,
        };

        let constants = match args.co_consts {
            OptionalArg::Present(constants) => constants,
            OptionalArg::Missing => self.code.constants.iter().map(|x| x.0.clone()).collect(),
        };

        let obj_name = match args.co_name {
            OptionalArg::Present(obj_name) => obj_name,
            OptionalArg::Missing => self.code.obj_name.to_owned(),
        };

        let names = match args.co_names {
            OptionalArg::Present(names) => names,
            OptionalArg::Missing => self
                .code
                .names
                .deref()
                .iter()
                .map(|name| name.to_pyobject(vm))
                .collect(),
        };

        let flags = match args.co_flags {
            OptionalArg::Present(flags) => flags,
            OptionalArg::Missing => self.code.flags.bits(),
        };

        let varnames = match args.co_varnames {
            OptionalArg::Present(varnames) => varnames,
            OptionalArg::Missing => self.code.varnames.iter().map(|s| s.to_object()).collect(),
        };

        let qualname = match args.co_qualname {
            OptionalArg::Present(qualname) => qualname,
            OptionalArg::Missing => self.code.qualname.to_owned(),
        };

        let max_stackdepth = match args.co_stacksize {
            OptionalArg::Present(stacksize) => stacksize,
            OptionalArg::Missing => self.code.max_stackdepth,
        };

        let instructions = match args.co_code {
            OptionalArg::Present(_code_bytes) => {
                // Convert bytes back to instructions
                // For now, keep the original instructions
                // TODO: Properly parse bytecode from bytes
                self.code.instructions.clone()
            }
            OptionalArg::Missing => self.code.instructions.clone(),
        };

        let cellvars = match args.co_cellvars {
            OptionalArg::Present(cellvars) => cellvars
                .into_iter()
                .map(|o| o.as_interned_str(vm).unwrap())
                .collect(),
            OptionalArg::Missing => self.code.cellvars.clone(),
        };

        let freevars = match args.co_freevars {
            OptionalArg::Present(freevars) => freevars
                .into_iter()
                .map(|o| o.as_interned_str(vm).unwrap())
                .collect(),
            OptionalArg::Missing => self.code.freevars.clone(),
        };

        // Validate co_nlocals if provided
        if let OptionalArg::Present(nlocals) = args.co_nlocals
            && nlocals as usize != varnames.len()
        {
            return Err(vm.new_value_error(format!(
                "co_nlocals ({}) != len(co_varnames) ({})",
                nlocals,
                varnames.len()
            )));
        }

        // Handle linetable and exceptiontable
        let linetable = match args.co_linetable {
            OptionalArg::Present(linetable) => linetable.as_bytes().to_vec().into_boxed_slice(),
            OptionalArg::Missing => self.code.linetable.clone(),
        };

        let exceptiontable = match args.co_exceptiontable {
            OptionalArg::Present(exceptiontable) => {
                exceptiontable.as_bytes().to_vec().into_boxed_slice()
            }
            OptionalArg::Missing => self.code.exceptiontable.clone(),
        };

        Ok(Self {
            code: CodeObject {
                flags: CodeFlags::from_bits_truncate(flags),
                posonlyarg_count,
                arg_count,
                kwonlyarg_count,
                source_path: source_path.as_object().as_interned_str(vm).unwrap(),
                first_line_number,
                obj_name: obj_name.as_object().as_interned_str(vm).unwrap(),
                qualname: qualname.as_object().as_interned_str(vm).unwrap(),

                max_stackdepth,
                instructions,
                locations: self.code.locations.clone(),
                constants: constants.into_iter().map(Literal).collect(),
                names: names
                    .into_iter()
                    .map(|o| o.as_interned_str(vm).unwrap())
                    .collect(),
                varnames: varnames
                    .into_iter()
                    .map(|o| o.as_interned_str(vm).unwrap())
                    .collect(),
                cellvars,
                freevars,
                cell2arg: self.code.cell2arg.clone(),
                linetable,
                exceptiontable,
            },
        })
    }
}

impl fmt::Display for PyCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl ToPyObject for CodeObject {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_code(self).into()
    }
}

impl ToPyObject for bytecode::CodeObject {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_code(self).into()
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

pub fn init(ctx: &Context) {
    PyCode::extend_class(ctx, ctx.types.code_type);
}
