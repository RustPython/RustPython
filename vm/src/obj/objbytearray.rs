//! Implementation of the python bytearray object.
use std::cell::{Cell, RefCell};
use std::convert::TryFrom;

use super::objbyteinner::{
    ByteInnerExpandtabsOptions, ByteInnerFindOptions, ByteInnerNewOptions, ByteInnerPaddingOptions,
    ByteInnerPosition, ByteInnerSplitOptions, ByteInnerSplitlinesOptions,
    ByteInnerTranslateOptions, ByteOr, PyByteInner,
};
use super::objint::PyIntRef;
use super::objiter;
use super::objslice::PySliceRef;
use super::objstr::PyStringRef;
use super::objtuple::PyTupleRef;
use super::objtype::PyClassRef;
use crate::cformat::CFormatString;
use crate::function::OptionalArg;
use crate::obj::objstr::do_cformat_string;
use crate::pyobject::{
    Either, PyClassImpl, PyContext, PyIterable, PyObjectRef, PyRef, PyResult, PyValue,
    TryFromObject,
};
use crate::vm::VirtualMachine;
use std::mem::size_of;
use std::str::FromStr;

/// "bytearray(iterable_of_ints) -> bytearray\n\
///  bytearray(string, encoding[, errors]) -> bytearray\n\
///  bytearray(bytes_or_buffer) -> mutable copy of bytes_or_buffer\n\
///  bytearray(int) -> bytes array of size given by the parameter initialized with null bytes\n\
///  bytearray() -> empty bytes array\n\n\
///  Construct a mutable bytearray object from:\n  \
///  - an iterable yielding integers in range(256)\n  \
///  - a text string encoded using the specified encoding\n  \
///  - a bytes or a buffer object\n  \
///  - any object implementing the buffer API.\n  \
///  - an integer";
#[pyclass(name = "bytearray")]
#[derive(Clone, Debug)]
pub struct PyByteArray {
    pub inner: RefCell<PyByteInner>,
}
pub type PyByteArrayRef = PyRef<PyByteArray>;

impl PyByteArray {
    pub fn new(data: Vec<u8>) -> Self {
        PyByteArray {
            inner: RefCell::new(PyByteInner { elements: data }),
        }
    }

    pub fn from_inner(inner: PyByteInner) -> Self {
        PyByteArray {
            inner: RefCell::new(inner),
        }
    }

    // pub fn get_value(&self) -> Vec<u8> {
    //     self.inner.borrow().clone().elements
    // }

    // pub fn get_value_mut(&self) -> Vec<u8> {
    //     self.inner.borrow_mut().clone().elements
    // }
}

impl PyValue for PyByteArray {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.bytearray_type()
    }
}

// pub fn get_value(obj: &PyObjectRef) -> Vec<u8> {
//     obj.payload::<PyByteArray>().unwrap().get_value()
// }

// pub fn get_value_mut(obj: &PyObjectRef) -> Vec<u8> {
//     obj.payload::<PyByteArray>().unwrap().get_value_mut()
// }

/// Fill bytearray class methods dictionary.
pub fn init(context: &PyContext) {
    PyByteArrayRef::extend_class(context, &context.types.bytearray_type);
    let bytearray_type = &context.types.bytearray_type;
    extend_class!(context, bytearray_type, {
    "fromhex" => context.new_rustfunc(PyByteArrayRef::fromhex),
    "maketrans" => context.new_rustfunc(PyByteInner::maketrans),
    });

    PyByteArrayIterator::extend_class(context, &context.types.bytearrayiterator_type);
}

