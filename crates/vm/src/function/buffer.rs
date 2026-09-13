use crate::{
    AsObject, PyObject, PyObjectRef, PyResult, TryFromBorrowedObject, TryFromObject,
    VirtualMachine,
    builtins::{PyStr, PyStrRef},
    common::borrow::{BorrowedValue, BorrowedValueMut},
    protocol::{BufferFlags, PyBuffer},
};

// Python/getargs.c

/// any bytes-like object. Like the `y*` format code for `PyArg_Parse` in CPython.
#[derive(Debug, Traverse)]
pub struct ArgBytesLike(PyBuffer);

impl PyObject {
    pub fn try_bytes_like<F, R>(&self, vm: &VirtualMachine, f: F) -> PyResult<R>
    where
        F: FnOnce(&[u8]) -> R,
    {
        let buffer = PyBuffer::from_object(vm, self, BufferFlags::SIMPLE)?;
        buffer
            .as_contiguous()
            .map(|x| f(&x))
            .ok_or_else(|| vm.new_buffer_error("non-contiguous buffer is not a bytes-like object"))
    }

    pub fn try_rw_bytes_like<F, R>(&self, vm: &VirtualMachine, f: F) -> PyResult<R>
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let buffer = PyBuffer::from_object(vm, self, BufferFlags::WRITABLE)?;
        buffer
            .as_contiguous_mut()
            .map(|mut x| f(&mut x))
            .ok_or_else(|| vm.new_type_error("buffer is not a read-write bytes-like object"))
    }
}

impl ArgBytesLike {
    #[must_use]
    pub fn borrow_buf(&self) -> BorrowedValue<'_, [u8]> {
        unsafe { self.0.contiguous_unchecked() }
    }

    pub fn with_ref<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        f(&self.borrow_buf())
    }

    /// The bytes to hand to an operation that may wait, and whatever keeps
    /// them readable while it does.
    ///
    /// `borrow_buf` may answer with a lock that every other thread writing to
    /// the same object waits on, and a thread waiting on a lock never reaches
    /// a safepoint, so keeping one across a wait for a peer, a pipe or a
    /// signal stops the world from being stopped at all. Bytes reached that
    /// way are copied out first. Bytes that lock nothing -- an immutable
    /// object's -- are borrowed where they lie, which is all CPython holds in
    /// either case.
    pub fn borrow_buf_unlocked(&self, vm: &VirtualMachine) -> PyResult<UnlockedBuf<'_>> {
        let borrowed = self.borrow_buf();
        if !borrowed.is_locked() {
            return Ok(UnlockedBuf::Borrowed(borrowed));
        }
        let mut copy = Vec::new();
        copy.try_reserve_exact(borrowed.len())
            .map_err(|_| vm.no_memory_error())?;
        copy.extend_from_slice(&borrowed);
        Ok(UnlockedBuf::Copied(copy))
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.desc.len
    }

    /// The width of one item. Callers that read the buffer as bytes rather
    /// than as whatever it holds have to ask, since a contiguous buffer of
    /// wider items is contiguous all the same.
    #[must_use]
    pub const fn itemsize(&self) -> usize {
        self.0.desc.itemsize
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn as_object(&self) -> &PyObject {
        &self.0.obj
    }

    /// The object whose storage is borrowed while this buffer is read: a view
    /// borrows the object it looks at, not itself.
    #[must_use]
    pub fn source_object(&self) -> &PyObject {
        self.0
            .obj
            .downcast_ref::<crate::builtins::PyMemoryView>()
            .map_or(&self.0.obj, |view| view.viewed_object())
    }
}

impl From<ArgBytesLike> for PyBuffer {
    fn from(buffer: ArgBytesLike) -> Self {
        buffer.0
    }
}

impl From<ArgBytesLike> for PyObjectRef {
    fn from(buffer: ArgBytesLike) -> Self {
        buffer.as_object().to_owned()
    }
}

impl ArgBytesLike {
    fn from_request(vm: &VirtualMachine, obj: &PyObject, flags: BufferFlags) -> PyResult<Self> {
        let buffer = PyBuffer::from_object(vm, obj, flags)?;
        if buffer.desc.is_contiguous() {
            Ok(Self(buffer))
        } else {
            Err(vm.new_buffer_error("non-contiguous buffer is not a bytes-like object"))
        }
    }
}

impl<'a> TryFromBorrowedObject<'a> for ArgBytesLike {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
        Self::from_request(vm, obj, BufferFlags::SIMPLE)
    }
}

