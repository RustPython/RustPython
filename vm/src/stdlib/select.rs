use crate::pyobject::{PyObjectRef, PyResult, TryFromObject};
use crate::vm::VirtualMachine;
use std::io;

pub(crate) fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    #[cfg(windows)]
    {
        let _ = unsafe { winapi::um::winsock2::WSAStartup(0x0101, &mut std::mem::zeroed()) };
    }

    decl::make_module(vm)
}

#[cfg(unix)]
mod platform {
    pub use libc::{fd_set, select, timeval, FD_ISSET, FD_SET, FD_SETSIZE, FD_ZERO};
    pub use std::os::unix::io::RawFd;
}

#[allow(non_snake_case)]
#[cfg(windows)]
mod platform {
    pub use winapi::um::winsock2::{fd_set, select, timeval, FD_SETSIZE, SOCKET as RawFd};

    // from winsock2.h: https://gist.github.com/piscisaureus/906386#file-winsock2-h-L128-L141

    pub unsafe fn FD_SET(fd: RawFd, set: *mut fd_set) {
        let mut i = 0;
        for idx in 0..(*set).fd_count as usize {
            i = idx;
            if (*set).fd_array[i] == fd {
                break;
            }
        }
        if i == (*set).fd_count as usize {
            if (*set).fd_count < FD_SETSIZE as u32 {
                (*set).fd_array[i] = fd as _;
                (*set).fd_count += 1;
            }
        }
    }

    pub unsafe fn FD_ZERO(set: *mut fd_set) {
        (*set).fd_count = 0;
    }

    pub unsafe fn FD_ISSET(fd: RawFd, set: *mut fd_set) -> bool {
        use winapi::um::winsock2::__WSAFDIsSet;
        __WSAFDIsSet(fd as _, set) != 0
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
        let fno = RawFd::try_from_object(vm, obj.clone()).or_else(|_| {
            let meth = vm.get_method_or_type_error(obj.clone(), "fileno", || {
                "select arg must be an int or object with a fileno() method".to_owned()
            })?;
            RawFd::try_from_object(vm, vm.invoke(&meth, ())?)
        })?;
        Ok(Selectable { obj, fno })
    }
}

#[repr(transparent)]
pub struct FdSet(platform::fd_set);

impl FdSet {
    pub fn new() -> FdSet {
        // it's just ints, and all the code that's actually
        // interacting with it is in C, so it's safe to zero
        let mut fdset = std::mem::MaybeUninit::zeroed();
        unsafe { platform::FD_ZERO(fdset.as_mut_ptr()) };
        FdSet(unsafe { fdset.assume_init() })
    }

    pub fn insert(&mut self, fd: RawFd) {
        unsafe { platform::FD_SET(fd, &mut self.0) };
    }

    pub fn contains(&mut self, fd: RawFd) -> bool {
        unsafe { platform::FD_ISSET(fd, &mut self.0) }
    }

    pub fn clear(&mut self) {
        unsafe { platform::FD_ZERO(&mut self.0) };
    }

    pub fn highest(&mut self) -> Option<RawFd> {
        for i in (0..platform::FD_SETSIZE as RawFd).rev() {
            if self.contains(i) {
                return Some(i);
            }
        }

        None
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
            &mut readfds.0,
            &mut writefds.0,
            &mut errfds.0,
            timeout,
        )
    };
    if ret < 0 {
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
    use super::super::time_module;
    use super::*;
    use crate::exceptions::IntoPyException;
    use crate::function::OptionalOption;
    use crate::pyobject::{Either, PyObjectRef, PyResult};
    use crate::vm::VirtualMachine;

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
        let deadline = timeout.map(|s| time_module::get_time() + s);

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
                *timeout = deadline.unwrap() - time_module::get_time();
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
}
