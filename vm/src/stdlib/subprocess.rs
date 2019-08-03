use std::cell::RefCell;
use std::time::Duration;

use subprocess;

use crate::function::OptionalArg;
use crate::obj::objlist::PyListRef;
use crate::obj::objsequence;
use crate::obj::objstr::{self, PyStringRef};
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{Either, IntoPyObject, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

#[derive(Debug)]
struct Popen {
    process: RefCell<subprocess::Popen>,
}

impl PyValue for Popen {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("subprocess", "Popen")
    }
}

type PopenRef = PyRef<Popen>;

#[derive(FromArgs)]
struct PopenArgs {
    #[pyarg(positional_only)]
    args: Either<PyStringRef, PyListRef>,
}

impl IntoPyObject for subprocess::ExitStatus {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyResult {
        let status: i32 = match self {
            subprocess::ExitStatus::Exited(status) => status as i32,
            subprocess::ExitStatus::Signaled(status) => -(status as i32),
            subprocess::ExitStatus::Other(status) => status as i32,
            _ => return Err(vm.new_os_error("Unknown exist status".to_string())),
        };
        Ok(vm.new_int(status))
    }
}

impl PopenRef {
    fn new(cls: PyClassRef, args: PopenArgs, vm: &VirtualMachine) -> PyResult<PopenRef> {
        let command_list = match args.args {
            Either::A(command) => vec![command.as_str().to_string()],
            Either::B(command_list) => objsequence::get_elements_list(command_list.as_object())
                .iter()
                .map(|x| objstr::get_value(x))
                .collect(),
        };

        let process = subprocess::Popen::create(&command_list, subprocess::PopenConfig::default())
            .map_err(|s| vm.new_os_error(format!("Could not start program: {}", s)))?;

        Popen {
            process: RefCell::new(process),
        }
        .into_ref_with_type(vm, cls)
    }

    fn poll(self, _vm: &VirtualMachine) -> Option<subprocess::ExitStatus> {
        self.process.borrow_mut().poll()
    }

    fn return_code(self, _vm: &VirtualMachine) -> Option<subprocess::ExitStatus> {
        self.process.borrow().exit_status()
    }

    fn wait(self, timeout: OptionalArg<u64>, vm: &VirtualMachine) -> PyResult<()> {
        let timeout = match timeout.into_option() {
            Some(timeout) => self
                .process
                .borrow_mut()
                .wait_timeout(Duration::new(timeout, 0)),
            None => self.process.borrow_mut().wait().map(|x| Some(x)),
        }
        .map_err(|s| vm.new_os_error(format!("Could not start program: {}", s)))?;
        if timeout.is_none() {
            let timeout_expired = vm.class("subprocess", "TimeoutExpired");
            Err(vm.new_exception(timeout_expired, "Timeout".to_string()))
        } else {
            Ok(())
        }
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let subprocess_error = ctx.new_class("SubprocessError", ctx.exceptions.exception_type.clone());
    let timeout_expired = ctx.new_class("TimeoutExpired", subprocess_error.clone());

    let popen = py_class!(ctx, "Popen", ctx.object(), {
        "__new__" => ctx.new_rustfunc(PopenRef::new),
        "poll" => ctx.new_rustfunc(PopenRef::poll),
        "returncode" => ctx.new_property(PopenRef::return_code),
        "wait" => ctx.new_rustfunc(PopenRef::wait)
    });

    let module = py_module!(vm, "subprocess", {
        "Popen" => popen,
        "SubprocessError" => subprocess_error,
        "TimeoutExpired" => timeout_expired,
    });

    module
}
