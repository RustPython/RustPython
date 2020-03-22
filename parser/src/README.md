# RustPython/parser/src

**lib.rs**   
This is the crate's root.

**lexer.rs**   
This module takes care of lexing python source text. This means source code is translated into separate tokens.

**parser.rs**   
A python parsing module. Use this module to parse python code into an AST. There are three ways to parse python code. You could parse a whole program, a single statement, or a single expression.

**ast.rs**   
 Implements abstract syntax tree (AST) nodes for the python language. Roughly equivalent to [the python AST](https://docs.python.org/3/library/ast.html).

**python.lalrpop**   
Python grammar.

**token.rs**   
Different token definitions. Loosely based on token.h from CPython source.

**errors.rs**   
Define internal parse error types. The goal is to provide a matching and a safe error API, masking errors from LALR.

**fstring.rs**   
Format strings.

**function.rs**   
Collection of functions for parsing parameters, arguments.

**location.rs**   
Datatypes to support source location information.

**mode.rs**   
Execution mode check. Allowed modes are `exec`, `eval` or `single`.
