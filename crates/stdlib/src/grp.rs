// spell-checker:disable
pub(crate) use grp::module_def;

#[pymodule]
mod grp {
    use crate::vm::{
        AsObject, PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyListRef, PyUtf8StrRef},
        convert::ToPyObject,
        exceptions,
        types::PyStructSequence,
    };
    use core::hint::cold_path;
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
            Self {
                gr_name: group.name,
                gr_passwd: group.passwd,
                gr_gid: group.gid,
                gr_mem: vm
                    .ctx
                    .new_list(group.mem.iter().map(|s| s.to_pyobject(vm)).collect()),
            }
        }
    }

    #[derive(FromArgs)]
    struct GetGrGidArgs {
        #[pyarg(any)]
        id: PyObjectRef,
    }

    #[pyfunction]
    fn getgrgid(id: GetGrGidArgs, vm: &VirtualMachine) -> PyResult<GroupData> {
        let index = id.id.try_index(vm).map_err(|_| {
            let name = id.id.class().slot_name().to_string();
            let name = &name[..name.floor_char_boundary(200.min(name.len()))];
            vm.new_type_error(format!("gid should be integer, not {name}"))
        })?;
        let value = index.as_bigint();
        // Match CPython's _Py_Gid_Converter, including the -1 sentinel and
        // the different overflow paths before and after signed-long conversion.
        let gid = if let Ok(value) = libc::c_long::try_from(value) {
            if value == -1 {
                -1_i32 as host_grp::gid_t
            } else {
                if value < 0 {
                    return Err(vm.new_overflow_error("gid is less than minimum"));
                }
                host_grp::gid_t::try_from(value)
                    .map_err(|_| vm.new_overflow_error("gid is less than minimum"))?
            }
        } else if value < &0.into() {
            return Err(vm.new_overflow_error("gid is less than minimum"));
        } else {
            libc::c_ulong::try_from(value)
                .ok()
                .and_then(|value| host_grp::gid_t::try_from(value).ok())
                .filter(|&gid| gid != -1_i32 as host_grp::gid_t)
                .ok_or_else(|| vm.new_overflow_error("gid is greater than maximum"))?
        };
        let group = host_grp::getgrgid(gid).ok().flatten().ok_or_else(|| {
            vm.new_key_error(
                vm.ctx
                    .new_str(format!("getgrgid(): gid not found: {gid}"))
                    .into(),
            )
        })?;
        Ok(GroupData::from_group(group, vm))
    }

    #[derive(FromArgs)]
    struct GetGrNamArgs {
        #[pyarg(any)]
        name: PyUtf8StrRef,
    }

    #[pyfunction]
    fn getgrnam(name: GetGrNamArgs, vm: &VirtualMachine) -> PyResult<GroupData> {
        let name = name.name;
        if name.as_pystr().contains_nuls() {
            cold_path();
            return Err(exceptions::nul_char_error(vm));
        }
        let gr_name = name.as_str();
        let group = host_grp::getgrnam(gr_name).ok().flatten().ok_or_else(|| {
            vm.new_key_error(
                vm.ctx
                    .new_str(format!("getgrnam: group name {gr_name} not found"))
                    .into(),
            )
        })?;
        Ok(GroupData::from_group(group, vm))
    }

    #[pyfunction]
    fn getgrall(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        host_grp::getgrall()
            .into_iter()
            .map(|group| GroupData::from_group(group, vm).to_pyobject(vm))
            .collect()
    }
}
