#![feature(test)]

extern crate test;

use test::Bencher;

use rustpython_compiler::compile;
use rustpython_vm::pyobject::PyResult;
use rustpython_vm::Interpreter;

const MINIDOM: &str = include_str!("./benchmarks/minidom.py");
const NBODY: &str = include_str!("./benchmarks/nbody.py");
const MANDELBROT: &str = include_str!("./benchmarks/mandelbrot.py");
const PYSTONE: &str = include_str!("./benchmarks/pystone.py");

#[bench]
fn bench_tokenization(b: &mut Bencher) {
    use rustpython_parser::lexer::{make_tokenizer, Tok};

    let source = MINIDOM;

    b.bytes = source.len() as _;
    b.iter(|| {
        let lexer = make_tokenizer(source);
        for res in lexer {
            let _token: Tok = res.unwrap().1;
        }
    })
}

#[bench]
fn bench_rustpy_parse_to_ast(b: &mut Bencher) {
    use rustpython_parser::parser::parse_program;

    let source = MINIDOM;

    b.bytes = source.len() as _;
    b.iter(|| parse_program(source).unwrap())
}

#[bench]
fn bench_cpython_parse_to_ast(b: &mut Bencher) {
    let source = MINIDOM;

    let gil = cpython::Python::acquire_gil();
    let python = gil.python();

    let globals = None;
    let locals = cpython::PyDict::new(python);

    locals.set_item(python, "SOURCE_CODE", source).unwrap();

    let code = "compile(SOURCE_CODE, mode=\"exec\", filename=\"minidom.py\")";

    b.bytes = source.len() as _;
    b.iter(|| {
        let res: cpython::PyResult<cpython::PyObject> = python.eval(code, globals, Some(&locals));
        assert!(res.is_ok());
    })
}

fn bench_cpython(b: &mut Bencher, source: &str) {
    let gil = cpython::Python::acquire_gil();
    let python = gil.python();

    let globals = None;
    let locals = None;

    b.iter(|| {
        let res: cpython::PyResult<()> = python.run(source, globals, locals);
        assert!(res.is_ok());
    })
}

#[bench]
fn bench_cpython_nbody(b: &mut Bencher) {
    bench_cpython(b, NBODY)
}

#[bench]
fn bench_cpython_mandelbrot(b: &mut Bencher) {
    bench_cpython(b, MANDELBROT)
}

#[bench]
fn bench_cpython_pystone(b: &mut Bencher) {
    bench_cpython(b, PYSTONE)
}

fn bench_rustpy(b: &mut Bencher, name: &str, source: &str) {
    // NOTE: Take long time.
    Interpreter::default().enter(|vm| {
        let code = vm
            .compile(source, compile::Mode::Exec, name.to_owned())
            .unwrap();

        b.iter(|| {
            let scope = vm.new_scope_with_builtins();
            let res: PyResult = vm.run_code_obj(code.clone(), scope);
            vm.unwrap_pyresult(res);
        })
    })
}

#[bench]
fn bench_rustpy_nbody(b: &mut Bencher) {
    bench_rustpy(b, "nbody.py", NBODY)
}

#[bench]
fn bench_rustpy_mandelbrot(b: &mut Bencher) {
    bench_rustpy(b, "mandelbrot.py", MANDELBROT)
}

#[bench]
fn bench_rustpy_pystone(b: &mut Bencher) {
    bench_rustpy(b, "pystone.py", PYSTONE)
}
