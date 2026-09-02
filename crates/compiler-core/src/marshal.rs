use crate::{OneIndexed, SourceLocation, bytecode::*};
use alloc::{boxed::Box, vec::Vec};
use core::convert::Infallible;
use malachite_bigint::{BigInt, Sign};
use num_complex::Complex64;
use rustpython_wtf8::Wtf8;

pub const FORMAT_VERSION: u32 = 5;

#[derive(Clone, Copy, Debug)]
pub enum MarshalError {
    /// Unexpected End Of File while reading object data
    Eof,
    /// Unexpected End Of File where an object was expected
    EofObject,
    /// Unexpected End Of File at a direct byte read (e.g. a length byte)
    EofByte,
    /// A run of bytes that the data ends in the middle of
    DataTooShort,
    /// Nesting deeper than MAX_MARSHAL_STACK_DEPTH
    RecursionLimitExceeded,
    /// TYPE_NULL read where an object was expected
    NullObject,
    /// TYPE_NULL read as a tuple element
    NullInTuple,
    /// TYPE_NULL read as a list element
    NullInList,
    /// TYPE_NULL read as a set element
    NullInSet,
    /// TYPE_NULL read as a code object field
    NullInCode,
    /// Corrupt data; payload is the parenthetical of "bad marshal data (...)"
    BadData(&'static str),
    /// Error reported out-of-band by the reader (e.g. a Python-level read error)
    ReadFailed,
    /// Invalid Bytecode
    InvalidBytecode,
    /// Invalid utf8 in string
    InvalidUtf8,
    /// Invalid source location
    InvalidLocation,
    /// Bad type marker
    BadType,
    /// A type marker no reader knows
    UnknownType,
    /// A back reference that names nothing
    InvalidRef,
    /// A container length that is negative or does not fit, named by what it counts
    BadSize(&'static str),
}

impl MarshalError {
    /// Report a bare TYPE_NULL with the containing object's context.
    fn null_in(self, container: Self) -> Self {
        match self {
            Self::NullObject => container,
            e => e,
        }
    }
}

impl core::fmt::Display for MarshalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Eof => f.write_str("unexpected end of data"),
            Self::EofObject => f.write_str("unexpected end of data at object boundary"),
            Self::EofByte => f.write_str("unexpected end of data at byte read"),
            Self::DataTooShort => f.write_str("data ends in the middle of a value"),
            Self::RecursionLimitExceeded => f.write_str("recursion limit exceeded"),
            Self::NullObject => f.write_str("null object in marshal data"),
            Self::NullInTuple => f.write_str("null object in marshal data for tuple"),
            Self::NullInList => f.write_str("null object in marshal data for list"),
            Self::NullInSet => f.write_str("null object in marshal data for set"),
            Self::NullInCode => f.write_str("null object in marshal data for code object"),
            Self::BadData(msg) => write!(f, "bad marshal data ({msg})"),
            Self::ReadFailed => f.write_str("read failed"),
            Self::InvalidBytecode => f.write_str("invalid bytecode"),
            Self::InvalidUtf8 => f.write_str("invalid utf8"),
            Self::InvalidLocation => f.write_str("invalid source location"),
            Self::BadType => f.write_str("bad type marker"),
            Self::UnknownType => f.write_str("unknown type code"),
            Self::InvalidRef => f.write_str("invalid reference"),
            Self::BadSize(what) => write!(f, "{what} size out of range"),
        }
    }
}

/// Remap a payload EOF to a direct-`r_byte` EOF (CPython length-byte reads).
fn eof_at_byte(e: MarshalError) -> MarshalError {
    match e {
        MarshalError::Eof => MarshalError::EofByte,
        e => e,
    }
}

impl From<core::str::Utf8Error> for MarshalError {
    fn from(_: core::str::Utf8Error) -> Self {
        Self::InvalidUtf8
    }
}

impl core::error::Error for MarshalError {}

type Result<T, E = MarshalError> = core::result::Result<T, E>;

#[derive(Clone, Copy)]
#[repr(u8)]
enum Type {
    Null = b'0',
    None = b'N',
    False = b'F',
    True = b'T',
    StopIter = b'S',
    Ellipsis = b'.',
    Int = b'i',
    Int64 = b'I',
    Long = b'l',
    Float = b'g',
    FloatStr = b'f',
    ComplexStr = b'x',
    Complex = b'y',
    Bytes = b's',
    Interned = b't',
    Ref = b'r',
    Tuple = b'(',
    SmallTuple = b')',
    List = b'[',
    Dict = b'{',
    Code = b'c',
    Unicode = b'u',
    Set = b'<',
    FrozenSet = b'>',
    Slice = b':',
    Ascii = b'a',
    AsciiInterned = b'A',
    ShortAscii = b'z',
    ShortAsciiInterned = b'Z',
}

impl TryFrom<u8> for Type {
    type Error = MarshalError;

    fn try_from(value: u8) -> Result<Self> {
        Ok(match value {
            b'0' => Self::Null,
            b'N' => Self::None,
            b'F' => Self::False,
            b'T' => Self::True,
            b'S' => Self::StopIter,
            b'.' => Self::Ellipsis,
            b'i' => Self::Int,
            b'I' => Self::Int64,
            b'l' => Self::Long,
            b'f' => Self::FloatStr,
            b'g' => Self::Float,
            b'x' => Self::ComplexStr,
            b'y' => Self::Complex,
            b's' => Self::Bytes,
            b't' => Self::Interned,
            b'r' => Self::Ref,
            b'(' => Self::Tuple,
            b')' => Self::SmallTuple,
            b'[' => Self::List,
            b'{' => Self::Dict,
            b'c' => Self::Code,
            b'u' => Self::Unicode,
            b'<' => Self::Set,
            b'>' => Self::FrozenSet,
            b':' => Self::Slice,
            b'a' => Self::Ascii,
            b'A' => Self::AsciiInterned,
            b'z' => Self::ShortAscii,
            b'Z' => Self::ShortAsciiInterned,
            _ => return Err(MarshalError::UnknownType),
        })
    }
}

pub trait Read {
    fn read_slice(&mut self, n: u32) -> Result<&[u8]>;

    fn read_array<const N: usize>(&mut self) -> Result<&[u8; N]> {
        self.read_slice(N as u32).map(|s| s.try_into().unwrap())
    }

    fn read_str(&mut self, len: u32) -> Result<&str> {
        Ok(core::str::from_utf8(self.read_slice(len)?)?)
    }

    fn read_wtf8(&mut self, len: u32) -> Result<&Wtf8> {
        Wtf8::from_bytes(self.read_slice(len)?).ok_or(MarshalError::InvalidUtf8)
    }

    fn read_u8(&mut self) -> Result<u8> {
        // A single byte is not a run that got cut short: nothing came at all.
        let byte = self.read_array().map_err(|_| MarshalError::Eof)?;
        Ok(u8::from_le_bytes(*byte))
    }

    /// Read a type byte at an object boundary (CPython `r_object`'s `r_byte`).
    fn read_type_byte(&mut self) -> Result<u8> {
        self.read_u8().map_err(|e| match e {
            MarshalError::Eof => MarshalError::EofObject,
            e => e,
        })
    }

    fn read_u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(*self.read_array()?))
    }

    fn read_u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(*self.read_array()?))
    }

    fn read_u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(*self.read_array()?))
    }

    /// A length, read the way `r_long` reads one: it is signed, so a value
    /// with the top bit set is out of range rather than four billion items.
    fn read_len(&mut self, what: &'static str) -> Result<usize> {
        let len = self.read_u32()? as i32;
        usize::try_from(len).map_err(|_| MarshalError::BadSize(what))
    }
}

pub(crate) trait ReadBorrowed<'a>: Read {
    fn read_slice_borrow(&mut self, n: u32) -> Result<&'a [u8]>;

    fn read_str_borrow(&mut self, len: u32) -> Result<&'a str> {
        Ok(core::str::from_utf8(self.read_slice_borrow(len)?)?)
    }
}

impl Read for &[u8] {
    fn read_slice(&mut self, n: u32) -> Result<&[u8]> {
        self.read_slice_borrow(n)
    }

    fn read_array<const N: usize>(&mut self) -> Result<&[u8; N]> {
        let (chunk, rest) = self
            .split_first_chunk::<N>()
            .ok_or(MarshalError::DataTooShort)?;
        *self = rest;
        Ok(chunk)
    }
}

impl<'a> ReadBorrowed<'a> for &'a [u8] {
    fn read_slice_borrow(&mut self, n: u32) -> Result<&'a [u8]> {
        self.split_off(..n as usize)
            .ok_or(MarshalError::DataTooShort)
    }
}

pub struct Cursor<B> {
    pub data: B,
    pub position: usize,
}

impl<B: AsRef<[u8]>> Read for Cursor<B> {
    fn read_slice(&mut self, n: u32) -> Result<&[u8]> {
        let data = &self.data.as_ref()[self.position..];
        let slice = data.get(..n as usize).ok_or(MarshalError::DataTooShort)?;
        self.position += n as usize;
        Ok(slice)
    }
}

/// Deserialize a code object (CPython field order).
pub fn deserialize_code<R: Read, Bag: ConstantBag>(
    rdr: &mut R,
    bag: Bag,
) -> Result<CodeObject<Bag::Constant>> {
    let mut refs: Vec<Option<Bag::Constant>> = Vec::new();
    deserialize_code_inner(rdr, bag, MAX_MARSHAL_STACK_DEPTH, &mut refs)
}

