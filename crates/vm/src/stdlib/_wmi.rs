// spell-checker:disable
#![allow(non_snake_case)]

pub(crate) use _wmi::module_def;

#[pymodule]
mod _wmi {
    use crate::builtins::PyStrRef;
    use crate::convert::ToPyException;
    use crate::{PyResult, VirtualMachine};
    use rustpython_host_env::wmi as host_wmi;

    const BUFFER_SIZE: usize = 8192;

    #[pyfunction]
    fn exec_query(query: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
        let query_str = query.expect_str();

        if !query_str
            .get(..7)
            .is_some_and(|s| s.eq_ignore_ascii_case("select "))
        {
            return Err(vm.new_value_error("only SELECT queries are supported"));
        }

        match host_wmi::exec_query(query_str) {
            Ok(result) => Ok(result),
            Err(host_wmi::ExecQueryError::MoreData) => Err(vm.new_os_error(format!(
                "Query returns more than {} characters",
                BUFFER_SIZE,
            ))),
            Err(host_wmi::ExecQueryError::Code(err)) => {
                Err(std::io::Error::from_raw_os_error(err as i32).to_pyexception(vm))
            }
        }
    }
}
