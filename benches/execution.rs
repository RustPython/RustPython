use criterion::measurement::WallTime;
use criterion::{
    criterion_group, criterion_main, Bencher, BenchmarkGroup, BenchmarkId, Criterion, Throughput,
};
use rustpython_compiler::Mode;
use rustpython_parser::parser::parse_program;
use rustpython_vm::pyobject::PyResult;
use rustpython_vm::Interpreter;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::{fs, io};

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

pub fn benchmark_file_parsing(group: &mut BenchmarkGroup<WallTime>, name: &str, contents: &String) {
    group.throughput(Throughput::Bytes(contents.len() as u64));
    group.bench_function(BenchmarkId::new("rustpython", name), |b| {
        b.iter(|| parse_program(contents).unwrap())
    });
    group.bench_function(BenchmarkId::new("cpython", name), |b| {
        let gil = cpython::Python::acquire_gil();
        let python = gil.python();

        let globals = None;
        let locals = cpython::PyDict::new(python);

        locals.set_item(python, "SOURCE_CODE", &contents).unwrap();

        let code = "compile(SOURCE_CODE, mode=\"exec\", filename=\"minidom.py\")";
        b.iter(|| {
            let res: cpython::PyResult<cpython::PyObject> =
                python.eval(code, globals, Some(&locals));
            if let Err(e) = res {
                e.print(python);
                panic!("Error compiling source")
            }
        })
    });
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
    let dirs: Vec<fs::DirEntry> = benchmark_dir
        .read_dir()
        .unwrap()
        .collect::<io::Result<_>>()
        .unwrap();
    let paths: Vec<PathBuf> = dirs.iter().map(|p| p.path()).collect();

    let mut name_to_contents: HashMap<String, String> = paths
        .into_iter()
        .map(|p| {
            let name = p.file_name().unwrap().to_os_string();
            let contents = fs::read_to_string(p).unwrap();
            (name.into_string().unwrap(), contents)
        })
        .collect();

    // Benchmark parsing
    let mut parse_group = c.benchmark_group("parse_to_ast");
    for (name, contents) in name_to_contents.iter() {
        benchmark_file_parsing(&mut parse_group, name, contents);
    }
    parse_group.finish();

    // Benchmark PyStone
    if let Some(pystone_contents) = name_to_contents.remove("pystone.py") {
        let mut pystone_group = c.benchmark_group("pystone");
        benchmark_pystone(&mut pystone_group, pystone_contents);
        pystone_group.finish();
    }

    // Benchmark execution
    let mut execution_group = c.benchmark_group("execution");
    for (name, contents) in name_to_contents.iter() {
        benchmark_file_execution(&mut execution_group, name, contents);
    }
    execution_group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
