/* String builtin module
 *
 *
 */

use super::super::pyobject::{PyContext, PyObjectRef};

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let ascii_uppercase_str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let ascii_lowercase_str = "abcdefghijklmnopqrstuvwxyz";
    py_item!(ctx, mod string {
        // Constants:
        let ascii_lowercase = ctx.new_str(
            ascii_lowercase_str.to_string()
        );
        let ascii_uppercase = ctx.new_str(
            ascii_uppercase_str.to_string()
        );
        let ascii_letters = ctx.new_str(
            format!("{}{}", ascii_lowercase_str, ascii_uppercase_str)
        );
        let digits = ctx.new_str(
            "0123456789".to_string()
        );
        let hexdigits = ctx.new_str(
            "0123456789abcdefABCDEF".to_string()
        );
        let octdigits = ctx.new_str(
            "01234567".to_string()
        );
        let punctuation = ctx.new_str(
            "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~".to_string()
        );
        /* FIXME
        let whitespace = " \t\n\r\x0b\x0c".to_string();
        let printable = format!("{}{}{}{}", digits, ascii_letters, punctuation, whitespace);
        */
    })
}