/// Inner code-object deserializer that shares a ref table with caller.
/// Used when decoding a code object embedded in another marshal stream so
/// that TYPE_REF entries inside the code can resolve across nested values.
fn deserialize_code_inner<R: Read, Bag: ConstantBag>(
    rdr: &mut R,
    bag: Bag,
    depth: usize,
    refs: &mut Vec<Option<Bag::Constant>>,
) -> Result<CodeObject<Bag::Constant>> {
    if depth == 0 {
        return Err(MarshalError::RecursionLimitExceeded);
    }
    // 1–5: scalar fields
    let arg_count = rdr.read_u32()?;
    let posonlyarg_count = rdr.read_u32()?;
    let kwonlyarg_count = rdr.read_u32()?;
    let max_stackdepth = rdr.read_u32()?;
    let flags = CodeFlags::from_bits_truncate(rdr.read_u32()?);

    // 6: co_code
    let code_bytes =
        read_marshal_bytes(rdr, &bag, refs).map_err(|e| e.null_in(MarshalError::NullInCode))?;

    // 7: co_consts
    let constants = read_marshal_const_tuple(rdr, bag, depth, refs)
        .map_err(|e| e.null_in(MarshalError::NullInCode))?;

    // 8: co_names
    let names = read_marshal_name_tuple(rdr, &bag, refs)
        .map_err(|e| e.null_in(MarshalError::NullInCode))?;

    // 9: co_localsplusnames
    let localsplusnames =
        read_marshal_str_vec(rdr, &bag, refs).map_err(|e| e.null_in(MarshalError::NullInCode))?;

    // 10: co_localspluskinds
    let localspluskinds =
        read_marshal_bytes(rdr, &bag, refs).map_err(|e| e.null_in(MarshalError::NullInCode))?;

    // 11–13: filename, name, qualname
    let source_path = bag.make_name(
        &read_marshal_str(rdr, &bag, refs).map_err(|e| e.null_in(MarshalError::NullInCode))?,
    );
    let obj_name = bag.make_name(
        &read_marshal_str(rdr, &bag, refs).map_err(|e| e.null_in(MarshalError::NullInCode))?,
    );
    let qualname = bag.make_name(
        &read_marshal_str(rdr, &bag, refs).map_err(|e| e.null_in(MarshalError::NullInCode))?,
    );

    // 14: co_firstlineno
    let first_line_raw = rdr.read_u32()? as i32;
    let first_line_number = if first_line_raw > 0 {
        OneIndexed::new(first_line_raw as usize)
    } else {
        None
    };

    // 15–16: linetable, exceptiontable
    let linetable = read_marshal_bytes(rdr, &bag, refs)
        .map_err(|e| e.null_in(MarshalError::NullInCode))?
        .into_boxed_slice();
    let exceptiontable = read_marshal_bytes(rdr, &bag, refs)
        .map_err(|e| e.null_in(MarshalError::NullInCode))?
        .into_boxed_slice();

    // Split localsplusnames/kinds → varnames/cellvars/freevars
    let lp = split_localplus(
        &localsplusnames
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<&str>>(),
        &localspluskinds,
        arg_count,
        kwonlyarg_count,
        flags,
    )?;

    // Bytecode already uses flat localsplus indices (no translation needed)
    let instructions = CodeUnits::try_from(code_bytes.as_slice())?;
    let locations = linetable_to_locations(&linetable, first_line_raw, instructions.len());

    // Use original localspluskinds from marshal data (preserves CO_FAST_HIDDEN etc.)
    let localspluskinds = localspluskinds.into_boxed_slice();

    Ok(CodeObject {
        instructions,
        locations,
        flags,
        posonlyarg_count,
        arg_count,
        kwonlyarg_count,
        source_path,
        first_line_number,
        max_stackdepth,
        obj_name,
        qualname,
        constants,
        names,
        varnames: lp.varnames.iter().map(|s| bag.make_name(s)).collect(),
        cellvars: lp.cellvars.iter().map(|s| bag.make_name(s)).collect(),
        freevars: lp.freevars.iter().map(|s| bag.make_name(s)).collect(),
        localspluskinds,
        linetable,
        exceptiontable,
    })
}

/// Reserve a ref slot if `FLAG_REF` was present, returning its index.
fn reserve_ref_slot<T>(has_flag: bool, refs: &mut Vec<Option<T>>) -> Result<Option<usize>> {
    if has_flag {
        let idx = refs.len();
        if idx >= 0x7fff_fffe {
            return Err(MarshalError::BadData("index list too large"));
        }
        refs.push(None);
        Ok(Some(idx))
    } else {
        Ok(None)
    }
}

/// Resolve a TYPE_REF index, returning the previously stored value.
fn resolve_ref<T: Clone>(idx: usize, refs: &[Option<T>]) -> Result<T> {
    refs.get(idx)
        .and_then(|v| v.clone())
        .ok_or(MarshalError::InvalidRef)
}

/// Read a marshal bytes object (TYPE_STRING = b's'), resolving TYPE_REF
/// and registering this read in the ref table when `FLAG_REF` is set.
fn read_marshal_bytes<R: Read, Bag: ConstantBag>(
    rdr: &mut R,
    bag: &Bag,
    refs: &mut Vec<Option<Bag::Constant>>,
) -> Result<Vec<u8>> {
    let raw = rdr.read_type_byte()?;
    let type_byte = raw & !FLAG_REF;
    let has_flag = raw & FLAG_REF != 0;

    if type_byte == Type::Ref as u8 {
        let idx = rdr.read_u32()? as usize;
        let stored = resolve_ref(idx, refs)?;
        return match stored.borrow_constant() {
            BorrowedConstant::Bytes { value } => Ok(value.to_vec()),
            _ => Err(MarshalError::BadType),
        };
    }

    if type_byte == Type::Null as u8 {
        return Err(MarshalError::NullObject);
    }
    if type_byte != Type::Bytes as u8 {
        return Err(if Type::try_from(type_byte).is_ok() {
            MarshalError::BadType
        } else {
            MarshalError::BadData("unknown type code")
        });
    }

    let slot = reserve_ref_slot(has_flag, refs)?;
    let len = read_i32(rdr)?;
    if len < 0 {
        return Err(MarshalError::BadData("bytes object size out of range"));
    }
    let bytes = rdr.read_slice(len as u32)?.to_vec();
    if let Some(idx) = slot {
        refs[idx] =
            Some(bag.make_constant::<Bag::Constant>(BorrowedConstant::Bytes { value: &bytes }));
    }
    Ok(bytes)
}

/// Read a marshal string object, resolving TYPE_REF and registering
/// this read in the ref table when `FLAG_REF` is set.
fn read_marshal_str<R: Read, Bag: ConstantBag>(
    rdr: &mut R,
    bag: &Bag,
    refs: &mut Vec<Option<Bag::Constant>>,
) -> Result<alloc::string::String> {
    let raw = rdr.read_type_byte()?;
    let type_byte = raw & !FLAG_REF;
    let has_flag = raw & FLAG_REF != 0;

    if type_byte == Type::Ref as u8 {
        let idx = rdr.read_u32()? as usize;
        let stored = resolve_ref(idx, refs)?;
        return match stored.borrow_constant() {
            BorrowedConstant::Str { value } => Ok(value.to_string_lossy().into_owned()),
            _ => Err(MarshalError::BadType),
        };
    }

    if type_byte == Type::Null as u8 {
        return Err(MarshalError::NullObject);
    }

    let slot = reserve_ref_slot(has_flag, refs)?;
    let owned = match type_byte {
        b'u' | b't' | b'a' | b'A' => {
            let len = read_i32(rdr)?;
            if len < 0 {
                return Err(MarshalError::BadData("string size out of range"));
            }
            alloc::string::String::from(rdr.read_str(len as u32)?)
        }
        b'z' | b'Z' => {
            let len = rdr.read_u8().map_err(eof_at_byte)? as u32;
            alloc::string::String::from(rdr.read_str(len)?)
        }
        _ if Type::try_from(type_byte).is_ok() => return Err(MarshalError::BadType),
        _ => return Err(MarshalError::BadData("unknown type code")),
    };
    if let Some(idx) = slot {
        refs[idx] = Some(bag.make_constant::<Bag::Constant>(BorrowedConstant::Str {
            value: Wtf8::new(owned.as_str()),
        }));
    }
    Ok(owned)
}

