/*! Infamous code object. The python class `code`

*/

use super::{PyStrRef, PyTupleRef, PyType, PyTypeRef};
use crate::{
    builtins::PyStrInterned,
    bytecode::{self, BorrowedConstant, Constant, ConstantBag},
    class::{PyClassImpl, StaticType},
    convert::ToPyObject,
    function::FuncArgs,
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
};
use num_traits::Zero;
use std::{borrow::Borrow, fmt, ops::Deref};

#[derive(Clone)]
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

fn borrow_obj_constant(obj: &PyObject) -> BorrowedConstant<Literal> {
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
        ref s @ super::pystr::PyStr => BorrowedConstant::Str { value: s.as_str() },
        ref b @ super::bytes::PyBytes => BorrowedConstant::Bytes {
            value: b.as_bytes()
        },
        ref c @ PyCode => {
            BorrowedConstant::Code { code: &c.code }
        }
        ref t @ super::tuple::PyTuple => {
            BorrowedConstant::Tuple {
                elements: Box::new(t.iter().map(|o| borrow_obj_constant(o))),
            }
        }
        super::singletons::PyNone => BorrowedConstant::None,
        super::slice::PyEllipsis => BorrowedConstant::Ellipsis,
        _ => panic!("unexpected payload for constant python value"),
    })
}

impl Constant for Literal {
    type Name = &'static PyStrInterned;
    fn borrow_constant(&self) -> BorrowedConstant<Self> {
        borrow_obj_constant(&self.0)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct PyObjBag<'a>(pub &'a Context);

impl ConstantBag for PyObjBag<'_> {
    type Constant = Literal;

    fn make_constant<C: Constant>(&self, constant: BorrowedConstant<C>) -> Self::Constant {
        let ctx = self.0;
        let obj = match constant {
            bytecode::BorrowedConstant::Integer { value } => ctx.new_bigint(value).into(),
            bytecode::BorrowedConstant::Float { value } => ctx.new_float(value).into(),
            bytecode::BorrowedConstant::Complex { value } => ctx.new_complex(value).into(),
            bytecode::BorrowedConstant::Str { value } if value.len() <= 20 => {
                ctx.intern_str(value).to_object()
            }
            bytecode::BorrowedConstant::Str { value } => ctx.new_str(value).into(),
            bytecode::BorrowedConstant::Bytes { value } => ctx.new_bytes(value.to_vec()).into(),
            bytecode::BorrowedConstant::Boolean { value } => ctx.new_bool(value).into(),
            bytecode::BorrowedConstant::Code { code } => {
                ctx.new_code(code.map_clone_bag(self)).into()
            }
            bytecode::BorrowedConstant::Tuple { elements } => {
                let elements = elements
                    .into_iter()
                    .map(|constant| self.make_constant(constant).0)
                    .collect();
                ctx.new_tuple(elements).into()
            }
            bytecode::BorrowedConstant::None => ctx.none(),
            bytecode::BorrowedConstant::Ellipsis => ctx.ellipsis(),
        };
        Literal(obj)
    }

    fn make_name(&self, name: &str) -> &'static PyStrInterned {
        self.0.intern_str(name)
    }
}

pub type CodeObject = bytecode::CodeObject<Literal>;

pub trait IntoCodeObject {
    fn into_codeobj(self, ctx: &Context) -> CodeObject;
}

impl IntoCodeObject for CodeObject {
    fn into_codeobj(self, _ctx: &Context) -> CodeObject {
        self
    }
}

impl IntoCodeObject for bytecode::CodeObject {
    fn into_codeobj(self, ctx: &Context) -> CodeObject {
        self.map_bag(PyObjBag(ctx))
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
    pub fn new(code: CodeObject) -> PyCode {
        PyCode { code }
    }
}

impl fmt::Debug for PyCode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "code: {:?}", self.code)
    }
}

impl PyPayload for PyCode {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.code_type
    }
}

#[pyclass(with(PyRef))]
impl PyCode {}

#[pyclass]
impl PyRef<PyCode> {
    #[pyslot]
    fn slot_new(_cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error("Cannot directly create code object".to_owned()))
    }

    #[pymethod(magic)]
    fn repr(self) -> String {
        let code = &self.code;
        format!(
            "<code object {} at {:#x} file {:?}, line {}>",
            code.obj_name,
            self.get_id(),
            code.source_path.as_str(),
            code.first_line_number
        )
    }

    #[pygetset]
    fn co_posonlyargcount(self) -> usize {
        self.code.posonlyarg_count
    }

    #[pygetset]
    fn co_argcount(self) -> usize {
        self.code.arg_count
    }

    #[pygetset]
    pub fn co_filename(self) -> PyStrRef {
        self.code.source_path.to_owned()
    }

    #[pygetset]
    fn co_firstlineno(self) -> usize {
        self.code.first_line_number
    }

    #[pygetset]
    fn co_kwonlyargcount(self) -> usize {
        self.code.kwonlyarg_count
    }

    #[pygetset]
    fn co_consts(self, vm: &VirtualMachine) -> PyTupleRef {
        let consts = self.code.constants.iter().map(|x| x.0.clone()).collect();
        vm.ctx.new_tuple(consts)
    }

    #[pygetset]
    fn co_name(self) -> PyStrRef {
        self.code.obj_name.to_owned()
    }

    #[pygetset]
    fn co_flags(self) -> u16 {
        self.code.flags.bits()
    }

    #[pygetset]
    pub fn co_varnames(self, vm: &VirtualMachine) -> PyTupleRef {
        let varnames = self.code.varnames.iter().map(|s| s.to_object()).collect();
        vm.ctx.new_tuple(varnames)
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

pub fn init(ctx: &Context) {
    PyRef::<PyCode>::extend_class(ctx, ctx.types.code_type);
}
