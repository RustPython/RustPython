use super::{PyBytes, PyBytesRef, PyList, PyListRef, PySliceRef, PyStr, PyStrRef, PyTypeRef};
use crate::common::{
    borrow::{BorrowedValue, BorrowedValueMut},
    hash::PyHash,
    lock::OnceCell,
    rc::PyRc,
};
use crate::{
    bytesinner::bytes_to_hex,
    function::{FuncArgs, IntoPyObject, OptionalArg},
    protocol::{BufferInternal, BufferOptions, PyBuffer, PyMappingMethods},
    sliceable::{convert_slice, wrap_index, SequenceIndex},
    slots::{AsBuffer, AsMapping, Comparable, Hashable, PyComparisonOp, SlotConstructor},
    stdlib::pystruct::FormatSpec,
    utils::Either,
    IdProtocol, PyClassDef, PyClassImpl, PyComparisonValue, PyContext, PyObjectRef, PyRef,
    PyResult, PyValue, TryFromBorrowedObject, TryFromObject, TypeProtocol, VirtualMachine,
};
use crossbeam_utils::atomic::AtomicCell;
use itertools::Itertools;
use num_traits::ToPrimitive;
use std::fmt::Debug;
use std::ops::Deref;

#[derive(FromArgs)]
pub struct PyMemoryViewNewArgs {
    object: PyObjectRef,
}

#[pyclass(module = false, name = "memoryview")]
#[derive(Debug)]
pub struct PyMemoryView {
    buffer: PyBuffer,
    released: AtomicCell<bool>,
    // start should always less or equal to the stop
    // start and stop pointing to the memory index not slice index
    // if length is not zero than [start, stop)
    start: usize,
    stop: usize,
    // step can be negative, means read the memory from stop-1 to start
    step: isize,
    format_spec: FormatSpec,
    hash: OnceCell<PyHash>,
}

impl SlotConstructor for PyMemoryView {
    type Args = PyMemoryViewNewArgs;

    fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        let buffer = PyBuffer::try_from_borrowed_object(vm, &args.object)?;
        let zelf = PyMemoryView::from_buffer(buffer, vm)?;
        zelf.into_pyresult_with_type(vm, cls)
    }
}

#[pyimpl(with(Hashable, Comparable, AsBuffer, AsMapping, SlotConstructor))]
impl PyMemoryView {
    #[cfg(debug_assertions)]
    fn validate(self) -> Self {
        let options = &self.buffer.options;
        let bytes_len = options.len * options.itemsize;
        let buffer_len = self.buffer.internal.obj_bytes().len();
        let t1 = self.stop - self.start == bytes_len;
        let t2 = buffer_len >= self.stop;
        let t3 = buffer_len >= self.start + bytes_len;
        assert!(t1);
        assert!(t2);
        assert!(t3);
        self
    }
    #[cfg(not(debug_assertions))]
    fn validate(self) -> Self {
        self
    }

    fn parse_format(format: &str, vm: &VirtualMachine) -> PyResult<FormatSpec> {
        FormatSpec::parse(format.as_bytes(), vm)
    }

    pub fn from_buffer(buffer: PyBuffer, vm: &VirtualMachine) -> PyResult<Self> {
        // when we get a buffer means the buffered object is size locked
        // so we can assume the buffer's options will never change as long
        // as memoryview is still alive
        let options = &buffer.options;
        let stop = options.len * options.itemsize;
        let format_spec = Self::parse_format(&options.format, vm)?;
        Ok(PyMemoryView {
            buffer,
            released: AtomicCell::new(false),
            start: 0,
            stop,
            step: 1,
            format_spec,
            hash: OnceCell::new(),
        }
        .validate())
    }

    pub fn from_buffer_range(
        mut buffer: PyBuffer,
        range: std::ops::Range<usize>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        let itemsize = buffer.options.itemsize;
        let format_spec = Self::parse_format(&buffer.options.format, vm)?;
        buffer.options.len = range.len();
        Ok(PyMemoryView {
            buffer,
            released: AtomicCell::new(false),
            start: range.start * itemsize,
            stop: range.end * itemsize,
            step: 1,
            format_spec,
            hash: OnceCell::new(),
        }
        .validate())
    }

    fn as_contiguous(&self) -> Option<BorrowedValue<[u8]>> {
        if !self.buffer.options.contiguous {
            return None;
        }
        Some(self.obj_bytes())
    }

    fn as_contiguous_mut(&self) -> Option<BorrowedValueMut<[u8]>> {
        if !self.buffer.options.contiguous {
            return None;
        }
        Some(self.obj_bytes_mut())
    }

