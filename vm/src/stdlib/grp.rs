pub(crate) use grp::make_module;

#[pymodule]
mod grp {
    use std::ptr::NonNull;

    use crate::{
        builtins::{PyIntRef, PyListRef, PyStrRef},
        convert::{IntoPyException, ToPyObject},
        exceptions,
        types::PyStructSequence,
        PyObjectRef, PyResult, VirtualMachine,
    };
    use nix::unistd;

    #[pyattr]
    #[pyclass(module = "grp", name = "struct_group")]
    #[derive(PyStructSequence)]
    struct Group {
        gr_name: String,
        gr_passwd: String,
        gr_gid: u32,
        gr_mem: PyListRef,
    }
    #[pyimpl(with(PyStructSequence))]
    impl Group {
        fn from_unistd_group(group: unistd::Group, vm: &VirtualMachine) -> Self {
            let cstr_lossy = |s: std::ffi::CString| {
                s.into_string()
                    .unwrap_or_else(|e| e.into_cstring().to_string_lossy().into_owned())
            };
            Group {
                gr_name: group.name,
                gr_passwd: cstr_lossy(group.passwd),
                gr_gid: group.gid.as_raw(),
                gr_mem: vm
                    .ctx
                    .new_list(group.mem.iter().map(|s| s.to_pyobject(vm)).collect()),
            }
        }
    }

    #[pyfunction]
    fn getgrgid(gid: PyIntRef, vm: &VirtualMachine) -> PyResult<Group> {
        let gr_gid = gid.as_bigint();
        let gid = libc::gid_t::try_from(gr_gid)
            .map(unistd::Gid::from_raw)
            .ok();
        let group = gid
            .map(|gid| unistd::Group::from_gid(gid))
            .transpose()
            .map_err(|err| err.into_pyexception(vm))?
            .flatten();
        let group = group.ok_or_else(|| {
            vm.new_key_error(
                vm.ctx
                    .new_str(format!("getgrgid: group id {} not found", gr_gid))
                    .into(),
            )
        })?;
        Ok(Group::from_unistd_group(group, vm))
    }

    #[pyfunction]
    fn getgrnam(name: PyStrRef, vm: &VirtualMachine) -> PyResult<Group> {
        let gr_name = name.as_str();
        if gr_name.contains('\0') {
            return Err(exceptions::cstring_error(vm));
        }
        let group = unistd::Group::from_name(gr_name).map_err(|err| err.into_pyexception(vm))?;
        let group = group.ok_or_else(|| {
            vm.new_key_error(
                vm.ctx
                    .new_str(format!("getgrnam: group name {} not found", gr_name))
                    .into(),
            )
        })?;
        Ok(Group::from_unistd_group(group, vm))
    }

    #[pyfunction]
    fn getgrall(vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        // setgrent, getgrent, etc are not thread safe. Could use fgetgrent_r, but this is easier
        static GETGRALL: parking_lot::Mutex<()> = parking_lot::const_mutex(());
        let _guard = GETGRALL.lock();
        let mut list = Vec::new();

        unsafe { libc::setgrent() };
        while let Some(ptr) = NonNull::new(unsafe { libc::getgrent() }) {
            let group = unistd::Group::from(unsafe { ptr.as_ref() });
            let group = Group::from_unistd_group(group, vm).to_pyobject(vm);
            list.push(group);
        }
        unsafe { libc::endgrent() };

        Ok(list)
    }
}
