use crate::{
    Py, PyObject, PyObjectRef, PyResult, VirtualMachine,
    builtins::{PyBaseExceptionRef, PyByteArray, PyBytes, PyTuple, PyTupleRef, PyType, PyTypeRef},
    common::{lock::PyRwLock, rc::PyRc, static_cell, str::wchar_t},
    convert::ToPyObject,
    exceptions,
    function::{ArgIntoBool, ArgIntoComplex, ArgIntoFloat, ArgMemoryBuffer},
};

use rustpython_common::wtf8::Wtf8Buf;

use core::{fmt, iter::Peekable, mem};
use half::f16;
use itertools::Itertools;
use malachite_bigint::BigInt;
use num_complex::Complex64;
use num_traits::{PrimInt, ToPrimitive};
use std::{collections::HashMap, os::raw};

type PackFunc = fn(&VirtualMachine, FormatType, &PyObject, &mut dyn FnMut(&[u8])) -> PyResult<()>;
type UnpackFunc = fn(&VirtualMachine, &[u8]) -> PyObjectRef;

static OVERFLOW_MSG: &str = "total struct size too long"; // not a const to reduce code size

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Endianness {
    Native,
    Little,
    Big,
    Host,
}

impl Endianness {
    /// Parse endianness
    /// See also: https://docs.python.org/3/library/struct.html?highlight=struct#byte-order-size-and-alignment
    fn parse<I>(chars: &mut Peekable<I>) -> Self
    where
        I: Sized + Iterator<Item = u8>,
    {
        let e = match chars.peek() {
            Some(b'@') => Self::Native,
            Some(b'=') => Self::Host,
            Some(b'<') => Self::Little,
            Some(b'>' | b'!') => Self::Big,
            _ => return Self::Native,
        };

        // SAFETY:
        // We just ensured with `chars.peek()` that this is safe
        unsafe {
            let _ = chars.next().unwrap_unchecked();
        }
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

type NativeEndian = cfg_select! {
    target_endian = "big" => BigEndian,
    target_endian = "little" => LittleEndian,
};

#[derive(Copy, Clone, num_enum::TryFromPrimitive, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum FormatType {
    Pad = b'x',
    SByte = b'b',
    UByte = b'B',
    Char = b'c',
    WideChar = b'u',
    Ucs4Char = b'w',
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
    LongDouble = b'g',
    FloatComplex = b'F',
    DoubleComplex = b'D',
    VoidP = b'P',
    PyObject = b'O',
}

impl fmt::Debug for FormatType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&(*self as u8 as char), f)
    }
}

