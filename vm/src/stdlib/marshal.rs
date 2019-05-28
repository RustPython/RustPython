use crate::bytecode;
use crate::obj::objbytes::{PyBytes, PyBytesRef};
use crate::obj::objcode::{PyCode, PyCodeRef};
use crate::pyobject::{IntoPyObject, PyObjectRef, PyResult};
use crate::vm::VirtualMachine;

fn marshal_dumps(co: PyCodeRef, vm: &VirtualMachine) -> PyResult {
    PyBytes::new(bincode::serialize(&co.code).unwrap()).into_pyobject(vm)
}

fn marshal_loads(code_bytes: PyBytesRef, vm: &VirtualMachine) -> PyResult {
    let code = bincode::deserialize::<bytecode::CodeObject>(&code_bytes).unwrap();
    let pycode = PyCode { code };
    pycode.into_pyobject(vm)
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    py_module!(vm, "marshal", {
        "loads" => ctx.new_rustfunc(marshal_loads),
        "dumps" => ctx.new_rustfunc(marshal_dumps)
    })
}