    pub fn try_bytes<F, R>(&self, vm: &VirtualMachine, f: F) -> PyResult<R>
    where
        F: FnOnce(&[u8]) -> R,
    {
        self.try_not_released(vm)?;
        self.as_contiguous().map(|x| f(&*x)).ok_or_else(|| {
            vm.new_type_error("non-contiguous memoryview is not a bytes-like object".to_owned())
        })
    }

    #[pymethod]
    fn release(&self) {
        self._release();
    }

    fn _release(&self) {
        // avoid double release
        if self.released.compare_exchange(false, true).is_ok() {
            unsafe {
                // SAFETY: this branch is only once accessible form _release and guarded by AtomicCell released
                let buffer: &std::cell::UnsafeCell<PyBuffer> = std::mem::transmute(&self.buffer);
                let buffer = &mut *buffer.get();
                let internal = std::mem::replace(&mut buffer.internal, PyRc::new(Released));
                internal.release();
            }
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
        self.try_not_released(vm).map(|_| self.buffer.obj.clone())
    }

    #[pyproperty]
    fn nbytes(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.try_not_released(vm)
            .map(|_| self.buffer.options.len * self.buffer.options.itemsize)
    }

    #[pyproperty]
    fn readonly(&self, vm: &VirtualMachine) -> PyResult<bool> {
        self.try_not_released(vm)
            .map(|_| self.buffer.options.readonly)
    }

    #[pyproperty]
    fn itemsize(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.try_not_released(vm)
            .map(|_| self.buffer.options.itemsize)
    }

    #[pyproperty]
    fn ndim(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.try_not_released(vm).map(|_| self.buffer.options.ndim)
    }

    // TODO
    #[pyproperty]
    fn shape(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        self.try_not_released(vm)
            .map(|_| (self.buffer.options.len,).into_pyobject(vm))
    }

    // TODO
    #[pyproperty]
    fn strides(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        self.try_not_released(vm).map(|_| (0,).into_pyobject(vm))
    }

    #[pyproperty]
    fn format(&self, vm: &VirtualMachine) -> PyResult<PyStr> {
        self.try_not_released(vm)
            .map(|_| PyStr::from(&self.buffer.options.format))
    }

    #[pymethod(magic)]
    fn enter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.try_not_released(vm).map(|_| zelf)
    }

    #[pymethod(magic)]
    fn exit(&self, _args: FuncArgs) {
        self._release();
    }

    // translate the slice index to memory index
    fn get_pos(&self, i: isize) -> Option<usize> {
        let len = self.buffer.options.len;
        let itemsize = self.buffer.options.itemsize;
        let i = wrap_index(i, len)?;
        let i = if self.step < 0 {
            (len - 1 + (self.step as usize) * i) * itemsize
        } else {
            self.step as usize * i * itemsize
        };
        Some(i)
    }

    fn getitem_by_idx(zelf: PyRef<Self>, i: isize, vm: &VirtualMachine) -> PyResult {
        let i = zelf
            .get_pos(i)
            .ok_or_else(|| vm.new_index_error("index out of range".to_owned()))?;
        let bytes = &zelf.obj_bytes()[i..i + zelf.buffer.options.itemsize];
        zelf.format_spec.unpack(bytes, vm).map(|x| {
            if x.len() == 1 {
                x.fast_getitem(0)
            } else {
                x.into()
            }
        })
    }

    fn getitem_by_slice(zelf: PyRef<Self>, slice: PySliceRef, vm: &VirtualMachine) -> PyResult {
        // slicing a memoryview return a new memoryview
        let len = zelf.buffer.options.len;
        let (range, step, is_negative_step) = convert_slice(&slice, len, vm)?;
        let abs_step = step.unwrap();
        let step = if is_negative_step {
            -(abs_step as isize)
        } else {
            abs_step as isize
        };
        let newstep = step * zelf.step;
        let itemsize = zelf.buffer.options.itemsize;

        let format_spec = zelf.format_spec.clone();

        if newstep == 1 {
            let newlen = range.end - range.start;
            let start = zelf.start + range.start * itemsize;
            let stop = zelf.start + range.end * itemsize;
            return Ok(PyMemoryView {
                buffer: zelf.buffer.clone_with_options(BufferOptions {
                    len: newlen,
                    contiguous: true,
                    ..zelf.buffer.options.clone()
                }),
                released: AtomicCell::new(false),
                start,
                stop,
                step: 1,
                format_spec,
                hash: OnceCell::new(),
            }
            .validate()
            .into_object(vm));
        }

        if range.start >= range.end {
            return Ok(PyMemoryView {
                buffer: zelf.buffer.clone_with_options(BufferOptions {
                    len: 0,
                    contiguous: true,
                    ..zelf.buffer.options.clone()
                }),
                released: AtomicCell::new(false),
                start: range.end,
                stop: range.end,
                step: 1,
                format_spec,
                hash: OnceCell::new(),
            }
            .validate()
            .into_object(vm));
        };

        // overflow is not possible as dividing a positive integer
        let newlen = ((range.end - range.start - 1) / abs_step)
            .to_usize()
            .unwrap()
            + 1;

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

        let options = BufferOptions {
            len: newlen,
            contiguous: false,
            ..zelf.buffer.options.clone()
        };
        Ok(PyMemoryView {
            buffer: zelf.buffer.clone_with_options(options),
            released: AtomicCell::new(false),
            start,
            stop,
            step: newstep,
            format_spec,
            hash: OnceCell::new(),
        }
        .validate()
        .into_object(vm))
    }

