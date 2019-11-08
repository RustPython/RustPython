/*
 * Python struct module.
 *
 * Docs: https://docs.python.org/3/library/struct.html
 *
 * renamed to pystruct since struct is a rust keyword.
 *
 * Use this rust module to do byte packing:
 * https://docs.rs/byteorder/1.2.6/byteorder/
 */

use std::io::{Cursor, Read, Write};
use std::iter::Peekable;

use byteorder::{ReadBytesExt, WriteBytesExt};

use crate::function::PyFuncArgs;
use crate::obj::{objbytes, objstr, objtype};
use crate::pyobject::{PyObjectRef, PyResult, TryFromObject};
use crate::VirtualMachine;

#[derive(Debug)]
struct FormatSpec {
    endianness: Endianness,
    codes: Vec<FormatCode>,
}

#[derive(Debug)]
enum Endianness {
    Native,
    Little,
    Big,
    Network,
}

#[derive(Debug)]
struct FormatCode {
    code: char,
}

fn parse_format_string(fmt: String) -> Result<FormatSpec, String> {
    let mut chars = fmt.chars().peekable();

    // First determine "<", ">","!" or "="
    let endianness = parse_endiannes(&mut chars);

    // Now, analyze struct string furter:
    let codes = parse_format_codes(&mut chars)?;

    Ok(FormatSpec { endianness, codes })
}

/// Parse endianness
/// See also: https://docs.python.org/3/library/struct.html?highlight=struct#byte-order-size-and-alignment
fn parse_endiannes<I>(chars: &mut Peekable<I>) -> Endianness
where
    I: Sized + Iterator<Item = char>,
{
    match chars.peek() {
        Some('@') => {
            chars.next().unwrap();
            Endianness::Native
        }
        Some('=') => {
            chars.next().unwrap();
            Endianness::Native
        }
        Some('<') => {
            chars.next().unwrap();
            Endianness::Little
        }
        Some('>') => {
            chars.next().unwrap();
            Endianness::Big
        }
        Some('!') => {
            chars.next().unwrap();
            Endianness::Network
        }
        _ => Endianness::Native,
    }
}

fn parse_format_codes<I>(chars: &mut Peekable<I>) -> Result<Vec<FormatCode>, String>
where
    I: Sized + Iterator<Item = char>,
{
    let mut codes = vec![];
    while chars.peek().is_some() {
        // determine repeat operator:
        let repeat = match chars.peek() {
            Some('0'..='9') => {
                let mut repeat = 0;
                while let Some('0'..='9') = chars.peek() {
                    if let Some(c) = chars.next() {
                        let current_digit = c.to_digit(10).unwrap();
                        repeat = repeat * 10 + current_digit;
                    }
                }
                Some(repeat)
            }
            _ => None,
        };

        // determine format char:
        let c = chars.next();
        match c {
            Some(c) if is_supported_format_character(c) => {
                if let Some(repeat) = repeat {
                    for _ in 0..repeat {
                        codes.push(FormatCode { code: c })
                    }
                } else {
                    codes.push(FormatCode { code: c })
                }
            }
            _ => return Err(format!("Illegal format code {:?}", c)),
        }
    }

    Ok(codes)
}

fn is_supported_format_character(c: char) -> bool {
    match c {
        'b' | 'B' | 'h' | 'H' | 'i' | 'I' | 'l' | 'L' | 'q' | 'Q' | 'f' | 'd' => true,
        _ => false,
    }
}

macro_rules! make_pack_no_endianess {
    ($T:ty) => {
        paste::item! {
            fn [<pack_ $T>](vm: &VirtualMachine, arg: &PyObjectRef, data: &mut dyn Write) -> PyResult<()> {
                let v = $T::try_from_object(vm, arg.clone())?;
                data.[<write_$T>](v).unwrap();
                Ok(())
            }
        }
    };
}

