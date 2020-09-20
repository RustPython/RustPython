use super::objtype::PyTypeRef;
use std::{fmt::Debug, ops::Deref};

use crate::obj::objbytes::{PyBytes, PyBytesRef};
use crate::obj::objlist::{PyList, PyListRef};
use crate::obj::{objslice::PySliceRef, objstr::PyStr};
use crate::pyobject::{
    IdProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyThreadingConstraint,
    PyValue, TypeProtocol,
};
use crate::sliceable::{saturate_range, wrap_index, SequenceIndex};
use crate::slots::{BufferProtocol, Hashable};
use crate::stdlib::pystruct::_struct::FormatSpec;
use crate::VirtualMachine;
use crate::{bytesinner::try_as_bytes, slots::Comparable};
use crate::{common::hash::PyHash, pyobject::PyComparisonValue};
use crossbeam_utils::atomic::AtomicCell;
use itertools::Itertools;
use num_bigint::BigInt;
use num_traits::{One, Signed, ToPrimitive, Zero};
use rustpython_common::borrow::{BorrowedValue, BorrowedValueMut};

#[derive(Debug)]
pub struct BufferRef(Box<dyn Buffer>);
impl Drop for BufferRef {
    fn drop(&mut self) {
        self.0.release();
    }
}
impl Deref for BufferRef {
    type Target = dyn Buffer;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}
pub trait Buffer: Debug + PyThreadingConstraint {
    fn get_options(&self) -> BorrowedValue<BufferOptions>;
    fn obj_bytes(&self) -> BorrowedValue<[u8]>;
    fn obj_bytes_mut(&self) -> BorrowedValueMut<[u8]>;
    fn release(&self);
    fn is_resizable(&self) -> bool;

    fn as_contiguous(&self) -> Option<BorrowedValue<[u8]>> {
        let options = self.get_options();
        if !options.contiguous {
            return None;
        }
        Some(self.obj_bytes())
    }

    fn as_contiguous_mut(&self) -> Option<BorrowedValueMut<[u8]>> {
        let options = self.get_options();
        if !options.contiguous {
            return None;
        }
        Some(self.obj_bytes_mut())
    }

    fn try_resizable(&self, vm: &VirtualMachine) -> PyResult<()> {
        if self.is_resizable() {
            Ok(())
        } else {
            Err(vm.new_exception_msg(
                vm.ctx.exceptions.buffer_error.clone(),
                "Existing exports of data: object cannot be re-sized".to_owned(),
            ))
        }
    }
}

#[derive(Debug, Clone)]
pub struct BufferOptions {
    pub readonly: bool,
    pub len: usize,
    pub itemsize: usize,
    pub contiguous: bool,
    // TODO: support multiple dimension array
    pub ndim: usize,
    pub format: String,
}

impl Default for BufferOptions {
    fn default() -> Self {
        BufferOptions {
            readonly: true,
            len: 0,
            itemsize: 1,
            contiguous: true,
            ndim: 1,
            format: "B".to_owned(),
        }
    }
}

#[pyclass(module = false, name = "memoryview")]
#[derive(Debug)]
pub struct PyMemoryView {
    obj: PyObjectRef,
    buffer: BufferRef,
    options: BufferOptions,
    released: AtomicCell<bool>,
    // start should always less or equal to the stop
    // start and stop pointing to the memory index not slice index
    // if length is not zero than [start, stop)
    start: usize,
    stop: usize,
    // step can be negative, means read the memory from stop-1 to start
    step: isize,
    exports: AtomicCell<usize>,
    format_spec: FormatSpec,
}

pub type PyMemoryViewRef = PyRef<PyMemoryView>;

#[pyimpl(with(Hashable, Comparable, BufferProtocol))]
impl PyMemoryView {
    fn parse_format(format: &str, vm: &VirtualMachine) -> PyResult<FormatSpec> {
        FormatSpec::parse(format)
            .map_err(|msg| vm.new_exception_msg(vm.ctx.types.memoryview_type.clone(), msg))
    }

    pub fn from_buffer(obj: PyObjectRef, buffer: BufferRef, vm: &VirtualMachine) -> PyResult<Self> {
        // when we get a buffer means the buffered object is size locked
        // so we can assume the buffer's options will never change as long
        // as memoryview is still alive
        let options = buffer.get_options().clone();
        let len = options.len;
        let format_spec = Self::parse_format(&options.format, vm)?;
        Ok(PyMemoryView {
            obj,
            buffer,
            options,
            released: AtomicCell::new(false),
            start: 0,
            stop: len,
            step: 1,
            exports: AtomicCell::new(0),
            format_spec,
        })
    }

