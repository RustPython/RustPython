use std::{borrow::Cow, fmt::Debug, ops::Deref};

use crate::builtins::bytes::{PyBytes, PyBytesRef};
use crate::builtins::list::{PyList, PyListRef};
use crate::builtins::pystr::{PyStr, PyStrRef};
use crate::builtins::pytype::PyTypeRef;
use crate::builtins::slice::PySliceRef;
use crate::bytesinner::bytes_to_hex;
use crate::common::borrow::{BorrowedValue, BorrowedValueMut};
use crate::common::hash::PyHash;
use crate::common::lock::OnceCell;
use crate::common::rc::PyRc;
use crate::function::{FuncArgs, OptionalArg};
use crate::pyobject::{
    Either, IdProtocol, IntoPyObject, PyClassImpl, PyComparisonValue, PyContext, PyObjectRef,
    PyRef, PyResult, PyThreadingConstraint, PyValue, TypeProtocol,
};
use crate::sliceable::{convert_slice, saturate_range, wrap_index, SequenceIndex};
use crate::slots::{BufferProtocol, Comparable, Hashable, PyComparisonOp};
use crate::stdlib::pystruct::_struct::FormatSpec;
use crate::VirtualMachine;
use crossbeam_utils::atomic::AtomicCell;
use itertools::Itertools;
use num_bigint::BigInt;
use num_traits::{One, Signed, ToPrimitive, Zero};
use rustpython_common::borrow::BorrowValue;

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
impl BufferRef {
    pub fn new(buffer: impl Buffer + 'static) -> Self {
        Self(Box::new(buffer))
    }
    pub fn into_rcbuf(self) -> RcBuffer {
        // move self.0 out of self; BufferRef impls Drop so it's tricky
        let this = std::mem::ManuallyDrop::new(self);
        let buf_box = unsafe { std::ptr::read(&this.0) };
        RcBuffer(buf_box.into())
    }
}
impl From<Box<dyn Buffer>> for BufferRef {
    fn from(buffer: Box<dyn Buffer>) -> Self {
        BufferRef(buffer)
    }
}
#[derive(Debug, Clone)]
pub struct RcBuffer(PyRc<dyn Buffer>);
impl Deref for RcBuffer {
    type Target = dyn Buffer;
    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}
impl Drop for RcBuffer {
    fn drop(&mut self) {
        // check if this is the last rc before the inner buffer gets dropped
        if let Some(buf) = PyRc::get_mut(&mut self.0) {
            buf.release()
        }
    }
}
impl Buffer for RcBuffer {
    fn get_options(&self) -> &BufferOptions {
        self.0.get_options()
    }
    fn obj_bytes(&self) -> BorrowedValue<[u8]> {
        self.0.obj_bytes()
    }
    fn obj_bytes_mut(&self) -> BorrowedValueMut<[u8]> {
        self.0.obj_bytes_mut()
    }
    fn release(&self) {}
    fn as_contiguous(&self) -> Option<BorrowedValue<[u8]>> {
        self.0.as_contiguous()
    }
    fn as_contiguous_mut(&self) -> Option<BorrowedValueMut<[u8]>> {
        self.0.as_contiguous_mut()
    }
    fn to_contiguous(&self) -> Vec<u8> {
        self.0.to_contiguous()
    }
}

pub trait Buffer: Debug + PyThreadingConstraint {
    fn get_options(&self) -> &BufferOptions;
    /// Get the full inner buffer of this memory. You probably want [`as_contiguous()`], as
    /// `obj_bytes` doesn't take into account the range a memoryview might operate on, among other
    /// footguns.
    fn obj_bytes(&self) -> BorrowedValue<[u8]>;
    /// Get the full inner buffer of this memory, mutably. You probably want
    /// [`as_contiguous_mut()`], as `obj_bytes` doesn't take into account the range a memoryview
    /// might operate on, among other footguns.
    fn obj_bytes_mut(&self) -> BorrowedValueMut<[u8]>;
    fn release(&self);

    fn as_contiguous(&self) -> Option<BorrowedValue<[u8]>> {
        if !self.get_options().contiguous {
            return None;
        }
        Some(self.obj_bytes())
    }

    fn as_contiguous_mut(&self) -> Option<BorrowedValueMut<[u8]>> {
        if !self.get_options().contiguous {
            return None;
        }
        Some(self.obj_bytes_mut())
    }

