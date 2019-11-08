use std::cell::Cell;
use std::mem::size_of;
use std::ops::Deref;

use wtf8;

use super::objbyteinner::{
    ByteInnerExpandtabsOptions, ByteInnerFindOptions, ByteInnerNewOptions, ByteInnerPaddingOptions,
    ByteInnerPosition, ByteInnerSplitOptions, ByteInnerSplitlinesOptions,
    ByteInnerTranslateOptions, PyByteInner,
};
use super::objint::PyIntRef;
use super::objiter;
use super::objslice::PySliceRef;
use super::objstr::PyStringRef;
use super::objtuple::PyTupleRef;
use super::objtype::PyClassRef;
use crate::function::OptionalArg;
use crate::pyhash;
use crate::pyobject::{
    Either, IntoPyObject, PyClassImpl, PyContext, PyIterable, PyObjectRef, PyRef, PyResult,
    PyValue, TryFromObject,
};
use crate::vm::VirtualMachine;

/// "bytes(iterable_of_ints) -> bytes\n\
/// bytes(string, encoding[, errors]) -> bytes\n\
/// bytes(bytes_or_buffer) -> immutable copy of bytes_or_buffer\n\
/// bytes(int) -> bytes object of size given by the parameter initialized with null bytes\n\
/// bytes() -> empty bytes object\n\nConstruct an immutable array of bytes from:\n  \
/// - an iterable yielding integers in range(256)\n  \
/// - a text string encoded using the specified encoding\n  \
/// - any object implementing the buffer API.\n  \
/// - an integer";
#[pyclass(name = "bytes")]
#[derive(Clone, Debug)]
pub struct PyBytes {
    inner: PyByteInner,
}
pub type PyBytesRef = PyRef<PyBytes>;

impl PyBytes {
    pub fn new(elements: Vec<u8>) -> Self {
        PyBytes {
            inner: PyByteInner { elements },
        }
    }

    pub fn from_string(value: &str, encoding: &str, vm: &VirtualMachine) -> PyResult<Self> {
        Ok(PyBytes {
            inner: PyByteInner::from_string(value, encoding, vm)?,
        })
    }

    pub fn get_value(&self) -> &[u8] {
        &self.inner.elements
    }
}

impl IntoPyObject for Vec<u8> {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytes(self))
    }
}

impl Deref for PyBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.inner.elements
    }
}

impl PyValue for PyBytes {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.bytes_type()
    }
}

pub fn get_value<'a>(obj: &'a PyObjectRef) -> impl Deref<Target = Vec<u8>> + 'a {
    &obj.payload::<PyBytes>().unwrap().inner.elements
}

pub fn init(context: &PyContext) {
    PyBytesRef::extend_class(context, &context.types.bytes_type);
    let bytes_type = &context.types.bytes_type;
    extend_class!(context, bytes_type, {
    "fromhex" => context.new_rustfunc(PyBytesRef::fromhex),
    "maketrans" => context.new_rustfunc(PyByteInner::maketrans),

    });
    PyBytesIterator::extend_class(context, &context.types.bytesiterator_type);
}

#[pyimpl]
impl PyBytesRef {
    #[pyslot(new)]
    fn tp_new(
        cls: PyClassRef,
        options: ByteInnerNewOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyBytesRef> {
        PyBytes {
            inner: options.get_value(vm)?,
        }
        .into_ref_with_type(vm, cls)
    }

    #[pymethod(name = "__repr__")]
    fn repr(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.new_str(format!("b'{}'", self.inner.repr()?)))
    }

    #[pymethod(name = "__len__")]
    fn len(self, _vm: &VirtualMachine) -> usize {
        self.inner.len()
    }

    #[pymethod(name = "__eq__")]
    fn eq(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.inner.eq(other, vm)
    }
    #[pymethod(name = "__ge__")]
    fn ge(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.inner.ge(other, vm)
    }
    #[pymethod(name = "__le__")]
    fn le(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.inner.le(other, vm)
    }
    #[pymethod(name = "__gt__")]
    fn gt(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.inner.gt(other, vm)
    }
    #[pymethod(name = "__lt__")]
    fn lt(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.inner.lt(other, vm)
    }

    #[pymethod(name = "__hash__")]
    fn hash(self, _vm: &VirtualMachine) -> pyhash::PyHash {
        self.inner.hash()
    }

