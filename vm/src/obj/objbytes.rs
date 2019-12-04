use std::cell::Cell;
use std::mem::size_of;
use std::ops::Deref;

use super::objbyteinner::{
    ByteInnerExpandtabsOptions, ByteInnerFindOptions, ByteInnerNewOptions, ByteInnerPaddingOptions,
    ByteInnerPosition, ByteInnerSplitOptions, ByteInnerSplitlinesOptions,
    ByteInnerTranslateOptions, PyByteInner,
};
use super::objint::PyIntRef;
use super::objiter;
use super::objslice::PySliceRef;
use super::objstr::{PyString, PyStringRef};
use super::objtuple::PyTupleRef;
use super::objtype::PyClassRef;
use crate::cformat::CFormatString;
use crate::function::OptionalArg;
use crate::obj::objstr::do_cformat_string;
use crate::pyhash;
use crate::pyobject::{
    Either, IntoPyObject, PyClassImpl, PyContext, PyIterable, PyObjectRef, PyRef, PyResult,
    PyValue, TryFromObject, TypeProtocol,
};
use crate::vm::VirtualMachine;
use std::str::FromStr;

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
            Ok(vm.ctx.new_bytes(self.inner.add(other)))
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
    fn join(self, iter: PyIterable<PyByteInner>, vm: &VirtualMachine) -> PyResult {
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

    fn do_cformat(
        &self,
        vm: &VirtualMachine,
        format_string: CFormatString,
        values_obj: PyObjectRef,
    ) -> PyResult {
        let final_string = do_cformat_string(vm, format_string, values_obj)?;
        Ok(vm
            .ctx
            .new_bytes(final_string.as_str().as_bytes().to_owned()))
    }

    #[pymethod(name = "__mod__")]
    fn modulo(self, values: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let format_string_text = std::str::from_utf8(&self.inner.elements).unwrap();
        let format_string = CFormatString::from_str(format_string_text)
            .map_err(|err| vm.new_value_error(err.to_string()))?;
        self.do_cformat(vm, format_string, values.clone())
    }

    #[pymethod(name = "__rmod__")]
    fn rmod(self, _values: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.not_implemented())
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
    ) -> PyResult<PyStringRef> {
        let encoding = encoding.into_option();
        vm.decode(self.into_object(), encoding.clone(), errors.into_option())?
            .downcast::<PyString>()
            .map_err(|obj| {
                vm.new_type_error(format!(
                    "'{}' decoder returned '{}' instead of 'str'; use codecs.encode() to \
                     encode arbitrary types",
                    encoding.as_ref().map_or("utf-8", |s| s.as_str()),
                    obj.class().name,
                ))
            })
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
