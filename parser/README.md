# RustPython/parser

This directory has the code for python lexing, parsing and generating Abstract Syntax Trees (AST).

The steps are:
- Lexical analysis: splits the source code into tokens.
- Parsing and generating the AST: transforms those tokens into an AST. Uses `LALRPOP`, a Rust parser generator framework.

This crate is published on [https://docs.rs/rustpython-parser](https://docs.rs/rustpython-parser).

We wrote [a blog post](https://rustpython.github.io/featured/2020/03/11/thing-explainer-parser.html) with screenshots and an explanation to help you understand the steps by seeing them in action.

For more information on LALRPOP, here is a link to the [LALRPOP book](https://github.com/lalrpop/lalrpop).

There is a readme in the `src` folder with the details of each file.

## How to use

For example, one could do this:
```
  use rustpython_parser::{parser, ast};
  let python_source = "print('Hello world')";
  let python_ast = parser::parse_expression(python_source).unwrap();
```
