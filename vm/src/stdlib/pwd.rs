use pwd::Passwd;

use crate::obj::objstr::PyStringRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

impl PyValue for Passwd {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("pwd", "struct_passwd")
    }
}

type PasswdRef = PyRef<Passwd>;

impl PasswdRef {
    fn pw_name(self) -> String {
        self.name.clone()
    }

    fn pw_passwd(self) -> Option<String> {
        self.passwd.clone()
    }

    fn pw_uid(self) -> u32 {
        self.uid
    }

    fn pw_gid(self) -> u32 {
        self.gid
    }

    fn pw_gecos(self) -> Option<String> {
        self.gecos.clone()
    }

    fn pw_dir(self) -> String {
        self.dir.clone()
    }

    fn pw_shell(self) -> String {
        self.shell.clone()
    }
}

fn pwd_getpwnam(name: PyStringRef, vm: &VirtualMachine) -> PyResult<Passwd> {
    match Passwd::from_name(name.as_str()) {
        Ok(Some(passwd)) => Ok(passwd),
        _ => {
            let name_repr = vm.to_repr(name.as_object())?;
            let message = vm.new_str(format!("getpwnam(): name not found: {}", name_repr));
            Err(vm.new_key_error(message))
        }
    }
}

fn pwd_getpwuid(uid: u32, vm: &VirtualMachine) -> PyResult<Passwd> {
    match Passwd::from_uid(uid) {
        Some(passwd) => Ok(passwd),
        _ => {
            let message = vm.new_str(format!("getpwuid(): uid not found: {}", uid));
            Err(vm.new_key_error(message))
        }
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let passwd_type = py_class!(ctx, "struct_passwd", ctx.object(), {
        "pw_name" => ctx.new_readonly_getset("pw_name", PasswdRef::pw_name),
        "pw_passwd" => ctx.new_readonly_getset("pw_passwd", PasswdRef::pw_passwd),
        "pw_uid" => ctx.new_readonly_getset("pw_uid", PasswdRef::pw_uid),
        "pw_gid" => ctx.new_readonly_getset("pw_gid", PasswdRef::pw_gid),
        "pw_gecos" => ctx.new_readonly_getset("pw_gecos", PasswdRef::pw_gecos),
        "pw_dir" => ctx.new_readonly_getset("pw_dir", PasswdRef::pw_dir),
        "pw_shell" => ctx.new_readonly_getset("pw_shell", PasswdRef::pw_shell),
    });

    py_module!(vm, "pwd", {
        "struct_passwd" => passwd_type,
        "getpwnam" => ctx.new_function(pwd_getpwnam),
        "getpwuid" => ctx.new_function(pwd_getpwuid),
    })
}
