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
pub(crate) mod _struct {
    use byteorder::{ByteOrder, ReadBytesExt, WriteBytesExt};
    use crossbeam_utils::atomic::AtomicCell;
    use itertools::Itertools;
    use num_bigint::BigInt;
    use num_traits::{AsPrimitive, ToPrimitive};
    use std::convert::TryFrom;
    use std::io::{Cursor, Read, Write};
    use std::iter::Peekable;
    use std::{fmt, mem, os::raw};

    use crate::builtins::{
        bytes::PyBytesRef, float::IntoPyFloat, int::try_to_primitive, pybool::IntoPyBool,
        pystr::PyStr, pystr::PyStrRef, pytype::PyTypeRef, tuple::PyTupleRef,
    };
    use crate::byteslike::{PyBytesLike, PyRwBytesLike};
    use crate::exceptions::PyBaseExceptionRef;
    use crate::function::Args;
    use crate::pyobject::{
        BorrowValue, Either, IntoPyObject, PyObjectRef, PyRef, PyResult, PyValue, StaticType,
        TryFromObject,
    };
    use crate::slots::PyIter;
    use crate::VirtualMachine;

    #[derive(Debug, Copy, Clone, PartialEq)]
    enum Endianness {
        Native,
        Little,
        Big,
        Host,
    }

    #[derive(Debug, Clone)]
    struct FormatCode {
        repeat: isize,
        code: FormatType,
        info: &'static FormatInfo,
    }