#[pyimpl]
impl PyByteArrayRef {
    #[pyslot(new)]
    fn tp_new(
        cls: PyClassRef,
        options: ByteInnerNewOptions,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteArrayRef> {
        PyByteArray::from_inner(options.get_value(vm)?).into_ref_with_type(vm, cls)
    }

    #[pymethod(name = "__repr__")]
    fn repr(self, _vm: &VirtualMachine) -> PyResult<String> {
        Ok(format!("bytearray(b'{}')", self.inner.borrow().repr()?))
    }

    #[pymethod(name = "__len__")]
    fn len(self, _vm: &VirtualMachine) -> usize {
        self.inner.borrow().len()
    }

    #[pymethod(name = "__sizeof__")]
    fn sizeof(self, _vm: &VirtualMachine) -> usize {
        size_of::<Self>() + self.inner.borrow().len() * size_of::<u8>()
    }

    #[pymethod(name = "__eq__")]
    fn eq(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.inner.borrow().eq(other, vm)
    }

    #[pymethod(name = "__ge__")]
    fn ge(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.inner.borrow().ge(other, vm)
    }

    #[pymethod(name = "__le__")]
    fn le(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.inner.borrow().le(other, vm)
    }

    #[pymethod(name = "__gt__")]
    fn gt(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.inner.borrow().gt(other, vm)
    }

    #[pymethod(name = "__lt__")]
    fn lt(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.inner.borrow().lt(other, vm)
    }

    #[pymethod(name = "__hash__")]
    fn hash(self, vm: &VirtualMachine) -> PyResult<()> {
        Err(vm.new_type_error("unhashable type: bytearray".to_string()))
    }

    #[pymethod(name = "__iter__")]
    fn iter(self, _vm: &VirtualMachine) -> PyByteArrayIterator {
        PyByteArrayIterator {
            position: Cell::new(0),
            bytearray: self,
        }
    }

    #[pymethod(name = "__add__")]
    fn add(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Ok(other) = PyByteInner::try_from_object(vm, other) {
            Ok(vm.ctx.new_bytearray(self.inner.borrow().add(other)))
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
        self.inner.borrow().contains(needle, vm)
    }

    #[pymethod(name = "__getitem__")]
    fn getitem(self, needle: Either<i32, PySliceRef>, vm: &VirtualMachine) -> PyResult {
        self.inner.borrow().getitem(needle, vm)
    }

    #[pymethod(name = "__setitem__")]
    fn setitem(
        self,
        needle: Either<i32, PySliceRef>,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        self.inner.borrow_mut().setitem(needle, value, vm)
    }

    #[pymethod(name = "__delitem__")]
    fn delitem(self, needle: Either<i32, PySliceRef>, vm: &VirtualMachine) -> PyResult<()> {
        self.inner.borrow_mut().delitem(needle, vm)
    }

    #[pymethod(name = "isalnum")]
    fn isalnum(self, vm: &VirtualMachine) -> bool {
        self.inner.borrow().isalnum(vm)
    }

    #[pymethod(name = "isalpha")]
    fn isalpha(self, vm: &VirtualMachine) -> bool {
        self.inner.borrow().isalpha(vm)
    }

    #[pymethod(name = "isascii")]
    fn isascii(self, vm: &VirtualMachine) -> bool {
        self.inner.borrow().isascii(vm)
    }

    #[pymethod(name = "isdigit")]
    fn isdigit(self, vm: &VirtualMachine) -> bool {
        self.inner.borrow().isdigit(vm)
    }

    #[pymethod(name = "islower")]
    fn islower(self, vm: &VirtualMachine) -> bool {
        self.inner.borrow().islower(vm)
    }

    #[pymethod(name = "isspace")]
    fn isspace(self, vm: &VirtualMachine) -> bool {
        self.inner.borrow().isspace(vm)
    }

    #[pymethod(name = "isupper")]
    fn isupper(self, vm: &VirtualMachine) -> bool {
        self.inner.borrow().isupper(vm)
    }

    #[pymethod(name = "istitle")]
    fn istitle(self, vm: &VirtualMachine) -> bool {
        self.inner.borrow().istitle(vm)
    }

    #[pymethod(name = "lower")]
    fn lower(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytearray(self.inner.borrow().lower(vm)))
    }

    #[pymethod(name = "upper")]
    fn upper(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytearray(self.inner.borrow().upper(vm)))
    }