    #[pymethod(name = "__iter__")]
    fn iter(self, _vm: &VirtualMachine) -> PyBytesIterator {
        PyBytesIterator {
            position: Cell::new(0),
            bytes: self,
        }
    }

    #[pymethod(name = "__sizeof__")]
    fn sizeof(self, _vm: &VirtualMachine) -> PyResult<usize> {
        Ok(size_of::<Self>() + self.inner.elements.len() * size_of::<u8>())
    }

    #[pymethod(name = "__add__")]
    fn add(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Ok(other) = PyByteInner::try_from_object(vm, other) {
            Ok(vm.ctx.new_bytearray(self.inner.add(other)))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__contains__")]
    fn contains(
        self,
        needle: Either<PyByteInner, PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        self.inner.contains(needle, vm)
    }

    #[pymethod(name = "__getitem__")]
    fn getitem(self, needle: Either<i32, PySliceRef>, vm: &VirtualMachine) -> PyResult {
        self.inner.getitem(needle, vm)
    }

    #[pymethod(name = "isalnum")]
    fn isalnum(self, vm: &VirtualMachine) -> bool {
        self.inner.isalnum(vm)
    }

    #[pymethod(name = "isalpha")]
    fn isalpha(self, vm: &VirtualMachine) -> bool {
        self.inner.isalpha(vm)
    }

    #[pymethod(name = "isascii")]
    fn isascii(self, vm: &VirtualMachine) -> bool {
        self.inner.isascii(vm)
    }

    #[pymethod(name = "isdigit")]
    fn isdigit(self, vm: &VirtualMachine) -> bool {
        self.inner.isdigit(vm)
    }

    #[pymethod(name = "islower")]
    fn islower(self, vm: &VirtualMachine) -> bool {
        self.inner.islower(vm)
    }

    #[pymethod(name = "isspace")]
    fn isspace(self, vm: &VirtualMachine) -> bool {
        self.inner.isspace(vm)
    }

    #[pymethod(name = "isupper")]
    fn isupper(self, vm: &VirtualMachine) -> bool {
        self.inner.isupper(vm)
    }

    #[pymethod(name = "istitle")]
    fn istitle(self, vm: &VirtualMachine) -> bool {
        self.inner.istitle(vm)
    }

    #[pymethod(name = "lower")]
    fn lower(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytes(self.inner.lower(vm)))
    }

    #[pymethod(name = "upper")]
    fn upper(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytes(self.inner.upper(vm)))
    }