/// A bytes-like object asked for as `PyBUF_CONTIG_RO`, which is what a shape is
/// requested with rather than assumed.
#[derive(Debug, Traverse)]
pub struct ArgContiguousBytesLike(ArgBytesLike);

impl core::ops::Deref for ArgContiguousBytesLike {
    type Target = ArgBytesLike;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> TryFromBorrowedObject<'a> for ArgContiguousBytesLike {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
        ArgBytesLike::from_request(vm, obj, BufferFlags::CONTIG_RO).map(Self)
    }
}

/// Bytes that stay readable across a wait, from [`ArgBytesLike::borrow_buf_unlocked`].
#[derive(Debug)]
pub enum UnlockedBuf<'a> {
    Borrowed(BorrowedValue<'a, [u8]>),
    Copied(Vec<u8>),
}

impl core::ops::Deref for UnlockedBuf<'_> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            Self::Borrowed(b) => b,
            Self::Copied(v) => v,
        }
    }
}

/// A memory buffer, read-write access. Like the `w*` format code for `PyArg_Parse` in CPython.
#[derive(Debug, Traverse)]
pub struct ArgMemoryBuffer(PyBuffer);

impl ArgMemoryBuffer {
    #[must_use]
    pub fn borrow_buf_mut(&self) -> BorrowedValueMut<'_, [u8]> {
        unsafe { self.0.contiguous_mut_unchecked() }
    }

    pub fn with_ref<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        f(&mut self.borrow_buf_mut())
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.desc.len
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The object whose storage is borrowed while this buffer is written: a
    /// view borrows the object it looks at, not itself.
    #[must_use]
    pub fn source_object(&self) -> &PyObject {
        self.0
            .obj
            .downcast_ref::<crate::builtins::PyMemoryView>()
            .map_or(&self.0.obj, |view| view.viewed_object())
    }
}

impl From<ArgMemoryBuffer> for PyBuffer {
    fn from(buffer: ArgMemoryBuffer) -> Self {
        buffer.0
    }
}

impl<'a> TryFromBorrowedObject<'a> for ArgMemoryBuffer {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
        let buffer = PyBuffer::from_object(vm, obj, BufferFlags::WRITABLE).map_err(|exc| {
            if obj.check_buffer() {
                // An exporter that cannot serve the request leaves the argument
                // simply the wrong kind of object, as `PyArg_Parse` reports it.
                vm.new_type_error("buffer is not a read-write bytes-like object")
            } else {
                exc
            }
        })?;
        if !buffer.desc.is_contiguous() {
            Err(vm.new_buffer_error("non-contiguous buffer is not a bytes-like object"))
        } else if buffer.desc.readonly {
            Err(vm.new_type_error("buffer is not a read-write bytes-like object"))
        } else {
            Ok(Self(buffer))
        }
    }
}

/// A text string or bytes-like object. Like the `s*` format code for `PyArg_Parse` in CPython.
pub enum ArgStrOrBytesLike {
    Buf(ArgBytesLike),
    Str(PyStrRef),
}

impl ArgStrOrBytesLike {
    #[must_use]
    pub fn as_object(&self) -> &PyObject {
        match self {
            Self::Buf(b) => b.as_object(),
            Self::Str(s) => s.as_object(),
        }
    }
}

impl TryFromObject for ArgStrOrBytesLike {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        obj.downcast()
            .map(Self::Str)
            .or_else(|obj| ArgBytesLike::try_from_object(vm, obj).map(Self::Buf))
    }
}

impl ArgStrOrBytesLike {
    #[must_use]
    pub fn borrow_bytes(&self) -> BorrowedValue<'_, [u8]> {
        match self {
            Self::Buf(b) => b.borrow_buf(),
            Self::Str(s) => s.as_bytes().into(),
        }
    }
}

#[derive(Debug)]
pub enum ArgAsciiBuffer {
    String(PyStrRef),
    Buffer(ArgBytesLike),
}

impl TryFromObject for ArgAsciiBuffer {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        match obj.downcast::<PyStr>() {
            Ok(string) => {
                if string.as_wtf8().is_ascii() {
                    Ok(Self::String(string))
                } else {
                    Err(vm.new_value_error("string argument should contain only ASCII characters"))
                }
            }
            Err(obj) => ArgBytesLike::try_from_object(vm, obj).map(ArgAsciiBuffer::Buffer),
        }
    }
}

impl ArgAsciiBuffer {
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::String(s) => s.as_wtf8().len(),
            Self::Buffer(buffer) => buffer.len(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn with_ref<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        match self {
            Self::String(s) => f(s.as_bytes()),
            Self::Buffer(buffer) => buffer.with_ref(f),
        }
    }
}
