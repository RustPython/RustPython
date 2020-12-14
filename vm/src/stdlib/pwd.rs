use crate::builtins::int::PyIntRef;
use crate::builtins::pystr::PyStrRef;
use crate::exceptions::IntoPyException;
use crate::pyobject::{BorrowValue, PyClassImpl, PyObjectRef, PyResult, PyStructSequence};
use crate::vm::VirtualMachine;
use std::convert::TryFrom;
use std::ptr::NonNull;

use nix::unistd::{self, User};

#[pyclass(module = "pwd", name = "struct_passwd")]
#[derive(PyStructSequence)]
struct Passwd {
    pw_name: String,
    pw_passwd: String,
    pw_uid: u32,
    pw_gid: u32,
    pw_gecos: String,
    pw_dir: String,
    pw_shell: String,
}
#[pyimpl(with(PyStructSequence))]
impl Passwd {}

impl From<User> for Passwd {
    fn from(user: User) -> Self {
        // this is just a pain...
        let cstr_lossy = |s: std::ffi::CString| {
            s.into_string()
                .unwrap_or_else(|e| e.into_cstring().to_string_lossy().into_owned())
        };
        let pathbuf_lossy = |p: std::path::PathBuf| {
            p.into_os_string()
                .into_string()
                .unwrap_or_else(|s| s.to_string_lossy().into_owned())
        };
        Passwd {
            pw_name: user.name,
            pw_passwd: cstr_lossy(user.passwd),
            pw_uid: user.uid.as_raw(),
            pw_gid: user.gid.as_raw(),
            pw_gecos: cstr_lossy(user.gecos),
            pw_dir: pathbuf_lossy(user.dir),
            pw_shell: pathbuf_lossy(user.shell),
        }
    }
}

fn pwd_getpwnam(name: PyStrRef, vm: &VirtualMachine) -> PyResult {
    match User::from_name(name.borrow_value()).map_err(|err| err.into_pyexception(vm))? {
        Some(user) => Ok(Passwd::from(user).into_struct_sequence(vm)?.into_object()),
        None => {
            let name_repr = vm.to_repr(name.as_object())?;
            let message = vm
                .ctx
                .new_str(format!("getpwnam(): name not found: {}", name_repr));
            Err(vm.new_key_error(message))
        }
    }
}

fn pwd_getpwuid(uid: PyIntRef, vm: &VirtualMachine) -> PyResult {
    let uid_t = libc::uid_t::try_from(uid.borrow_value()).map(unistd::Uid::from_raw);
    let user = match uid_t {
        Ok(uid) => User::from_uid(uid).map_err(|err| err.into_pyexception(vm))?,
        Err(_) => None,
    };
    match user {
        Some(user) => Ok(Passwd::from(user).into_struct_sequence(vm)?.into_object()),
        None => {
            let message = vm
                .ctx
                .new_str(format!("getpwuid(): uid not found: {}", uid.borrow_value()));
            Err(vm.new_key_error(message))
        }
    }
}

// TODO: maybe merge this functionality into nix?
fn pwd_getpwall(vm: &VirtualMachine) -> PyResult {
    // setpwent, getpwent, etc are not thread safe. Could use fgetpwent_r, but this is easier
    static GETPWALL: parking_lot::Mutex<()> = parking_lot::const_mutex(());
    let _guard = GETPWALL.lock();
    let mut list = Vec::new();

    unsafe { libc::setpwent() };
    while let Some(ptr) = NonNull::new(unsafe { libc::getpwent() }) {
        let user = User::from(unsafe { ptr.as_ref() });
        let passwd = Passwd::from(user).into_struct_sequence(vm)?.into_object();
        list.push(passwd);
    }
    unsafe { libc::endpwent() };

    Ok(vm.ctx.new_list(list))
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    py_module!(vm, "pwd", {
        "struct_passwd" => Passwd::make_class(ctx),
        "getpwnam" => named_function!(ctx, pwd, getpwnam),
        "getpwuid" => named_function!(ctx, pwd, getpwuid),
        "getpwall" => named_function!(ctx, pwd, getpwall),
    })
}
