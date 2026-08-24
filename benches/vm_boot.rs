//! Benchmarks for interpreter construction and the import machinery.
//!
//! The other bench files deliberately build the `Interpreter` outside the
//! timed closure so they measure guest-code execution only. These do the
//! opposite: the VM construction *is* the subject, which is the part of
//! startup cost that lives in Rust and that a PR can regress.
//!
//! What this can and cannot see: a real `rustpython -c pass` process costs
//! ~135M instructions, of which building the VM and running the code in an
//! already-warm process accounts for ~40M. The remaining ~95M is process
//! exec, dynamic linking, one-time lazy initialisation and teardown, all of
//! which happen exactly once per process and therefore cannot be observed by
//! any in-process harness -- the first `Interpreter` built in a process costs
//! ~7.5x what later ones do. The subprocess workloads in scripts/perf_ci.py
//! cover that end-to-end number; these cover the reconstructible part.

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use rustpython_compiler::Mode;
use rustpython_vm::{Interpreter, PyResult, Settings};
use std::hint::black_box;

fn build_interpreter() -> Interpreter {
    let mut settings = Settings::default();
    settings.path_list.push("Lib/".to_string());
    // Never touch __pycache__, so every run compiles the same sources and the
    // measurement does not depend on what a previous run left behind.
    settings.write_bytecode = false;
    settings.user_site_directory = false;
    let builder = Interpreter::builder(settings);
    let defs = rustpython_stdlib::stdlib_module_defs(&builder.ctx);
    builder.add_native_modules(&defs).build()
}

fn run_source(interpreter: &Interpreter, name: &str, source: &str) {
    interpreter.enter(|vm| {
        let code = vm.compile(source, Mode::Exec, name.to_owned()).unwrap();
        let scope = vm.new_scope_with_builtins();
        let res: PyResult = vm.run_code_obj(code, scope);
        vm.unwrap_pyresult(res);
    })
}

pub fn criterion_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("vm_boot");
    // Each iteration builds a whole interpreter, so keep the sample count low
    // enough that `cargo bench` stays usable.
    group.sample_size(20);

    // Interpreter construction on its own: registering native modules, setting
    // up types, populating builtins.
    group.bench_function("build", |b| {
        b.iter(|| black_box(build_interpreter()));
    });

    // Construction plus executing a trivial program, the in-process analogue
    // of `rustpython -c pass`.
    group.bench_function("build_and_run_pass", |b| {
        b.iter(|| {
            let interpreter = build_interpreter();
            run_source(&interpreter, "<bench>", "pass");
            black_box(interpreter);
        });
    });

    // The import machinery, with construction moved into the setup closure,
    // which is not measured, so only importing is. A fresh interpreter per
    // iteration is what makes these real imports rather than sys.modules
    // lookups.
    group.bench_function("import_stdlib", |b| {
        b.iter_batched(
            build_interpreter,
            |interpreter| {
                run_source(
                    &interpreter,
                    "<bench-import>",
                    "import json, re, collections, functools, itertools",
                );
                interpreter
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
