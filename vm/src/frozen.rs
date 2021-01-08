use crate::builtins::code;
use crate::bytecode;
use crate::VirtualMachine;
use std::collections::HashMap;

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

pub fn get_module_inits(
    vm: &VirtualMachine,
) -> HashMap<String, code::FrozenModule, ahash::RandomState> {
    let mut modules = HashMap::default();

    macro_rules! ext_modules {
        ($($t:tt)*) => {
            modules.extend(map_frozen(vm, py_freeze!($($t)*)));
        };
    }

    ext_modules!(
        source = "initialized = True; print(\"Hello world!\")\n",
        module_name = "__hello__",
    );

    // Python modules that the vm calls into, but are not actually part of the stdlib. They could
    // in theory be implemented in Rust, but are easiest to do in Python for one reason or another.
    // Includes _importlib_bootstrap and _importlib_bootstrap_external
    // For Windows: did you forget to run `powershell scripts\symlinks-to-hardlinks.ps1`?
    ext_modules!(dir = "Lib/python_builtins/");

    #[cfg(not(feature = "freeze-stdlib"))]
    {
        // core stdlib Python modules that the vm calls into, but are still used in Python
        // application code, e.g. copyreg
        ext_modules!(dir = "Lib/core_modules/");
    }
    // if we're on freeze-stdlib, the core stdlib modules will be included anyway
    #[cfg(feature = "freeze-stdlib")]
    {
        modules.extend(map_frozen(vm, rustpython_pylib::frozen_stdlib()));
    }

    modules
}
