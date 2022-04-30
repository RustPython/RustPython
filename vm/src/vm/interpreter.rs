use super::{setting::Settings, thread, VirtualMachine};
use crate::{
    stdlib::{atexit, sys},
    PyResult,
};

/// The general interface for the VM
///
/// # Examples
/// Runs a simple embedded hello world program.
/// ```
/// use rustpython_vm::Interpreter;
/// use rustpython_vm::compile::Mode;
/// Interpreter::without_stdlib(Default::default()).enter(|vm| {
///     let scope = vm.new_scope_with_builtins();
///     let code_obj = vm.compile(r#"print("Hello World!")"#,
///             Mode::Exec,
///             "<embedded>".to_owned(),
///     ).map_err(|err| vm.new_syntax_error(&err)).unwrap();
///     vm.run_code_obj(code_obj, scope).unwrap();
/// });
/// ```
pub struct Interpreter {
    vm: VirtualMachine,
}

impl Interpreter {
    /// To create with stdlib, use `with_init`
    pub fn without_stdlib(settings: Settings) -> Self {
        Self::with_init(settings, |_| {})
    }

    /// Create with initialize function taking mutable vm reference.
    /// ```
    /// use rustpython_vm::Interpreter;
    /// Interpreter::with_init(Default::default(), |vm| {
    ///     // put this line to add stdlib to the vm
    ///     // vm.add_native_modules(rustpython_stdlib::get_module_inits());
    /// }).enter(|vm| {
    ///     vm.run_code_string(vm.new_scope_with_builtins(), "print(1)", "<...>".to_owned());
    /// });
    /// ```
    pub fn with_init<F>(settings: Settings, init: F) -> Self
    where
        F: FnOnce(&mut VirtualMachine),
    {
        let mut vm = VirtualMachine::new(settings);
        init(&mut vm);
        vm.initialize();
        Self { vm }
    }

    pub fn enter<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&VirtualMachine) -> R,
    {
        thread::enter_vm(&self.vm, || f(&self.vm))
    }

    pub fn run<F, R>(self, f: F) -> i32
    where
        F: FnOnce(&VirtualMachine) -> PyResult<R>,
    {
        self.enter(|vm| {
            let res = f(vm);
            flush_std(vm);

            // See if any exception leaked out:
            let exit_code = res
                .map(|_| 0)
                .map_err(|exc| vm.handle_exit_exception(exc))
                .unwrap_or_else(|code| code);

            let _ = atexit::_run_exitfuncs(vm);

            flush_std(vm);

            exit_code
        })
    }
}

fn flush_std(vm: &VirtualMachine) {
    if let Ok(stdout) = sys::get_stdout(vm) {
        let _ = vm.call_method(&stdout, "flush", ());
    }
    if let Ok(stderr) = sys::get_stderr(vm) {
        let _ = vm.call_method(&stderr, "flush", ());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        builtins::{int, PyStr},
        PyObjectRef,
    };
    use num_bigint::ToBigInt;

    #[test]
    fn test_add_py_integers() {
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let a: PyObjectRef = vm.ctx.new_int(33_i32).into();
            let b: PyObjectRef = vm.ctx.new_int(12_i32).into();
            let res = vm._add(&a, &b).unwrap();
            let value = int::get_value(&res);
            assert_eq!(*value, 45_i32.to_bigint().unwrap());
        })
    }

    #[test]
    fn test_multiply_str() {
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let a = vm.new_pyobj(crate::common::ascii!("Hello "));
            let b = vm.new_pyobj(4_i32);
            let res = vm._mul(&a, &b).unwrap();
            let value = res.payload::<PyStr>().unwrap();
            assert_eq!(value.as_ref(), "Hello Hello Hello Hello ")
        })
    }
}
