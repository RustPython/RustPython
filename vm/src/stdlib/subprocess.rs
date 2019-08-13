use std::cell::RefCell;
use std::ffi::OsString;
use std::fs::File;
use std::time::Duration;

use subprocess;

use crate::function::OptionalArg;
use crate::obj::objbytes::PyBytesRef;
use crate::obj::objlist::PyListRef;
use crate::obj::objsequence;
use crate::obj::objstr::{self, PyStringRef};
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{Either, IntoPyObject, PyObjectRef, PyRef, PyResult, PyValue};
use crate::stdlib::io::io_open;
use crate::stdlib::os::{convert_io_error, raw_file_number, rust_file};
use crate::vm::VirtualMachine;

#[derive(Debug)]
struct Popen {
    process: RefCell<subprocess::Popen>,
    args: PyObjectRef,
}

impl PyValue for Popen {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("_subprocess", "Popen")
    }
}

type PopenRef = PyRef<Popen>;

#[derive(FromArgs)]
#[allow(dead_code)]
struct PopenArgs {
    #[pyarg(positional_only)]
    args: Either<PyStringRef, PyListRef>,
    #[pyarg(positional_or_keyword, default = "None")]
    stdin: Option<i64>,
    #[pyarg(positional_or_keyword, default = "None")]
    stdout: Option<i64>,
    #[pyarg(positional_or_keyword, default = "None")]
    stderr: Option<i64>,
    #[pyarg(positional_or_keyword, default = "None")]
    close_fds: Option<bool>, // TODO: use these unused options
    #[pyarg(positional_or_keyword, default = "None")]
    cwd: Option<PyStringRef>,
    #[pyarg(positional_or_keyword, default = "None")]
    start_new_session: Option<bool>,
}

impl IntoPyObject for subprocess::ExitStatus {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyResult {
        let status: i32 = match self {
            subprocess::ExitStatus::Exited(status) => status as i32,
            subprocess::ExitStatus::Signaled(status) => -i32::from(status),
            subprocess::ExitStatus::Other(status) => status as i32,
            _ => return Err(vm.new_os_error("Unknown exist status".to_string())),
        };
        Ok(vm.new_int(status))
    }
}

#[cfg(windows)]
const NULL_DEVICE: &str = "nul";
#[cfg(unix)]
const NULL_DEVICE: &str = "/dev/null";

fn convert_redirection(arg: Option<i64>, vm: &VirtualMachine) -> PyResult<subprocess::Redirection> {
    match arg {
        Some(fd) => match fd {
            -1 => Ok(subprocess::Redirection::Pipe),
            -2 => Ok(subprocess::Redirection::Merge),
            -3 => Ok(subprocess::Redirection::File(
                File::open(NULL_DEVICE).unwrap(),
            )),
            fd => {
                if fd < 0 {
                    Err(vm.new_value_error(format!("Invalid fd: {}", fd)))
                } else {
                    Ok(subprocess::Redirection::File(rust_file(fd)))
                }
            }
        },
        None => Ok(subprocess::Redirection::None),
    }
}

fn convert_to_file_io(file: &Option<File>, mode: String, vm: &VirtualMachine) -> PyResult {
    match file {
        Some(ref stdin) => io_open(
            vm,
            vec![
                vm.new_int(raw_file_number(stdin.try_clone().unwrap())),
                vm.new_str(mode),
            ]
            .into(),
        ),
        None => Ok(vm.get_none()),
    }
}

impl PopenRef {
    fn new(cls: PyClassRef, args: PopenArgs, vm: &VirtualMachine) -> PyResult<PopenRef> {
        let stdin = convert_redirection(args.stdin, vm)?;
        let stdout = convert_redirection(args.stdout, vm)?;
        let stderr = convert_redirection(args.stderr, vm)?;
        let command_list = match &args.args {
            Either::A(command) => vec![command.as_str().to_string()],
            Either::B(command_list) => objsequence::get_elements_list(command_list.as_object())
                .iter()
                .map(|x| objstr::get_value(x))
                .collect(),
        };
        let cwd = args.cwd.map(|x| OsString::from(x.as_str()));

        let process = subprocess::Popen::create(
            &command_list,
            subprocess::PopenConfig {
                stdin,
                stdout,
                stderr,
                cwd,
                ..Default::default()
            },
        )
        .map_err(|s| vm.new_os_error(format!("Could not start program: {}", s)))?;

        Popen {
            process: RefCell::new(process),
            args: args.args.into_object(),
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
            None => self.process.borrow_mut().wait().map(Some),
        }
        .map_err(|s| vm.new_os_error(format!("Could not start program: {}", s)))?;
        if timeout.is_none() {
            let timeout_expired = vm.try_class("_subprocess", "TimeoutExpired")?;
            Err(vm.new_exception(timeout_expired, "Timeout".to_string()))
        } else {
            Ok(())
        }
    }