/// Read a marshal tuple of strings, returning owned Strings.
fn read_marshal_str_vec<R: Read, Bag: ConstantBag>(
    rdr: &mut R,
    bag: &Bag,
    refs: &mut Vec<Option<Bag::Constant>>,
) -> Result<Vec<alloc::string::String>> {
    let raw = rdr.read_type_byte()?;
    let type_byte = raw & !FLAG_REF;
    let has_flag = raw & FLAG_REF != 0;

    if type_byte == Type::Ref as u8 {
        let idx = rdr.read_u32()? as usize;
        let stored = resolve_ref(idx, refs)?;
        return match stored.borrow_constant() {
            BorrowedConstant::Tuple { elements } => elements
                .iter()
                .map(|c| match c.borrow_constant() {
                    BorrowedConstant::Str { value } => Ok(value.to_string_lossy().into_owned()),
                    _ => Err(MarshalError::BadType),
                })
                .collect(),
            _ => Err(MarshalError::BadType),
        };
    }

    if type_byte == Type::Null as u8 {
        return Err(MarshalError::NullObject);
    }

    let n = match type_byte {
        b'(' => rdr.read_len("tuple")?,
        b')' => rdr.read_u8()? as usize,
        _ => return Err(MarshalError::BadType),
    };
    let slot = reserve_ref_slot(has_flag, refs)?;
    let items: Vec<alloc::string::String> = (0..n)
        .map(|_| read_marshal_str(rdr, bag, refs).map_err(|e| e.null_in(MarshalError::NullInTuple)))
        .collect::<Result<_>>()?;
    if let Some(idx) = slot {
        let elements: Vec<Bag::Constant> = items
            .iter()
            .map(|s| {
                bag.make_constant::<Bag::Constant>(BorrowedConstant::Str {
                    value: Wtf8::new(s.as_str()),
                })
            })
            .collect();
        refs[idx] = Some(bag.make_constant::<Bag::Constant>(BorrowedConstant::Tuple {
            elements: &elements,
        }));
    }
    Ok(items)
}

fn read_marshal_name_tuple<R: Read, Bag: ConstantBag>(
    rdr: &mut R,
    bag: &Bag,
    refs: &mut Vec<Option<Bag::Constant>>,
) -> Result<Box<[<Bag::Constant as Constant>::Name]>> {
    let names = read_marshal_str_vec(rdr, bag, refs)?;
    Ok(names
        .iter()
        .map(|s| bag.make_name(s))
        .collect::<Vec<_>>()
        .into_boxed_slice())
}

/// Read a marshal tuple of constants. Shares the ref table with the
/// surrounding code-object decode so that nested TYPE_REF entries (for
/// strings, bytes, code objects, etc.) resolve correctly.
fn read_marshal_const_tuple<R: Read, Bag: ConstantBag>(
    rdr: &mut R,
    bag: Bag,
    depth: usize,
    refs: &mut Vec<Option<Bag::Constant>>,
) -> Result<Constants<Bag::Constant>> {
    if depth == 0 {
        return Err(MarshalError::RecursionLimitExceeded);
    }
    let raw = rdr.read_type_byte()?;
    let type_byte = raw & !FLAG_REF;
    let has_flag = raw & FLAG_REF != 0;

    if type_byte == Type::Ref as u8 {
        let idx = rdr.read_u32()? as usize;
        let stored = resolve_ref(idx, refs)?;
        return match stored.borrow_constant() {
            BorrowedConstant::Tuple { elements } => Ok(elements.iter().cloned().collect()),
            _ => Err(MarshalError::BadType),
        };
    }

    if type_byte == Type::Null as u8 {
        return Err(MarshalError::NullObject);
    }

    let n = match type_byte {
        b'(' => rdr.read_len("tuple")?,
        b')' => rdr.read_u8()? as usize,
        _ => return Err(MarshalError::BadType),
    };
    let slot = reserve_ref_slot(has_flag, refs)?;
    let child_depth = depth - 1;
    let items: Vec<Bag::Constant> = (0..n)
        .map(|_| {
            read_const_value(rdr, bag, child_depth, refs)
                .map_err(|e| e.null_in(MarshalError::NullInTuple))
        })
        .collect::<Result<_>>()?;
    if let Some(idx) = slot {
        refs[idx] =
            Some(bag.make_constant::<Bag::Constant>(BorrowedConstant::Tuple { elements: &items }));
    }
    Ok(items.into_iter().collect())
}

/// Read a single value while staying inside an existing code-object ref
/// space. Unlike `deserialize_value_depth`, encountering `Type::Code`
/// here reuses the caller's ref table instead of opening a fresh one —
/// this matches CPython's single global ref space for objects nested
/// inside a code object's const tuple.
fn read_const_value<R: Read, Bag: ConstantBag>(
    rdr: &mut R,
    bag: Bag,
    depth: usize,
    refs: &mut Vec<Option<Bag::Constant>>,
) -> Result<Bag::Constant> {
    if depth == 0 {
        return Err(MarshalError::RecursionLimitExceeded);
    }
    let raw = rdr.read_type_byte()?;
    let flag = raw & FLAG_REF != 0;
    let type_code = raw & !FLAG_REF;

    if type_code == Type::Ref as u8 {
        let idx = rdr.read_u32()? as usize;
        return resolve_ref(idx, refs);
    }

    let slot = reserve_ref_slot(flag, refs)?;
    let typ = Type::try_from(type_code)?;
    let value = if matches!(typ, Type::Code) {
        let code = deserialize_code_inner(rdr, bag, depth - 1, refs)?;
        bag.make_code(code)
    } else {
        deserialize_value_typed(rdr, bag, depth, refs, typ, slot)?
    };
    if let Some(idx) = slot {
        refs[idx] = Some(value.clone());
    }
    Ok(value)
}

pub trait MarshalBag: Copy {
    type Value: Clone;
    type ConstantBag: ConstantBag;

    fn make_bool(&self, value: bool) -> Self::Value;

    fn make_none(&self) -> Self::Value;

    fn make_ellipsis(&self) -> Self::Value;

    fn make_float(&self, value: f64) -> Self::Value;

    fn make_complex(&self, value: Complex64) -> Self::Value;

    fn make_str(&self, value: &Wtf8) -> Self::Value;

    fn make_interned_str(&self, value: &Wtf8) -> Self::Value {
        self.make_str(value)
    }

    fn make_bytes(&self, value: &[u8]) -> Self::Value;

    fn make_int(&self, value: BigInt) -> Self::Value;

    fn make_tuple(&self, elements: impl Iterator<Item = Self::Value>) -> Self::Value;

    fn make_code(
        &self,
        code: CodeObject<<Self::ConstantBag as ConstantBag>::Constant>,
    ) -> Result<Self::Value>;

    /// Construct a runtime code object while retaining the exact values read
    /// from ``co_consts``. Compiler bags ignore this second channel; runtime
    /// bags use it for marshalable values (lists, dicts, sets, recursive
    /// containers) that their compiler constant representation cannot hold.
    fn make_code_with_constants(
        &self,
        code: CodeObject<<Self::ConstantBag as ConstantBag>::Constant>,
        _constants: Vec<Self::Value>,
    ) -> Result<Self::Value> {
        self.make_code(code)
    }

    /// Decode the public `co_code` byte string into the execution-oriented
    /// instruction storage.
    ///
    /// Runtime code-object implementations may accept byte values that the
    /// compiler's `Instruction` enum cannot represent, retaining those bytes
    /// separately and substituting a non-executable placeholder here.  The
    /// default remains the strict compiler representation.
    fn code_units_from_bytes(&self, code_bytes: &[u8]) -> Result<CodeUnits> {
        CodeUnits::try_from(code_bytes)
    }

    /// Construct a runtime code object while retaining both its exact public
    /// `co_code` bytes and the exact values read from `co_consts`.
    ///
    /// The default ignores the redundant byte spelling because ordinary
    /// compiler code is represented losslessly by `CodeUnits`.  Runtime bags
    /// that accepted otherwise unrepresentable opcode bytes in
    /// [`MarshalBag::code_units_from_bytes`] can preserve them here.
    fn make_code_with_constants_and_bytes(
        &self,
        code: CodeObject<<Self::ConstantBag as ConstantBag>::Constant>,
        constants: Vec<Self::Value>,
        _code_bytes: Vec<u8>,
    ) -> Result<Self::Value> {
        self.make_code_with_constants(code, constants)
    }

    fn make_stop_iter(&self) -> Result<Self::Value>;

    fn make_list(&self, it: impl Iterator<Item = Self::Value>) -> Result<Self::Value>;

    fn make_set(&self, it: impl Iterator<Item = Self::Value>) -> Result<Self::Value>;

    fn make_frozenset(&self, it: impl Iterator<Item = Self::Value>) -> Result<Self::Value>;

    fn make_dict(
        &self,
        it: impl Iterator<Item = (Self::Value, Self::Value)>,
    ) -> Result<Self::Value>;

    /// Install partially-built containers in the marshal reference table
    /// before reading their children, as CPython's `r_object()` does.
    /// Runtime bags can opt in; constant bags retain collect-then-construct.
    ///
    /// `len` comes straight from the input and is only bounded by what a
    /// length can hold, so a bag that opts in reports the room it cannot get
    /// rather than taking it for granted.
    fn make_tuple_placeholder(&self, _len: usize) -> Result<Option<Self::Value>> {
        Ok(None)
    }

    fn set_tuple_item(
        &self,
        _tuple: &Self::Value,
        _index: usize,
        _value: Self::Value,
    ) -> Result<()> {
        Err(MarshalError::BadType)
    }

    fn make_list_placeholder(&self, _len: usize) -> Result<Option<Self::Value>> {
        Ok(None)
    }

    fn set_list_item(&self, _list: &Self::Value, _index: usize, _value: Self::Value) -> Result<()> {
        Err(MarshalError::BadType)
    }

    fn make_set_placeholder(&self) -> Option<Self::Value> {
        None
    }

    fn insert_set_item(&self, _set: &Self::Value, _value: Self::Value) -> Result<()> {
        Err(MarshalError::BadType)
    }

    fn make_dict_placeholder(&self) -> Option<Self::Value> {
        None
    }

    fn insert_dict_item(
        &self,
        _dict: &Self::Value,
        _key: Self::Value,
        _value: Self::Value,
    ) -> Result<()> {
        Err(MarshalError::BadType)
    }

