use crate::bytecode::frozen_lib::FrozenModule;

pub fn core_frozen_inits() -> impl Iterator<Item = (&'static str, FrozenModule)> {
    let iter = std::iter::empty();
    macro_rules! ext_modules {
        ($iter:ident, $($t:tt)*) => {
            let $iter = $iter.chain(py_freeze!($($t)*));
        };
    }

    // keep as example but use file one now
    // ext_modules!(
    //     iter,
    //     source = "initialized = True; print(\"Hello world!\")\n",
    //     module_name = "__hello__",
    // );

    // Python modules that the vm calls into, but are not actually part of the stdlib. They could
    // in theory be implemented in Rust, but are easiest to do in Python for one reason or another.
    // Includes _importlib_bootstrap and _importlib_bootstrap_external
    ext_modules!(
        iter,
        dir = "./Lib/python_builtins",
        crate_name = "rustpython_compiler_core"
    );

    // core stdlib Python modules that the vm calls into, but are still used in Python
    // application code, e.g. copyreg
    #[cfg(not(feature = "freeze-stdlib"))]
    ext_modules!(
        iter,
        dir = "./Lib/core_modules",
        crate_name = "rustpython_compiler_core"
    );

    iter
}
