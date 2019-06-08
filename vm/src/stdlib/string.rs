/* String builtin module
 *
 *
 */

use crate::pyobject::PyObjectRef;
use crate::vm::VirtualMachine;

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

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
    py_module!(vm, "string", {
        "ascii_letters" => ctx.new_str(ascii_letters),
        "ascii_lowercase" => ctx.new_str(ascii_lowercase),
        "ascii_uppercase" => ctx.new_str(ascii_uppercase),
        "digits" => ctx.new_str(digits),
        "hexdigits" => ctx.new_str(hexdigits),
        "octdigits" => ctx.new_str(octdigits),
        // "printable", ctx.new_str(printable)
        "punctuation" => ctx.new_str(punctuation)
        // "whitespace", ctx.new_str(whitespace)
    })
}
