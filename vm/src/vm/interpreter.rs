use super::{setting::Settings, thread, InitParameter, VirtualMachine};

/// The general interface for the VM
///
/// # Examples
/// Runs a simple embedded hello world program.
/// ```
/// use rustpython_vm::Interpreter;
/// use rustpython_vm::compile::Mode;
/// Interpreter::default().enter(|vm| {
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
    pub fn new(settings: Settings, init: InitParameter) -> Self {
        Self::new_with_init(settings, |_| init)
    }

    pub fn new_with_init<F>(settings: Settings, init: F) -> Self
    where
        F: FnOnce(&mut VirtualMachine) -> InitParameter,
    {
        let mut vm = VirtualMachine::new(settings);
        let init = init(&mut vm);
        vm.initialize(init);
        Self { vm }
    }

    pub fn enter<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&VirtualMachine) -> R,
    {
        thread::enter_vm(&self.vm, || f(&self.vm))
    }

    // TODO: interpreter shutdown
    // pub fn run<F>(self, f: F)
    // where
    //     F: FnOnce(&VirtualMachine),
    // {
    //     self.enter(f);
    //     self.shutdown();
    // }

    // pub fn shutdown(self) {}
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new(Settings::default(), InitParameter::External)
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
        Interpreter::default().enter(|vm| {
            let a: PyObjectRef = vm.ctx.new_int(33_i32).into();
            let b: PyObjectRef = vm.ctx.new_int(12_i32).into();
            let res = vm._add(&a, &b).unwrap();
            let value = int::get_value(&res);
            assert_eq!(*value, 45_i32.to_bigint().unwrap());
        })
    }

    #[test]
    fn test_multiply_str() {
        Interpreter::default().enter(|vm| {
            let a = vm.new_pyobj(crate::common::ascii!("Hello "));
            let b = vm.new_pyobj(4_i32);
            let res = vm._mul(&a, &b).unwrap();
            let value = res.payload::<PyStr>().unwrap();
            assert_eq!(value.as_ref(), "Hello Hello Hello Hello ")
        })
    }
}
