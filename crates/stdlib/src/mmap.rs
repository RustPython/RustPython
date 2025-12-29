// spell-checker:disable
//! mmap module
pub(crate) use mmap::make_module;

#[pymodule]
mod mmap {
    use crate::common::{
        borrow::{BorrowedValue, BorrowedValueMut},
        lock::{MapImmutable, PyMutex, PyMutexGuard},
    };
    use crate::vm::{
        AsObject, FromArgs, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult,
        TryFromBorrowedObject, VirtualMachine, atomic_func,
        builtins::{PyBytes, PyBytesRef, PyInt, PyIntRef, PyType, PyTypeRef},
        byte::{bytes_from_object, value_from_object},
        convert::ToPyException,
        function::{ArgBytesLike, FuncArgs, OptionalArg},
        protocol::{
            BufferDescriptor, BufferMethods, PyBuffer, PyMappingMethods, PySequenceMethods,
        },
        sliceable::{SaturatedSlice, SequenceIndex, SequenceIndexOp},
        types::{AsBuffer, AsMapping, AsSequence, Constructor, Representable},
    };
    use core::ops::{Deref, DerefMut};
    use crossbeam_utils::atomic::AtomicCell;
    use memmap2::{Mmap, MmapMut, MmapOptions};
    use num_traits::Signed;
    use std::io::{self, Write};

    #[cfg(unix)]
    use nix::{sys::stat::fstat, unistd};
    #[cfg(unix)]
    use rustpython_common::crt_fd;

