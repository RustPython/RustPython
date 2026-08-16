// spell-checker:disable

pub(crate) use resource::module_def;

#[pymodule]
mod resource {
    use crate::vm::{
        PyObject, PyObjectRef, PyResult, TryFromBorrowedObject, VirtualMachine,
        builtins::PyIntRef,
        convert::{ToPyException, ToPyObject},
        types::PyStructSequence,
    };
    use num_traits::{Signed, ToPrimitive};
    use rustpython_host_env::resource as host_resource;
    use std::io;

    #[cfg_attr(target_os = "android", expect(deprecated))]
    const RLIM_NLIMITS: i32 = cfg_select! {
        target_os = "android" => {
            host_resource::RLIM_NLIMITS
        }
        _ => {
            // This constant isn't abi-stable across os versions, so we just
            // pick a high number so we don't get false positive ValueErrors and just bubble up the
            // EINVAL that get/setrlimit return on an invalid resource
            256
        }
    };

    // TODO: RLIMIT_OFILE,
    #[pyattr]
    use host_resource::{
        RLIM_INFINITY, RLIMIT_AS, RLIMIT_CORE, RLIMIT_CPU, RLIMIT_DATA, RLIMIT_FSIZE,
        RLIMIT_MEMLOCK, RLIMIT_NOFILE, RLIMIT_NPROC, RLIMIT_RSS, RLIMIT_STACK,
    };

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "emscripten"))]
    #[pyattr]
    use host_resource::{RLIMIT_MSGQUEUE, RLIMIT_NICE, RLIMIT_RTPRIO, RLIMIT_SIGPENDING};
    // TODO: I think this is supposed to be defined for all linux_like?
    #[cfg(target_os = "linux")]
    #[pyattr]
    use host_resource::RLIMIT_RTTIME;

    #[cfg(any(
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "solaris",
        target_os = "illumos"
    ))]
    #[pyattr]
    use host_resource::RLIMIT_SBSIZE;

    #[cfg(any(target_os = "freebsd", target_os = "solaris", target_os = "illumos"))]
    #[pyattr]
    use host_resource::{RLIMIT_NPTS, RLIMIT_SWAP};

    #[cfg(any(target_os = "solaris", target_os = "illumos"))]
    #[pyattr]
    use host_resource::RLIMIT_VMEM;

    #[cfg(any(target_os = "linux", target_os = "emscripten", target_os = "freebsd"))]
    #[pyattr]
    use host_resource::RUSAGE_THREAD;
    #[cfg(not(any(target_os = "windows", target_os = "redox")))]
    #[pyattr]
    use host_resource::{RUSAGE_CHILDREN, RUSAGE_SELF};

    #[pystruct_sequence_data]
    struct RUsageData {
        ru_utime: f64,
        ru_stime: f64,
        ru_maxrss: host_resource::c_long,
        ru_ixrss: host_resource::c_long,
        ru_idrss: host_resource::c_long,
        ru_isrss: host_resource::c_long,
        ru_minflt: host_resource::c_long,
        ru_majflt: host_resource::c_long,
        ru_nswap: host_resource::c_long,
        ru_inblock: host_resource::c_long,
        ru_oublock: host_resource::c_long,
        ru_msgsnd: host_resource::c_long,
        ru_msgrcv: host_resource::c_long,
        ru_nsignals: host_resource::c_long,
        ru_nvcsw: host_resource::c_long,
        ru_nivcsw: host_resource::c_long,
    }

    #[pyattr]
    #[pystruct_sequence(name = "struct_rusage", module = "resource", data = "RUsageData")]
    struct PyRUsage;

    #[pyclass(with(PyStructSequence))]
    impl PyRUsage {}

    impl From<host_resource::RUsage> for RUsageData {
        fn from(rusage: host_resource::RUsage) -> Self {
            let tv =
                |tv: host_resource::timeval| tv.tv_sec as f64 + (tv.tv_usec as f64 / 1_000_000.0);
            Self {
                ru_utime: tv(rusage.ru_utime),
                ru_stime: tv(rusage.ru_stime),
                ru_maxrss: rusage.ru_maxrss,
                ru_ixrss: rusage.ru_ixrss,
                ru_idrss: rusage.ru_idrss,
                ru_isrss: rusage.ru_isrss,
                ru_minflt: rusage.ru_minflt,
                ru_majflt: rusage.ru_majflt,
                ru_nswap: rusage.ru_nswap,
                ru_inblock: rusage.ru_inblock,
                ru_oublock: rusage.ru_oublock,
                ru_msgsnd: rusage.ru_msgsnd,
                ru_msgrcv: rusage.ru_msgrcv,
                ru_nsignals: rusage.ru_nsignals,
                ru_nvcsw: rusage.ru_nvcsw,
                ru_nivcsw: rusage.ru_nivcsw,
            }
        }
    }

    #[pyfunction]
    fn getrusage(who: i32, vm: &VirtualMachine) -> PyResult<RUsageData> {
        let res = host_resource::getrusage(who);
        res.map(RUsageData::from).map_err(|e| {
            if e.kind() == io::ErrorKind::InvalidInput {
                vm.new_value_error("invalid who parameter")
            } else {
                e.to_pyexception(vm)
            }
        })
    }

    struct Limits(host_resource::rlimit);

    impl<'a> TryFromBorrowedObject<'a> for Limits {
        fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
            let seq: Vec<PyIntRef> = obj.try_to_value(vm)?;
            match seq.as_slice() {
                [cur, max] => Ok(Self(host_resource::rlimit {
                    rlim_cur: py2rlim(cur.clone(), vm)?,
                    rlim_max: py2rlim(max.clone(), vm)?,
                })),
                _ => Err(vm.new_value_error("expected a tuple of 2 integers")),
            }
        }
    }

    impl ToPyObject for Limits {
        fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
            (self.0.rlim_cur, self.0.rlim_max).to_pyobject(vm)
        }
    }

    fn py2rlim(obj: PyIntRef, vm: &VirtualMachine) -> PyResult<host_resource::rlim_t> {
        let value = obj.as_bigint();

        // CPython converts the int as unsigned native bytes: a negative int is
        // only accepted when it maps to RLIM_INFINITY, which is exactly -1.
        if value.is_negative() {
            return if value.to_i64() == Some(-1) {
                Ok(RLIM_INFINITY)
            } else {
                Err(vm.new_value_error("Cannot convert negative int"))
            };
        }

        host_resource::rlim_t::try_from(value)
            .map_err(|_| vm.new_overflow_error("Python int too large to convert to C rlim_t"))
    }

    // The resource argument is a C int in CPython.
    fn py2cint(obj: PyIntRef, vm: &VirtualMachine) -> PyResult<i32> {
        obj.as_bigint()
            .to_i32()
            .ok_or_else(|| vm.new_overflow_error("Python int too large to convert to C int"))
    }

    fn check_resource(resource: i32, vm: &VirtualMachine) -> PyResult<i32> {
        if !(0..RLIM_NLIMITS).contains(&resource) {
            return Err(vm.new_value_error("invalid resource specified"));
        }
        Ok(resource)
    }

    #[pyfunction]
    fn getrlimit(resource: PyIntRef, vm: &VirtualMachine) -> PyResult<Limits> {
        let resource = check_resource(py2cint(resource, vm)?, vm)?;

        let rlimit = host_resource::getrlimit(resource as host_resource::rlim_t)
            .map_err(|_| vm.new_last_errno_error())?;
        Ok(Limits(rlimit))
    }

    #[pyfunction]
    fn setrlimit(resource: PyIntRef, limits: Limits, vm: &VirtualMachine) -> PyResult<()> {
        let resource = check_resource(py2cint(resource, vm)?, vm)?;

        let res = host_resource::setrlimit(resource as host_resource::rlim_t, limits.0);

        res.map_err(|e| match e.kind() {
            io::ErrorKind::InvalidInput => {
                vm.new_value_error("current limit exceeds maximum limit")
            }
            io::ErrorKind::PermissionDenied => {
                vm.new_value_error("not allowed to raise maximum limit")
            }
            _ => e.to_pyexception(vm),
        })
    }
}
