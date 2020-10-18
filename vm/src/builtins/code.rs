/*! Infamous code object. The python class `code`

*/

use std::fmt;
use std::ops::Deref;

use super::pytype::PyTypeRef;
use crate::bytecode::{self, BorrowedConstant, Constant, ConstantBag};
use crate::pyobject::{
    BorrowValue, IdProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue,
    TypeProtocol,
};
use crate::VirtualMachine;
use num_traits::Zero;

#[derive(Clone)]
pub struct PyConstant(pub PyObjectRef);
// pub(crate) enum PyConstant {
//     Integer { value: super::int::PyIntRef },
//     Float { value: super::int::PyFloatRef },
//     Complex { value: super::complex::PyComplexRef },
//     Boolean { value: super::int::PyIntRef },
//     Str { value: super::pystr::PyStrRef },
//     Bytes { value: super::bytes::PyBytesRef },
//     Code { code: PyCodeRef },
//     Tuple { elements: super::tuple::PyTupleRef },
//     None(PyObjectRef),
//     Ellipsis(PyObjectRef),
// }

fn borrow_obj_constant(obj: &PyObjectRef) -> BorrowedConstant<PyConstant> {
    match_class!(match obj {
        ref i @ super::int::PyInt => {
            let value = i.borrow_value();
            // TODO: figure out a better way to tell this
            if obj.class().name == "bool" {
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
        ref s @ super::pystr::PyStr => BorrowedConstant::Str {
            value: s.borrow_value()
        },
        ref b @ super::bytes::PyBytes => BorrowedConstant::Bytes {
            value: b.borrow_value()
        },
        ref c @ PyCode => {
            BorrowedConstant::Code { code: &c.code }
        }
        ref t @ super::tuple::PyTuple => {
            BorrowedConstant::Tuple {
                elements: Box::new(t.borrow_value().iter().map(borrow_obj_constant)),
            }
        }
        super::singletons::PyNone => BorrowedConstant::None,
        super::slice::PyEllipsis => BorrowedConstant::Ellipsis,
        _ => panic!("unexpected payload for constant python value"),
    })
}

impl Constant for PyConstant {
    fn borrow_constant(&self) -> BorrowedConstant<Self> {
        borrow_obj_constant(&self.0)
    }
}

pub(crate) struct PyObjBag<'a>(pub &'a PyContext);

impl ConstantBag for PyObjBag<'_> {
    type Constant = PyConstant;
    fn make_constant(&self, constant: bytecode::ConstantData) -> Self::Constant {
        PyConstant(self.0.unwrap_constant(constant))
    }
    fn make_constant_borrowed<C: Constant>(&self, constant: BorrowedConstant<C>) -> Self::Constant {
        // TODO: check if the constant is a string and try interning it without cloning
        self.make_constant(constant.into_data())
    }
}

pub type PyCodeRef = PyRef<PyCode>;

pub type CodeObject = bytecode::CodeObject<PyConstant>;
pub type FrozenModule = bytecode::FrozenModule<PyConstant>;

pub trait IntoCodeObject {
    fn into_codeobj(self, ctx: &PyContext) -> CodeObject;
}
impl IntoCodeObject for CodeObject {
    fn into_codeobj(self, _ctx: &PyContext) -> CodeObject {
        self
    }
}
impl IntoCodeObject for bytecode::CodeObject {
    fn into_codeobj(self, ctx: &PyContext) -> CodeObject {
        ctx.map_codeobj(self)
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

impl PyValue for PyCode {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.code_type
    }
}

#[pyimpl(with(PyRef))]
impl PyCode {}

#[pyimpl]
impl PyCodeRef {
    #[pyslot]
    fn tp_new(_cls: PyTypeRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        Err(vm.new_type_error("Cannot directly create code object".to_owned()))
    }

    #[pymethod(magic)]
    fn repr(self) -> String {
        let code = &self.code;
        format!(
            "<code object {} at 0x{:x} file {:?}, line {}>",
            code.obj_name,
            self.get_id(),
            code.source_path,
            code.first_line_number
        )
    }

    #[pyproperty]
    fn co_posonlyargcount(self) -> usize {
        self.code.posonlyarg_count
    }

    #[pyproperty]
    fn co_argcount(self) -> usize {
        self.code.arg_names.len()
    }

    #[pyproperty]
    fn co_filename(self) -> String {
        self.code.source_path.clone()
    }

    #[pyproperty]
    fn co_firstlineno(self) -> usize {
        self.code.first_line_number
    }

    #[pyproperty]
    fn co_kwonlyargcount(self) -> usize {
        self.code.kwonlyarg_names.len()
    }

    #[pyproperty]
    fn co_consts(self, vm: &VirtualMachine) -> PyObjectRef {
        let consts = self.code.constants.iter().map(|x| x.0.clone()).collect();
        vm.ctx.new_tuple(consts)
    }

    #[pyproperty]
    fn co_name(self) -> String {
        self.code.obj_name.clone()
    }

    #[pyproperty]
    fn co_flags(self) -> u16 {
        self.code.flags.bits()
    }

    #[pyproperty]
    fn co_varnames(self, vm: &VirtualMachine) -> PyObjectRef {
        let varnames = self.code.varnames().map(|s| vm.ctx.new_str(s)).collect();
        vm.ctx.new_tuple(varnames)
    }
}

pub fn init(ctx: &PyContext) {
    PyCodeRef::extend_class(ctx, &ctx.types.code_type);
}
