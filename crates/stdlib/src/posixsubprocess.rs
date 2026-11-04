// spell-checker:disable

use crate::vm::{
    AsObject, PyObject, PyObjectRef, PyResult, TryFromObject, VirtualMachine,
    builtins::{PyBytes, PyBytesRef, PyInt, PyList, PyTuple, PyTupleRef},
    function::{ArgIntoBool, FsPath},
    stdlib::posix::module::{RawGid, RawUid},
};
use rustpython_host_env::posix as host_posix;
use std::io::prelude::*;

use alloc::ffi::CString;
use core::{convert::Infallible as Never, ffi::CStr, marker::PhantomData, ops::Deref};

pub(crate) use _posixsubprocess::module_def;

#[pymodule]
mod _posixsubprocess {
    use super::*;
    use crate::vm::convert::IntoPyException;

    #[pyfunction]
    fn fork_exec(args: ForkExecArgs, vm: &VirtualMachine) -> PyResult<host_posix::pid_t> {
        // Clinic converts scalar controls before entering parent-side preparation.
        let close_fds = args.close_fds == ArgIntoBool::TRUE;
        let restore_signals = args.restore_signals == ArgIntoBool::TRUE;
        let call_setsid = args.call_setsid == ArgIntoBool::TRUE;
        let controls = ChildControls {
            close_fds,
            restore_signals,
            call_setsid,
        };
        if args.preexec_fn.is_some() {
            if vm
                .state
                .finalizing
                .load(core::sync::atomic::Ordering::Acquire)
            {
                return Err(vm.new_python_finalization_error(
                    "preexec_fn not supported at interpreter shutdown".to_owned(),
                ));
            }
            if !vm.state.is_main_interpreter() {
                return Err(vm.new_runtime_error("preexec_fn not supported within subinterpreters"));
            }
        }
        if close_fds && args.errpipe_write < 3 {
            return Err(vm.new_value_error("errpipe_write must be >= 3"));
        }
        check_pass_fds(&args.pass_fds, vm)?;

        let prepared = PreparedArgs::new(&args, vm)?;
        let argv = prepared.argv.as_ref().map(CharPtrVec::from_iter);
        let envp = prepared.env.as_ref().map(CharPtrVec::from_iter);
        let procargs = ProcArgs {
            argv: argv.as_deref(),
            envp: envp.as_deref(),
        };
        // The hook observes the original objects, after all native data is ready.
        vm.audit("_posixsubprocess.fork_exec", || {
            (
                args.executable_list.clone(),
                args.args.clone(),
                args.env.clone(),
            )
        })?;
        match host_posix::fork().map_err(|err| err.into_pyexception(vm))? {
            0 => exec(&args, &prepared, controls, procargs, vm),
            child => Ok(child),
        }
    }
}

