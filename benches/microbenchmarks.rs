use criterion::measurement::WallTime;
use criterion::{
    criterion_group, criterion_main, BatchSize, BenchmarkGroup, BenchmarkId, Criterion, Throughput,
};
use rustpython_compiler::Mode;
use rustpython_vm::pyobject::ItemProtocol;
use rustpython_vm::pyobject::PyResult;
use rustpython_vm::{InitParameter, Interpreter, PySettings};
use std::path::{Path, PathBuf};
use std::{ffi, fs, io};

pub struct MicroBenchmark {
    name: String,
    setup: String,
    code: String,
    iterate: bool,
}

fn bench_cpython_code(group: &mut BenchmarkGroup<WallTime>, bench: &MicroBenchmark) {
    let gil = cpython::Python::acquire_gil();
    let py = gil.python();

    let setup_code = ffi::CString::new(&*bench.setup).unwrap();
    let setup_name = ffi::CString::new(format!("{}_setup", bench.name)).unwrap();
    let setup_code = cpy_compile_code(py, &setup_code, &setup_name).unwrap();

    let code = ffi::CString::new(&*bench.code).unwrap();
    let name = ffi::CString::new(&*bench.name).unwrap();
    let code = cpy_compile_code(py, &code, &name).unwrap();

    let bench_func = |(globals, locals): &mut (cpython::PyDict, cpython::PyDict)| {
        let res = cpy_run_code(py, &code, globals, locals);
        if let Err(e) = res {
            e.print(py);
            panic!("Error running microbenchmark")
        }
    };

    let bench_setup = |iterations| {
        let globals = cpython::PyDict::new(py);
        // setup the __builtins__ attribute - no other way to do this (other than manually) as far
        // as I can tell
        let _ = py.run("", Some(&globals), None);
        let locals = cpython::PyDict::new(py);
        if let Some(idx) = iterations {
            globals.set_item(py, "ITERATIONS", idx).unwrap();
        }

        let res = cpy_run_code(py, &setup_code, &globals, &locals);
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
}

unsafe fn cpy_res(
    py: cpython::Python<'_>,
    x: *mut python3_sys::PyObject,
) -> cpython::PyResult<cpython::PyObject> {
    cpython::PyObject::from_owned_ptr_opt(py, x).ok_or_else(|| cpython::PyErr::fetch(py))
}

fn cpy_compile_code(
    py: cpython::Python<'_>,
    s: &ffi::CStr,
    fname: &ffi::CStr,
) -> cpython::PyResult<cpython::PyObject> {
    unsafe {
        let res =
            python3_sys::Py_CompileString(s.as_ptr(), fname.as_ptr(), python3_sys::Py_file_input);
        cpy_res(py, res)
    }
}

fn cpy_run_code(
    py: cpython::Python<'_>,
    code: &cpython::PyObject,
    locals: &cpython::PyDict,
    globals: &cpython::PyDict,
) -> cpython::PyResult<cpython::PyObject> {
    use cpython::PythonObject;
    unsafe {
        let res = python3_sys::PyEval_EvalCode(
            code.as_ptr(),
            locals.as_object().as_ptr(),
            globals.as_object().as_ptr(),
        );
        cpy_res(py, res)
    }
}

fn bench_rustpy_code(group: &mut BenchmarkGroup<WallTime>, bench: &MicroBenchmark) {
    let mut settings = PySettings::default();
    settings.path_list.push("Lib/".to_string());
    settings.dont_write_bytecode = true;
    settings.no_user_site = true;

    Interpreter::new(settings, InitParameter::External).enter(|vm| {
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
                    .set_item(vm.ctx.new_str("ITERATIONS"), vm.ctx.new_int(idx), vm)
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
        run_micro_benchmark(c, benchmark);
    }
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
