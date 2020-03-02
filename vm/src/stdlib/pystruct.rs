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

use crate::function::Args;
use crate::obj::{
    objbytes::PyBytesRef, objstr::PyStringRef, objtuple::PyTuple, objtype::PyClassRef,
};
use crate::pyobject::{Either, PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject};
use crate::VirtualMachine;

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

impl FormatCode {
    fn size(&self) -> usize {
        match self.code {
            'b' | 'B' | '?' => 1,
            'h' | 'H' => 2,
            'i' | 'l' | 'I' | 'L' | 'f' => 4,
            'q' | 'Q' | 'd' => 8,
            c => {
                panic!("Unsupported format code {:?}", c);
            }
        }
    }
}

#[derive(Debug)]
struct FormatSpec {
    endianness: Endianness,
    codes: Vec<FormatCode>,
}

impl FormatSpec {
    fn parse(fmt: &str) -> Result<FormatSpec, String> {
        let mut chars = fmt.chars().peekable();

        // First determine "<", ">","!" or "="
        let endianness = parse_endiannes(&mut chars);

        // Now, analyze struct string furter:
        let codes = parse_format_codes(&mut chars)?;

        Ok(FormatSpec { endianness, codes })
    }

    fn pack(&self, args: &[PyObjectRef], vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        if self.codes.len() != args.len() {
            return Err(vm.new_exception_msg(
                vm.try_class("_struct", "error")?,
                format!(
                    "pack expected {} items for packing (got {})",
                    self.codes.len(),
                    args.len()
                ),
            ));
        }

        // Create data vector:
        let mut data = Vec::<u8>::new();
        // Loop over all opcodes:
        for (code, arg) in self.codes.iter().zip(args.iter()) {
            debug!("code: {:?}", code);
            match self.endianness {
                Endianness::Little => {
                    pack_item::<byteorder::LittleEndian>(vm, code, arg, &mut data)?
                }
                Endianness::Big => pack_item::<byteorder::BigEndian>(vm, code, arg, &mut data)?,
                Endianness::Network => {
                    pack_item::<byteorder::NetworkEndian>(vm, code, arg, &mut data)?
                }
                Endianness::Native => {
                    pack_item::<byteorder::NativeEndian>(vm, code, arg, &mut data)?
                }
            }
        }

        Ok(data)
    }

    fn unpack(&self, data: &[u8], vm: &VirtualMachine) -> PyResult<PyTuple> {
        let mut rdr = Cursor::new(data);

        let mut items = vec![];
        for code in &self.codes {
            debug!("unpack code: {:?}", code);
            let item = match self.endianness {
                Endianness::Little => unpack_code::<byteorder::LittleEndian>(vm, &code, &mut rdr)?,
                Endianness::Big => unpack_code::<byteorder::BigEndian>(vm, &code, &mut rdr)?,
                Endianness::Network => {
                    unpack_code::<byteorder::NetworkEndian>(vm, &code, &mut rdr)?
                }
                Endianness::Native => unpack_code::<byteorder::NativeEndian>(vm, &code, &mut rdr)?,
            };
            items.push(item);
        }

        Ok(PyTuple::from(items))
    }

    fn size(&self) -> usize {
        self.codes.iter().map(FormatCode::size).sum()
    }
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

fn struct_pack(fmt: PyStringRef, args: Args, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
    let format_spec = FormatSpec::parse(fmt.as_str()).map_err(|e| vm.new_value_error(e))?;
    format_spec.pack(args.as_ref(), vm)
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

fn struct_unpack(fmt: PyStringRef, buffer: PyBytesRef, vm: &VirtualMachine) -> PyResult<PyTuple> {
    let fmt_str = fmt.as_str();
    let format_spec = FormatSpec::parse(fmt_str).map_err(|e| vm.new_value_error(e))?;
    format_spec.unpack(buffer.get_value(), vm)
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

fn struct_calcsize(fmt: Either<PyStringRef, PyBytesRef>, vm: &VirtualMachine) -> PyResult<usize> {
    let parsed = match fmt {
        Either::A(string) => FormatSpec::parse(string.as_str()),
        Either::B(bytes) => FormatSpec::parse(std::str::from_utf8(&bytes).unwrap()),
    };
    let format_spec = parsed.map_err(|e| vm.new_value_error(e))?;
    Ok(format_spec.size())
}

#[pyclass(name = "Struct")]
#[derive(Debug)]
struct PyStruct {
    spec: FormatSpec,
    fmt_str: PyStringRef,
}

impl PyValue for PyStruct {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("_struct", "Struct")
    }
}

#[pyimpl]
impl PyStruct {
    #[pyslot]
    fn tp_new(cls: PyClassRef, fmt_str: PyStringRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        let spec = FormatSpec::parse(fmt_str.as_str()).map_err(|e| vm.new_value_error(e))?;

        PyStruct { spec, fmt_str }.into_ref_with_type(vm, cls)
    }

    #[pyproperty]
    fn format(&self) -> PyStringRef {
        self.fmt_str.clone()
    }
    #[pyproperty]
    fn size(&self) -> usize {
        self.spec.size()
    }

    #[pymethod]
    fn pack(&self, args: Args, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        self.spec.pack(args.as_ref(), vm)
    }
    #[pymethod]
    fn unpack(&self, data: PyBytesRef, vm: &VirtualMachine) -> PyResult<PyTuple> {
        self.spec.unpack(data.get_value(), vm)
    }
}

// seems weird that this is part of the "public" API, but whatever
// TODO: implement a format code->spec cache like CPython does?
fn clearcache() {}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let struct_error = ctx.new_class("struct.error", ctx.exceptions.exception_type.clone());

    py_module!(vm, "_struct", {
        "_clearcache" => ctx.new_function(clearcache),
        "pack" => ctx.new_function(struct_pack),
        "unpack" => ctx.new_function(struct_unpack),
        "calcsize" => ctx.new_function(struct_calcsize),
        "error" => struct_error,
        "Struct" => PyStruct::make_class(ctx),
    })
}
