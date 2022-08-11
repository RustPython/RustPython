use crate::bytecode::FrozenModule;

pub fn get_module_inits() -> impl Iterator<Item = (String, FrozenModule)> {
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
    // ext_modules!(iter, dir = "Lib/python_builtins/");

    // #[cfg(not(feature = "freeze-stdlib"))]
    // core stdlib Python modules that the vm calls into, but are still used in Python
    // application code, e.g. copyreg
    // ext_modules!(iter, dir = "Lib/core_modules/");
    // if we're on freeze-stdlib, the core stdlib modules will be included anyway
    // #[cfg(feature = "freeze-stdlib")]
    // ext_modules!(iter, (rustpython_pylib::frozen_stdlib()));

    iter
}
