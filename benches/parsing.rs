use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use rustpython_parser::lexer::{make_tokenizer, Tok};
use rustpython_parser::parser::parse_program;

const MINIDOM: &str = include_str!("./benchmarks/minidom.py");

pub fn criterion_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("tokenization");
    group.throughput(Throughput::Bytes(MINIDOM.len() as u64));
    group.bench_function("tokenization", |b| {
        b.iter(|| {
            let lexer = make_tokenizer(black_box(MINIDOM));
            for res in lexer {
                let _token: Tok = res.unwrap().1;
            }
        })
    });
    group.finish();

    let mut group = c.benchmark_group("parse_to_ast");
    group.throughput(Throughput::Bytes(MINIDOM.len() as u64));
    group.bench_function("rustpy", |b| {
        b.iter(|| parse_program(black_box(MINIDOM)).unwrap())
    });
    group.bench_function("cpython", |b| {
        let gil = cpython::Python::acquire_gil();
        let python = gil.python();

        let globals = None;
        let locals = cpython::PyDict::new(python);

        locals.set_item(python, "SOURCE_CODE", MINIDOM).unwrap();

        let code = "compile(SOURCE_CODE, mode=\"exec\", filename=\"minidom.py\")";
        b.iter(|| {
            let res: cpython::PyResult<cpython::PyObject> =
                python.eval(black_box(code), globals, Some(&locals));
            assert!(res.is_ok());
        })
    });
    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
