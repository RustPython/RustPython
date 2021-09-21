use crate::vm::{PyObjectRef, PyResult, TryFromBorrowedObject, TryFromObject, VirtualMachine};
use std::{io, mem};

pub(crate) fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    #[cfg(windows)]
    crate::vm::stdlib::nt::init_winsock();

    #[cfg(unix)]
    {
        use crate::vm::PyClassImpl;
        decl::poll::PyPoll::make_class(&vm.ctx);
    }

    decl::make_module(vm)
}

#[cfg(unix)]
mod platform {
    pub use libc::{fd_set, select, timeval, FD_ISSET, FD_SET, FD_SETSIZE, FD_ZERO};
    pub use std::os::unix::io::RawFd;

    pub fn check_err(x: i32) -> bool {
        x < 0
    }
}

#[allow(non_snake_case)]
#[cfg(windows)]
mod platform {
    use winapi::um::winsock2;
    pub use winsock2::{fd_set, select, timeval, FD_SETSIZE, SOCKET as RawFd};

    // based off winsock2.h: https://gist.github.com/piscisaureus/906386#file-winsock2-h-L128-L141

    pub unsafe fn FD_SET(fd: RawFd, set: *mut fd_set) {
        let mut slot = std::ptr::addr_of_mut!((*set).fd_array).cast::<RawFd>();
        let fd_count = (*set).fd_count;
        for _ in 0..fd_count {
            if *slot == fd {
                return;
            }
            slot = slot.add(1);
        }
        // slot == &fd_array[fd_count] at this point
        if fd_count < FD_SETSIZE as u32 {
            *slot = fd as RawFd;
            (*set).fd_count += 1;
        }
    }

    pub unsafe fn FD_ZERO(set: *mut fd_set) {
        (*set).fd_count = 0;
    }

    pub unsafe fn FD_ISSET(fd: RawFd, set: *mut fd_set) -> bool {
        use winapi::um::winsock2::__WSAFDIsSet;
        __WSAFDIsSet(fd as _, set) != 0
    }

    pub fn check_err(x: i32) -> bool {
        x == winsock2::SOCKET_ERROR
    }
}

pub use platform::timeval;
use platform::RawFd;

struct Selectable {
    obj: PyObjectRef,
    fno: RawFd,
}

impl TryFromObject for Selectable {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let fno = RawFd::try_from_borrowed_object(vm, &obj).or_else(|_| {
            let meth = vm.get_method_or_type_error(obj.clone(), "fileno", || {
                "select arg must be an int or object with a fileno() method".to_owned()
            })?;
            RawFd::try_from_object(vm, vm.invoke(&meth, ())?)
        })?;
        Ok(Selectable { obj, fno })
    }
}

// Keep it in a MaybeUninit, since on windows FD_ZERO doesn't actually zero the whole thing
#[repr(transparent)]
pub struct FdSet(mem::MaybeUninit<platform::fd_set>);

impl FdSet {
    pub fn new() -> FdSet {
        // it's just ints, and all the code that's actually
        // interacting with it is in C, so it's safe to zero
        let mut fdset = std::mem::MaybeUninit::zeroed();
        unsafe { platform::FD_ZERO(fdset.as_mut_ptr()) };
        FdSet(fdset)
    }

    pub fn insert(&mut self, fd: RawFd) {
        unsafe { platform::FD_SET(fd, self.0.as_mut_ptr()) };
    }

    pub fn contains(&mut self, fd: RawFd) -> bool {
        unsafe { platform::FD_ISSET(fd, self.0.as_mut_ptr()) }
    }

    pub fn clear(&mut self) {
        unsafe { platform::FD_ZERO(self.0.as_mut_ptr()) };
    }

    pub fn highest(&mut self) -> Option<RawFd> {
        (0..platform::FD_SETSIZE as RawFd)
            .rev()
            .find(|&i| self.contains(i))
    }
}

