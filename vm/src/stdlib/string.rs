/* String builtin module
 *
 *
 */

use super::super::pyobject::{PyContext, PyObjectRef};

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let py_mod = ctx.new_module(&"string".to_string(), ctx.new_scope(None));

    let ascii_lowercase = "abcdefghijklmnopqrstuvwxyz".to_string();
    let ascii_uppercase = "ABCDEFGHIJKLMNOPQRSTUVWXYZ".to_string();
    let ascii_letters = format!("{}{}", ascii_lowercase, ascii_uppercase);
    let digits = "0123456789".to_string();
    let hexdigits = "0123456789abcdefABCDEF".to_string();
    let octdigits = "01234567".to_string();
    let punctuation = "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~".to_string();
    /* FIXME
    let whitespace = " \t\n\r\x0b\x0c".to_string();
    let printable = format!("{}{}{}{}", digits, ascii_letters, punctuation, whitespace);
    */

    // Constants:
    ctx.set_attr(&py_mod, "ascii_letters", ctx.new_str(ascii_letters.clone()));
    ctx.set_attr(
        &py_mod,
        "ascii_lowercase",
        ctx.new_str(ascii_lowercase.clone()),
    );
    ctx.set_attr(
        &py_mod,
        "ascii_uppercase",
        ctx.new_str(ascii_uppercase.clone()),
    );
    ctx.set_attr(&py_mod, "digits", ctx.new_str(digits.clone()));
    ctx.set_attr(&py_mod, "hexdigits", ctx.new_str(hexdigits.clone()));
    ctx.set_attr(&py_mod, "octdigits", ctx.new_str(octdigits.clone()));
    // ctx.set_attr(&py_mod, "printable", ctx.new_str(printable.clone()));
    ctx.set_attr(&py_mod, "punctuation", ctx.new_str(punctuation.clone()));
    // ctx.set_attr(&py_mod, "whitespace", ctx.new_str(whitespace.clone()));

    py_mod
}