    fn to_contiguous(&self) -> Vec<u8> {
        self.obj_bytes().to_vec()
    }
}

#[derive(Debug, Clone)]
pub struct BufferOptions {
    pub readonly: bool,
    pub len: usize,
    pub itemsize: usize,
    pub contiguous: bool,
    pub format: Cow<'static, str>,
    // TODO: support multiple dimension array
    pub ndim: usize,
    pub shape: Vec<usize>,
    pub strides: Vec<isize>,
}

impl BufferOptions {
    pub const DEFAULT: Self = BufferOptions {
        readonly: true,
        len: 0,
        itemsize: 1,
        contiguous: true,
        format: Cow::Borrowed("B"),
        ndim: 1,
        shape: Vec::new(),
        strides: Vec::new(),
    };
}

impl Default for BufferOptions {
    fn default() -> Self {
        Self::DEFAULT
    }
}

pub(crate) trait ResizeGuard<'a> {
    type Resizable: 'a;
    fn try_resizable(&'a self, vm: &VirtualMachine) -> PyResult<Self::Resizable>;
}

#[derive(FromArgs)]
struct PyMemoryViewNewArgs {
    #[pyarg(any)]
    object: PyObjectRef,
}

#[pyclass(module = false, name = "memoryview")]
#[derive(Debug)]
pub struct PyMemoryView {
    obj: PyObjectRef,
    buffer: BufferRef,
    options: BufferOptions,
    pub(crate) released: AtomicCell<bool>,
    // start should always less or equal to the stop
    // start and stop pointing to the memory index not slice index
    // if length is not zero than [start, stop)
    start: usize,
    stop: usize,
    // step can be negative, means read the memory from stop-1 to start
    step: isize,
    exports: AtomicCell<usize>,
    format_spec: FormatSpec,
    hash: OnceCell<PyHash>,
}

