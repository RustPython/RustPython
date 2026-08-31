use super::{
    PositionIterInternal, PyBytes, PyBytesRef, PyGenericAlias, PyInt, PyListRef, PySlice, PyStr,
    PyStrRef, PyTuple, PyTupleRef, PyType, PyTypeRef, PyUtf8StrRef, iter::builtins_iter,
    locked_next,
};
use crate::common::lock::LazyLock;
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult,
    TryFromBorrowedObject, TryFromObject, VirtualMachine, atomic_func,
    buffer::{FormatSpec, PackErrorKind},
    bytes_inner::{ByteInnerHexOptions, bytes_to_hex},
    class::{PyClassImpl, StaticType},
    common::{
        borrow::{BorrowedValue, BorrowedValueMut},
        hash::PyHash,
        lock::OnceCell,
    },
    convert::ToPyObject,
    function::Either,
    function::{ArgIndex, FuncArgs, OptionalArg, PyComparisonValue},
    protocol::{
        BufferDescriptor, BufferFlags, BufferMethods, PyBuffer, PyIterReturn, PyMappingMethods,
        PySequenceMethods, VecBuffer,
    },
    sliceable::SequenceIndexOp,
    types::{
        AsBuffer, AsMapping, AsSequence, Comparable, Constructor, Hashable, IterNext, Iterable,
        PyComparisonOp, Representable, SelfIter,
    },
};
use core::{cmp::Ordering, fmt::Debug, ops::Range};
use crossbeam_utils::atomic::AtomicCell;
use itertools::Itertools;
use rustpython_common::lock::PyMutex;

/// The most dimensions a view can describe. PyBUF_MAX_NDIM
const MAX_NDIM: usize = 64;

#[derive(FromArgs)]
pub struct PyMemoryViewNewArgs {
    object: PyObjectRef,
}

#[derive(FromArgs)]
struct PyMemoryViewFromFlagsArgs {
    object: PyObjectRef,
    flags: ArgIndex,
}

#[pyclass(module = false, name = "memoryview", traverse)]
#[derive(Debug)]
pub struct PyMemoryView {
    /// One share of the acquisition this view is looking at, given up when the
    /// view is released or dropped.
    buffer: PyBuffer,
    // the released memoryview does not mean the buffer is destroyed
    // because the possible another memoryview is viewing from it
    #[pytraverse(skip)]
    released: AtomicCell<bool>,
    /// Forbids handing out anything that outlives this view, for the window
    /// passed to `__release_buffer__`.
    #[pytraverse(skip)]
    restricted: AtomicCell<bool>,
    #[pytraverse(skip)]
    format_spec: FormatSpec,
    // memoryview's options could be different from buffer's options
    #[pytraverse(skip)]
    desc: BufferDescriptor,
    #[pytraverse(skip)]
    hash: OnceCell<PyHash>,
    /// Buffers handed out of this view that have not been given back yet. The
    /// view cannot be released while any of them is outstanding, so that what
    /// reads them keeps reading memory that is still there. self->exports
    #[pytraverse(skip)]
    exports: AtomicCell<usize>,
}

impl Constructor for PyMemoryView {
    type Args = PyMemoryViewNewArgs;

    fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
        Self::from_object(&args.object, vm)
    }
}

impl PyMemoryView {
    fn parse_format(format: &str, vm: &VirtualMachine) -> PyResult<FormatSpec> {
        FormatSpec::parse(format.as_bytes(), vm)
    }

    /// The single native format character a cast is allowed to name, with an
    /// optional `@` in front of it. get_native_fmtchar
    fn native_fmtchar(format: &str) -> Option<u8> {
        let format = format.strip_prefix('@').unwrap_or(format);
        let [c] = *format.as_bytes() else {
            return None;
        };
        matches!(
            c,
            b'c' | b'b'
                | b'B'
                | b'h'
                | b'H'
                | b'i'
                | b'I'
                | b'l'
                | b'L'
                | b'q'
                | b'Q'
                | b'n'
                | b'N'
                | b'f'
                | b'd'
                | b'e'
                | b'?'
                | b'P'
        )
        .then_some(c)
    }

    /// this should be the main entrance to create the memoryview
    /// to avoid the chained memoryview
    pub fn from_object(obj: &PyObject, vm: &VirtualMachine) -> PyResult<Self> {
        Self::from_object_with_flags(obj, BufferFlags::FULL_RO, vm)
    }

    /// One share of the underlying buffer export. Used for cross-interpreter
    /// `send_buffer` so the destination memoryview sees the same memory.
    #[must_use]
    pub fn clone_buffer(&self) -> PyBuffer {
        let mut buffer = self.buffer.clone();
        buffer.desc = self.desc.clone();
        buffer
    }

    // PyMemoryView_FromObjectAndFlags
    pub fn from_object_with_flags(
        obj: &PyObject,
        flags: BufferFlags,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        if let Some(other) = obj.downcast_ref::<Self>() {
            other.try_not_released(vm)?;
            other.try_not_restricted(vm)?;
            Ok(other.new_view())
        } else if obj.check_buffer() {
            let buffer = PyBuffer::from_object(vm, obj, flags)?;
            Self::from_buffer(buffer, vm)
        } else {
            Err(vm.new_type_error(format!(
                "memoryview: a bytes-like object is required, not '{}'",
                obj.class().name()
            )))
        }
    }

    /// don't use this function to create the memoryview if the buffer is exporting
    /// via another memoryview, use PyMemoryView::new_view() or PyMemoryView::from_object
    /// to reduce the chain
    pub fn from_buffer(buffer: PyBuffer, vm: &VirtualMachine) -> PyResult<Self> {
        // when we get a buffer means the buffered object is size locked
        // so we can assume the buffer's options will never change as long
        // as memoryview is still alive
        let format_spec = Self::parse_format(&buffer.desc.format, vm)?;
        let desc = buffer.desc.clone();

        Ok(Self {
            buffer,
            released: AtomicCell::new(false),
            restricted: AtomicCell::new(false),
            format_spec,
            desc,
            hash: OnceCell::new(),
            exports: AtomicCell::new(0),
        })
    }

    /// don't use this function to create the memoryview if the buffer is exporting
    /// via another memoryview, use PyMemoryView::new_view() or PyMemoryView::from_object
    /// to reduce the chain
    pub fn from_buffer_range(
        buffer: PyBuffer,
        range: Range<usize>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        let mut zelf = Self::from_buffer(buffer, vm)?;

        zelf.init_range(range, 0);
        zelf.init_len();
        Ok(zelf)
    }

    /// this should be the only way to create a memoryview from another memoryview.
    #[must_use]
    pub fn new_view(&self) -> Self {
        Self {
            buffer: self.buffer.clone(),
            released: AtomicCell::new(false),
            restricted: AtomicCell::new(false),
            format_spec: self.format_spec.clone(),
            desc: self.desc.clone(),
            hash: OnceCell::new(),
            exports: AtomicCell::new(0),
        }
    }

