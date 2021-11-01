use super::{
    PyBytes, PyBytesRef, PyList, PyListRef, PySlice, PyStr, PyStrRef, PyTuple, PyTupleRef,
    PyTypeRef,
};
use crate::common::{
    borrow::{BorrowedValue, BorrowedValueMut},
    hash::PyHash,
    lock::OnceCell,
};
use crate::{
    bytesinner::bytes_to_hex,
    function::{FuncArgs, IntoPyObject, OptionalArg},
    protocol::{BufferDescriptor, BufferMethods, PyBuffer, PyMappingMethods},
    sliceable::{wrap_index, SequenceIndex},
    stdlib::pystruct::FormatSpec,
    types::{AsBuffer, AsMapping, Comparable, Constructor, Hashable, PyComparisonOp},
    utils::Either,
    IdProtocol, PyClassDef, PyClassImpl, PyComparisonValue, PyContext, PyObject, PyObjectRef,
    PyObjectView, PyObjectWrap, PyRef, PyResult, PyValue, TryFromBorrowedObject, TryFromObject,
    TypeProtocol, VirtualMachine,
};
use crossbeam_utils::atomic::AtomicCell;
use itertools::Itertools;
use std::{cmp::Ordering, fmt::Debug, mem::ManuallyDrop, ops::Range};

#[derive(FromArgs)]
pub struct PyMemoryViewNewArgs {
    object: PyObjectRef,
}

#[pyclass(module = false, name = "memoryview")]
#[derive(Debug)]
pub struct PyMemoryView {
    // avoid double release when memoryview had released the buffer before drop
    buffer: ManuallyDrop<PyBuffer>,
    // the released memoryview does not mean the buffer is destoryed
    // because the possible another memeoryview is viewing from it
    released: AtomicCell<bool>,
    start: usize,
    format_spec: FormatSpec,
    // memoryview's options could be different from buffer's options
    desc: BufferDescriptor,
    hash: OnceCell<PyHash>,
    // exports
    // memoryview has no exports count by itself
    // instead it relay on the buffer it viewing to maintain the count
}

impl Constructor for PyMemoryView {
    type Args = PyMemoryViewNewArgs;

    fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        let zelf = if let Some(other) = args.object.payload::<Self>() {
            other.new_view()
        } else {
            let buffer = PyBuffer::try_from_borrowed_object(vm, &args.object)?;
            PyMemoryView::from_buffer(buffer, vm)?
        };
        zelf.into_pyresult_with_type(vm, cls)
    }
}

#[pyimpl(with(Hashable, Comparable, AsBuffer, AsMapping, Constructor))]
impl PyMemoryView {
    fn parse_format(format: &str, vm: &VirtualMachine) -> PyResult<FormatSpec> {
        FormatSpec::parse(format.as_bytes(), vm)
    }

    pub fn from_buffer(buffer: PyBuffer, vm: &VirtualMachine) -> PyResult<Self> {
        // when we get a buffer means the buffered object is size locked
        // so we can assume the buffer's options will never change as long
        // as memoryview is still alive
        let format_spec = Self::parse_format(&buffer.desc.format, vm)?;
        let desc = buffer.desc.clone();

        Ok(PyMemoryView {
            buffer: ManuallyDrop::new(buffer),
            released: AtomicCell::new(false),
            start: 0,
            format_spec,
            desc,
            hash: OnceCell::new(),
        })
    }

    pub fn from_buffer_range(
        buffer: PyBuffer,
        range: Range<usize>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        let format_spec = Self::parse_format(&buffer.desc.format, vm)?;
        let desc = buffer.desc.clone();

        let mut zelf = PyMemoryView {
            buffer: ManuallyDrop::new(buffer),
            released: AtomicCell::new(false),
            start: range.start,
            format_spec,
            desc,
            hash: OnceCell::new(),
        };

        zelf.init_range(range, 0);
        zelf.init_len();
        Ok(zelf)
    }

    fn new_view(&self) -> Self {
        let zelf = PyMemoryView {
            buffer: self.buffer.clone(),
            released: AtomicCell::new(false),
            start: self.start,
            format_spec: self.format_spec.clone(),
            desc: self.desc.clone(),
            hash: OnceCell::new(),
        };
        zelf.buffer.retain();
        zelf
    }

