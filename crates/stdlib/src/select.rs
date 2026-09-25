// spell-checker:disable

pub(crate) use decl::module_def;

use crate::vm::{
    PyObject, PyObjectRef, PyResult, TryFromObject, VirtualMachine, builtins::PyListRef,
};
use rustpython_host_env::select::{self as host_select, FdSet, RawFd, platform::FD_SETSIZE};
use std::io;

#[derive(Traverse)]
struct Selectable {
    obj: PyObjectRef,
    #[pytraverse(skip)]
    fno: RawFd,
}

impl TryFromObject for Selectable {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let fno = obj.try_to_value(vm).or_else(|_| {
            let meth = vm.get_method_or_type_error(
                obj.clone(),
                vm.ctx.interned_str("fileno").unwrap(),
                || "select arg must be an int or object with a fileno() method".to_owned(),
            )?;
            meth.call((), vm)?.try_into_value(vm)
        })?;
        Ok(Self { obj, fno })
    }
}

#[pymodule(name = "select")]
mod decl {
    use super::*;
    use crate::vm::{
        Py, PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyModule, PyTypeRef},
        convert::ToPyException,
        function::Either,
        stdlib::time,
    };

    #[expect(clippy::unnecessary_wraps, reason = "Needs to comply with a signature")]
    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        #[cfg(windows)]
        rustpython_host_env::windows::init_winsock();

        #[cfg(unix)]
        {
            use crate::vm::class::PyClassImpl;
            let _ = poll::PyPoll::make_static_type();
        }

        __module_exec(vm, module);
        Ok(())
    }

    #[pyattr]
    fn error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.exceptions.os_error.to_owned()
    }

    #[derive(FromArgs)]
    struct SelectArgs {
        #[pyarg(positional)]
        rlist: PyObjectRef,
        #[pyarg(positional)]
        wlist: PyObjectRef,
        #[pyarg(positional)]
        xlist: PyObjectRef,
        #[pyarg(positional, optional)]
        timeout: Option<Either<f64, isize>>,
    }

    #[pyfunction]
    fn select(
        args: SelectArgs,
        vm: &VirtualMachine,
    ) -> PyResult<(PyListRef, PyListRef, PyListRef)> {
        let SelectArgs {
            rlist,
            wlist,
            xlist,
            timeout,
        } = args;
        let mut timeout = timeout.map(|e| match e {
            Either::A(f) => f,
            Either::B(i) => i as f64,
        });
        if let Some(timeout) = timeout
            && timeout < 0.0
        {
            return Err(vm.new_value_error("timeout must be positive"));
        }
        let deadline = timeout.map(|s| time::time(vm).unwrap() + s);

        let max_fds: usize = cfg_select! {
            windows => FD_SETSIZE as usize,
            _ => FD_SETSIZE,
        };

        let seq2set = |list: &PyObject| -> PyResult<(Vec<Selectable>, FdSet)> {
            // The limit is answered while the sequence is walked rather than
            // from the length of the result. fileno() runs Python and can
            // append to the very list being walked, and a walk that re-reads
            // the list each step -- which is what `seq2set` does -- then never
            // reaches a length to check.
            let seen = core::cell::Cell::new(0usize);
            let v: Vec<Selectable> = vm.extract_elements_with(list, |obj| {
                let selectable = Selectable::try_from_object(vm, obj)?;
                seen.set(seen.get() + 1);
                if seen.get() > max_fds {
                    return Err(vm.new_value_error("too many file descriptors in select()"));
                }
                Ok(selectable)
            })?;

            let mut fds = FdSet::new();
            for fd in &v {
                #[cfg(unix)]
                if fd.fno as usize >= FD_SETSIZE {
                    return Err(vm.new_value_error("file descriptor out of range in select()"));
                }

                fds.insert(fd.fno);
            }
            Ok((v, fds))
        };

        let (rlist, _) = seq2set(&rlist)?;
        let (wlist, _) = seq2set(&wlist)?;
        let (xlist, _) = seq2set(&xlist)?;

        let nfds = cfg_select! {
            windows => 0, // value is ignored on windows

            _ => rlist
                .iter()
                .chain(&wlist)
                .chain(&xlist)
                .map(|fd| fd.fno)
                .max()
                .map_or(0, |n| n + 1) as _,
        };

        let fill = |list: &[Selectable]| {
            let mut fds = FdSet::new();
            for fd in list {
                fds.insert(fd.fno);
            }
            fds
        };

        let set2list = |list: Vec<Selectable>, mut set: FdSet| {
            vm.ctx.new_list(
                list.into_iter()
                    .filter(|fd| set.contains(fd.fno))
                    .map(|fd| fd.obj)
                    .collect(),
            )
        };

        loop {
            // `select(2)` updates the fd sets in place. Rebuild them on every
            // attempt, including EINTR retries; otherwise an interrupted call
            // can leave empty sets and a NULL timeout, which blocks forever.
            let mut r = fill(&rlist);
            let mut w = fill(&wlist);
            let mut x = fill(&xlist);

            let mut tv = timeout.map(host_select::sec_to_timeval);
            let res =
                vm.allow_threads(|| host_select::select(nfds, &mut r, &mut w, &mut x, tv.as_mut()));

            match res {
                Ok(_) => {
                    return Ok((set2list(rlist, r), set2list(wlist, w), set2list(xlist, x)));
                }
                Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                Err(err) => return Err(err.to_pyexception(vm)),
            }

            vm.check_signals()?;

            if let Some(ref mut timeout) = timeout {
                *timeout = deadline.unwrap() - time::time(vm).unwrap();
                if *timeout < 0.0 {
                    return Ok((
                        vm.ctx.new_list(Vec::new()),
                        vm.ctx.new_list(Vec::new()),
                        vm.ctx.new_list(Vec::new()),
                    ));
                }
            }
        }
    }

    #[cfg(unix)]
    #[pyfunction]
    fn poll() -> poll::PyPoll {
        poll::PyPoll::default()
    }

    #[cfg(unix)]
    #[pyattr]
    use host_select::{
        PIPE_BUF, POLLERR, POLLHUP, POLLIN, POLLNVAL, POLLOUT, POLLPRI, POLLRDBAND, POLLRDNORM,
        POLLWRBAND, POLLWRNORM,
    };

    #[cfg(unix)]
    pub(super) mod poll {
        use super::*;
        use crate::vm::{
            AsObject, PyPayload,
            builtins::PyFloat,
            common::lock::PyMutex,
            convert::{IntoPyException, ToPyObject},
            function::OptionalArg,
            stdlib::_io::Fildes,
        };
        use core::{
            convert::TryFrom,
            sync::atomic::{AtomicBool, Ordering},
            time::Duration,
        };
        use num_traits::{Signed, ToPrimitive};
        use std::time::Instant;

        #[derive(Default)]
        pub(super) struct TimeoutArg<const MILLIS: bool>(pub Option<Duration>);

        impl<const MILLIS: bool> TryFromObject for TimeoutArg<MILLIS> {
            fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
                let timeout = if vm.is_none(&obj) {
                    None
                } else if let Some(float) = obj.downcast_ref::<PyFloat>() {
                    let float = float.to_f64();
                    if float.is_nan() {
                        return Err(vm.new_value_error("Invalid value NaN (not a number)"));
                    }
                    if float.is_sign_negative() {
                        None
                    } else {
                        // MILLIS: the Python timeout is in milliseconds.
                        let secs = if MILLIS { float / 1e3 } else { float };
                        Some(
                            Duration::try_from_secs_f64(secs)
                                .map_err(|_| vm.new_overflow_error("timeout is too large"))?,
                        )
                    }
                } else if let Some(int) = obj.try_index_opt(vm).transpose()? {
                    if int.as_bigint().is_negative() {
                        None
                    } else {
                        let n = int
                            .as_bigint()
                            .to_u64()
                            .ok_or_else(|| vm.new_overflow_error("value out of range"))?;
                        Some(if MILLIS {
                            Duration::from_millis(n)
                        } else {
                            Duration::from_secs(n)
                        })
                    }
                } else {
                    return Err(vm.new_type_error(format!(
                        "expected an int or float for duration, got {}",
                        obj.class()
                    )));
                };
                Ok(Self(timeout))
            }
        }

        #[pyclass(module = "select", name = "poll")]
        #[derive(Default, Debug, PyPayload)]
        pub(crate) struct PyPoll {
            // keep sorted
            fds: PyMutex<Vec<host_select::PollFd>>,
            poll_running: AtomicBool,
        }

        // new EventMask type
        #[derive(Copy, Clone)]
        #[repr(transparent)]
        pub(crate) struct EventMask(pub i16);

        impl TryFromObject for EventMask {
            fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
                use crate::builtins::PyInt;
                let int = obj
                    .downcast::<PyInt>()
                    .map_err(|_| vm.new_type_error("argument must be an integer"))?;

                let val = int.as_bigint();
                if val.is_negative() {
                    return Err(vm.new_value_error("negative event mask"));
                }

                // Try converting to i16, should raise OverflowError if too large
                let mask = i16::try_from(val)
                    .map_err(|_| vm.new_overflow_error("event mask value out of range"))?;

                Ok(Self(mask))
            }
        }

        const DEFAULT_EVENTS: i16 =
            host_select::POLLIN | host_select::POLLPRI | host_select::POLLOUT;

        #[pyclass(flags(DISALLOW_INSTANTIATION))]
        impl PyPoll {
            #[pymethod]
            fn register(&self, Fildes(fd): Fildes, eventmask: OptionalArg<EventMask>) {
                let mask = match eventmask {
                    OptionalArg::Present(event_mask) => event_mask.0,
                    OptionalArg::Missing => DEFAULT_EVENTS,
                };
                host_select::insert_poll_fd(&mut self.fds.lock(), fd, mask);
            }

            #[pymethod]
            fn modify(
                &self,
                Fildes(fd): Fildes,
                eventmask: EventMask,
                vm: &VirtualMachine,
            ) -> PyResult<()> {
                let mut fds = self.fds.lock();
                // CPython raises KeyError if fd is not registered, match that behavior
                let pfd = host_select::get_poll_fd_mut(&mut fds, fd)
                    .ok_or_else(|| vm.new_key_error(vm.ctx.new_int(fd).into()))?;
                pfd.events = eventmask.0;
                Ok(())
            }

            #[pymethod]
            fn unregister(&self, Fildes(fd): Fildes, vm: &VirtualMachine) -> PyResult<()> {
                let removed = host_select::remove_poll_fd(&mut self.fds.lock(), fd);
                removed
                    .map(drop)
                    .ok_or_else(|| vm.new_key_error(vm.ctx.new_int(fd).into()))
            }

            #[pymethod]
            fn poll(
                &self,
                timeout: OptionalArg<TimeoutArg<true>>,
                vm: &VirtualMachine,
            ) -> PyResult<Vec<PyObjectRef>> {
                if self.poll_running.swap(true, Ordering::SeqCst) {
                    return Err(vm.new_runtime_error("concurrent poll() invocation"));
                }
                struct ClearRunning<'a>(&'a AtomicBool);
                impl Drop for ClearRunning<'_> {
                    fn drop(&mut self) {
                        self.0.store(false, Ordering::Release);
                    }
                }
                let _running = ClearRunning(&self.poll_running);

                // Poll a copy: the wait releases the GIL-equivalent and runs
                // signal handlers, which can register or unregister on the same
                // object, and a held lock would deadlock them.
                let mut fds = self.fds.lock().clone();
                let TimeoutArg(timeout) = timeout.unwrap_or_default();
                let timeout_ms = match timeout {
                    Some(d) => host_select::duration_as_millis_ceiling(d)
                        .ok_or_else(|| vm.new_overflow_error("value out of range"))?,
                    None => -1i32,
                };
                let deadline = timeout
                    .map(|d| {
                        Instant::now()
                            .checked_add(d)
                            .ok_or_else(|| vm.new_overflow_error("timeout is too large"))
                    })
                    .transpose()?;
                let mut poll_timeout = timeout_ms;
                loop {
                    match vm.allow_threads(|| host_select::poll_fds(&mut fds, poll_timeout)) {
                        Ok(_) => break,
                        Err(err) if err.raw_os_error() == Some(host_select::EINTR) => {
                            vm.check_signals()?
                        }
                        Err(err) => return Err(err.into_pyexception(vm)),
                    }
                    if let Some(d) = deadline {
                        if let Some(remaining) = d.checked_duration_since(Instant::now()) {
                            poll_timeout = host_select::duration_as_millis_ceiling(remaining)
                                .ok_or_else(|| vm.new_overflow_error("value out of range"))?;
                        } else {
                            break;
                        }
                    }
                }
                Ok(fds
                    .iter()
                    .filter(|pfd| pfd.revents != 0)
                    .map(|pfd| (pfd.fd, pfd.revents & 0xfff).to_pyobject(vm))
                    .collect())
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "redox"))]
    #[pyattr(name = "epoll", once)]
    fn epoll(_vm: &VirtualMachine) -> PyTypeRef {
        use crate::vm::class::PyClassImpl;
        epoll::PyEpoll::make_static_type()
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "redox"))]
    #[pyattr]
    use host_select::{
        EPOLL_CLOEXEC, EPOLLERR, EPOLLEXCLUSIVE, EPOLLHUP, EPOLLIN, EPOLLMSG, EPOLLONESHOT,
        EPOLLOUT, EPOLLPRI, EPOLLRDBAND, EPOLLRDHUP, EPOLLRDNORM, EPOLLWAKEUP, EPOLLWRBAND,
        EPOLLWRNORM,
    };
    #[cfg(any(target_os = "linux", target_os = "android", target_os = "redox"))]
    #[pyattr]
    const EPOLLET: u32 = host_select::EPOLLET as u32;

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "redox"))]
    pub(super) mod epoll {
        use super::*;
        use crate::vm::{
            Py, PyPayload, PyRef,
            builtins::PyType,
            common::lock::{PyRwLock, PyRwLockReadGuard},
            convert::{IntoPyException, ToPyObject},
            function::OptionalArg,
            stdlib::_io::Fildes,
            types::Constructor,
        };
        use core::ops::Deref;
        use std::os::fd::{AsRawFd, OwnedFd};
        use std::time::Instant;

        #[pyclass(module = "select", name = "epoll")]
        #[derive(Debug, rustpython_vm::PyPayload)]
        pub(crate) struct PyEpoll {
            epoll_fd: PyRwLock<Option<OwnedFd>>,
        }

        #[derive(FromArgs)]
        pub(crate) struct EpollNewArgs {
            #[pyarg(any, default = -1)]
            sizehint: i32,
            #[pyarg(any, default = 0)]
            flags: i32,
        }

        impl Constructor for PyEpoll {
            type Args = EpollNewArgs;

            fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
                if let ..=-2 | 0 = args.sizehint {
                    return Err(vm.new_value_error("negative sizehint"));
                }
                if !matches!(args.flags, 0 | host_select::EPOLL_CLOEXEC) {
                    return Err(vm.new_os_error("invalid flags"));
                }
                Self::new().map_err(|e| e.into_pyexception(vm))
            }
        }

        #[derive(FromArgs)]
        struct EpollPollArgs {
            #[pyarg(any, default)]
            timeout: poll::TimeoutArg<false>,
            #[pyarg(any, default = -1)]
            maxevents: i32,
        }

        #[pyclass(with(Constructor))]
        impl PyEpoll {
            fn new() -> std::io::Result<Self> {
                let epoll_fd = host_select::epoll::create()?;
                let epoll_fd = Some(epoll_fd).into();
                Ok(Self { epoll_fd })
            }

            #[pymethod]
            fn close(&self) -> std::io::Result<()> {
                let fd = self.epoll_fd.write().take();
                if let Some(fd) = fd {
                    host_select::epoll::close(fd)?;
                }
                Ok(())
            }

            #[pygetset]
            fn closed(&self) -> bool {
                self.epoll_fd.read().is_none()
            }

            fn get_epoll(
                &self,
                vm: &VirtualMachine,
            ) -> PyResult<impl Deref<Target = OwnedFd> + '_> {
                PyRwLockReadGuard::try_map(self.epoll_fd.read(), |x| x.as_ref())
                    .map_err(|_| vm.new_value_error("I/O operation on closed epoll object"))
            }

            #[pymethod]
            fn fileno(&self, vm: &VirtualMachine) -> PyResult<i32> {
                self.get_epoll(vm).map(|epoll_fd| epoll_fd.as_raw_fd())
            }

            #[pyclassmethod]
            fn fromfd(cls: PyTypeRef, fd: OwnedFd, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
                let epoll_fd = Some(fd).into();
                Self { epoll_fd }.into_ref_with_type(vm, cls)
            }

            #[pymethod]
            fn register(
                &self,
                fd: Fildes,
                eventmask: OptionalArg<u32>,
                vm: &VirtualMachine,
            ) -> PyResult<()> {
                let events = match eventmask {
                    OptionalArg::Present(mask) => mask,
                    OptionalArg::Missing => (host_select::epoll::EventFlags::IN
                        | host_select::epoll::EventFlags::PRI
                        | host_select::epoll::EventFlags::OUT)
                        .bits(),
                };
                let epoll_fd = &*self.get_epoll(vm)?;
                host_select::epoll::add(epoll_fd, fd, fd.as_raw_fd() as u64, events)
                    .map_err(|e| e.into_pyexception(vm))
            }

            #[pymethod]
            fn modify(&self, fd: Fildes, eventmask: u32, vm: &VirtualMachine) -> PyResult<()> {
                let epoll_fd = &*self.get_epoll(vm)?;
                host_select::epoll::modify(epoll_fd, fd, fd.as_raw_fd() as u64, eventmask)
                    .map_err(|e| e.into_pyexception(vm))
            }

            #[pymethod]
            fn unregister(&self, fd: Fildes, vm: &VirtualMachine) -> PyResult<()> {
                let epoll_fd = &*self.get_epoll(vm)?;
                host_select::epoll::delete(epoll_fd, fd).map_err(|e| e.into_pyexception(vm))
            }

            #[pymethod]
            fn poll(&self, args: EpollPollArgs, vm: &VirtualMachine) -> PyResult<PyListRef> {
                let poll::TimeoutArg(timeout) = args.timeout;
                let maxevents = args.maxevents;

                let mut poll_timeout = timeout
                    .map(host_select::epoll::Timespec::try_from)
                    .transpose()
                    .map_err(|_| vm.new_overflow_error("timeout is too large"))?;

                let deadline = timeout
                    .map(|d| {
                        Instant::now()
                            .checked_add(d)
                            .ok_or_else(|| vm.new_overflow_error("timeout is too large"))
                    })
                    .transpose()?;
                let maxevents = match maxevents {
                    ..-1 => {
                        return Err(vm.new_value_error(format!(
                            "maxevents must be greater than 0, got {maxevents}"
                        )));
                    }
                    -1 => host_select::FD_SETSIZE - 1,
                    _ => maxevents as usize,
                };

                let mut events = Vec::<host_select::epoll::Event>::with_capacity(maxevents);

                let epoll = &*self.get_epoll(vm)?;

                loop {
                    match vm.allow_threads(|| {
                        host_select::epoll::wait(epoll, &mut events, poll_timeout.as_ref())
                    }) {
                        Ok(_) => break,
                        Err(host_select::epoll::WaitError::Interrupted) => vm.check_signals()?,
                        Err(host_select::epoll::WaitError::Io(e)) => {
                            return Err(e.into_pyexception(vm));
                        }
                    }
                    if let Some(deadline) = deadline {
                        if let Some(new_timeout) = deadline.checked_duration_since(Instant::now()) {
                            poll_timeout = Some(
                                new_timeout
                                    .try_into()
                                    .map_err(|_| vm.new_overflow_error("timeout is too large"))?,
                            );
                        } else {
                            break;
                        }
                    }
                }

                let ret = events
                    .iter()
                    .map(|ev| (ev.data.u64() as i32, { ev.flags }.bits()).to_pyobject(vm))
                    .collect();

                Ok(vm.ctx.new_list(ret))
            }

            #[pymethod]
            fn __enter__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
                zelf.get_epoll(vm)?;
                Ok(zelf)
            }

            #[pymethod]
            fn __exit__(
                &self,
                _exc_type: OptionalArg,
                _exc_value: OptionalArg,
                _exc_tb: OptionalArg,
            ) -> std::io::Result<()> {
                self.close()
            }
        }
    }

    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly",
    ))]
    #[pyattr(name = "kqueue", once)]
    fn kqueue_type(_vm: &VirtualMachine) -> PyTypeRef {
        use crate::vm::class::PyClassImpl;
        kqueue::PyKqueue::make_static_type()
    }

    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly",
    ))]
    #[pyattr(name = "kevent", once)]
    fn kevent_type(_vm: &VirtualMachine) -> PyTypeRef {
        use crate::vm::class::PyClassImpl;
        kqueue::PyKevent::make_static_type()
    }

    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly",
    ))]
    #[pyattr]
    use host_select::kqueue::{
        EV_ADD as KQ_EV_ADD, EV_CLEAR as KQ_EV_CLEAR, EV_DELETE as KQ_EV_DELETE,
        EV_DISABLE as KQ_EV_DISABLE, EV_ENABLE as KQ_EV_ENABLE, EV_EOF as KQ_EV_EOF,
        EV_ERROR as KQ_EV_ERROR, EV_FLAG1 as KQ_EV_FLAG1, EV_ONESHOT as KQ_EV_ONESHOT,
        EV_SYSFLAGS as KQ_EV_SYSFLAGS, EVFILT_AIO as KQ_FILTER_AIO, EVFILT_PROC as KQ_FILTER_PROC,
        EVFILT_READ as KQ_FILTER_READ, EVFILT_SIGNAL as KQ_FILTER_SIGNAL,
        EVFILT_TIMER as KQ_FILTER_TIMER, EVFILT_VNODE as KQ_FILTER_VNODE,
        EVFILT_WRITE as KQ_FILTER_WRITE, NOTE_ATTRIB as KQ_NOTE_ATTRIB,
        NOTE_CHILD as KQ_NOTE_CHILD, NOTE_DELETE as KQ_NOTE_DELETE, NOTE_EXEC as KQ_NOTE_EXEC,
        NOTE_EXIT as KQ_NOTE_EXIT, NOTE_EXTEND as KQ_NOTE_EXTEND, NOTE_FORK as KQ_NOTE_FORK,
        NOTE_LINK as KQ_NOTE_LINK, NOTE_LOWAT as KQ_NOTE_LOWAT,
        NOTE_PCTRLMASK as KQ_NOTE_PCTRLMASK, NOTE_PDATAMASK as KQ_NOTE_PDATAMASK,
        NOTE_RENAME as KQ_NOTE_RENAME, NOTE_REVOKE as KQ_NOTE_REVOKE, NOTE_TRACK as KQ_NOTE_TRACK,
        NOTE_TRACKERR as KQ_NOTE_TRACKERR, NOTE_WRITE as KQ_NOTE_WRITE,
    };

    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly",
    ))]
    pub(super) mod kqueue {
        use super::*;
        use crate::vm::{
            Py, PyObject, PyPayload, PyRef,
            builtins::{PyFloat, PyType},
            class_or_notimplemented,
            common::lock::{PyMutex, PyRwLock},
            convert::{IntoPyException, ToPyObject},
            function::PyComparisonValue,
            types::{Comparable, Constructor, Destructor, PyComparisonOp, Representable},
        };
        use alloc::sync::Arc;
        use core::sync::atomic::AtomicI32;
        use num_traits::ToPrimitive;
        use std::time::Instant;

        #[pyclass(module = "select", name = "kevent")]
        #[derive(Debug, PyPayload)]
        pub(crate) struct PyKevent {
            ev: PyMutex<host_select::kqueue::Event>,
        }

        #[derive(FromArgs)]
        pub(crate) struct KeventNewArgs {
            #[pyarg(any)]
            ident: PyObjectRef,
            #[pyarg(any, default = host_select::kqueue::DEFAULT_FILTER, py_default = "<unrepresentable>")]
            filter: i16,
            #[pyarg(any, default = host_select::kqueue::DEFAULT_FLAGS, py_default = "<unrepresentable>")]
            flags: u16,
            #[pyarg(any, default = 0)]
            fflags: u32,
            #[pyarg(any, default = 0)]
            data: isize,
            #[pyarg(any, default = 0)]
            udata: usize,
        }

        fn ident_from_object(obj: &PyObject, vm: &VirtualMachine) -> PyResult<usize> {
            if let Some(idx) = obj.try_index_opt(vm) {
                let n = idx?;
                n.try_to_primitive(vm).map_err(|_| {
                    vm.new_overflow_error("Python int too large for C kqueue event identifier")
                })
            } else {
                Selectable::try_from_object(vm, obj.to_owned()).map(|s| s.fno as usize)
            }
        }

        impl Constructor for PyKevent {
            type Args = KeventNewArgs;

            fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
                let ident = ident_from_object(&args.ident, vm)?;
                Ok(Self {
                    ev: PyMutex::new(host_select::kqueue::Event {
                        ident,
                        filter: args.filter,
                        flags: args.flags,
                        fflags: args.fflags,
                        data: args.data,
                        udata: args.udata,
                    }),
                })
            }
        }

        #[pyclass(with(Constructor, Comparable, Representable))]
        impl PyKevent {
            pub(super) fn event(&self) -> host_select::kqueue::Event {
                *self.ev.lock()
            }

            pub(super) fn from_event(ev: host_select::kqueue::Event) -> Self {
                Self {
                    ev: PyMutex::new(ev),
                }
            }

            #[pygetset]
            fn ident(&self) -> usize {
                self.ev.lock().ident
            }

            #[pygetset(setter)]
            fn set_ident(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
                self.ev.lock().ident = ident_from_object(&value, vm)?;
                Ok(())
            }

            #[pygetset]
            fn filter(&self) -> i16 {
                self.ev.lock().filter
            }

            #[pygetset(setter)]
            fn set_filter(&self, value: i16) {
                self.ev.lock().filter = value;
            }

            #[pygetset]
            fn flags(&self) -> u16 {
                self.ev.lock().flags
            }

            #[pygetset(setter)]
            fn set_flags(&self, value: u16) {
                self.ev.lock().flags = value;
            }

            #[pygetset]
            fn fflags(&self) -> u32 {
                self.ev.lock().fflags
            }

            #[pygetset(setter)]
            fn set_fflags(&self, value: u32) {
                self.ev.lock().fflags = value;
            }

            #[pygetset]
            fn data(&self) -> isize {
                self.ev.lock().data
            }

            #[pygetset(setter)]
            fn set_data(&self, value: isize) {
                self.ev.lock().data = value;
            }

            #[pygetset]
            fn udata(&self) -> usize {
                self.ev.lock().udata
            }

            #[pygetset(setter)]
            fn set_udata(&self, value: usize) {
                self.ev.lock().udata = value;
            }
        }

        impl Comparable for PyKevent {
            fn cmp(
                zelf: &Py<Self>,
                other: &PyObject,
                op: PyComparisonOp,
                _vm: &VirtualMachine,
            ) -> PyResult<PyComparisonValue> {
                let other = class_or_notimplemented!(Self, other);
                let a = zelf.event();
                let b = other.event();
                let ord = a
                    .ident
                    .cmp(&b.ident)
                    .then_with(|| a.filter.cmp(&b.filter))
                    .then_with(|| a.flags.cmp(&b.flags))
                    .then_with(|| a.fflags.cmp(&b.fflags))
                    .then_with(|| a.data.cmp(&b.data))
                    .then_with(|| a.udata.cmp(&b.udata));
                Ok(PyComparisonValue::Implemented(op.eval_ord(ord)))
            }
        }

        impl Representable for PyKevent {
            fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
                let e = zelf.event();
                Ok(format!(
                    "<select.kevent ident={} filter={} flags=0x{:x} fflags=0x{:x} data=0x{:x} udata={:p}>",
                    e.ident, e.filter, e.flags, e.fflags, e.data, e.udata as *const (),
                ))
            }
        }

        #[pyclass(module = "select", name = "kqueue")]
        #[derive(Debug, PyPayload)]
        pub(crate) struct PyKqueue {
            kqfd: PyRwLock<Option<Arc<AtomicI32>>>,
        }

        impl Constructor for PyKqueue {
            type Args = ();

            fn py_new(_cls: &Py<PyType>, _args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
                let kqfd = host_select::kqueue::create().map_err(|e| e.into_pyexception(vm))?;
                Ok(Self {
                    kqfd: PyRwLock::new(Some(kqfd)),
                })
            }
        }

        impl Destructor for PyKqueue {
            fn del(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<()> {
                let cell = zelf.kqfd.write().take();
                if let Some(cell) = cell {
                    let _ = host_select::kqueue::close(&cell);
                }
                Ok(())
            }
        }

        #[derive(FromArgs)]
        struct KqueueControlArgs {
            #[pyarg(positional)]
            changelist: PyObjectRef,
            #[pyarg(positional)]
            maxevents: i32,
            #[pyarg(positional, optional)]
            timeout: Option<PyObjectRef>,
        }

        fn timespec_from_timeout(
            timeout: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<Option<host_select::kqueue::Timespec>> {
            let Some(obj) = timeout else {
                return Ok(None);
            };
            if vm.is_none(&obj) {
                return Ok(None);
            }
            let secs = if let Some(float) = obj.downcast_ref::<PyFloat>() {
                float.to_f64()
            } else if let Some(int) = obj.try_index_opt(vm).transpose()? {
                int.as_bigint()
                    .to_f64()
                    .ok_or_else(|| vm.new_overflow_error("timeout is too large"))?
            } else {
                return Err(vm.new_type_error(format!(
                    "timeout argument must be a number or None, got {}",
                    obj.class()
                )));
            };
            if secs.is_nan() {
                return Err(vm.new_value_error("Invalid value NaN (not a number)"));
            }
            if secs < 0.0 {
                return Err(vm.new_value_error("timeout must be positive or None"));
            }
            host_select::kqueue::Timespec::from_secs(secs)
                .ok_or_else(|| vm.new_overflow_error("timeout is too large"))
                .map(Some)
        }

        #[pyclass(with(Constructor, Destructor))]
        impl PyKqueue {
            #[pymethod]
            fn close(&self) -> io::Result<()> {
                let cell = self.kqfd.write().take();
                if let Some(cell) = cell {
                    host_select::kqueue::close(&cell)?;
                }
                Ok(())
            }

            #[pygetset]
            fn closed(&self) -> bool {
                self.kqfd
                    .read()
                    .as_ref()
                    .is_none_or(|cell| host_select::kqueue::fd(cell) < 0)
            }

            fn fd_or_closed(cell: Option<&Arc<AtomicI32>>, vm: &VirtualMachine) -> PyResult<i32> {
                match cell {
                    Some(cell) => {
                        let fd = host_select::kqueue::fd(cell);
                        if fd < 0 {
                            Err(vm.new_value_error("I/O operation on closed kqueue object"))
                        } else {
                            Ok(fd)
                        }
                    }
                    None => Err(vm.new_value_error("I/O operation on closed kqueue object")),
                }
            }

            #[pymethod]
            fn fileno(&self, vm: &VirtualMachine) -> PyResult<i32> {
                Self::fd_or_closed(self.kqfd.read().as_ref(), vm)
            }

            #[pyclassmethod]
            fn fromfd(cls: PyTypeRef, fd: i32, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
                Self {
                    kqfd: PyRwLock::new(Some(host_select::kqueue::from_fd(fd))),
                }
                .into_ref_with_type(vm, cls)
            }

            #[pymethod]
            fn control(&self, args: KqueueControlArgs, vm: &VirtualMachine) -> PyResult<PyListRef> {
                if args.maxevents < 0 {
                    return Err(vm.new_value_error(format!(
                        "Length of eventlist must be 0 or positive, got {}",
                        args.maxevents
                    )));
                }

                let mut timeout = timespec_from_timeout(args.timeout, vm)?;
                let deadline = timeout
                    .map(|ts| {
                        let duration = ts
                            .to_duration()
                            .ok_or_else(|| vm.new_overflow_error("timeout is too large"))?;
                        Instant::now()
                            .checked_add(duration)
                            .ok_or_else(|| vm.new_overflow_error("timeout is too large"))
                    })
                    .transpose()?;

                let changelist = if vm.is_none(&args.changelist) {
                    Vec::new()
                } else {
                    let items: Vec<PyRef<PyKevent>> =
                        vm.extract_elements_with(&args.changelist, |item| {
                            item.downcast().map_err(|_| {
                                vm.new_type_error(
                                    "changelist must be an iterable of select.kevent objects",
                                )
                            })
                        })?;
                    if items.len() > i32::MAX as usize {
                        return Err(vm.new_overflow_error("changelist is too long"));
                    }
                    items.iter().map(|ev| ev.event()).collect()
                };

                let mut eventlist =
                    vec![host_select::kqueue::Event::default(); args.maxevents as usize];

                let n = loop {
                    let result = {
                        let guard = self.kqfd.read();
                        let fd = Self::fd_or_closed(guard.as_ref(), vm)?;
                        vm.allow_threads(|| {
                            host_select::kqueue::kevent(
                                fd,
                                &changelist,
                                &mut eventlist,
                                timeout.as_ref(),
                            )
                        })
                    };
                    match result {
                        Ok(n) => break n,
                        Err(err) if err.kind() == io::ErrorKind::Interrupted => {
                            vm.check_signals()?;
                            if let Some(deadline) = deadline {
                                if let Some(remaining) =
                                    deadline.checked_duration_since(Instant::now())
                                {
                                    timeout = Some(host_select::kqueue::Timespec::from_duration(
                                        remaining,
                                    ));
                                } else {
                                    break 0;
                                }
                            }
                        }
                        Err(err) => return Err(err.into_pyexception(vm)),
                    }
                };

                let out = eventlist
                    .into_iter()
                    .take(n)
                    .map(|ev| PyKevent::from_event(ev).to_pyobject(vm))
                    .collect();
                Ok(vm.ctx.new_list(out))
            }
        }
    }
}