    /// A view for a temporary that never reaches Python. It counts as no export,
    /// so the exporter stays exactly as resizable as it already was, the way a
    /// `Py_buffer dest = *view` copy does.
    #[must_use]
    fn borrowed_view(&self) -> Self {
        Self {
            buffer: self.buffer.detached(),
            released: AtomicCell::new(false),
            restricted: AtomicCell::new(false),
            format_spec: self.format_spec.clone(),
            desc: self.desc.clone(),
            hash: OnceCell::new(),
            exports: AtomicCell::new(0),
        }
    }

    /// The object this view looks at, whose storage it borrows.
    pub fn viewed_object(&self) -> &PyObject {
        &self.buffer.obj
    }

    fn try_not_released(&self, vm: &VirtualMachine) -> PyResult<()> {
        if self.released.load() {
            Err(vm.new_value_error("operation forbidden on released memoryview object"))
        } else {
            Ok(())
        }
    }

    fn try_not_restricted(&self, vm: &VirtualMachine) -> PyResult<()> {
        if self.restricted.load() {
            Err(vm.new_value_error("cannot create new view on restricted memoryview"))
        } else {
            Ok(())
        }
    }

    fn try_usable(&self, vm: &VirtualMachine) -> PyResult<()> {
        self.try_not_released(vm)?;
        self.try_not_restricted(vm)
    }

    /// Reject a request this view cannot serve. memory_getbuf
    fn check_buffer_request(&self, flags: BufferFlags, vm: &VirtualMachine) -> PyResult<()> {
        let c_contiguous = self.desc.is_contiguous();
        flags.check_writable(
            self.desc.readonly,
            "memoryview: underlying buffer is not writable",
            vm,
        )?;
        if flags.contains(BufferFlags::C_CONTIGUOUS) && !c_contiguous {
            return Err(vm.new_buffer_error("memoryview: underlying buffer is not C-contiguous"));
        }
        if flags.contains(BufferFlags::F_CONTIGUOUS) && !self.desc.is_fortran_contiguous() {
            return Err(
                vm.new_buffer_error("memoryview: underlying buffer is not Fortran contiguous")
            );
        }
        if flags.contains(BufferFlags::ANY_CONTIGUOUS)
            && !c_contiguous
            && !self.desc.is_fortran_contiguous()
        {
            return Err(vm.new_buffer_error("memoryview: underlying buffer is not contiguous"));
        }
        // No exporter here produces a suboffset, so this is a guard rather than a
        // reachable rejection.
        if !flags.contains(BufferFlags::INDIRECT) && self.desc.has_suboffsets() {
            return Err(vm.new_buffer_error("memoryview: underlying buffer requires suboffsets"));
        }
        if !flags.contains(BufferFlags::STRIDES) && !c_contiguous {
            return Err(vm.new_buffer_error("memoryview: underlying buffer is not C-contiguous"));
        }
        if !flags.contains(BufferFlags::ND) && flags.intersects(BufferFlags::FORMAT) {
            return Err(vm.new_buffer_error(
                "memoryview: cannot cast to unsigned bytes if the format flag is present",
            ));
        }
        Ok(())
    }

    /// The descriptor this view exports for `flags`, or an error if it cannot
    /// serve the request. memory_getbuf
    fn requested_desc(
        &self,
        flags: BufferFlags,
        vm: &VirtualMachine,
    ) -> PyResult<BufferDescriptor> {
        self.check_buffer_request(flags, vm)?;
        Ok(self.desc.projected(flags))
    }

    fn getitem_by_idx(&self, i: isize, vm: &VirtualMachine) -> PyResult {
        if self.desc.ndim() != 1 {
            return Err(
                vm.new_not_implemented_error("multi-dimensional sub-views are not implemented")
            );
        }
        let (shape, _, _) = self.desc.dim_desc[0];
        // ptr_from_index
        let index = i
            .wrapped_at(shape)
            .ok_or_else(|| vm.new_index_error("index out of bounds on dimension 1"))?;
        self.unpack_single(self.desc.fast_position(&[index]) as usize, vm)
    }

    fn getitem_by_slice(&self, slice: &PySlice, vm: &VirtualMachine) -> PyResult {
        self.try_not_restricted(vm)?;
        let mut other = self.new_view();
        other.init_slice(slice, 0, vm)?;
        other.init_len();

        Ok(other.into_ref(&vm.ctx).into())
    }

    fn getitem_by_multi_idx(&self, indexes: &[isize], vm: &VirtualMachine) -> PyResult {
        let pos = self.pos_from_multi_index(indexes, vm)?;
        self.unpack_single(pos, vm)
    }

    fn setitem_by_idx(&self, i: isize, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if self.desc.ndim() != 1 {
            return Err(vm.new_not_implemented_error("sub-views are not implemented"));
        }
        let (shape, _, _) = self.desc.dim_desc[0];
        // ptr_from_index
        let index = i
            .wrapped_at(shape)
            .ok_or_else(|| vm.new_index_error("index out of bounds on dimension 1"))?;
        self.pack_single(self.desc.fast_position(&[index]) as usize, value, vm)
    }

    fn setitem_by_multi_idx(
        &self,
        indexes: &[isize],
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let pos = self.pos_from_multi_index(indexes, vm)?;
        self.pack_single(pos, value, vm)
    }

    fn pack_single(&self, pos: usize, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        // The value is converted before the destination is borrowed, because the
        // conversion runs `__index__` or `__float__`, which can read or write the
        // same buffer.
        // TODO: Optimize
        // A value of the wrong kind and a value the format has no room for are
        // different errors here, though packing reports both the same way.
        let data = self.format_spec.try_pack(vec![value], vm).map_err(|err| {
            let what = match err.kind {
                PackErrorKind::Type => "type",
                PackErrorKind::Value => "value",
                PackErrorKind::Raised => return err.exception,
            };
            let msg = format!(
                "memoryview: invalid {what} for format '{}'",
                self.desc.format
            );
            match err.kind {
                PackErrorKind::Type => vm.new_type_error(msg),
                _ => vm.new_value_error(msg),
            }
        })?;
        // The conversion, and the index that produced `pos`, could have released
        // the view; `pos` addresses a buffer that is no longer there.
        // CHECK_RELEASED_INT_AGAIN
        self.try_not_released(vm)?;
        let mut bytes = self.buffer.obj_bytes_mut();
        bytes[pos..pos + self.format_spec.size()].copy_from_slice(&data);
        Ok(())
    }

    fn unpack_single(&self, pos: usize, vm: &VirtualMachine) -> PyResult {
        // The index that produced `pos` could have released the view.
        // CHECK_RELEASED_AGAIN
        self.try_not_released(vm)?;
        let bytes = self.buffer.obj_bytes();
        // TODO: Optimize
        self.format_spec
            .unpack(&bytes[pos..pos + self.format_spec.size()], vm)
            .map(|x| {
                if x.len() == 1 {
                    x[0].to_owned()
                } else {
                    x.into()
                }
            })
    }

