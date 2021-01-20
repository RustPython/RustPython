use crate::builtins::code;
use crate::bytecode;
use crate::VirtualMachine;

pub fn map_frozen<'a>(
    vm: &'a VirtualMachine,
    i: impl IntoIterator<Item = (String, bytecode::FrozenModule)> + 'a,
) -> impl Iterator<Item = (String, code::FrozenModule)> + 'a {
    i.into_iter()
        .map(move |(k, bytecode::FrozenModule { code, package })| {
            (
                k,
                code::FrozenModule {
                    code: vm.map_codeobj(code),
                    package,
                },
            )
        })
}

pub fn get_module_inits() -> impl Iterator<Item = (String, bytecode::FrozenModule)> {
    let iter = std::iter::empty();
    macro_rules! ext_modules {
        ($iter:ident, ($modules:expr)) => {
            let $iter = $iter.chain($modules);
        };
        ($iter:ident, $($t:tt)*) => {
            ext_modules!($iter, (py_freeze!($($t)*)))
        };
    }

    ext_modules!(
        iter,
        source = "initialized = True; print(\"Hello world!\")\n",
        module_name = "__hello__",
    );

    // Python modules that the vm calls into, but are not actually part of the stdlib. They could
    // in theory be implemented in Rust, but are easiest to do in Python for one reason or another.
    // Includes _importlib_bootstrap and _importlib_bootstrap_external
    // For Windows: did you forget to run `powershell scripts\symlinks-to-hardlinks.ps1`?
    ext_modules!(iter, dir = "Lib/python_builtins/");

    #[cfg(not(feature = "freeze-stdlib"))]
    // core stdlib Python modules that the vm calls into, but are still used in Python
    // application code, e.g. copyreg
    ext_modules!(iter, dir = "Lib/core_modules/");
    // if we're on freeze-stdlib, the core stdlib modules will be included anyway
    #[cfg(feature = "freeze-stdlib")]
    ext_modules!(iter, (rustpython_pylib::frozen_stdlib()));

    iter
}