    fn make_slice(
        &self,
        _start: Self::Value,
        _stop: Self::Value,
        _step: Self::Value,
    ) -> Result<Self::Value> {
        Err(MarshalError::BadType)
    }

    fn constant_bag(self) -> Self::ConstantBag;

    fn constant_ref_from_value(
        &self,
        _value: &Self::Value,
    ) -> Option<<Self::ConstantBag as ConstantBag>::Constant> {
        None
    }

    /// Convert a runtime constant to the compiler-side shape stored in
    /// ``CodeObject``. Runtime implementations may return a semantically
    /// unused placeholder when the exact value is carried by
    /// `make_code_with_constants` instead.
    fn code_constant_from_value(
        &self,
        value: &Self::Value,
    ) -> Result<<Self::ConstantBag as ConstantBag>::Constant> {
        self.constant_ref_from_value(value)
            .ok_or(MarshalError::BadType)
    }

    fn bytes_from_value(&self, _value: &Self::Value) -> Option<Vec<u8>> {
        None
    }

    fn str_from_value(&self, _value: &Self::Value) -> Option<alloc::string::String> {
        None
    }

    fn tuple_elements_from_value(&self, _value: &Self::Value) -> Option<Vec<Self::Value>> {
        None
    }
}

impl<Bag: ConstantBag> MarshalBag for Bag {
    type Value = Bag::Constant;
    type ConstantBag = Self;

    fn make_bool(&self, value: bool) -> Self::Value {
        self.make_constant::<Bag::Constant>(BorrowedConstant::Boolean { value })
    }

    fn make_none(&self) -> Self::Value {
        self.make_constant::<Bag::Constant>(BorrowedConstant::None)
    }

    fn make_ellipsis(&self) -> Self::Value {
        self.make_constant::<Bag::Constant>(BorrowedConstant::Ellipsis)
    }

    fn make_float(&self, value: f64) -> Self::Value {
        self.make_constant::<Bag::Constant>(BorrowedConstant::Float { value })
    }

    fn make_complex(&self, value: Complex64) -> Self::Value {
        self.make_constant::<Bag::Constant>(BorrowedConstant::Complex { value })
    }

    fn make_str(&self, value: &Wtf8) -> Self::Value {
        self.make_constant::<Bag::Constant>(BorrowedConstant::Str { value })
    }

    fn make_bytes(&self, value: &[u8]) -> Self::Value {
        self.make_constant::<Bag::Constant>(BorrowedConstant::Bytes { value })
    }

    fn make_int(&self, value: BigInt) -> Self::Value {
        self.make_int(value)
    }

    fn make_tuple(&self, elements: impl Iterator<Item = Self::Value>) -> Self::Value {
        self.make_tuple(elements)
    }

    fn make_slice(
        &self,
        start: Self::Value,
        stop: Self::Value,
        step: Self::Value,
    ) -> Result<Self::Value> {
        let elements = [start, stop, step];
        Ok(
            self.make_constant::<Bag::Constant>(BorrowedConstant::Slice {
                elements: &elements,
            }),
        )
    }

    fn make_code(
        &self,
        code: CodeObject<<Self::ConstantBag as ConstantBag>::Constant>,
    ) -> Result<Self::Value> {
        Ok(self.make_code(code))
    }

    fn make_stop_iter(&self) -> Result<Self::Value> {
        Err(MarshalError::BadType)
    }

    fn make_list(&self, _: impl Iterator<Item = Self::Value>) -> Result<Self::Value> {
        Err(MarshalError::BadType)
    }

    fn make_set(&self, _: impl Iterator<Item = Self::Value>) -> Result<Self::Value> {
        Err(MarshalError::BadType)
    }

    fn make_frozenset(&self, it: impl Iterator<Item = Self::Value>) -> Result<Self::Value> {
        let elements: Vec<Self::Value> = it.collect();
        Ok(
            self.make_constant::<Bag::Constant>(BorrowedConstant::Frozenset {
                elements: &elements,
            }),
        )
    }

    fn make_dict(
        &self,
        _: impl Iterator<Item = (Self::Value, Self::Value)>,
    ) -> Result<Self::Value> {
        Err(MarshalError::BadType)
    }

    fn constant_bag(self) -> Self::ConstantBag {
        self
    }

    fn constant_ref_from_value(
        &self,
        value: &Self::Value,
    ) -> Option<<Self::ConstantBag as ConstantBag>::Constant> {
        Some(value.clone())
    }

    fn bytes_from_value(&self, value: &Self::Value) -> Option<Vec<u8>> {
        match value.borrow_constant() {
            BorrowedConstant::Bytes { value } => Some(value.to_vec()),
            _ => None,
        }
    }

    fn str_from_value(&self, value: &Self::Value) -> Option<alloc::string::String> {
        match value.borrow_constant() {
            BorrowedConstant::Str { value } => Some(value.to_string_lossy().into_owned()),
            _ => None,
        }
    }

    fn tuple_elements_from_value(&self, value: &Self::Value) -> Option<Vec<Self::Value>> {
        match value.borrow_constant() {
            BorrowedConstant::Tuple { elements } => Some(elements.to_vec()),
            _ => None,
        }
    }
}

pub const MAX_MARSHAL_STACK_DEPTH: usize = 2000;

pub fn deserialize_value<R: Read, Bag: MarshalBag>(rdr: &mut R, bag: Bag) -> Result<Bag::Value> {
    let mut refs: Vec<Option<Bag::Value>> = Vec::new();
    deserialize_value_depth(rdr, bag, MAX_MARSHAL_STACK_DEPTH, &mut refs)
}

fn deserialize_value_depth<R: Read, Bag: MarshalBag>(
    rdr: &mut R,
    bag: Bag,
    depth: usize,
    refs: &mut Vec<Option<Bag::Value>>,
) -> Result<Bag::Value> {
    // CPython's r_object() reads the type byte before checking the depth limit
    let raw = rdr.read_type_byte()?;
    deserialize_value_after_header(rdr, bag, depth, refs, raw)
}

/// Continue deserializing a value after the header byte has already been
/// consumed. Shared by `deserialize_value_depth` and the dict-key branch,
/// where the header byte is read up front to detect the TYPE_NULL
/// terminator.
fn deserialize_value_after_header<R: Read, Bag: MarshalBag>(
    rdr: &mut R,
    bag: Bag,
    depth: usize,
    refs: &mut Vec<Option<Bag::Value>>,
    raw: u8,
) -> Result<Bag::Value> {
    if depth == 0 {
        return Err(MarshalError::RecursionLimitExceeded);
    }
    let flag = raw & FLAG_REF != 0;
    let type_code = raw & !FLAG_REF;

    // TYPE_REF: return previously stored object
    if type_code == Type::Ref as u8 {
        let idx = rdr.read_u32()? as usize;
        return resolve_ref(idx, refs);
    }

    // Reserve ref slot before reading (matches write order)
    let slot = reserve_ref_slot(flag, refs)?;

    let typ = Type::try_from(type_code)?;
    let value = if matches!(typ, Type::Code) {
        deserialize_code_value_inner(rdr, bag, depth - 1, refs)?
    } else {
        deserialize_value_typed(rdr, bag, depth, refs, typ, slot)?
    };

    if let Some(idx) = slot {
        refs[idx] = Some(value.clone());
    }
    Ok(value)
}