    fn pos_from_multi_index(&self, indexes: &[isize], vm: &VirtualMachine) -> PyResult<usize> {
        match indexes.len().cmp(&self.desc.ndim()) {
            Ordering::Less => {
                return Err(vm.new_not_implemented_error("sub-views are not implemented"));
            }
            Ordering::Greater => {
                return Err(vm.new_type_error(format!(
                    "cannot index {}-dimension view with {}-element tuple",
                    self.desc.ndim(),
                    indexes.len()
                )));
            }
            Ordering::Equal => (),
        }

        Ok(self.desc.position(indexes, vm)? as usize)
    }

    fn init_len(&mut self) {
        let product: usize = self.desc.dim_desc.iter().map(|x| x.0).product();
        self.desc.len = product * self.desc.itemsize;
    }

    /// Move this view by `delta` bytes. The offset moves, unless a dimension
    /// outside `dim` is reached through a pointer, in which case its suboffset
    /// does.
    fn adjust_position(&mut self, dim: usize, delta: isize) {
        match self.desc.dim_desc[..dim]
            .iter()
            .rposition(|&(_, _, suboffset)| suboffset != 0)
        {
            Some(n) => self.desc.dim_desc[n].2 += delta,
            None => self.desc.offset += delta,
        }
    }

    fn init_range(&mut self, range: Range<usize>, dim: usize) {
        let (shape, stride, _) = self.desc.dim_desc[dim];
        debug_assert!(shape >= range.len());

        self.adjust_position(dim, stride * range.start as isize);
        self.desc.dim_desc[dim].0 = range.len();
    }