    #[pymethod(name = "capitalize")]
    fn capitalize(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytearray(self.inner.borrow().capitalize(vm)))
    }

    #[pymethod(name = "swapcase")]
    fn swapcase(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytearray(self.inner.borrow().swapcase(vm)))
    }

    #[pymethod(name = "hex")]
    fn hex(self, vm: &VirtualMachine) -> String {
        self.inner.borrow().hex(vm)
    }

    fn fromhex(string: PyStringRef, vm: &VirtualMachine) -> PyResult {
        Ok(vm
            .ctx
            .new_bytearray(PyByteInner::fromhex(string.as_str(), vm)?))
    }

    #[pymethod(name = "center")]
    fn center(self, options: ByteInnerPaddingOptions, vm: &VirtualMachine) -> PyResult {
        Ok(vm
            .ctx
            .new_bytearray(self.inner.borrow().center(options, vm)?))
    }

    #[pymethod(name = "ljust")]
    fn ljust(self, options: ByteInnerPaddingOptions, vm: &VirtualMachine) -> PyResult {
        Ok(vm
            .ctx
            .new_bytearray(self.inner.borrow().ljust(options, vm)?))
    }

    #[pymethod(name = "rjust")]
    fn rjust(self, options: ByteInnerPaddingOptions, vm: &VirtualMachine) -> PyResult {
        Ok(vm
            .ctx
            .new_bytearray(self.inner.borrow().rjust(options, vm)?))
    }

    #[pymethod(name = "count")]
    fn count(self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<usize> {
        self.inner.borrow().count(options, vm)
    }

    #[pymethod(name = "join")]
    fn join(self, iter: PyIterable<PyByteInner>, vm: &VirtualMachine) -> PyResult {
        self.inner.borrow().join(iter, vm)
    }

    #[pymethod(name = "endswith")]
    fn endswith(
        self,
        suffix: Either<PyByteInner, PyTupleRef>,
        start: OptionalArg<PyObjectRef>,
        end: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        self.inner
            .borrow()
            .startsendswith(suffix, start, end, true, vm)
    }

    #[pymethod(name = "startswith")]
    fn startswith(
        self,
        prefix: Either<PyByteInner, PyTupleRef>,
        start: OptionalArg<PyObjectRef>,
        end: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        self.inner
            .borrow()
            .startsendswith(prefix, start, end, false, vm)
    }

    #[pymethod(name = "find")]
    fn find(self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<isize> {
        self.inner.borrow().find(options, false, vm)
    }

    #[pymethod(name = "index")]
    fn index(self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<isize> {
        let res = self.inner.borrow().find(options, false, vm)?;
        if res == -1 {
            return Err(vm.new_value_error("substring not found".to_string()));
        }
        Ok(res)
    }

    #[pymethod(name = "rfind")]
    fn rfind(self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<isize> {
        self.inner.borrow().find(options, true, vm)
    }

    #[pymethod(name = "rindex")]
    fn rindex(self, options: ByteInnerFindOptions, vm: &VirtualMachine) -> PyResult<isize> {
        let res = self.inner.borrow().find(options, true, vm)?;
        if res == -1 {
            return Err(vm.new_value_error("substring not found".to_string()));
        }
        Ok(res)
    }

    #[pymethod(name = "remove")]
    fn remove(self, x: PyIntRef, vm: &VirtualMachine) -> PyResult<()> {
        let x = x.as_bigint().byte_or(vm)?;

        let bytes = &mut self.inner.borrow_mut().elements;
        let pos = bytes
            .iter()
            .position(|b| *b == x)
            .ok_or_else(|| vm.new_value_error("value not found in bytearray".to_string()))?;

        bytes.remove(pos);

        Ok(())
    }

    #[pymethod(name = "translate")]
    fn translate(self, options: ByteInnerTranslateOptions, vm: &VirtualMachine) -> PyResult {
        self.inner.borrow().translate(options, vm)
    }

    #[pymethod(name = "strip")]
    fn strip(self, chars: OptionalArg<PyByteInner>, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytes(
            self.inner
                .borrow()
                .strip(chars, ByteInnerPosition::All, vm)?,
        ))
    }

    #[pymethod(name = "lstrip")]
    fn lstrip(self, chars: OptionalArg<PyByteInner>, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytes(
            self.inner
                .borrow()
                .strip(chars, ByteInnerPosition::Left, vm)?,
        ))
    }

    #[pymethod(name = "rstrip")]
    fn rstrip(self, chars: OptionalArg<PyByteInner>, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytes(
            self.inner
                .borrow()
                .strip(chars, ByteInnerPosition::Right, vm)?,
        ))
    }

    #[pymethod(name = "split")]
    fn split(self, options: ByteInnerSplitOptions, vm: &VirtualMachine) -> PyResult {
        let as_bytes = self
            .inner
            .borrow()
            .split(options, false)?
            .iter()
            .map(|x| vm.ctx.new_bytearray(x.to_vec()))
            .collect::<Vec<PyObjectRef>>();
        Ok(vm.ctx.new_list(as_bytes))
    }

    #[pymethod(name = "rsplit")]
    fn rsplit(self, options: ByteInnerSplitOptions, vm: &VirtualMachine) -> PyResult {
        let as_bytes = self
            .inner
            .borrow()
            .split(options, true)?
            .iter()
            .map(|x| vm.ctx.new_bytearray(x.to_vec()))
            .collect::<Vec<PyObjectRef>>();
        Ok(vm.ctx.new_list(as_bytes))
    }

    #[pymethod(name = "partition")]
    fn partition(self, sep: PyByteInner, vm: &VirtualMachine) -> PyResult {
        // sep ALWAYS converted to  bytearray even it's bytes or memoryview
        // so its ok to accept PyByteInner
        let (left, right) = self.inner.borrow().partition(&sep, false)?;
        Ok(vm.ctx.new_tuple(vec![
            vm.ctx.new_bytearray(left),
            vm.ctx.new_bytearray(sep.elements),
            vm.ctx.new_bytearray(right),
        ]))
    }

    #[pymethod(name = "rpartition")]
    fn rpartition(self, sep: PyByteInner, vm: &VirtualMachine) -> PyResult {
        let (left, right) = self.inner.borrow().partition(&sep, true)?;
        Ok(vm.ctx.new_tuple(vec![
            vm.ctx.new_bytearray(left),
            vm.ctx.new_bytearray(sep.elements),
            vm.ctx.new_bytearray(right),
        ]))
    }

    #[pymethod(name = "expandtabs")]
    fn expandtabs(self, options: ByteInnerExpandtabsOptions, vm: &VirtualMachine) -> PyResult {
        Ok(vm
            .ctx
            .new_bytearray(self.inner.borrow().expandtabs(options)))
    }

    #[pymethod(name = "splitlines")]
    fn splitlines(self, options: ByteInnerSplitlinesOptions, vm: &VirtualMachine) -> PyResult {
        let as_bytes = self
            .inner
            .borrow()
            .splitlines(options)
            .iter()
            .map(|x| vm.ctx.new_bytearray(x.to_vec()))
            .collect::<Vec<PyObjectRef>>();
        Ok(vm.ctx.new_list(as_bytes))
    }

    #[pymethod(name = "zfill")]
    fn zfill(self, width: PyIntRef, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytearray(self.inner.borrow().zfill(width)))
    }

    #[pymethod(name = "replace")]
    fn replace(
        self,
        old: PyByteInner,
        new: PyByteInner,
        count: OptionalArg<PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        Ok(vm
            .ctx
            .new_bytearray(self.inner.borrow().replace(old, new, count)?))
    }

    #[pymethod(name = "clear")]
    fn clear(self, _vm: &VirtualMachine) {
        self.inner.borrow_mut().elements.clear();
    }

    #[pymethod(name = "copy")]
    fn copy(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytearray(self.inner.borrow().elements.clone()))
    }

    #[pymethod(name = "append")]
    fn append(self, x: PyIntRef, vm: &VirtualMachine) -> PyResult<()> {
        self.inner
            .borrow_mut()
            .elements
            .push(x.as_bigint().byte_or(vm)?);
        Ok(())
    }

    #[pymethod(name = "extend")]
    fn extend(self, iterable_of_ints: PyIterable, vm: &VirtualMachine) -> PyResult<()> {
        let mut inner = self.inner.borrow_mut();

        for x in iterable_of_ints.iter(vm)? {
            let x = x?;
            let x = PyIntRef::try_from_object(vm, x)?;
            let x = x.as_bigint().byte_or(vm)?;
            inner.elements.push(x);
        }

        Ok(())
    }

    #[pymethod(name = "insert")]
    fn insert(self, mut index: isize, x: PyIntRef, vm: &VirtualMachine) -> PyResult<()> {
        let bytes = &mut self.inner.borrow_mut().elements;
        let len = isize::try_from(bytes.len())
            .map_err(|_e| vm.new_overflow_error("bytearray too big".to_string()))?;

        let x = x.as_bigint().byte_or(vm)?;

        if index >= len {
            bytes.push(x);
            return Ok(());
        }

        if index < 0 {
            index += len;
            index = index.max(0);
        }

        let index = usize::try_from(index)
            .map_err(|_e| vm.new_overflow_error("overflow in index calculation".to_string()))?;

        bytes.insert(index, x);

        Ok(())
    }

    #[pymethod(name = "pop")]
    fn pop(self, vm: &VirtualMachine) -> PyResult<u8> {
        let bytes = &mut self.inner.borrow_mut().elements;
        bytes
            .pop()
            .ok_or_else(|| vm.new_index_error("pop from empty bytearray".to_string()))
    }

    #[pymethod(name = "title")]
    fn title(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytearray(self.inner.borrow().title()))
    }

    #[pymethod(name = "__mul__")]
    fn repeat(self, n: isize, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytearray(self.inner.borrow().repeat(n, vm)?))
    }

    #[pymethod(name = "__rmul__")]
    fn rmul(self, n: isize, vm: &VirtualMachine) -> PyResult {
        self.repeat(n, vm)
    }

    #[pymethod(name = "__imul__")]
    fn irepeat(self, n: isize, vm: &VirtualMachine) -> PyResult<()> {
        self.inner.borrow_mut().irepeat(n, vm)
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
            .new_bytearray(final_string.as_str().as_bytes().to_owned()))
    }

    #[pymethod(name = "__mod__")]
    fn modulo(self, values: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let format_string =
            CFormatString::from_str(std::str::from_utf8(&self.inner.borrow().elements).unwrap())
                .map_err(|err| vm.new_value_error(err.to_string()))?;
        self.do_cformat(vm, format_string, values.clone())
    }

    #[pymethod(name = "__rmod__")]
    fn rmod(self, _values: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.not_implemented())
    }

    #[pymethod(name = "reverse")]
    fn reverse(self, _vm: &VirtualMachine) -> PyResult<()> {
        self.inner.borrow_mut().elements.reverse();
        Ok(())
    }
}

// fn set_value(obj: &PyObjectRef, value: Vec<u8>) {
//     obj.borrow_mut().kind = PyObjectPayload::Bytes { value };
// }

#[pyclass]
#[derive(Debug)]
pub struct PyByteArrayIterator {
    position: Cell<usize>,
    bytearray: PyByteArrayRef,
}

impl PyValue for PyByteArrayIterator {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.bytearrayiterator_type()
    }
}

#[pyimpl]
impl PyByteArrayIterator {
    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult<u8> {
        if self.position.get() < self.bytearray.inner.borrow().len() {
            let ret = self.bytearray.inner.borrow().elements[self.position.get()];
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
