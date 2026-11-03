// spell-checker:disable

pub(crate) use pwd::module_def;

#[pymodule]
mod pwd {
    use crate::{
        PyObjectRef, PyResult, VirtualMachine, builtins::PyUtf8StrRef, convert::TryFromObject,
        exceptions, stdlib::posix::module::RawUid, types::PyStructSequence,
    };
    use core::hint::cold_path;
    use rustpython_host_env::pwd as host_pwd;

    #[cfg(not(target_os = "android"))]
    use crate::convert::ToPyObject;

    #[pystruct_sequence_data]
    struct PasswdData {
        pw_name: String,
        pw_passwd: String,
        pw_uid: u32,
        pw_gid: u32,
        pw_gecos: String,
        pw_dir: String,
        pw_shell: String,
    }

    #[pyattr]
    #[pystruct_sequence(name = "struct_passwd", module = "pwd", data = "PasswdData")]
    struct PyPasswd;

    #[pyclass(with(PyStructSequence))]
    impl PyPasswd {}

    impl From<host_pwd::Passwd> for PasswdData {
        fn from(user: host_pwd::Passwd) -> Self {
            Self {
                pw_name: user.name,
                pw_passwd: user.passwd,
                pw_uid: user.uid,
                pw_gid: user.gid,
                pw_gecos: user.gecos,
                pw_dir: user.dir,
                pw_shell: user.shell,
            }
        }
    }

    #[pyfunction]
    fn getpwnam(name: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<PasswdData> {
        if name.as_pystr().contains_nuls() {
            cold_path();
            return Err(exceptions::nul_char_error(vm));
        }
        let name = name.as_str();
        let user = host_pwd::getpwnam(name);
        let user = user.ok_or_else(|| {
            vm.new_key_error(
                vm.ctx
                    .new_str(format!("getpwnam(): name not found: {name}"))
                    .into(),
            )
        })?;
        Ok(PasswdData::from(user))
    }

    #[pyfunction]
    fn getpwuid(uidobj: PyObjectRef, vm: &VirtualMachine) -> PyResult<PasswdData> {
        let uid = RawUid::try_from_object(vm, uidobj)
            .map_err(|err| {
                if err.fast_isinstance(vm.ctx.exceptions.overflow_error) {
                    vm.new_key_error(vm.ctx.new_str("getpwuid(): uid not found").into())
                } else {
                    err
                }
            })?
            .0;
        let user = match host_pwd::getpwuid(uid) {
            Ok(user) => user,
            Err(err) if err.raw_os_error() == Some(libc::ENOMEM) => {
                return Err(vm.no_memory_error());
            }
            Err(_) => None,
        };
        let user = user.ok_or_else(|| {
            vm.new_key_error(
                vm.ctx
                    .new_str(format!(
                        "getpwuid(): uid not found: {}",
                        if uid == u32::MAX { -1 } else { i64::from(uid) }
                    ))
                    .into(),
            )
        })?;
        Ok(PasswdData::from(user))
    }

    // TODO: maybe merge this functionality into nix?
    #[cfg(not(target_os = "android"))]
    #[pyfunction]
    fn getpwall(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        host_pwd::getpwall()
            .into_iter()
            .map(PasswdData::from)
            .map(|passwd| passwd.to_pyobject(vm))
            .collect()
    }
}