pub fn select(
    nfds: libc::c_int,
    readfds: &mut FdSet,
    writefds: &mut FdSet,
    errfds: &mut FdSet,
    timeout: Option<&mut timeval>,
) -> io::Result<i32> {
    let timeout = match timeout {
        Some(tv) => tv as *mut timeval,
        None => std::ptr::null_mut(),
    };
    let ret = unsafe {
        platform::select(
            nfds,
            readfds.0.as_mut_ptr(),
            writefds.0.as_mut_ptr(),
            errfds.0.as_mut_ptr(),
            timeout,
        )
    };
    if platform::check_err(ret) {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

fn sec_to_timeval(sec: f64) -> timeval {
    timeval {
        tv_sec: sec.trunc() as _,
        tv_usec: (sec.fract() * 1e6) as _,
    }
}

#[pymodule(name = "select")]
mod decl {
    use super::*;
    use crate::vm::{
        exceptions::IntoPyException, function::OptionalOption, stdlib::time, utils::Either,
        PyObjectRef, PyResult, VirtualMachine,
    };

    #[pyfunction]
    fn select(
        rlist: PyObjectRef,
        wlist: PyObjectRef,
        xlist: PyObjectRef,
        timeout: OptionalOption<Either<f64, isize>>,
        vm: &VirtualMachine,
    ) -> PyResult<(PyObjectRef, PyObjectRef, PyObjectRef)> {
        let mut timeout = timeout.flatten().map(|e| match e {
            Either::A(f) => f,
            Either::B(i) => i as f64,
        });
        if let Some(timeout) = timeout {
            if timeout < 0.0 {
                return Err(vm.new_value_error("timeout must be positive".to_owned()));
            }
        }
        let deadline = timeout.map(|s| time::time(vm).unwrap() + s);

        let seq2set = |list| -> PyResult<(Vec<Selectable>, FdSet)> {
            let v = vm.extract_elements::<Selectable>(list)?;
            let mut fds = FdSet::new();
            for fd in &v {
                fds.insert(fd.fno);
            }
            Ok((v, fds))
        };

        let (rlist, mut r) = seq2set(&rlist)?;
        let (wlist, mut w) = seq2set(&wlist)?;
        let (xlist, mut x) = seq2set(&xlist)?;

        if rlist.is_empty() && wlist.is_empty() && xlist.is_empty() {
            let empty = vm.ctx.new_list(vec![]);
            return Ok((empty.clone(), empty.clone(), empty));
        }

        let nfds = [&mut r, &mut w, &mut x]
            .iter_mut()
            .filter_map(|set| set.highest())
            .max()
            .map_or(0, |n| n + 1) as i32;

        loop {
            let mut tv = timeout.map(sec_to_timeval);
            let res = super::select(nfds, &mut r, &mut w, &mut x, tv.as_mut());

            match res {
                Ok(_) => break,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                Err(err) => return Err(err.into_pyexception(vm)),
            }

            vm.check_signals()?;

            if let Some(ref mut timeout) = timeout {
                *timeout = deadline.unwrap() - time::time(vm).unwrap();
                if *timeout < 0.0 {
                    r.clear();
                    w.clear();
                    x.clear();
                    break;
                }
                // retry select() if we haven't reached the deadline yet
            }
        }

        let set2list = |list: Vec<Selectable>, mut set: FdSet| {
            vm.ctx.new_list(
                list.into_iter()
                    .filter(|fd| set.contains(fd.fno))
                    .map(|fd| fd.obj)
                    .collect(),
            )
        };

        let rlist = set2list(rlist, r);
        let wlist = set2list(wlist, w);
        let xlist = set2list(xlist, x);

        Ok((rlist, wlist, xlist))
    }

    #[cfg(unix)]
    #[pyfunction]
    fn poll() -> poll::PyPoll {
        poll::PyPoll::default()
    }

    #[cfg(unix)]
    #[pyattr]
    use libc::{POLLERR, POLLHUP, POLLIN, POLLNVAL, POLLOUT, POLLPRI};

    #[cfg(unix)]
    pub(super) mod poll {
        use super::*;
        use crate::vm::{
            builtins::PyFloat, common::lock::PyMutex, function::OptionalArg, stdlib::io::Fildes,
            IntoPyObject, PyValue, TypeProtocol,
        };
        use libc::pollfd;
        use num_traits::ToPrimitive;
        use std::time;

        #[pyclass(module = "select", name = "poll")]
        #[derive(Default, Debug, PyValue)]
        pub struct PyPoll {
            // keep sorted
            fds: PyMutex<Vec<pollfd>>,
        }

        #[inline]
        fn search(fds: &[pollfd], fd: i32) -> Result<usize, usize> {
            fds.binary_search_by_key(&fd, |pfd| pfd.fd)
        }

        fn insert_fd(fds: &mut Vec<pollfd>, fd: i32, events: i16) {
            match search(fds, fd) {
                Ok(i) => fds[i].events = events,
                Err(i) => fds.insert(
                    i,
                    pollfd {
                        fd,
                        events,
                        revents: 0,
                    },
                ),
            }
        }

        fn get_fd_mut(fds: &mut [pollfd], fd: i32) -> Option<&mut pollfd> {
            search(fds, fd).ok().map(move |i| &mut fds[i])
        }

        fn remove_fd(fds: &mut Vec<pollfd>, fd: i32) -> Option<pollfd> {
            search(fds, fd).ok().map(|i| fds.remove(i))
        }

        const DEFAULT_EVENTS: i16 = libc::POLLIN | libc::POLLPRI | libc::POLLOUT;

        #[pyimpl]
        impl PyPoll {
            #[pymethod]
            fn register(&self, Fildes(fd): Fildes, eventmask: OptionalArg<u16>) {
                insert_fd(
                    &mut self.fds.lock(),
                    fd,
                    eventmask.map_or(DEFAULT_EVENTS, |e| e as i16),
                )
            }

            #[pymethod]
            fn modify(
                &self,
                Fildes(fd): Fildes,
                eventmask: u16,
                vm: &VirtualMachine,
            ) -> PyResult<()> {
                let mut fds = self.fds.lock();
                let pfd = get_fd_mut(&mut fds, fd).ok_or_else(|| {
                    io::Error::from_raw_os_error(libc::ENOENT).into_pyexception(vm)
                })?;
                pfd.events = eventmask as i16;
                Ok(())
            }

            #[pymethod]
            fn unregister(&self, Fildes(fd): Fildes, vm: &VirtualMachine) -> PyResult<()> {
                let removed = remove_fd(&mut self.fds.lock(), fd);
                removed
                    .map(drop)
                    .ok_or_else(|| vm.new_key_error(vm.ctx.new_int(fd)))
            }

            #[pymethod]
            fn poll(&self, timeout: OptionalOption, vm: &VirtualMachine) -> PyResult {
                let mut fds = self.fds.lock();
                let timeout_ms = match timeout.flatten() {
                    Some(ms) => {
                        let ms = if let Some(float) = ms.payload::<PyFloat>() {
                            float.to_f64().to_i32()
                        } else if let Some(int) = vm.to_index_opt(ms.clone()) {
                            int?.as_bigint().to_i32()
                        } else {
                            return Err(vm.new_type_error(format!(
                                "expected an int or float for duration, got {}",
                                ms.class()
                            )));
                        };
                        ms.ok_or_else(|| vm.new_value_error("value out of range".to_owned()))?
                    }
                    None => -1,
                };
                let timeout_ms = if timeout_ms < 0 { -1 } else { timeout_ms };
                let deadline = (timeout_ms >= 0)
                    .then(|| time::Instant::now() + time::Duration::from_millis(timeout_ms as u64));
                let mut poll_timeout = timeout_ms;
                loop {
                    let res = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, poll_timeout) };
                    let res = if res < 0 {
                        Err(io::Error::last_os_error())
                    } else {
                        Ok(())
                    };
                    match res {
                        Ok(()) => break,
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                            vm.check_signals()?;
                            if let Some(d) = deadline {
                                match d.checked_duration_since(time::Instant::now()) {
                                    Some(remaining) => poll_timeout = remaining.as_millis() as i32,
                                    // we've timed out
                                    None => break,
                                }
                            }
                        }
                        Err(e) => return Err(e.into_pyexception(vm)),
                    }
                }
                let list = fds
                    .iter()
                    .filter(|pfd| pfd.revents != 0)
                    .map(|pfd| (pfd.fd, pfd.revents & 0xfff).into_pyobject(vm))
                    .collect();
                Ok(vm.ctx.new_list(list))
            }
        }
    }
}