/// Decode a code object through the runtime bag.  CPython's marshal reader
/// keeps one reference table for the code fields and `co_consts`; using
/// `Bag::Value` here preserves that index space and lets runtime-only
/// constants survive alongside the compiler representation.
fn deserialize_code_value_inner<R: Read, Bag: MarshalBag>(
    rdr: &mut R,
    bag: Bag,
    depth: usize,
    refs: &mut Vec<Option<Bag::Value>>,
) -> Result<Bag::Value> {
    if depth == 0 {
        return Err(MarshalError::InvalidBytecode);
    }
    let arg_count = rdr.read_u32()?;
    let posonlyarg_count = rdr.read_u32()?;
    let kwonlyarg_count = rdr.read_u32()?;
    let max_stackdepth = rdr.read_u32()?;
    let flags = CodeFlags::from_bits_truncate(rdr.read_u32()?);
    let child_depth = depth - 1;

    let code_value = deserialize_value_depth(rdr, bag, child_depth, refs)?;
    let code_bytes = bag
        .bytes_from_value(&code_value)
        .ok_or(MarshalError::BadType)?;

    let consts_value = deserialize_value_depth(rdr, bag, child_depth, refs)?;
    let constant_values = bag
        .tuple_elements_from_value(&consts_value)
        .ok_or(MarshalError::BadType)?;
    let constants = constant_values
        .iter()
        .map(|value| bag.code_constant_from_value(value))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .collect();

    let read_strings =
        |rdr: &mut R, refs: &mut Vec<Option<Bag::Value>>| -> Result<Vec<alloc::string::String>> {
            let tuple = deserialize_value_depth(rdr, bag, child_depth, refs)?;
            bag.tuple_elements_from_value(&tuple)
                .ok_or(MarshalError::BadType)?
                .iter()
                .map(|value| bag.str_from_value(value).ok_or(MarshalError::BadType))
                .collect()
        };
    let names_raw = read_strings(rdr, refs)?;
    let localsplusnames = read_strings(rdr, refs)?;

    let kinds_value = deserialize_value_depth(rdr, bag, child_depth, refs)?;
    let localspluskinds = bag
        .bytes_from_value(&kinds_value)
        .ok_or(MarshalError::BadType)?;

    let read_string =
        |rdr: &mut R, refs: &mut Vec<Option<Bag::Value>>| -> Result<alloc::string::String> {
            let value = deserialize_value_depth(rdr, bag, child_depth, refs)?;
            bag.str_from_value(&value).ok_or(MarshalError::BadType)
        };
    let source_path_raw = read_string(rdr, refs)?;
    let obj_name_raw = read_string(rdr, refs)?;
    let qualname_raw = read_string(rdr, refs)?;

    let first_line_raw = rdr.read_u32()? as i32;
    let first_line_number = if first_line_raw > 0 {
        OneIndexed::new(first_line_raw as usize)
    } else {
        None
    };
    let linetable_value = deserialize_value_depth(rdr, bag, child_depth, refs)?;
    let linetable = bag
        .bytes_from_value(&linetable_value)
        .ok_or(MarshalError::BadType)?
        .into_boxed_slice();
    let exceptiontable_value = deserialize_value_depth(rdr, bag, child_depth, refs)?;
    let exceptiontable = bag
        .bytes_from_value(&exceptiontable_value)
        .ok_or(MarshalError::BadType)?
        .into_boxed_slice();

    let lp = split_localplus(
        &localsplusnames
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>(),
        &localspluskinds,
        arg_count,
        kwonlyarg_count,
        flags,
    )?;
    let instructions = bag.code_units_from_bytes(&code_bytes)?;
    let locations = linetable_to_locations(&linetable, first_line_raw, instructions.len());
    let constant_bag = bag.constant_bag();
    let code = CodeObject {
        instructions,
        locations,
        flags,
        posonlyarg_count,
        arg_count,
        kwonlyarg_count,
        source_path: constant_bag.make_name(&source_path_raw),
        first_line_number,
        max_stackdepth,
        obj_name: constant_bag.make_name(&obj_name_raw),
        qualname: constant_bag.make_name(&qualname_raw),
        constants,
        names: names_raw
            .iter()
            .map(|name| constant_bag.make_name(name))
            .collect(),
        varnames: lp
            .varnames
            .iter()
            .map(|name| constant_bag.make_name(name))
            .collect(),
        cellvars: lp
            .cellvars
            .iter()
            .map(|name| constant_bag.make_name(name))
            .collect(),
        freevars: lp
            .freevars
            .iter()
            .map(|name| constant_bag.make_name(name))
            .collect(),
        localspluskinds: localspluskinds.into_boxed_slice(),
        linetable,
        exceptiontable,
    };
    bag.make_code_with_constants_and_bytes(code, constant_values, code_bytes)
}

fn deserialize_value_typed<R: Read, Bag: MarshalBag>(
    rdr: &mut R,
    bag: Bag,
    depth: usize,
    refs: &mut Vec<Option<Bag::Value>>,
    typ: Type,
    slot: Option<usize>,
) -> Result<Bag::Value> {
    if depth == 0 {
        return Err(MarshalError::RecursionLimitExceeded);
    }
    let value = match typ {
        Type::True => bag.make_bool(true),
        Type::False => bag.make_bool(false),
        Type::None => bag.make_none(),
        Type::StopIter => bag.make_stop_iter()?,
        Type::Ellipsis => bag.make_ellipsis(),
        Type::Int => {
            let val = rdr.read_u32()? as i32;
            bag.make_int(BigInt::from(val))
        }
        Type::Int64 => {
            let lo = rdr.read_u32()? as u64;
            let hi = rdr.read_u32()? as u64;
            bag.make_int(BigInt::from(((hi << 32) | lo) as i64))
        }
        Type::Long => bag.make_int(read_pylong(rdr)?),
        Type::FloatStr => bag.make_float(read_float_str(rdr)?),
        Type::Float => {
            let value = f64::from_bits(rdr.read_u64()?);
            bag.make_float(value)
        }
        Type::ComplexStr => {
            let re = read_float_str(rdr)?;
            let im = read_float_str(rdr)?;
            bag.make_complex(Complex64 { re, im })
        }
        Type::Complex => {
            let re = f64::from_bits(rdr.read_u64()?);
            let im = f64::from_bits(rdr.read_u64()?);
            let value = Complex64 { re, im };
            bag.make_complex(value)
        }
        Type::Ascii | Type::Unicode => {
            let len = rdr.read_len("string")?;
            let value = rdr.read_wtf8(len as u32)?;
            bag.make_str(value)
        }
        Type::AsciiInterned | Type::Interned => {
            let len = rdr.read_len("string")?;
            let value = rdr.read_wtf8(len as u32)?;
            bag.make_interned_str(value)
        }
        Type::ShortAscii => {
            let len = rdr.read_u8().map_err(eof_at_byte)? as u32;
            let value = rdr.read_wtf8(len)?;
            bag.make_str(value)
        }
        Type::ShortAsciiInterned => {
            let len = rdr.read_u8().map_err(eof_at_byte)? as u32;
            let value = rdr.read_wtf8(len)?;
            bag.make_interned_str(value)
        }
        Type::SmallTuple => {
            let len = rdr.read_u8().map_err(eof_at_byte)? as usize;
            let d = depth - 1;
            if let Some(index) = slot
                && let Some(tuple) = bag.make_tuple_placeholder(len)?
            {
                refs[index] = Some(tuple.clone());
                for item_index in 0..len {
                    let item = deserialize_value_depth(rdr, bag, d, refs)
                        .map_err(|e| e.null_in(MarshalError::NullInTuple))?;
                    bag.set_tuple_item(&tuple, item_index, item)?;
                }
                tuple
            } else {
                let it = (0..len).map(|_| {
                    deserialize_value_depth(rdr, bag, d, refs)
                        .map_err(|e| e.null_in(MarshalError::NullInTuple))
                });
                itertools::process_results(it, |it| bag.make_tuple(it))?
            }
        }
        Type::Null => {
            return Err(MarshalError::NullObject);
        }
        Type::Ref => {
            // Handled in deserialize_value_depth before calling this function
            return Err(MarshalError::BadType);
        }
        Type::Tuple => {
            let len = rdr.read_len("tuple")?;
            let d = depth - 1;
            if let Some(index) = slot
                && let Some(tuple) = bag.make_tuple_placeholder(len)?
            {
                refs[index] = Some(tuple.clone());
                for item_index in 0..len {
                    let item = deserialize_value_depth(rdr, bag, d, refs)
                        .map_err(|e| e.null_in(MarshalError::NullInTuple))?;
                    bag.set_tuple_item(&tuple, item_index, item)?;
                }
                tuple
            } else {
                let it = (0..len).map(|_| {
                    deserialize_value_depth(rdr, bag, d, refs)
                        .map_err(|e| e.null_in(MarshalError::NullInTuple))
                });
                itertools::process_results(it, |it| bag.make_tuple(it))?
            }
        }
        Type::List => {
            let len = rdr.read_len("list")?;
            let d = depth - 1;
            if let Some(index) = slot
                && let Some(list) = bag.make_list_placeholder(len)?
            {
                refs[index] = Some(list.clone());
                for item_index in 0..len {
                    let item = deserialize_value_depth(rdr, bag, d, refs)
                        .map_err(|e| e.null_in(MarshalError::NullInList))?;
                    bag.set_list_item(&list, item_index, item)?;
                }
                list
            } else {
                let it = (0..len).map(|_| {
                    deserialize_value_depth(rdr, bag, d, refs)
                        .map_err(|e| e.null_in(MarshalError::NullInList))
                });
                itertools::process_results(it, |it| bag.make_list(it))??
            }
        }
        Type::Set => {
            let len = rdr.read_len("set")?;
            let d = depth - 1;
            if let Some(index) = slot
                && let Some(set) = bag.make_set_placeholder()
            {
                refs[index] = Some(set.clone());
                for _ in 0..len {
                    let item = deserialize_value_depth(rdr, bag, d, refs)
                        .map_err(|e| e.null_in(MarshalError::NullInSet))?;
                    bag.insert_set_item(&set, item)?;
                }
                set
            } else {
                let it = (0..len).map(|_| {
                    deserialize_value_depth(rdr, bag, d, refs)
                        .map_err(|e| e.null_in(MarshalError::NullInSet))
                });
                itertools::process_results(it, |it| bag.make_set(it))??
            }
        }
        Type::FrozenSet => {
            let len = rdr.read_len("set")?;
            let d = depth - 1;
            let it = (0..len).map(|_| {
                deserialize_value_depth(rdr, bag, d, refs)
                    .map_err(|e| e.null_in(MarshalError::NullInSet))
            });
            itertools::process_results(it, |it| bag.make_frozenset(it))??
        }
        Type::Dict => {
            let d = depth - 1;
            if let Some(index) = slot
                && let Some(dict) = bag.make_dict_placeholder()
            {
                refs[index] = Some(dict.clone());
                loop {
                    let raw = rdr.read_type_byte()?;
                    if raw & !FLAG_REF == b'0' {
                        break;
                    }
                    let key = deserialize_value_after_header(rdr, bag, d, refs, raw)?;
                    // CPython drops the pending pair and ends the dict on a NULL value
                    let value = match deserialize_value_depth(rdr, bag, d, refs) {
                        Err(MarshalError::NullObject) => break,
                        res => res?,
                    };
                    bag.insert_dict_item(&dict, key, value)?;
                }
                dict
            } else {
                let mut pairs = Vec::new();
                loop {
                    let raw = rdr.read_type_byte()?;
                    if raw & !FLAG_REF == b'0' {
                        break;
                    }
                    let key = deserialize_value_after_header(rdr, bag, d, refs, raw)?;
                    // CPython drops the pending pair and ends the dict on a NULL value
                    let value = match deserialize_value_depth(rdr, bag, d, refs) {
                        Err(MarshalError::NullObject) => break,
                        res => res?,
                    };
                    pairs.push((key, value));
                }
                bag.make_dict(pairs.into_iter())?
            }
        }
        Type::Bytes => {
            // After marshaling, byte arrays are converted into bytes.
            let len = rdr.read_len("bytes object")?;
            let value = rdr.read_slice(len as u32)?;
            bag.make_bytes(value)
        }
        Type::Code => return Err(MarshalError::BadType),
        Type::Slice => {
            let d = depth - 1;
            let start = deserialize_value_depth(rdr, bag, d, refs)?;
            let stop = deserialize_value_depth(rdr, bag, d, refs)?;
            let step = deserialize_value_depth(rdr, bag, d, refs)?;
            bag.make_slice(start, stop, step)?
        }
    };
    Ok(value)
}

