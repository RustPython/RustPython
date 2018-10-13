/*
 * Python struct module.
 *
 * renamed to pystruct since struct is a rust keyword.
 *
 * Use this rust module to do byte packing:
 * https://docs.rs/byteorder/1.2.6/byteorder/
 */

extern crate byteorder;
use self::byteorder::{LittleEndian, WriteBytesExt};

use super::super::obj::{objint, objstr, objtype};
use super::super::pyobject::{DictProtocol, PyContext, PyFuncArgs, PyObjectRef, PyResult};
use super::super::VirtualMachine;

#[derive(Debug)]
struct FormatCode {
    code: char,
    size: i32,
    repeat: i32,
}

// fn bp_float(v: PyObjectRef) {}

// fn bp_uint() {}

fn parse_format_string(fmt: String) -> Vec<FormatCode> {
    // First determine "<", ">","!" or "="
    // TODO

    // Now, analyze struct string furter:
    let mut codes = vec![];
    for c in fmt.chars() {
        match c {
            'I' => codes.push(FormatCode {
                code: c,
                size: 1,
                repeat: 1,
            }),
            'H' => codes.push(FormatCode {
                code: c,
                size: 1,
                repeat: 1,
            }),
            _ => {}
        }
    }
    codes
}

fn struct_pack(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    if args.args.len() < 1 {
        Err(vm.new_type_error(format!(
            "Expected at least 1 argument (got: {})",
            args.args.len()
        )))
    } else {
        let fmt_arg = args.args[0].clone();
        if objtype::isinstance(&fmt_arg, vm.ctx.str_type()) {
            let fmt_str = objstr::get_value(&fmt_arg);

            let codes = parse_format_string(fmt_str);

            if codes.len() + 1 == args.args.len() {
                // Create data vector:
                let mut data = Vec::<u8>::new();
                // Loop over all opcodes:
                for (code, arg) in codes.iter().zip(args.args.iter().skip(1)) {
                    debug!("code: {:?}", code);
                    match code.code {
                        'I' => {
                            if objtype::isinstance(&arg, vm.ctx.int_type()) {
                                let v = objint::get_value(arg) as u32;
                                match data.write_u32::<LittleEndian>(v) {
                                    Ok(_v) => {}
                                    Err(err) => panic!("Error: {:?}", err),
                                }
                            } else {
                                return Err(vm.new_type_error(format!("Expected int")));
                            }
                        }
                        'H' => {
                            if objtype::isinstance(&arg, vm.ctx.int_type()) {
                                let v = objint::get_value(arg) as u16;
                                match data.write_u16::<LittleEndian>(v) {
                                    Ok(_v) => {}
                                    Err(err) => panic!("Error: {:?}", err),
                                }
                            } else {
                                return Err(vm.new_type_error(format!("Expected int")));
                            }
                        }
                        _ => {
                            panic!("Unsupported format code");
                        }
                    }
                }

                Ok(vm.ctx.new_bytes(data))
            } else {
                Err(vm.new_type_error(format!(
                    "Expected {} arguments (got: {})",
                    codes.len() + 1,
                    args.args.len()
                )))
            }
        } else {
            Err(vm.new_type_error(format!("First argument must be of str type")))
        }
    }
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let py_mod = ctx.new_module(&"struct".to_string(), ctx.new_scope(None));
    py_mod.set_item("pack", ctx.new_rustfunc(struct_pack));
    py_mod
}