    #[pymethod(magic)]
    fn getitem(zelf: PyRef<Self>, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        zelf.try_not_released(vm)?;
        match SequenceIndex::try_from_object_for(vm, needle, Self::NAME)? {
            SequenceIndex::Int(index) => Self::getitem_by_idx(zelf, index, vm),
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
        let itemsize = zelf.buffer.options.itemsize;
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
        let items = PyBuffer::try_from_object(vm, items)?;
        let options = &items.options;
        let len = options.len;
        let itemsize = options.itemsize;

        if itemsize != zelf.buffer.options.itemsize {
            return Err(vm.new_type_error(format!(
                "memoryview: invalid type for format '{}'",
                zelf.buffer.options.format
            )));
        }

        let diff_err = || {
            Err(vm.new_value_error(
                "memoryview assignment: lvalue and rvalue have different structures".to_owned(),
            ))
        };

        if options.format != zelf.buffer.options.format {
            return diff_err();
        }

        let (range, step, is_negative_step) = convert_slice(&slice, zelf.buffer.options.len, vm)?;

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
        needle: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        zelf.try_not_released(vm)?;
        if zelf.buffer.options.readonly {
            return Err(vm.new_type_error("cannot modify read-only memory".to_owned()));
        }
        match SequenceIndex::try_from_object_for(vm, needle, Self::NAME)? {
            SequenceIndex::Int(index) => Self::setitem_by_idx(zelf, index, value, vm),
            SequenceIndex::Slice(slice) => Self::setitem_by_slice(zelf, slice, value, vm),
        }
    }

    #[pymethod(magic)]
    fn len(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.try_not_released(vm).map(|_| self.buffer.options.len)
    }

    fn to_bytes_vec(zelf: &PyRef<Self>) -> Vec<u8> {
        if let Some(bytes) = zelf.as_contiguous() {
            bytes.to_vec()
        } else {
            let bytes = &*zelf.obj_bytes();
            let len = zelf.buffer.options.len;
            let itemsize = zelf.buffer.options.itemsize;
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
        let elements: Vec<PyObjectRef> = (0..zelf.buffer.options.len)
            .map(|i| zelf.get_pos(i as isize).unwrap())
            .map(|i| {
                format_unpack(
                    &zelf.format_spec,
                    &bytes[i..i + zelf.buffer.options.itemsize],
                    vm,
                )
            })
            .try_collect()?;

        Ok(PyList::from(elements).into_ref(vm))
    }

    #[pymethod]
    fn toreadonly(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.try_not_released(vm)?;
        let buffer = zelf.buffer.clone_with_options(BufferOptions {
            readonly: true,
            ..zelf.buffer.options.clone()
        });
        Ok(PyMemoryView {
            buffer,
            released: AtomicCell::new(false),
            format_spec: zelf.format_spec.clone(),
            hash: OnceCell::new(),
            ..*zelf
        }
        .validate()
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
                vec = Self::to_bytes_vec(&zelf);
                vec.as_slice()
            }
        };

        bytes_to_hex(bytes, sep, bytes_per_sep, vm)
    }

    // TODO: support cast shape
    #[pymethod]
    fn cast(zelf: PyRef<Self>, format: PyStrRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.try_not_released(vm)?;
        if !zelf.buffer.options.contiguous {
            return Err(vm.new_type_error(
                "memoryview: casts are restricted to C-contiguous views".to_owned(),
            ));
        }

        let format_spec = Self::parse_format(format.as_str(), vm)?;
        let itemsize = format_spec.size();
        let bytelen = zelf.buffer.options.len * zelf.buffer.options.itemsize;

        if bytelen % itemsize != 0 {
            return Err(
                vm.new_type_error("memoryview: length is not a multiple of itemsize".to_owned())
            );
        }

        let buffer = zelf.buffer.clone_with_options(BufferOptions {
            itemsize,
            len: bytelen / itemsize,
            format: format.to_string().into(),
            ..zelf.buffer.options.clone()
        });
        Ok(PyMemoryView {
            buffer,
            released: AtomicCell::new(false),
            format_spec,
            hash: OnceCell::new(),
            ..*zelf
        }
        .validate()
        .into_ref(vm))
    }

    fn eq(zelf: &PyRef<Self>, other: &PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        if zelf.is(other) {
            return Ok(true);
        }
        if zelf.released.load() {
            return Ok(false);
        }

        let other = match PyBuffer::try_from_borrowed_object(vm, other) {
            Ok(buf) => buf,
            Err(_) => return Ok(false),
        };

        let a_options = &zelf.buffer.options;
        let b_options = &other.options;

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
                a_vec = Self::to_bytes_vec(zelf);
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

impl AsBuffer for PyMemoryView {
    fn as_buffer(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyBuffer> {
        if zelf.released.load() {
            Err(vm.new_value_error("operation forbidden on released memoryview object".to_owned()))
        } else {
            Ok(PyBuffer::new(
                zelf.as_object().clone(),
                zelf.clone(),
                zelf.buffer.options.clone(),
            ))
        }
    }
}

#[derive(Debug)]
struct Released;
impl BufferInternal for Released {
    fn obj_bytes(&self) -> BorrowedValue<[u8]> {
        panic!();
    }

    fn obj_bytes_mut(&self) -> BorrowedValueMut<[u8]> {
        panic!();
    }

    fn release(&self) {}

    fn retain(&self) {}
}

impl BufferInternal for PyMemoryView {
    // NOTE: This impl maybe is anti-pattern. Only used for internal usage.
    fn obj_bytes(&self) -> BorrowedValue<[u8]> {
        BorrowedValue::map(self.buffer.internal.obj_bytes(), |x| {
            &x[self.start..self.stop]
        })
    }

    fn obj_bytes_mut(&self) -> BorrowedValueMut<[u8]> {
        BorrowedValueMut::map(self.buffer.internal.obj_bytes_mut(), |x| {
            &mut x[self.start..self.stop]
        })
    }

    fn release(&self) {}

    fn retain(&self) {}
}

impl BufferInternal for PyRef<PyMemoryView> {
    fn obj_bytes(&self) -> BorrowedValue<[u8]> {
        self.deref().obj_bytes()
    }
    fn obj_bytes_mut(&self) -> BorrowedValueMut<[u8]> {
        self.deref().obj_bytes_mut()
    }
    fn release(&self) {}
    fn retain(&self) {}
}

impl AsMapping for PyMemoryView {
    fn as_mapping(_zelf: &PyRef<Self>, _vm: &VirtualMachine) -> PyResult<PyMappingMethods> {
        Ok(PyMappingMethods {
            length: Some(Self::length),
            subscript: Some(Self::subscript),
            ass_subscript: Some(Self::ass_subscript),
        })
    }

    #[inline]
    fn length(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        Self::downcast_ref(&zelf, vm).map(|zelf| zelf.len(vm))?
    }

    #[inline]
    fn subscript(zelf: PyObjectRef, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Self::downcast(zelf, vm).map(|zelf| Self::getitem(zelf, needle, vm))?
    }

    #[inline]
    fn ass_subscript(
        zelf: PyObjectRef,
        needle: PyObjectRef,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match value {
            Some(value) => {
                Self::downcast(zelf, vm).map(|zelf| Self::setitem(zelf, needle, value, vm))?
            }
            None => Err(vm.new_type_error("cannot delete memory".to_owned())),
        }
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
                zelf.class().name(),
                other.class().name()
            ))),
        }
    }
}

impl Hashable for PyMemoryView {
    fn hash(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        zelf.hash
            .get_or_try_init(|| {
                zelf.try_not_released(vm)?;
                if !zelf.buffer.options.readonly {
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
                        vec = Self::to_bytes_vec(zelf);
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

fn format_unpack(
    format_spec: &FormatSpec,
    bytes: &[u8],
    vm: &VirtualMachine,
) -> PyResult<PyObjectRef> {
    format_spec.unpack(bytes, vm).map(|x| {
        if x.len() == 1 {
            x.fast_getitem(0)
        } else {
            x.into()
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