    #[pymethod]
    pub fn release(&self) {
        if self.released.compare_exchange(false, true).is_ok() {
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
        self.try_not_released(vm).map(|_| self.desc.len)
    }

    #[pyproperty]
    fn readonly(&self, vm: &VirtualMachine) -> PyResult<bool> {
        self.try_not_released(vm).map(|_| self.desc.readonly)
    }

    #[pyproperty]
    fn itemsize(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.try_not_released(vm).map(|_| self.desc.itemsize)
    }

    #[pyproperty]
    fn ndim(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.try_not_released(vm).map(|_| self.desc.ndim())
    }

    #[pyproperty]
    fn shape(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        self.try_not_released(vm)?;
        Ok(vm.ctx.new_tuple(
            self.desc
                .dim_desc
                .iter()
                .map(|(shape, _, _)| shape.into_pyobject(vm))
                .collect(),
        ))
    }

    #[pyproperty]
    fn strides(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        self.try_not_released(vm)?;
        Ok(vm.ctx.new_tuple(
            self.desc
                .dim_desc
                .iter()
                .map(|(_, stride, _)| stride.into_pyobject(vm))
                .collect(),
        ))
    }

    #[pyproperty]
    fn suboffsets(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        self.try_not_released(vm)?;
        Ok(vm.ctx.new_tuple(
            self.desc
                .dim_desc
                .iter()
                .map(|(_, _, suboffset)| suboffset.into_pyobject(vm))
                .collect(),
        ))
    }

    #[pyproperty]
    fn format(&self, vm: &VirtualMachine) -> PyResult<PyStr> {
        self.try_not_released(vm)
            .map(|_| PyStr::from(self.desc.format.clone()))
    }

    #[pyproperty]
    fn contiguous(&self, vm: &VirtualMachine) -> PyResult<bool> {
        self.try_not_released(vm).map(|_| self.desc.is_contiguous())
    }

    #[pyproperty]
    fn c_contiguous(&self, vm: &VirtualMachine) -> PyResult<bool> {
        self.try_not_released(vm).map(|_| self.desc.is_contiguous())
    }

    #[pyproperty]
    fn f_contiguous(&self, vm: &VirtualMachine) -> PyResult<bool> {
        // TODO: fortain order
        self.try_not_released(vm)
            .map(|_| self.desc.ndim() <= 1 && self.desc.is_contiguous())
    }

    #[pymethod(magic)]
    fn enter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.try_not_released(vm).map(|_| zelf)
    }

    #[pymethod(magic)]
    fn exit(&self, _args: FuncArgs) {
        self.release();
    }

    fn getitem_by_idx(&self, i: isize, vm: &VirtualMachine) -> PyResult {
        if self.desc.ndim() != 1 {
            return Err(vm.new_not_implemented_error(
                "multi-dimensional sub-views are not implemented".to_owned(),
            ));
        }
        let (shape, stride, suboffset) = self.desc.dim_desc[0];
        let index = wrap_index(i, shape)
            .ok_or_else(|| vm.new_index_error("index out of range".to_owned()))?;
        let index = index as isize * stride + suboffset;
        let pos = (index + self.start as isize) as usize;
        let bytes = self.buffer.obj_bytes();
        self.format_spec
            .unpack(&bytes[pos..pos + self.desc.itemsize], vm)
            .map(|x| {
                if x.len() == 1 {
                    x.fast_getitem(0)
                } else {
                    x.into()
                }
            })
    }

    fn getitem_by_slice(&self, slice: &PySlice, vm: &VirtualMachine) -> PyResult {
        let mut other = self.new_view();
        other.init_slice(slice, 0, vm)?;
        other.init_len();

        Ok(other.into_ref(vm).into_object())
    }

    fn getitem_by_multi_idx(&self, tuple: &PyTuple, vm: &VirtualMachine) -> PyResult {
        let tuple = tuple.as_slice();
        match tuple.len().cmp(&self.desc.ndim()) {
            Ordering::Less => {
                return Err(vm.new_not_implemented_error("sub-views are not implemented".to_owned()))
            }
            Ordering::Greater => {
                return Err(vm.new_type_error(format!(
                    "cannot index {}-dimension view with {}-element tuple",
                    self.desc.ndim(),
                    tuple.len()
                )))
            }
            Ordering::Equal => (),
        }

        let indices: Vec<isize> = tuple
            .iter()
            .map(|x| isize::try_from_borrowed_object(vm, x))
            .try_collect()?;
        let pos = self.desc.get_position(&indices, vm)?;
        let pos = (pos + self.start as isize) as usize;

        let bytes = self.buffer.obj_bytes();
        self.format_spec
            .unpack(&bytes[pos..pos + self.desc.itemsize], vm)
            .map(|x| {
                if x.len() == 1 {
                    x.fast_getitem(0)
                } else {
                    x.into()
                }
            })
    }