    // init_slice
    fn init_slice(&mut self, slice: &PySlice, dim: usize, vm: &VirtualMachine) -> PyResult<()> {
        let (shape, stride, _) = self.desc.dim_desc[dim];
        let slice = slice.to_saturated(vm)?;
        let (start, slice_len) = slice.adjust_indices_start(shape);

        // Repeated slicing multiplies the stride by the step every time, which
        // overflows after about twenty rounds; C wraps there and so does this.
        self.adjust_position(dim, stride.wrapping_mul(start));
        self.desc.dim_desc[dim].0 = slice_len;
        self.desc.dim_desc[dim].1 = stride.wrapping_mul(slice.step());

        Ok(())
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
                let pos = (index + suboffset) as usize;
                let obj = format_unpack(
                    &self.format_spec,
                    &bytes[pos..pos + self.format_spec.size()],
                    vm,
                )?;
                v.push(obj);
                index += stride;
            }
            return Ok(vm.ctx.new_list(v));
        }

        let mut v = Vec::with_capacity(shape);
        for _ in 0..shape {
            let obj = self._to_list(bytes, index + suboffset, dim + 1, vm)?.into();
            v.push(obj);
            index += stride;
        }
        Ok(vm.ctx.new_list(v))
    }

    fn eq(zelf: &Py<Self>, other: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
        if zelf.is(other) {
            return Ok(true);
        }
        if zelf.released.load() {
            return Ok(false);
        }

        let other = if let Some(mv) = other.downcast_ref::<Self>() {
            if mv.released.load() {
                return Ok(false);
            }
            // Another view's buffer is read where it lies rather than acquired,
            // so that a restricted view still compares. memory_richcompare
            let mut view = mv.buffer.detached();
            view.desc = mv.desc.clone();
            view
        } else {
            match PyBuffer::try_from_borrowed_object(vm, other) {
                Ok(buf) => buf,
                Err(_) => return Ok(false),
            }
        };

        if !is_equiv_shape(&zelf.desc, &other.desc) {
            return Ok(false);
        }

        let a_format_spec = &zelf.format_spec;
        let b_format_spec = &Self::parse_format(&other.desc.format, vm)?;
        // An element is as wide as its format, which a projected descriptor can
        // make narrower than the item size it steps by.
        let a_itemsize = a_format_spec.size();
        let b_itemsize = b_format_spec.size();

        if zelf.desc.ndim() == 0 {
            let a_pos = zelf.desc.offset as usize;
            let b_pos = other.desc.offset as usize;
            let a_bytes = zelf.buffer.obj_bytes();
            let a_val = format_unpack(a_format_spec, &a_bytes[a_pos..a_pos + a_itemsize], vm)?;
            drop(a_bytes);
            let b_bytes = other.obj_bytes();
            let b_val = format_unpack(b_format_spec, &b_bytes[b_pos..b_pos + b_itemsize], vm)?;
            drop(b_bytes);
            return vm.bool_eq(&a_val, &b_val);
        }

        // TODO: optimize cmp by format
        let mut ret = Ok(true);
        let a_bytes = zelf.buffer.obj_bytes();
        let b_bytes = other.obj_bytes();
        zelf.desc.zip_eq(&other.desc, false, |a_range, b_range| {
            let a_range = a_range.start as usize..a_range.start as usize + a_itemsize;
            let b_range = b_range.start as usize..b_range.start as usize + b_itemsize;
            let a_val = match format_unpack(a_format_spec, &a_bytes[a_range], vm) {
                Ok(val) => val,
                Err(e) => {
                    ret = Err(e);
                    return true;
                }
            };
            let b_val = match format_unpack(b_format_spec, &b_bytes[b_range], vm) {
                Ok(val) => val,
                Err(e) => {
                    ret = Err(e);
                    return true;
                }
            };
            ret = vm.bool_eq(&a_val, &b_val);
            if let Ok(b) = ret { !b } else { true }
        });
        ret
    }

    fn as_contiguous(&self) -> Option<BorrowedValue<'_, [u8]>> {
        self.desc.is_contiguous().then(|| {
            let range = self.desc.contiguous_range();
            BorrowedValue::map(self.buffer.obj_bytes(), |x| &x[range])
        })
    }

    fn _as_contiguous_mut(&self) -> Option<BorrowedValueMut<'_, [u8]>> {
        self.desc.is_contiguous().then(|| {
            let range = self.desc.contiguous_range();
            BorrowedValueMut::map(self.buffer.obj_bytes_mut(), |x| &mut x[range])
        })
    }

    fn append_to(&self, buf: &mut Vec<u8>) {
        if let Some(bytes) = self.as_contiguous() {
            buf.extend_from_slice(&bytes);
        } else {
            buf.reserve(self.desc.len);
            let bytes = &*self.buffer.obj_bytes();
            self.desc.for_each_segment(true, |range| {
                buf.extend_from_slice(&bytes[range.start as usize..range.end as usize]);
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
            self.append_to(&mut collected);
            &collected
        };
        f(v)
    }

    /// clone data from memoryview
    /// keep the shape, convert to contiguous
    pub fn to_contiguous(&self, vm: &VirtualMachine) -> PyBuffer {
        let mut data = vec![];
        self.append_to(&mut data);

        let desc = self.desc.contiguous();

        VecBuffer::from(data)
            .into_ref(&vm.ctx)
            .into_pybuffer_with_descriptor(desc)
    }
}

impl Py<PyMemoryView> {
    fn setitem_by_slice(
        &self,
        slice: &PySlice,
        src: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if self.desc.ndim() != 1 {
            return Err(vm.new_not_implemented_error("sub-view are not implemented"));
        }

        let mut dest = self.borrowed_view();
        dest.init_slice(slice, 0, vm)?;
        dest.init_len();

        if self.is(&src) {
            return if !is_equiv_structure(&self.desc, &dest.desc) {
                Err(vm.new_value_error(
                    "memoryview assignment: lvalue and rvalue have different structures",
                ))
            } else {
                // assign self[:] to self
                Ok(())
            };
        };

        // PyObject_GetBuffer(value, &src, PyBUF_FULL_RO)
        let src = PyBuffer::try_from_object(vm, src)?;
        // Acquiring the source ran `__buffer__`, which can release this view.
        // copy_single: CHECK_RELEASED_INT_AGAIN
        self.try_not_released(vm)?;

        if !is_equiv_structure(&src.desc, &dest.desc) {
            return Err(vm.new_value_error(
                "memoryview assignment: lvalue and rvalue have different structures",
            ));
        }

        // copy_buffer reads the source as it stood before the copy began, which an
        // overlapping assignment depends on and which also keeps the two borrows
        // below off the same storage.
        let src = if root_exporter(&src).is(&root_exporter(&dest.buffer)) {
            let owned = src.to_contiguous(vm);
            drop(src);
            owned
        } else {
            src
        };

        let mut bytes_mut = dest.buffer.obj_bytes_mut();
        let src_bytes = src.obj_bytes();
        dest.desc.zip_eq(&src.desc, true, |a_range, b_range| {
            let a_range = a_range.start as usize..a_range.end as usize;
            let b_range = b_range.start as usize..b_range.end as usize;
            bytes_mut[a_range].copy_from_slice(&src_bytes[b_range]);
            false
        });

        Ok(())
    }
}

#[pyclass(
    with(
        Py,
        Hashable,
        Comparable,
        AsBuffer,
        AsMapping,
        AsSequence,
        Constructor,
        Iterable,
        Representable
    ),
    flags(SEQUENCE, HAS_WEAKREF)
)]
impl PyMemoryView {
    #[pyclassmethod]
    fn __class_getitem__(
        cls: PyTypeRef,
        args: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyGenericAlias> {
        PyGenericAlias::from_args(cls, args, vm)
    }

    #[pyclassmethod]
    fn _from_flags(
        _cls: PyTypeRef,
        args: PyMemoryViewFromFlagsArgs,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        let flags =
            BufferFlags::from_bits_retain(args.flags.as_ref().try_to_primitive::<i32>(vm)? as u32);
        Self::from_object_with_flags(&args.object, flags, vm).map(|mv| mv.into_ref(&vm.ctx))
    }

    #[pymethod(name = "release")]
    fn py_release(&self, vm: &VirtualMachine) -> PyResult<()> {
        // _memory_release: what still reads this view holds it open.
        let exports = self.exports.load();
        if !self.released.load() && exports > 0 {
            let plural = if exports == 1 { "" } else { "s" };
            return Err(
                vm.new_buffer_error(format!("memoryview has {exports} exported buffer{plural}"))
            );
        }
        self.release();
        Ok(())
    }

    /// Give up the view's share without asking whether anything is reading it.
    /// The teardown paths have nowhere to report a refusal.
    pub fn release(&self) {
        if self.released.compare_exchange(false, true).is_ok() {
            self.buffer.release();
        }
    }

    /// Count this view as exported while `f` runs, so Python reached from inside
    /// it cannot release the memory being read out from under it.
    fn while_exported<R>(&self, f: impl FnOnce() -> R) -> R {
        self.exports.fetch_add(1);
        let result = f();
        self.exports.fetch_sub(1);
        result
    }

    #[pygetset]
    fn obj(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        self.try_not_released(vm)?;
        // A window over a buffer being released exposes no exporter, like a
        // Py_buffer whose obj is NULL.
        Ok(if self.buffer.obj.downcastable::<PyBufferWindow>() {
            vm.ctx.none()
        } else {
            self.buffer.obj.clone()
        })
    }

    #[pygetset]
    fn nbytes(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.try_not_released(vm).map(|_| self.desc.len)
    }

    #[pygetset]
    fn readonly(&self, vm: &VirtualMachine) -> PyResult<bool> {
        self.try_not_released(vm).map(|_| self.desc.readonly)
    }

    #[pygetset]
    fn itemsize(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.try_not_released(vm).map(|_| self.desc.itemsize)
    }

    #[pygetset]
    fn ndim(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.try_not_released(vm).map(|_| self.desc.ndim())
    }

    #[pygetset]
    fn shape(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        self.try_not_released(vm)?;
        Ok(vm.ctx.new_tuple(
            self.desc
                .dim_desc
                .iter()
                .map(|(shape, _, _)| shape.to_pyobject(vm))
                .collect(),
        ))
    }

    #[pygetset]
    fn strides(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        self.try_not_released(vm)?;
        Ok(vm.ctx.new_tuple(
            self.desc
                .dim_desc
                .iter()
                .map(|(_, stride, _)| stride.to_pyobject(vm))
                .collect(),
        ))
    }

    #[pygetset]
    fn suboffsets(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        self.try_not_released(vm)?;
        let has_suboffsets = self
            .desc
            .dim_desc
            .iter()
            .any(|(_, _, suboffset)| *suboffset != 0);
        if has_suboffsets {
            Ok(vm.ctx.new_tuple(
                self.desc
                    .dim_desc
                    .iter()
                    .map(|(_, _, suboffset)| suboffset.to_pyobject(vm))
                    .collect(),
            ))
        } else {
            Ok(vm.ctx.empty_tuple.clone())
        }
    }

    #[pygetset]
    fn format(&self, vm: &VirtualMachine) -> PyResult<PyStr> {
        self.try_not_released(vm)
            .map(|_| PyStr::from(self.desc.format.clone()))
    }

    #[pygetset]
    fn contiguous(&self, vm: &VirtualMachine) -> PyResult<bool> {
        self.try_not_released(vm)
            .map(|_| self.desc.is_contiguous() || self.desc.is_fortran_contiguous())
    }

    #[pygetset]
    fn c_contiguous(&self, vm: &VirtualMachine) -> PyResult<bool> {
        self.try_not_released(vm).map(|_| self.desc.is_contiguous())
    }

    #[pygetset]
    fn f_contiguous(&self, vm: &VirtualMachine) -> PyResult<bool> {
        self.try_not_released(vm)
            .map(|_| self.desc.is_fortran_contiguous())
    }

    #[pymethod]
    fn __enter__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        zelf.try_not_released(vm).map(|_| zelf)
    }

    // memory_exit
    #[pymethod]
    fn __exit__(&self, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        self.py_release(vm)
    }

    fn __getitem__(zelf: PyRef<Self>, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        zelf.try_not_released(vm)?;
        if zelf.desc.ndim() == 0 {
            // 0-d memoryview can be referenced using mv[...] or mv[()] only
            if needle.is(&vm.ctx.ellipsis) {
                return Ok(zelf.into());
            }
            if let Some(tuple) = needle.downcast_ref::<PyTuple>()
                && tuple.is_empty()
            {
                return zelf.unpack_single(zelf.desc.offset as usize, vm);
            }
            return Err(vm.new_type_error("invalid indexing of 0-dim memory"));
        }

        match SubscriptNeedle::try_from_object(vm, needle)? {
            SubscriptNeedle::Index(i) => zelf.getitem_by_idx(i, vm),
            SubscriptNeedle::Slice(slice) => zelf.getitem_by_slice(&slice, vm),
            SubscriptNeedle::MultiIndex(indices) => zelf.getitem_by_multi_idx(&indices, vm),
        }
    }

