# Architecture

This document contains a high-level architectural overview of RustPython, thus it's very well-suited to get to know [the codebase](https://github.com/RustPython/RustPython).

RustPython is an Open Source (MIT) Python 3 interpreter written in Rust, available as both a library and a shell environment. Using Rust to implement the Python interpreter enables Python to be used as a programming language for Rust applications. Moreover, it allows Python to be immediately compiled in the browser using WebAssembly, meaning that anyone could easily run their Python code in the browser. For a more detailed introduction to RustPython, have a look at [this blog post](https://2021.desosa.nl/projects/rustpython/posts/vision/).

RustPython consists of several components which are described in the section below. Take a look at [this video](https://www.youtube.com/watch?v=nJDY9ASuiLc&t=213s) for a brief walk-through of the components of RustPython. For a more elaborate introduction to one of these components, the parser, see [this blog post](https://rustpython.github.io/2020/04/02/thing-explainer-parser.html) for more information.

Have a look at these websites for a demo of RustPython running in the browser using WebAssembly:

- [https://rustpython.github.io/demo/](https://rustpython.github.io/demo/)
- [https://rustpython.github.io/demo/notebook/](https://rustpython.github.io/demo/notebook/)

If, after reading this, you are interested to contribute to RustPython, take a look at these sources to get to know how and where to start:

- [https://github.com/RustPython/RustPython/blob/master/DEVELOPMENT.md](https://github.com/RustPython/RustPython/blob/master/DEVELOPMENT.md)
- [https://rustpython.github.io/guideline/2020/04/04/how-to-contribute-by-cpython-unittest.html](https://rustpython.github.io/guideline/2020/04/04/how-to-contribute-by-cpython-unittest.html)

## Bird's eye view

A high-level overview of the workings of RustPython is visible in the figure below, showing how Python source files are interpreted.

![overview.png](overview.png)

Main architecture of RustPython.

The RustPython interpreter can be decoupled into three distinct modules: the parser, compiler and VM. 

1. The parser is responsible for converting the source code into tokens, and deriving an Abstract Syntax Tree (AST) from it.
2. The compiler converts the generated AST to bytecode.
3. The VM then executes the bytecode given user supplied input parameters and returns its result.

## Entry points

The entry points are as follows:

- The 'main' method of the application: `run`, located in `src/lib.rs:70`. This method will call the compiler, which in turn will call the parser, and pass the compiled bytecode to the VM.
- Parser: `parse`, located in `parser/src/parser.rs:74`.
- Compiler: `compile_top`, located in `compiler/src/compiler.rs:90`.
- VM: `run_code_obj`, located in `vm/src/rm.rs:459`.

## Modules

### Bytecode

This module (single file at moment) holds the representation of bytecode for RustPython
For instance, the following function
```python
def f(x):
    return x + 1
```
Is compiled to
```rust

```

### Compiler

Python compilation to bytecode. The interface is exposed through the porcelain crate with `compile` and `compile_symtable`, while the inner workings are defined in compiler/src/compile.rs, which is mostly an adaptation of the CPython implementation.

### Derive

Rust language extensions and macros specific to rustpython. Here we can find the definition of `PyModule` and `PyClass` along with usefule macros like `py_compile!`

### Parser

All the functionality required for parsing python sourcecode to an abstract syntax tree (AST)
1. Lexical Analysis
2. Parsing

As Python heavily relies on whitespace and indentation to organize code, the crate used for parsing, [LALRPOP](https://github.com/lalrpop/lalrpop), the raw source code is first preprocessed by a lexer which makes sure that `Indent` and `Dedent` tokens occur at the correct locations. Then, the parser recursively generates an AST for the code which can be processed by the compiler.

### Lib

python side of standard libary

### Lib/test

CPython test suite

### VM

Python VM

- builtins: all the builtin functions
- compile: from AST to bytecode?
- obj: Builtin types
- stdlib: the parts of the standard library implemented in rust

### src

The RustPython executable is implemented here


### py_code_object

CPython bytecode → rustpython bytecode

### WASM

crate for WebAssembly build

### extra_tests

extra integration tests