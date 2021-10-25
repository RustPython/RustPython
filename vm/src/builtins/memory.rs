use super::{PyBytes, PyBytesRef, PyList, PyListRef, PySlice, PyStr, PyStrRef, PyTypeRef};
use crate::common::{
    borrow::{BorrowedValue, BorrowedValueMut},
    hash::PyHash,
    lock::OnceCell,
};
use crate::{
    bytesinner::bytes_to_hex,
    function::{FuncArgs, IntoPyObject, OptionalArg},
    protocol::{BufferMethods, BufferOptions, PyBuffer, PyMappingMethods},
    sliceable::{wrap_index, SaturatedSlice, SequenceIndex},
    stdlib::pystruct::FormatSpec,
    types::{AsBuffer, AsMapping, Comparable, Constructor, Hashable, PyComparisonOp},
    utils::Either,
    IdProtocol, PyClassDef, PyClassImpl, PyComparisonValue, PyContext, PyObject, PyObjectRef,
    PyObjectView, PyObjectWrap, PyRef, PyResult, PyValue, TryFromBorrowedObject, TryFromObject,
    TypeProtocol, VirtualMachine,
};
use crossbeam_utils::atomic::AtomicCell;
use itertools::Itertools;
use num_traits::ToPrimitive;
use std::fmt::Debug;

#[derive(FromArgs)]
pub struct PyMemoryViewNewArgs {
    object: PyObjectRef,
}

#[pyclass(module = false, name = "memoryview")]
#[derive(Debug)]
pub struct PyMemoryView {
    buffer: PyBuffer,
    // the released memoryview does not mean the buffer is destoryed
    // because the possible another memeoryview is viewing from it
    released: AtomicCell<bool>,
    // start should always less or equal to the stop
    // start and stop pointing to the memory index not slice index
    // if length is not zero than [start, stop)
    start: usize,
    stop: usize,
    // step can be negative, means read the memory from stop-1 to start
    // the memoryview is not contiguous if the buffer is not contiguous
    // or the step is not 1 or -1
    step: isize,
    format_spec: FormatSpec,
    // memoryview's options could be different from buffer's options
    options: BufferOptions,
    hash: OnceCell<PyHash>,
    // exports
    // memoryview has no exports count by itself
    // instead it relay on the buffer it viewing to maintain the count
}

impl Constructor for PyMemoryView {
    type Args = PyMemoryViewNewArgs;

    fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        let buffer = PyBuffer::try_from_borrowed_object(vm, &args.object)?;
        let zelf = PyMemoryView::from_buffer(buffer, vm)?;
        zelf.into_pyresult_with_type(vm, cls)
    }
}

#[pyimpl(with(Hashable, Comparable, AsBuffer, AsMapping, Constructor))]
impl PyMemoryView {
    #[cfg(debug_assertions)]
    fn validate(self) -> Self {
        let options = &self.options;
        let bytes_len = options.len * options.itemsize
            + (self.step.abs() as usize - 1) * options.itemsize * options.len.saturating_sub(1);
        let buffer_len = self.buffer.obj_bytes().len();
        assert!(self.start <= self.stop);
        assert!(self.step != 0);
        assert!(self.start + bytes_len <= buffer_len);
        assert!(self.stop <= buffer_len);
        assert!(self.stop - self.start == bytes_len);
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
        let options = buffer.options.clone();
        let stop = options.len * options.itemsize;
        let format_spec = Self::parse_format(&options.format, vm)?;
        Ok(PyMemoryView {
            buffer,
            released: AtomicCell::new(false),
            start: 0,
            stop,
            step: 1,
            options,
            format_spec,
            hash: OnceCell::new(),
        }
        .validate())
    }

    pub fn from_buffer_range(
        buffer: PyBuffer,
        range: std::ops::Range<usize>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        let options = buffer.options.clone();
        let itemsize = options.itemsize;
        let format_spec = Self::parse_format(&options.format, vm)?;
        Ok(PyMemoryView {
            buffer,
            released: AtomicCell::new(false),
            start: range.start * itemsize,
            stop: range.end * itemsize,
            step: 1,
            options: BufferOptions {
                len: range.len(),
                ..options
            },
            format_spec,
            hash: OnceCell::new(),
        }
        .validate())
    }

