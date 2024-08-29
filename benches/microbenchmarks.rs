use criterion::{
    criterion_group, criterion_main, measurement::WallTime, BatchSize, BenchmarkGroup, BenchmarkId,
    Criterion, Throughput,
};
use pyo3::types::PyAnyMethods;
use rustpython_compiler::Mode;
use rustpython_vm::{AsObject, Interpreter, PyResult, Settings};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

// List of microbenchmarks to skip.
//
// These result in excessive memory usage, some more so than others. For example, while
// exception_context.py consumes a lot of memory, it still finishes. On the other hand,
// call_kwargs.py seems like it performs an excessive amount of allocations and results in
// a system freeze.
// In addition, the fact that we don't yet have a GC means that benchmarks which might consume
// a bearable amount of memory accumulate. As such, best to skip them for now.
const SKIP_MICROBENCHMARKS: [&str; 8] = [
    "call_simple.py",
    "call_kwargs.py",
    "construct_object.py",
    "define_function.py",
    "define_class.py",
    "exception_nested.py",
    "exception_simple.py",
    "exception_context.py",
];

pub struct MicroBenchmark {
    name: String,
    setup: String,
    code: String,
    iterate: bool,
}

fn bench_cpython_code(group: &mut BenchmarkGroup<WallTime>, bench: &MicroBenchmark) {
    pyo3::Python::with_gil(|py| {
        let setup_name = format!("{}_setup", bench.name);
        let setup_code = cpy_compile_code(py, &bench.setup, &setup_name).unwrap();

        let code = cpy_compile_code(py, &bench.code, &bench.name).unwrap();

        // Grab the exec function in advance so we don't have lookups in the hot code
        let builtins =
            pyo3::types::PyModule::import_bound(py, "builtins").expect("Failed to import builtins");
        let exec = builtins.getattr("exec").expect("no exec in builtins");

        let bench_func = |(globals, locals): &mut (
            pyo3::Bound<pyo3::types::PyDict>,
            pyo3::Bound<pyo3::types::PyDict>,
        )| {
            let res = exec.call((&code, &*globals, &*locals), None);
            if let Err(e) = res {
                e.print(py);
                panic!("Error running microbenchmark")
            }
        };

        let bench_setup = |iterations| {
            let globals = pyo3::types::PyDict::new_bound(py);
            let locals = pyo3::types::PyDict::new_bound(py);
            if let Some(idx) = iterations {
                globals.set_item("ITERATIONS", idx).unwrap();
            }

            let res = exec.call((&setup_code, &globals, &locals), None);
            if let Err(e) = res {
                e.print(py);
                panic!("Error running microbenchmark setup code")
            }
            (globals, locals)
        };

        if bench.iterate {
            for idx in (100..=1_000).step_by(200) {
                group.throughput(Throughput::Elements(idx as u64));
                group.bench_with_input(BenchmarkId::new("cpython", &bench.name), &idx, |b, idx| {
                    b.iter_batched_ref(
                        || bench_setup(Some(*idx)),
                        bench_func,
                        BatchSize::LargeInput,
                    );
                });
            }
        } else {
            group.bench_function(BenchmarkId::new("cpython", &bench.name), move |b| {
                b.iter_batched_ref(|| bench_setup(None), bench_func, BatchSize::LargeInput);
            });
        }
    })
}

fn cpy_compile_code<'a>(
    py: pyo3::Python<'a>,
    code: &str,
    name: &str,
) -> pyo3::PyResult<pyo3::Bound<'a, pyo3::types::PyCode>> {
    let builtins =
        pyo3::types::PyModule::import_bound(py, "builtins").expect("Failed to import builtins");
    let compile = builtins.getattr("compile").expect("no compile in builtins");
    compile.call1((code, name, "exec"))?.extract()
}

fn bench_rustpy_code(group: &mut BenchmarkGroup<WallTime>, bench: &MicroBenchmark) {
    let mut settings = Settings::default();
    settings.path_list.push("Lib/".to_string());
    settings.write_bytecode = false;
    settings.user_site_directory = false;

    Interpreter::with_init(settings, |vm| {
        for (name, init) in rustpython_stdlib::get_module_inits() {
            vm.add_native_module(name, init);
        }
    })
    .enter(|vm| {
        let setup_code = vm
            .compile(&bench.setup, Mode::Exec, bench.name.to_owned())
            .expect("Error compiling setup code");
        let bench_code = vm
            .compile(&bench.code, Mode::Exec, bench.name.to_owned())
            .expect("Error compiling bench code");

        let bench_func = |scope| {
            let res: PyResult = vm.run_code_obj(bench_code.clone(), scope);
            vm.unwrap_pyresult(res);
        };

        let bench_setup = |iterations| {
            let scope = vm.new_scope_with_builtins();
            if let Some(idx) = iterations {
                scope
                    .locals
                    .as_object()
                    .set_item("ITERATIONS", vm.new_pyobj(idx), vm)
                    .expect("Error adding ITERATIONS local variable");
            }
            let setup_result = vm.run_code_obj(setup_code.clone(), scope.clone());
            vm.unwrap_pyresult(setup_result);
            scope
        };

        if bench.iterate {
            for idx in (100..=1_000).step_by(200) {
                group.throughput(Throughput::Elements(idx as u64));
                group.bench_with_input(
                    BenchmarkId::new("rustpython", &bench.name),
                    &idx,
                    |b, idx| {
                        b.iter_batched(
                            || bench_setup(Some(*idx)),
                            bench_func,
                            BatchSize::LargeInput,
                        );
                    },
                );
            }
        } else {
            group.bench_function(BenchmarkId::new("rustpython", &bench.name), move |b| {
                b.iter_batched(|| bench_setup(None), bench_func, BatchSize::LargeInput);
            });
        }
    })
}

pub fn run_micro_benchmark(c: &mut Criterion, benchmark: MicroBenchmark) {
    let mut group = c.benchmark_group("microbenchmarks");

    bench_cpython_code(&mut group, &benchmark);
    bench_rustpy_code(&mut group, &benchmark);

    group.finish();
}

pub fn criterion_benchmark(c: &mut Criterion) {
    let benchmark_dir = Path::new("./benches/microbenchmarks/");
    let dirs: Vec<fs::DirEntry> = benchmark_dir
        .read_dir()
        .unwrap()
        .collect::<io::Result<_>>()
        .unwrap();
    let paths: Vec<PathBuf> = dirs.iter().map(|p| p.path()).collect();

    let benchmarks: Vec<MicroBenchmark> = paths
        .into_iter()
        .map(|p| {
            let name = p.file_name().unwrap().to_os_string();
            let contents = fs::read_to_string(p).unwrap();
            let iterate = contents.contains("ITERATIONS");

            let (setup, code) = if contents.contains("# ---") {
                let split: Vec<&str> = contents.splitn(2, "# ---").collect();
                (split[0].to_string(), split[1].to_string())
            } else {
                ("".to_string(), contents)
            };
            let name = name.into_string().unwrap();
            MicroBenchmark {
                name,
                setup,
                code,
                iterate,
            }
        })
        .collect();

    for benchmark in benchmarks {
        if SKIP_MICROBENCHMARKS.contains(&benchmark.name.as_str()) {
            continue;
        }
        run_micro_benchmark(c, benchmark);
    }
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
