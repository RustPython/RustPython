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

use crate::pyobject::PyObjectRef;
use crate::VirtualMachine;

#[pymodule]
mod _struct {
    use byteorder::{ReadBytesExt, WriteBytesExt};
    use num_bigint::BigInt;
    use num_traits::ToPrimitive;
    use std::io::{Cursor, Read, Write};
    use std::iter::Peekable;

    use crate::exceptions::PyBaseExceptionRef;
    use crate::function::Args;
    use crate::obj::{
        objbool::IntoPyBool, objbytes::PyBytesRef, objstr::PyString, objstr::PyStringRef,
        objtuple::PyTuple, objtype::PyClassRef,
    };
    use crate::pyobject::{
        Either, PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject,
    };
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
        repeat: u32,
        code: char,
    }

    impl FormatCode {
        fn unit_size(&self) -> usize {
            match self.code {
                'x' | 'c' | 'b' | 'B' | '?' | 's' | 'p' => 1,
                'h' | 'H' => 2,
                'i' | 'l' | 'I' | 'L' | 'f' => 4,
                'q' | 'Q' | 'd' => 8,
                'n' | 'N' | 'P' => std::mem::size_of::<usize>(),
                c => {
                    panic!("Unsupported format code {:?}", c);
                }
            }
        }

        fn size(&self) -> usize {
            self.unit_size() * self.repeat as usize
        }

        fn arg_count(&self) -> usize {
            match self.code {
                'x' => 0,
                's' | 'p' => 1,
                _ => self.repeat as usize,
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
            let arg_count: usize = self.codes.iter().map(|c| c.arg_count()).sum();
            if arg_count != args.len() {
                return Err(new_struct_error(
                    vm,
                    format!(
                        "pack expected {} items for packing (got {})",
                        self.codes.len(),
                        args.len()
                    ),
                ));
            }

            // Create data vector:
            let mut data = Vec::<u8>::new();
            let mut arg_idx = 0;
            // Loop over all opcodes:
            for code in self.codes.iter() {
                debug!("code: {:?}", code);
                let pack_item = match self.endianness {
                    Endianness::Little => pack_item::<byteorder::LittleEndian>,
                    Endianness::Big => pack_item::<byteorder::BigEndian>,
                    Endianness::Network => pack_item::<byteorder::NetworkEndian>,
                    Endianness::Native => pack_item::<byteorder::NativeEndian>,
                };
                arg_idx += pack_item(vm, code, &args[arg_idx..], &mut data)?;
            }

            Ok(data)
        }

        fn unpack(&self, data: &[u8], vm: &VirtualMachine) -> PyResult<PyTuple> {
            if self.size() != data.len() {
                return Err(new_struct_error(
                    vm,
                    format!("unpack requires a buffer of {} bytes", self.size()),
                ));
            }

            let mut rdr = Cursor::new(data);
            let mut items = vec![];
            for code in &self.codes {
                debug!("unpack code: {:?}", code);
                match self.endianness {
                    Endianness::Little => {
                        unpack_code::<byteorder::LittleEndian>(vm, &code, &mut rdr, &mut items)?
                    }
                    Endianness::Big => {
                        unpack_code::<byteorder::BigEndian>(vm, &code, &mut rdr, &mut items)?
                    }
                    Endianness::Network => {
                        unpack_code::<byteorder::NetworkEndian>(vm, &code, &mut rdr, &mut items)?
                    }
                    Endianness::Native => {
                        unpack_code::<byteorder::NativeEndian>(vm, &code, &mut rdr, &mut items)?
                    }
                };
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
                    repeat
                }
                _ => 1,
            };

            // determine format char:
            let c = chars.next();
            match c {
                Some(c) if is_supported_format_character(c) => {
                    codes.push(FormatCode { repeat, code: c })
                }
                _ => return Err(format!("Illegal format code {:?}", c)),
            }
        }

        Ok(codes)
    }

    fn is_supported_format_character(c: char) -> bool {
        match c {
            'x' | 'c' | 'b' | 'B' | '?' | 'h' | 'H' | 'i' | 'I' | 'l' | 'L' | 'q' | 'Q' | 'n'
            | 'N' | 'f' | 'd' | 's' | 'p' | 'P' => true,
            _ => false,
        }
    }

    fn get_int_or_index<T>(vm: &VirtualMachine, arg: &PyObjectRef) -> PyResult<T>
    where
        T: TryFromObject,
    {
        match vm.to_index(arg) {
            Some(index) => Ok(T::try_from_object(vm, index?.into_object())?),
            None => Err(new_struct_error(
                vm,
                "required argument is not an integer".to_owned(),
            )),
        }
    }

    macro_rules! make_pack_no_endianess {
    ($T:ty) => {
        paste::item! {
            fn [<pack_ $T>](vm: &VirtualMachine, arg: &PyObjectRef, data: &mut dyn Write) -> PyResult<()> {
                data.[<write_$T>](get_int_or_index(vm, arg)?).unwrap();
                Ok(())
            }
        }
    };
}

    macro_rules! make_pack_with_endianess_int {
    ($T:ty) => {
        paste::item! {
            fn [<pack_ $T>]<Endianness>(vm: &VirtualMachine, arg: &PyObjectRef, data: &mut dyn Write) -> PyResult<()>
            where
                Endianness: byteorder::ByteOrder,
            {
                data.[<write_$T>]::<Endianness>(get_int_or_index(vm, arg)?).unwrap();
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
    make_pack_with_endianess_int!(i16);
    make_pack_with_endianess_int!(u16);
    make_pack_with_endianess_int!(i32);
    make_pack_with_endianess_int!(u32);
    make_pack_with_endianess_int!(i64);
    make_pack_with_endianess_int!(u64);
    make_pack_with_endianess!(f32);
    make_pack_with_endianess!(f64);

    fn pack_bool(vm: &VirtualMachine, arg: &PyObjectRef, data: &mut dyn Write) -> PyResult<()> {
        let v = if IntoPyBool::try_from_object(vm, arg.clone())?.to_bool() {
            1
        } else {
            0
        };
        data.write_u8(v).unwrap();
        Ok(())
    }

    fn pack_isize<Endianness>(
        vm: &VirtualMachine,
        arg: &PyObjectRef,
        data: &mut dyn Write,
    ) -> PyResult<()>
    where
        Endianness: byteorder::ByteOrder,
    {
        let v: isize = get_int_or_index(vm, arg)?;
        match std::mem::size_of::<isize>() {
            8 => data.write_i64::<Endianness>(v as i64).unwrap(),
            4 => data.write_i32::<Endianness>(v as i32).unwrap(),
            _ => unreachable!("unexpected architecture"),
        }
        Ok(())
    }

    fn pack_usize<Endianness>(
        vm: &VirtualMachine,
        arg: &PyObjectRef,
        data: &mut dyn Write,
    ) -> PyResult<()>
    where
        Endianness: byteorder::ByteOrder,
    {
        let v: usize = get_int_or_index(vm, arg)?;
        match std::mem::size_of::<usize>() {
            8 => data.write_u64::<Endianness>(v as u64).unwrap(),
            4 => data.write_u32::<Endianness>(v as u32).unwrap(),
            _ => unreachable!("unexpected architecture"),
        }
        Ok(())
    }

    fn pack_string(
        vm: &VirtualMachine,
        arg: &PyObjectRef,
        data: &mut dyn Write,
        length: usize,
    ) -> PyResult<()> {
        let mut v = PyBytesRef::try_from_object(vm, arg.clone())?
            .get_value()
            .to_vec();
        v.resize(length, 0);
        match data.write_all(&v) {
            Ok(_) => Ok(()),
            Err(e) => Err(new_struct_error(vm, format!("{:?}", e))),
        }
    }

    fn pack_pascal(
        vm: &VirtualMachine,
        arg: &PyObjectRef,
        data: &mut dyn Write,
        length: usize,
    ) -> PyResult<()> {
        let mut v = PyBytesRef::try_from_object(vm, arg.clone())?
            .get_value()
            .to_vec();
        let string_length = std::cmp::min(std::cmp::min(v.len(), 255), length - 1);
        data.write_u8(string_length as u8).unwrap();
        v.resize(length - 1, 0);
        match data.write_all(&v) {
            Ok(_) => Ok(()),
            Err(e) => Err(new_struct_error(vm, format!("{:?}", e))),
        }
    }

    fn pack_char(vm: &VirtualMachine, arg: &PyObjectRef, data: &mut dyn Write) -> PyResult<()> {
        let v = PyBytesRef::try_from_object(vm, arg.clone())?;
        if v.len() == 1 {
            data.write_u8(v[0]).unwrap();
            Ok(())
        } else {
            Err(new_struct_error(
                vm,
                "char format requires a bytes object of length 1".to_owned(),
            ))
        }
    }

    fn pack_item<Endianness>(
        vm: &VirtualMachine,
        code: &FormatCode,
        args: &[PyObjectRef],
        data: &mut dyn Write,
    ) -> PyResult<usize>
    where
        Endianness: byteorder::ByteOrder,
    {
        let pack = match code.code {
            'c' => pack_char,
            'b' => pack_i8,
            'B' => pack_u8,
            '?' => pack_bool,
            'h' => pack_i16::<Endianness>,
            'H' => pack_u16::<Endianness>,
            'i' | 'l' => pack_i32::<Endianness>,
            'I' | 'L' => pack_u32::<Endianness>,
            'q' => pack_i64::<Endianness>,
            'Q' => pack_u64::<Endianness>,
            'n' => pack_isize::<Endianness>,
            'N' | 'P' => pack_usize::<Endianness>,
            'f' => pack_f32::<Endianness>,
            'd' => pack_f64::<Endianness>,
            's' => {
                pack_string(vm, &args[0], data, code.repeat as usize)?;
                return Ok(1);
            }
            'p' => {
                pack_pascal(vm, &args[0], data, code.repeat as usize)?;
                return Ok(1);
            }
            'x' => {
                for _ in 0..code.repeat as usize {
                    data.write_u8(0).unwrap();
                }
                return Ok(0);
            }
            c => {
                panic!("Unsupported format code {:?}", c);
            }
        };

        for arg in args.iter().take(code.repeat as usize) {
            pack(vm, arg, data)?;
        }
        Ok(code.repeat as usize)
    }

    #[pyfunction]
    fn pack(fmt: PyStringRef, args: Args, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let format_spec = FormatSpec::parse(fmt.as_str()).map_err(|e| new_struct_error(vm, e))?;
        format_spec.pack(args.as_ref(), vm)
    }

    #[inline]
    fn _unpack<F, T, G>(vm: &VirtualMachine, rdr: &mut dyn Read, read: F, transform: G) -> PyResult
    where
        F: Fn(&mut dyn Read) -> std::io::Result<T>,
        G: Fn(T) -> PyResult,
    {
        match read(rdr) {
            Ok(v) => transform(v),
            Err(_) => Err(new_struct_error(
                vm,
                format!(
                    "unpack requires a buffer of {} bytes",
                    std::mem::size_of::<T>()
                ),
            )),
        }
    }

    #[inline]
    fn unpack_int<F, T>(vm: &VirtualMachine, rdr: &mut dyn Read, read: F) -> PyResult
    where
        F: Fn(&mut dyn Read) -> std::io::Result<T>,
        T: Into<BigInt> + ToPrimitive,
    {
        _unpack(vm, rdr, read, |v| Ok(vm.ctx.new_int(v)))
    }

    fn unpack_bool(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult {
        _unpack(vm, rdr, |rdr| rdr.read_u8(), |v| Ok(vm.ctx.new_bool(v > 0)))
    }

    fn unpack_i8(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult {
        unpack_int(vm, rdr, |rdr| rdr.read_i8())
    }

    fn unpack_u8(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult {
        unpack_int(vm, rdr, |rdr| rdr.read_u8())
    }

    fn unpack_i16<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
    where
        Endianness: byteorder::ByteOrder,
    {
        unpack_int(vm, rdr, |rdr| rdr.read_i16::<Endianness>())
    }

    fn unpack_u16<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
    where
        Endianness: byteorder::ByteOrder,
    {
        unpack_int(vm, rdr, |rdr| rdr.read_u16::<Endianness>())
    }

    fn unpack_i32<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
    where
        Endianness: byteorder::ByteOrder,
    {
        unpack_int(vm, rdr, |rdr| rdr.read_i32::<Endianness>())
    }

    fn unpack_u32<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
    where
        Endianness: byteorder::ByteOrder,
    {
        unpack_int(vm, rdr, |rdr| rdr.read_u32::<Endianness>())
    }

    fn unpack_i64<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
    where
        Endianness: byteorder::ByteOrder,
    {
        unpack_int(vm, rdr, |rdr| rdr.read_i64::<Endianness>())
    }

    fn unpack_u64<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
    where
        Endianness: byteorder::ByteOrder,
    {
        unpack_int(vm, rdr, |rdr| rdr.read_u64::<Endianness>())
    }

    fn unpack_isize<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
    where
        Endianness: byteorder::ByteOrder,
    {
        match std::mem::size_of::<isize>() {
            8 => unpack_i64::<Endianness>(vm, rdr),
            4 => unpack_i32::<Endianness>(vm, rdr),
            _ => unreachable!("unexpected architecture"),
        }
    }

    fn unpack_usize<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
    where
        Endianness: byteorder::ByteOrder,
    {
        match std::mem::size_of::<usize>() {
            8 => unpack_u64::<Endianness>(vm, rdr),
            4 => unpack_u32::<Endianness>(vm, rdr),
            _ => unreachable!("unexpected architecture"),
        }
    }

    fn unpack_f32<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
    where
        Endianness: byteorder::ByteOrder,
    {
        _unpack(
            vm,
            rdr,
            |rdr| rdr.read_f32::<Endianness>(),
            |v| Ok(vm.ctx.new_float(f64::from(v))),
        )
    }

    fn unpack_f64<Endianness>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult
    where
        Endianness: byteorder::ByteOrder,
    {
        _unpack(
            vm,
            rdr,
            |rdr| rdr.read_f64::<Endianness>(),
            |v| Ok(vm.ctx.new_float(v)),
        )
    }

    fn unpack_empty(_vm: &VirtualMachine, rdr: &mut dyn Read, length: u32) {
        let mut handle = rdr.take(length as u64);
        let mut buf: Vec<u8> = Vec::new();
        let _ = handle.read_to_end(&mut buf);
    }

    fn unpack_char(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult {
        unpack_string(vm, rdr, 1)
    }

    fn unpack_string(vm: &VirtualMachine, rdr: &mut dyn Read, length: u32) -> PyResult {
        let mut handle = rdr.take(length as u64);
        let mut buf: Vec<u8> = Vec::new();
        handle.read_to_end(&mut buf).map_err(|_| {
            new_struct_error(vm, format!("unpack requires a buffer of {} bytes", length,))
        })?;
        Ok(vm.ctx.new_bytes(buf))
    }

    fn unpack_pascal(vm: &VirtualMachine, rdr: &mut dyn Read, length: u32) -> PyResult {
        let mut handle = rdr.take(length as u64);
        let mut buf: Vec<u8> = Vec::new();
        handle.read_to_end(&mut buf).map_err(|_| {
            new_struct_error(vm, format!("unpack requires a buffer of {} bytes", length,))
        })?;
        let string_length = buf[0] as usize;
        Ok(vm.ctx.new_bytes(buf[1..=string_length].to_vec()))
    }

    #[pyfunction]
    fn unpack(
        fmt: Either<PyStringRef, PyBytesRef>,
        buffer: PyBytesRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyTuple> {
        // FIXME: the given fmt must be parsed as ascii string
        // https://github.com/RustPython/RustPython/pull/1792#discussion_r387340905
        let parsed = match fmt {
            Either::A(string) => FormatSpec::parse(string.as_str()),
            Either::B(bytes) => FormatSpec::parse(std::str::from_utf8(&bytes).unwrap()),
        };
        let format_spec = parsed.map_err(|e| new_struct_error(vm, e))?;
        format_spec.unpack(buffer.get_value(), vm)
    }

    fn unpack_code<Endianness>(
        vm: &VirtualMachine,
        code: &FormatCode,
        rdr: &mut dyn Read,
        items: &mut Vec<PyObjectRef>,
    ) -> PyResult<()>
    where
        Endianness: byteorder::ByteOrder,
    {
        let unpack = match code.code {
            'b' => unpack_i8,
            'B' => unpack_u8,
            'c' => unpack_char,
            '?' => unpack_bool,
            'h' => unpack_i16::<Endianness>,
            'H' => unpack_u16::<Endianness>,
            'i' | 'l' => unpack_i32::<Endianness>,
            'I' | 'L' => unpack_u32::<Endianness>,
            'q' => unpack_i64::<Endianness>,
            'Q' => unpack_u64::<Endianness>,
            'n' => unpack_isize::<Endianness>,
            'N' => unpack_usize::<Endianness>,
            'P' => unpack_usize::<Endianness>, // FIXME: native-only
            'f' => unpack_f32::<Endianness>,
            'd' => unpack_f64::<Endianness>,
            'x' => {
                unpack_empty(vm, rdr, code.repeat);
                return Ok(());
            }
            's' => {
                items.push(unpack_string(vm, rdr, code.repeat)?);
                return Ok(());
            }
            'p' => {
                items.push(unpack_pascal(vm, rdr, code.repeat)?);
                return Ok(());
            }
            c => {
                panic!("Unsupported format code {:?}", c);
            }
        };
        for _ in 0..code.repeat {
            items.push(unpack(vm, rdr)?);
        }
        Ok(())
    }

    #[pyfunction]
    fn calcsize(fmt: Either<PyStringRef, PyBytesRef>, vm: &VirtualMachine) -> PyResult<usize> {
        // FIXME: the given fmt must be parsed as ascii string
        // https://github.com/RustPython/RustPython/pull/1792#discussion_r387340905
        let parsed = match fmt {
            Either::A(string) => FormatSpec::parse(string.as_str()),
            Either::B(bytes) => FormatSpec::parse(std::str::from_utf8(&bytes).unwrap()),
        };
        let format_spec = parsed.map_err(|e| new_struct_error(vm, e))?;
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
        fn tp_new(
            cls: PyClassRef,
            fmt: Either<PyStringRef, PyBytesRef>,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let fmt_str = match fmt {
                Either::A(s) => s,
                Either::B(b) => PyString::from(std::str::from_utf8(b.get_value()).unwrap())
                    .into_ref_with_type(vm, vm.ctx.str_type())?,
            };
            let spec = FormatSpec::parse(fmt_str.as_str()).map_err(|e| new_struct_error(vm, e))?;
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
    #[pyfunction]
    fn _clearcache() {}

    fn new_struct_error(vm: &VirtualMachine, msg: String) -> PyBaseExceptionRef {
        // _struct.error must exist
        let class = vm.try_class("_struct", "error").unwrap();
        vm.new_exception_msg(class, msg)
    }
}

pub(crate) fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let struct_error = ctx.new_class("struct.error", ctx.exceptions.exception_type.clone());

    let module = _struct::make_module(vm);
    extend_module!(vm, module, {
        "error" => struct_error,
    });
    module
}