macro_rules! gen_args {
    ($($field:ident: $t:ty),*$(,)?) => {
        #[derive(FromArgs)]
        struct ForkExecArgs {
            $(#[pyarg(positional)] $field: $t,)*
        }
    };
}

gen_args! {
    args: PyObjectRef,
    executable_list: PyObjectRef,
    close_fds: ArgIntoBool,
    pass_fds: PyTupleRef,
    cwd: Option<PyObjectRef>,
    env: Option<PyObjectRef>,
    p2cread: i32,
    p2cwrite: i32,
    c2pread: i32,
    c2pwrite: i32,
    errread: i32,
    errwrite: i32,
    errpipe_read: i32,
    errpipe_write: i32,
    restore_signals: ArgIntoBool,
    call_setsid: ArgIntoBool,
    pgid_to_set: host_posix::pid_t,
    gid: Option<PyObjectRef>,
    extra_groups: Option<PyObjectRef>,
    uid: Option<PyObjectRef>,
    child_umask: i32,
    preexec_fn: Option<PyObjectRef>,
}

struct ChildControls {
    close_fds: bool,
    restore_signals: bool,
    call_setsid: bool,
}

// Everything that can call Python or allocate belongs to the parent, not exec().
struct PreparedArgs {
    executable_list: Vec<CString>,
    argv: Option<Vec<CString>>,
    env: Option<Vec<CString>>,
    cwd: Option<CString>,
    // FSConverter may return a bytes subclass whose destructor must wait.
    _cwd_bytes: Option<PyBytesRef>,
    extra_groups: Option<Vec<u32>>,
    gid: Option<RawGid>,
    uid: Option<RawUid>,
    pass_fds: Vec<i32>,
}

impl PreparedArgs {
    fn new(args: &ForkExecArgs, vm: &VirtualMachine) -> PyResult<Self> {
        let executable_list = bytes_sequence(&args.executable_list, vm)?;
        let argv = if vm.is_none(&args.args) {
            None
        } else {
            Some(prepare_argv(&args.args, vm)?)
        };
        let env = args
            .env
            .as_ref()
            .map(|env| bytes_sequence(env, vm))
            .transpose()?;
        let cwd_bytes = args
            .cwd
            .as_ref()
            .map(|cwd| filesystem_bytes(cwd, vm))
            .transpose()?;
        let cwd = cwd_bytes
            .as_ref()
            .map(|bytes| CString::new(bytes.as_bytes()).expect("FSConverter checked for NUL"));
        let extra_groups = args
            .extra_groups
            .as_ref()
            .map(|groups| prepare_groups(groups, vm))
            .transpose()?;
        let gid = args
            .gid
            .clone()
            .map(|gid| RawGid::try_from_object(vm, gid))
            .transpose()?;
        let uid = args
            .uid
            .clone()
            .map(|uid| RawUid::try_from_object(vm, uid))
            .transpose()?;
        // Repeat conversion here, after all Python callbacks, as CPython does.
        let pass_fds = args
            .pass_fds
            .as_slice()
            .iter()
            .map(|fd| fd.try_to_ref::<PyInt>(vm)?.try_to_primitive::<i32>(vm))
            .collect::<PyResult<Vec<_>>>()?;
        Ok(Self {
            executable_list,
            argv,
            env,
            cwd,
            _cwd_bytes: cwd_bytes,
            extra_groups,
            gid,
            uid,
            pass_fds,
        })
    }
}

fn check_pass_fds(pass_fds: &PyTuple, vm: &VirtualMachine) -> PyResult<()> {
    let mut previous = -1;
    // Tuple subclasses and int subclasses use their stored values, not overrides.
    for fd in pass_fds.as_slice() {
        let value = fd
            .downcast_ref::<PyInt>()
            .and_then(|fd| i32::try_from(fd.as_bigint()).ok());
        let Some(value) = value.filter(|value| *value >= 0 && *value > previous) else {
            return Err(vm.new_value_error("bad value(s) in fds_to_keep"));
        };
        previous = value;
    }
    Ok(())
}

fn bytes_sequence(obj: &PyObject, vm: &VirtualMachine) -> PyResult<Vec<CString>> {
    let sequence = obj.sequence_unchecked();
    let count = sequence.length(vm)?;
    let mut strings = Vec::new();
    strings
        .try_reserve_exact(count)
        .map_err(|_| vm.no_memory_error())?;
    for i in 0..count {
        let item = sequence.get_item(i as isize, vm)?;
        let bytes = item.downcast_ref::<PyBytes>().ok_or_else(|| {
            vm.new_type_error(format!("expected bytes, {} found", item.class().name()))
        })?;
        strings.push(
            CString::new(bytes.as_bytes()).map_err(|_| vm.new_value_error("embedded null byte"))?,
        );
    }
    Ok(strings)
}

// PyUnicode_FSConverter applies PyOS_FSPath, filesystem encoding, then the
// bytes NUL check. PathConverter has different argument and NUL diagnostics.
fn filesystem_bytes(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
    let bytes = match FsPath::try_from_path_like(obj.clone(), false, vm)? {
        FsPath::Bytes(bytes) => bytes,
        FsPath::Str(value) => vm
            .ctx
            .new_bytes(vm.fsencode(&value)?.as_encoded_bytes().to_vec()),
    };
    if bytes.as_bytes().contains(&0) {
        return Err(vm.new_value_error("embedded null byte"));
    }
    Ok(bytes)
}

// CPython clears temporary tuples/lists from the last element to the first.
// These references can own subclasses with Python destructors; the order is
// observable on both successful preparation and error unwinding.
#[derive(Default)]
struct ReverseReferences<T>(Vec<T>);

impl<T> Drop for ReverseReferences<T> {
    fn drop(&mut self) {
        while let Some(item) = self.0.pop() {
            drop(item);
        }
    }
}

fn prepare_argv(args: &PyObjectRef, vm: &VirtualMachine) -> PyResult<Vec<CString>> {
    // Keep exact lists live across __fspath__ calls. A snapshot would conceal a
    // size change or use stale elements after an earlier element replaces them.
    let list = args.downcast_ref_if_exact::<PyList>(vm);
    let items;
    let slice: &[PyObjectRef] = if let Some(tuple) = args.downcast_ref_if_exact::<PyTuple>(vm) {
        tuple.as_slice()
    } else if list.is_some() {
        &[]
    } else {
        let iter = args.clone().get_iter(vm).map_err(|error| {
            if error.fast_isinstance(vm.ctx.exceptions.type_error) {
                vm.new_type_error("argv must be a tuple")
            } else {
                error
            }
        })?;
        items = ReverseReferences(vm.extract_elements_sized(iter.as_object(), &|| 0, Ok)?);
        &items.0
    };
    let count = list.map_or_else(|| slice.len(), |list| list.borrow_vec().len());
    let mut converted = ReverseReferences(Vec::new());
    converted
        .0
        .try_reserve_exact(count)
        .map_err(|_| vm.no_memory_error())?;
    for i in 0..count {
        let item = if let Some(list) = list {
            let elements = list.borrow_vec();
            if elements.len() != count {
                return Err(vm.new_runtime_error("args changed during iteration"));
            }
            // Retain the item, then release the lock before calling __fspath__.
            elements[i].clone()
        } else {
            slice[i].clone()
        };
        converted.0.push(filesystem_bytes(&item, vm)?);
    }
    // CPython retains all converted bytes until native argv is copied. In
    // particular, bytes-subclass destructors must not run between conversions.
    let mut strings = Vec::new();
    strings
        .try_reserve_exact(count)
        .map_err(|_| vm.no_memory_error())?;
    for bytes in &converted.0 {
        strings.push(CString::new(bytes.as_bytes()).expect("FSConverter checked for NUL"));
    }
    Ok(strings)
}

fn prepare_groups(groups: &PyObject, vm: &VirtualMachine) -> PyResult<Vec<u32>> {
    if groups.downcast_ref::<PyList>().is_none() {
        return Err(vm.new_type_error("setgroups argument must be a list"));
    }
    let sequence = groups.sequence_unchecked();
    let count = sequence.length(vm)?;
    if host_posix::SUBPROCESS_MAX_GROUPS.is_some_and(|limit| count > limit) {
        return Err(vm.new_value_error("too many extra_groups"));
    }
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| vm.new_memory_error("failed to allocate memory for group list"))?;
    for i in 0..count {
        let item = sequence.get_item(i as isize, vm)?;
        if item.downcast_ref::<PyInt>().is_none() {
            return Err(vm.new_type_error("extra_groups must be integers"));
        }
        let gid = RawGid::try_from_object(vm, item)
            .map_err(|_| vm.new_value_error("invalid group id"))?;
        values.push(gid.0);
    }
    Ok(values)
}

