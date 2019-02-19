//! Implementation in line with the python `weakref` module.
//!
//! See also:
//! - [python weakref module](https://docs.python.org/3/library/weakref.html)
//! - [rust weak struct](https://doc.rust-lang.org/std/rc/struct.Weak.html)
//!

use super::super::pyobject::{PyContext, PyObjectRef};

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let py_mod = ctx.new_module("_weakref", ctx.new_scope(None));

    ctx.set_attr(&py_mod, "ref", ctx.weakref_type());
    py_mod
}
