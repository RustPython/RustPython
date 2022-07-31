use crate::{
    builtins::{PyBaseExceptionRef, PyBytesRef, PyTuple, PyTupleRef, PyTypeRef},
    common::{static_cell, str::wchar_t},
    convert::ToPyObject,
    function::{ArgBytesLike, ArgIntoBool, ArgIntoFloat},
    PyObjectRef, PyResult, TryFromObject, VirtualMachine,
};
use half::f16;
use itertools::Itertools;
use num_bigint::BigInt;
use num_traits::{PrimInt, ToPrimitive};
use std::{fmt, iter::Peekable, mem, os::raw};

type PackFunc = fn(&VirtualMachine, PyObjectRef, &mut [u8]) -> PyResult<()>;
type UnpackFunc = fn(&VirtualMachine, &[u8]) -> PyObjectRef;

static OVERFLOW_MSG: &str = "total struct size too long"; // not a const to reduce code size

#[derive(Debug, Copy, Clone, PartialEq)]
pub(crate) enum Endianness {
    Native,
    Little,
    Big,
    Host,
}

impl Endianness {
    /// Parse endianness
    /// See also: https://docs.python.org/3/library/struct.html?highlight=struct#byte-order-size-and-alignment
    fn parse<I>(chars: &mut Peekable<I>) -> Endianness
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
}

trait ByteOrder {
    fn convert<I: PrimInt>(i: I) -> I;
}
enum BigEndian {}
impl ByteOrder for BigEndian {
    fn convert<I: PrimInt>(i: I) -> I {
        i.to_be()
    }
}
enum LittleEndian {}
impl ByteOrder for LittleEndian {
    fn convert<I: PrimInt>(i: I) -> I {
        i.to_le()
    }
}

#[cfg(target_endian = "big")]
type NativeEndian = BigEndian;
#[cfg(target_endian = "little")]
type NativeEndian = LittleEndian;

#[derive(Copy, Clone, num_enum::TryFromPrimitive)]
#[repr(u8)]
pub(crate) enum FormatType {
    Pad = b'x',
    SByte = b'b',
    UByte = b'B',
    Char = b'c',
    WideChar = b'u',
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
    Half = b'e',
    Float = b'f',
    Double = b'd',
    VoidP = b'P',
}

impl fmt::Debug for FormatType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&(*self as u8 as char), f)
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
                    pack: Some(<$t as Packable>::pack::<NativeEndian>),
                    unpack: Some(<$t as Packable>::unpack::<NativeEndian>),
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
                    Half => nonnative_info!(f16, $end),
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
                WideChar => native_info!(wchar_t),
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
                Half => native_info!(f16),
                Float => native_info!(raw::c_float),
                Double => native_info!(raw::c_double),
                VoidP => native_info!(*mut raw::c_void),
            },
            Endianness::Big => match_nonnative!(self, BigEndian),
            Endianness::Little => match_nonnative!(self, LittleEndian),
            Endianness::Host => match_nonnative!(self, NativeEndian),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FormatCode {
    pub repeat: usize,
    pub code: FormatType,
    pub info: &'static FormatInfo,
    pub pre_padding: usize,
}

impl FormatCode {
    pub fn arg_count(&self) -> usize {
        match self.code {
            FormatType::Pad => 0,
            FormatType::Str | FormatType::Pascal => 1,
            _ => self.repeat,
        }
    }

