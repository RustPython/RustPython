use std::ffi::OsString;
use std::fs::File;
use std::io::ErrorKind;
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Duration;

use crate::function::OptionalArg;
use crate::obj::objbytes::PyBytesRef;
use crate::obj::objlist::PyListRef;
use crate::obj::objstr::{self, PyStringRef};
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{Either, IntoPyObject, PyObjectRef, PyRef, PyResult, PyValue};
use crate::stdlib::io::io_open;
use crate::stdlib::os::{convert_io_error, raw_file_number, rust_file};
use crate::vm::VirtualMachine;

#[derive(Debug)]
struct Popen {
    process: RwLock<subprocess::Popen>,
    args: PyObjectRef,
}

// Remove once https://github.com/hniksic/rust-subprocess/issues/42 is resolved
#[cfg(windows)]
unsafe impl Sync for Popen {}

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

#[derive(FromArgs)]
struct PopenWaitArgs {
    #[pyarg(positional_or_keyword, default = "None")]
    timeout: Option<u64>,
}

impl IntoPyObject for subprocess::ExitStatus {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyResult {
        let status: i32 = match self {
            subprocess::ExitStatus::Exited(status) => status as i32,
            subprocess::ExitStatus::Signaled(status) => -i32::from(status),
            subprocess::ExitStatus::Other(status) => status as i32,
            _ => return Err(vm.new_os_error("Unknown exist status".to_owned())),
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

fn convert_to_file_io(file: &Option<File>, mode: &str, vm: &VirtualMachine) -> PyResult {
    match file {
        Some(ref stdin) => io_open(
            vm.new_int(raw_file_number(stdin.try_clone().unwrap())),
            Some(mode),
            Default::default(),
            vm,
        ),
        None => Ok(vm.get_none()),
    }
}

impl PopenRef {
    fn borrow_process(&self) -> RwLockReadGuard<'_, subprocess::Popen> {
        self.process.read().unwrap()
    }

    fn borrow_process_mut(&self) -> RwLockWriteGuard<'_, subprocess::Popen> {
        self.process.write().unwrap()
    }

    fn new(cls: PyClassRef, args: PopenArgs, vm: &VirtualMachine) -> PyResult<PopenRef> {
        let stdin = convert_redirection(args.stdin, vm)?;
        let stdout = convert_redirection(args.stdout, vm)?;
        let stderr = convert_redirection(args.stderr, vm)?;
        let command_list = match &args.args {
            Either::A(command) => vec![command.as_str().to_owned()],
            Either::B(command_list) => command_list
                .borrow_elements()
                .iter()
                .map(|x| objstr::clone_value(x))
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
            process: RwLock::new(process),
            args: args.args.into_object(),
        }
        .into_ref_with_type(vm, cls)
    }

    fn poll(self) -> Option<subprocess::ExitStatus> {
        self.borrow_process_mut().poll()
    }

    fn return_code(self) -> Option<subprocess::ExitStatus> {
        self.borrow_process().exit_status()
    }

    fn wait(self, args: PopenWaitArgs, vm: &VirtualMachine) -> PyResult<i64> {
        let timeout = match args.timeout {
            Some(timeout) => self
                .borrow_process_mut()
                .wait_timeout(Duration::new(timeout, 0)),
            None => self.borrow_process_mut().wait().map(Some),
        }
        .map_err(|s| vm.new_os_error(format!("Could not start program: {}", s)))?;
        if let Some(exit) = timeout {
            use subprocess::ExitStatus::*;
            Ok(match exit {
                Exited(i) => i.into(),
                Signaled(s) => -i64::from(s),
                _ => unreachable!("should not occur in normal operation"),
            })
        } else {
            let timeout_expired = vm.try_class("_subprocess", "TimeoutExpired")?;
            Err(vm.new_exception_msg(timeout_expired, "Timeout".to_owned()))
        }
    }

    fn stdin(self, vm: &VirtualMachine) -> PyResult {
        convert_to_file_io(&self.borrow_process().stdin, "wb", vm)
    }

    fn stdout(self, vm: &VirtualMachine) -> PyResult {
        convert_to_file_io(&self.borrow_process().stdout, "rb", vm)
    }

    fn stderr(self, vm: &VirtualMachine) -> PyResult {
        convert_to_file_io(&self.borrow_process().stderr, "rb", vm)
    }

    fn terminate(self, vm: &VirtualMachine) -> PyResult<()> {
        self.borrow_process_mut()
            .terminate()
            .map_err(|err| convert_io_error(vm, err))
    }

    fn kill(self, vm: &VirtualMachine) -> PyResult<()> {
        self.borrow_process_mut()
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
            OptionalArg::Present(ref bytes) => Some(bytes.get_value().to_vec()),
            OptionalArg::Missing => None,
        };
        let mut communicator = self.borrow_process_mut().communicate_start(bytes);
        if let OptionalArg::Present(timeout) = args.timeout {
            communicator = communicator.limit_time(Duration::new(timeout, 0));
        }
        communicator.read().map_err(|err| {
            if err.error.kind() == ErrorKind::TimedOut {
                let timeout_expired = vm.try_class("_subprocess", "TimeoutExpired").unwrap();
                vm.new_exception_msg(timeout_expired, "Timeout".to_owned())
            } else {
                convert_io_error(vm, err.error)
            }
        })
    }

    fn pid(self) -> Option<u32> {
        self.borrow_process().pid()
    }

    fn enter(self) -> Self {
        self
    }

    fn exit(
        self,
        _exception_type: PyObjectRef,
        _exception_value: PyObjectRef,
        _traceback: PyObjectRef,
    ) {
        let mut process = self.borrow_process_mut();
        process.stdout.take();
        process.stdin.take();
        process.stderr.take();
    }

    fn args(self) -> PyObjectRef {
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
    let called_process_error = ctx.new_class("CalledProcessError", subprocess_error.clone());

    let popen = py_class!(ctx, "Popen", ctx.object(), {
        (slot new) => PopenRef::new,
        "poll" => ctx.new_method(PopenRef::poll),
        "returncode" => ctx.new_readonly_getset("returncode", PopenRef::return_code),
        "wait" => ctx.new_method(PopenRef::wait),
        "stdin" => ctx.new_readonly_getset("stdin", PopenRef::stdin),
        "stdout" => ctx.new_readonly_getset("stdout", PopenRef::stdout),
        "stderr" => ctx.new_readonly_getset("stderr", PopenRef::stderr),
        "terminate" => ctx.new_method(PopenRef::terminate),
        "kill" => ctx.new_method(PopenRef::kill),
        "communicate" => ctx.new_method(PopenRef::communicate),
        "pid" => ctx.new_readonly_getset("pid", PopenRef::pid),
        "__enter__" => ctx.new_method(PopenRef::enter),
        "__exit__" => ctx.new_method(PopenRef::exit),
        "args" => ctx.new_readonly_getset("args", PopenRef::args),
    });

    py_module!(vm, "_subprocess", {
        "Popen" => popen,
        "SubprocessError" => subprocess_error,
        "TimeoutExpired" => timeout_expired,
        "CalledProcessError" => called_process_error,
        "PIPE" => ctx.new_int(-1),
        "STDOUT" => ctx.new_int(-2),
        "DEVNULL" => ctx.new_int(-3),
    })
}
