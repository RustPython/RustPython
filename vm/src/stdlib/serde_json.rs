pub(crate) use _serde_json::make_module;

#[pymodule]
mod _serde_json {
    use crate::common::borrow::BorrowValue;
    use crate::exceptions::PyBaseExceptionRef;
    use crate::obj::objstr::PyStrRef;
    use crate::py_serde;
    use crate::pyobject::{PyResult, TryFromObject};
    use crate::VirtualMachine;

    #[pyfunction]
    fn decode(s: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let res = (|| -> serde_json::Result<_> {
            let mut de = serde_json::Deserializer::from_str(s.borrow_value());
            let res = py_serde::deserialize(vm, &mut de)?;
            de.end()?;
            Ok(res)
        })();

        res.map_err(|err| match json_exception(err, s, vm) {
            Ok(x) | Err(x) => x,
        })
    }

    fn json_exception(
        err: serde_json::Error,
        s: PyStrRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyBaseExceptionRef> {
        let decode_error = vm.try_class("json", "JSONDecodeError")?;
        let from_serde = vm.get_attribute(decode_error.into_object(), "_from_serde")?;
        let mut err_msg = err.to_string();
        let pos = err_msg.rfind(" at line ").unwrap();
        err_msg.truncate(pos);
        let decode_error = vm.invoke(
            &from_serde,
            vec![
                vm.ctx.new_str(err_msg),
                s.into_object(),
                vm.ctx.new_int(err.line()),
                vm.ctx.new_int(err.column()),
            ],
        )?;
        PyBaseExceptionRef::try_from_object(vm, decode_error)
    }
}