type PyMemoryViewRef = PyRef<PyMemoryView>;

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
        let itemsize = options.itemsize;
        let format_spec = Self::parse_format(&options.format, vm)?;
        Ok(PyMemoryView {
            obj,
            buffer,
            options,
            released: AtomicCell::new(false),
            start: 0,
            stop: len * itemsize,
            step: 1,
            exports: AtomicCell::new(0),
            format_spec,
            hash: OnceCell::new(),
        })
    }

    pub fn from_buffer_range(
        obj: PyObjectRef,
        buffer: BufferRef,
        range: std::ops::Range<usize>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        let options = buffer.get_options().clone();
        let itemsize = options.itemsize;
        let format_spec = Self::parse_format(&options.format, vm)?;
        Ok(PyMemoryView {
            obj,
            buffer,
            options,
            released: AtomicCell::new(false),
            start: range.start * itemsize,
            stop: range.end * itemsize,
            step: 1,
            exports: AtomicCell::new(0),
            format_spec,
            hash: OnceCell::new(),
        })
    }

    pub fn try_bytes<F, R>(&self, vm: &VirtualMachine, f: F) -> PyResult<R>
    where
        F: FnOnce(&[u8]) -> R,
    {
        self.try_not_released(vm)?;
        self.buffer.as_contiguous().map(|x| f(&*x)).ok_or_else(|| {
            vm.new_type_error("non-contiguous memoryview is not a bytes-like object".to_owned())
        })
    }

    #[pyslot]
    fn tp_new(
        cls: PyTypeRef,
        args: PyMemoryViewNewArgs,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        let buffer = try_buffer_from_object(vm, &args.object)?;
        let zelf = PyMemoryView::from_buffer(args.object, buffer, vm)?;
        zelf.into_ref_with_type(vm, cls)
    }

    #[pymethod]
    fn release(&self) {
        // avoid double release
        if self.released.compare_exchange(false, true).is_ok() && self.exports.load() == 0 {
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

    // TODO
    #[pyproperty]
    fn shape(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        self.try_not_released(vm)
            .map(|_| (self.options.len,).into_pyobject(vm))
    }

    // TODO
    #[pyproperty]
    fn strides(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        self.try_not_released(vm).map(|_| (0,).into_pyobject(vm))
    }

    #[pyproperty]
    fn format(&self, vm: &VirtualMachine) -> PyResult<PyStr> {
        self.try_not_released(vm)
            .map(|_| PyStr::from(&self.options.format))
    }

    #[pymethod(magic)]
    fn enter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.try_not_released(vm).map(|_| zelf)
    }

    #[pymethod(magic)]
    fn exit(&self, _args: FuncArgs) {
        self.release();
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
                hash: OnceCell::new(),
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
                hash: OnceCell::new(),
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
            hash: OnceCell::new(),
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

    fn setitem_by_idx(
        zelf: PyRef<Self>,
        i: isize,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let i = zelf
            .get_pos(i)
            .ok_or_else(|| vm.new_index_error("index out of range".to_owned()))?;
        let itemsize = zelf.options.itemsize;
        let data = zelf.format_spec.pack(vec![value], vm)?;
        zelf.obj_bytes_mut()[i..i + itemsize].copy_from_slice(&data);
        Ok(())
    }

    fn setitem_by_slice(
        zelf: PyRef<Self>,
        slice: PySliceRef,
        items: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let items = try_buffer_from_object(vm, &items)?;
        let options = items.get_options();
        let len = options.len;
        let itemsize = options.itemsize;

        if itemsize != zelf.options.itemsize {
            return Err(vm.new_type_error(format!(
                "memoryview: invalid type for format '{}'",
                zelf.options.format
            )));
        }

        let diff_err = || {
            Err(vm.new_value_error(
                "memoryview assignment: lvalue and rvalue have different structures".to_owned(),
            ))
        };

        if options.format != zelf.options.format {
            return diff_err();
        }

        let (range, step, is_negative_step) = convert_slice(&slice, zelf.options.len, vm)?;

        let bytes = items.to_contiguous();
        assert_eq!(bytes.len(), len * itemsize);

        if !is_negative_step && step == Some(1) {
            if range.end - range.start != len {
                return diff_err();
            }

            if let Some(mut buffer) = zelf.as_contiguous_mut() {
                buffer[range.start * itemsize..range.end * itemsize].copy_from_slice(&bytes);
                return Ok(());
            }
        }

        if let Some(step) = step {
            let slicelen = if range.end > range.start {
                (range.end - range.start - 1) / step + 1
            } else {
                0
            };

            if slicelen != len {
                return diff_err();
            }

            let indexes = if is_negative_step {
                itertools::Either::Left(range.rev().step_by(step))
            } else {
                itertools::Either::Right(range.step_by(step))
            };

            let item_index = (0..len).step_by(itemsize);

            let mut buffer = zelf.obj_bytes_mut();

            indexes
                .map(|i| zelf.get_pos(i as isize).unwrap())
                .zip(item_index)
                .for_each(|(i, item_i)| {
                    buffer[i..i + itemsize].copy_from_slice(&bytes[item_i..item_i + itemsize]);
                });
            Ok(())
        } else {
            let slicelen = if range.start < range.end { 1 } else { 0 };
            if match len {
                0 => slicelen == 0,
                1 => {
                    let mut buffer = zelf.obj_bytes_mut();
                    let i = zelf.get_pos(range.start as isize).unwrap();
                    buffer[i..i + itemsize].copy_from_slice(&bytes);
                    true
                }
                _ => false,
            } {
                Ok(())
            } else {
                diff_err()
            }
        }
    }

    #[pymethod(magic)]
    fn setitem(
        zelf: PyRef<Self>,
        needle: SequenceIndex,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        zelf.try_not_released(vm)?;
        if zelf.options.readonly {
            return Err(vm.new_type_error("cannot modify read-only memory".to_owned()));
        }
        match needle {
            SequenceIndex::Int(i) => Self::setitem_by_idx(zelf, i, value, vm),
            SequenceIndex::Slice(slice) => Self::setitem_by_slice(zelf, slice, value, vm),
        }
    }

    #[pymethod(magic)]
    fn len(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.try_not_released(vm).map(|_| self.options.len)
    }

    fn to_bytes_vec(zelf: &PyRef<Self>) -> Vec<u8> {
        if let Some(bytes) = zelf.as_contiguous() {
            bytes.to_vec()
        } else {
            let bytes = &*zelf.obj_bytes();
            let len = zelf.options.len;
            let itemsize = zelf.options.itemsize;
            (0..len)
                .map(|i| zelf.get_pos(i as isize).unwrap())
                .flat_map(|i| bytes[i..i + itemsize].to_vec())
                .collect()
        }
    }

    #[pymethod]
    fn tobytes(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
        zelf.try_not_released(vm)?;
        Ok(PyBytes::from(Self::to_bytes_vec(&zelf)).into_ref(vm))
    }

    #[pymethod]
    fn tolist(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyListRef> {
        zelf.try_not_released(vm)?;

        let bytes = &*zelf.obj_bytes();
        let elements: Vec<PyObjectRef> = (0..zelf.options.len)
            .map(|i| zelf.get_pos(i as isize).unwrap())
            .map(|i| format_unpack(&zelf.format_spec, &bytes[i..i + zelf.options.itemsize], vm))
            .try_collect()?;

        Ok(PyList::from(elements).into_ref(vm))
    }

    #[pymethod]
    fn toreadonly(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.try_not_released(vm)?;
        let buffer = BufferRef(Box::new(zelf.clone()));
        zelf.exports.fetch_add(1);
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
            hash: OnceCell::new(),
            ..*zelf
        }
        .into_ref(vm))
    }

    #[pymethod(magic)]
    fn repr(zelf: PyRef<Self>) -> String {
        if zelf.released.load() {
            format!("<released memory at {:#x}>", zelf.get_id())
        } else {
            format!("<memory at {:#x}>", zelf.get_id())
        }
    }

    #[pymethod]
    fn hex(
        zelf: PyRef<Self>,
        sep: OptionalArg<Either<PyStrRef, PyBytesRef>>,
        bytes_per_sep: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        zelf.try_not_released(vm)?;
        let guard;
        let vec;
        let bytes = match zelf.as_contiguous() {
            Some(bytes) => {
                guard = bytes;
                &*guard
            }
            None => {
                vec = zelf.to_contiguous();
                vec.as_slice()
            }
        };

        bytes_to_hex(bytes, sep, bytes_per_sep, vm)
    }

    // TODO: support cast shape
    #[pymethod]
    fn cast(zelf: PyRef<Self>, format: PyStrRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.try_not_released(vm)?;
        if !zelf.options.contiguous {
            return Err(vm.new_type_error(
                "memoryview: casts are restricted to C-contiguous views".to_owned(),
            ));
        }

        let format_spec = Self::parse_format(format.borrow_value(), vm)?;
        let itemsize = format_spec.size();
        let bytelen = zelf.options.len * zelf.options.itemsize;

        if bytelen % itemsize != 0 {
            return Err(
                vm.new_type_error("memoryview: length is not a multiple of itemsize".to_owned())
            );
        }

        let buffer = BufferRef(Box::new(zelf.clone()));
        zelf.exports.fetch_add(1);

        Ok(PyMemoryView {
            obj: zelf.obj.clone(),
            buffer,
            options: BufferOptions {
                itemsize,
                len: bytelen / itemsize,
                format: format.to_string().into(),
                ..zelf.options.clone()
            },
            released: AtomicCell::new(false),
            stop: zelf.stop + itemsize - zelf.options.itemsize,
            exports: AtomicCell::new(0),
            format_spec,
            hash: OnceCell::new(),
            ..*zelf
        }
        .into_ref(vm))
    }

    fn eq(zelf: &PyRef<Self>, other: &PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        if zelf.is(other) {
            return Ok(true);
        }
        if zelf.released.load() {
            return Ok(false);
        }

        let other = match try_buffer_from_object(vm, other) {
            Ok(buf) => buf,
            Err(_) => return Ok(false),
        };

        let a_options = &zelf.options;
        let b_options = other.get_options();

        if a_options.len != b_options.len
            || a_options.ndim != b_options.ndim
            || a_options.shape != b_options.shape
        {
            return Ok(false);
        }

        let a_guard;
        let a_vec;
        let a = match zelf.as_contiguous() {
            Some(bytes) => {
                a_guard = bytes;
                &*a_guard
            }
            None => {
                a_vec = zelf.to_contiguous();
                a_vec.as_slice()
            }
        };
        let b_guard;
        let b_vec;
        let b = match other.as_contiguous() {
            Some(bytes) => {
                b_guard = bytes;
                &*b_guard
            }
            None => {
                b_vec = other.to_contiguous();
                b_vec.as_slice()
            }
        };

        if a_options.format == b_options.format {
            Ok(a == b)
        } else {
            let a_list = unpack_bytes_seq_to_list(a, &a_options.format, vm)?;
            let b_list = unpack_bytes_seq_to_list(b, &b_options.format, vm)?;

            Ok(vm.bool_eq(a_list.as_object(), b_list.as_object())?)
        }
    }

    #[pymethod(magic)]
    fn reduce_ex(zelf: PyRef<Self>, _proto: usize, vm: &VirtualMachine) -> PyResult {
        Self::reduce(zelf, vm)
    }

    #[pymethod(magic)]
    fn reduce(_zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error("cannot pickle 'memoryview' object".to_owned()))
    }
}