    #[pymethod(magic)]
    fn getitem(zelf: PyRef<Self>, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        zelf.try_not_released(vm)?;
        if zelf.desc.ndim() == 0 {
            // 0-d memoryview can be referenced using mv[...] or mv[()] only
            if needle.is(&vm.ctx.ellipsis) {
                return Ok(zelf.into_object());
            }
            if let Some(tuple) = needle.payload::<PyTuple>() {
                if tuple.len() == 0 {
                    return Ok(zelf
                        .format_spec
                        .unpack(&zelf.buffer.obj_bytes()[..zelf.desc.itemsize], vm)?
                        .fast_getitem(0));
                }
            }
            return Err(vm.new_type_error("invalid indexing of 0-dim memory".to_owned()));
        }

        // TODO: avoid clone
        if let Ok(seq_index) = SequenceIndex::try_from_object_for(vm, needle.clone(), Self::NAME) {
            match seq_index {
                SequenceIndex::Int(index) => zelf.getitem_by_idx(index, vm),
                SequenceIndex::Slice(slice) => zelf.getitem_by_slice(&slice, vm),
            }
        } else if let Some(tuple) = needle.payload::<PyTuple>() {
            zelf.getitem_by_multi_idx(tuple, vm)
        } else {
            // TODO: support multi slice
            Err(vm.new_type_error("memoryview: invalid slice key".to_owned()))
        }
    }

    fn setitem_by_idx(&self, i: isize, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if self.desc.ndim() != 1 {
            return Err(vm.new_not_implemented_error("sub-views are not implemented".to_owned()));
        }
        let (shape, stride, suboffset) = self.desc.dim_desc[0];
        let index = wrap_index(i, shape)
            .ok_or_else(|| vm.new_index_error("index out of range".to_owned()))?;
        let index = index as isize * stride + suboffset;
        let pos = (index + self.start as isize) as usize;
        let bytes = &mut *self.buffer.obj_bytes_mut();
        let data = self.format_spec.pack(vec![value], vm)?;
        bytes[pos..pos + self.desc.itemsize].copy_from_slice(&data);
        Ok(())
    }

    fn setitem_by_slice(
        &self,
        slice: &PySlice,
        items: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if self.desc.ndim() != 1 {
            return Err(vm.new_not_implemented_error("sub-view are not implemented".to_owned()));
        }
        let src = PyBuffer::try_from_object(vm, items)?;
        let mut dest = self.new_view();
        dest.init_slice(slice, 0, vm)?;
        dest.init_len();

        if !is_equiv_structure(&src.desc, &dest.desc) {
            return Err(vm.new_type_error(
                "memoryview assigment: lvalue and rvalue have different structures".to_owned(),
            ));
        }

        let mut bytes_mut = dest.buffer.obj_bytes_mut();
        let src_bytes = src.obj_bytes();
        dest.desc.zip_eq(&src.desc, true, |a_pos, b_pos, len| {
            let a_pos = (a_pos + self.start as isize) as usize;
            let b_pos = b_pos as usize;
            bytes_mut[a_pos..a_pos + len].copy_from_slice(&src_bytes[b_pos..b_pos + len]);
        });

        Ok(())
    }

    #[pymethod(magic)]
    fn setitem(
        &self,
        needle: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self.try_not_released(vm)?;
        if self.desc.readonly {
            return Err(vm.new_type_error("cannot modify read-only memory".to_owned()));
        }
        if value.is(&vm.ctx.none) {
            return Err(vm.new_type_error("cannot delete memory".to_owned()));
        }

        if self.desc.ndim() == 0 {
            if needle.is(&vm.ctx.ellipsis) {
                let bytes = &mut *self.buffer.obj_bytes_mut();
                let data = self.format_spec.pack(vec![value], vm)?;
                // TODO: fix panic if data no march itemsize
                bytes[..self.desc.itemsize].copy_from_slice(&data);
            } else if let Some(tuple) = needle.payload::<PyTuple>() {
                if tuple.len() == 0 {
                    let bytes = &mut *self.buffer.obj_bytes_mut();
                    let data = self.format_spec.pack(vec![value], vm)?;
                    // TODO: fix panic if data no march itemsize
                    bytes[..self.desc.itemsize].copy_from_slice(&data);
                }
            }
            return Err(vm.new_type_error("invalid indexing of 0-dim memory".to_owned()));
        }
        // TODO: SequenceIndex do not need to take the ownership
        if let Ok(seq_index) = SequenceIndex::try_from_object_for(vm, needle.clone(), Self::NAME) {
            match seq_index {
                SequenceIndex::Int(index) => self.setitem_by_idx(index, value, vm),
                SequenceIndex::Slice(slice) => self.setitem_by_slice(&slice, value, vm),
            }
        } else if let Some(_tuple) = needle.payload::<PyTuple>() {
            Err(vm.new_type_error("TODO".to_owned()))
        } else {
            // TODO: support multi slice
            Err(vm.new_type_error("memoryview: invalid slice key".to_owned()))
        }
    }

