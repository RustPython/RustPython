//! mmap module
pub(crate) use mmap::make_module;

#[pymodule]
mod mmap {
    use crate::common::lock::{PyMutex, PyMutexGuard};
    use crate::vm::{
        builtins::{PyInt, PyIntRef, PyTypeRef},
        function::FuncArgs,
        sliceable::saturate_index,
        types::Constructor,
        FromArgs, PyObject, PyPayload, PyRef, PyResult, TryFromBorrowedObject, VirtualMachine,
    };
    use crossbeam_utils::atomic::AtomicCell;
    use memmap2::{Mmap, MmapMut, MmapOptions};
    use nix::unistd;
    use std::fs::File;
    use std::ops::Deref;
    #[cfg(all(unix, not(target_os = "redox")))]
    use std::os::unix::io::{FromRawFd, IntoRawFd, RawFd};

    #[repr(C)]
    #[derive(PartialEq, Eq, Debug)]
    enum AccessMode {
        Default = 0,
        Read = 1,
        Write = 2,
        Copy = 3,
    }

    impl TryFromBorrowedObject for AccessMode {
        fn try_from_borrowed_object(vm: &VirtualMachine, obj: &PyObject) -> PyResult<Self> {
            let i = u32::try_from_borrowed_object(vm, obj)?;
            Ok(match i {
                0 => Self::Default,
                1 => Self::Read,
                2 => Self::Write,
                3 => Self::Copy,
                _ => return Err(vm.new_value_error("Not a valid AccessMode value".to_owned())),
            })
        }
    }

    #[pyattr]
    use libc::{
        MADV_DONTNEED, MADV_NORMAL, MADV_RANDOM, MADV_SEQUENTIAL, MADV_WILLNEED, MAP_ANON,
        MAP_ANONYMOUS, MAP_PRIVATE, MAP_SHARED, PROT_READ, PROT_WRITE,
    };

    #[cfg(target_os = "macos")]
    #[pyattr]
    use libc::{MADV_FREE_REUSABLE, MADV_FREE_REUSE};

    #[cfg(target_os = "linux")]
    #[pyattr]
    use libc::{
        MADV_DODUMP, MADV_DOFORK, MADV_DONTDUMP, MADV_DONTFORK, MADV_FREE, MADV_HUGEPAGE,
        MADV_HWPOISON, MADV_MERGEABLE, MADV_NOHUGEPAGE, MADV_REMOVE, MADV_SOFT_OFFLINE,
        MADV_UNMERGEABLE,
    };

    #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
    #[pyattr]
    use libc::{MAP_DENYWRITE, MAP_EXECUTABLE, MAP_POPULATE};

    #[pyattr]
    const ACCESS_DEFAULT: u32 = AccessMode::Default as u32;
    #[pyattr]
    const ACCESS_READ: u32 = AccessMode::Read as u32;
    #[pyattr]
    const ACCESS_WRITE: u32 = AccessMode::Write as u32;
    #[pyattr]
    const ACCESS_COPY: u32 = AccessMode::Copy as u32;

    #[cfg(all(unix, not(target_arch = "wasm32")))]
    #[pyattr(name = "PAGESIZE", once)]
    fn page_size(_vm: &VirtualMachine) -> usize {
        page_size::get()
    }

    #[cfg(all(unix, not(target_arch = "wasm32")))]
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

    #[pyattr]
    #[pyclass(name = "mmap")]
    #[derive(Debug, PyPayload)]
    struct PyMmap {
        closed: AtomicCell<bool>,
        mmap: PyMutex<Option<MmapObj>>,
        fd: RawFd,
        offset: isize,
        size: AtomicCell<isize>,
        pos: AtomicCell<isize>, // relative to offset
        exports: AtomicCell<usize>,
        access: AccessMode,
    }

    #[derive(FromArgs)]
    struct MmapNewArgs {
        #[pyarg(any)]
        fileno: RawFd,
        #[pyarg(any)]
        length: isize,
        #[pyarg(any, default = "MAP_SHARED")]
        flags: libc::c_int,
        #[pyarg(any, default = "PROT_WRITE|PROT_READ")]
        prot: libc::c_int,
        #[pyarg(any, default = "AccessMode::Default")]
        access: AccessMode,
        #[pyarg(any, default = "0")]
        offset: isize,
    }

    #[derive(FromArgs)]
    pub struct FlushOptions {
        #[pyarg(positional, default)]
        offset: Option<isize>,
        #[pyarg(positional, default)]
        size: Option<isize>,
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