#[derive(Default)]
struct CharPtrVec<'a> {
    vec: Vec<*const host_posix::c_char>,
    marker: PhantomData<Vec<&'a CStr>>,
}

impl<'a, T: AsRef<CStr>> FromIterator<&'a T> for CharPtrVec<'a> {
    fn from_iter<I: IntoIterator<Item = &'a T>>(iter: I) -> Self {
        let vec = iter
            .into_iter()
            .map(|x| x.as_ref().as_ptr())
            .chain(core::iter::once(core::ptr::null()))
            .collect();
        Self {
            vec,
            marker: PhantomData,
        }
    }
}

impl<'a> Deref for CharPtrVec<'a> {
    type Target = CharPtrSlice<'a>;
    fn deref(&self) -> &Self::Target {
        unsafe {
            &*(self.vec.as_slice() as *const [*const host_posix::c_char] as *const CharPtrSlice<'a>)
        }
    }
}

#[repr(transparent)]
struct CharPtrSlice<'a> {
    marker: PhantomData<[&'a CStr]>,
    slice: [*const host_posix::c_char],
}

impl CharPtrSlice<'_> {
    const fn as_ptr(&self) -> *const *const host_posix::c_char {
        self.slice.as_ptr()
    }
}

struct Fd(i32);

impl Write for Fd {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        host_posix::write_fd_raw(self.0, buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// can't reallocate inside of exec(), so we reallocate prior to fork() and pass this along
struct ProcArgs<'a> {
    argv: Option<&'a CharPtrSlice<'a>>,
    envp: Option<&'a CharPtrSlice<'a>>,
}

fn exec(
    args: &ForkExecArgs,
    prepared: &PreparedArgs,
    controls: ChildControls,
    procargs: ProcArgs<'_>,
    vm: &VirtualMachine,
) -> ! {
    let mut ctx = ExecErrorContext::NoExec;
    match exec_inner(args, prepared, controls, procargs, &mut ctx, vm) {
        Ok(x) => match x {},
        Err(e) => {
            let mut pipe = Fd(args.errpipe_write);
            if matches!(ctx, ExecErrorContext::PreExec) {
                // For preexec_fn errors, use SubprocessError format (errno=0)
                let _ = write!(pipe, "SubprocessError:0:{}", ctx.as_msg());
            } else {
                // errno is written in hex format
                let errno = e.raw_os_error().unwrap_or(0);
                let _ = write!(pipe, "OSError:{errno:x}:{}", ctx.as_msg());
            }
            rustpython_host_env::os::exit_immediately(255)
        }
    }
}

enum ExecErrorContext {
    NoExec,
    ChDir,
    PreExec,
    Exec,
}

impl ExecErrorContext {
    const fn as_msg(&self) -> &'static str {
        match self {
            Self::NoExec => "noexec",
            Self::ChDir => "noexec:chdir",
            Self::PreExec => "Exception occurred in preexec_fn.",
            Self::Exec => "",
        }
    }
}