    pub fn try_bytes<F, R>(&self, f: F) -> Option<R>
    where
        F: Fn(&[u8]) -> R,
    {
        try_as_bytes(self.obj.clone(), f)
    }

    #[pyslot]
    fn tp_new(_cls: PyTypeRef, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyMemoryViewRef> {
        let buffer = try_buffer_from_object(obj.clone(), vm)?;
        Ok(PyMemoryView::from_buffer(obj, buffer, vm)?.into_ref(vm))
    }

    #[pymethod]
    fn release(&self) {
        // avoid double release
        if !self.released.compare_and_swap(false, true) && self.exports.load() == 0 {
            self.buffer.release();
        }
    }

    fn try_not_released(&self, vm: &VirtualMachine) -> PyResult<()> {
        if self.released.load() {
            Err(vm.new_value_error("operation forbidden on released memoryview object".to_owned()))
        } else {
            Ok(())
        }
    }

    #[pyproperty]
    fn obj(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        self.try_not_released(vm).map(|_| self.obj.clone())
    }

    #[pyproperty]
    fn nbytes(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.try_not_released(vm)
            .map(|_| self.options.len * self.options.itemsize)
    }

    #[pyproperty]
    fn readonly(&self, vm: &VirtualMachine) -> PyResult<bool> {
        self.try_not_released(vm).map(|_| self.options.readonly)
    }

    #[pyproperty]
    fn itemsize(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.try_not_released(vm).map(|_| self.options.itemsize)
    }

    #[pyproperty]
    fn ndim(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.try_not_released(vm).map(|_| self.options.ndim)
    }

    #[pyproperty]
    fn format(&self, vm: &VirtualMachine) -> PyResult<PyStr> {
        self.try_not_released(vm)
            .map(|_| PyStr::from(&self.options.format))
    }

    // translate the slice index to memory index
    fn get_pos(&self, i: isize) -> Option<usize> {
        let len = self.options.len;
        let itemsize = self.options.itemsize;
        let i = wrap_index(i, len)?;
        let i = if self.step < 0 {
            self.stop - itemsize - (-self.step as usize) * i * itemsize
        } else {
            self.start + self.step as usize * i * itemsize
        };
        Some(i)
    }

    fn getitem_by_idx(zelf: PyRef<Self>, i: isize, vm: &VirtualMachine) -> PyResult {
        let i = zelf
            .get_pos(i)
            .ok_or_else(|| vm.new_index_error("index out of range".to_owned()))?;
        zelf.format_spec
            .unpack(&zelf.obj_bytes()[i..i + zelf.options.itemsize], vm)
            .map(|x| {
                if x.len() == 1 {
                    x.fast_getitem(0)
                } else {
                    x.into_object()
                }
            })
    }

    fn getitem_by_slice(zelf: PyRef<Self>, slice: PySliceRef, vm: &VirtualMachine) -> PyResult {
        // slicing a memoryview return a new memoryview
        let start = slice.start_index(vm)?;
        let stop = slice.stop_index(vm)?;
        let step = slice.step_index(vm)?.unwrap_or_else(BigInt::one);
        if step.is_zero() {
            return Err(vm.new_value_error("slice step cannot be zero".to_owned()));
        }
        let newstep: BigInt = step.clone() * zelf.step;
        let len = zelf.options.len;
        let itemsize = zelf.options.itemsize;

        let obj = zelf.obj.clone();
        let buffer = BufferRef(Box::new(zelf.clone()));
        zelf.exports.fetch_add(1);
        let options = zelf.options.clone();
        let format_spec = zelf.format_spec.clone();

        if newstep == BigInt::one() {
            let range = saturate_range(&start, &stop, len);
            let range = if range.end < range.start {
                range.start..range.start
            } else {
                range
            };
            let newlen = range.end - range.start;
            let start = zelf.start + range.start * itemsize;
            let stop = zelf.start + range.end * itemsize;
            return Ok(PyMemoryView {
                obj,
                buffer,
                options: BufferOptions {
                    len: newlen,
                    contiguous: true,
                    ..options
                },
                released: AtomicCell::new(false),
                start,
                stop,
                step: 1,
                exports: AtomicCell::new(0),
                format_spec,
            }
            .into_object(vm));
        }

        let (start, stop) = if step.is_negative() {
            (
                stop.map(|x| {
                    if x == -BigInt::one() {
                        len + BigInt::one()
                    } else {
                        x + 1
                    }
                }),
                start.map(|x| {
                    if x == -BigInt::one() {
                        BigInt::from(len)
                    } else {
                        x + 1
                    }
                }),
            )
        } else {
            (start, stop)
        };

        let range = saturate_range(&start, &stop, len);
        let newlen = if range.end > range.start {
            // overflow is not possible as dividing a positive integer
            ((range.end - range.start - 1) / step.abs())
                .to_usize()
                .unwrap()
                + 1
        } else {
            return Ok(PyMemoryView {
                obj,
                buffer,
                options: BufferOptions {
                    len: 0,
                    contiguous: true,
                    ..options
                },
                released: AtomicCell::new(false),
                start: range.end,
                stop: range.end,
                step: 1,
                exports: AtomicCell::new(0),
                format_spec,
            }
            .into_object(vm));
        };

        // newlen will be 0 if step is overflowed
        let newstep = newstep.to_isize().unwrap_or(-1);

        let (start, stop) = if newstep < 0 {
            let stop = zelf.stop - range.start * itemsize * zelf.step.abs() as usize;
            let start = stop - (newlen - 1) * itemsize * newstep.abs() as usize - itemsize;
            (start, stop)
        } else {
            let start = zelf.start + range.start * itemsize * zelf.step.abs() as usize;
            let stop = start + (newlen - 1) * itemsize * newstep.abs() as usize + itemsize;
            (start, stop)
        };

        Ok(PyMemoryView {
            obj,
            buffer,
            options: BufferOptions {
                len: newlen,
                contiguous: false,
                ..options
            },
            released: AtomicCell::new(false),
            start,
            stop,
            step: newstep,
            exports: AtomicCell::new(0),
            format_spec,
        }
        .into_object(vm))
    }

    #[pymethod(magic)]
    fn getitem(zelf: PyRef<Self>, needle: SequenceIndex, vm: &VirtualMachine) -> PyResult {
        zelf.try_not_released(vm)?;
        match needle {
            SequenceIndex::Int(i) => Self::getitem_by_idx(zelf, i, vm),
            SequenceIndex::Slice(slice) => Self::getitem_by_slice(zelf, slice, vm),
        }
    }

    #[pymethod(magic)]
    fn len(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.try_not_released(vm).map(|_| self.options.len)
    }

    #[pymethod]
    fn tobytes(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
        zelf.try_not_released(vm)?;
        if let Some(bytes) = zelf.as_contiguous() {
            Ok(PyBytes::from(bytes.to_vec()).into_ref(vm))
        } else {
            let bytes = &*zelf.obj_bytes();
            let bytes = (0..zelf.options.len)
                .map(|i| zelf.get_pos(i as isize).unwrap())
                .flat_map(|i| (i..i + zelf.options.itemsize).map(|i| bytes[i]))
                .collect::<Vec<u8>>();
            Ok(PyBytes::from(bytes).into_ref(vm))
        }
    }

    #[pymethod]
    fn tolist(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyListRef> {
        zelf.try_not_released(vm)?;

        let bytes = &*zelf.obj_bytes();
        let elements: Vec<PyObjectRef> = (0..zelf.options.len)
            .map(|i| zelf.get_pos(i as isize).unwrap())
            .map(|i| {
                zelf.format_spec
                    .unpack(&bytes[i..i + zelf.options.itemsize], vm)
                    .map(|x| {
                        if x.len() == 1 {
                            x.fast_getitem(0)
                        } else {
                            x.into_object()
                        }
                    })
            })
            .try_collect()?;

        Ok(PyList::from(elements).into_ref(vm))
    }

    #[pymethod]
    fn toreadonly(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyMemoryViewRef> {
        zelf.try_not_released(vm)?;
        let buffer = BufferRef(Box::new(zelf.clone()));
        Ok(PyMemoryView {
            obj: zelf.obj.clone(),
            buffer,
            options: BufferOptions {
                readonly: true,
                ..zelf.options.clone()
            },
            released: AtomicCell::new(false),
            exports: AtomicCell::new(0),
            format_spec: zelf.format_spec.clone(),
            ..*zelf
        }
        .into_ref(vm))
    }

    #[pymethod]
    fn repr(zelf: PyRef<Self>) -> String {
        if zelf.released.load() {
            format!("<released memory at 0x{:x}>", zelf.get_id())
        } else {
            format!("<memory at 0x{:x}>", zelf.get_id())
        }
    }
}

impl Drop for PyMemoryView {
    fn drop(&mut self) {
        self.release();
    }
}

impl BufferProtocol for PyMemoryView {
    fn get_buffer(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<Box<dyn Buffer>> {
        if zelf.released.load() {
            Err(vm.new_value_error("operation forbidden on released memoryview object".to_owned()))
        } else {
            Ok(Box::new(zelf))
        }
    }
}

impl Buffer for PyMemoryViewRef {
    fn get_options(&self) -> BorrowedValue<BufferOptions> {
        (&self.options).into()
    }

    fn obj_bytes(&self) -> BorrowedValue<[u8]> {
        self.buffer.obj_bytes()
    }

    fn obj_bytes_mut(&self) -> BorrowedValueMut<[u8]> {
        self.buffer.obj_bytes_mut()
    }

    fn release(&self) {
        if self.exports.fetch_sub(1) == 1 && !self.released.load() {
            self.buffer.release();
        }
    }

    fn is_resizable(&self) -> bool {
        self.buffer.is_resizable()
    }

    fn as_contiguous(&self) -> Option<BorrowedValue<[u8]>> {
        let options = self.get_options();
        if !options.contiguous {
            return None;
        }
        Some(BorrowedValue::map(self.obj_bytes(), |x| {
            &x[self.start..self.stop]
        }))
    }

    fn as_contiguous_mut(&self) -> Option<BorrowedValueMut<[u8]>> {
        let options = self.get_options();
        if !options.contiguous {
            return None;
        }
        Some(BorrowedValueMut::map(self.obj_bytes_mut(), |x| {
            &mut x[self.start..self.stop]
        }))
    }
}

impl Comparable for PyMemoryView {
    fn cmp(
        zelf: &PyRef<Self>,
        other: &PyObjectRef,
        op: crate::slots::PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        op.eq_only(|| {
            zelf.try_not_released(vm)?;
            if zelf.is(other) {
                return Ok(PyComparisonValue::Implemented(true));
            }
            let options_cmp = |a: &BufferOptions, b: &BufferOptions| -> bool {
                a.len == b.len && a.itemsize == b.itemsize
            };
            // TODO: fast pass for contiguous buffer
            match other.clone().downcast::<PyMemoryView>() {
                Ok(other) => {
                    if options_cmp(&zelf.options, &other.options) {
                        let a = Self::tolist(zelf.clone(), vm)?;
                        let b = Self::tolist(other, vm)?;
                        if vm.bool_eq(a.as_object(), b.as_object())? {
                            return Ok(PyComparisonValue::Implemented(true));
                        }
                    }
                }
                Err(other) => {
                    if let Ok(buffer) = try_buffer_from_object(other, vm) {
                        let options = buffer.get_options();
                        // FIXME
                        if options_cmp(&zelf.options, &options)
                            && (**(Self::tobytes(zelf.clone(), vm)?) == *buffer.obj_bytes())
                        {
                            return Ok(PyComparisonValue::Implemented(true));
                        }
                    }
                }
            }
            Ok(PyComparisonValue::Implemented(false))
        })
    }
}

impl Hashable for PyMemoryView {
    fn hash(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        vm._hash(&zelf.obj)
    }
}

impl PyValue for PyMemoryView {
    fn class(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.types.memoryview_type.clone()
    }
}

pub(crate) fn init(ctx: &PyContext) {
    PyMemoryView::extend_class(ctx, &ctx.types.memoryview_type)
}

pub fn try_buffer_from_object(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<BufferRef> {
    let obj_cls = obj.class();
    obj_cls
        .slots
        .buffer
        .as_ref()
        .and_then(|buffer_func| buffer_func(obj, vm).ok().map(|x| BufferRef(x)))
        .ok_or_else(|| {
            vm.new_type_error(format!(
                "memoryview: a bytes-like object is required, not '{}'",
                obj_cls.name
            ))
        })
}
