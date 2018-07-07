extern crate python_compiler;

use python_compiler::python_compiler::compile;

fn main() {
    println!("{:?}", compile());
}