fn exec_inner(
    args: &ForkExecArgs,
    prepared: &PreparedArgs,
    controls: ChildControls,
    procargs: ProcArgs<'_>,
    ctx: &mut ExecErrorContext,
    vm: &VirtualMachine,
) -> std::io::Result<Never> {
    host_posix::setup_child_fds_raw(
        prepared.pass_fds.as_slice(),
        args.errpipe_write,
        args.p2cread,
        args.p2cwrite,
        args.c2pread,
        args.c2pwrite,
        args.errread,
        args.errwrite,
        args.errpipe_read,
    )?;

    if let Some(ref cwd) = prepared.cwd {
        host_posix::chdir(cwd.as_c_str()).inspect_err(|_| *ctx = ExecErrorContext::ChDir)?
    }

    host_posix::set_umask(args.child_umask);

    if controls.restore_signals {
        host_posix::restore_signals();
    }

    host_posix::setsid_if_needed(controls.call_setsid)?;
    host_posix::setpgid_if_needed(args.pgid_to_set)?;
    host_posix::setgroups_if_needed(prepared.extra_groups.as_deref())?;
    host_posix::setregid_if_needed(prepared.gid.map(|gid| gid.0))?;
    host_posix::setreuid_if_needed(prepared.uid.map(|uid| uid.0))?;

    // Call preexec_fn after all process setup but before closing FDs
    if let Some(ref preexec_fn) = args.preexec_fn {
        match preexec_fn.call((), vm) {
            Ok(_) => {}
            Err(_e) => {
                // Cannot safely stringify exception after fork
                *ctx = ExecErrorContext::PreExec;
                return Err(std::io::Error::from_raw_os_error(0));
            }
        }
    }

    *ctx = ExecErrorContext::Exec;

    if controls.close_fds {
        host_posix::close_fds_raw(2, prepared.pass_fds.as_slice());
    }

    let err = host_posix::exec_replace(
        prepared.executable_list.as_slice(),
        procargs
            .argv
            .map_or(core::ptr::null(), CharPtrSlice::as_ptr),
        procargs.envp.map(CharPtrSlice::as_ptr),
    );
    Err(std::io::Error::from_raw_os_error(err as i32))
}
