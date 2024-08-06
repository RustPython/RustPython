use super::{setting::Settings, thread, Context, VirtualMachine};
use crate::{stdlib::atexit, vm::PyBaseExceptionRef, PyResult};
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

    /// Run a function with the main virtual machine and return a PyResult of the result.
    ///
    /// To enter vm context multiple times or to avoid buffer/exception management, this function is preferred.
    /// `enter` is lightweight and it returns a python object in PyResult.
    /// You can stop or continue the execution multiple times by calling `enter`.
    ///
    /// To finalize the vm once all desired `enter`s are called, calling `finalize` will be helpful.
    ///
    /// See also [`run`] for managed way to run the interpreter.
    pub fn enter<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&VirtualMachine) -> R,
    {
        thread::enter_vm(&self.vm, || f(&self.vm))
    }

    /// Run [`enter`] and call `expect_pyresult` for the result.
    ///
    /// This function is useful when you want to expect a result from the function,
    /// but also print useful panic information when exception raised.
    ///
    /// See [`enter`] for more information.
    /// See [`expect_pyresult`] for more information.
    pub fn enter_and_expect<F, R>(&self, f: F, msg: &str) -> R
    where
        F: FnOnce(&VirtualMachine) -> PyResult<R>,
    {
        self.enter(|vm| {
            let result = f(vm);
            vm.expect_pyresult(result, msg)
        })
    }

    /// Run a function with the main virtual machine and return exit code.
    ///
    /// To enter vm context only once and safely terminate the vm, this function is preferred.
    /// Unlike [`enter`], `run` calls finalize and returns exit code.
    /// You will not be able to obtain Python exception in this way.
    ///
    /// See [`finalize`] for the finalization steps.
    /// See also [`enter`] for pure function call to obtain Python exception.
    pub fn run<F>(self, f: F) -> u8
    where
        F: FnOnce(&VirtualMachine) -> PyResult<()>,
    {
        let res = self.enter(|vm| f(vm));
        self.finalize(res.err())
    }

    /// Finalize vm and turns an exception to exit code.
    ///
    /// Finalization steps including 4 steps:
    /// 1. Flush stdout and stderr.
    /// 1. Handle exit exception and turn it to exit code.
    /// 1. Run atexit exit functions.
    /// 1. Mark vm as finalized.
    ///
    /// Note that calling `finalize` is not necessary by purpose though.
    pub fn finalize(self, exc: Option<PyBaseExceptionRef>) -> u8 {
        self.enter(|vm| {
            vm.flush_std();

            // See if any exception leaked out:
            let exit_code = if let Some(exc) = exc {
                vm.handle_exit_exception(exc)
            } else {
                0
            };

            atexit::_run_exitfuncs(vm);

            vm.state.finalizing.store(true, Ordering::Release);

            vm.flush_std();

            exit_code
        })
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
