use crate::pyobject::{PyClassImpl, PyObjectRef, PyValue};
use crate::VirtualMachine;

mod array;
mod basics;
mod dll;
mod function;
mod pointer;
mod primitive;
mod shared_lib;
mod structure;

use array::PyCArray;
use basics::{addressof, alignment, byref, sizeof_func, PyCData};
use dll::*;
use function::PyCFuncPtr;
use pointer::{pointer_fn, PyCPointer, POINTER};
use primitive::PySimpleType;
use structure::PyCStructure;

pub(crate) fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;
    PyCData::make_class(ctx);

    py_module!(vm, "_ctypes", {
        "__version__" => ctx.new_str("1.1.0"),

        "dlopen" => ctx.new_function(dlopen),
        "dlsym" => ctx.new_function(dlsym),
        "dlclose" => ctx.new_function(dlclose),

        "alignment" => ctx.new_function(alignment),
        "sizeof" => ctx.new_function(sizeof_func),
        "byref" => ctx.new_function(byref),
        "addressof" => ctx.new_function(addressof),

        "POINTER" => ctx.new_function(POINTER),
        "pointer" => ctx.new_function(pointer_fn),
        "_pointer_type_cache" => ctx.new_dict(),

        "CFuncPtr" => PyCFuncPtr::make_class(ctx),
        "_SimpleCData" => PySimpleType::make_class(ctx),
        "_Pointer" => PyCPointer::make_class(ctx),
        "Array" => PyCArray::make_class(ctx),
        "Struct" => PyCStructure::make_class(ctx)
    })
}
