// spell-checker:disable
#![allow(non_snake_case)]

pub(crate) use _wmi::module_def;

#[pymodule]
mod _wmi {
    use crate::builtins::PyStrRef;
    use crate::convert::ToPyException;
    use crate::{PyResult, VirtualMachine};
    use rustpython_host_env::wmi as host_wmi;

    #[derive(FromArgs)]
    struct ExecQueryArgs {
        #[pyarg(any)]
        query: PyStrRef,
    }

    #[pyfunction]
    fn exec_query(args: ExecQueryArgs, vm: &VirtualMachine) -> PyResult<String> {
        // Bind all keywords before converting the query, as Argument Clinic does.
        let query = args.query;
        vm.audit("_wmi.exec_query", || (query.clone(),))?;
        let mut code_units = query.as_wtf8().encode_wide().collect::<Vec<_>>();
        // Adding the terminator first makes even a trailing input NUL interior.
        code_units.push(0);
        let wide = widestring::WideCString::from_vec(code_units)
            .map_err(|_| vm.new_value_error("embedded null character"))?;
        if !query
            .as_wtf8()
            .as_bytes()
            .get(..7)
            .is_some_and(|s| s.eq_ignore_ascii_case(b"select "))
        {
            return Err(vm.new_value_error("only SELECT queries are supported"));
        }
        host_wmi::exec_query_wide(wide).map_err(|err| err.to_pyexception(vm))
    }
}