    fn __delitem__(&self, _needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.try_not_released(vm)?;
        // What cannot be written cannot be deleted from either, and that is
        // the first thing answered.
        if self.desc.readonly {
            return Err(vm.new_type_error("cannot modify read-only memory"));
        }
        Err(vm.new_type_error("cannot delete memory"))
    }

    fn __len__(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.try_not_released(vm)?;
        if self.desc.ndim() == 0 {
            // 0-dimensional memoryview has no length
            Err(vm.new_type_error("0-dim memory has no length"))
        } else {
            // shape for dim[0]
            Ok(self.desc.dim_desc[0].0)
        }
    }

    #[pymethod]
    fn tobytes(&self, args: ToBytesArgs, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
        self.try_not_released(vm)?;
        let order = match &args.order {
            None => Order::C,
            Some(order) => match order.to_str() {
                Some("C") => Order::C,
                Some("F") => Order::Fortran,
                Some("A") => Order::Any,
                _ => return Err(vm.new_value_error("order must be 'C', 'F' or 'A'")),
            },
        };

        let mut v = vec![];
        // 'A' asks for the memory as it is laid out, which is what appending a
        // contiguous view does. Only a Fortran walk of a view that is not
        // already Fortran-contiguous reorders anything, and a view of fewer
        // than two dimensions has one layout under either name.
        if order == Order::Fortran && self.desc.ndim() > 1 {
            v.reserve(self.desc.len);
            let bytes = &*self.buffer.obj_bytes();
            self.desc.for_each_segment_fortran(|range| {
                v.extend_from_slice(&bytes[range.start as usize..range.end as usize]);
            });
        } else {
            self.append_to(&mut v);
        }
        Ok(PyBytes::from(v).into_ref(&vm.ctx))
    }

    #[pymethod]
    // memory_tolist
    fn tolist(&self, vm: &VirtualMachine) -> PyResult {
        self.try_not_released(vm)?;
        let bytes = self.buffer.obj_bytes();
        if self.desc.ndim() == 0 {
            // A 0-dim view holds one element, which is what it unpacks to.
            let pos = self.desc.offset as usize;
            return format_unpack(
                &self.format_spec,
                &bytes[pos..pos + self.format_spec.size()],
                vm,
            );
        }
        self._to_list(&bytes, self.desc.offset, 0, vm)
            .map(Into::into)
    }

    #[pymethod]
    fn toreadonly(&self, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        self.try_usable(vm)?;
        let mut other = self.new_view();
        other.desc.readonly = true;
        Ok(other.into_ref(&vm.ctx))
    }

    #[pymethod]
    fn hex(&self, options: ByteInnerHexOptions, vm: &VirtualMachine) -> PyResult<String> {
        self.try_not_released(vm)?;
        // Measuring the separator runs Python, which must not release the bytes
        // being written out. memoryview_hex_impl
        let (sep, bytes_per_sep) = self.while_exported(|| options.resolve(vm))?;
        self.try_not_released(vm)?;
        Ok(self.contiguous_or_collect(|x| bytes_to_hex(x, sep, bytes_per_sep)))
    }

    #[pymethod]
    fn count(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        self.try_not_released(vm)?;
        if self.desc.ndim() != 1 {
            return Err(
                vm.new_not_implemented_error("multi-dimensional sub-views are not implemented")
            );
        }
        let len = self.desc.dim_desc[0].0;
        let mut count = 0;
        for i in 0..len {
            let item = self.getitem_by_idx(i as isize, vm)?;
            if vm.bool_eq(&item, &value)? {
                count += 1;
            }
        }
        Ok(count)
    }

    #[pymethod]
    fn index(
        &self,
        value: PyObjectRef,
        start: OptionalArg<isize>,
        stop: OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        self.try_not_released(vm)?;
        if self.desc.ndim() != 1 {
            return Err(
                vm.new_not_implemented_error("multi-dimensional sub-views are not implemented")
            );
        }
        let len = self.desc.dim_desc[0].0;
        let start = start.unwrap_or(0);
        let stop = stop.unwrap_or(len as isize);

        let start = if start < 0 {
            (start + len as isize).max(0) as usize
        } else {
            (start as usize).min(len)
        };
        let stop = if stop < 0 {
            (stop + len as isize).max(0) as usize
        } else {
            (stop as usize).min(len)
        };

        for i in start..stop {
            let item = self.getitem_by_idx(i as isize, vm)?;
            if vm.bool_eq(&item, &value)? {
                return Ok(i);
            }
        }
        Err(vm.new_value_error("memoryview.index(x): x not in memoryview"))
    }