    #[derive(Copy, Clone, num_enum::TryFromPrimitive)]
    #[repr(u8)]
    enum FormatType {
        Pad = b'x',
        SByte = b'b',
        UByte = b'B',
        Char = b'c',
        Str = b's',
        Pascal = b'p',
        Short = b'h',
        UShort = b'H',
        Int = b'i',
        UInt = b'I',
        Long = b'l',
        ULong = b'L',
        SSizeT = b'n',
        SizeT = b'N',
        LongLong = b'q',
        ULongLong = b'Q',
        Bool = b'?',
        // TODO: Half = 'e',
        Float = b'f',
        Double = b'd',
        VoidP = b'P',
    }
    impl fmt::Debug for FormatType {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            fmt::Debug::fmt(&(*self as u8 as char), f)
        }
    }

    type PackFunc = fn(&VirtualMachine, &PyObjectRef, &mut dyn Write) -> PyResult<()>;
    type UnpackFunc = fn(&VirtualMachine, &mut dyn Read) -> PyResult;

    struct FormatInfo {
        size: usize,
        align: usize,
        pack: Option<PackFunc>,
        unpack: Option<UnpackFunc>,
    }
    impl fmt::Debug for FormatInfo {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("FormatInfo")
                .field("size", &self.size)
                .field("align", &self.align)
                .finish()
        }
    }

    impl FormatType {
        fn info(self, e: Endianness) -> &'static FormatInfo {
            use mem::{align_of, size_of};
            use FormatType::*;
            macro_rules! native_info {
                ($t:ty) => {{
                    &FormatInfo {
                        size: size_of::<$t>(),
                        align: align_of::<$t>(),
                        pack: Some(<$t as Packable>::pack::<byteorder::NativeEndian>),
                        unpack: Some(<$t as Packable>::unpack::<byteorder::NativeEndian>),
                    }
                }};
            }
            macro_rules! nonnative_info {
                ($t:ty, $end:ty) => {{
                    &FormatInfo {
                        size: size_of::<$t>(),
                        align: 0,
                        pack: Some(<$t as Packable>::pack::<$end>),
                        unpack: Some(<$t as Packable>::unpack::<$end>),
                    }
                }};
            }
            macro_rules! match_nonnative {
                ($zelf:expr, $end:ty) => {{
                    match $zelf {
                        Pad | Str | Pascal => &FormatInfo {
                            size: size_of::<u8>(),
                            align: 0,
                            pack: None,
                            unpack: None,
                        },
                        SByte => nonnative_info!(i8, $end),
                        UByte => nonnative_info!(u8, $end),
                        Char => &FormatInfo {
                            size: size_of::<u8>(),
                            align: 0,
                            pack: Some(pack_char),
                            unpack: Some(unpack_char),
                        },
                        Short => nonnative_info!(i16, $end),
                        UShort => nonnative_info!(u16, $end),
                        Int | Long => nonnative_info!(i32, $end),
                        UInt | ULong => nonnative_info!(u32, $end),
                        LongLong => nonnative_info!(i64, $end),
                        ULongLong => nonnative_info!(u64, $end),
                        Bool => nonnative_info!(bool, $end),
                        Float => nonnative_info!(f32, $end),
                        Double => nonnative_info!(f64, $end),
                        _ => unreachable!(), // size_t or void*
                    }
                }};
            }
            match e {
                Endianness::Native => match self {
                    Pad | Str | Pascal => &FormatInfo {
                        size: size_of::<raw::c_char>(),
                        align: 0,
                        pack: None,
                        unpack: None,
                    },
                    SByte => native_info!(raw::c_schar),
                    UByte => native_info!(raw::c_uchar),
                    Char => &FormatInfo {
                        size: size_of::<raw::c_char>(),
                        align: 0,
                        pack: Some(pack_char),
                        unpack: Some(unpack_char),
                    },
                    Short => native_info!(raw::c_short),
                    UShort => native_info!(raw::c_ushort),
                    Int => native_info!(raw::c_int),
                    UInt => native_info!(raw::c_uint),
                    Long => native_info!(raw::c_long),
                    ULong => native_info!(raw::c_ulong),
                    SSizeT => native_info!(isize), // ssize_t == isize
                    SizeT => native_info!(usize),  //  size_t == usize
                    LongLong => native_info!(raw::c_longlong),
                    ULongLong => native_info!(raw::c_ulonglong),
                    Bool => native_info!(bool),
                    Float => native_info!(raw::c_float),
                    Double => native_info!(raw::c_double),
                    VoidP => native_info!(*mut raw::c_void),
                },
                Endianness::Big => match_nonnative!(self, byteorder::BigEndian),
                Endianness::Little => match_nonnative!(self, byteorder::LittleEndian),
                Endianness::Host => match_nonnative!(self, byteorder::NativeEndian),
            }
        }
    }

    impl FormatCode {
        fn arg_count(&self) -> usize {
            match self.code {
                FormatType::Pad => 0,
                FormatType::Str | FormatType::Pascal => 1,
                _ => self.repeat as usize,
            }
        }
    }

    const OVERFLOW_MSG: &str = "total struct size too long";

    #[derive(Debug, Clone)]
    pub(crate) struct FormatSpec {
        endianness: Endianness,
        codes: Vec<FormatCode>,
        size: usize,
    }

    impl FormatSpec {
        fn decode_and_parse(
            vm: &VirtualMachine,
            fmt: &Either<PyStrRef, PyBytesRef>,
        ) -> PyResult<FormatSpec> {
            let decoded_fmt = match fmt {
                Either::A(string) => string.borrow_value(),
                Either::B(bytes) if bytes.is_ascii() => std::str::from_utf8(&bytes).unwrap(),
                _ => {
                    return Err(vm.new_unicode_decode_error(
                        "Struct format must be a ascii string".to_owned(),
                    ))
                }
            };
            FormatSpec::parse(decoded_fmt).map_err(|err| new_struct_error(vm, err))
        }

        pub fn parse(fmt: &str) -> Result<FormatSpec, String> {
            let mut chars = fmt.bytes().peekable();

            // First determine "@", "<", ">","!" or "="
            let endianness = parse_endianness(&mut chars);

            // Now, analyze struct string furter:
            let codes = parse_format_codes(&mut chars, endianness)?;

            let size = Self::calc_size(&codes).ok_or_else(|| OVERFLOW_MSG.to_owned())?;

            Ok(FormatSpec {
                endianness,
                codes,
                size,
            })
        }

        pub fn pack(&self, args: &[PyObjectRef], vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            // Create data vector:
            let mut data = vec![0; self.size()];

            self.pack_into(&mut Cursor::new(&mut data), args, vm)?;

            Ok(data)
        }

        fn pack_into(
            &self,
            buffer: &mut Cursor<&mut [u8]>,
            args: &[PyObjectRef],
            vm: &VirtualMachine,
        ) -> PyResult<()> {
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

            let mut args = args.iter();
            // Loop over all opcodes:
            for code in self.codes.iter() {
                debug!("code: {:?}", code);
                match code.code {
                    FormatType::Str => {
                        pack_string(vm, args.next().unwrap(), buffer, code.repeat as usize)?;
                    }
                    FormatType::Pascal => {
                        pack_pascal(vm, args.next().unwrap(), buffer, code.repeat as usize)?;
                    }
                    FormatType::Pad => {
                        for _ in 0..code.repeat {
                            buffer.write_u8(0).unwrap();
                        }
                    }
                    _ => {
                        let pos = buffer.position() as usize;
                        let extra = compensate_alignment(pos, code.info.align).unwrap();
                        buffer.set_position((pos + extra) as u64);

                        let pack = code.info.pack.unwrap();
                        for arg in args.by_ref().take(code.repeat as usize) {
                            pack(vm, arg, buffer)?;
                        }
                    }
                }
            }

            Ok(())
        }

        pub fn unpack(&self, data: &[u8], vm: &VirtualMachine) -> PyResult<PyTupleRef> {
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
                match code.code {
                    FormatType::Pad => {
                        unpack_empty(vm, &mut rdr, code.repeat);
                    }
                    FormatType::Str => {
                        items.push(unpack_string(vm, &mut rdr, code.repeat)?);
                    }
                    FormatType::Pascal => {
                        items.push(unpack_pascal(vm, &mut rdr, code.repeat)?);
                    }
                    _ => {
                        let pos = rdr.position() as usize;
                        let extra = compensate_alignment(pos, code.info.align).unwrap();
                        rdr.set_position((pos + extra) as u64);

                        let unpack = code.info.unpack.unwrap();
                        for _ in 0..code.repeat {
                            items.push(unpack(vm, &mut rdr)?);
                        }
                    }
                };
            }

            Ok(PyTupleRef::with_elements(items, &vm.ctx))
        }

        #[inline]
        pub fn size(&self) -> usize {
            self.size
        }

        fn calc_size(codes: &[FormatCode]) -> Option<usize> {
            // cpython has size as an isize, so check for isize overflow but then cast it to usize
            let mut offset = 0isize;
            for c in codes {
                let extra = compensate_alignment(offset as usize, c.info.align)?;
                offset = offset.checked_add(extra.to_isize()?)?;

                let item_size = (c.info.size as isize).checked_mul(c.repeat)?;
                offset = offset.checked_add(item_size)?;
            }
            Some(offset as usize)
        }
    }

    fn compensate_alignment(offset: usize, align: usize) -> Option<usize> {
        if align != 0 && offset != 0 {
            // a % b == a & (b-1) if b is a power of 2
            (align - 1).checked_sub((offset - 1) & (align - 1))
        } else {
            // alignment is already all good
            Some(0)
        }
    }

    /// Parse endianness
    /// See also: https://docs.python.org/3/library/struct.html?highlight=struct#byte-order-size-and-alignment
    fn parse_endianness<I>(chars: &mut Peekable<I>) -> Endianness
    where
        I: Sized + Iterator<Item = u8>,
    {
        let e = match chars.peek() {
            Some(b'@') => Endianness::Native,
            Some(b'=') => Endianness::Host,
            Some(b'<') => Endianness::Little,
            Some(b'>') | Some(b'!') => Endianness::Big,
            _ => return Endianness::Native,
        };
        chars.next().unwrap();
        e
    }

    fn parse_format_codes<I>(
        chars: &mut Peekable<I>,
        endianness: Endianness,
    ) -> Result<Vec<FormatCode>, String>
    where
        I: Sized + Iterator<Item = u8>,
    {
        let mut codes = vec![];
        while chars.peek().is_some() {
            // determine repeat operator:
            let repeat = match chars.peek() {
                Some(b'0'..=b'9') => {
                    let mut repeat = 0isize;
                    while let Some(b'0'..=b'9') = chars.peek() {
                        if let Some(c) = chars.next() {
                            let current_digit = (c as char).to_digit(10).unwrap() as isize;
                            repeat = repeat
                                .checked_mul(10)
                                .and_then(|r| r.checked_add(current_digit))
                                .ok_or_else(|| OVERFLOW_MSG.to_owned())?;
                        }
                    }
                    repeat
                }
                _ => 1,
            };

            // determine format char:
            let c = chars
                .next()
                .ok_or_else(|| "repeat count given without format specifier".to_owned())?;
            let code = FormatType::try_from(c)
                .ok()
                .filter(|c| match c {
                    FormatType::SSizeT | FormatType::SizeT | FormatType::VoidP => {
                        endianness == Endianness::Native
                    }
                    _ => true,
                })
                .ok_or_else(|| "bad char in struct format".to_owned())?;
            codes.push(FormatCode {
                repeat,
                code,
                info: code.info(endianness),
            })
        }

        Ok(codes)
    }

    fn get_int_or_index<T>(vm: &VirtualMachine, arg: PyObjectRef) -> PyResult<T>
    where
        T: num_traits::PrimInt + for<'a> std::convert::TryFrom<&'a BigInt>,
    {
        match vm.to_index_opt(arg) {
            Some(index) => try_to_primitive(index?.borrow_value(), vm),
            None => Err(new_struct_error(
                vm,
                "required argument is not an integer".to_owned(),
            )),
        }
    }

    fn get_float<T>(vm: &VirtualMachine, arg: PyObjectRef) -> PyResult<T>
    where
        T: num_traits::Float + 'static,
        f64: AsPrimitive<T>,
    {
        IntoPyFloat::try_from_object(vm, arg).map(|f| f.to_f64().as_())
    }

    fn get_buffer_offset(
        buffer_len: usize,
        offset: isize,
        needed: usize,
        is_pack: bool,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let offset_from_start = if offset < 0 {
            if (-offset) as usize > buffer_len {
                return Err(new_struct_error(
                    vm,
                    format!(
                        "offset {} out of range for {}-byte buffer",
                        offset, buffer_len
                    ),
                ));
            }
            buffer_len - (-offset as usize)
        } else {
            if offset as usize >= buffer_len {
                let msg = format!(
                    "{op} requires a buffer of at least {required} bytes for {op_action} {needed} \
                    bytes at offset {offset} (actual buffer size is {buffer_len})",
                    op = if is_pack { "pack_into" } else { "unpack_from" },
                    op_action = if is_pack { "packing" } else { "unpacking" },
                    required = needed + offset as usize,
                    needed = needed,
                    offset = offset,
                    buffer_len = buffer_len
                );
                return Err(new_struct_error(vm, msg));
            }
            offset as usize
        };

        if (buffer_len - offset_from_start) < needed {
            Err(new_struct_error(
                vm,
                if is_pack {
                    format!("no space to pack {} bytes at offset {}", needed, offset)
                } else {
                    format!(
                        "not enough data to unpack {} bytes at offset {}",
                        needed, offset
                    )
                },
            ))
        } else {
            Ok(offset_from_start)
        }
    }

    trait Packable {
        fn pack<Endianness: ByteOrder>(
            vm: &VirtualMachine,
            arg: &PyObjectRef,
            data: &mut dyn Write,
        ) -> PyResult<()>;
        fn unpack<Endianness: ByteOrder>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult;
    }

    macro_rules! make_pack_no_endianess {
        ($T:ty) => {
            paste::item! {
                impl Packable for $T {
                    fn pack<Endianness: ByteOrder>(vm: &VirtualMachine, arg: &PyObjectRef, data: &mut dyn Write) -> PyResult<()> {
                        data.[<write_$T>](get_int_or_index(vm, arg.clone())?).unwrap();
                        Ok(())
                    }

                    fn unpack<Endianness: ByteOrder>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult {
                        _unpack(vm, rdr, |rdr| rdr.[<read_$T>](), |i| Ok(i.into_pyobject(vm)))
                    }
                }
            }
        };
    }

    macro_rules! make_pack_with_endianess {
        ($T:ty, $fromobj:path) => {
            paste::item! {
                impl Packable for $T {
                    fn pack<Endianness: ByteOrder>(vm: &VirtualMachine, arg: &PyObjectRef, data: &mut dyn Write) -> PyResult<()> {
                        data.[<write_$T>]::<Endianness>($fromobj(vm, arg.clone())?).unwrap();
                        Ok(())
                    }

                    fn unpack<Endianness: ByteOrder>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult {
                        _unpack(vm, rdr, |rdr| rdr.[<read_$T>]::<Endianness>(), |i| Ok(i.into_pyobject(vm)))
                    }
                }
            }
        };
    }

    make_pack_no_endianess!(i8);
    make_pack_no_endianess!(u8);
    make_pack_with_endianess!(i16, get_int_or_index);
    make_pack_with_endianess!(u16, get_int_or_index);
    make_pack_with_endianess!(i32, get_int_or_index);
    make_pack_with_endianess!(u32, get_int_or_index);
    make_pack_with_endianess!(i64, get_int_or_index);
    make_pack_with_endianess!(u64, get_int_or_index);
    make_pack_with_endianess!(f32, get_float);
    make_pack_with_endianess!(f64, get_float);

    impl Packable for *mut raw::c_void {
        fn pack<Endianness: ByteOrder>(
            vm: &VirtualMachine,
            arg: &PyObjectRef,
            data: &mut dyn Write,
        ) -> PyResult<()> {
            usize::pack::<Endianness>(vm, arg, data)
        }

        fn unpack<Endianness: ByteOrder>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult {
            usize::unpack::<Endianness>(vm, rdr)
        }
    }

    impl Packable for bool {
        fn pack<Endianness: ByteOrder>(
            vm: &VirtualMachine,
            arg: &PyObjectRef,
            data: &mut dyn Write,
        ) -> PyResult<()> {
            let v = IntoPyBool::try_from_object(vm, arg.clone())?.to_bool() as u8;
            data.write_u8(v).unwrap();
            Ok(())
        }

        fn unpack<Endianness: ByteOrder>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult {
            _unpack(
                vm,
                rdr,
                |rdr| rdr.read_u8(),
                |i| Ok(vm.ctx.new_bool(i != 0)),
            )
        }
    }

    macro_rules! make_pack_varsize {
        ($T:ty, $int:ident) => {
            paste::item! {
                impl Packable for $T {
                    fn pack<Endianness: ByteOrder>(
                        vm: &VirtualMachine,
                        arg: &PyObjectRef,
                        data: &mut dyn Write,
                    ) -> PyResult<()> {
                        let v: Self = get_int_or_index(vm, arg.clone())?;
                        data.[<write_$int>]::<Endianness>(v as _, std::mem::size_of::<isize>())
                            .unwrap();
                        Ok(())
                    }

                    fn unpack<Endianness: ByteOrder>(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult {
                        unpack_int(
                            vm,
                            rdr,
                            |rdr| rdr.[<read_$int>]::<Endianness>(std::mem::size_of::<Self>()),
                        )
                    }
                }
            }
        };
    }

    make_pack_varsize!(usize, uint);
    make_pack_varsize!(isize, int);

    fn pack_string(
        vm: &VirtualMachine,
        arg: &PyObjectRef,
        data: &mut dyn Write,
        length: usize,
    ) -> PyResult<()> {
        let mut v = PyBytesRef::try_from_object(vm, arg.clone())?
            .borrow_value()
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
            .borrow_value()
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
        let ch = *v.borrow_value().iter().exactly_one().map_err(|_| {
            new_struct_error(
                vm,
                "char format requires a bytes object of length 1".to_owned(),
            )
        })?;
        data.write_u8(ch).unwrap();
        Ok(())
    }

    #[pyfunction]
    fn pack(
        fmt: Either<PyStrRef, PyBytesRef>,
        args: Args,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        let format_spec = FormatSpec::decode_and_parse(vm, &fmt)?;
        format_spec.pack(args.as_ref(), vm)
    }

    #[pyfunction]
    fn pack_into(
        fmt: Either<PyStrRef, PyBytesRef>,
        buffer: PyRwBytesLike,
        offset: isize,
        args: Args,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let format_spec = FormatSpec::decode_and_parse(vm, &fmt)?;
        let offset = get_buffer_offset(buffer.len(), offset, format_spec.size(), true, vm)?;
        buffer.with_ref(|data| {
            let mut data = Cursor::new(data);
            data.set_position(offset as u64);
            format_spec.pack_into(&mut data, args.as_ref(), vm)
        })
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
                format!("unpack requires a buffer of {} bytes", mem::size_of::<T>()),
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

    fn unpack_empty(_vm: &VirtualMachine, rdr: &mut dyn Read, length: isize) {
        let mut handle = rdr.take(length as u64);
        let mut buf: Vec<u8> = Vec::new();
        let _ = handle.read_to_end(&mut buf);
    }

    fn unpack_char(vm: &VirtualMachine, rdr: &mut dyn Read) -> PyResult {
        unpack_string(vm, rdr, 1)
    }

    fn unpack_string(vm: &VirtualMachine, rdr: &mut dyn Read, length: isize) -> PyResult {
        let mut handle = rdr.take(length as u64);
        let mut buf: Vec<u8> = Vec::new();
        handle.read_to_end(&mut buf).map_err(|_| {
            new_struct_error(vm, format!("unpack requires a buffer of {} bytes", length,))
        })?;
        Ok(vm.ctx.new_bytes(buf))
    }

    fn unpack_pascal(vm: &VirtualMachine, rdr: &mut dyn Read, length: isize) -> PyResult {
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
        fmt: Either<PyStrRef, PyBytesRef>,
        buffer: PyBytesLike,
        vm: &VirtualMachine,
    ) -> PyResult<PyTupleRef> {
        let format_spec = FormatSpec::decode_and_parse(vm, &fmt)?;
        buffer.with_ref(|buf| format_spec.unpack(buf, vm))
    }

    #[derive(FromArgs)]
    struct UpdateFromArgs {
        buffer: PyBytesLike,
        #[pyarg(any, default = "0")]
        offset: isize,
    }

    #[pyfunction]
    fn unpack_from(
        fmt: Either<PyStrRef, PyBytesRef>,
        args: UpdateFromArgs,
        vm: &VirtualMachine,
    ) -> PyResult<PyTupleRef> {
        let format_spec = FormatSpec::decode_and_parse(vm, &fmt)?;
        let size = format_spec.size();
        let offset = get_buffer_offset(args.buffer.len(), args.offset, size, false, vm)?;
        args.buffer
            .with_ref(|buf| format_spec.unpack(&buf[offset..offset + size], vm))
    }

    #[pyattr]
    #[pyclass(name = "unpack_iterator")]
    #[derive(Debug)]
    struct UnpackIterator {
        format_spec: FormatSpec,
        buffer: PyBytesLike,
        offset: AtomicCell<usize>,
    }

    impl UnpackIterator {
        fn new(
            vm: &VirtualMachine,
            format_spec: FormatSpec,
            buffer: PyBytesLike,
        ) -> PyResult<UnpackIterator> {
            if format_spec.size() == 0 {
                Err(new_struct_error(
                    vm,
                    "cannot iteratively unpack with a struct of length 0".to_owned(),
                ))
            } else if buffer.len() % format_spec.size() != 0 {
                Err(new_struct_error(
                    vm,
                    format!(
                        "iterative unpacking requires a buffer of a multiple of {} bytes",
                        format_spec.size()
                    ),
                ))
            } else {
                Ok(UnpackIterator {
                    format_spec,
                    buffer,
                    offset: AtomicCell::new(0),
                })
            }
        }
    }

    impl PyValue for UnpackIterator {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl(with(PyIter))]
    impl UnpackIterator {
        #[pymethod(magic)]
        fn length_hint(&self) -> usize {
            self.buffer.len().saturating_sub(self.offset.load()) / self.format_spec.size()
        }
    }
    impl PyIter for UnpackIterator {
        fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let size = zelf.format_spec.size();
            let offset = zelf.offset.fetch_add(size);
            zelf.buffer.with_ref(|buf| {
                if let Some(buf) = buf.get(offset..offset + size) {
                    zelf.format_spec.unpack(buf, vm).map(|x| x.into_object())
                } else {
                    Err(vm.new_stop_iteration())
                }
            })
        }
    }

    #[pyfunction]
    fn iter_unpack(
        fmt: Either<PyStrRef, PyBytesRef>,
        buffer: PyBytesLike,
        vm: &VirtualMachine,
    ) -> PyResult<UnpackIterator> {
        let format_spec = FormatSpec::decode_and_parse(vm, &fmt)?;
        UnpackIterator::new(vm, format_spec, buffer)
    }

    #[pyfunction]
    fn calcsize(fmt: Either<PyStrRef, PyBytesRef>, vm: &VirtualMachine) -> PyResult<usize> {
        let format_spec = FormatSpec::decode_and_parse(vm, &fmt)?;
        Ok(format_spec.size())
    }

    #[pyattr]
    #[pyclass(name = "Struct")]
    #[derive(Debug)]
    struct PyStruct {
        spec: FormatSpec,
        fmt_str: PyStrRef,
    }

    impl PyValue for PyStruct {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl]
    impl PyStruct {
        #[pyslot]
        fn tp_new(
            cls: PyTypeRef,
            fmt: Either<PyStrRef, PyBytesRef>,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let spec = FormatSpec::decode_and_parse(vm, &fmt)?;
            let fmt_str = match fmt {
                Either::A(s) => s,
                Either::B(b) => PyStr::from(std::str::from_utf8(b.borrow_value()).unwrap())
                    .into_ref_with_type(vm, vm.ctx.types.str_type.clone())?,
            };
            PyStruct { spec, fmt_str }.into_ref_with_type(vm, cls)
        }

        #[pyproperty]
        fn format(&self) -> PyStrRef {
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
        fn pack_into(
            &self,
            buffer: PyRwBytesLike,
            offset: isize,
            args: Args,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let offset = get_buffer_offset(buffer.len(), offset, self.size(), true, vm)?;
            buffer.with_ref(|data| {
                let mut data = Cursor::new(data);
                data.set_position(offset as u64);
                self.spec.pack_into(&mut data, args.as_ref(), vm)
            })
        }

        #[pymethod]
        fn unpack(&self, data: PyBytesLike, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
            data.with_ref(|buf| self.spec.unpack(buf, vm))
        }

        #[pymethod]
        fn unpack_from(&self, args: UpdateFromArgs, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
            let size = self.size();
            let offset = get_buffer_offset(args.buffer.len(), args.offset, size, false, vm)?;
            args.buffer
                .with_ref(|buf| self.spec.unpack(&buf[offset..offset + size], vm))
        }

        #[pymethod]
        fn iter_unpack(
            &self,
            buffer: PyBytesLike,
            vm: &VirtualMachine,
        ) -> PyResult<UnpackIterator> {
            UnpackIterator::new(vm, self.spec.clone(), buffer)
        }
    }

    // seems weird that this is part of the "public" API, but whatever
    // TODO: implement a format code->spec cache like CPython does?
    #[pyfunction]
    fn _clearcache() {}

    rustpython_common::static_cell! {
        pub(crate) static STRUCT_ERROR: PyTypeRef;
    }

    fn new_struct_error(vm: &VirtualMachine, msg: String) -> PyBaseExceptionRef {
        let class = STRUCT_ERROR.get().unwrap();
        vm.new_exception_msg(class.clone(), msg)
    }
}

pub(crate) fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let struct_error = _struct::STRUCT_ERROR
        .get_or_init(|| {
            ctx.new_class(
                "struct.error",
                &ctx.exceptions.exception_type,
                Default::default(),
            )
        })
        .clone();

    let module = _struct::make_module(vm);
    extend_module!(vm, module, {
        "error" => struct_error,
    });
    module
}
