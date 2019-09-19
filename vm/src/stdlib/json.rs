use crate::obj::objbytearray::PyByteArray;
use crate::obj::objbytes::PyBytes;
use crate::obj::objstr::PyString;
use crate::py_serde;
use crate::pyobject::{ItemProtocol, PyObjectRef, PyResult, TypeProtocol};
use crate::types::create_type;
use crate::VirtualMachine;
use serde_json;

/// Implement json.dumps
pub fn json_dumps(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
    let serializer = py_serde::PyObjectSerializer::new(vm, &obj);
    serde_json::to_string(&serializer).map_err(|err| vm.new_type_error(err.to_string()))
}

pub fn json_dump(obj: PyObjectRef, fs: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let result = json_dumps(obj, vm)?;
    vm.call_method(&fs, "write", vec![vm.new_str(result)])?;
    Ok(vm.get_none())
}

/// Implement json.loads
pub fn json_loads(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let de_result = match_class!(match obj {
        s @ PyString => {
            py_serde::deserialize(vm, &mut serde_json::Deserializer::from_str(s.as_str()))
        }
        b @ PyBytes => py_serde::deserialize(vm, &mut serde_json::Deserializer::from_slice(&b)),
        ba @ PyByteArray => py_serde::deserialize(
            vm,
            &mut serde_json::Deserializer::from_slice(&ba.inner.borrow().elements)
        ),
        obj => {
            let msg = format!(
                "the JSON object must be str, bytes or bytearray, not {}",
                obj.class().name
            );
            return Err(vm.new_type_error(msg));
        }
    });
    de_result.map_err(|err| {
        let module = vm
            .get_attribute(vm.sys_module.clone(), "modules")
            .unwrap()
            .get_item("json", vm)
            .unwrap();
        let json_decode_error = vm.get_attribute(module, "JSONDecodeError").unwrap();
        let json_decode_error = json_decode_error.downcast().unwrap();
        let exc = vm.new_exception(json_decode_error, format!("{}", err));
        vm.set_attr(&exc, "lineno", vm.ctx.new_int(err.line()))
            .unwrap();
        vm.set_attr(&exc, "colno", vm.ctx.new_int(err.column()))
            .unwrap();
        exc
    })
}

pub fn json_load(fp: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let result = vm.call_method(&fp, "read", vec![])?;
    json_loads(result, vm)
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    // TODO: Make this a proper type with a constructor
    let json_decode_error = create_type(
        "JSONDecodeError",
        &ctx.types.type_type,
        &ctx.exceptions.exception_type,
    );

    py_module!(vm, "json", {
        "dumps" => ctx.new_rustfunc(json_dumps),
        "dump" => ctx.new_rustfunc(json_dump),
        "loads" => ctx.new_rustfunc(json_loads),
        "load" => ctx.new_rustfunc(json_load),
        "JSONDecodeError" => json_decode_error
    })
}