    fn cast_to_1d(&self, format: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<Self> {
        let format_str = format.as_str();
        let Some(dest_char) = Self::native_fmtchar(format_str) else {
            return Err(vm.new_value_error(
                "memoryview: destination format must be a native single character format prefixed with an optional '@'",
            ));
        };
        // One side has to be bytes. Casting between two item types would
        // reinterpret the items rather than re-divide the memory, and the
        // source items were written by something that chose their type.
        let source_is_bytes = Self::native_fmtchar(&self.desc.format).is_some_and(is_byte_fmtchar);
        if !source_is_bytes && !is_byte_fmtchar(dest_char) {
            return Err(vm.new_type_error("memoryview: cannot cast between two non-byte formats"));
        }
        let format_spec = Self::parse_format(format_str, vm)?;
        let itemsize = format_spec.size();
        if !self.desc.len.is_multiple_of(itemsize) {
            return Err(vm.new_type_error("memoryview: length is not a multiple of itemsize"));
        }

        let zelf = Self {
            buffer: self.buffer.clone(),
            released: AtomicCell::new(false),
            restricted: AtomicCell::new(false),
            format_spec,
            desc: BufferDescriptor {
                len: self.desc.len,
                offset: self.desc.offset,
                readonly: self.desc.readonly,
                itemsize,
                format: format_str.to_owned().into(),
                dim_desc: vec![(self.desc.len / itemsize, itemsize as isize, 0)],
            },
            hash: OnceCell::new(),
            exports: AtomicCell::new(0),
        };
        Ok(zelf)
    }

    #[pymethod]
    fn cast(&self, args: CastArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        self.try_usable(vm)?;
        if !self.desc.is_contiguous() {
            return Err(vm.new_type_error("memoryview: casts are restricted to C-contiguous views"));
        }

        let CastArgs { format, shape } = args;

        if let OptionalArg::Present(shape) = shape {
            if self.desc.is_zero_in_shape() {
                return Err(vm.new_type_error(
                    "memoryview: cannot cast view with zeros in shape or strides",
                ));
            }

            let tup;
            let list;
            let list_borrow;
            let shape = match shape {
                Either::A(shape) => {
                    tup = shape;
                    tup.as_slice()
                }
                Either::B(shape) => {
                    list = shape;
                    list_borrow = list.borrow_vec();
                    &list_borrow
                }
            };

            let shape_ndim = shape.len();
            if shape_ndim > MAX_NDIM {
                return Err(vm.new_value_error(format!(
                    "memoryview: number of dimensions must not exceed {MAX_NDIM}"
                )));
            }
            if self.desc.ndim() != 1 && shape_ndim != 1 {
                return Err(vm.new_type_error("memoryview: cast must be 1D -> ND or ND -> 1D"));
            }

            let mut other = self.cast_to_1d(format, vm)?;
            let itemsize = other.desc.itemsize;

            // 0 ndim is single item, so the buffer has to be that one item
            if shape_ndim == 0 {
                if itemsize != other.desc.len {
                    return Err(
                        vm.new_type_error("memoryview: product(shape) * itemsize != buffer size")
                    );
                }
                other.desc.dim_desc = vec![];
                return Ok(other.into_ref(&vm.ctx));
            }

            let mut product_shape = itemsize;
            let mut dim_descriptor = Vec::with_capacity(shape_ndim);

            for x in shape {
                let x = x
                    .downcast_ref::<PyInt>()
                    .ok_or_else(|| {
                        vm.new_type_error("memoryview.cast(): elements of shape must be integers")
                    })?
                    .try_to_primitive::<usize>(vm)
                    .ok()
                    .filter(|x| *x > 0)
                    .ok_or_else(|| {
                        vm.new_value_error(
                            "memoryview.cast(): elements of shape must be integers > 0",
                        )
                    })?;

                if x > isize::MAX as usize / product_shape {
                    return Err(vm.new_value_error("memoryview.cast(): product(shape) > SSIZE_MAX"));
                }
                product_shape *= x;
                dim_descriptor.push((x, 0, 0));
            }

            dim_descriptor.last_mut().unwrap().1 = itemsize as isize;
            for i in (0..dim_descriptor.len() - 1).rev() {
                dim_descriptor[i].1 = dim_descriptor[i + 1].1 * dim_descriptor[i + 1].0 as isize;
            }

            if product_shape != other.desc.len {
                return Err(
                    vm.new_type_error("memoryview: product(shape) * itemsize != buffer size")
                );
            }

            other.desc.dim_desc = dim_descriptor;

            Ok(other.into_ref(&vm.ctx))
        } else {
            Ok(self.cast_to_1d(format, vm)?.into_ref(&vm.ctx))
        }
    }
}

#[pyclass]
impl Py<PyMemoryView> {
    fn __setitem__(
        &self,
        needle: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        self.try_not_released(vm)?;
        if self.desc.readonly {
            return Err(vm.new_type_error("cannot modify read-only memory"));
        }
        if self.desc.ndim() == 0 {
            // TODO: merge branches when we got conditional if let
            if needle.is(&vm.ctx.ellipsis) {
                return self.pack_single(self.desc.offset as usize, value, vm);
            } else if let Some(tuple) = needle.downcast_ref::<PyTuple>()
                && tuple.is_empty()
            {
                return self.pack_single(self.desc.offset as usize, value, vm);
            }
            return Err(vm.new_type_error("invalid indexing of 0-dim memory"));
        }
        match SubscriptNeedle::try_from_object(vm, needle)? {
            SubscriptNeedle::Index(i) => self.setitem_by_idx(i, value, vm),
            SubscriptNeedle::Slice(slice) => self.setitem_by_slice(&slice, value, vm),
            SubscriptNeedle::MultiIndex(indices) => self.setitem_by_multi_idx(&indices, value, vm),
        }
    }

    #[pymethod]
    fn __reduce_ex__(&self, _proto: usize, vm: &VirtualMachine) -> PyResult {
        self.__reduce__(vm)
    }

    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error("cannot pickle 'memoryview' object"))
    }
}

#[derive(FromArgs)]
struct ToBytesArgs {
    #[pyarg(any, default)]
    order: Option<PyStrRef>,
}

/// The layout a copy of a view is written in.
#[derive(PartialEq, Eq)]
enum Order {
    C,
    Fortran,
    Any,
}

#[derive(FromArgs)]
struct CastArgs {
    #[pyarg(any)]
    format: PyUtf8StrRef,
    #[pyarg(any, optional)]
    shape: OptionalArg<Either<PyTupleRef, PyListRef>>,
}

enum SubscriptNeedle {
    Index(isize),
    Slice(PyRef<PySlice>),
    MultiIndex(Vec<isize>),
    // MultiSlice(Vec<PySliceRef>),
}

/// memory_subscript
///
/// Which kind of key this is follows from the types in it alone, so an item that
/// answers `__index__` but raises on the way reports that rather than making the
/// whole key invalid. is_multiindex / is_multislice
impl TryFromObject for SubscriptNeedle {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        if obj.number().is_index() {
            return Ok(Self::Index(obj.try_index(vm)?.try_to_primitive(vm)?));
        }
        if obj.downcastable::<PySlice>() {
            return Ok(Self::Slice(unsafe { obj.downcast_unchecked::<PySlice>() }));
        }
        if let Some(tuple) = obj.downcast_ref::<PyTuple>() {
            if tuple.iter().all(|x| x.number().is_index()) {
                // ptr_from_tuple: each item is converted where it sits, and the
                // conversion can run Python that releases the view.
                let indices = tuple
                    .iter()
                    .map(|x| x.try_index(vm)?.try_to_primitive::<isize>(vm))
                    .try_collect()?;
                return Ok(Self::MultiIndex(indices));
            }
            if tuple.iter().all(|x| x.downcastable::<PySlice>()) {
                return Err(
                    vm.new_not_implemented_error("multi-dimensional slicing is not implemented")
                );
            }
        }
        Err(vm.new_type_error("memoryview: invalid slice key"))
    }
}

static BUFFER_METHODS: BufferMethods = BufferMethods {
    obj_bytes: |buffer| buffer.obj_as::<PyMemoryView>().buffer.obj_bytes(),
    obj_bytes_mut: |buffer| buffer.obj_as::<PyMemoryView>().buffer.obj_bytes_mut(),
    // memory_releasebuf / memory_getbuf: a consumer's export of this view is a
    // share of the acquisition the view is looking at, and one more reason the
    // view itself cannot be released.
    release: |buffer| {
        let mv = buffer.obj_as::<PyMemoryView>();
        mv.exports.fetch_sub(1);
        mv.buffer.release_share();
    },
    retain: |buffer| {
        let mv = buffer.obj_as::<PyMemoryView>();
        mv.exports.fetch_add(1);
        mv.buffer.retain_share();
    },
};

impl AsBuffer for PyMemoryView {
    const RELEASE_BUFFER: bool = true;

    // memory_getbuf
    fn slot_as_buffer(
        zelf: &PyObject,
        flags: BufferFlags,
        vm: &VirtualMachine,
    ) -> PyResult<PyBuffer> {
        let zelf = zelf
            .downcast_ref::<Self>()
            .ok_or_else(|| vm.new_type_error("unexpected payload for as_buffer"))?;
        zelf.try_usable(vm)?;
        Ok(PyBuffer::new(
            zelf.to_owned().into(),
            zelf.requested_desc(flags, vm)?,
            &BUFFER_METHODS,
        ))
    }

