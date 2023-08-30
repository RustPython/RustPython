use super::{setting::Settings, thread, Context, VirtualMachine};
use crate::{
    stdlib::{atexit, sys},
    PyResult,
};
use std::sync::atomic::Ordering;

/// The general interface for the VM
///
/// # Examples
/// Runs a simple embedded hello world program.
/// ```
/// use rustpython_vm::Interpreter;
/// use rustpython_vm::compiler::Mode;
/// Interpreter::without_stdlib(Default::default()).enter(|vm| {
///     let scope = vm.new_scope_with_builtins();
///     let source = r#"print("Hello World!")"#;
///     let code_obj = vm.compile(
///             source,
///             Mode::Exec,
///             "<embedded>".to_owned(),
///     ).map_err(|err| vm.new_syntax_error(&err, Some(source))).unwrap();
///     vm.run_code_obj(code_obj, scope).unwrap();
/// });
/// ```
pub struct Interpreter {
    vm: VirtualMachine,
}

impl Interpreter {
    /// This is a bare unit to build up an interpreter without the standard library.
    /// To create an interpreter with the standard library with the `rustpython` crate, use `rustpython::InterpreterConfig`.
    /// To create an interpreter without the `rustpython` crate, but only with `rustpython-vm`,
    /// try to build one from the source code of `InterpreterConfig`. It will not be a one-liner but it also will not be too hard.
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
        let ctx = Context::genesis();
        crate::types::TypeZoo::extend(ctx);
        crate::exceptions::ExceptionZoo::extend(ctx);
        let mut vm = VirtualMachine::new(settings, ctx.clone());
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

    pub fn run<F, R>(self, f: F) -> u8
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

            atexit::_run_exitfuncs(vm);

            vm.state.finalizing.store(true, Ordering::Release);

            flush_std(vm);

            exit_code
        })
    }
}

pub(crate) fn flush_std(vm: &VirtualMachine) {
    if let Ok(stdout) = sys::get_stdout(vm) {
        let _ = vm.call_method(&stdout, identifier!(vm, flush).as_str(), ());
    }
    if let Ok(stderr) = sys::get_stderr(vm) {
        let _ = vm.call_method(&stderr, identifier!(vm, flush).as_str(), ());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        builtins::{int, PyStr},
        PyObjectRef,
    };
    use malachite_bigint::ToBigInt;

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