    fn init_len(&mut self) {
        let product = self.desc.dim_desc.iter().map(|x| x.0).product();
        self.desc.len = product;
    }

    fn init_range(&mut self, range: Range<usize>, dim: usize) {
        let (shape, stride, _) = self.desc.dim_desc[dim];
        debug_assert!(shape >= range.len());

        let mut is_adjusted_suboffset = false;
        for (_, _, suboffset) in self.desc.dim_desc.iter_mut().rev() {
            if *suboffset != 0 {
                *suboffset += stride * range.start as isize;
                is_adjusted_suboffset = true;
                break;
            }
        }
        if !is_adjusted_suboffset {
            self.start += stride as usize * range.start;
        }
        let newlen = range.len();
        self.desc.dim_desc[dim].0 = newlen;
    }

    fn init_slice(&mut self, slice: &PySlice, dim: usize, vm: &VirtualMachine) -> PyResult<()> {
        let (shape, stride, _) = self.desc.dim_desc[dim];
        let slice = slice.to_saturated(vm)?;
        let (range, step, slicelen) = slice.adjust_indices(shape);

        let mut is_adjusted_suboffset = false;
        for (_, _, suboffset) in self.desc.dim_desc.iter_mut().rev() {
            if *suboffset != 0 {
                *suboffset += stride * range.start as isize;
                is_adjusted_suboffset = true;
                break;
            }
        }
        if !is_adjusted_suboffset {
            // TODO: AdjustIndices
            self.start += stride as usize
                * if step.is_negative() {
                    range.end - 1
                } else {
                    range.start
                };
        }
        self.desc.dim_desc[dim].0 = slicelen;
        self.desc.dim_desc[dim].1 *= step;

        Ok(())
    }

    #[pymethod(magic)]
    fn len(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.try_not_released(vm)?;
        Ok(if self.desc.ndim() == 0 {
            1
        } else {
            // shape for dim[0]
            self.desc.dim_desc[0].0
        })
    }

    #[pymethod]
    fn tobytes(&self, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
        self.try_not_released(vm)?;
        let mut v = vec![];
        self.collect(&mut v);
        Ok(PyBytes::from(v).into_ref(vm))
    }

    fn _to_list(
        &self,
        bytes: &[u8],
        mut index: isize,
        dim: usize,
        vm: &VirtualMachine,
    ) -> PyResult<PyListRef> {
        let (shape, stride, suboffset) = self.desc.dim_desc[dim];
        if dim + 1 == self.desc.ndim() {
            let mut v = Vec::with_capacity(shape);
            for _ in 0..shape {
                let pos = index + suboffset;
                let pos = (pos + self.start as isize) as usize;
                let obj =
                    format_unpack(&self.format_spec, &bytes[pos..pos + self.desc.itemsize], vm)?;
                v.push(obj);
                index += stride;
            }
            return Ok(vm.ctx.new_list(v));
        }

        let mut v = Vec::with_capacity(shape);
        for _ in 0..shape {
            let obj = self
                ._to_list(bytes, index + suboffset, dim + 1, vm)?
                .into_object();
            v.push(obj);
            index += stride;
        }
        Ok(vm.ctx.new_list(v))
    }

    #[pymethod]
    fn tolist(&self, vm: &VirtualMachine) -> PyResult<PyListRef> {
        self.try_not_released(vm)?;
        if self.desc.ndim() == 0 {
            // TODO: unpack_single(view->buf, fmt)
            return Ok(vm.ctx.new_list(vec![]));
        }
        let bytes = self.buffer.obj_bytes();
        self._to_list(&bytes, 0, 0, vm)
    }

