use crate::{PyObjectRef, VirtualMachine};

mod array;
mod basics;
mod dll;
mod function;
mod pointer;
mod primitive;
mod shared_lib;
mod structure;
mod union;

use array::PyCArrayMeta;
use basics::PyCData;
use primitive::PySimpleMeta;
use crate::class::PyClassImpl;
use crate::convert::IntoObject;

#[pymodule]
mod _ctypes {
    use rustpython_vm::stdlib::ctypes::basics;
    use crate::function::Either;
    use crate::builtins::PyTypeRef;
    use crate::{PyObjectRef, PyResult, VirtualMachine};
    use crate::stdlib::ctypes::pointer;

    #[pyattr(name="__version__")]
    pub(crate) fn version(_vm: &VirtualMachine) -> &'static str {
        "1.1.0"
    }

    #[pyfunction]
    pub(crate) fn alignment(tp: Either<PyTypeRef, PyObjectRef>, vm: &VirtualMachine) -> PyResult<usize> {
        basics::alignment(tp, vm)
    }

    #[pyfunction]
    pub(crate) fn sizeof(tp: Either<PyTypeRef, PyObjectRef>, vm: &VirtualMachine) -> PyResult<usize> {
        basics::sizeof_func(tp, vm)
    }

    #[pyfunction]
    pub(crate) fn byref(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        basics::byref(obj, vm)
    }

    #[pyfunction]
    pub(crate) fn addressof(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        basics::addressof(obj, vm)
    }

    #[pyfunction]
    pub(crate) fn POINTER(tp: PyTypeRef, vm: &VirtualMachine) -> PyResult {
        pointer::POINTER(tp);
        Ok(vm.ctx.none())
    }

    #[pyfunction]
    pub(crate) fn pointer(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        pointer::pointer_fn(obj);
        Ok(vm.ctx.none())
    }

    #[pyfunction]
    pub(crate) fn _pointer_type_cache(vm: &VirtualMachine) -> PyResult {
        Ok(PyObjectRef::from(vm.ctx.new_dict()))
    }

    // TODO: add the classes
}

pub(crate) fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;
    PyCData::make_class(ctx);
    PySimpleMeta::make_class(ctx);
    PyCArrayMeta::make_class(ctx);

    let module = _ctypes::make_module(vm);
    // let module = py_module!(vm, "_ctypes", {
    //     "CFuncPtr" => PyCFuncPtr::make_class(ctx),
    //     "_SimpleCData" => PyCSimple::make_class(ctx),
    //     "_Pointer" => PyCPointer::make_class(ctx),
    //     "Array" => PyCArray::make_class(ctx),
    //     "Struct" => PyCStructure::make_class(ctx)
    // });

    dll::extend_module(vm, &module).unwrap();

    module.into_object()
}
