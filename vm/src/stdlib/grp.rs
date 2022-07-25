pub(crate) use grp::make_module;

#[pymodule]
mod grp {
    use std::ptr::NonNull;

    use crate::{
        builtins::{PyIntRef, PyStrRef},
        PyResult, VirtualMachine, PyObjectRef, convert::{IntoPyException, ToPyObject}, AsObject,
    };
    use nix::unistd;

    #[pyattr]
    #[pyclass(module = "grp", name = "struct_group")]
    #[derive(PyStructSequence)]
    struct Group {
        gr_name: String,
        gr_passwd: String,
        gr_gid: u32,
        gr_mem: Vec<String>,
    }
    #[pyimpl(with(PyStructSequence))]
    impl Group {}

    impl From<unistd::Group> for Group {
        fn from(group: unistd::Group) -> Self {
            // this is just a pain...
            let cstr_lossy = |s: std::ffi::CString| {
                s.into_string()
                    .unwrap_or_else(|e| e.into_cstring().to_string_lossy().into_owned())
            };
            Group {
                gr_name: group.name,
                gr_passwd: cstr_lossy(group.passwd),
                gr_gid: group.gid.as_raw(),
                gr_mem: group.mem,
            }
        }
    }

    #[pyfunction]
    fn getgrgid(gid: PyIntRef, vm: &VirtualMachine) -> PyResult<Group> {
        let gid_t = libc::gid_t::try_from(gid.as_bigint()).map(unistd::Gid::from_raw);
        let group = match gid_t {
            Ok(gid) => unistd::Group::from_gid(gid).map_err(|err| err.into_pyexception(vm))?,
            Err(_) => None,
        };
        match group {
            Some(group) => Ok(Group::from(group)),
            None => {
                let message = vm
                    .ctx
                    .new_str(format!("getgrgid: group id {} not found", gid.as_bigint()))
                    .into();
                Err(vm.new_key_error(message))
            }
        }
    }

    #[pyfunction]
    fn getgrnam(name: PyStrRef, vm: &VirtualMachine) -> PyResult<Group> {
        match unistd::Group::from_name(name.as_str()).map_err(|err| err.into_pyexception(vm))? {
            Some(group) => Ok(Group::from(group)),
            None => {
                let name_repr = name.as_object().repr(vm)?;
                let message = vm
                    .ctx
                    .new_str(format!("getgrnam(): name not found: {}", name_repr))
                    .into();
                Err(vm.new_key_error(message))
            }
        }
    }

    #[pyfunction]
    fn getgrall(vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        // setpwent, getpwent, etc are not thread safe. Could use fgetpwent_r, but this is easier
        static GETGRALL: parking_lot::Mutex<()> = parking_lot::const_mutex(());
        let _guard = GETGRALL.lock();
        let mut list = Vec::new();

        unsafe { libc::setpwent() };
        while let Some(ptr) = NonNull::new(unsafe { libc::getgrent() }) {
            let group = unistd::Group::from(unsafe { ptr.as_ref() });
            let group = Group::from(group).to_pyobject(vm);
            list.push(group);
        }
        unsafe { libc::endpwent() };

        Ok(list)
    }
}
