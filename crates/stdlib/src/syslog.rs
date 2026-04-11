// spell-checker:ignore logoption openlog setlogmask upto NDELAY ODELAY

pub(crate) use syslog::module_def;

#[pymodule(name = "syslog")]
mod syslog {
    use crate::vm::{
        PyObjectRef, PyPayload, PyResult, VirtualMachine,
        builtins::{PyStr, PyStrRef},
        function::{OptionalArg, OptionalOption},
        utils::ToCString,
    };
    use rustpython_host_env::syslog as host_syslog;

    #[pyattr]
    use libc::{
        LOG_ALERT, LOG_AUTH, LOG_CONS, LOG_CRIT, LOG_DAEMON, LOG_DEBUG, LOG_EMERG, LOG_ERR,
        LOG_INFO, LOG_KERN, LOG_LOCAL0, LOG_LOCAL1, LOG_LOCAL2, LOG_LOCAL3, LOG_LOCAL4, LOG_LOCAL5,
        LOG_LOCAL6, LOG_LOCAL7, LOG_LPR, LOG_MAIL, LOG_NDELAY, LOG_NEWS, LOG_NOTICE, LOG_NOWAIT,
        LOG_ODELAY, LOG_PID, LOG_SYSLOG, LOG_USER, LOG_UUCP, LOG_WARNING,
    };

    #[cfg(not(target_os = "redox"))]
    #[pyattr]
    use libc::{LOG_AUTHPRIV, LOG_CRON, LOG_PERROR};

    fn get_argv(vm: &VirtualMachine) -> Option<PyStrRef> {
        if let Some(argv) = vm.state.config.settings.argv.first()
            && !argv.is_empty()
        {
            return Some(
                PyStr::from(match argv.find('\\') {
                    Some(value) => &argv[value..],
                    None => argv,
                })
                .into_ref(&vm.ctx),
            );
        }
        None
    }

    #[derive(Default, FromArgs)]
    struct OpenLogArgs {
        #[pyarg(any, optional)]
        ident: OptionalOption<PyStrRef>,
        #[pyarg(any, optional)]
        logoption: OptionalArg<i32>,
        #[pyarg(any, optional)]
        facility: OptionalArg<i32>,
    }

    #[pyfunction]
    fn openlog(args: OpenLogArgs, vm: &VirtualMachine) -> PyResult<()> {
        let logoption = args.logoption.unwrap_or(0);
        let facility = args.facility.unwrap_or(LOG_USER);
        let ident = match args.ident.flatten() {
            Some(args) => Some(args.to_cstring(vm)?),
            None => get_argv(vm).map(|argv| argv.to_cstring(vm)).transpose()?,
        }
        .map(|ident| ident.into_boxed_c_str());

        host_syslog::openlog(ident, logoption, facility);
        Ok(())
    }

    #[derive(FromArgs)]
    struct SysLogArgs {
        #[pyarg(positional)]
        priority: PyObjectRef,
        #[pyarg(positional, optional)]
        message_object: OptionalOption<PyStrRef>,
    }

    #[pyfunction]
    fn syslog(args: SysLogArgs, vm: &VirtualMachine) -> PyResult<()> {
        let (priority, msg) = match args.message_object.flatten() {
            Some(s) => (args.priority.try_into_value(vm)?, s),
            None => (LOG_INFO, args.priority.try_into_value(vm)?),
        };

        if !host_syslog::is_open() {
            openlog(OpenLogArgs::default(), vm)?;
        }

        let cmsg = msg.to_cstring(vm)?;
        host_syslog::syslog(priority, cmsg.as_c_str());
        Ok(())
    }

    #[pyfunction]
    fn closelog() {
        host_syslog::closelog();
    }

    #[pyfunction]
    fn setlogmask(maskpri: i32) -> i32 {
        host_syslog::setlogmask(maskpri)
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
