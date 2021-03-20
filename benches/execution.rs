use criterion::measurement::WallTime;
use criterion::{
    criterion_group, criterion_main, Bencher, BenchmarkGroup, BenchmarkId, Criterion, Throughput,
};
use rustpython_compiler::Mode;
use rustpython_parser::parser::parse_program;
use rustpython_vm::pyobject::PyResult;
use rustpython_vm::Interpreter;
use std::collections::HashMap;
use std::path::Path;

fn bench_cpython_code(b: &mut Bencher, source: &str) {
    let gil = cpython::Python::acquire_gil();
    let python = gil.python();

    b.iter(|| {
        let res: cpython::PyResult<()> = python.run(source, None, None);
        if let Err(e) = res {
            e.print(python);
            panic!("Error running source")
        }
    });
}

fn bench_rustpy_code(b: &mut Bencher, name: &str, source: &str) {
    // NOTE: Take long time.
    Interpreter::default().enter(|vm| {
        // Note: bench_cpython is both compiling and executing the code.
        // As such we compile the code in the benchmark loop as well.
        b.iter(|| {
            let code = vm.compile(source, Mode::Exec, name.to_owned()).unwrap();
            let scope = vm.new_scope_with_builtins();
            let res: PyResult = vm.run_code_obj(code.clone(), scope);
            vm.unwrap_pyresult(res);
        })
    })
}

pub fn benchmark_file_execution(
    group: &mut BenchmarkGroup<WallTime>,
    name: &str,
    contents: &String,
) {
    group.bench_function(BenchmarkId::new(name, "cpython"), |b| {
        bench_cpython_code(b, &contents)
    });
    group.bench_function(BenchmarkId::new(name, "rustpython"), |b| {
        bench_rustpy_code(b, name, &contents)
    });
}

pub fn benchmark_file_parsing(group: &mut BenchmarkGroup<WallTime>, name: &str, contents: &str) {
    group.throughput(Throughput::Bytes(contents.len() as u64));
    group.bench_function(BenchmarkId::new("rustpython", name), |b| {
        b.iter(|| parse_program(contents).unwrap())
    });
    group.bench_function(BenchmarkId::new("cpython", name), |b| {
        let gil = cpython::Python::acquire_gil();
        let py = gil.python();

        let code = std::ffi::CString::new(contents).unwrap();
        let fname = cpython::PyString::new(py, name);

        b.iter(|| parse_program_cpython(py, &code, &fname))
    });
}

fn parse_program_cpython(
    py: cpython::Python<'_>,
    code: &std::ffi::CStr,
    fname: &cpython::PyString,
) {
    extern "C" {
        fn PyArena_New() -> *mut python3_sys::PyArena;
        fn PyArena_Free(arena: *mut python3_sys::PyArena);
    }
    use cpython::PythonObject;
    let fname = fname.as_object();
    unsafe {
        let arena = PyArena_New();
        assert!(!arena.is_null());
        let ret = python3_sys::PyParser_ASTFromStringObject(
            code.as_ptr() as _,
            fname.as_ptr(),
            python3_sys::Py_file_input,
            std::ptr::null_mut(),
            arena,
        );
        if ret.is_null() {
            cpython::PyErr::fetch(py).print(py);
        }
        PyArena_Free(arena);
    }
}

pub fn benchmark_pystone(group: &mut BenchmarkGroup<WallTime>, contents: String) {
    // Default is 50_000. This takes a while, so reduce it to 30k.
    for idx in (10_000..=30_000).step_by(10_000) {
        let code_with_loops = format!("LOOPS = {}\n{}", idx, contents);
        let code_str = code_with_loops.as_str();

        group.throughput(Throughput::Elements(idx as u64));
        group.bench_function(BenchmarkId::new("cpython", idx), |b| {
            bench_cpython_code(b, code_str)
        });
        group.bench_function(BenchmarkId::new("rustpython", idx), |b| {
            bench_rustpy_code(b, "pystone", code_str)
        });
    }
}

pub fn criterion_benchmark(c: &mut Criterion) {
    let benchmark_dir = Path::new("./benches/benchmarks/");
    let mut benches = benchmark_dir
        .read_dir()
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            (
                path.file_name().unwrap().to_str().unwrap().to_owned(),
                std::fs::read_to_string(path).unwrap(),
            )
        })
        .collect::<HashMap<_, _>>();

    // Benchmark parsing
    let mut parse_group = c.benchmark_group("parse_to_ast");
    for (name, contents) in &benches {
        benchmark_file_parsing(&mut parse_group, name, contents);
    }
    parse_group.finish();

    // Benchmark PyStone
    if let Some(pystone_contents) = benches.remove("pystone.py") {
        let mut pystone_group = c.benchmark_group("pystone");
        benchmark_pystone(&mut pystone_group, pystone_contents);
        pystone_group.finish();
    }

    // Benchmark execution
    let mut execution_group = c.benchmark_group("execution");
    for (name, contents) in &benches {
        benchmark_file_execution(&mut execution_group, name, contents);
    }
    execution_group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