    pub fn parse<I>(
        chars: &mut Peekable<I>,
        endianness: Endianness,
    ) -> Result<(Vec<Self>, usize, usize), String>
    where
        I: Sized + Iterator<Item = u8>,
    {
        let mut offset = 0isize;
        let mut arg_count = 0usize;
        let mut codes = vec![];
        while chars.peek().is_some() {
            // determine repeat operator:
            let repeat = match chars.peek() {
                Some(b'0'..=b'9') => {
                    let mut repeat = 0isize;
                    while let Some(b'0'..=b'9') = chars.peek() {
                        if let Some(c) = chars.next() {
                            let current_digit = c - b'0';
                            repeat = repeat
                                .checked_mul(10)
                                .and_then(|r| r.checked_add(current_digit as _))
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

            let info = code.info(endianness);

            let padding = compensate_alignment(offset as usize, info.align)
                .ok_or_else(|| OVERFLOW_MSG.to_owned())?;
            offset = padding
                .to_isize()
                .and_then(|extra| offset.checked_add(extra))
                .ok_or_else(|| OVERFLOW_MSG.to_owned())?;

            let code = FormatCode {
                repeat: repeat as usize,
                code,
                info,
                pre_padding: padding,
            };
            arg_count += code.arg_count();
            codes.push(code);

            offset = (info.size as isize)
                .checked_mul(repeat)
                .and_then(|item_size| offset.checked_add(item_size))
                .ok_or_else(|| OVERFLOW_MSG.to_owned())?;
        }

        Ok((codes, offset as usize, arg_count))
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

pub(crate) struct FormatInfo {
    pub size: usize,
    pub align: usize,
    pub pack: Option<PackFunc>,
    pub unpack: Option<UnpackFunc>,
}
impl fmt::Debug for FormatInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FormatInfo")
            .field("size", &self.size)
            .field("align", &self.align)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct FormatSpec {
    #[allow(dead_code)]
    pub(crate) endianness: Endianness,
    pub(crate) codes: Vec<FormatCode>,
    pub size: usize,
    pub arg_count: usize,
}

impl FormatSpec {
    pub fn parse(fmt: &[u8], vm: &VirtualMachine) -> PyResult<FormatSpec> {
        let mut chars = fmt.iter().copied().peekable();

        // First determine "@", "<", ">","!" or "="
        let endianness = Endianness::parse(&mut chars);

        // Now, analyze struct string further:
        let (codes, size, arg_count) =
            FormatCode::parse(&mut chars, endianness).map_err(|err| new_struct_error(vm, err))?;

        Ok(FormatSpec {
            endianness,
            codes,
            size,
            arg_count,
        })
    }

    pub fn pack(&self, args: Vec<PyObjectRef>, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        // Create data vector:
        let mut data = vec![0; self.size];

        self.pack_into(&mut data, args, vm)?;

        Ok(data)
    }

    pub fn pack_into(
        &self,
        mut buffer: &mut [u8],
        args: Vec<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if self.arg_count != args.len() {
            return Err(new_struct_error(
                vm,
                format!(
                    "pack expected {} items for packing (got {})",
                    self.codes.len(),
                    args.len()
                ),
            ));
        }

        let mut args = args.into_iter();
        // Loop over all opcodes:
        for code in &self.codes {
            buffer = &mut buffer[code.pre_padding..];
            debug!("code: {:?}", code);
            match code.code {
                FormatType::Str => {
                    let (buf, rest) = buffer.split_at_mut(code.repeat);
                    pack_string(vm, args.next().unwrap(), buf)?;
                    buffer = rest;
                }
                FormatType::Pascal => {
                    let (buf, rest) = buffer.split_at_mut(code.repeat);
                    pack_pascal(vm, args.next().unwrap(), buf)?;
                    buffer = rest;
                }
                FormatType::Pad => {
                    let (pad_buf, rest) = buffer.split_at_mut(code.repeat);
                    for el in pad_buf {
                        *el = 0
                    }
                    buffer = rest;
                }
                _ => {
                    let pack = code.info.pack.unwrap();
                    for arg in args.by_ref().take(code.repeat) {
                        let (item_buf, rest) = buffer.split_at_mut(code.info.size);
                        pack(vm, arg, item_buf)?;
                        buffer = rest;
                    }
                }
            }
        }

        Ok(())
    }

    pub fn unpack(&self, mut data: &[u8], vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        if self.size != data.len() {
            return Err(new_struct_error(
                vm,
                format!("unpack requires a buffer of {} bytes", self.size),
            ));
        }

        let mut items = Vec::with_capacity(self.arg_count);
        for code in &self.codes {
            data = &data[code.pre_padding..];
            debug!("unpack code: {:?}", code);
            match code.code {
                FormatType::Pad => {
                    data = &data[code.repeat..];
                }
                FormatType::Str => {
                    let (str_data, rest) = data.split_at(code.repeat);
                    // string is just stored inline
                    items.push(vm.ctx.new_bytes(str_data.to_vec()).into());
                    data = rest;
                }
                FormatType::Pascal => {
                    let (str_data, rest) = data.split_at(code.repeat);
                    items.push(unpack_pascal(vm, str_data));
                    data = rest;
                }
                _ => {
                    let unpack = code.info.unpack.unwrap();
                    for _ in 0..code.repeat {
                        let (item_data, rest) = data.split_at(code.info.size);
                        items.push(unpack(vm, item_data));
                        data = rest;
                    }
                }
            };
        }

        Ok(PyTuple::new_ref(items, &vm.ctx))
    }

    #[inline]
    pub fn size(&self) -> usize {
        self.size
    }
}

trait Packable {
    fn pack<E: ByteOrder>(vm: &VirtualMachine, arg: PyObjectRef, data: &mut [u8]) -> PyResult<()>;
    fn unpack<E: ByteOrder>(vm: &VirtualMachine, data: &[u8]) -> PyObjectRef;
}

trait PackInt: PrimInt {
    fn pack_int<E: ByteOrder>(self, data: &mut [u8]);
    fn unpack_int<E: ByteOrder>(data: &[u8]) -> Self;
}

macro_rules! make_pack_primint {
    ($T:ty) => {
        impl PackInt for $T {
            fn pack_int<E: ByteOrder>(self, data: &mut [u8]) {
                let i = E::convert(self);
                data.copy_from_slice(&i.to_ne_bytes());
            }
            #[inline]
            fn unpack_int<E: ByteOrder>(data: &[u8]) -> Self {
                let mut x = [0; std::mem::size_of::<$T>()];
                x.copy_from_slice(data);
                E::convert(<$T>::from_ne_bytes(x))
            }
        }

        impl Packable for $T {
            fn pack<E: ByteOrder>(
                vm: &VirtualMachine,
                arg: PyObjectRef,
                data: &mut [u8],
            ) -> PyResult<()> {
                let i: $T = get_int_or_index(vm, arg)?;
                i.pack_int::<E>(data);
                Ok(())
            }

            fn unpack<E: ByteOrder>(vm: &VirtualMachine, rdr: &[u8]) -> PyObjectRef {
                let i = <$T>::unpack_int::<E>(rdr);
                vm.ctx.new_int(i).into()
            }
        }
    };
}

fn get_int_or_index<T>(vm: &VirtualMachine, arg: PyObjectRef) -> PyResult<T>
where
    T: PrimInt + for<'a> TryFrom<&'a BigInt>,
{
    let index = arg.try_index_opt(vm).unwrap_or_else(|| {
        Err(new_struct_error(
            vm,
            "required argument is not an integer".to_owned(),
        ))
    })?;
    index
        .try_to_primitive(vm)
        .map_err(|_| new_struct_error(vm, "argument out of range".to_owned()))
}

make_pack_primint!(i8);
make_pack_primint!(u8);
make_pack_primint!(i16);
make_pack_primint!(u16);
make_pack_primint!(i32);
make_pack_primint!(u32);
make_pack_primint!(i64);
make_pack_primint!(u64);
make_pack_primint!(usize);
make_pack_primint!(isize);

macro_rules! make_pack_float {
    ($T:ty) => {
        impl Packable for $T {
            fn pack<E: ByteOrder>(
                vm: &VirtualMachine,
                arg: PyObjectRef,
                data: &mut [u8],
            ) -> PyResult<()> {
                let f = *ArgIntoFloat::try_from_object(vm, arg)? as $T;
                f.to_bits().pack_int::<E>(data);
                Ok(())
            }

            fn unpack<E: ByteOrder>(vm: &VirtualMachine, rdr: &[u8]) -> PyObjectRef {
                let i = PackInt::unpack_int::<E>(rdr);
                <$T>::from_bits(i).to_pyobject(vm)
            }
        }
    };
}

make_pack_float!(f32);
make_pack_float!(f64);

impl Packable for f16 {
    fn pack<E: ByteOrder>(vm: &VirtualMachine, arg: PyObjectRef, data: &mut [u8]) -> PyResult<()> {
        let f_64 = *ArgIntoFloat::try_from_object(vm, arg)?;
        let f_16 = f16::from_f64(f_64);
        if f_16.is_infinite() != f_64.is_infinite() {
            return Err(vm.new_overflow_error("float too large to pack with e format".to_owned()));
        }
        f_16.to_bits().pack_int::<E>(data);
        Ok(())
    }

    fn unpack<E: ByteOrder>(vm: &VirtualMachine, rdr: &[u8]) -> PyObjectRef {
        let i = PackInt::unpack_int::<E>(rdr);
        f16::from_bits(i).to_f64().to_pyobject(vm)
    }
}

impl Packable for *mut raw::c_void {
    fn pack<E: ByteOrder>(vm: &VirtualMachine, arg: PyObjectRef, data: &mut [u8]) -> PyResult<()> {
        usize::pack::<E>(vm, arg, data)
    }

    fn unpack<E: ByteOrder>(vm: &VirtualMachine, rdr: &[u8]) -> PyObjectRef {
        usize::unpack::<E>(vm, rdr)
    }
}

impl Packable for bool {
    fn pack<E: ByteOrder>(vm: &VirtualMachine, arg: PyObjectRef, data: &mut [u8]) -> PyResult<()> {
        let v = *ArgIntoBool::try_from_object(vm, arg)? as u8;
        v.pack_int::<E>(data);
        Ok(())
    }

    fn unpack<E: ByteOrder>(vm: &VirtualMachine, rdr: &[u8]) -> PyObjectRef {
        let i = u8::unpack_int::<E>(rdr);
        vm.ctx.new_bool(i != 0).into()
    }
}

fn pack_char(vm: &VirtualMachine, arg: PyObjectRef, data: &mut [u8]) -> PyResult<()> {
    let v = PyBytesRef::try_from_object(vm, arg)?;
    let ch = *v.as_bytes().iter().exactly_one().map_err(|_| {
        new_struct_error(
            vm,
            "char format requires a bytes object of length 1".to_owned(),
        )
    })?;
    data[0] = ch;
    Ok(())
}

fn pack_string(vm: &VirtualMachine, arg: PyObjectRef, buf: &mut [u8]) -> PyResult<()> {
    let b = ArgBytesLike::try_from_object(vm, arg)?;
    b.with_ref(|data| write_string(buf, data));
    Ok(())
}

fn pack_pascal(vm: &VirtualMachine, arg: PyObjectRef, buf: &mut [u8]) -> PyResult<()> {
    if buf.is_empty() {
        return Ok(());
    }
    let b = ArgBytesLike::try_from_object(vm, arg)?;
    b.with_ref(|data| {
        let string_length = std::cmp::min(std::cmp::min(data.len(), 255), buf.len() - 1);
        buf[0] = string_length as u8;
        write_string(&mut buf[1..], data);
    });
    Ok(())
}

fn write_string(buf: &mut [u8], data: &[u8]) {
    let len_from_data = std::cmp::min(data.len(), buf.len());
    buf[..len_from_data].copy_from_slice(&data[..len_from_data]);
    for byte in &mut buf[len_from_data..] {
        *byte = 0
    }
}

fn unpack_char(vm: &VirtualMachine, data: &[u8]) -> PyObjectRef {
    vm.ctx.new_bytes(vec![data[0]]).into()
}

fn unpack_pascal(vm: &VirtualMachine, data: &[u8]) -> PyObjectRef {
    let (&len, data) = match data.split_first() {
        Some(x) => x,
        None => {
            // cpython throws an internal SystemError here
            return vm.ctx.new_bytes(vec![]).into();
        }
    };
    let len = std::cmp::min(len as usize, data.len());
    vm.ctx.new_bytes(data[..len].to_vec()).into()
}

// XXX: are those functions expected to be placed here?
pub fn struct_error_type(vm: &VirtualMachine) -> &'static PyTypeRef {
    static_cell! {
        static INSTANCE: PyTypeRef;
    }
    INSTANCE.get_or_init(|| vm.ctx.new_exception_type("struct", "error", None))
}

pub fn new_struct_error(vm: &VirtualMachine, msg: String) -> PyBaseExceptionRef {
    // can't just STRUCT_ERROR.get().unwrap() cause this could be called before from buffer
    // machinery, independent of whether _struct was ever imported
    vm.new_exception_msg(struct_error_type(vm).clone(), msg)
}
