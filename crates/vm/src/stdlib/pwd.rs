// spell-checker:disable

pub(crate) use pwd::module_def;

#[pymodule]
mod pwd {
    use crate::{
        PyResult, VirtualMachine,
        builtins::{PyIntRef, PyUtf8StrRef},
        convert::IntoPyException,
        exceptions,
        types::PyStructSequence,
    };
    use rustpython_host_env::pwd as host_pwd;

    #[cfg(not(target_os = "android"))]
    use crate::{PyObjectRef, convert::ToPyObject};

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
            PasswdData {
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
        let pw_name = name.as_str();
        if pw_name.contains('\0') {
            return Err(exceptions::cstring_error(vm));
        }
        let user = host_pwd::getpwnam(name.as_str());
        let user = user.ok_or_else(|| {
            vm.new_key_error(
                vm.ctx
                    .new_str(format!("getpwnam(): name not found: {pw_name}"))
                    .into(),
            )
        })?;
        Ok(PasswdData::from(user))
    }

    #[pyfunction]
    fn getpwuid(uid: PyIntRef, vm: &VirtualMachine) -> PyResult<PasswdData> {
        let uid_t = libc::uid_t::try_from(uid.as_bigint()).ok();
        let user = uid_t
            .map(host_pwd::getpwuid)
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
        Ok(PasswdData::from(user))
    }

    // TODO: maybe merge this functionality into nix?
    #[cfg(not(target_os = "android"))]
    #[pyfunction]
    fn getpwall(vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        Ok(host_pwd::getpwall()
            .into_iter()
            .map(PasswdData::from)
            .map(|passwd| passwd.to_pyobject(vm))
            .collect())
    }
}