    fn stdin(self, vm: &VirtualMachine) -> PyResult {
        convert_to_file_io(&self.process.borrow().stdin, "wb".to_string(), vm)
    }

    fn stdout(self, vm: &VirtualMachine) -> PyResult {
        convert_to_file_io(&self.process.borrow().stdout, "rb".to_string(), vm)
    }

    fn stderr(self, vm: &VirtualMachine) -> PyResult {
        convert_to_file_io(&self.process.borrow().stderr, "rb".to_string(), vm)
    }

    fn terminate(self, vm: &VirtualMachine) -> PyResult<()> {
        self.process
            .borrow_mut()
            .terminate()
            .map_err(|err| convert_io_error(vm, err))
    }

    fn kill(self, vm: &VirtualMachine) -> PyResult<()> {
        self.process
            .borrow_mut()
            .kill()
            .map_err(|err| convert_io_error(vm, err))
    }

    #[allow(clippy::type_complexity)]
    fn communicate(
        self,
        args: PopenCommunicateArgs,
        vm: &VirtualMachine,
    ) -> PyResult<(Option<Vec<u8>>, Option<Vec<u8>>)> {
        let bytes = match args.input {
            OptionalArg::Present(ref bytes) => Some(bytes.get_value()),
            OptionalArg::Missing => None,
        };
        self.process
            .borrow_mut()
            .communicate_bytes(bytes)
            .map_err(|err| convert_io_error(vm, err))
    }

    fn pid(self, _vm: &VirtualMachine) -> Option<u32> {
        self.process.borrow().pid()
    }

    fn enter(self, _vm: &VirtualMachine) -> Self {
        self
    }

    fn exit(
        self,
        _exception_type: PyObjectRef,
        _exception_value: PyObjectRef,
        _traceback: PyObjectRef,
        _vm: &VirtualMachine,
    ) {
        let mut process = self.process.borrow_mut();
        process.stdout.take();
        process.stdin.take();
        process.stderr.take();
    }

    fn args(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.args.clone()
    }
}

#[derive(FromArgs)]
#[allow(dead_code)]
struct PopenCommunicateArgs {
    #[pyarg(positional_or_keyword, optional = true)]
    input: OptionalArg<PyBytesRef>,
    #[pyarg(positional_or_keyword, optional = true)]
    timeout: OptionalArg<u64>,
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let subprocess_error = ctx.new_class("SubprocessError", ctx.exceptions.exception_type.clone());
    let timeout_expired = ctx.new_class("TimeoutExpired", subprocess_error.clone());

    let popen = py_class!(ctx, "Popen", ctx.object(), {
        (slot new) => PopenRef::new,
        "poll" => ctx.new_rustfunc(PopenRef::poll),
        "returncode" => ctx.new_property(PopenRef::return_code),
        "wait" => ctx.new_rustfunc(PopenRef::wait),
        "stdin" => ctx.new_property(PopenRef::stdin),
        "stdout" => ctx.new_property(PopenRef::stdout),
        "stderr" => ctx.new_property(PopenRef::stderr),
        "terminate" => ctx.new_rustfunc(PopenRef::terminate),
        "kill" => ctx.new_rustfunc(PopenRef::kill),
        "communicate" => ctx.new_rustfunc(PopenRef::communicate),
        "pid" => ctx.new_property(PopenRef::pid),
        "__enter__" => ctx.new_rustfunc(PopenRef::enter),
        "__exit__" => ctx.new_rustfunc(PopenRef::exit),
        "args" => ctx.new_property(PopenRef::args),
    });

    py_module!(vm, "_subprocess", {
        "Popen" => popen,
        "SubprocessError" => subprocess_error,
        "TimeoutExpired" => timeout_expired,
        "PIPE" => ctx.new_int(-1),
        "STDOUT" => ctx.new_int(-2),
        "DEVNULL" => ctx.new_int(-3),
    })
}
