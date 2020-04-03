#![feature(test)]

extern crate cpython;
extern crate rustpython_parser;
extern crate rustpython_vm;
extern crate test;

use rustpython_compiler::compile;
use rustpython_vm::pyobject::PyResult;
use rustpython_vm::VirtualMachine;

#[bench]
fn bench_tokenization(b: &mut test::Bencher) {
    use rustpython_parser::lexer::{make_tokenizer, Tok};

    let source = include_str!("./benchmarks/minidom.py");

    b.bytes = source.len() as _;
    b.iter(|| {
        let lexer = make_tokenizer(source);
        for res in lexer {
            let _token: Tok = res.unwrap().1;
        }
    })
}

#[bench]
fn bench_rustpy_parse_to_ast(b: &mut test::Bencher) {
    use rustpython_parser::parser::parse_program;

    let source = include_str!("./benchmarks/minidom.py");

    b.bytes = source.len() as _;
    b.iter(|| parse_program(source).unwrap())
}

#[bench]
fn bench_cpython_parse_to_ast(b: &mut test::Bencher) {
    let source = include_str!("./benchmarks/minidom.py");

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

#[bench]
fn bench_cpython_nbody(b: &mut test::Bencher) {
    let source = include_str!("./benchmarks/nbody.py");

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
fn bench_cpython_mandelbrot(b: &mut test::Bencher) {
    let source = include_str!("./benchmarks/mandelbrot.py");

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
fn bench_rustpy_nbody(b: &mut test::Bencher) {
    // NOTE: Take long time.
    let source = include_str!("./benchmarks/nbody.py");

    let vm = VirtualMachine::default();

    let code = vm
        .compile(source, compile::Mode::Exec, "<stdin>".to_owned())
        .unwrap();

    b.iter(|| {
        let scope = vm.new_scope_with_builtins();
        let res: PyResult = vm.run_code_obj(code.clone(), scope);
        vm.unwrap_pyresult(res);
    })
}

#[bench]
fn bench_rustpy_mandelbrot(b: &mut test::Bencher) {
    // NOTE: Take long time.
    let source = include_str!("./benchmarks/mandelbrot.py");

    let vm = VirtualMachine::default();

    let code = vm
        .compile(source, compile::Mode::Exec, "<stdin>".to_owned())
        .unwrap();

    b.iter(|| {
        let scope = vm.new_scope_with_builtins();
        let res: PyResult = vm.run_code_obj(code.clone(), scope);
        vm.unwrap_pyresult(res);
    })
}