macro_rules! make_pack_with_endianess {
    ($T:ty) => {
        paste::item! {
            fn [<pack_ $T>]<Endianness>(vm: &VirtualMachine, arg: &PyObjectRef, data: &mut dyn Write) -> PyResult<()>
            where
                Endianness: byteorder::ByteOrder,
            {
                let v = $T::try_from_object(vm, arg.clone())?;
                data.[<write_$T>]::<Endianness>(v).unwrap();
                Ok(())
            }
        }
    };
}

make_pack_no_endianess!(i8);
make_pack_no_endianess!(u8);
make_pack_with_endianess!(i16);
make_pack_with_endianess!(u16);
make_pack_with_endianess!(i32);
make_pack_with_endianess!(u32);
make_pack_with_endianess!(i64);
make_pack_with_endianess!(u64);
make_pack_with_endianess!(f32);
make_pack_with_endianess!(f64);

fn pack_bool(vm: &VirtualMachine, arg: &PyObjectRef, data: &mut dyn Write) -> PyResult<()> {
    let v = if bool::try_from_object(vm, arg.clone())? {
        1
    } else {
        0
    };
    data.write_u8(v).unwrap();
    Ok(())
}

fn pack_item<Endianness>(
    vm: &VirtualMachine,
    code: &FormatCode,
    arg: &PyObjectRef,
    data: &mut dyn Write,
) -> PyResult<()>
where
    Endianness: byteorder::ByteOrder,
{
    match code.code {
        'b' => pack_i8(vm, arg, data)?,
        'B' => pack_u8(vm, arg, data)?,
        '?' => pack_bool(vm, arg, data)?,
        'h' => pack_i16::<Endianness>(vm, arg, data)?,
        'H' => pack_u16::<Endianness>(vm, arg, data)?,
        'i' | 'l' => pack_i32::<Endianness>(vm, arg, data)?,
        'I' | 'L' => pack_u32::<Endianness>(vm, arg, data)?,
        'q' => pack_i64::<Endianness>(vm, arg, data)?,
        'Q' => pack_u64::<Endianness>(vm, arg, data)?,
        'f' => pack_f32::<Endianness>(vm, arg, data)?,
        'd' => pack_f64::<Endianness>(vm, arg, data)?,
        c => {
            panic!("Unsupported format code {:?}", c);
        }
    }
    Ok(())
}

fn struct_pack(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    if args.args.is_empty() {
        Err(vm.new_type_error(format!(
            "Expected at least 1 argument (got: {})",
            args.args.len()
        )))
    } else {
        let fmt_arg = args.args[0].clone();
        if objtype::isinstance(&fmt_arg, &vm.ctx.str_type()) {
            let fmt_str = objstr::get_value(&fmt_arg);

            let format_spec = parse_format_string(fmt_str).map_err(|e| vm.new_value_error(e))?;

            if format_spec.codes.len() + 1 == args.args.len() {
                // Create data vector:
                let mut data = Vec::<u8>::new();
                // Loop over all opcodes:
                for (code, arg) in format_spec.codes.iter().zip(args.args.iter().skip(1)) {
                    debug!("code: {:?}", code);
                    match format_spec.endianness {
                        Endianness::Little => {
                            pack_item::<byteorder::LittleEndian>(vm, code, arg, &mut data)?
                        }
                        Endianness::Big => {
                            pack_item::<byteorder::BigEndian>(vm, code, arg, &mut data)?
                        }
                        Endianness::Network => {
                            pack_item::<byteorder::NetworkEndian>(vm, code, arg, &mut data)?
                        }
                        Endianness::Native => {
                            pack_item::<byteorder::NativeEndian>(vm, code, arg, &mut data)?
                        }
                    }
                }

                Ok(vm.ctx.new_bytes(data))
            } else {
                Err(vm.new_type_error(format!(
                    "Expected {} arguments (got: {})",
                    format_spec.codes.len() + 1,
                    args.args.len()
                )))
            }
        } else {
            Err(vm.new_type_error("First argument must be of str type".to_string()))
        }
    }
}

fn unpack_i8(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult {
    match rdr.read_i8() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_int(v)),
    }
}