pub trait Dumpable: Sized {
    type Error;
    type Constant: Constant;

    fn with_dump<R>(&self, f: impl FnOnce(DumpableValue<'_, Self>) -> R) -> Result<R, Self::Error>;
}

pub enum DumpableValue<'a, D: Dumpable> {
    Integer(&'a BigInt),
    Float(f64),
    Complex(Complex64),
    Boolean(bool),
    Str(&'a Wtf8),
    Bytes(&'a [u8]),
    Code(&'a CodeObject<D::Constant>),
    Tuple(&'a [D]),
    None,
    Ellipsis,
    StopIter,
    List(&'a [D]),
    Set(&'a [D]),
    Frozenset(&'a [D]),
    Dict(&'a [(D, D)]),
    Slice(&'a D, &'a D, &'a D),
}

impl<'a, C: Constant> From<BorrowedConstant<'a, C>> for DumpableValue<'a, C> {
    fn from(c: BorrowedConstant<'a, C>) -> Self {
        match c {
            BorrowedConstant::Integer { value } => Self::Integer(value),
            BorrowedConstant::Float { value } => Self::Float(value),
            BorrowedConstant::Complex { value } => Self::Complex(value),
            BorrowedConstant::Boolean { value } => Self::Boolean(value),
            BorrowedConstant::Str { value } => Self::Str(value),
            BorrowedConstant::Bytes { value } => Self::Bytes(value),
            BorrowedConstant::Code { code } => Self::Code(code),
            BorrowedConstant::Tuple { elements } => Self::Tuple(elements),
            BorrowedConstant::Slice { elements } => {
                Self::Slice(&elements[0], &elements[1], &elements[2])
            }
            BorrowedConstant::Frozenset { elements } => Self::Frozenset(elements),
            BorrowedConstant::None => Self::None,
            BorrowedConstant::Ellipsis => Self::Ellipsis,
        }
    }
}

impl<C: Constant> Dumpable for C {
    type Error = Infallible;
    type Constant = Self;

    #[inline(always)]
    fn with_dump<R>(&self, f: impl FnOnce(DumpableValue<'_, Self>) -> R) -> Result<R, Self::Error> {
        Ok(f(self.borrow_constant().into()))
    }
}

pub trait Write {
    fn write_slice(&mut self, slice: &[u8]);

    fn write_u8(&mut self, v: u8) {
        self.write_slice(&v.to_le_bytes())
    }

    fn write_u16(&mut self, v: u16) {
        self.write_slice(&v.to_le_bytes())
    }

    fn write_u32(&mut self, v: u32) {
        self.write_slice(&v.to_le_bytes())
    }

    fn write_u64(&mut self, v: u64) {
        self.write_slice(&v.to_le_bytes())
    }
}

impl Write for Vec<u8> {
    fn write_slice(&mut self, slice: &[u8]) {
        self.extend_from_slice(slice)
    }
}

pub(crate) fn write_len<W: Write>(buf: &mut W, len: usize) {
    let Ok(len) = len.try_into() else {
        panic!("too long to serialize")
    };
    buf.write_u32(len);
}

pub(crate) fn write_vec<W: Write>(buf: &mut W, slice: &[u8]) {
    write_len(buf, slice.len());
    buf.write_slice(slice);
}

pub fn serialize_value<W: Write, D: Dumpable>(
    buf: &mut W,
    constant: DumpableValue<'_, D>,
) -> Result<(), D::Error> {
    match constant {
        DumpableValue::Integer(int) => {
            if let Ok(val) = i32::try_from(int) {
                buf.write_u8(Type::Int as u8); // TYPE_INT: 4-byte LE i32
                buf.write_u32(val as u32);
            } else {
                buf.write_u8(Type::Long as u8);
                let (sign, raw) = int.to_bytes_le();
                let mut digits = alloc::vec::Vec::new();
                let mut accum: u32 = 0;
                let mut bits = 0u32;
                for &byte in &raw {
                    accum |= (byte as u32) << bits;
                    bits += 8;
                    while bits >= 15 {
                        digits.push((accum & 0x7fff) as u16);
                        accum >>= 15;
                        bits -= 15;
                    }
                }
                if accum > 0 || digits.is_empty() {
                    digits.push(accum as u16);
                }
                while digits.len() > 1 && *digits.last().unwrap() == 0 {
                    digits.pop();
                }
                let n = digits.len() as i32;
                let n = if sign == Sign::Minus { -n } else { n };
                buf.write_u32(n as u32);
                for d in &digits {
                    buf.write_u16(*d);
                }
            }
        }
        DumpableValue::Float(f) => {
            buf.write_u8(Type::Float as u8);
            buf.write_u64(f.to_bits());
        }
        DumpableValue::Complex(c) => {
            buf.write_u8(Type::Complex as u8);
            buf.write_u64(c.re.to_bits());
            buf.write_u64(c.im.to_bits());
        }
        DumpableValue::Boolean(b) => {
            buf.write_u8(if b { Type::True } else { Type::False } as u8);
        }
        DumpableValue::Str(s) => {
            buf.write_u8(Type::Unicode as u8);
            write_vec(buf, s.as_bytes());
        }
        DumpableValue::Bytes(b) => {
            buf.write_u8(Type::Bytes as u8);
            write_vec(buf, b);
        }
        DumpableValue::Code(c) => {
            buf.write_u8(Type::Code as u8);
            serialize_code(buf, c);
        }
        DumpableValue::Tuple(tup) => {
            buf.write_u8(Type::Tuple as u8);
            write_len(buf, tup.len());
            for val in tup {
                val.with_dump(|val| serialize_value(buf, val))??
            }
        }
        DumpableValue::None => {
            buf.write_u8(Type::None as u8);
        }
        DumpableValue::Ellipsis => {
            buf.write_u8(Type::Ellipsis as u8);
        }
        DumpableValue::StopIter => {
            buf.write_u8(Type::StopIter as u8);
        }
        DumpableValue::List(l) => {
            buf.write_u8(Type::List as u8);
            write_len(buf, l.len());
            for val in l {
                val.with_dump(|val| serialize_value(buf, val))??
            }
        }
        DumpableValue::Set(set) => {
            buf.write_u8(Type::Set as u8);
            write_len(buf, set.len());
            for val in set {
                val.with_dump(|val| serialize_value(buf, val))??
            }
        }
        DumpableValue::Frozenset(set) => {
            buf.write_u8(Type::FrozenSet as u8);
            write_len(buf, set.len());
            for val in set {
                val.with_dump(|val| serialize_value(buf, val))??
            }
        }
        DumpableValue::Dict(d) => {
            buf.write_u8(Type::Dict as u8);
            for (k, v) in d {
                k.with_dump(|val| serialize_value(buf, val))??;
                v.with_dump(|val| serialize_value(buf, val))??;
            }
            buf.write_u8(b'0'); // TYPE_NULL
        }
        DumpableValue::Slice(start, stop, step) => {
            buf.write_u8(Type::Slice as u8);
            start.with_dump(|val| serialize_value(buf, val))??;
            stop.with_dump(|val| serialize_value(buf, val))??;
            step.with_dump(|val| serialize_value(buf, val))??;
        }
    }
    Ok(())
}

/// Serialize a code object in CPython field order.
///
/// Split varnames/cellvars/freevars are reassembled into
/// co_localsplusnames/co_localspluskinds.
pub fn serialize_code<W: Write, C: Constant>(buf: &mut W, code: &CodeObject<C>) {
    serialize_code_with(buf, code, |buf, constant| {
        serialize_value(buf, constant.borrow_constant().into()).unwrap_or_else(|x| match x {});
        Ok::<(), core::convert::Infallible>(())
    })
    .unwrap_or_else(|x| match x {})
}

/// Serialize a code object, writing each `co_consts` entry through
/// `write_constant`.
///
/// A runtime caller passes its own object writer so that values its constant
/// representation carries but `BorrowedConstant` cannot describe — lists,
/// dicts, sets — reach the stream, and so a constant shared with the enclosing
/// object keeps its entry in that writer's reference table.
pub fn serialize_code_with<W: Write, C: Constant, E>(
    buf: &mut W,
    code: &CodeObject<C>,
    mut write_constant: impl FnMut(&mut W, &C) -> core::result::Result<(), E>,
) -> core::result::Result<(), E> {
    // 1–5: scalar fields
    buf.write_u32(code.arg_count);
    buf.write_u32(code.posonlyarg_count);
    buf.write_u32(code.kwonlyarg_count);
    buf.write_u32(code.max_stackdepth);
    buf.write_u32(code.flags.bits());

    // 6: co_code (TYPE_STRING) — bytecode already uses flat localsplus indices
    let bytecode = code.instructions.original_bytes();
    buf.write_u8(Type::Bytes as u8);
    write_vec(buf, &bytecode);

    // 7: co_consts (TYPE_TUPLE)
    buf.write_u8(Type::Tuple as u8);
    write_len(buf, code.constants.len());
    for constant in &*code.constants {
        write_constant(buf, constant)?;
    }

    // 8: co_names (tuple of strings)
    write_marshal_name_tuple(buf, &code.names);

    // 9: co_localsplusnames — varnames ++ cell_only ++ freevars
    let cell_only_names: Vec<&str> = code
        .cellvars
        .iter()
        .filter(|cv| !code.varnames.iter().any(|v| v.as_ref() == cv.as_ref()))
        .map(|cv| cv.as_ref())
        .collect();
    let total_lp_count = code.varnames.len() + cell_only_names.len() + code.freevars.len();
    buf.write_u8(Type::Tuple as u8);
    write_len(buf, total_lp_count);
    for n in &code.varnames {
        write_marshal_str(buf, n.as_ref());
    }
    for &n in &cell_only_names {
        write_marshal_str(buf, n);
    }
    for n in &code.freevars {
        write_marshal_str(buf, n.as_ref());
    }
    // 10: co_localspluskinds — use the stored kinds directly
    buf.write_u8(Type::Bytes as u8);
    write_vec(buf, &code.localspluskinds);

    // 11: co_filename
    write_marshal_str(buf, code.source_path.as_ref());
    // 12: co_name
    write_marshal_str(buf, code.obj_name.as_ref());
    // 13: co_qualname
    write_marshal_str(buf, code.qualname.as_ref());
    // 14: co_firstlineno
    buf.write_u32(code.first_line_number.map_or(0, |x| x.get() as _));
    // 15: co_linetable
    buf.write_u8(Type::Bytes as u8);
    write_vec(buf, &code.linetable);
    // 16: co_exceptiontable
    buf.write_u8(Type::Bytes as u8);
    write_vec(buf, &code.exceptiontable);
    Ok(())
}

fn write_marshal_str<W: Write>(buf: &mut W, s: &str) {
    let bytes = s.as_bytes();
    if bytes.len() < 256 && bytes.is_ascii() {
        buf.write_u8(b'z'); // TYPE_SHORT_ASCII
        buf.write_u8(bytes.len() as u8);
    } else {
        buf.write_u8(Type::Unicode as u8);
        write_len(buf, bytes.len());
    }
    buf.write_slice(bytes);
}

fn write_marshal_name_tuple<W: Write, N: AsRef<str>>(buf: &mut W, names: &[N]) {
    buf.write_u8(Type::Tuple as u8);
    write_len(buf, names.len());
    for name in names {
        write_marshal_str(buf, name.as_ref());
    }
}

pub const FLAG_REF: u8 = 0x80;

/// Read a signed 32-bit LE integer.
pub fn read_i32<R: Read>(rdr: &mut R) -> Result<i32> {
    let bytes = rdr.read_array::<4>()?;
    Ok(i32::from_le_bytes(*bytes))
}

/// Read a TYPE_LONG arbitrary-precision integer (base-2^15 digits).
pub fn read_pylong<R: Read>(rdr: &mut R) -> Result<BigInt> {
    const MARSHAL_SHIFT: u32 = 15;
    const MARSHAL_BASE: u32 = 1 << MARSHAL_SHIFT;
    let n = read_i32(rdr)?;
    // CPython: n < -SIZE32_MAX || n > SIZE32_MAX (only i32::MIN qualifies)
    if n == i32::MIN {
        return Err(MarshalError::BadData("long size out of range"));
    }
    if n == 0 {
        return Ok(BigInt::from(0));
    }
    let negative = n < 0;
    let num_digits = n.unsigned_abs() as usize;
    let mut accum = BigInt::from(0);
    let mut last_digit = 0u32;
    for i in 0..num_digits {
        let d = rdr.read_u16()? as u32;
        if d >= MARSHAL_BASE {
            return Err(MarshalError::BadData("digit out of range in long"));
        }
        last_digit = d;
        accum += BigInt::from(d) << (i as u32 * MARSHAL_SHIFT);
    }
    if num_digits > 0 && last_digit == 0 {
        return Err(MarshalError::BadData("unnormalized long data"));
    }
    if negative {
        accum = -accum;
    }
    Ok(accum)
}

/// Read a text-encoded float (1-byte length + ASCII).
pub fn read_float_str<R: Read>(rdr: &mut R) -> Result<f64> {
    let n = rdr.read_u8().map_err(eof_at_byte)? as u32;
    let s = rdr.read_str(n)?;
    s.parse::<f64>().map_err(|_| MarshalError::InvalidBytecode)
}

/// Read a 4-byte-length-prefixed byte string.
pub fn read_pstring<R: Read>(rdr: &mut R) -> Result<&[u8]> {
    let n = read_i32(rdr)?;
    if n < 0 {
        return Err(MarshalError::InvalidBytecode);
    }
    rdr.read_slice(n as u32)
}

const CO_FAST_LOCAL: u8 = 0x20;
const CO_FAST_CELL: u8 = 0x40;
const CO_FAST_FREE: u8 = 0x80;

pub struct LocalsPlusResult<S> {
    pub varnames: Vec<S>,
    pub cellvars: Vec<S>,
    pub freevars: Vec<S>,
    pub cell2arg: Option<Box<[i32]>>,
    pub deref_map: Vec<u32>,
}

pub fn split_localplus<S: Clone>(
    names: &[S],
    kinds: &[u8],
    arg_count: u32,
    kwonlyarg_count: u32,
    flags: CodeFlags,
) -> Result<LocalsPlusResult<S>> {
    if names.len() != kinds.len() {
        return Err(MarshalError::InvalidBytecode);
    }

    let mut varnames = Vec::new();
    let mut cellvars = Vec::new();
    let mut freevars = Vec::new();

    // First pass: collect varnames (LOCAL entries) and freevars
    for (name, &kind) in names.iter().zip(kinds.iter()) {
        if kind & CO_FAST_LOCAL != 0 {
            varnames.push(name.clone());
        }
        if kind & CO_FAST_FREE != 0 {
            freevars.push(name.clone());
        }
    }

    // Second pass: collect cellvars in localsplusnames order.
    // CELL-only vars come from non-LOCAL CELL entries.
    // LOCAL|CELL vars are also added to cellvars.
    // This preserves the original ordering from localsplusnames.
    let mut arg_cell_positions = Vec::new(); // (cell_idx, localplus_idx)
    for (i, (name, &kind)) in names.iter().zip(kinds.iter()).enumerate() {
        let is_local = kind & CO_FAST_LOCAL != 0;
        let is_cell = kind & CO_FAST_CELL != 0;
        if is_cell {
            let cell_idx = cellvars.len();
            cellvars.push(name.clone());
            if is_local {
                arg_cell_positions.push((cell_idx, i));
            }
        }
    }

    let total_args = {
        let mut t = arg_count + kwonlyarg_count;
        if flags.contains(CodeFlags::VARARGS) {
            t += 1;
        }
        if flags.contains(CodeFlags::VARKEYWORDS) {
            t += 1;
        }
        t
    };

    let cell2arg = if !cellvars.is_empty() {
        let mut mapping = alloc::vec![-1i32; cellvars.len()];
        for &(cell_idx, localplus_idx) in &arg_cell_positions {
            if (localplus_idx as u32) < total_args {
                mapping[cell_idx] = localplus_idx as i32;
            }
        }
        if mapping.iter().any(|&x| x >= 0) {
            Some(mapping.into_boxed_slice())
        } else {
            None
        }
    } else {
        None
    };

    // Build deref_map: localsplusnames index → cellvar/freevar index
    let mut deref_map = alloc::vec![u32::MAX; names.len()];
    let mut cell_idx = 0u32;
    for (i, &kind) in kinds.iter().enumerate() {
        if kind & CO_FAST_CELL != 0 {
            deref_map[i] = cell_idx;
            cell_idx += 1;
        }
    }
    let ncells = cellvars.len();
    let mut free_idx = 0u32;
    for (i, &kind) in kinds.iter().enumerate() {
        if kind & CO_FAST_FREE != 0 {
            deref_map[i] = ncells as u32 + free_idx;
            free_idx += 1;
        }
    }

    Ok(LocalsPlusResult {
        varnames,
        cellvars,
        freevars,
        cell2arg,
        deref_map,
    })
}

#[must_use]
pub fn linetable_to_locations(
    linetable: &[u8],
    first_line: i32,
    num_instructions: usize,
) -> Box<[(SourceLocation, SourceLocation)]> {
    let default_loc = || {
        let line = if first_line > 0 {
            OneIndexed::new(first_line as usize).unwrap_or(OneIndexed::MIN)
        } else {
            OneIndexed::MIN
        };
        let loc = SourceLocation {
            line,
            character_offset: OneIndexed::from_zero_indexed(0),
        };
        (loc, loc)
    };
    if linetable.is_empty() {
        return alloc::vec![default_loc(); num_instructions].into_boxed_slice();
    }

    let mut locations = Vec::with_capacity(num_instructions);
    let mut pos = 0;
    let mut line = first_line;

    while pos < linetable.len() && locations.len() < num_instructions {
        let first_byte = linetable[pos];
        pos += 1;
        if first_byte & 0x80 == 0 {
            break;
        }
        let code = (first_byte >> 3) & 0x0f;
        let length = ((first_byte & 0x07) + 1) as usize;
        let kind = match PyCodeLocationInfoKind::from_code(code) {
            Some(k) => k,
            None => break,
        };

        let (line_delta, end_line_delta, col, end_col): (i32, i32, Option<u32>, Option<u32>) =
            match kind {
                PyCodeLocationInfoKind::None => (0, 0, None, None),
                PyCodeLocationInfoKind::Long => {
                    let d = lt_read_signed_varint(linetable, &mut pos);
                    let ed = lt_read_varint(linetable, &mut pos) as i32;
                    let c = lt_read_varint(linetable, &mut pos);
                    let ec = lt_read_varint(linetable, &mut pos);
                    (
                        d,
                        ed,
                        if c == 0 { None } else { Some(c - 1) },
                        if ec == 0 { None } else { Some(ec - 1) },
                    )
                }
                PyCodeLocationInfoKind::NoColumns => {
                    (lt_read_signed_varint(linetable, &mut pos), 0, None, None)
                }
                PyCodeLocationInfoKind::OneLine0
                | PyCodeLocationInfoKind::OneLine1
                | PyCodeLocationInfoKind::OneLine2 => {
                    let c = lt_byte(linetable, &mut pos) as u32;
                    let ec = lt_byte(linetable, &mut pos) as u32;
                    (kind.one_line_delta().unwrap_or(0), 0, Some(c), Some(ec))
                }
                _ if kind.is_short() => {
                    let d = lt_byte(linetable, &mut pos);
                    let g = kind.short_column_group().unwrap_or(0);
                    let c = ((g as u32) << 3) | ((d >> 4) as u32);
                    (0, 0, Some(c), Some(c + (d & 0x0f) as u32))
                }
                _ => (0, 0, None, None),
            };

        line += line_delta;
        let mk = |l: i32| {
            if l > 0 {
                OneIndexed::new(l as usize).unwrap_or(OneIndexed::MIN)
            } else {
                OneIndexed::MIN
            }
        };
        for _ in 0..length {
            if locations.len() >= num_instructions {
                break;
            }
            if kind == PyCodeLocationInfoKind::None {
                let loc = SourceLocation {
                    line: mk(line),
                    character_offset: OneIndexed::from_zero_indexed(0),
                };
                locations.push((loc, loc));
            } else {
                locations.push((
                    SourceLocation {
                        line: mk(line),
                        character_offset: OneIndexed::from_zero_indexed(col.unwrap_or(0) as usize),
                    },
                    SourceLocation {
                        line: mk(line + end_line_delta),
                        character_offset: OneIndexed::from_zero_indexed(
                            end_col.unwrap_or(0) as usize
                        ),
                    },
                ));
            }
        }
    }
    while locations.len() < num_instructions {
        locations.push(default_loc());
    }
    locations.into_boxed_slice()
}

fn lt_byte(data: &[u8], pos: &mut usize) -> u8 {
    if *pos < data.len() {
        let b = data[*pos];
        *pos += 1;
        b
    } else {
        0
    }
}

/// Linetable uses little-endian varint.
fn lt_read_varint(data: &[u8], pos: &mut usize) -> u32 {
    let mut result: u32 = 0;
    let mut shift = 0;
    loop {
        if *pos >= data.len() {
            break;
        }
        let b = data[*pos];
        *pos += 1;
        result |= ((b & 0x3f) as u32) << shift;
        shift += 6;
        if b & 0x40 == 0 {
            break;
        }
    }
    result
}

fn lt_read_signed_varint(data: &[u8], pos: &mut usize) -> i32 {
    let val = lt_read_varint(data, pos);
    if val & 1 != 0 {
        -((val >> 1) as i32)
    } else {
        (val >> 1) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{BasicBag, ConstantData};

    fn hex_to_bytes(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect()
    }

    fn decode_code(hex: &str) -> CodeObject<ConstantData> {
        let bytes = hex_to_bytes(hex);
        let value = deserialize_value(&mut &bytes[..], BasicBag).expect("decode failed");
        match value {
            ConstantData::Code { code } => *code,
            other => panic!("expected Code, got {other:?}"),
        }
    }

    fn decode_tuple(hex: &str) -> Vec<ConstantData> {
        let bytes = hex_to_bytes(hex);
        let value = deserialize_value(&mut &bytes[..], BasicBag).expect("decode failed");
        match value {
            ConstantData::Tuple { elements } => elements,
            other => panic!("expected Tuple, got {other:?}"),
        }
    }

    /// CPython 3.14 marshal output for: `compile("x = 1", "<t>", "exec")`.
    /// Exercises FLAG_REF on the code object and TYPE_REF for qualname
    /// pointing back at the obj_name slot.
    #[test]
    fn cpython_314_trivial_assignment() {
        let hex = "e30000000000000000000000000100000000000000f30a00000080005e017400520123002902\
                   e9010000004e2901da0178a900f300000000da033c743eda083c6d6f64756c653e72070000000100\
                   0000730a000000f003010101d8040582017205000000";
        let code = decode_code(hex);
        assert_eq!(code.obj_name.as_str(), "<module>");
        assert_eq!(code.qualname.as_str(), "<module>");
        assert_eq!(code.source_path.as_str(), "<t>");
        assert_eq!(code.arg_count, 0);
        assert_eq!(code.max_stackdepth, 1);
        assert_eq!(code.names.len(), 1);
        assert_eq!(code.names[0].as_str(), "x");
        assert_eq!(code.constants.len(), 2);
        // (1, None)
        let consts: &[ConstantData] = &code.constants;
        assert!(matches!(
            consts[0],
            ConstantData::Integer { ref value } if *value == 1.into(),
        ));
        assert!(matches!(consts[1], ConstantData::None));
    }

    /// CPython 3.14 marshal output for a module with a nested function
    /// and a string constant. Verifies that nested code objects inside
    /// a const tuple share the surrounding code's ref space.
    #[test]
    fn cpython_314_nested_code_and_string_const() {
        let hex = "e30000000000000000000000000100000000000000f310000000800052001700740052017401\
                   520223002903630200000000000000000000000200000003000000f3120000008000570\
                   12c0000000000000000000000230029014ea9002902da0161da016273020000002626da033c743e\
                   da0361646472070000000200000073090000008000d80b0c8d35804cf300000000da0568656c6c\
                   6f4e29027207000000da084752454554494e47720300000072080000007206000000da083c6d6f\
                   64756c653e720b000000010000007311000000f003010101f204010111f006000c1382087208000000";
        let code = decode_code(hex);
        assert_eq!(code.obj_name.as_str(), "<module>");
        assert_eq!(code.names.len(), 2);
        assert_eq!(code.names[0].as_str(), "add");
        assert_eq!(code.names[1].as_str(), "GREETING");
        assert_eq!(code.constants.len(), 3);
        // Inner code, "hello", None
        let consts: &[ConstantData] = &code.constants;
        let inner = match &consts[0] {
            ConstantData::Code { code } => code,
            other => panic!("expected nested Code, got {other:?}"),
        };
        assert_eq!(inner.obj_name.as_str(), "add");
        assert_eq!(inner.qualname.as_str(), "add");
        assert_eq!(inner.arg_count, 2);
        assert_eq!(inner.varnames.len(), 2);
        assert_eq!(inner.varnames[0].as_str(), "a");
        assert_eq!(inner.varnames[1].as_str(), "b");
        assert!(matches!(
            consts[1],
            ConstantData::Str { ref value } if value.as_str().ok() == Some("hello"),
        ));
        assert!(matches!(consts[2], ConstantData::None));
    }

    /// CPython 3.14 marshal output for:
    /// `(compile("x = 1", "<t>", "exec"),)`.
    /// The outer tuple occupies ref slot 0 and the code object occupies
    /// slot 1, so code-object fields must preserve that global ref offset.
    #[test]
    fn cpython_314_code_inside_tuple_preserves_ref_indexes() {
        let hex = "a901630000000000000000000000000100000000000000f30a00000080005e017400\
                   520123002902e9010000004e2901da0178a900f300000000da033c743eda083c6d6f\
                   64756c653e720700000001000000730a000000f003010101d8040582017205000000";
        let tuple = decode_tuple(hex);
        assert_eq!(tuple.len(), 1);
        let code = match &tuple[0] {
            ConstantData::Code { code } => code,
            other => panic!("expected nested Code, got {other:?}"),
        };
        assert_eq!(code.obj_name.as_str(), "<module>");
        assert_eq!(code.qualname.as_str(), "<module>");
        assert_eq!(code.source_path.as_str(), "<t>");
        assert_eq!(code.names.len(), 1);
        assert_eq!(code.names[0].as_str(), "x");
        assert_eq!(code.constants.len(), 2);
    }
}
