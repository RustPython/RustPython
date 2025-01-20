use crate::{PyObjectRef, VirtualMachine};

mod array;
mod basics;
mod dll;
mod function;
mod pointer;
mod primitive;
mod shared_lib;
mod structure;
mod structure;
mod array;
mod basics;
mod dll;
mod function;
mod pointer;
mod primitive;
mod shared_lib;
mod union;

use array::{PyCArray, PyCArrayMeta};
use basics::{addressof, alignment, byref, sizeof_func, PyCData};
use function::PyCFuncPtr;
use pointer::{pointer_fn, PyCPointer, POINTER};
use primitive::{PyCSimple, PySimpleMeta};
use structure::PyCStructure;

pub(crate) fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;
    PyCData::make_class(ctx);
    PySimpleMeta::make_class(ctx);
    PyCArrayMeta::make_class(ctx);

    let module = py_module!(vm, "_ctypes", {
        "__version__" => ctx.new_str("1.1.0"),

        "alignment" => ctx.new_function("alignment", alignment),
        "sizeof" => ctx.new_function("sizeof", sizeof_func),
        "byref" => ctx.new_function("byref", byref),
        "addressof" => ctx.new_function("addressof", addressof),

        "POINTER" => ctx.new_function("POINTER", POINTER),
        "pointer" => ctx.new_function("pointer", pointer_fn),
        "_pointer_type_cache" => ctx.new_dict(),

        "CFuncPtr" => PyCFuncPtr::make_class(ctx),
        "_SimpleCData" => PyCSimple::make_class(ctx),
        "_Pointer" => PyCPointer::make_class(ctx),
        "Array" => PyCArray::make_class(ctx),
        "Struct" => PyCStructure::make_class(ctx)
    });

    dll::extend_module(vm, &module);

    module
}
