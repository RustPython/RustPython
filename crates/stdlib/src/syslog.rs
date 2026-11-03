// spell-checker:ignore logoption openlog setlogmask upto NDELAY ODELAY

pub(crate) use syslog::module_def;

#[pymodule(name = "syslog")]
mod syslog {
    use crate::vm::{
        PyObjectRef, PyPayload, PyResult, VirtualMachine,
        builtins::{PyList, PyStr, PyStrRef},
        function::OptionalArg,
        utils::ToCString,
    };
    use core::ffi::c_long;
    use rustpython_host_env::syslog as host_syslog;

    #[pyattr]
    use host_syslog::{
        LOG_ALERT, LOG_AUTH, LOG_CONS, LOG_CRIT, LOG_DAEMON, LOG_DEBUG, LOG_EMERG, LOG_ERR,
        LOG_INFO, LOG_KERN, LOG_LOCAL0, LOG_LOCAL1, LOG_LOCAL2, LOG_LOCAL3, LOG_LOCAL4, LOG_LOCAL5,
        LOG_LOCAL6, LOG_LOCAL7, LOG_LPR, LOG_MAIL, LOG_NDELAY, LOG_NEWS, LOG_NOTICE, LOG_NOWAIT,
        LOG_ODELAY, LOG_PID, LOG_SYSLOG, LOG_USER, LOG_UUCP, LOG_WARNING,
    };

    #[cfg(not(target_os = "redox"))]
    #[pyattr]
    use host_syslog::{LOG_AUTHPRIV, LOG_CRON, LOG_PERROR};

    #[cfg(target_vendor = "apple")]
    #[pyattr]
    use host_syslog::{LOG_FTP, LOG_INSTALL, LOG_LAUNCHD, LOG_NETINFO, LOG_RAS, LOG_REMOTEAUTH};

    fn get_argv(vm: &VirtualMachine) -> Option<PyStrRef> {
        let argv = vm.sys_module.dict().get_item_opt("argv", vm).ok()??;
        let argv = argv.downcast_ref::<PyList>()?;
        let script = argv.borrow_vec().first()?.clone();
        let script = script.downcast::<PyStr>().ok()?;
        let name = script.as_wtf8();
        if name.is_empty() {
            return None;
        }
        // syslog is a Unix module: use the final slash in the current sys.argv[0].
        match name.as_bytes().iter().rposition(|&b| b == b'/') {
            Some(slash) => Some(PyStr::from(&name[slash + 1..]).into_ref(&vm.ctx)),
            None => Some(script),
        }
    }

    #[derive(Default, FromArgs)]
    struct OpenLogArgs {
        #[pyarg(any, optional)]
        ident: OptionalArg<PyStrRef>,
        #[pyarg(any, default)]
        logoption: c_long,
        #[pyarg(any, default = LOG_USER as c_long, py_default = "LOG_USER")]
        facility: c_long,
    }

    #[pyfunction]
    fn openlog(args: OpenLogArgs, vm: &VirtualMachine) -> PyResult<()> {
        if !vm.state.is_main_interpreter() {
            return Err(vm.new_runtime_error("subinterpreter can't use syslog.openlog()"));
        }
        let logoption = args.logoption;
        let facility = args.facility;
        let ident = args.ident.into_option().or_else(|| get_argv(vm));
        let c_ident = ident
            .as_ref()
            .map(|ident| {
                let utf8 = ident.try_as_utf8(vm)?;
                // openlog accepts a Unicode ident, not a NUL-checked "s" argument.
                // Keep the full object for auditing, but pass its C string prefix.
                utf8.as_str()
                    .split('\0')
                    .next()
                    .unwrap_or_default()
                    .to_cstring(vm)
            })
            .transpose()?
            .map(|ident| ident.into_boxed_c_str());

        vm.audit("syslog.openlog", || {
            (
                ident
                    .as_ref()
                    .map_or_else(|| vm.ctx.none(), |s| s.clone().into()),
                logoption,
                facility,
            )
        })?;

        host_syslog::openlog(c_ident, logoption as i32, facility as i32);
        Ok(())
    }

    #[derive(FromArgs)]
    struct SysLogArgs {
        #[pyarg(positional)]
        priority: PyObjectRef,
        #[pyarg(positional, optional)]
        message: OptionalArg<PyObjectRef>,
    }

    #[pyfunction]
    fn syslog(args: SysLogArgs, vm: &VirtualMachine) -> PyResult<()> {
        let (priority, msg): (i32, PyStrRef) = match args.message {
            OptionalArg::Present(message) => {
                // Convert priority before inspecting message, as PyArg_ParseTuple does.
                let priority = args.priority.try_into_value(vm)?;
                (priority, message.try_into_value(vm)?)
            }
            OptionalArg::Missing => (LOG_INFO, args.priority.try_into_value(vm)?),
        };
        // Reject invalid UTF-8 and embedded NUL before auditing or opening the log.
        let utf8 = msg.try_as_utf8(vm)?;
        let cmsg = utf8.to_cstring(vm)?;

        vm.audit("syslog.syslog", || {
            (priority, vm.ctx.new_str(utf8.as_str()))
        })?;

        if !host_syslog::is_open() {
            if !vm.state.is_main_interpreter() {
                return Err(vm.new_runtime_error(
                    "subinterpreter can't use syslog.syslog() until the syslog is opened by the main interpreter",
                ));
            }
            openlog(
                OpenLogArgs {
                    facility: LOG_USER as c_long,
                    ..OpenLogArgs::default()
                },
                vm,
            )?;
        }

        host_syslog::syslog(priority, cmsg.as_c_str());
        Ok(())
    }

    #[pyfunction]
    fn closelog(vm: &VirtualMachine) -> PyResult<()> {
        if !vm.state.is_main_interpreter() {
            return Err(vm.new_runtime_error("subinterpreter can't use syslog.closelog()"));
        }
        vm.audit("syslog.closelog", || ())?;

        host_syslog::closelog();
        Ok(())
    }

    #[pyfunction]
    fn setlogmask(maskpri: c_long, vm: &VirtualMachine) -> PyResult<i32> {
        vm.audit("syslog.setlogmask", || (maskpri,))?;

        Ok(host_syslog::setlogmask(maskpri as i32))
    }

    #[inline]
    #[pyfunction(name = "LOG_MASK")]
    const fn log_mask(pri: i32) -> i32 {
        host_syslog::log_mask(pri)
    }

    #[inline]
    #[pyfunction(name = "LOG_UPTO")]
    const fn log_upto(pri: i32) -> i32 {
        host_syslog::log_upto(pri)
    }
}
