/*
 * Some functions are built into the interpreter, for example the print
 * function is such a builtin function.
 *
 * Inspiration can be found here:
 * https://github.com/python/cpython/blob/master/Python/bltinmodule.c
 */

use super::pyobject::PyObjectRef;
use std::io::{self, Write};

pub fn fill_scope() {
    // scope[String::from("print")] = print;
}

pub fn print(args: Vec<PyObjectRef>) {
    // println!("Woot: {:?}", args);
    trace!("print called with {:?}", args);
    for a in args {
        print!("{} ", a.borrow_mut().str());
    }
    println!();
    io::stdout().flush().unwrap();
}

fn any() {}

fn all() {}