    #[cfg(windows)]
    use rustpython_common::suppress_iph;
    #[cfg(windows)]
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
    #[cfg(windows)]
    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, INVALID_HANDLE_VALUE,
        },
        Storage::FileSystem::{FILE_BEGIN, GetFileSize, SetEndOfFile, SetFilePointerEx},
        System::Threading::GetCurrentProcess,
    };

    #[cfg(unix)]
    fn validate_advice(vm: &VirtualMachine, advice: i32) -> PyResult<i32> {
        match advice {
            libc::MADV_NORMAL
            | libc::MADV_RANDOM
            | libc::MADV_SEQUENTIAL
            | libc::MADV_WILLNEED
            | libc::MADV_DONTNEED => Ok(advice),
            #[cfg(any(
                target_os = "linux",
                target_os = "macos",
                target_os = "ios",
                target_os = "freebsd"
            ))]
            libc::MADV_FREE => Ok(advice),
            #[cfg(target_os = "linux")]
            libc::MADV_DONTFORK
            | libc::MADV_DOFORK
            | libc::MADV_MERGEABLE
            | libc::MADV_UNMERGEABLE
            | libc::MADV_HUGEPAGE
            | libc::MADV_NOHUGEPAGE
            | libc::MADV_REMOVE
            | libc::MADV_DONTDUMP
            | libc::MADV_DODUMP
            | libc::MADV_HWPOISON => Ok(advice),
            #[cfg(target_os = "freebsd")]
            libc::MADV_NOSYNC
            | libc::MADV_AUTOSYNC
            | libc::MADV_NOCORE
            | libc::MADV_CORE
            | libc::MADV_PROTECT => Ok(advice),
            _ => Err(vm.new_value_error("Not a valid Advice value")),
        }
    }

    #[repr(C)]
    #[derive(PartialEq, Eq, Debug)]
    enum AccessMode {
        Default = 0,
        Read = 1,
        Write = 2,
        Copy = 3,
    }

    impl<'a> TryFromBorrowedObject<'a> for AccessMode {
        fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
            let i = u32::try_from_borrowed_object(vm, obj)?;
            Ok(match i {
                0 => Self::Default,
                1 => Self::Read,
                2 => Self::Write,
                3 => Self::Copy,
                _ => return Err(vm.new_value_error("Not a valid AccessMode value")),
            })
        }
    }

    #[cfg(unix)]
    #[pyattr]
    use libc::{
        MADV_DONTNEED, MADV_NORMAL, MADV_RANDOM, MADV_SEQUENTIAL, MADV_WILLNEED, MAP_ANON,
        MAP_ANONYMOUS, MAP_PRIVATE, MAP_SHARED, PROT_EXEC, PROT_READ, PROT_WRITE,
    };

    #[cfg(target_os = "macos")]
    #[pyattr]
    use libc::{MADV_FREE_REUSABLE, MADV_FREE_REUSE};

    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "fuchsia",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "netbsd",
        target_os = "openbsd",
        target_vendor = "apple"
    ))]
    #[pyattr]
    use libc::MADV_FREE;

    #[cfg(target_os = "linux")]
    #[pyattr]
    use libc::{
        MADV_DODUMP, MADV_DOFORK, MADV_DONTDUMP, MADV_DONTFORK, MADV_HUGEPAGE, MADV_HWPOISON,
        MADV_MERGEABLE, MADV_NOHUGEPAGE, MADV_REMOVE, MADV_UNMERGEABLE,
    };

    #[cfg(any(
        target_os = "android",
        all(
            target_os = "linux",
            any(
                target_arch = "aarch64",
                target_arch = "arm",
                target_arch = "powerpc",
                target_arch = "powerpc64",
                target_arch = "s390x",
                target_arch = "x86",
                target_arch = "x86_64",
                target_arch = "sparc64"
            )
        )
    ))]
    #[pyattr]
    use libc::MADV_SOFT_OFFLINE;

    #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
    #[pyattr]
    use libc::{MAP_DENYWRITE, MAP_EXECUTABLE, MAP_POPULATE};

    // MAP_STACK is available on Linux, OpenBSD, and NetBSD
    #[cfg(any(target_os = "linux", target_os = "openbsd", target_os = "netbsd"))]
    #[pyattr]
    use libc::MAP_STACK;

    // FreeBSD-specific MADV constants
    #[cfg(target_os = "freebsd")]
    #[pyattr]
    use libc::{MADV_AUTOSYNC, MADV_CORE, MADV_NOCORE, MADV_NOSYNC, MADV_PROTECT};

    #[pyattr]
    const ACCESS_DEFAULT: u32 = AccessMode::Default as u32;
    #[pyattr]
    const ACCESS_READ: u32 = AccessMode::Read as u32;
    #[pyattr]
    const ACCESS_WRITE: u32 = AccessMode::Write as u32;
    #[pyattr]
    const ACCESS_COPY: u32 = AccessMode::Copy as u32;

    #[cfg(not(target_arch = "wasm32"))]
    #[pyattr(name = "PAGESIZE", once)]
    fn page_size(_vm: &VirtualMachine) -> usize {
        page_size::get()
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[pyattr(name = "ALLOCATIONGRANULARITY", once)]
    fn granularity(_vm: &VirtualMachine) -> usize {
        page_size::get_granularity()
    }

    #[pyattr(name = "error", once)]
    fn error_type(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.exceptions.os_error.to_owned()
    }

    #[derive(Debug)]
    enum MmapObj {
        Write(MmapMut),
        Read(Mmap),
    }

    impl MmapObj {
        fn as_slice(&self) -> &[u8] {
            match self {
                MmapObj::Read(mmap) => &mmap[..],
                MmapObj::Write(mmap) => &mmap[..],
            }
        }
    }

    #[pyattr]
    #[pyclass(name = "mmap")]
    #[derive(Debug, PyPayload)]
    struct PyMmap {
        closed: AtomicCell<bool>,
        mmap: PyMutex<Option<MmapObj>>,
        #[cfg(unix)]
        fd: AtomicCell<i32>,
        #[cfg(windows)]
        handle: AtomicCell<isize>, // HANDLE is isize on Windows
        offset: i64,
        size: AtomicCell<usize>,
        pos: AtomicCell<usize>, // relative to offset
        exports: AtomicCell<usize>,
        access: AccessMode,
    }

    impl PyMmap {
        /// Close the underlying file handle/descriptor if open
        fn close_handle(&self) {
            #[cfg(unix)]
            {
                let fd = self.fd.swap(-1);
                if fd >= 0 {
                    unsafe { libc::close(fd) };
                }
            }
            #[cfg(windows)]
            {
                let handle = self.handle.swap(INVALID_HANDLE_VALUE as isize);
                if handle != INVALID_HANDLE_VALUE as isize {
                    unsafe { CloseHandle(handle as HANDLE) };
                }
            }
        }
    }

    impl Drop for PyMmap {
        fn drop(&mut self) {
            self.close_handle();
        }
    }

    #[cfg(unix)]
    #[derive(FromArgs)]
    struct MmapNewArgs {
        #[pyarg(any)]
        fileno: i32,
        #[pyarg(any)]
        length: isize,
        #[pyarg(any, default = libc::MAP_SHARED)]
        flags: libc::c_int,
        #[pyarg(any, default = libc::PROT_WRITE | libc::PROT_READ)]
        prot: libc::c_int,
        #[pyarg(any, default = AccessMode::Default)]
        access: AccessMode,
        #[pyarg(any, default = 0)]
        offset: i64,
    }

    #[cfg(windows)]
    #[derive(FromArgs)]
    struct MmapNewArgs {
        #[pyarg(any)]
        fileno: i32,
        #[pyarg(any)]
        length: isize,
        #[pyarg(any, default)]
        #[allow(dead_code)]
        tagname: Option<PyObjectRef>,
        #[pyarg(any, default = AccessMode::Default)]
        access: AccessMode,
        #[pyarg(any, default = 0)]
        offset: i64,
    }

    impl MmapNewArgs {
        /// Validate mmap constructor arguments
        fn validate_new_args(&self, vm: &VirtualMachine) -> PyResult<usize> {
            if self.length < 0 {
                return Err(vm.new_overflow_error("memory mapped length must be positive"));
            }
            if self.offset < 0 {
                return Err(vm.new_overflow_error("memory mapped offset must be positive"));
            }
            Ok(self.length as usize)
        }
    }

    #[derive(FromArgs)]
    pub struct FlushOptions {
        #[pyarg(positional, default)]
        offset: Option<isize>,
        #[pyarg(positional, default)]
        size: Option<isize>,
    }

    impl FlushOptions {
        fn values(self, len: usize) -> Option<(usize, usize)> {
            let offset = match self.offset {
                Some(o) if o < 0 => return None,
                Some(o) => o as usize,
                None => 0,
            };

            let size = match self.size {
                Some(s) if s < 0 => return None,
                Some(s) => s as usize,
                None => len,
            };

            if len.checked_sub(offset)? < size {
                return None;
            }

            Some((offset, size))
        }
    }

    #[derive(FromArgs, Clone)]
    pub struct FindOptions {
        #[pyarg(positional)]
        sub: Vec<u8>,
        #[pyarg(positional, default)]
        start: Option<isize>,
        #[pyarg(positional, default)]
        end: Option<isize>,
    }

    #[cfg(all(unix, not(target_os = "redox")))]
    #[derive(FromArgs)]
    pub struct AdviseOptions {
        #[pyarg(positional)]
        option: libc::c_int,
        #[pyarg(positional, default)]
        start: Option<PyIntRef>,
        #[pyarg(positional, default)]
        length: Option<PyIntRef>,
    }

    #[cfg(all(unix, not(target_os = "redox")))]
    impl AdviseOptions {
        fn values(self, len: usize, vm: &VirtualMachine) -> PyResult<(libc::c_int, usize, usize)> {
            let start = self
                .start
                .map(|s| {
                    s.try_to_primitive::<usize>(vm)
                        .ok()
                        .filter(|s| *s < len)
                        .ok_or_else(|| vm.new_value_error("madvise start out of bounds"))
                })
                .transpose()?
                .unwrap_or(0);
            let length = self
                .length
                .map(|s| {
                    s.try_to_primitive::<usize>(vm)
                        .map_err(|_| vm.new_value_error("madvise length invalid"))
                })
                .transpose()?
                .unwrap_or(len);

            if isize::MAX as usize - start < length {
                return Err(vm.new_overflow_error("madvise length too large"));
            }

            let length = if start + length > len {
                len - start
            } else {
                length
            };

            Ok((self.option, start, length))
        }
    }

    impl Constructor for PyMmap {
        type Args = MmapNewArgs;

        #[cfg(unix)]
        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            use libc::{MAP_PRIVATE, MAP_SHARED, PROT_READ, PROT_WRITE};

            let mut map_size = args.validate_new_args(vm)?;
            let MmapNewArgs {
                fileno: fd,
                flags,
                prot,
                access,
                offset,
                ..
            } = args;

            if (access != AccessMode::Default)
                && ((flags != MAP_SHARED) || (prot != (PROT_WRITE | PROT_READ)))
            {
                return Err(vm.new_value_error("mmap can't specify both access and flags, prot."));
            }

            // TODO: memmap2 doesn't support mapping with prot and flags right now
            let (_flags, _prot, access) = match access {
                AccessMode::Read => (MAP_SHARED, PROT_READ, access),
                AccessMode::Write => (MAP_SHARED, PROT_READ | PROT_WRITE, access),
                AccessMode::Copy => (MAP_PRIVATE, PROT_READ | PROT_WRITE, access),
                AccessMode::Default => {
                    let access = if (prot & PROT_READ) != 0 && (prot & PROT_WRITE) != 0 {
                        access
                    } else if (prot & PROT_WRITE) != 0 {
                        AccessMode::Write
                    } else {
                        AccessMode::Read
                    };
                    (flags, prot, access)
                }
            };

            let fd = unsafe { crt_fd::Borrowed::try_borrow_raw(fd) };

            // macOS: Issue #11277: fsync(2) is not enough on OS X - a special, OS X specific
            // fcntl(2) is necessary to force DISKSYNC and get around mmap(2) bug
            #[cfg(target_os = "macos")]
            if let Ok(fd) = fd {
                use std::os::fd::AsRawFd;
                unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_FULLFSYNC) };
            }

            if let Ok(fd) = fd {
                let metadata = fstat(fd)
                    .map_err(|err| io::Error::from_raw_os_error(err as i32).to_pyexception(vm))?;
                let file_len = metadata.st_size as i64;

                if map_size == 0 {
                    if file_len == 0 {
                        return Err(vm.new_value_error("cannot mmap an empty file"));
                    }

                    if offset > file_len {
                        return Err(vm.new_value_error("mmap offset is greater than file size"));
                    }

                    map_size = (file_len - offset)
                        .try_into()
                        .map_err(|_| vm.new_value_error("mmap length is too large"))?;
                } else if offset > file_len || file_len - offset < map_size as i64 {
                    return Err(vm.new_value_error("mmap length is greater than file size"));
                }
            }

            let mut mmap_opt = MmapOptions::new();
            let mmap_opt = mmap_opt.offset(offset as u64).len(map_size);

            let (fd, mmap) = || -> std::io::Result<_> {
                if let Ok(fd) = fd {
                    let new_fd: crt_fd::Owned = unistd::dup(fd)?.into();
                    let mmap = match access {
                        AccessMode::Default | AccessMode::Write => {
                            MmapObj::Write(unsafe { mmap_opt.map_mut(&new_fd) }?)
                        }
                        AccessMode::Read => MmapObj::Read(unsafe { mmap_opt.map(&new_fd) }?),
                        AccessMode::Copy => MmapObj::Write(unsafe { mmap_opt.map_copy(&new_fd) }?),
                    };
                    Ok((Some(new_fd), mmap))
                } else {
                    let mmap = MmapObj::Write(mmap_opt.map_anon()?);
                    Ok((None, mmap))
                }
            }()
            .map_err(|e| e.to_pyexception(vm))?;

            Ok(Self {
                closed: AtomicCell::new(false),
                mmap: PyMutex::new(Some(mmap)),
                fd: AtomicCell::new(fd.map_or(-1, |fd| fd.into_raw())),
                offset,
                size: AtomicCell::new(map_size),
                pos: AtomicCell::new(0),
                exports: AtomicCell::new(0),
                access,
            })
        }

        #[cfg(windows)]
        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let mut map_size = args.validate_new_args(vm)?;
            let MmapNewArgs {
                fileno,
                access,
                offset,
                ..
            } = args;

            // Get file handle from fileno
            // fileno -1 or 0 means anonymous mapping
            let fh: Option<HANDLE> = if fileno != -1 && fileno != 0 {
                // Convert CRT file descriptor to Windows HANDLE
                // Use suppress_iph! to avoid crashes when the fd is invalid.
                // This is critical because socket fds wrapped via _open_osfhandle
                // may cause crashes in _get_osfhandle on Windows.
                // See Python bug https://bugs.python.org/issue30114
                let handle = unsafe { suppress_iph!(libc::get_osfhandle(fileno)) };
                // Check for invalid handle value (-1 on Windows)
                if handle == -1 || handle == INVALID_HANDLE_VALUE as isize {
                    return Err(vm.new_os_error(format!("Invalid file descriptor: {}", fileno)));
                }
                Some(handle as HANDLE)
            } else {
                None
            };

            // Get file size if we have a file handle and map_size is 0
            let mut duplicated_handle: HANDLE = INVALID_HANDLE_VALUE;
            if let Some(fh) = fh {
                // Duplicate handle so Python code can close the original
                let mut new_handle: HANDLE = INVALID_HANDLE_VALUE;
                let result = unsafe {
                    DuplicateHandle(
                        GetCurrentProcess(),
                        fh,
                        GetCurrentProcess(),
                        &mut new_handle,
                        0,
                        0, // not inheritable
                        DUPLICATE_SAME_ACCESS,
                    )
                };
                if result == 0 {
                    return Err(io::Error::last_os_error().to_pyexception(vm));
                }
                duplicated_handle = new_handle;

                // Get file size
                let mut high: u32 = 0;
                let low = unsafe { GetFileSize(fh, &mut high) };
                if low == u32::MAX {
                    let err = io::Error::last_os_error();
                    if err.raw_os_error() != Some(0) {
                        unsafe { CloseHandle(duplicated_handle) };
                        return Err(err.to_pyexception(vm));
                    }
                }
                let file_len = ((high as i64) << 32) | (low as i64);

                if map_size == 0 {
                    if file_len == 0 {
                        unsafe { CloseHandle(duplicated_handle) };
                        return Err(vm.new_value_error("cannot mmap an empty file"));
                    }
                    if offset >= file_len {
                        unsafe { CloseHandle(duplicated_handle) };
                        return Err(vm.new_value_error("mmap offset is greater than file size"));
                    }
                    if file_len - offset > isize::MAX as i64 {
                        unsafe { CloseHandle(duplicated_handle) };
                        return Err(vm.new_value_error("mmap length is too large"));
                    }
                    map_size = (file_len - offset) as usize;
                } else {
                    // If map_size > file_len, extend the file (Windows behavior)
                    let required_size = offset.checked_add(map_size as i64).ok_or_else(|| {
                        unsafe { CloseHandle(duplicated_handle) };
                        vm.new_overflow_error("mmap size would cause file size overflow")
                    })?;
                    if required_size > file_len {
                        // Extend file using SetFilePointerEx + SetEndOfFile
                        let result = unsafe {
                            SetFilePointerEx(
                                duplicated_handle,
                                required_size,
                                std::ptr::null_mut(),
                                FILE_BEGIN,
                            )
                        };
                        if result == 0 {
                            let err = io::Error::last_os_error();
                            unsafe { CloseHandle(duplicated_handle) };
                            return Err(err.to_pyexception(vm));
                        }
                        let result = unsafe { SetEndOfFile(duplicated_handle) };
                        if result == 0 {
                            let err = io::Error::last_os_error();
                            unsafe { CloseHandle(duplicated_handle) };
                            return Err(err.to_pyexception(vm));
                        }
                    }
                }
            }

            let mut mmap_opt = MmapOptions::new();
            let mmap_opt = mmap_opt.offset(offset as u64).len(map_size);

            let (handle, mmap) = if duplicated_handle != INVALID_HANDLE_VALUE {
                // Safety: We just duplicated this handle and it's valid
                let owned_handle =
                    unsafe { OwnedHandle::from_raw_handle(duplicated_handle as RawHandle) };

                let mmap_result = match access {
                    AccessMode::Default | AccessMode::Write => {
                        unsafe { mmap_opt.map_mut(&owned_handle) }.map(MmapObj::Write)
                    }
                    AccessMode::Read => unsafe { mmap_opt.map(&owned_handle) }.map(MmapObj::Read),
                    AccessMode::Copy => {
                        unsafe { mmap_opt.map_copy(&owned_handle) }.map(MmapObj::Write)
                    }
                };

                let mmap = mmap_result.map_err(|e| e.to_pyexception(vm))?;

                // Keep the handle alive
                let raw = owned_handle.as_raw_handle() as isize;
                std::mem::forget(owned_handle);
                (raw, mmap)
            } else {
                // Anonymous mapping
                let mmap = mmap_opt.map_anon().map_err(|e| e.to_pyexception(vm))?;
                (INVALID_HANDLE_VALUE as isize, MmapObj::Write(mmap))
            };

            Ok(Self {
                closed: AtomicCell::new(false),
                mmap: PyMutex::new(Some(mmap)),
                handle: AtomicCell::new(handle),
                offset,
                size: AtomicCell::new(map_size),
                pos: AtomicCell::new(0),
                exports: AtomicCell::new(0),
                access,
            })
        }
    }

    static BUFFER_METHODS: BufferMethods = BufferMethods {
        obj_bytes: |buffer| buffer.obj_as::<PyMmap>().as_bytes(),
        obj_bytes_mut: |buffer| buffer.obj_as::<PyMmap>().as_bytes_mut(),
        release: |buffer| {
            buffer.obj_as::<PyMmap>().exports.fetch_sub(1);
        },
        retain: |buffer| {
            buffer.obj_as::<PyMmap>().exports.fetch_add(1);
        },
    };

    impl AsBuffer for PyMmap {
        fn as_buffer(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyBuffer> {
            let readonly = matches!(zelf.access, AccessMode::Read);
            let buf = PyBuffer::new(
                zelf.to_owned().into(),
                BufferDescriptor::simple(zelf.__len__(), readonly),
                &BUFFER_METHODS,
            );

            Ok(buf)
        }
    }

    impl AsMapping for PyMmap {
        fn as_mapping() -> &'static PyMappingMethods {
            static AS_MAPPING: PyMappingMethods = PyMappingMethods {
                length: atomic_func!(
                    |mapping, _vm| Ok(PyMmap::mapping_downcast(mapping).__len__())
                ),
                subscript: atomic_func!(|mapping, needle, vm| {
                    PyMmap::mapping_downcast(mapping).getitem_inner(needle, vm)
                }),
                ass_subscript: atomic_func!(|mapping, needle, value, vm| {
                    let zelf = PyMmap::mapping_downcast(mapping);
                    if let Some(value) = value {
                        PyMmap::setitem_inner(zelf, needle, value, vm)
                    } else {
                        Err(vm
                            .new_type_error("mmap object doesn't support item deletion".to_owned()))
                    }
                }),
            };
            &AS_MAPPING
        }
    }

    impl AsSequence for PyMmap {
        fn as_sequence() -> &'static PySequenceMethods {
            use std::sync::LazyLock;
            static AS_SEQUENCE: LazyLock<PySequenceMethods> = LazyLock::new(|| PySequenceMethods {
                length: atomic_func!(|seq, _vm| Ok(PyMmap::sequence_downcast(seq).__len__())),
                item: atomic_func!(|seq, i, vm| {
                    let zelf = PyMmap::sequence_downcast(seq);
                    zelf.getitem_by_index(i, vm)
                }),
                ass_item: atomic_func!(|seq, i, value, vm| {
                    let zelf = PyMmap::sequence_downcast(seq);
                    if let Some(value) = value {
                        PyMmap::setitem_by_index(zelf, i, value, vm)
                    } else {
                        Err(vm
                            .new_type_error("mmap object doesn't support item deletion".to_owned()))
                    }
                }),
                ..PySequenceMethods::NOT_IMPLEMENTED
            });
            &AS_SEQUENCE
        }
    }

    #[pyclass(
        with(Constructor, AsMapping, AsSequence, AsBuffer, Representable),
        flags(BASETYPE)
    )]
    impl PyMmap {
        fn as_bytes_mut(&self) -> BorrowedValueMut<'_, [u8]> {
            PyMutexGuard::map(self.mmap.lock(), |m| {
                match m.as_mut().expect("mmap closed or invalid") {
                    MmapObj::Read(_) => panic!("mmap can't modify a readonly memory map."),
                    MmapObj::Write(mmap) => &mut mmap[..],
                }
            })
            .into()
        }

        fn as_bytes(&self) -> BorrowedValue<'_, [u8]> {
            PyMutexGuard::map_immutable(self.mmap.lock(), |m| {
                m.as_ref().expect("mmap closed or invalid").as_slice()
            })
            .into()
        }

        #[pymethod]
        fn __len__(&self) -> usize {
            self.size.load()
        }

        #[inline]
        fn pos(&self) -> usize {
            self.pos.load()
        }

        #[inline]
        fn advance_pos(&self, step: usize) {
            self.pos.store(self.pos() + step);
        }

        #[inline]
        fn try_writable<R>(
            &self,
            vm: &VirtualMachine,
            f: impl FnOnce(&mut MmapMut) -> R,
        ) -> PyResult<R> {
            if matches!(self.access, AccessMode::Read) {
                return Err(vm.new_type_error("mmap can't modify a readonly memory map."));
            }

            match self.check_valid(vm)?.deref_mut().as_mut().unwrap() {
                MmapObj::Write(mmap) => Ok(f(mmap)),
                _ => unreachable!("already check"),
            }
        }

        fn check_valid(&self, vm: &VirtualMachine) -> PyResult<PyMutexGuard<'_, Option<MmapObj>>> {
            let m = self.mmap.lock();

            if m.is_none() {
                return Err(vm.new_value_error("mmap closed or invalid"));
            }

            Ok(m)
        }

        /// TODO: impl resize
        #[allow(dead_code)]
        fn check_resizeable(&self, vm: &VirtualMachine) -> PyResult<()> {
            if self.exports.load() > 0 {
                return Err(vm.new_buffer_error("mmap can't resize with extant buffers exported."));
            }

            if self.access == AccessMode::Write || self.access == AccessMode::Default {
                return Ok(());
            }

            Err(vm.new_type_error("mmap can't resize a readonly or copy-on-write memory map."))
        }

        #[pygetset]
        fn closed(&self) -> bool {
            self.closed.load()
        }

        #[pymethod]
        fn close(&self, vm: &VirtualMachine) -> PyResult<()> {
            if self.closed() {
                return Ok(());
            }

            if self.exports.load() > 0 {
                return Err(vm.new_buffer_error("cannot close exported pointers exist."));
            }

            let mut mmap = self.mmap.lock();
            self.closed.store(true);
            *mmap = None;

            self.close_handle();

            Ok(())
        }

        fn get_find_range(&self, options: FindOptions) -> (usize, usize) {
            let size = self.__len__();
            let start = options
                .start
                .map(|start| start.saturated_at(size))
                .unwrap_or_else(|| self.pos());
            let end = options
                .end
                .map(|end| end.saturated_at(size))
                .unwrap_or(size);
            (start, end)
        }

        #[pymethod]
        fn find(&self, options: FindOptions, vm: &VirtualMachine) -> PyResult<PyInt> {
            let (start, end) = self.get_find_range(options.clone());

            let sub = &options.sub;

            // returns start position for empty string
            if sub.is_empty() {
                return Ok(PyInt::from(start as isize));
            }

            let mmap = self.check_valid(vm)?;
            let buf = &mmap.as_ref().unwrap().as_slice()[start..end];
            let pos = buf.windows(sub.len()).position(|window| window == sub);

            Ok(pos.map_or(PyInt::from(-1isize), |i| PyInt::from(start + i)))
        }

        #[pymethod]
        fn rfind(&self, options: FindOptions, vm: &VirtualMachine) -> PyResult<PyInt> {
            let (start, end) = self.get_find_range(options.clone());

            let sub = &options.sub;
            // returns start position for empty string
            if sub.is_empty() {
                return Ok(PyInt::from(start as isize));
            }

            let mmap = self.check_valid(vm)?;
            let buf = &mmap.as_ref().unwrap().as_slice()[start..end];
            let pos = buf.windows(sub.len()).rposition(|window| window == sub);

            Ok(pos.map_or(PyInt::from(-1isize), |i| PyInt::from(start + i)))
        }

        #[pymethod]
        fn flush(&self, options: FlushOptions, vm: &VirtualMachine) -> PyResult<()> {
            let (offset, size) = options
                .values(self.__len__())
                .ok_or_else(|| vm.new_value_error("flush values out of range"))?;

            if self.access == AccessMode::Read || self.access == AccessMode::Copy {
                return Ok(());
            }

            match self.check_valid(vm)?.deref().as_ref().unwrap() {
                MmapObj::Read(_mmap) => {}
                MmapObj::Write(mmap) => {
                    mmap.flush_range(offset, size)
                        .map_err(|e| e.to_pyexception(vm))?;
                }
            }

            Ok(())
        }

        #[cfg(all(unix, not(target_os = "redox")))]
        #[pymethod]
        fn madvise(&self, options: AdviseOptions, vm: &VirtualMachine) -> PyResult<()> {
            let (option, start, length) = options.values(self.__len__(), vm)?;
            let advice = validate_advice(vm, option)?;

            let guard = self.check_valid(vm)?;
            let mmap = guard.deref().as_ref().unwrap();
            let ptr = match mmap {
                MmapObj::Read(m) => m.as_ptr(),
                MmapObj::Write(m) => m.as_ptr(),
            };

            // Apply madvise to the specified range (start, length)
            let ptr_with_offset = unsafe { ptr.add(start) };
            let result =
                unsafe { libc::madvise(ptr_with_offset as *mut libc::c_void, length, advice) };
            if result != 0 {
                return Err(io::Error::last_os_error().to_pyexception(vm));
            }

            Ok(())
        }

        #[pymethod(name = "move")]
        fn move_(
            &self,
            dest: PyIntRef,
            src: PyIntRef,
            cnt: PyIntRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            fn args(
                dest: PyIntRef,
                src: PyIntRef,
                cnt: PyIntRef,
                size: usize,
                vm: &VirtualMachine,
            ) -> Option<(usize, usize, usize)> {
                if dest.as_bigint().is_negative()
                    || src.as_bigint().is_negative()
                    || cnt.as_bigint().is_negative()
                {
                    return None;
                }
                let dest = dest.try_to_primitive(vm).ok()?;
                let src = src.try_to_primitive(vm).ok()?;
                let cnt = cnt.try_to_primitive(vm).ok()?;
                if size - dest < cnt || size - src < cnt {
                    return None;
                }
                Some((dest, src, cnt))
            }

            let size = self.__len__();
            let (dest, src, cnt) = args(dest, src, cnt, size, vm)
                .ok_or_else(|| vm.new_value_error("source, destination, or count out of range"))?;

            let dest_end = dest + cnt;
            let src_end = src + cnt;

            self.try_writable(vm, |mmap| {
                let src_buf = mmap[src..src_end].to_vec();
                (&mut mmap[dest..dest_end])
                    .write(&src_buf)
                    .map_err(|e| e.to_pyexception(vm))?;
                Ok(())
            })?
        }

        #[pymethod]
        fn read(&self, n: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
            let num_bytes = n
                .map(|obj| {
                    let class = obj.class().to_owned();
                    obj.try_into_value::<Option<isize>>(vm).map_err(|_| {
                        vm.new_type_error(format!(
                            "read argument must be int or None, not {}",
                            class.name()
                        ))
                    })
                })
                .transpose()?
                .flatten();
            let mmap = self.check_valid(vm)?;
            let pos = self.pos();
            let remaining = self.__len__().saturating_sub(pos);
            let num_bytes = num_bytes
                .filter(|&n| n >= 0 && (n as usize) <= remaining)
                .map(|n| n as usize)
                .unwrap_or(remaining);

            let end_pos = pos + num_bytes;
            let bytes = mmap.deref().as_ref().unwrap().as_slice()[pos..end_pos].to_vec();

            let result = PyBytes::from(bytes).into_ref(&vm.ctx);

            self.advance_pos(num_bytes);

            Ok(result)
        }

        #[pymethod]
        fn read_byte(&self, vm: &VirtualMachine) -> PyResult<PyIntRef> {
            let pos = self.pos();
            if pos >= self.__len__() {
                return Err(vm.new_value_error("read byte out of range"));
            }

            let b = self.check_valid(vm)?.deref().as_ref().unwrap().as_slice()[pos];

            self.advance_pos(1);

            Ok(PyInt::from(b).into_ref(&vm.ctx))
        }

        #[pymethod]
        fn readline(&self, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
            let pos = self.pos();
            let mmap = self.check_valid(vm)?;

            let remaining = self.__len__().saturating_sub(pos);
            if remaining == 0 {
                return Ok(PyBytes::from(vec![]).into_ref(&vm.ctx));
            }

            let slice = mmap.as_ref().unwrap().as_slice();
            let eof = slice[pos..].iter().position(|&x| x == b'\n');

            let end_pos = if let Some(i) = eof {
                pos + i + 1
            } else {
                self.__len__()
            };

            let bytes = slice[pos..end_pos].to_vec();

            let result = PyBytes::from(bytes).into_ref(&vm.ctx);

            self.advance_pos(end_pos - pos);

            Ok(result)
        }

        #[cfg(unix)]
        #[pymethod]
        fn resize(&self, _newsize: PyIntRef, vm: &VirtualMachine) -> PyResult<()> {
            self.check_resizeable(vm)?;
            // TODO: implement using mremap on Linux
            Err(vm.new_system_error("mmap: resizing not available--no mremap()"))
        }

        #[cfg(windows)]
        #[pymethod]
        fn resize(&self, newsize: PyIntRef, vm: &VirtualMachine) -> PyResult<()> {
            self.check_resizeable(vm)?;

            let newsize: usize = newsize
                .try_to_primitive(vm)
                .map_err(|_| vm.new_value_error("new size out of range"))?;

            if newsize == 0 {
                return Err(vm.new_value_error("new size must be positive"));
            }

            let handle = self.handle.load();
            let is_anonymous = handle == INVALID_HANDLE_VALUE as isize;

            // Get the lock on mmap
            let mut mmap_guard = self.mmap.lock();

            if is_anonymous {
                // For anonymous mmap, we need to:
                // 1. Create a new anonymous mmap with the new size
                // 2. Copy data from old mmap to new mmap
                // 3. Replace the old mmap

                let old_size = self.size.load();
                let copy_size = core::cmp::min(old_size, newsize);

                // Create new anonymous mmap
                let mut new_mmap_opts = MmapOptions::new();
                let mut new_mmap = new_mmap_opts
                    .len(newsize)
                    .map_anon()
                    .map_err(|e| e.to_pyexception(vm))?;

                // Copy data from old mmap to new mmap
                if let Some(old_mmap) = mmap_guard.as_ref() {
                    let src = match old_mmap {
                        MmapObj::Write(m) => &m[..copy_size],
                        MmapObj::Read(m) => &m[..copy_size],
                    };
                    new_mmap[..copy_size].copy_from_slice(src);
                }

                *mmap_guard = Some(MmapObj::Write(new_mmap));
                self.size.store(newsize);
            } else {
                // File-backed mmap resize

                // Drop the current mmap to release the file mapping
                *mmap_guard = None;

                // Resize the file
                let required_size = self.offset + newsize as i64;
                let result = unsafe {
                    SetFilePointerEx(
                        handle as HANDLE,
                        required_size,
                        std::ptr::null_mut(),
                        FILE_BEGIN,
                    )
                };
                if result == 0 {
                    // Restore original mmap on error
                    let err = io::Error::last_os_error();
                    self.try_restore_mmap(&mut mmap_guard, handle as HANDLE, self.size.load());
                    return Err(err.to_pyexception(vm));
                }

                let result = unsafe { SetEndOfFile(handle as HANDLE) };
                if result == 0 {
                    let err = io::Error::last_os_error();
                    self.try_restore_mmap(&mut mmap_guard, handle as HANDLE, self.size.load());
                    return Err(err.to_pyexception(vm));
                }

                // Create new mmap with the new size
                let new_mmap =
                    Self::create_mmap_windows(handle as HANDLE, self.offset, newsize, &self.access)
                        .map_err(|e| e.to_pyexception(vm))?;

                *mmap_guard = Some(new_mmap);
                self.size.store(newsize);
            }

            // Adjust position if it's beyond the new size
            let pos = self.pos.load();
            if pos > newsize {
                self.pos.store(newsize);
            }

            Ok(())
        }

        #[pymethod]
        fn seek(
            &self,
            dist: isize,
            whence: OptionalArg<libc::c_int>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let how = whence.unwrap_or(0);
            let size = self.__len__();

            let new_pos = match how {
                0 => dist, // relative to start
                1 => {
                    // relative to current position
                    let pos = self.pos();
                    if (((isize::MAX as usize) - pos) as isize) < dist {
                        return Err(vm.new_value_error("seek out of range"));
                    }
                    pos as isize + dist
                }
                2 => {
                    // relative to end
                    if (((isize::MAX as usize) - size) as isize) < dist {
                        return Err(vm.new_value_error("seek out of range"));
                    }
                    size as isize + dist
                }
                _ => return Err(vm.new_value_error("unknown seek type")),
            };

            if new_pos < 0 || (new_pos as usize) > size {
                return Err(vm.new_value_error("seek out of range"));
            }

            self.pos.store(new_pos as usize);

            Ok(())
        }

        #[cfg(unix)]
        #[pymethod]
        fn size(&self, vm: &VirtualMachine) -> std::io::Result<PyIntRef> {
            let fd = unsafe { crt_fd::Borrowed::try_borrow_raw(self.fd.load())? };
            let file_len = fstat(fd)?.st_size;
            Ok(PyInt::from(file_len).into_ref(&vm.ctx))
        }

        #[cfg(windows)]
        #[pymethod]
        fn size(&self, vm: &VirtualMachine) -> PyResult<PyIntRef> {
            let handle = self.handle.load();
            if handle == INVALID_HANDLE_VALUE as isize {
                // Anonymous mapping, return the mmap size
                return Ok(PyInt::from(self.__len__()).into_ref(&vm.ctx));
            }

            let mut high: u32 = 0;
            let low = unsafe { GetFileSize(handle as HANDLE, &mut high) };
            if low == u32::MAX {
                let err = io::Error::last_os_error();
                if err.raw_os_error() != Some(0) {
                    return Err(err.to_pyexception(vm));
                }
            }
            let file_len = ((high as i64) << 32) | (low as i64);
            Ok(PyInt::from(file_len).into_ref(&vm.ctx))
        }

        #[pymethod]
        fn tell(&self) -> PyResult<usize> {
            Ok(self.pos())
        }

        #[pymethod]
        fn write(&self, bytes: ArgBytesLike, vm: &VirtualMachine) -> PyResult<PyIntRef> {
            let pos = self.pos();
            let size = self.__len__();

            let data = bytes.borrow_buf();

            if pos > size || size - pos < data.len() {
                return Err(vm.new_value_error("data out of range"));
            }

            let len = self.try_writable(vm, |mmap| {
                (&mut mmap[pos..(pos + data.len())])
                    .write(&data)
                    .map_err(|err| err.to_pyexception(vm))?;
                Ok(data.len())
            })??;

            self.advance_pos(len);

            Ok(PyInt::from(len).into_ref(&vm.ctx))
        }

        #[pymethod]
        fn write_byte(&self, byte: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let b = value_from_object(vm, &byte)?;

            let pos = self.pos();
            let size = self.__len__();

            if pos >= size {
                return Err(vm.new_value_error("write byte out of range"));
            }

            self.try_writable(vm, |mmap| {
                mmap[pos] = b;
            })?;

            self.advance_pos(1);

            Ok(())
        }

        #[pymethod]
        fn __getitem__(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            self.getitem_inner(&needle, vm)
        }

        #[pymethod]
        fn __setitem__(
            zelf: &Py<Self>,
            needle: PyObjectRef,
            value: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            Self::setitem_inner(zelf, &needle, value, vm)
        }

        #[pymethod]
        fn __enter__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            let _m = zelf.check_valid(vm)?;
            Ok(zelf.to_owned())
        }

        #[pymethod]
        fn __exit__(zelf: &Py<Self>, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            zelf.close(vm)
        }

        #[cfg(windows)]
        #[pymethod]
        fn __sizeof__(&self) -> usize {
            std::mem::size_of::<Self>()
        }
    }

    impl PyMmap {
        #[cfg(windows)]
        fn create_mmap_windows(
            handle: HANDLE,
            offset: i64,
            size: usize,
            access: &AccessMode,
        ) -> io::Result<MmapObj> {
            use std::fs::File;

            // Create an owned handle wrapper for memmap2
            // We need to create a File from the handle
            let file = unsafe { File::from_raw_handle(handle as RawHandle) };

            let mut mmap_opt = MmapOptions::new();
            let mmap_opt = mmap_opt.offset(offset as u64).len(size);

            let result = match access {
                AccessMode::Default | AccessMode::Write => {
                    unsafe { mmap_opt.map_mut(&file) }.map(MmapObj::Write)
                }
                AccessMode::Read => unsafe { mmap_opt.map(&file) }.map(MmapObj::Read),
                AccessMode::Copy => unsafe { mmap_opt.map_copy(&file) }.map(MmapObj::Write),
            };

            // Don't close the file handle - we're borrowing it
            std::mem::forget(file);

            result
        }

        /// Try to restore mmap after a failed resize operation.
        /// Returns true if restoration succeeded, false otherwise.
        /// If restoration fails, marks the mmap as closed.
        #[cfg(windows)]
        fn try_restore_mmap(&self, mmap_guard: &mut Option<MmapObj>, handle: HANDLE, size: usize) {
            match Self::create_mmap_windows(handle, self.offset, size, &self.access) {
                Ok(mmap) => *mmap_guard = Some(mmap),
                Err(_) => self.closed.store(true),
            }
        }

        fn getitem_by_index(&self, i: isize, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let i = i
                .wrapped_at(self.__len__())
                .ok_or_else(|| vm.new_index_error("mmap index out of range"))?;

            let b = self.check_valid(vm)?.deref().as_ref().unwrap().as_slice()[i];

            Ok(PyInt::from(b).into_ref(&vm.ctx).into())
        }

        fn getitem_by_slice(
            &self,
            slice: &SaturatedSlice,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            let (range, step, slice_len) = slice.adjust_indices(self.__len__());

            let mmap = self.check_valid(vm)?;
            let slice_data = mmap.deref().as_ref().unwrap().as_slice();

            if slice_len == 0 {
                return Ok(PyBytes::from(vec![]).into_ref(&vm.ctx).into());
            } else if step == 1 {
                return Ok(PyBytes::from(slice_data[range].to_vec())
                    .into_ref(&vm.ctx)
                    .into());
            }

            let mut result_buf = Vec::with_capacity(slice_len);
            if step.is_negative() {
                for i in range.rev().step_by(step.unsigned_abs()) {
                    result_buf.push(slice_data[i]);
                }
            } else {
                for i in range.step_by(step.unsigned_abs()) {
                    result_buf.push(slice_data[i]);
                }
            }
            Ok(PyBytes::from(result_buf).into_ref(&vm.ctx).into())
        }

        fn getitem_inner(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            match SequenceIndex::try_from_borrowed_object(vm, needle, "mmap")? {
                SequenceIndex::Int(i) => self.getitem_by_index(i, vm),
                SequenceIndex::Slice(slice) => self.getitem_by_slice(&slice, vm),
            }
        }

        fn setitem_inner(
            zelf: &Py<Self>,
            needle: &PyObject,
            value: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            match SequenceIndex::try_from_borrowed_object(vm, needle, "mmap")? {
                SequenceIndex::Int(i) => Self::setitem_by_index(zelf, i, value, vm),
                SequenceIndex::Slice(slice) => Self::setitem_by_slice(zelf, &slice, value, vm),
            }
        }

        fn setitem_by_index(
            &self,
            i: isize,
            value: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let i: usize = i
                .wrapped_at(self.__len__())
                .ok_or_else(|| vm.new_index_error("mmap index out of range"))?;

            let b = value_from_object(vm, &value)?;

            self.try_writable(vm, |mmap| {
                mmap[i] = b;
            })?;

            Ok(())
        }

        fn setitem_by_slice(
            &self,
            slice: &SaturatedSlice,
            value: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let (range, step, slice_len) = slice.adjust_indices(self.__len__());

            let bytes = bytes_from_object(vm, &value)?;

            if bytes.len() != slice_len {
                return Err(vm.new_index_error("mmap slice assignment is wrong size"));
            }

            if slice_len == 0 {
                // do nothing
                Ok(())
            } else if step == 1 {
                self.try_writable(vm, |mmap| {
                    (&mut mmap[range])
                        .write(&bytes)
                        .map_err(|err| err.to_pyexception(vm))?;
                    Ok(())
                })?
            } else {
                let mut bi = 0; // bytes index
                if step.is_negative() {
                    for i in range.rev().step_by(step.unsigned_abs()) {
                        self.try_writable(vm, |mmap| {
                            mmap[i] = bytes[bi];
                        })?;
                        bi += 1;
                    }
                } else {
                    for i in range.step_by(step.unsigned_abs()) {
                        self.try_writable(vm, |mmap| {
                            mmap[i] = bytes[bi];
                        })?;
                        bi += 1;
                    }
                }
                Ok(())
            }
        }
    }

    impl Representable for PyMmap {
        #[inline]
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            let mmap = zelf.mmap.lock();

            if mmap.is_none() {
                return Ok("<mmap.mmap closed=True>".to_owned());
            }

            let access_str = match zelf.access {
                AccessMode::Default => "ACCESS_DEFAULT",
                AccessMode::Read => "ACCESS_READ",
                AccessMode::Write => "ACCESS_WRITE",
                AccessMode::Copy => "ACCESS_COPY",
            };

            let repr = format!(
                "<mmap.mmap closed=False, access={}, length={}, pos={}, offset={}>",
                access_str,
                zelf.__len__(),
                zelf.pos(),
                zelf.offset
            );

            Ok(repr)
        }
    }
}