    #[pymethod]
    pub fn release(&self) {
        if self.released.compare_exchange(false, true).is_ok() {
            self.buffer.manually_release = true;
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
        self.try_not_released(vm).map(|_| self.buffer.obj.clone())
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
            .map(|_| PyStr::from(self.options.format.clone()))
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
        let i = wrap_index(i, len)?;
        Some(self.get_pos_no_wrap(i))
    }

    fn get_pos_no_wrap(&self, i: usize) -> usize {
        let itemsize = self.options.itemsize;
        if self.step < 0 {
            self.stop - itemsize - (-self.step as usize) * i * itemsize
        } else {
            self.start + self.step as usize * i * itemsize
        }
    }

    fn getitem_by_idx(zelf: PyRef<Self>, i: isize, vm: &VirtualMachine) -> PyResult {
        let i = zelf
            .get_pos(i)
            .ok_or_else(|| vm.new_index_error("index out of range".to_owned()))?;
        let bytes = &zelf.buffer.obj_bytes()[i..i + zelf.options.itemsize];
        zelf.format_spec.unpack(bytes, vm).map(|x| {
            if x.len() == 1 {
                x.fast_getitem(0)
            } else {
                x.into()
            }
        })
    }

    fn getitem_by_slice(zelf: PyRef<Self>, slice: PyRef<PySlice>, vm: &VirtualMachine) -> PyResult {
        // slicing a memoryview return a new memoryview
        let options = zelf.options.clone();
        let buffer = zelf.buffer.clone();

        let len = options.len;
        let (range, step, is_negative_step) =
            SaturatedSlice::with_slice(&slice, vm)?.adjust_indices(len);
        let abs_step = step.unwrap();
        let step = if is_negative_step {
            -(abs_step as isize)
        } else {
            abs_step as isize
        };
        let newstep = step * zelf.step;
        let itemsize = options.itemsize;

        let format_spec = zelf.format_spec.clone();

        if newstep == 1 {
            let newlen = range.end - range.start;
            let start = zelf.start + range.start * itemsize;
            let stop = zelf.start + range.end * itemsize;
            return Ok(PyMemoryView {
                buffer,
                released: AtomicCell::new(false),
                start,
                stop,
                step: 1,
                options: BufferOptions {
                    len: newlen,
                    contiguous: true,
                    ..options
                },
                format_spec,
                hash: OnceCell::new(),
            }
            .validate()
            .into_object(vm));
        }

        if range.start >= range.end {
            return Ok(PyMemoryView {
                buffer,
                released: AtomicCell::new(false),
                start: range.end,
                stop: range.end,
                step: 1,
                options: BufferOptions {
                    len: 0,
                    contiguous: true,
                    ..options
                },
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

        Ok(PyMemoryView {
            buffer,
            released: AtomicCell::new(false),
            start,
            stop,
            step: newstep,
            options: BufferOptions {
                len: newlen,
                contiguous: false,
                ..options
            },
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
        let itemsize = zelf.options.itemsize;
        let data = zelf.format_spec.pack(vec![value], vm)?;
        zelf.buffer.obj_bytes_mut()[i..i + itemsize].copy_from_slice(&data);
        Ok(())
    }

    fn setitem_by_slice(
        zelf: PyRef<Self>,
        slice: PyRef<PySlice>,
        items: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let items = PyBuffer::try_from_object(vm, items)?;
        let options = &items.options;
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

        let (range, step, is_negative_step) =
            SaturatedSlice::with_slice(&slice, vm)?.adjust_indices(zelf.options.len);

        // TODO: try borrow the vec, now cause deadlock
        // items.contiguous_or_collect(|bytes| {
        let mut bytes = vec![];
        items.collect_bytes(&mut bytes);
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

            let mut buffer = zelf.buffer.obj_bytes_mut();

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
                    let mut buffer = zelf.buffer.obj_bytes_mut();
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
        // })
    }

    #[pymethod(magic)]
    fn setitem(
        zelf: PyRef<Self>,
        needle: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        zelf.try_not_released(vm)?;
        if zelf.options.readonly {
            return Err(vm.new_type_error("cannot modify read-only memory".to_owned()));
        }
        match SequenceIndex::try_from_object_for(vm, needle, Self::NAME)? {
            SequenceIndex::Int(index) => Self::setitem_by_idx(zelf, index, value, vm),
            SequenceIndex::Slice(slice) => Self::setitem_by_slice(zelf, slice, value, vm),
        }
    }

    #[pymethod(magic)]
    fn len(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.try_not_released(vm).map(|_| self.options.len)
    }

    #[pymethod]
    fn tobytes(&self, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
        self.try_not_released(vm)?;
        let v = if self.options.contiguous {
            self.contiguous().to_vec()
        } else {
            let mut collected = vec![];
            self.collect_bytes(&mut collected);
            collected
        };
        Ok(PyBytes::from(v).into_ref(vm))
    }

    #[pymethod]
    fn tolist(&self, vm: &VirtualMachine) -> PyResult<PyListRef> {
        self.try_not_released(vm)?;
        let len = self.options.len;
        let itemsize = self.options.itemsize;
        let bytes = &*self.buffer.obj_bytes();
        let elements: Vec<PyObjectRef> = (0..len)
            .map(|i| {
                let i = self.get_pos_no_wrap(i);
                format_unpack(&self.format_spec, &bytes[i..i + itemsize], vm)
            })
            .try_collect()?;

        Ok(vm.ctx.new_list(elements))
    }

    #[pymethod]
    fn toreadonly(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.try_not_released(vm)?;
        Ok(PyMemoryView {
            buffer: zelf.buffer.clone(),
            released: AtomicCell::new(false),
            format_spec: zelf.format_spec.clone(),
            hash: OnceCell::new(),
            options: BufferOptions {
                readonly: true,
                ..zelf.options.clone()
            },
            ..**zelf
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
        &self,
        sep: OptionalArg<Either<PyStrRef, PyBytesRef>>,
        bytes_per_sep: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        self.try_not_released(vm)?;
        self.contiguous_or_collect(|x| bytes_to_hex(x, sep, bytes_per_sep, vm))
    }

    // TODO: support cast shape
    #[pymethod]
    fn cast(&self, format: PyStrRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        self.try_not_released(vm)?;
        if !self.options.contiguous {
            return Err(vm.new_type_error(
                "memoryview: casts are restricted to C-contiguous views".to_owned(),
            ));
        }

        let format_spec = Self::parse_format(format.as_str(), vm)?;
        let itemsize = format_spec.size();
        let bytelen = self.options.len * self.options.itemsize;

        if bytelen % itemsize != 0 {
            return Err(
                vm.new_type_error("memoryview: length is not a multiple of itemsize".to_owned())
            );
        }

        Ok(PyMemoryView {
            buffer: self.buffer.clone(),
            released: AtomicCell::new(false),
            options: BufferOptions {
                itemsize,
                len: bytelen / itemsize,
                format: format.to_string().into(),
                ..self.options.clone()
            },
            format_spec,
            hash: OnceCell::new(),
            ..*self
        }
        .validate()
        .into_ref(vm))
    }

    fn eq(
        zelf: &crate::PyObjectView<Self>,
        other: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
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

        let a_options = &zelf.options;
        let b_options = &other.options;

        if a_options.len != b_options.len
            || a_options.ndim != b_options.ndim
            || a_options.shape != b_options.shape
        {
            return Ok(false);
        }

        zelf.contiguous_or_collect(|a| {
            other.contiguous_or_collect(|b| {
                if a_options.format == b_options.format {
                    Ok(a == b)
                } else {
                    let a_list = unpack_bytes_seq_to_list(a, &a_options.format, vm)?;
                    let b_list = unpack_bytes_seq_to_list(b, &b_options.format, vm)?;

                    Ok(vm.bool_eq(a_list.as_object(), b_list.as_object())?)
                }
            })
        })
    }

    #[pymethod(magic)]
    fn reduce_ex(zelf: PyRef<Self>, _proto: usize, vm: &VirtualMachine) -> PyResult {
        Self::reduce(zelf, vm)
    }

    #[pymethod(magic)]
    fn reduce(_zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error("cannot pickle 'memoryview' object".to_owned()))
    }

    fn as_contiguous_mut(&self) -> Option<BorrowedValueMut<[u8]>> {
        self.options.contiguous.then(|| self.contiguous_mut())
    }

    fn contiguous_or_collect<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        let borrowed;
        let mut collected;
        let v = if self.options.contiguous {
            borrowed = self.contiguous();
            &*borrowed
        } else {
            collected = vec![];
            self.collect_bytes(&mut collected);
            &*collected
        };
        f(v)
    }

    fn contiguous(&self) -> BorrowedValue<[u8]> {
        BorrowedValue::map(self.buffer._contiguous(), |x| &x[self.start..self.stop])
    }
    fn contiguous_mut(&self) -> BorrowedValueMut<[u8]> {
        BorrowedValueMut::map(self.buffer._contiguous_mut(), |x| {
            &mut x[self.start..self.stop]
        })
    }
    fn collect_bytes(&self, buf: &mut Vec<u8>) {
        let bytes = &*self.buffer.obj_bytes();
        let len = self.options.len;
        let itemsize = self.options.itemsize;
        buf.reserve(len * itemsize);
        for i in 0..len {
            let i = self.get_pos_no_wrap(i);
            buf.extend_from_slice(&bytes[i..i + itemsize]);
        }
    }
}

static BUFFER_METHODS: BufferMethods = BufferMethods {
    obj_bytes: |buffer| buffer.obj_as::<PyMemoryView>().buffer.obj_bytes(),
    obj_bytes_mut: |buffer| buffer.obj_as::<PyMemoryView>().buffer.obj_bytes_mut(),
    contiguous: Some(|buffer| buffer.obj_as::<PyMemoryView>().contiguous()),
    contiguous_mut: Some(|buffer| buffer.obj_as::<PyMemoryView>().contiguous_mut()),
    collect_bytes: Some(|buffer, buf| buffer.obj_as::<PyMemoryView>().collect_bytes(buf)),
    release: Some(|buffer| buffer.obj_as::<PyMemoryView>().buffer.release()),
    retain: Some(|buffer| buffer.obj_as::<PyMemoryView>().buffer.retain()),
};

impl AsBuffer for PyMemoryView {
    fn as_buffer(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyBuffer> {
        if zelf.released.load() {
            Err(vm.new_value_error("operation forbidden on released memoryview object".to_owned()))
        } else {
            Ok(PyBuffer::new(
                zelf.to_owned().into_object(),
                zelf.options.clone(),
                &BUFFER_METHODS,
            ))
        }
    }
}

impl AsMapping for PyMemoryView {
    fn as_mapping(_zelf: &PyObjectView<Self>, _vm: &VirtualMachine) -> PyMappingMethods {
        PyMappingMethods {
            length: Some(Self::length),
            subscript: Some(Self::subscript),
            ass_subscript: Some(Self::ass_subscript),
        }
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
        zelf: &crate::PyObjectView<Self>,
        other: &PyObject,
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
    fn hash(zelf: &crate::PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        zelf.hash
            .get_or_try_init(|| {
                zelf.try_not_released(vm)?;
                if !zelf.options.readonly {
                    return Err(
                        vm.new_value_error("cannot hash writable memoryview object".to_owned())
                    );
                }
                Ok(zelf.contiguous_or_collect(|bytes| vm.state.hash_secret.hash_bytes(bytes)))
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

fn unpack_bytes_seq_to_list(
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