    #[pymethod(name = "capitalize")]
    fn capitalize(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytes(self.inner.capitalize(vm)))
    }

    #[pymethod(name = "swapcase")]
    fn swapcase(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytes(self.inner.swapcase(vm)))
    }

    #[pymethod(name = "hex")]
    fn hex(self, vm: &VirtualMachine) -> String {
        self.inner.hex(vm)
    }

    fn fromhex(string: PyStringRef, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytes(PyByteInner::fromhex(string.as_str(), vm)?))
    }

    #[pymethod(name = "center")]
    fn center(self, options: ByteInnerPaddingOptions, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytes(self.inner.center(options, vm)?))
    }

    #[pymethod(name = "ljust")]
    fn ljust(self, options: ByteInnerPaddingOptions, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytes(self.inner.ljust(options, vm)?))
    }

    #[pymethod(name = "rjust")]
    fn rjust(self, options: ByteInnerPaddingOptions, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytes(self.inner.rjust(options, vm)?))
    }

    #[pymethod(name = "count")]
    fn count(self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        self.inner.count(options, vm)
    }

    #[pymethod(name = "join")]
    fn join(self, iter: PyIterable, vm: &VirtualMachine) -> PyResult {
        self.inner.join(iter, vm)
    }

    #[pymethod(name = "endswith")]
    fn endswith(
        self,
        suffix: Either<PyByteInner, PyTupleRef>,
        start: OptionalArg<PyObjectRef>,
        end: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        self.inner.startsendswith(suffix, start, end, true, vm)
    }

    #[pymethod(name = "startswith")]
    fn startswith(
        self,
        prefix: Either<PyByteInner, PyTupleRef>,
        start: OptionalArg<PyObjectRef>,
        end: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        self.inner.startsendswith(prefix, start, end, false, vm)
    }

    #[pymethod(name = "find")]
    fn find(self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<isize> {
        self.inner.find(options, false, vm)
    }

    #[pymethod(name = "index")]
    fn index(self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<isize> {
        let res = self.inner.find(options, false, vm)?;
        if res == -1 {
            return Err(vm.new_value_error("substring not found".to_string()));
        }
        Ok(res)
    }

    #[pymethod(name = "rfind")]
    fn rfind(self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<isize> {
        self.inner.find(options, true, vm)
    }

    #[pymethod(name = "rindex")]
    fn rindex(self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<isize> {
        let res = self.inner.find(options, true, vm)?;
        if res == -1 {
            return Err(vm.new_value_error("substring not found".to_string()));
        }
        Ok(res)
    }

    #[pymethod(name = "translate")]
    fn translate(self, options: ByteInnerTranslateOptions, vm: &VirtualMachine) -> PyResult {
        self.inner.translate(options, vm)
    }

    #[pymethod(name = "strip")]
    fn strip(self, chars: OptionalArg<PyByteInner>, vm: &VirtualMachine) -> PyResult {
        Ok(vm
            .ctx
            .new_bytes(self.inner.strip(chars, ByteInnerPosition::All, vm)?))
    }

    #[pymethod(name = "lstrip")]
    fn lstrip(self, chars: OptionalArg<PyByteInner>, vm: &VirtualMachine) -> PyResult {
        Ok(vm
            .ctx
            .new_bytes(self.inner.strip(chars, ByteInnerPosition::Left, vm)?))
    }

    #[pymethod(name = "rstrip")]
    fn rstrip(self, chars: OptionalArg<PyByteInner>, vm: &VirtualMachine) -> PyResult {
        Ok(vm
            .ctx
            .new_bytes(self.inner.strip(chars, ByteInnerPosition::Right, vm)?))
    }

    #[pymethod(name = "split")]
    fn split(self, options: ByteInnerSplitOptions, vm: &VirtualMachine) -> PyResult {
        let as_bytes = self
            .inner
            .split(options, false)?
            .iter()
            .map(|x| vm.ctx.new_bytes(x.to_vec()))
            .collect::<Vec<PyObjectRef>>();
        Ok(vm.ctx.new_list(as_bytes))
    }

    #[pymethod(name = "rsplit")]
    fn rsplit(self, options: ByteInnerSplitOptions, vm: &VirtualMachine) -> PyResult {
        let as_bytes = self
            .inner
            .split(options, true)?
            .iter()
            .map(|x| vm.ctx.new_bytes(x.to_vec()))
            .collect::<Vec<PyObjectRef>>();
        Ok(vm.ctx.new_list(as_bytes))
    }

    #[pymethod(name = "partition")]
    fn partition(self, sep: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let sepa = PyByteInner::try_from_object(vm, sep.clone())?;

        let (left, right) = self.inner.partition(&sepa, false)?;
        Ok(vm
            .ctx
            .new_tuple(vec![vm.ctx.new_bytes(left), sep, vm.ctx.new_bytes(right)]))
    }
    #[pymethod(name = "rpartition")]
    fn rpartition(self, sep: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let sepa = PyByteInner::try_from_object(vm, sep.clone())?;

        let (left, right) = self.inner.partition(&sepa, true)?;
        Ok(vm
            .ctx
            .new_tuple(vec![vm.ctx.new_bytes(left), sep, vm.ctx.new_bytes(right)]))
    }

    #[pymethod(name = "expandtabs")]
    fn expandtabs(self, options: ByteInnerExpandtabsOptions, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytes(self.inner.expandtabs(options)))
    }

    #[pymethod(name = "splitlines")]
    fn splitlines(self, options: ByteInnerSplitlinesOptions, vm: &VirtualMachine) -> PyResult {
        let as_bytes = self
            .inner
            .splitlines(options)
            .iter()
            .map(|x| vm.ctx.new_bytes(x.to_vec()))
            .collect::<Vec<PyObjectRef>>();
        Ok(vm.ctx.new_list(as_bytes))
    }

    #[pymethod(name = "zfill")]
    fn zfill(self, width: PyIntRef, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytes(self.inner.zfill(width)))
    }

    #[pymethod(name = "replace")]
    fn replace(
        self,
        old: PyByteInner,
        new: PyByteInner,
        count: OptionalArg<PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        Ok(vm.ctx.new_bytes(self.inner.replace(old, new, count)?))
    }

    #[pymethod(name = "title")]
    fn title(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytes(self.inner.title()))
    }

    #[pymethod(name = "__mul__")]
    fn repeat(self, n: isize, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytes(self.inner.repeat(n, vm)?))
    }

    #[pymethod(name = "__rmul__")]
    fn rmul(self, n: isize, vm: &VirtualMachine) -> PyResult {
        self.repeat(n, vm)
    }

    /// Return a string decoded from the given bytes.
    /// Default encoding is 'utf-8'.
    /// Default errors is 'strict', meaning that encoding errors raise a UnicodeError.
    /// Other possible values are 'ignore', 'replace'
    /// For a list of possible encodings,
    /// see https://docs.python.org/3/library/codecs.html#standard-encodings
    /// currently, only 'utf-8' and 'ascii' emplemented
    #[pymethod(name = "decode")]
    fn decode(
        self,
        encoding: OptionalArg<PyStringRef>,
        errors: OptionalArg<PyStringRef>,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        let mut strict_mod = true;
        let replacing_char = match errors {
            OptionalArg::Present(ref input) => match input.as_str() {
                "replace" => {
                    strict_mod = false;
                    Some('\u{FFFD}')
                }
                "ignore" => {
                    strict_mod = false;
                    None
                }
                _ => None,
            },
            OptionalArg::Missing => None,
        };
        let encoding_type = match encoding {
            OptionalArg::Present(ref input) => input.as_str(),
            OptionalArg::Missing => "utf-8",
        };

        let decode_error = Err(vm.new_value_error("DecodeError".to_string()));

        let mut decode_content = String::new();
        match encoding_type {
            "ascii" => {
                for &b in self.get_value() {
                    if b.is_ascii() {
                        decode_content.push(b as char)
                    } else if !strict_mod && replacing_char.is_some() {
                        decode_content.push(replacing_char.unwrap())
                    }
                }
            }
            "utf-8" | "utf8" | "" => {
                let mut p: u32 = 0u32;
                let mut remaining_bytes = 0;
                for &b in self.get_value() {
                    if (b as u8) & 128 == 0 {
                        if b.is_ascii() {
                            decode_content.push(b as char)
                        } else if !strict_mod && replacing_char.is_some() {
                            decode_content.push(replacing_char.unwrap())
                        }
                    } else if (b as u8) & 192 == 128 {
                        remaining_bytes -= 1;

                        p += u32::from(b as u8 & 63) << (6 * remaining_bytes);

                        if remaining_bytes == 0 {
                            match wtf8::CodePoint::from_u32(p) {
                                Some(cp) => {
                                    if !strict_mod && replacing_char.is_some() {
                                        decode_content.push(cp.to_char_lossy());
                                    } else {
                                        match cp.to_char() {
                                            Some(c) => decode_content.push(c),
                                            None => {
                                                if strict_mod {
                                                    return decode_error;
                                                }
                                            }
                                        }
                                    }
                                }
                                None => {
                                    if let Some(replacing_char) = replacing_char {
                                        decode_content.push(replacing_char)
                                    }
                                }
                            }
                            p = 0u32;
                        }
                    } else if (b as u8) & 224 == 192 {
                        remaining_bytes = 1;
                        p = u32::from(b as u8 & 31) << 6;
                    } else if (b as u8) & 240 == 224 {
                        remaining_bytes = 2;
                        p = u32::from(b as u8 & 15) << 12;
                    } else if (b as u8) & 248 == 240 {
                        remaining_bytes = 3;
                        p = u32::from(b as u8 & 7) << 18;
                    } else if !strict_mod && replacing_char.is_some() {
                        decode_content.push(replacing_char.unwrap())
                    }
                }
            }
            _ => {
                return Err(vm.new_lookup_error(format!("unknown encoding: {}", encoding_type)));
            }
        }
        Ok(decode_content)
    }
}

#[pyclass]
#[derive(Debug)]
pub struct PyBytesIterator {
    position: Cell<usize>,
    bytes: PyBytesRef,
}

impl PyValue for PyBytesIterator {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.bytesiterator_type()
    }
}

#[pyimpl]
impl PyBytesIterator {
    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult<u8> {
        if self.position.get() < self.bytes.inner.len() {
            let ret = self.bytes[self.position.get()];
            self.position.set(self.position.get() + 1);
            Ok(ret)
        } else {
            Err(objiter::new_stop_iteration(vm))
        }
    }

    #[pymethod(name = "__iter__")]
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyRef<Self> {
        zelf
    }
}
