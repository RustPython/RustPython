//! Implementation in line with the python `weakref` module.
//!
//! See also:
//! - [python weakref module](https://docs.python.org/3/library/weakref.html)
//! - [rust weak struct](https://doc.rust-lang.org/std/rc/struct.Weak.html)
//!

use super::super::pyobject::{PyContext, PyObjectRef};

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    py_module!(ctx, "_weakref", {
        "ref" => ctx.weakref_type()
    })
}