    fn as_buffer(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyBuffer> {
        zelf.try_usable(vm)?;
        // memory_getbuf: *view = *base — the descriptor already says where the
        // view starts.
        Ok(PyBuffer::new(
            zelf.to_owned().into(),
            zelf.desc.clone(),
            &BUFFER_METHODS,
        ))
    }
}

impl AsMapping for PyMemoryView {
    fn as_mapping() -> &'static PyMappingMethods {
        static AS_MAPPING: PyMappingMethods = PyMappingMethods {
            length: atomic_func!(|mapping, vm| PyMemoryView::mapping_downcast(mapping).__len__(vm)),
            subscript: atomic_func!(|mapping, needle, vm| {
                let zelf = PyMemoryView::mapping_downcast(mapping);
                PyMemoryView::__getitem__(zelf.to_owned(), needle.to_owned(), vm)
            }),
            ass_subscript: atomic_func!(|mapping, needle, value, vm| {
                let zelf = PyMemoryView::mapping_downcast(mapping);
                if let Some(value) = value {
                    zelf.__setitem__(needle.to_owned(), value, vm)
                } else {
                    zelf.__delitem__(needle.to_owned(), vm)
                }
            }),
        };
        &AS_MAPPING
    }
}

impl AsSequence for PyMemoryView {
    fn as_sequence() -> &'static PySequenceMethods {
        static AS_SEQUENCE: LazyLock<PySequenceMethods> = LazyLock::new(|| PySequenceMethods {
            length: atomic_func!(|seq, vm| {
                let zelf = PyMemoryView::sequence_downcast(seq);
                zelf.try_not_released(vm)?;
                zelf.__len__(vm)
            }),
            item: atomic_func!(|seq, i, vm| {
                let zelf = PyMemoryView::sequence_downcast(seq);
                zelf.try_not_released(vm)?;
                zelf.getitem_by_idx(i, vm)
            }),
            ..PySequenceMethods::NOT_IMPLEMENTED
        });
        &AS_SEQUENCE
    }
}

impl Comparable for PyMemoryView {
    fn cmp(
        zelf: &Py<Self>,
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
                zelf.class().slot_name(),
                other.class().slot_name()
            ))),
        }
    }
}

impl Hashable for PyMemoryView {
    fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        if let Some(val) = zelf.hash.get() {
            return Ok(*val);
        }
        zelf.try_not_released(vm)?;
        if !zelf.desc.readonly {
            return Err(vm.new_value_error("cannot hash writable memoryview object"));
        }
        // The hash is over the bytes, so it agrees with the hash of the same
        // bytes only where an item is a byte.
        if !Self::native_fmtchar(&zelf.desc.format).is_some_and(is_byte_fmtchar) {
            return Err(
                vm.new_value_error("memoryview: hashing is restricted to formats 'B', 'b' or 'c'")
            );
        }
        // A view is no more hashable than what it looks at, and asking that runs
        // Python, which must not release the memory the hash is taken over.
        // memory_hash
        if !zelf.buffer.obj.downcastable::<PyBufferWindow>() {
            zelf.while_exported(|| zelf.buffer.obj.hash(vm))?;
        }
        let val = zelf.contiguous_or_collect(|bytes| vm.state.hash_secret.hash_bytes(bytes));
        let _ = zelf.hash.set(val);
        Ok(*zelf.hash.get().unwrap())
    }
}

impl PyPayload for PyMemoryView {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.memoryview_type
    }
}

impl Representable for PyMemoryView {
    #[inline]
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        let repr = if zelf.released.load() {
            format!("<released memory at {:#x}>", zelf.get_id())
        } else {
            format!("<memory at {:#x}>", zelf.get_id())
        };
        Ok(repr)
    }
}

pub(crate) fn init(ctx: &'static Context) {
    PyMemoryView::extend_class(ctx, ctx.types.memoryview_type);
    PyMemoryViewIterator::extend_class(ctx, ctx.types.memoryviewiterator_type);
    let wrapper_type = PyBufferWrapper::init_builtin_type();
    // bufferwrapper_as_buffer: bf_releasebuffer and no bf_getbuffer, so the type
    // has `__release_buffer__` but no `__buffer__`.
    wrapper_type.slots.has_release_buffer.store(true);
    PyBufferWrapper::extend_class(ctx, wrapper_type);
    PyBufferWindow::extend_class(ctx, PyBufferWindow::init_builtin_type());
}

#[pyclass(module = false, name = "_buffer_wrapper", traverse)]
#[derive(Debug)]
struct PyBufferWrapper {
    // bw->obj: the object whose `__buffer__` produced the view
    exporter: PyObjectRef,
    // bw->mv: the memoryview `__buffer__` returned, dropped with the last export
    returned_mv: PyMutex<Option<PyRef<PyMemoryView>>>,
    /// Memory of `returned_mv`, held on behalf of every live export. The wrapper
    /// forwards shares of it rather than owning one.
    view: PyBuffer,
    /// Exports handed out for this wrapper; the wrapper is spent at zero.
    #[pytraverse(skip)]
    exports: AtomicCell<usize>,
}

impl PyPayload for PyBufferWrapper {
    fn class(_ctx: &Context) -> &'static Py<PyType> {
        Self::static_type()
    }
}

#[pyclass(flags(DISALLOW_INSTANTIATION))]
impl PyBufferWrapper {}

static BUFFER_WRAPPER_METHODS: BufferMethods = BufferMethods {
    obj_bytes: |buffer| buffer.obj_as::<PyBufferWrapper>().view.obj_bytes(),
    obj_bytes_mut: |buffer| buffer.obj_as::<PyBufferWrapper>().view.obj_bytes_mut(),
    retain: |buffer| {
        let wrapper = buffer.obj_as::<PyBufferWrapper>();
        wrapper.exports.fetch_add(1);
        wrapper.view.retain_share();
    },
    // bufferwrapper_releasebuf
    release: |buffer| {
        let wrapper = buffer.obj_as::<PyBufferWrapper>();
        wrapper.view.release_share();
        if wrapper.exports.fetch_sub(1) != 1 {
            return;
        }
        let Some(mv) = wrapper.returned_mv.lock().take() else {
            return;
        };
        // A native release runs when the memoryview itself is torn down; only a
        // Python-level hook on a foreign exporter has to be called here.
        if !mv.buffer.obj.is(&wrapper.exporter)
            && wrapper.exporter.class().slots.python_release_buffer.load()
        {
            call_python_release_buffer(&wrapper.exporter, mv.clone());
        }
        // Py_CLEAR(bw->mv): the view outlives this only if user code kept it.
        drop(mv);
    },
};

// Read-only window over an exporter, handed to `__release_buffer__`. It owns no
// export, like a `Py_buffer` whose `obj` is NULL, so releasing it is inert and
// cannot recurse back into the hook.
#[pyclass(module = false, name = "_buffer_window", traverse)]
#[derive(Debug)]
struct PyBufferWindow {
    source: PyBuffer,
}

