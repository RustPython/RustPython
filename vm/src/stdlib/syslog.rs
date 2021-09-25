pub(crate) use syslog::make_module;

#[pymodule(name = "syslog")]
mod syslog {
    use crate::common::lock::PyRwLock;
    use crate::{
        builtins::{PyStr, PyStrRef},
        function::{OptionalArg, OptionalOption},
        utils::ToCString,
        PyObjectRef, PyResult, PyValue, TryFromObject, VirtualMachine,
    };
    use std::{ffi::CStr, os::raw::c_char};

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
        if let Some(argv) = vm.state.settings.argv.first() {
            if !argv.is_empty() {
                return Some(
                    PyStr::from(match argv.find('\\') {
                        Some(value) => &argv[value..],
                        None => argv,
                    })
                    .into_ref(vm),
                );
            }
        }
        None
    }

    #[derive(Debug)]
    enum GlobalIdent {
        Explicit(Box<CStr>),
        Implicit,
    }

    impl GlobalIdent {
        fn as_ptr(&self) -> *const c_char {
            match self {
                GlobalIdent::Explicit(ref cstr) => cstr.as_ptr(),
                GlobalIdent::Implicit => std::ptr::null(),
            }
        }
    }

    fn global_ident() -> &'static PyRwLock<Option<GlobalIdent>> {
        rustpython_common::static_cell! {
            static IDENT: PyRwLock<Option<GlobalIdent>>;
        };
        IDENT.get_or_init(|| PyRwLock::new(None))
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

        let ident = match ident {
            Some(ident) => GlobalIdent::Explicit(ident),
            None => GlobalIdent::Implicit,
        };

        {
            let mut locked_ident = global_ident().write();
            unsafe { libc::openlog(ident.as_ptr(), logoption, facility) };
            *locked_ident = Some(ident);
        }
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
            Some(s) => (i32::try_from_object(vm, args.priority)?, s),
            None => (LOG_INFO, PyStrRef::try_from_object(vm, args.priority)?),
        };

        if global_ident().read().is_none() {
            openlog(OpenLogArgs::default(), vm)?;
        }

        let (cformat, cmsg) = ("%s".to_cstring(vm)?, msg.to_cstring(vm)?);
        unsafe { libc::syslog(priority, cformat.as_ptr(), cmsg.as_ptr()) };
        Ok(())
    }

    #[pyfunction]
    fn closelog() {
        if global_ident().read().is_some() {
            let mut locked_ident = global_ident().write();
            unsafe { libc::closelog() };
            *locked_ident = None;
        }
    }

    #[pyfunction]
    fn setlogmask(maskpri: i32) -> i32 {
        unsafe { libc::setlogmask(maskpri) }
    }

    #[inline]
    #[pyfunction(name = "LOG_MASK")]
    fn log_mask(pri: i32) -> i32 {
        pri << 1
    }

    #[inline]
    #[pyfunction(name = "LOG_UPTO")]
    fn log_upto(pri: i32) -> i32 {
        (1 << (pri + 1)) - 1
    }
}