    #[pymethod]
    fn toreadonly(&self, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        self.try_not_released(vm)?;
        let mut other = self.new_view();
        other.desc.readonly = true;
        Ok(other.into_ref(vm))
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

    fn cast_to_1d(&self, format: PyStrRef, vm: &VirtualMachine) -> PyResult<Self> {
        let format_spec = Self::parse_format(format.as_str(), vm)?;
        let itemsize = format_spec.size();
        if self.desc.len % itemsize != 0 {
            return Err(
                vm.new_type_error("memoryview: length is not a multiple of itemsize".to_owned())
            );
        }

        Ok(Self {
            buffer: self.buffer.clone(),
            released: AtomicCell::new(false),
            start: self.start,
            format_spec,
            desc: BufferDescriptor {
                len: self.desc.len,
                readonly: self.desc.readonly,
                itemsize,
                format: format.to_string().into(),
                dim_desc: vec![(self.desc.len / itemsize, itemsize as isize, 0)],
            },
            hash: OnceCell::new(),
        })
    }

    #[pymethod]
    fn cast(&self, args: CastArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        self.try_not_released(vm)?;
        if !self.desc.is_contiguous() {
            return Err(vm.new_type_error(
                "memoryview: casts are restricted to C-contiguous views".to_owned(),
            ));
        }

        let CastArgs { format, shape } = args;

        if let OptionalArg::Present(shape) = shape {
            if self.desc.is_zero_in_shape() {
                return Err(vm.new_type_error(
                    "memoryview: cannot cast view with zeros in shape or strides".to_owned(),
                ));
            }

            let shape_vec = shape.borrow_vec();
            let shape_ndim = shape_vec.len();
            // TODO: MAX_NDIM
            if self.desc.ndim() != 1 && shape_ndim != 1 {
                return Err(
                    vm.new_type_error("memoryview: cast must be 1D -> ND or ND -> 1D".to_owned())
                );
            }

            let mut other = self.cast_to_1d(format, vm)?;
            let itemsize = other.desc.itemsize;

            // 0 ndim is single item
            if shape_ndim == 0 {
                other.desc.dim_desc = vec![];
                other.desc.len = itemsize;
                return Ok(other.into_ref(vm));
            }

            let mut product_shape = itemsize;
            let mut dim_descriptor = Vec::with_capacity(shape_ndim);

            for x in shape_vec.iter() {
                let x = usize::try_from_borrowed_object(vm, x)?;

                if x > isize::MAX as usize / product_shape {
                    return Err(vm.new_value_error(
                        "memoryview.cast(): product(shape) > SSIZE_MAX".to_owned(),
                    ));
                }
                product_shape *= x;
                dim_descriptor.push((x, 0, 0));
            }

            dim_descriptor.last_mut().unwrap().1 = itemsize as isize;
            for i in (0..dim_descriptor.len() - 1).rev() {
                dim_descriptor[i].1 = dim_descriptor[i + 1].1 * dim_descriptor[i + 1].0 as isize;
            }

            if product_shape != other.desc.len {
                return Err(vm.new_type_error(
                    "memoryview: product(shape) * itemsize != buffer size".to_owned(),
                ));
            }

            other.desc.dim_desc = dim_descriptor;

            Ok(other.into_ref(vm))
        } else {
            Ok(self.cast_to_1d(format, vm)?.into_ref(vm))
        }
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

        if let Some(other) = other.payload::<Self>() {
            if other.released.load() {
                return Ok(false);
            }
        }

        let other = match PyBuffer::try_from_borrowed_object(vm, other) {
            Ok(buf) => buf,
            Err(_) => return Ok(false),
        };

        if !is_equiv_shape(&zelf.desc, &other.desc) {
            return Ok(false);
        }

        let a_itemsize = zelf.desc.itemsize;
        let b_itemsize = other.desc.itemsize;
        let a_format_spec = &zelf.format_spec;
        let b_format_spec = &Self::parse_format(&other.desc.format, vm)?;

        if zelf.desc.ndim() == 0 {
            let a_val = format_unpack(a_format_spec, &zelf.buffer.obj_bytes()[..a_itemsize], vm)?;
            let b_val = format_unpack(b_format_spec, &other.obj_bytes()[..b_itemsize], vm)?;
            return Ok(vm.bool_eq(&a_val, &b_val)?);
        }

        zelf.contiguous_or_collect(|a| {
            other.contiguous_or_collect(|b| {
                // TODO: optimize cmp by format
                let a_list = unpack_bytes_seq_to_list(a, a_format_spec, vm)?;
                let b_list = unpack_bytes_seq_to_list(b, b_format_spec, vm)?;

                vm.bool_eq(a_list.as_object(), b_list.as_object())
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

    fn obj_bytes(&self) -> BorrowedValue<[u8]> {
        BorrowedValue::map(self.buffer.obj_bytes(), |x| &x[self.start..])
    }

    fn obj_bytes_mut(&self) -> BorrowedValueMut<[u8]> {
        BorrowedValueMut::map(self.buffer.obj_bytes_mut(), |x| &mut x[self.start..])
    }

    fn as_contiguous(&self) -> Option<BorrowedValue<[u8]>> {
        self.desc
            .is_contiguous()
            .then(|| BorrowedValue::map(self.buffer.obj_bytes(), |x| &x[self.start..self.start + self.desc.len]))
    }

    fn _as_contiguous_mut(&self) -> Option<BorrowedValueMut<[u8]>> {
        self.desc.is_contiguous().then(|| {
            BorrowedValueMut::map(self.buffer.obj_bytes_mut(), |x| {
                &mut x[self.start..self.start + self.desc.len]
            })
        })
    }

    fn collect(&self, buf: &mut Vec<u8>) {
        if let Some(bytes) = self.as_contiguous() {
            buf.extend_from_slice(&bytes);
        } else {
            let bytes = &*self.buffer.obj_bytes();
            self.desc.for_each_segment(true, |range| {
                let start = (range.start + self.start as isize) as usize;
                let end = (range.end + self.start as isize) as usize;
                buf.extend_from_slice(&bytes[start..end]);
            })
        }
    }

    fn contiguous_or_collect<R, F: FnOnce(&[u8]) -> R>(&self, f: F) -> R {
        let borrowed;
        let mut collected;
        let v = if let Some(bytes) = self.as_contiguous() {
            borrowed = bytes;
            &*borrowed
        } else {
            collected = vec![];
            self.collect(&mut collected);
            &collected
        };
        f(v)
    }
}

#[derive(FromArgs)]
struct CastArgs {
    #[pyarg(any)]
    format: PyStrRef,
    #[pyarg(any, optional)]
    shape: OptionalArg<PyListRef>,
}

static BUFFER_METHODS: BufferMethods = BufferMethods {
    obj_bytes: |buffer| buffer.obj_as::<PyMemoryView>().obj_bytes(),
    obj_bytes_mut: |buffer| buffer.obj_as::<PyMemoryView>().obj_bytes_mut(),
    release: |buffer| buffer.obj_as::<PyMemoryView>().buffer.release(),
    retain: |buffer| buffer.obj_as::<PyMemoryView>().buffer.retain(),
};

impl AsBuffer for PyMemoryView {
    fn as_buffer(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyBuffer> {
        if zelf.released.load() {
            Err(vm.new_value_error("operation forbidden on released memoryview object".to_owned()))
        } else {
            Ok(PyBuffer::new(
                zelf.to_owned().into_object(),
                zelf.desc.clone(),
                &BUFFER_METHODS,
            ))
        }
    }
}

impl Drop for PyMemoryView {
    fn drop(&mut self) {
        if self.released.load() {
            unsafe { self.buffer.drop_without_release() };
        } else {
            unsafe { ManuallyDrop::drop(&mut self.buffer) };
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
            Some(value) => Self::downcast(zelf, vm).map(|zelf| zelf.setitem(needle, value, vm))?,
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
                if !zelf.desc.readonly {
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
    format_spec: &FormatSpec,
    vm: &VirtualMachine,
) -> PyResult<PyListRef> {
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

fn is_equiv_shape(a: &BufferDescriptor, b: &BufferDescriptor) -> bool {
    if a.ndim() != b.ndim() {
        return false;
    }

    let a_iter = a.dim_desc.iter().map(|x| x.0);
    let b_iter = b.dim_desc.iter().map(|x| x.0);
    for (a_shape, b_shape) in a_iter.zip(b_iter) {
        if a_shape != b_shape {
            return false;
        }
        if a_shape == 0 {
            break;
        }
    }
    true
}

fn is_equiv_format(a: &BufferDescriptor, b: &BufferDescriptor) -> bool {
    // TODO: skip @
    a.itemsize == b.itemsize && a.format == b.format
}

fn is_equiv_structure(a: &BufferDescriptor, b: &BufferDescriptor) -> bool {
    is_equiv_format(a, b) && is_equiv_shape(a, b)
}
