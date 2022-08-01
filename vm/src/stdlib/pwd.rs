pub(crate) use pwd::make_module;

#[pymodule]
mod pwd {
    use crate::{
        builtins::{PyIntRef, PyStrRef},
        convert::{IntoPyException, ToPyObject},
        exceptions,
        types::PyStructSequence,
        AsObject, PyObjectRef, PyResult, VirtualMachine,
    };
    use nix::unistd::{self, User};
    use std::ptr::NonNull;

    #[pyattr]
    #[pyclass(module = "pwd", name = "struct_passwd")]
    #[derive(PyStructSequence)]
    struct Passwd {
        pw_name: String,
        pw_passwd: String,
        pw_uid: u32,
        pw_gid: u32,
        pw_gecos: String,
        pw_dir: String,
        pw_shell: String,
    }
    #[pyimpl(with(PyStructSequence))]
    impl Passwd {}

    impl From<User> for Passwd {
        fn from(user: User) -> Self {
            // this is just a pain...
            let cstr_lossy = |s: std::ffi::CString| {
                s.into_string()
                    .unwrap_or_else(|e| e.into_cstring().to_string_lossy().into_owned())
            };
            let pathbuf_lossy = |p: std::path::PathBuf| {
                p.into_os_string()
                    .into_string()
                    .unwrap_or_else(|s| s.to_string_lossy().into_owned())
            };
            Passwd {
                pw_name: user.name,
                pw_passwd: cstr_lossy(user.passwd),
                pw_uid: user.uid.as_raw(),
                pw_gid: user.gid.as_raw(),
                pw_gecos: cstr_lossy(user.gecos),
                pw_dir: pathbuf_lossy(user.dir),
                pw_shell: pathbuf_lossy(user.shell),
            }
        }
    }

    #[pyfunction]
    fn getpwnam(name: PyStrRef, vm: &VirtualMachine) -> PyResult<Passwd> {
        if name.as_str().contains('\0') {
            return Err(exceptions::cstring_error(vm));
        }
        let user = User::from_name(name.as_str()).map_err(|err| err.into_pyexception(vm))?;
        let user = user.ok_or_else(|| {
            let name_repr = name.as_object().repr(vm)?;
            vm.new_key_error(
                vm.ctx
                    .new_str(format!("getpwnam(): name not found: {}", name_repr))
                    .into(),
            )
        })?;
        Ok(Passwd::from(user))
    }

    #[pyfunction]
    fn getpwuid(uid: PyIntRef, vm: &VirtualMachine) -> PyResult<Passwd> {
        let uid_t = libc::uid_t::try_from(uid.as_bigint())
            .map(unistd::Uid::from_raw)
            .ok();
        let user = uid_t
            .map(User::from_uid)
            .transpose()
            .map_err(|err| err.into_pyexception(vm))?
            .flatten();
        let user = user.ok_or_else(|| {
            vm.new_key_error(
                vm.ctx
                    .new_str(format!("getpwuid(): uid not found: {}", uid.as_bigint()))
                    .into(),
            )
        })?;
        Ok(Passwd::from(user))
    }

    // TODO: maybe merge this functionality into nix?
    #[pyfunction]
    fn getpwall(vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        // setpwent, getpwent, etc are not thread safe. Could use fgetpwent_r, but this is easier
        static GETPWALL: parking_lot::Mutex<()> = parking_lot::const_mutex(());
        let _guard = GETPWALL.lock();
        let mut list = Vec::new();

        unsafe { libc::setpwent() };
        while let Some(ptr) = NonNull::new(unsafe { libc::getpwent() }) {
            let user = User::from(unsafe { ptr.as_ref() });
            let passwd = Passwd::from(user).to_pyobject(vm);
            list.push(passwd);
        }
        unsafe { libc::endpwent() };

        Ok(list)
    }
}
