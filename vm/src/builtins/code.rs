/*! Infamous code object. The python class `code`

*/

use super::{PyStrRef, PyTupleRef, PyTypeRef};
use crate::{
    bytecode::{self, BorrowedConstant, Constant, ConstantBag},
    function::FuncArgs,
    IdProtocol, PyClassImpl, PyContext, PyObject, PyObjectRef, PyRef, PyResult, PyValue,
    StaticType, TypeProtocol, VirtualMachine,
};
use num_traits::Zero;
use std::fmt;
use std::ops::Deref;

#[derive(Clone)]
pub struct PyConstant(pub PyObjectRef);
// pub(crate) enum PyConstant {
//     Integer { value: super::int::PyIntRef },
//     Float { value: super::int::PyFloatRef },
//     Complex { value: super::complex::PyComplexRef },
//     Boolean { value: super::int::PyIntRef },
//     Str { value: super::pystr::PyStrRef },
//     Bytes { value: super::bytes::PyBytesRef },
//     Code { code: PyRef<PyCode> },
//     Tuple { elements: super::tuple::PyTupleRef },
//     None(PyObjectRef),
//     Ellipsis(PyObjectRef),
// }

fn borrow_obj_constant(obj: &PyObject) -> BorrowedConstant<PyConstant> {
    match_class!(match obj {
        ref i @ super::int::PyInt => {
            let value = i.as_bigint();
            if obj.class().is(super::pybool::PyBool::static_type()) {
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
                elements: Box::new(t.as_slice().iter().map(|o| borrow_obj_constant(o))),
            }
        }
        super::singletons::PyNone => BorrowedConstant::None,
        super::slice::PyEllipsis => BorrowedConstant::Ellipsis,
        _ => panic!("unexpected payload for constant python value"),
    })
}

impl Constant for PyConstant {
    type Name = PyStrRef;
    fn borrow_constant(&self) -> BorrowedConstant<Self> {
        borrow_obj_constant(&self.0)
    }
    fn map_constant<Bag: ConstantBag>(self, bag: &Bag) -> Bag::Constant {
        bag.make_constant_borrowed(self.borrow_constant())
    }
}

pub(crate) struct PyObjBag<'a>(pub &'a VirtualMachine);

impl ConstantBag for PyObjBag<'_> {
    type Constant = PyConstant;
    fn make_constant(&self, constant: bytecode::ConstantData) -> Self::Constant {
        let vm = self.0;
        let ctx = &vm.ctx;
        let obj = match constant {
            bytecode::ConstantData::Integer { value } => ctx.new_int(value).into(),
            bytecode::ConstantData::Float { value } => ctx.new_float(value).into(),
            bytecode::ConstantData::Complex { value } => vm.new_pyobj(value),
            bytecode::ConstantData::Str { value } if value.len() <= 20 => {
                vm.intern_string(value).into()
            }
            bytecode::ConstantData::Str { value } => vm.ctx.new_str(value).into(),
            bytecode::ConstantData::Bytes { value } => ctx.new_bytes(value.to_vec()).into(),
            bytecode::ConstantData::Boolean { value } => ctx.new_bool(value).into(),
            bytecode::ConstantData::Code { code } => vm.new_code_object(code.map_bag(self)).into(),
            bytecode::ConstantData::Tuple { elements } => {
                let elements = elements
                    .into_iter()
                    .map(|constant| self.make_constant(constant).0)
                    .collect();
                ctx.new_tuple(elements).into()
            }
            bytecode::ConstantData::None => ctx.none(),
            bytecode::ConstantData::Ellipsis => ctx.ellipsis(),
        };
        PyConstant(obj)
    }
    fn make_constant_borrowed<C: Constant>(&self, constant: BorrowedConstant<C>) -> Self::Constant {
        let vm = self.0;
        let ctx = &vm.ctx;
        let obj = match constant {
            bytecode::BorrowedConstant::Integer { value } => ctx.new_bigint(value).into(),
            bytecode::BorrowedConstant::Float { value } => ctx.new_float(value).into(),
            bytecode::BorrowedConstant::Complex { value } => vm.new_pyobj(value),
            bytecode::BorrowedConstant::Str { value } if value.len() <= 20 => {
                vm.intern_string(value).into()
            }
            bytecode::BorrowedConstant::Str { value } => vm.ctx.new_str(value).into(),
            bytecode::BorrowedConstant::Bytes { value } => ctx.new_bytes(value.to_vec()).into(),
            bytecode::BorrowedConstant::Boolean { value } => ctx.new_bool(value).into(),
            bytecode::BorrowedConstant::Code { code } => {
                vm.new_code_object(code.map_clone_bag(self)).into()
            }
            bytecode::BorrowedConstant::Tuple { elements } => {
                let elements = elements
                    .into_iter()
                    .map(|constant| self.make_constant_borrowed(constant).0)
                    .collect();
                ctx.new_tuple(elements).into()
            }
            bytecode::BorrowedConstant::None => ctx.none(),
            bytecode::BorrowedConstant::Ellipsis => ctx.ellipsis(),
        };
        PyConstant(obj)
    }
    fn make_name(&self, name: String) -> PyStrRef {
        self.0.intern_string(name)
    }
    fn make_name_ref(&self, name: &str) -> PyStrRef {
        self.0.intern_string(name)
    }
}