impl PyPayload for PyBufferWindow {
    fn class(_ctx: &Context) -> &'static Py<PyType> {
        Self::static_type()
    }
}

#[pyclass(flags(DISALLOW_INSTANTIATION))]
impl PyBufferWindow {}

static BUFFER_WINDOW_METHODS: BufferMethods = BufferMethods {
    obj_bytes: |buffer| buffer.obj_as::<PyBufferWindow>().source.obj_bytes(),
    obj_bytes_mut: |buffer| buffer.obj_as::<PyBufferWindow>().source.obj_bytes_mut(),
    retain: |_buffer| {},
    release: |_buffer| {},
};

/// The object that ultimately owns the bytes a buffer reads, seen through the
/// payloads that only forward to another export: a view, the wrapper holding what
/// a `__buffer__` returned, and the window handed to `__release_buffer__`.
///
/// Two buffers that resolve to the same object address the same storage, so
/// borrowing one for writing while the other is borrowed for reading would
/// deadlock on it.
fn root_exporter(buffer: &PyBuffer) -> PyObjectRef {
    let mut obj = buffer.obj.clone();
    loop {
        let next = if let Some(view) = obj.downcast_ref::<PyMemoryView>() {
            view.buffer.obj.clone()
        } else if let Some(wrapper) = obj.downcast_ref::<PyBufferWrapper>() {
            wrapper.view.obj.clone()
        } else if let Some(window) = obj.downcast_ref::<PyBufferWindow>() {
            window.source.obj.clone()
        } else {
            return obj;
        };
        obj = next;
    }
}

// slot_bf_getbuffer
pub(crate) fn buffer_from_python_getbuffer(
    obj: &PyObject,
    flags: BufferFlags,
    vm: &VirtualMachine,
) -> PyResult<PyBuffer> {
    let flags_obj = vm.ctx.new_int(flags.bits() as i32);
    let ret = vm.call_special_method(obj, identifier!(vm, __buffer__), (flags_obj,))?;
    let mv = ret
        .downcast::<PyMemoryView>()
        .map_err(|_| vm.new_type_error("__buffer__ returned non-memoryview object"))?;

    // PyObject_GetBuffer(ret, buffer, flags): the returned view has to satisfy
    // the request in its own right.
    mv.try_usable(vm)?;
    let desc = mv.requested_desc(flags, vm)?;
    let wrapper = PyBufferWrapper {
        exporter: obj.to_owned(),
        view: mv.buffer.detached(),
        returned_mv: PyMutex::new(Some(mv)),
        exports: AtomicCell::new(0),
    }
    .into_pyobject(vm);

    // PyBuffer::new retains once through BUFFER_WRAPPER_METHODS.
    Ok(PyBuffer::new(wrapper, desc, &BUFFER_WRAPPER_METHODS))
}

// wrap_releasebuffer
pub(crate) fn release_buffer_from_python(
    obj: &PyObject,
    mv: PyRef<PyMemoryView>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let view_obj = &mv.buffer.obj;
    if view_obj.downcastable::<PyBufferWindow>() {
        // A window exports nothing, so there is nothing left to release, as for
        // a `Py_buffer` whose `obj` is NULL.
        return Ok(());
    }
    let exports_obj = view_obj.is(obj)
        || view_obj
            .downcast_ref::<PyBufferWrapper>()
            .is_some_and(|wrapper| wrapper.exporter.is(obj));
    if !exports_obj {
        return Err(vm.new_value_error("memoryview's buffer is not this object"));
    }
    if mv.released.load() {
        return Err(vm.new_value_error("memoryview's buffer has already been released"));
    }
    mv.release();
    Ok(())
}

// releasebuffer_call_python, for a buffer acquired from a native exporter
pub(crate) fn release_buffer_call_python(buffer: &PyBuffer) {
    crate::vm::thread::try_with_current_vm(|vm| {
        let exporter = buffer.obj.clone();
        let window = PyBufferWindow {
            source: buffer.detached(),
        }
        .into_pyobject(vm);
        let window = PyBuffer::new(window, buffer.desc.clone(), &BUFFER_WINDOW_METHODS);
        let mv = match PyMemoryView::from_buffer(window, vm) {
            Ok(mv) => mv,
            Err(exc) => {
                let msg = format!(
                    "Exception ignored in bf_releasebuffer of {}",
                    exporter.class().name()
                );
                return vm.run_unraisable(exc, Some(msg), vm.ctx.none());
            }
        };
        // Restricted, so user code cannot keep anything addressing the memory
        // that is about to go away.
        mv.restricted.store(true);
        let mv = mv.into_ref(&vm.ctx);
        call_python_release_buffer(&exporter, mv.clone());
        // The window does not outlive the release it was made for.
        mv.release();
    });
}

fn call_python_release_buffer(exporter: &PyObject, mv: PyRef<PyMemoryView>) {
    crate::vm::thread::try_with_current_vm(|vm| {
        let method = vm.get_special_method(exporter, identifier!(vm, __release_buffer__));
        if let Ok(Some(method)) = method
            && let Err(exc) = method.invoke((mv,), vm)
        {
            let msg = format!(
                "Exception ignored in __release_buffer__ of {}",
                exporter.class().name()
            );
            vm.run_unraisable(exc, Some(msg), vm.ctx.none());
        }
    });
}

fn format_unpack(
    format_spec: &FormatSpec,
    bytes: &[u8],
    vm: &VirtualMachine,
) -> PyResult<PyObjectRef> {
    format_spec.unpack(bytes, vm).map(|x| {
        if x.len() == 1 {
            x[0].to_owned()
        } else {
            x.into()
        }
    })
}

/// Whether `ch` names a format whose items are single bytes.
const fn is_byte_fmtchar(ch: u8) -> bool {
    matches!(ch, b'c' | b'b' | b'B')
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
        // if both shape is 0, ignore the rest
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

impl Iterable for PyMemoryView {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(PyMemoryViewIterator {
            internal: PyMutex::new(PositionIterInternal::new(zelf, 0)),
        }
        .into_pyobject(vm))
    }
}

#[pyclass(module = false, name = "memory_iterator")]
#[derive(Debug, Traverse)]
pub(crate) struct PyMemoryViewIterator {
    internal: PyMutex<PositionIterInternal<PyRef<PyMemoryView>>>,
}

impl PyPayload for PyMemoryViewIterator {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.memoryviewiterator_type
    }
}

#[pyclass(flags(DISALLOW_INSTANTIATION), with(IterNext, Iterable))]
impl PyMemoryViewIterator {
    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> PyTupleRef {
        let func = builtins_iter(vm);
        self.internal.lock().reduce(
            func,
            |x| x.clone().into(),
            |vm| vm.ctx.empty_tuple.clone().into(),
            vm,
        )
    }
}

impl SelfIter for PyMemoryViewIterator {}
impl IterNext for PyMemoryViewIterator {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        locked_next(&zelf.internal, |mv, pos| {
            let len = mv.__len__(vm)?;
            Ok(if pos >= len {
                PyIterReturn::StopIteration(None)
            } else {
                PyIterReturn::Return(mv.getitem_by_idx(pos.try_into().unwrap(), vm)?)
            })
        })
    }
}