impl FormatType {
    pub(crate) fn info(self, e: Endianness) -> &'static FormatInfo {
        use mem::{align_of, size_of};

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
                    Self::Pad | Self::Str | Self::Pascal => &FormatInfo {
                        size: size_of::<u8>(),
                        align: 0,
                        pack: None,
                        unpack: None,
                    },
                    Self::SByte => nonnative_info!(i8, $end),
                    Self::UByte => nonnative_info!(u8, $end),
                    Self::Char => &FormatInfo {
                        size: size_of::<u8>(),
                        align: 0,
                        pack: Some(pack_char),
                        unpack: Some(unpack_char),
                    },
                    Self::Short => nonnative_info!(i16, $end),
                    Self::UShort => nonnative_info!(u16, $end),
                    Self::Int | Self::Long => nonnative_info!(i32, $end),
                    Self::UInt | Self::ULong => nonnative_info!(u32, $end),
                    Self::LongLong => nonnative_info!(i64, $end),
                    Self::ULongLong => nonnative_info!(u64, $end),
                    Self::Bool => nonnative_info!(bool, $end),
                    Self::Half => nonnative_info!(f16, $end),
                    Self::Float => nonnative_info!(f32, $end),
                    Self::Double => nonnative_info!(f64, $end),
                    Self::LongDouble => nonnative_info!(f64, $end), // long double same as double
                    Self::FloatComplex => nonnative_info!(PackFloatComplex, $end),
                    Self::DoubleComplex => nonnative_info!(PackDoubleComplex, $end),
                    Self::PyObject => nonnative_info!(usize, $end), // pointer size
                    _ => unreachable!(),                            // size_t or void*
                }
            }};
        }

        match e {
            Endianness::Native => match self {
                Self::Pad | Self::Str | Self::Pascal => &FormatInfo {
                    size: size_of::<raw::c_char>(),
                    align: 0,
                    pack: None,
                    unpack: None,
                },
                Self::SByte => native_info!(raw::c_schar),
                Self::UByte => native_info!(raw::c_uchar),
                Self::Char => &FormatInfo {
                    size: size_of::<raw::c_char>(),
                    align: 0,
                    pack: Some(pack_char),
                    unpack: Some(unpack_char),
                },
                Self::WideChar => native_info!(wchar_t),
                Self::Ucs4Char => native_info!(u32),
                Self::Short => native_info!(raw::c_short),
                Self::UShort => native_info!(raw::c_ushort),
                Self::Int => native_info!(raw::c_int),
                Self::UInt => native_info!(raw::c_uint),
                Self::Long => native_info!(raw::c_long),
                Self::ULong => native_info!(raw::c_ulong),
                Self::SSizeT => native_info!(isize), // ssize_t == isize
                Self::SizeT => native_info!(usize),  //  size_t == usize
                Self::LongLong => native_info!(raw::c_longlong),
                Self::ULongLong => native_info!(raw::c_ulonglong),
                Self::Bool => native_info!(bool),
                Self::Half => native_info!(f16),
                Self::Float => native_info!(raw::c_float),
                Self::Double => native_info!(raw::c_double),
                Self::LongDouble => native_info!(raw::c_double), // long double same as double for now
                Self::FloatComplex => native_info!(PackFloatComplex),
                Self::DoubleComplex => native_info!(PackDoubleComplex),
                Self::VoidP => native_info!(*mut raw::c_void),
                Self::PyObject => native_info!(*mut raw::c_void), // pointer to PyObject
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
    pub(crate) const fn arg_count(&self) -> usize {
        match self.code {
            FormatType::Pad => 0,
            FormatType::Str | FormatType::Pascal => 1,
            _ => self.repeat,
        }
    }

    pub(crate) fn parse<I>(
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
            // Skip whitespace before repeat count or format char
            while let Some(b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) = chars.peek() {
                chars.next();
            }
            if chars.peek().is_none() {
                break;
            }

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
            let c = match chars.next() {
                Some(c) => c,
                None => return Err("repeat count given without format specifier".to_owned()),
            };

            // prepare_s: struct accepts primitive fields, not the extended
            // exporter grammar (records, subarrays, names or nested byte order).
            let c = if c == b'Z' {
                match chars.next() {
                    Some(b'f') => b'F',
                    Some(b'd') => b'D',
                    _ => return Err("bad char in struct format".to_owned()),
                }
            } else {
                c
            };
            let code = FormatType::try_from(c)
                .ok()
                .filter(|c| match c {
                    FormatType::SSizeT | FormatType::SizeT | FormatType::VoidP => {
                        endianness == Endianness::Native
                    }
                    FormatType::WideChar
                    | FormatType::Ucs4Char
                    | FormatType::LongDouble
                    | FormatType::PyObject => false,
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

            let code = Self {
                repeat: repeat as usize,
                code,
                info,
                pre_padding: padding,
            };
            arg_count = arg_count
                .checked_add(code.arg_count())
                .filter(|&count| isize::try_from(count).is_ok())
                .ok_or_else(|| OVERFLOW_MSG.to_owned())?;
            codes.push(code);

            offset = (info.size as isize)
                .checked_mul(repeat)
                .and_then(|item_size| offset.checked_add(item_size))
                .ok_or_else(|| OVERFLOW_MSG.to_owned())?;
        }

        Ok((codes, offset as usize, arg_count))
    }
}

const fn compensate_alignment(offset: usize, align: usize) -> Option<usize> {
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

/// Bounded, interpreter-local cache for `struct` functions and buffer comparison.
/// Keys and specifications contain no Python objects or user callbacks.
#[derive(Default)]
pub struct FormatSpecCache {
    entries: PyRwLock<HashMap<Box<[u8]>, PyRc<FormatSpec>>>,
}

impl FormatSpecCache {
    pub fn get_or_parse(&self, format: &[u8], vm: &VirtualMachine) -> PyResult<PyRc<FormatSpec>> {
        if let Some(spec) = self.entries.read().get(format) {
            return Ok(spec.clone());
        }

        // Parsing can raise; neither errors nor Python conversions run under the cache lock.
        let spec = PyRc::new(FormatSpec::parse(format, vm)?);
        let mut entries = self.entries.write();
        if let Some(spec) = entries.get(format) {
            return Ok(spec.clone());
        }
        if entries.len() >= 100 {
            entries.clear();
        }
        entries.insert(format.into(), spec.clone());
        Ok(spec)
    }

    pub fn clear(&self) {
        *self.entries.write() = HashMap::new();
    }
}

trait PackBuffer {
    fn clear(&mut self, size: usize);
    fn write(&mut self, offset: usize, data: &[u8]);
    fn needs_source_snapshot(&self) -> bool {
        false
    }
}

impl PackBuffer for [u8] {
    fn clear(&mut self, size: usize) {
        self[..size].fill(0);
    }

    fn write(&mut self, offset: usize, data: &[u8]) {
        self[offset..offset + data.len()].copy_from_slice(data);
    }
}

struct MemoryPackBuffer<'a> {
    buffer: &'a ArgMemoryBuffer,
    offset: usize,
}

impl PackBuffer for MemoryPackBuffer<'_> {
    fn needs_source_snapshot(&self) -> bool {
        true
    }

    fn clear(&mut self, size: usize) {
        self.buffer
            .with_ref(|data| data[self.offset..self.offset + size].fill(0));
    }

    fn write(&mut self, offset: usize, bytes: &[u8]) {
        let start = self.offset + offset;
        self.buffer
            .with_ref(|data| data[start..start + bytes.len()].copy_from_slice(bytes));
    }
}

impl FormatSpec {
    pub fn parse(fmt: &[u8], vm: &VirtualMachine) -> PyResult<Self> {
        // prepare_s rejects embedded NULs before examining any format code.
        if fmt.contains(&0) {
            return Err(new_struct_error(vm, exceptions::NulError.to_string()));
        }
        let mut chars = fmt.iter().copied().peekable();

        // First determine "@", "<", ">","!" or "="
        let endianness = Endianness::parse(&mut chars);

        // Now, analyze struct string further:
        let (codes, size, arg_count) =
            FormatCode::parse(&mut chars, endianness).map_err(|err| new_struct_error(vm, err))?;

        Ok(Self {
            endianness,
            codes,
            size,
            arg_count,
        })
    }

    pub fn pack(&self, args: Vec<PyObjectRef>, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let mut data = vm.new_zeroed_bytes(self.size)?;
        self.pack_into(&mut data, args, vm)?;
        Ok(data)
    }

    pub fn pack_into(
        &self,
        buffer: &mut [u8],
        args: Vec<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self.pack_to(buffer, &args, "pack", vm)
    }

    pub fn pack_into_buffer(
        &self,
        buffer: &ArgMemoryBuffer,
        offset: usize,
        args: Vec<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self.pack_to(
            &mut MemoryPackBuffer { buffer, offset },
            &args,
            "pack_into",
            vm,
        )
    }

    fn pack_to(
        &self,
        buffer: &mut (impl PackBuffer + ?Sized),
        args: &[PyObjectRef],
        operation: &str,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if self.arg_count != args.len() {
            return Err(new_struct_error(
                vm,
                format!(
                    "{operation} expected {} items for packing (got {})",
                    self.arg_count,
                    args.len()
                ),
            ));
        }

        buffer.clear(self.size);
        let mut args = args.iter();
        let mut offset = 0;
        // Loop over all opcodes:
        for code in &self.codes {
            offset += code.pre_padding;
            debug!("code: {code:?}");
            match code.code {
                FormatType::Str | FormatType::Pascal => {
                    pack_string(vm, args.next().unwrap(), code, buffer, offset)?;
                    offset += code.repeat;
                }
                FormatType::Pad => {
                    offset += code.repeat;
                }
                _ => {
                    let pack = code.info.pack.unwrap();
                    for arg in args.by_ref().take(code.repeat) {
                        let mut cursor = offset;
                        pack(vm, code.code, arg, &mut |bytes| {
                            buffer.write(cursor, bytes);
                            cursor += bytes.len();
                        })?;
                        offset += code.info.size;
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
            debug!("unpack code: {code:?}");
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
    #[must_use]
    pub const fn size(&self) -> usize {
        self.size
    }

    #[must_use]
    pub fn codes_sizeof(&self) -> usize {
        core::mem::size_of::<FormatCode>() * (self.codes.len() + 1)
    }
}

trait Packable {
    fn pack<E: ByteOrder>(
        vm: &VirtualMachine,
        code: FormatType,
        arg: &PyObject,
        write: &mut dyn FnMut(&[u8]),
    ) -> PyResult<()>;
    fn unpack<E: ByteOrder>(vm: &VirtualMachine, data: &[u8]) -> PyObjectRef;
}

trait PackInt: PrimInt {
    fn pack_int<E: ByteOrder>(self, write: &mut dyn FnMut(&[u8]));
    fn unpack_int<E: ByteOrder>(data: &[u8]) -> Self;
}

macro_rules! make_pack_prim_int {
    ($T:ty) => {
        impl PackInt for $T {
            fn pack_int<E: ByteOrder>(self, write: &mut dyn FnMut(&[u8])) {
                let i = E::convert(self);
                write(&i.to_ne_bytes());
            }
            #[inline]
            fn unpack_int<E: ByteOrder>(data: &[u8]) -> Self {
                let mut x = [0; core::mem::size_of::<$T>()];
                x.copy_from_slice(data);
                E::convert(<$T>::from_ne_bytes(x))
            }
        }

        impl Packable for $T {
            fn pack<E: ByteOrder>(
                vm: &VirtualMachine,
                code: FormatType,
                arg: &PyObject,
                write: &mut dyn FnMut(&[u8]),
            ) -> PyResult<()> {
                let i: $T = get_int_or_index(vm, code, arg)?;
                i.pack_int::<E>(write);
                Ok(())
            }

            fn unpack<E: ByteOrder>(vm: &VirtualMachine, rdr: &[u8]) -> PyObjectRef {
                let i = <$T>::unpack_int::<E>(rdr);
                vm.ctx.new_int(i).into()
            }
        }
    };
}

fn get_int_or_index<T>(vm: &VirtualMachine, code: FormatType, arg: &PyObject) -> PyResult<T>
where
    T: PrimInt + fmt::Display + for<'a> TryFrom<&'a BigInt>,
{
    let index = match arg.try_index_opt(vm) {
        None => {
            return Err(new_struct_error(vm, "required argument is not an integer"));
        }
        Some(Err(e)) => return Err(e),
        Some(Ok(index)) => index,
    };
    index.try_to_primitive(vm).map_err(|_| {
        // A pointer is converted rather than checked against the range of a
        // named format, so what it reports is the conversion failing.
        let msg = if code == FormatType::VoidP {
            "int too large to convert".to_owned()
        } else {
            format!(
                "'{}' format requires {} <= number <= {}",
                code as u8 as char,
                T::min_value(),
                T::max_value()
            )
        };
        new_struct_error(vm, msg)
    })
}

make_pack_prim_int!(i8);
make_pack_prim_int!(u8);
make_pack_prim_int!(i16);
make_pack_prim_int!(u16);
make_pack_prim_int!(i32);
make_pack_prim_int!(u32);
make_pack_prim_int!(i64);
make_pack_prim_int!(u64);
make_pack_prim_int!(usize);
make_pack_prim_int!(isize);

macro_rules! make_pack_float {
    ($T:ty, $fmt:literal) => {
        impl Packable for $T {
            fn pack<E: ByteOrder>(
                vm: &VirtualMachine,
                _code: FormatType,
                arg: &PyObject,
                write: &mut dyn FnMut(&[u8]),
            ) -> PyResult<()> {
                let f_64 = arg.try_to_value::<ArgIntoFloat>(vm)?.into_float();
                let f = f_64 as $T;
                if f.is_infinite() != f_64.is_infinite() {
                    return Err(vm.new_overflow_error(concat!(
                        "float too large to pack with ",
                        $fmt,
                        " format"
                    )));
                }
                f.to_bits().pack_int::<E>(write);
                Ok(())
            }

            fn unpack<E: ByteOrder>(vm: &VirtualMachine, rdr: &[u8]) -> PyObjectRef {
                let i = PackInt::unpack_int::<E>(rdr);
                <$T>::from_bits(i).to_pyobject(vm)
            }
        }
    };
}

make_pack_float!(f32, "f");
make_pack_float!(f64, "d");

#[repr(C)]
struct PackFloatComplex(f32, f32);

#[repr(C)]
struct PackDoubleComplex(f64, f64);

macro_rules! make_pack_complex {
    ($T:ty, $Elem:ty, $Bits:ty, $fmt:literal) => {
        impl Packable for $T {
            fn pack<E: ByteOrder>(
                vm: &VirtualMachine,
                _code: FormatType,
                arg: &PyObject,
                write: &mut dyn FnMut(&[u8]),
            ) -> PyResult<()> {
                let c = arg
                    .try_to_value::<ArgIntoComplex>(vm)
                    .map_err(|_| new_struct_error(vm, "required argument is not a complex"))?
                    .into_complex();
                for component in [c.re, c.im] {
                    let narrowed = component as $Elem;
                    // CPython uses native casts for matching-endian complex formats.
                    if E::convert(1u16) != 1 && narrowed.is_infinite() != component.is_infinite() {
                        return Err(vm.new_overflow_error(concat!(
                            "float too large to pack with ",
                            $fmt,
                            " format"
                        )));
                    }
                    narrowed.to_bits().pack_int::<E>(write);
                }
                Ok(())
            }

            fn unpack<E: ByteOrder>(vm: &VirtualMachine, rdr: &[u8]) -> PyObjectRef {
                let half = size_of::<$Elem>();
                let re = <$Elem>::from_bits(<$Bits>::unpack_int::<E>(&rdr[..half])) as f64;
                let im = <$Elem>::from_bits(<$Bits>::unpack_int::<E>(&rdr[half..half * 2])) as f64;
                vm.ctx.new_complex(Complex64::new(re, im)).into()
            }
        }
    };
}

make_pack_complex!(PackFloatComplex, f32, u32, "f");
make_pack_complex!(PackDoubleComplex, f64, u64, "d");

/// PyFloat_Pack2: round the full binary64 significand to nearest-even and
/// preserve the sign, signaling bit and representable payload of NaNs.
pub(crate) fn pack_float16(value: f64) -> Option<u16> {
    let bits = value.to_bits();
    let sign = ((bits >> 48) & 0x8000) as u16;
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & 0x000f_ffff_ffff_ffff;
    if exponent == 0x7ff {
        let mut payload = (fraction >> 42) as u16;
        if fraction != 0 && payload == 0 {
            payload = 0x0200;
        }
        return Some(sign | 0x7c00 | payload);
    }
    let exponent = exponent - 1023;
    if exponent > 15 {
        return None;
    }
    if exponent < -25 {
        return Some(sign);
    }
    let significand = (1_u64 << 52) | fraction;
    let shift = if exponent < -14 {
        (28 - exponent) as u32
    } else {
        42
    };
    let truncated = significand >> shift;
    let remainder = significand & ((1_u64 << shift) - 1);
    let halfway = 1_u64 << (shift - 1);
    let rounded =
        truncated + u64::from(remainder > halfway || (remainder == halfway && truncated & 1 != 0));
    let exponent_bits = if exponent < -14 {
        0
    } else {
        ((exponent + 14) as u16) << 10
    };
    let magnitude = exponent_bits + rounded as u16;
    (magnitude < 0x7c00).then_some(sign | magnitude)
}

/// PyFloat_Unpack2. A hardware half conversion would quiet signaling NaNs.
fn unpack_float16(bits: u16) -> f64 {
    if bits & 0x7fff > 0x7c00 {
        f64::from_bits(
            (u64::from(bits & 0x8000) << 48)
                | 0x7ff0_0000_0000_0000
                | (u64::from(bits & 0x03ff) << 42),
        )
    } else {
        f16::from_bits(bits).to_f64()
    }
}

/// PyFloat_Unpack4, as used by memoryview's complex scalar conversion.
pub(crate) fn unpack_float32(bits: u32) -> f64 {
    if bits & 0x7fff_ffff > 0x7f80_0000 {
        f64::from_bits(
            (u64::from(bits & 0x8000_0000) << 32)
                | 0x7ff0_0000_0000_0000
                | (u64::from(bits & 0x007f_ffff) << 29),
        )
    } else {
        f32::from_bits(bits) as f64
    }
}

impl Packable for f16 {
    fn pack<E: ByteOrder>(
        vm: &VirtualMachine,
        _code: FormatType,
        arg: &PyObject,
        write: &mut dyn FnMut(&[u8]),
    ) -> PyResult<()> {
        let f_64 = arg.try_to_value::<ArgIntoFloat>(vm)?.into_float();
        let bits = pack_float16(f_64)
            .ok_or_else(|| vm.new_overflow_error("float too large to pack with e format"))?;
        bits.pack_int::<E>(write);

        Ok(())
    }

    fn unpack<E: ByteOrder>(vm: &VirtualMachine, rdr: &[u8]) -> PyObjectRef {
        let i = PackInt::unpack_int::<E>(rdr);
        unpack_float16(i).to_pyobject(vm)
    }
}

impl Packable for *mut raw::c_void {
    fn pack<E: ByteOrder>(
        vm: &VirtualMachine,
        code: FormatType,
        arg: &PyObject,
        write: &mut dyn FnMut(&[u8]),
    ) -> PyResult<()> {
        usize::pack::<E>(vm, code, arg, write)
    }

    fn unpack<E: ByteOrder>(vm: &VirtualMachine, rdr: &[u8]) -> PyObjectRef {
        usize::unpack::<E>(vm, rdr)
    }
}

impl Packable for bool {
    fn pack<E: ByteOrder>(
        vm: &VirtualMachine,
        _code: FormatType,
        arg: &PyObject,
        write: &mut dyn FnMut(&[u8]),
    ) -> PyResult<()> {
        let v = arg.try_to_value::<ArgIntoBool>(vm)?.into_bool() as u8;
        v.pack_int::<E>(write);
        Ok(())
    }

    fn unpack<E: ByteOrder>(vm: &VirtualMachine, rdr: &[u8]) -> PyObjectRef {
        let i = u8::unpack_int::<E>(rdr);
        vm.ctx.new_bool(i != 0).into()
    }
}

fn pack_char(
    vm: &VirtualMachine,
    _code: FormatType,
    arg: &PyObject,
    write: &mut dyn FnMut(&[u8]),
) -> PyResult<()> {
    let v = arg.try_to_ref::<PyBytes>(vm)?;
    let ch = *v
        .as_bytes()
        .iter()
        .exactly_one()
        .map_err(|_| new_struct_error(vm, "char format requires a bytes object of length 1"))?;
    write(&[ch]);
    Ok(())
}

fn pack_string(
    vm: &VirtualMachine,
    arg: &PyObject,
    code: &FormatCode,
    buffer: &mut (impl PackBuffer + ?Sized),
    offset: usize,
) -> PyResult<()> {
    let pascal = code.code == FormatType::Pascal;
    let capacity = if pascal {
        code.repeat.saturating_sub(1)
    } else {
        code.repeat
    };
    let needs_snapshot = buffer.needs_source_snapshot();
    let mut write = |data: &[u8]| {
        let len = data.len().min(capacity);
        let start = offset + usize::from(pascal);
        if len != 0 {
            buffer.write(start, &data[..len]);
        }
        if pascal && code.repeat != 0 {
            buffer.write(offset, &[len.min(255) as u8]);
        }
    };
    if let Some(bytes) = arg.downcast_ref::<PyBytes>() {
        write(bytes.as_bytes());
    } else if let Some(bytes) = arg.downcast_ref::<PyByteArray>() {
        if needs_snapshot {
            let data = {
                let data = bytes.borrow_buf();
                data[..data.len().min(capacity)].to_vec()
            };
            write(&data);
        } else {
            write(&bytes.borrow_buf());
        }
    } else {
        return Err(new_struct_error(
            vm,
            format!(
                "argument for '{}' must be a bytes object",
                code.code as u8 as char
            ),
        ));
    }
    Ok(())
}

fn unpack_char(vm: &VirtualMachine, data: &[u8]) -> PyObjectRef {
    vm.ctx.new_bytes(vec![data[0]]).into()
}

fn unpack_pascal(vm: &VirtualMachine, data: &[u8]) -> PyObjectRef {
    let (&len, data) = match data.split_first() {
        Some(x) => x,
        None => {
            // Zero-width Pascal strings unpack to empty bytes (CPython 3.15).
            return vm.ctx.new_bytes(vec![]).into();
        }
    };
    let len = core::cmp::min(len as usize, data.len());
    vm.ctx.new_bytes(data[..len].to_vec()).into()
}

// XXX: are those functions expected to be placed here?
pub fn struct_error_type(vm: &VirtualMachine) -> &'static Py<PyType> {
    static_cell! {
        static INSTANCE: PyTypeRef;
    }
    INSTANCE.get_or_init(|| vm.ctx.new_exception_type("struct", "error", None))
}

pub fn new_struct_error<T: Into<Wtf8Buf>>(vm: &VirtualMachine, msg: T) -> PyBaseExceptionRef {
    // can't just STRUCT_ERROR.get().unwrap() cause this could be called before from buffer
    // machinery, independent of whether _struct was ever imported
    vm.new_exception_msg(struct_error_type(vm).to_owned(), msg.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Interpreter;

    #[test]
    fn struct_parser_rejects_exporter_extensions_and_dangling_counts() {
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            for format in ["B\u{b}", "B\u{c}", " \t\n\r\u{b}\u{c}B"] {
                assert_eq!(FormatSpec::parse(format.as_bytes(), vm).unwrap().size(), 1);
            }
            for format in [
                "1", "B1", "0", "1 ", "B@B", "<P", "u", "w", "g", "O", "T{B:x:}", "(2)B", "&B",
                "B:name:",
            ] {
                assert!(
                    FormatSpec::parse(format.as_bytes(), vm).is_err(),
                    "{format:?}"
                );
            }
        });
    }

    #[test]
    fn half_conversion_preserves_every_encoding_and_rounds_full_precision() {
        for bits in 0..=u16::MAX {
            assert_eq!(pack_float16(unpack_float16(bits)), Some(bits), "{bits:04x}");
        }
        for bits in 0..0x7bff_u16 {
            let low = unpack_float16(bits);
            let high = unpack_float16(bits + 1);
            let midpoint = (low + high) * 0.5;
            let below = f64::from_bits(midpoint.to_bits() - 1);
            let above = f64::from_bits(midpoint.to_bits() + 1);
            let even = bits + (bits & 1);
            for (value, expected) in [(below, bits), (midpoint, even), (above, bits + 1)] {
                assert_eq!(pack_float16(value), Some(expected));
                assert_eq!(pack_float16(-value), Some(expected | 0x8000));
            }
        }
        assert_eq!(pack_float16(65520.0), None);
        assert_eq!(pack_float16(-65520.0), None);
        assert_eq!(
            pack_float16(f64::from_bits(0x7ff0_0000_0000_0001)),
            Some(0x7e00)
        );
        assert_eq!(
            pack_float16(f64::from_bits(0xfff0_0000_0000_0001)),
            Some(0xfe00)
        );
    }

    #[test]
    fn format_cache_reuses_specs_and_releases_evicted_entries() {
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let cache = &vm.state.struct_format_cache;
            let spec = cache.get_or_parse(b"<IH", vm).unwrap();
            assert!(PyRc::ptr_eq(
                &spec,
                &cache.get_or_parse(b"<IH", vm).unwrap()
            ));
            let weak = PyRc::downgrade(&spec);
            drop(spec);

            for padding in 0..100 {
                let format = format!("{padding}x");
                cache.get_or_parse(format.as_bytes(), vm).unwrap();
            }
            assert!(weak.upgrade().is_none());

            let spec = cache.get_or_parse(b"<IH", vm).unwrap();
            let weak = PyRc::downgrade(&spec);
            drop(spec);
            cache.clear();
            assert!(weak.upgrade().is_none());
        });
    }
}
