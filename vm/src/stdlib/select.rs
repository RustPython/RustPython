use crate::function::OptionalOption;
use crate::pyobject::{Either, PyObjectRef, PyResult, TryFromObject};
use crate::vm::VirtualMachine;
use std::{io, mem};

#[cfg(unix)]
type RawFd = i32;

#[cfg(unix)]
use libc::{fd_set, select, timeval, FD_ISSET, FD_SET, FD_SETSIZE, FD_ZERO};

#[cfg(windows)]
use winapi::um::winsock2::{fd_set, select, timeval, WSAStartup, FD_SETSIZE, SOCKET as RawFd};

// from winsock2.h: https://gist.github.com/piscisaureus/906386#file-winsock2-h-L128-L141
#[cfg(windows)]
#[allow(non_snake_case)]
mod fd_ops {
    use winapi::um::winsock2::{__WSAFDIsSet, fd_set, FD_SETSIZE, SOCKET};

    pub unsafe fn FD_SET(fd: SOCKET, set: *mut fd_set) {
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

    pub unsafe fn FD_ISSET(fd: SOCKET, set: *mut fd_set) -> bool {
        __WSAFDIsSet(fd as _, set) != 0
    }
}
#[cfg(windows)]
use fd_ops::*;

struct Selectable {
    obj: PyObjectRef,
    fno: RawFd,
}

impl TryFromObject for Selectable {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let fno = RawFd::try_from_object(vm, obj.clone()).or_else(|_| {
            let meth = vm.get_method_or_type_error(obj.clone(), "fileno", || {
                "select arg must be an int or object with a fileno() method".to_string()
            })?;
            RawFd::try_from_object(vm, vm.invoke(&meth, vec![])?)
        })?;
        Ok(Selectable { obj, fno })
    }
}

#[repr(C)]
struct FdSet(fd_set);

impl FdSet {
    pub fn new() -> FdSet {
        // it's just ints, and all the code that's actually
        // interacting with it is in C, so it's safe to zero
        let mut fdset = mem::MaybeUninit::zeroed();
        unsafe { FD_ZERO(fdset.as_mut_ptr()) };
        FdSet(unsafe { fdset.assume_init() })
    }

    pub fn insert(&mut self, fd: RawFd) {
        unsafe { FD_SET(fd, &mut self.0) };
    }

    pub fn contains(&mut self, fd: RawFd) -> bool {
        unsafe { FD_ISSET(fd, &mut self.0) }
    }

    pub fn clear(&mut self) {
        unsafe { FD_ZERO(&mut self.0) };
    }

    pub fn highest(&mut self) -> Option<RawFd> {
        for i in (0..FD_SETSIZE as RawFd).rev() {
            if self.contains(i) {
                return Some(i);
            }
        }

        None
    }
}

fn sec_to_timeval(sec: f64) -> timeval {
    timeval {
        tv_sec: sec.trunc() as _,
        tv_usec: (sec.fract() * 1e6) as _,
    }
}

fn select_select(
    rlist: PyObjectRef,
    wlist: PyObjectRef,
    xlist: PyObjectRef,
    timeout: OptionalOption<Either<f64, isize>>,
    vm: &VirtualMachine,
) -> PyResult<(PyObjectRef, PyObjectRef, PyObjectRef)> {
    let mut timeout = timeout.flat_option().map(|e| match e {
        Either::A(f) => f,
        Either::B(i) => i as f64,
    });
    if let Some(timeout) = timeout {
        if timeout < 0.0 {
            return Err(vm.new_value_error("timeout must be positive".to_string()));
        }
    }
    let deadline = timeout.map(|s| super::time_module::get_time() + s);

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

    let nfds = [&mut r, &mut w, &mut x]
        .iter_mut()
        .filter_map(|set| set.highest())
        .max()
        .map_or(0, |n| n + 1) as i32;

    let (select_res, err) = loop {
        let mut tv = timeout.map(sec_to_timeval);
        let timeout_ptr = match tv {
            Some(ref mut tv) => tv as *mut _,
            None => std::ptr::null_mut(),
        };
        let res = unsafe { select(nfds, &mut r.0, &mut w.0, &mut x.0, timeout_ptr) };

        let err = io::Error::last_os_error();

        if res >= 0 || err.kind() != io::ErrorKind::Interrupted {
            break (res, err);
        }

        vm.check_signals()?;

        if let Some(ref mut timeout) = timeout {
            *timeout = deadline.unwrap() - super::time_module::get_time();
            if *timeout < 0.0 {
                r.clear();
                w.clear();
                x.clear();
                break (0, err);
            }
            // retry select() if we haven't reached the deadline yet
        }
    };

    if select_res < 0 {
        return Err(super::os::convert_io_error(vm, err));
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

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    #[cfg(windows)]
    {
        let _ = unsafe { WSAStartup(0x0101, &mut mem::zeroed()) };
    }

    py_module!(vm, "select", {
        "select" => vm.ctx.new_rustfunc(select_select),
    })
}