    #[derive(FromArgs)]
    pub struct AdviseOptions {
        #[pyarg(positional)]
        option: libc::c_int,
        #[pyarg(positional, default)]
        start: Option<isize>,
        #[pyarg(positional, default)]
        length: Option<isize>,
    }

    impl Constructor for PyMmap {
        type Args = MmapNewArgs;

        // TODO: Windows is not supported right now.
        #[cfg(all(unix, not(target_os = "redox")))]
        fn py_new(
            cls: PyTypeRef,
            MmapNewArgs {
                fileno: mut fd,
                length,
                flags,
                prot,
                access,
                offset,
            }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            let mut map_size = length;
            if map_size < 0 {
                return Err(
                    vm.new_overflow_error("memory mapped length must be positive".to_owned())
                );
            }

            if offset < 0 {
                return Err(
                    vm.new_overflow_error("memory mapped offset must be positive".to_owned())
                );
            }

            if (access != AccessMode::Default)
                && ((flags != MAP_SHARED) || (prot != (PROT_WRITE | PROT_READ)))
            {
                return Err(vm.new_value_error(
                    "mmap can't specify both access and flags, prot.".to_owned(),
                ));
            }

            // TODO: memmap2 doesn't support mapping with pro and flags right now
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

            if fd != -1 {
                let file = unsafe { File::from_raw_fd(fd) };
                let file_len = match file.metadata() {
                    Ok(m) => m.len().try_into().expect("file size overflow"),
                    Err(e) => return Err(vm.new_os_error(e.to_string())),
                };
                // File::from_raw_fd will consume the fd, so we
                // have to  get it again.
                fd = file.into_raw_fd();
                if map_size == 0 {
                    if file_len == 0 {
                        return Err(vm.new_value_error("cannot mmap an empty file".to_owned()));
                    }

                    if offset > file_len {
                        return Err(
                            vm.new_value_error("mmap offset is greater than file size".to_owned())
                        );
                    }

                    //if file_len - offset > isize::MAX {
                    //    return Err(vm.new_value_error("mmap length is too large".to_owned()));
                    //}

                    map_size = file_len - offset;
                } else if offset > file_len || file_len - offset < map_size {
                    return Err(
                        vm.new_value_error("mmap length is greater than file size".to_owned())
                    );
                }
            }

            let mut mmap_opt = MmapOptions::new();
            let mmap_opt = mmap_opt
                .offset(offset.try_into().unwrap())
                .len(map_size.try_into().unwrap());

            let (fd, mmap) = if fd == -1 {
                (
                    fd,
                    MmapObj::Write(
                        mmap_opt
                            .map_anon()
                            .map_err(|e| vm.new_os_error(e.to_string()))?,
                    ),
                )
            } else {
                let new_fd = unistd::dup(fd).map_err(|e| vm.new_os_error(e.to_string()))?;
                let mmap = match access {
                    AccessMode::Default | AccessMode::Write => MmapObj::Write(
                        unsafe { mmap_opt.map_mut(fd) }
                            .map_err(|e| vm.new_os_error(e.to_string()))?,
                    ),
                    AccessMode::Read => MmapObj::Read(
                        unsafe { mmap_opt.map(fd) }.map_err(|e| vm.new_os_error(e.to_string()))?,
                    ),
                    AccessMode::Copy => MmapObj::Write(
                        unsafe { mmap_opt.map_copy(fd) }
                            .map_err(|e| vm.new_os_error(e.to_string()))?,
                    ),
                };
                (new_fd, mmap)
            };

            let m_obj = Self {
                closed: AtomicCell::new(false),
                mmap: PyMutex::new(Some(mmap)),
                fd,
                offset,
                size: AtomicCell::new(map_size),
                pos: AtomicCell::new(0),
                exports: AtomicCell::new(0),
                access,
            };

            m_obj.into_ref_with_type(vm, cls).map(Into::into)
        }
    }

    #[pyimpl(with(Constructor), flags(BASETYPE))]
    impl PyMmap {
        #[pymethod(magic)]
        pub(crate) fn len(&self) -> usize {
            self.inner_size() as usize
        }

        #[inline]
        fn inner_size(&self) -> isize {
            self.size.load()
        }

        #[inline]
        fn inner_pos(&self) -> isize {
            self.pos.load()
        }