impl Drop for PyMemoryView {
    fn drop(&mut self) {
        self.release();
    }
}

impl BufferProtocol for PyMemoryView {
    fn get_buffer(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<Box<dyn Buffer>> {
        if zelf.released.load() {
            Err(vm.new_value_error("operation forbidden on released memoryview object".to_owned()))
        } else {
            Ok(Box::new(zelf.clone()))
        }
    }
}

impl Buffer for PyMemoryViewRef {
    fn get_options(&self) -> &BufferOptions {
        &self.options
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

    fn as_contiguous(&self) -> Option<BorrowedValue<[u8]>> {
        if !self.options.contiguous {
            return None;
        }
        Some(BorrowedValue::map(self.obj_bytes(), |x| {
            &x[self.start..self.stop]
        }))
    }

    fn as_contiguous_mut(&self) -> Option<BorrowedValueMut<[u8]>> {
        if !self.options.contiguous {
            return None;
        }
        Some(BorrowedValueMut::map(self.obj_bytes_mut(), |x| {
            &mut x[self.start..self.stop]
        }))
    }

    fn to_contiguous(&self) -> Vec<u8> {
        PyMemoryView::to_bytes_vec(self)
    }
}

impl Comparable for PyMemoryView {
    fn cmp(
        zelf: &PyRef<Self>,
        other: &PyObjectRef,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        match op {
            PyComparisonOp::Ne => {
                Self::eq(zelf, other, vm).map(|x| PyComparisonValue::Implemented(!x))
            }
            PyComparisonOp::Eq => Self::eq(zelf, other, vm).map(PyComparisonValue::Implemented),
            _ => Err(vm.new_type_error(format!(
                "'{}' not supported between instances of '{}' and '{}'",
                op.operator_token(),
                zelf.class().name,
                other.class().name
            ))),
        }
    }
}

impl Hashable for PyMemoryView {
    fn hash(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        zelf.hash
            .get_or_try_init(|| {
                zelf.try_not_released(vm)?;
                if !zelf.options.readonly {
                    return Err(
                        vm.new_value_error("cannot hash writable memoryview object".to_owned())
                    );
                }
                let guard;
                let vec;
                let bytes = match zelf.as_contiguous() {
                    Some(bytes) => {
                        guard = bytes;
                        &*guard
                    }
                    None => {
                        vec = zelf.to_contiguous();
                        vec.as_slice()
                    }
                };
                Ok(vm.state.hash_secret.hash_bytes(bytes))
            })
            .map(|&x| x)
    }
}

impl PyValue for PyMemoryView {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.memoryview_type
    }
}

pub(crate) fn init(ctx: &PyContext) {
    PyMemoryView::extend_class(ctx, &ctx.types.memoryview_type)
}

pub fn try_buffer_from_object(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult<BufferRef> {
    let obj_cls = obj.class();
    for cls in obj_cls.iter_mro() {
        if let Some(f) = cls.slots.buffer.as_ref() {
            return f(obj, vm).map(|x| BufferRef(x));
        }
    }
    Err(vm.new_type_error(format!(
        "a bytes-like object is required, not '{}'",
        obj_cls.name
    )))
}

fn format_unpack(
    format_spec: &FormatSpec,
    bytes: &[u8],
    vm: &VirtualMachine,
) -> PyResult<PyObjectRef> {
    format_spec.unpack(bytes, vm).map(|x| {
        if x.len() == 1 {
            x.fast_getitem(0)
        } else {
            x.into_object()
        }
    })
}

pub fn unpack_bytes_seq_to_list(
    bytes: &[u8],
    format: &str,
    vm: &VirtualMachine,
) -> PyResult<PyListRef> {
    let format_spec = PyMemoryView::parse_format(format, vm)?;
    let itemsize = format_spec.size();

    if bytes.len() % itemsize != 0 {
        return Err(vm.new_value_error("bytes length not a multiple of item size".to_owned()));
    }

    let len = bytes.len() / itemsize;

    let elements: Vec<PyObjectRef> = (0..len)
        .map(|i| format_unpack(&format_spec, &bytes[i..i + itemsize], vm))
        .try_collect()?;

    Ok(PyList::from(elements).into_ref(vm))
}
