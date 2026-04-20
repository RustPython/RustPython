// spell-checker:disable
pub(crate) use grp::module_def;

#[pymodule]
mod grp {
    use crate::vm::{
        PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyIntRef, PyListRef, PyUtf8StrRef},
        convert::{IntoPyException, ToPyObject},
        exceptions,
        types::PyStructSequence,
    };
    use rustpython_host_env::grp as host_grp;

    #[pystruct_sequence_data]
    struct GroupData {
        gr_name: String,
        gr_passwd: String,
        gr_gid: u32,
        gr_mem: PyListRef,
    }

    #[pyattr]
    #[pystruct_sequence(name = "struct_group", module = "grp", data = "GroupData")]
    struct PyGroup;

    #[pyclass(with(PyStructSequence))]
    impl PyGroup {}

    impl GroupData {
        fn from_group(group: host_grp::Group, vm: &VirtualMachine) -> Self {
            GroupData {
                gr_name: group.name,
                gr_passwd: group.passwd,
                gr_gid: group.gid,
                gr_mem: vm
                    .ctx
                    .new_list(group.mem.iter().map(|s| s.to_pyobject(vm)).collect()),
            }
        }
    }

    #[pyfunction]
    fn getgrgid(gid: PyIntRef, vm: &VirtualMachine) -> PyResult<GroupData> {
        let gr_gid = gid.as_bigint();
        let gid = libc::gid_t::try_from(gr_gid).ok();
        let group = gid
            .map(host_grp::getgrgid)
            .transpose()
            .map_err(|err| err.into_pyexception(vm))?
            .flatten();
        let group = group.ok_or_else(|| {
            vm.new_key_error(
                vm.ctx
                    .new_str(format!("getgrgid: group id {gr_gid} not found"))
                    .into(),
            )
        })?;
        Ok(GroupData::from_group(group, vm))
    }

    #[pyfunction]
    fn getgrnam(name: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<GroupData> {
        let gr_name = name.as_str();
        if gr_name.contains('\0') {
            return Err(exceptions::cstring_error(vm));
        }
        let group = host_grp::getgrnam(gr_name).map_err(|err| err.into_pyexception(vm))?;
        let group = group.ok_or_else(|| {
            vm.new_key_error(
                vm.ctx
                    .new_str(format!("getgrnam: group name {gr_name} not found"))
                    .into(),
            )
        })?;
        Ok(GroupData::from_group(group, vm))
    }

    #[pyfunction]
    fn getgrall(vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        Ok(host_grp::getgrall()
            .into_iter()
            .map(|group| GroupData::from_group(group, vm).to_pyobject(vm))
            .collect())
    }
}