pub type CodeObject = bytecode::CodeObject<PyConstant>;
pub type FrozenModule = bytecode::FrozenModule<PyConstant>;

pub trait IntoCodeObject {
    fn into_codeobj(self, vm: &VirtualMachine) -> CodeObject;
}
impl IntoCodeObject for CodeObject {
    fn into_codeobj(self, _vm: &VirtualMachine) -> CodeObject {
        self
    }
}
impl IntoCodeObject for bytecode::CodeObject {
    fn into_codeobj(self, vm: &VirtualMachine) -> CodeObject {
        vm.map_codeobj(self)
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

    /// Create a new `PyRef<PyCode>` from a `code::CodeObject`. If you have a non-mapped codeobject or
    /// this is giving you a type error even though you've passed a `CodeObject`, try
    /// [`vm.new_code_object()`](VirtualMachine::new_code_object) instead.
    pub fn new_ref(code: CodeObject, ctx: &PyContext) -> PyRef<Self> {
        PyRef::new_ref(PyCode { code }, ctx.types.code_type.clone(), None)
    }
}

impl fmt::Debug for PyCode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "code: {:?}", self.code)
    }
}

impl PyValue for PyCode {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.code_type
    }
}

#[pyimpl(with(PyRef))]
impl PyCode {}

#[pyimpl]
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

    #[pyproperty]
    fn co_posonlyargcount(self) -> usize {
        self.code.posonlyarg_count
    }

    #[pyproperty]
    fn co_argcount(self) -> usize {
        self.code.arg_count
    }

    #[pyproperty]
    fn co_filename(self) -> PyStrRef {
        self.code.source_path.clone()
    }

    #[pyproperty]
    fn co_firstlineno(self) -> usize {
        self.code.first_line_number
    }

    #[pyproperty]
    fn co_kwonlyargcount(self) -> usize {
        self.code.kwonlyarg_count
    }

    #[pyproperty]
    fn co_consts(self, vm: &VirtualMachine) -> PyTupleRef {
        let consts = self.code.constants.iter().map(|x| x.0.clone()).collect();
        vm.ctx.new_tuple(consts)
    }

    #[pyproperty]
    fn co_name(self) -> PyStrRef {
        self.code.obj_name.clone()
    }

    #[pyproperty]
    fn co_flags(self) -> u16 {
        self.code.flags.bits()
    }

    #[pyproperty]
    pub fn co_varnames(self, vm: &VirtualMachine) -> PyTupleRef {
        let varnames = self
            .code
            .varnames
            .iter()
            .map(|s| s.clone().into())
            .collect();
        vm.ctx.new_tuple(varnames)
    }
}

impl fmt::Display for PyCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

pub fn init(ctx: &PyContext) {
    PyRef::<PyCode>::extend_class(ctx, &ctx.types.code_type);
}