        fn check_valid(&self, vm: &VirtualMachine) -> PyResult<PyMutexGuard<Option<MmapObj>>> {
            let m = self.mmap.lock();

            if m.is_none() {
                return Err(vm.new_value_error("mmap closed or invalid".to_owned()));
            }

            Ok(m)
        }

        #[pyproperty]
        fn closed(&self) -> bool {
            self.closed.load()
        }

        #[pymethod(magic)]
        fn repr(zelf: PyRef<Self>) -> PyResult<String> {
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
                zelf.len(),
                zelf.inner_pos(),
                zelf.offset
            );

            Ok(repr)
        }

        #[pymethod]
        fn close(&self, vm: &VirtualMachine) -> PyResult<()> {
            if self.closed() {
                return Ok(());
            }

            if self.exports.load() > 0 {
                return Err(vm.new_buffer_error("cannot close exported pointers exist.".to_owned()));
            }
            let mut mmap = self.mmap.lock();
            self.closed.store(true);
            *mmap = None;

            Ok(())
        }

        fn get_find_range(&self, options: FindOptions) -> (usize, usize) {
            let pos = self.inner_pos();
            let size = self.inner_size();
            let start = options.start.unwrap_or(pos);
            let end = options.end.unwrap_or(size);

            let size = size.try_into().unwrap();
            (saturate_index(start, size), saturate_index(end, size))
        }

        #[pymethod]
        fn find(&self, options: FindOptions, vm: &VirtualMachine) -> PyResult<PyInt> {
            let (start, end) = self.get_find_range(options.clone());

            let sub = &options.sub;

            if sub.is_empty() {
                return Ok(PyInt::from(0isize));
            }

            let mmap = self.check_valid(vm)?;
            let buf = match mmap.as_ref().unwrap() {
                MmapObj::Read(mmap) => &mmap[start..end],
                MmapObj::Write(mmap) => &mmap[start..end],
            };
            let pos = buf.windows(sub.len()).position(|window| window == sub);

            Ok(pos.map_or(PyInt::from(-1isize), |i| PyInt::from(start + i)))
        }

        #[pymethod]
        fn rfind(&self, options: FindOptions, vm: &VirtualMachine) -> PyResult<PyInt> {
            let (start, end) = self.get_find_range(options.clone());

            let sub = &options.sub;
            if sub.is_empty() {
                return Ok(PyInt::from(0isize));
            }

            let mmap = self.check_valid(vm)?;
            let buf = match mmap.as_ref().unwrap() {
                MmapObj::Read(mmap) => &mmap[start..end],
                MmapObj::Write(mmap) => &mmap[start..end],
            };
            let pos = buf.windows(sub.len()).rposition(|window| window == sub);

            Ok(pos.map_or(PyInt::from(-1isize), |i| PyInt::from(start + i)))
        }

        #[pymethod]
        fn flush(&self, options: FlushOptions, vm: &VirtualMachine) -> PyResult<()> {
            let offset = options.offset.unwrap_or(0);
            let size = options.size.unwrap_or_else(|| self.inner_size());

            if size < 0 || offset < 0 || self.inner_size() - offset < size {
                return Err(vm.new_value_error("flush values out of range".to_owned()));
            }

            let size = size as usize;
            let offset = offset as usize;

            if self.access == AccessMode::Read || self.access == AccessMode::Copy {
                return Ok(());
            }

            match self.check_valid(vm)?.deref().as_ref().unwrap() {
                MmapObj::Read(_mmap) => {}
                MmapObj::Write(mmap) => {
                    mmap.flush_range(offset, size)
                        .map_err(|e| vm.new_os_error(e.to_string()))?;
                }
            }

            Ok(())
        }

        #[pymethod]
        fn size(&self, vm: &VirtualMachine) -> PyResult<PyIntRef> {
            let new_fd = unistd::dup(self.fd).map_err(|e| vm.new_os_error(e.to_string()))?;
            let file = unsafe { File::from_raw_fd(new_fd) };
            let file_len = match file.metadata() {
                Ok(m) => m.len(),
                Err(e) => return Err(vm.new_os_error(e.to_string())),
            };

            Ok(PyInt::from(file_len).into_ref(vm))
        }

        #[pymethod]
        fn tell(&self) -> PyResult<isize> {
            Ok(self.inner_pos())
        }

        #[pymethod(magic)]
        fn enter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            let _m = zelf.check_valid(vm)?;
            Ok(zelf.to_owned())
        }

        #[pymethod(magic)]
        fn exit(zelf: PyRef<Self>, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            zelf.close(vm)
        }
    }
}