fn unpack_u8(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult {
    match rdr.read_u8() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_int(v)),
    }
}

fn unpack_bool(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult {
    match rdr.read_u8() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_bool(v > 0)),
    }
}

fn unpack_i16<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
where
    Endianness: byteorder::ByteOrder,
{
    match rdr.read_i16::<Endianness>() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_int(v)),
    }
}

fn unpack_u16<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
where
    Endianness: byteorder::ByteOrder,
{
    match rdr.read_u16::<Endianness>() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_int(v)),
    }
}

fn unpack_i32<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
where
    Endianness: byteorder::ByteOrder,
{
    match rdr.read_i32::<Endianness>() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_int(v)),
    }
}

fn unpack_u32<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
where
    Endianness: byteorder::ByteOrder,
{
    match rdr.read_u32::<Endianness>() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_int(v)),
    }
}

fn unpack_i64<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
where
    Endianness: byteorder::ByteOrder,
{
    match rdr.read_i64::<Endianness>() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_int(v)),
    }
}

fn unpack_u64<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
where
    Endianness: byteorder::ByteOrder,
{
    match rdr.read_u64::<Endianness>() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_int(v)),
    }
}

fn unpack_f32<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
where
    Endianness: byteorder::ByteOrder,
{
    match rdr.read_f32::<Endianness>() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_float(f64::from(v))),
    }
}

fn unpack_f64<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
where
    Endianness: byteorder::ByteOrder,
{
    match rdr.read_f64::<Endianness>() {
        Err(err) => panic!("Error in reading {:?}", err),
        Ok(v) => Ok(vm.ctx.new_float(v)),
    }
}

fn struct_unpack(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (fmt, Some(vm.ctx.str_type())),
            (buffer, Some(vm.ctx.bytes_type()))
        ]
    );

    let fmt_str = objstr::get_value(&fmt);

    let format_spec = parse_format_string(fmt_str).map_err(|e| vm.new_value_error(e))?;
    let data = objbytes::get_value(buffer).to_vec();
    let mut rdr = Cursor::new(data);

    let mut items = vec![];
    for code in format_spec.codes {
        debug!("unpack code: {:?}", code);
        let item = match format_spec.endianness {
            Endianness::Little => unpack_code::<byteorder::LittleEndian>(vm, &code, &mut rdr)?,
            Endianness::Big => unpack_code::<byteorder::BigEndian>(vm, &code, &mut rdr)?,
            Endianness::Network => unpack_code::<byteorder::NetworkEndian>(vm, &code, &mut rdr)?,
            Endianness::Native => unpack_code::<byteorder::NativeEndian>(vm, &code, &mut rdr)?,
        };
        items.push(item);
    }

    Ok(vm.ctx.new_tuple(items))
}

fn unpack_code<Endianness>(vm: &VirtualMachine, code: &FormatCode, rdr: &mut dyn Read) -> PyResult
where
    Endianness: byteorder::ByteOrder,
{
    match code.code {
        'b' => unpack_i8(vm, rdr),
        'B' => unpack_u8(vm, rdr),
        '?' => unpack_bool(vm, rdr),
        'h' => unpack_i16::<Endianness>(vm, rdr),
        'H' => unpack_u16::<Endianness>(vm, rdr),
        'i' | 'l' => unpack_i32::<Endianness>(vm, rdr),
        'I' | 'L' => unpack_u32::<Endianness>(vm, rdr),
        'q' => unpack_i64::<Endianness>(vm, rdr),
        'Q' => unpack_u64::<Endianness>(vm, rdr),
        'f' => unpack_f32::<Endianness>(vm, rdr),
        'd' => unpack_f64::<Endianness>(vm, rdr),
        c => {
            panic!("Unsupported format code {:?}", c);
        }
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let struct_error = ctx.new_class("struct.error", ctx.object());

    py_module!(vm, "struct", {
        "pack" => ctx.new_rustfunc(struct_pack),
        "unpack" => ctx.new_rustfunc(struct_unpack),
        "error" => struct_error,
    })
}
