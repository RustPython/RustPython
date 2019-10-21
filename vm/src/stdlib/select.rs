use crate::function::OptionalOption;
use crate::pyobject::{PyObjectRef, PyResult};
use crate::vm::VirtualMachine;

#[cfg(unix)]
fn select_select(
    rlist: PyObjectRef,
    wlist: PyObjectRef,
    xlist: PyObjectRef,
    timeout: OptionalOption<f64>,
    vm: &VirtualMachine,
) -> PyResult<PyObjectRef> {
    use nix::sys::select;
    use nix::sys::time::{TimeVal, TimeValLike};
    use std::os::unix::io::RawFd;

    let seq2set = |list| -> PyResult<(Vec<RawFd>, select::FdSet)> {
        let v = vm.extract_elements(list)?;
        let mut fds = select::FdSet::new();
        for fd in &v {
            fds.insert(*fd);
        }
        Ok((v, fds))
    };

    let (rlist, mut r) = seq2set(&rlist)?;
    let (wlist, mut w) = seq2set(&wlist)?;
    let (xlist, mut x) = seq2set(&xlist)?;

    let mut timeout = timeout
        .flat_option()
        .map(|to| TimeVal::nanoseconds((to * 1e9) as i64));

    select::select(
        None,
        Some(&mut r),
        Some(&mut w),
        Some(&mut x),
        timeout.as_mut(),
    )
    .map_err(|err| super::os::convert_nix_error(vm, err))?;

    let set2list = |list: Vec<RawFd>, mut set: select::FdSet| -> PyObjectRef {
        vm.ctx.new_list(
            list.into_iter()
                .filter(|fd| set.contains(*fd))
                .map(|fd| vm.new_int(fd))
                .collect(),
        )
    };

    let rlist = set2list(rlist, r);
    let wlist = set2list(wlist, w);
    let xlist = set2list(xlist, x);

    Ok(vm.ctx.new_tuple(vec![rlist, wlist, xlist]))
}

#[cfg(not(unix))]
fn select_select(
    _rlist: PyObjectRef,
    _wlist: PyObjectRef,
    _xlist: PyObjectRef,
    _timeout: OptionalOption<f64>,
    _vm: &VirtualMachine,
) {
    // TODO: select.select on windows
    unimplemented!("select.select")
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    py_module!(vm, "select", {
        "select" => vm.ctx.new_rustfunc(select_select),
    })
}
