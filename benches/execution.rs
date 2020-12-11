use criterion::{black_box, criterion_group, criterion_main, Bencher, Criterion, BenchmarkId, Throughput};
use rustpython_compiler::{Mode};
use rustpython_vm::pyobject::PyResult;
use rustpython_vm::Interpreter;

const NBODY: &str = include_str!("./benchmarks/nbody.py");
const MANDELBROT: &str = include_str!("./benchmarks/mandelbrot.py");
const PYSTONE: &str = include_str!("./benchmarks/pystone.py");
const STRINGS: &str = include_str!("./benchmarks/strings.py");

fn bench_cpython(b: &mut Bencher, source: &str) {
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

fn bench_rustpy(b: &mut Bencher, name: &str, source: &str) {
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

pub fn benchmark_file(c: &mut Criterion, name: &str, contents: &str) {
    let mut group = c.benchmark_group(name);
    group.bench_function(BenchmarkId::new("cpython", name), |b| bench_cpython(b, black_box(contents)));
    group.bench_function(BenchmarkId::new("rustpython", name), |b| bench_rustpy(b, name, black_box(contents)));
    group.finish();
}

pub fn benchmark_pystone(c: &mut Criterion) {
    let mut group = c.benchmark_group("pystone");
    // Default is 50_000. This takes a while, so reduce it to 30k.
    for idx in (10_000..=30_000).step_by(10_000) {
        let code_with_loops = format!("LOOPS = {}\n{}", idx, PYSTONE);
        let code_str = code_with_loops.as_str();

        group.throughput(Throughput::Elements(idx as u64));
        group.bench_function(BenchmarkId::new("cpython", idx), |b| bench_cpython(b, black_box(code_str)));
        group.bench_function(BenchmarkId::new("rustpython", idx), |b| bench_rustpy(b, "pystone", black_box(code_str)));
    }
    group.finish();
}

pub fn criterion_benchmark(c: &mut Criterion) {
    benchmark_pystone(c);
    benchmark_file(c, "nbody.py", NBODY);
    benchmark_file(c, "mandlebrot.py", MANDELBROT);
    benchmark_file(c, "strings.py", STRINGS);
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
