//! mmap module
pub(crate) use mmap::make_module;

#[pymodule]
mod mmap {
    use crate::common::{
        borrow::{BorrowedValue, BorrowedValueMut},
        lock::{MapImmutable, PyMutex, PyMutexGuard},
    };
    use crate::vm::{
        builtins::{PyBytes, PyBytesRef, PyInt, PyIntRef, PyTypeRef},
        byte::{bytes_from_object, value_from_object},
        function::{ArgBytesLike, FuncArgs, OptionalArg},
        protocol::{
            BufferDescriptor, BufferMethods, PyBuffer, PyMappingMethods, PySequenceMethods,
        },
        sliceable::{saturate_index, wrap_index, SaturatedSlice, SequenceIndex},
        types::{AsBuffer, AsMapping, AsSequence, Constructor},
        AsObject, FromArgs, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult,
        TryFromBorrowedObject, VirtualMachine,
    };
    use crossbeam_utils::atomic::AtomicCell;
    use memmap2::{Advice, Mmap, MmapMut, MmapOptions};
    use nix::unistd;
    use std::fs::File;
    use std::io::Write;
    use std::ops::{Deref, DerefMut};
    #[cfg(all(unix, not(target_os = "redox")))]
    use std::os::unix::io::{FromRawFd, IntoRawFd, RawFd};

    fn advice_try_from_i32(vm: &VirtualMachine, i: i32) -> PyResult<Advice> {
        Ok(match i {
            libc::MADV_NORMAL => Advice::Normal,
            libc::MADV_RANDOM => Advice::Random,
            libc::MADV_SEQUENTIAL => Advice::Sequential,
            libc::MADV_WILLNEED => Advice::WillNeed,
            libc::MADV_DONTNEED => Advice::DontNeed,
            #[cfg(any(target_os = "linux", target_os = "macos", target_os = "ios"))]
            libc::MADV_FREE => Advice::Free,
            #[cfg(target_os = "linux")]
            libc::MADV_DONTFORK => Advice::DontFork,
            #[cfg(target_os = "linux")]
            libc::MADV_DOFORK => Advice::DoFork,
            #[cfg(target_os = "linux")]
            libc::MADV_MERGEABLE => Advice::Mergeable,
            #[cfg(target_os = "linux")]
            libc::MADV_UNMERGEABLE => Advice::Unmergeable,
            #[cfg(target_os = "linux")]
            libc::MADV_HUGEPAGE => Advice::HugePage,
            #[cfg(target_os = "linux")]
            libc::MADV_NOHUGEPAGE => Advice::NoHugePage,
            #[cfg(target_os = "linux")]
            libc::MADV_REMOVE => Advice::Remove,
            #[cfg(target_os = "linux")]
            libc::MADV_DONTDUMP => Advice::DontDump,
            #[cfg(target_os = "linux")]
            libc::MADV_DODUMP => Advice::DoDump,
            #[cfg(target_os = "linux")]
            libc::MADV_HWPOISON => Advice::HwPoison,
            _ => return Err(vm.new_value_error("Not a valid Advice value".to_owned())),
        })
    }

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
        MADV_MERGEABLE, MADV_NOHUGEPAGE, MADV_REMOVE, MADV_SOFT_OFFLINE, MADV_UNMERGEABLE,
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
            let buf = PyBuffer::new(
                zelf.to_owned().into(),
                BufferDescriptor::simple(zelf.len(), true),
                &BUFFER_METHODS,
            );

            Ok(buf)
        }
    }

    impl AsMapping for PyMmap {
        const AS_MAPPING: PyMappingMethods = PyMappingMethods {
            length: Some(|mapping, _vm| Ok(Self::mapping_downcast(mapping).len())),
            subscript: Some(|mapping, needle, vm| {
                Self::mapping_downcast(mapping)._getitem(needle, vm)
            }),
            ass_subscript: Some(|mapping, needle, value, vm| {
                let zelf = Self::mapping_downcast(mapping);
                if let Some(value) = value {
                    Self::_setitem(zelf.to_owned(), needle, value, vm)
                } else {
                    Err(vm.new_type_error("mmap object doesn't support item deletion".to_owned()))
                }
            }),
        };
    }

    impl AsSequence for PyMmap {
        const AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
            length: Some(|seq, _vm| Ok(Self::sequence_downcast(seq).len())),
            item: Some(|seq, i, vm| {
                let zelf = Self::sequence_downcast(seq);
                zelf.get_item_by_index(i, vm)
            }),
            ass_item: Some(|seq, i, value, vm| {
                let zelf = Self::sequence_downcast(seq);
                if let Some(value) = value {
                    Self::setitem_by_index(zelf.to_owned(), i, value, vm)
                } else {
                    Err(vm.new_type_error("mmap object doesn't support item deletion".to_owned()))
                }
            }),
            ..PySequenceMethods::NOT_IMPLEMENTED
        };
    }

    #[pyimpl(with(Constructor, AsMapping, AsSequence, AsBuffer), flags(BASETYPE))]
    impl PyMmap {
        fn as_bytes_mut(&self) -> BorrowedValueMut<[u8]> {
            PyMutexGuard::map(self.mmap.lock(), |m| {
                match m.as_mut().expect("mmap closed or invalid") {
                    MmapObj::Read(_) => panic!("mmap can't modify a readonly memory map."),
                    MmapObj::Write(mmap) => &mut mmap[..],
                }
            })
            .into()
        }

        fn as_bytes(&self) -> BorrowedValue<[u8]> {
            PyMutexGuard::map_immutable(self.mmap.lock(), |m| {
                match m.as_ref().expect("mmap closed or invalid") {
                    MmapObj::Read(ref mmap) => &mmap[..],
                    MmapObj::Write(ref mmap) => &mmap[..],
                }
            })
            .into()
        }

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

        #[inline]
        fn advance_pos(&self, step: isize) {
            self.pos.store(self.inner_pos() + step);
        }

        #[inline]
        fn try_writable<R>(
            &self,
            vm: &VirtualMachine,
            f: impl FnOnce(&mut MmapMut) -> R,
        ) -> PyResult<R> {
            if matches!(self.access, AccessMode::Read) {
                return Err(
                    vm.new_type_error("mmap can't modify a readonly memory map.".to_owned())
                );
            }

            match self.check_valid(vm)?.deref_mut().as_mut().unwrap() {
                MmapObj::Write(mmap) => Ok(f(mmap)),
                _ => unreachable!("already check"),
            }
        }

        fn check_valid(&self, vm: &VirtualMachine) -> PyResult<PyMutexGuard<Option<MmapObj>>> {
            let m = self.mmap.lock();

            if m.is_none() {
                return Err(vm.new_value_error("mmap closed or invalid".to_owned()));
            }

            Ok(m)
        }

        /// TODO: impl resize
        #[allow(dead_code)]
        fn check_resizeable(&self, vm: &VirtualMachine) -> PyResult<()> {
            if self.exports.load() > 0 {
                return Err(vm.new_buffer_error(
                    "mmap can't resize with extant buffers exported.".to_owned(),
                ));
            }

            if self.access == AccessMode::Write || self.access == AccessMode::Default {
                return Ok(());
            }

            Err(vm.new_type_error(
                "mmap can't resize a readonly or copy-on-write memory map.".to_owned(),
            ))
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

        #[allow(unused_assignments)]
        #[pymethod]
        fn madvise(&self, options: AdviseOptions, vm: &VirtualMachine) -> PyResult<()> {
            let start = options.start.unwrap_or(0);
            let mut length = options.length.unwrap_or_else(|| self.inner_size());

            if start < 0 || start >= self.inner_size() {
                return Err(vm.new_value_error("madvise start out of bounds".to_owned()));
            }
            if length < 0 {
                return Err(vm.new_value_error("madvise length invalid".to_owned()));
            }

            if isize::MAX - start < length {
                return Err(vm.new_overflow_error("madvise length too large".to_owned()));
            }

            if start + length > self.inner_size() {
                length = self.inner_size() - start;
            }

            let advice = advice_try_from_i32(vm, options.option)?;

            //TODO: memmap2 doesn't support madvise range right now.
            match self.check_valid(vm)?.deref().as_ref().unwrap() {
                MmapObj::Read(mmap) => mmap.advise(advice),
                MmapObj::Write(mmap) => mmap.advise(advice),
            }
            .map_err(|e| vm.new_os_error(e.to_string()))?;

            Ok(())
        }

        #[pymethod(name = "move")]
        fn move_(&self, dest: isize, src: isize, cnt: isize, vm: &VirtualMachine) -> PyResult<()> {
            let size = self.inner_size();
            if dest < 0 || src < 0 || cnt < 0 || size - dest < cnt || size - src < cnt {
                return Err(
                    vm.new_value_error("source, destination, or count out of range".to_owned())
                );
            }

            let dest: usize = dest.try_into().unwrap();
            let cnt: usize = cnt.try_into().unwrap();
            let dest_end = dest + cnt;
            let src: usize = src.try_into().unwrap();
            let src_end = src + cnt;

            self.try_writable(vm, |mmap| {
                let src_buf = mmap[src..src_end].to_vec();
                (&mut mmap[dest..dest_end])
                    .write(&src_buf)
                    .map_err(|e| vm.new_os_error(e.to_string()))?;
                Ok(())
            })?
        }

        #[pymethod]
        fn read(&self, n: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
            let mut num_bytes = n
                .map(|obj| {
                    let name = obj.class().name().to_string();
                    obj.try_into_value::<Option<isize>>(vm).map_err(|_| {
                        vm.new_type_error(format!(
                            "read argument must be int or None, not {}",
                            name,
                        ))
                    })
                })
                .transpose()?
                .flatten()
                .unwrap_or(isize::MAX);
            let mmap = self.check_valid(vm)?;
            let pos = self.inner_pos();

            let remaining = if pos < self.inner_size() {
                self.inner_size() - pos
            } else {
                0
            };

            if num_bytes < 0 || num_bytes > remaining {
                num_bytes = remaining;
            }

            let end_pos = (pos + num_bytes) as usize;
            let bytes = match mmap.deref().as_ref().unwrap() {
                MmapObj::Read(mmap) => mmap[pos as usize..end_pos].to_vec(),
                MmapObj::Write(mmap) => mmap[pos as usize..end_pos].to_vec(),
            };

            let result = PyBytes::from(bytes).into_ref(vm);

            self.advance_pos(num_bytes);

            Ok(result)
        }

        #[pymethod]
        fn read_byte(&self, vm: &VirtualMachine) -> PyResult<PyIntRef> {
            let pos = self.inner_pos();
            if pos >= self.inner_size() {
                return Err(vm.new_value_error("read byte out of range".to_owned()));
            }

            let b = match self.check_valid(vm)?.deref().as_ref().unwrap() {
                MmapObj::Read(mmap) => mmap[pos as usize],
                MmapObj::Write(mmap) => mmap[pos as usize],
            };

            self.advance_pos(1);

            Ok(PyInt::from(b).into_ref(vm))
        }

        #[pymethod]
        fn readline(&self, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
            let pos = self.inner_pos();
            let mmap = self.check_valid(vm)?;

            let remaining = if pos < self.inner_size() {
                self.inner_size() - pos
            } else {
                0
            };

            if remaining == 0 {
                return Ok(PyBytes::from(vec![]).into_ref(vm));
            }

            let eof = match mmap.as_ref().unwrap() {
                MmapObj::Read(mmap) => &mmap[pos as usize..],
                MmapObj::Write(mmap) => &mmap[pos as usize..],
            }
            .iter()
            .position(|&x| x == b'\n');

            let end_pos = if let Some(i) = eof {
                pos as usize + i + 1
            } else {
                self.inner_size() as usize
            };

            let bytes = match mmap.deref().as_ref().unwrap() {
                MmapObj::Read(mmap) => mmap[pos as usize..end_pos].to_vec(),
                MmapObj::Write(mmap) => mmap[pos as usize..end_pos].to_vec(),
            };

            let result = PyBytes::from(bytes).into_ref(vm);

            self.advance_pos(end_pos as isize - pos);

            Ok(result)
        }

        //TODO: supports resize
        #[pymethod]
        fn resize(&self, _newsize: PyIntRef, vm: &VirtualMachine) -> PyResult<()> {
            self.check_resizeable(vm)?;
            Err(vm.new_system_error("mmap: resizing not available--no mremap()".to_owned()))
        }

        #[pymethod]
        fn seek(
            &self,
            pos: isize,
            whence: OptionalArg<libc::c_int>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let dist = pos;

            let how = whence.unwrap_or(0);
            let size = self.inner_size();

            let new_pos = match how {
                0 => dist, // relative to start
                1 => {
                    // relative to current position
                    let pos = self.inner_pos();
                    if isize::MAX - pos < dist {
                        return Err(vm.new_value_error("seek out of range".to_owned()));
                    }
                    pos + dist
                }
                2 => {
                    // relative to end
                    if isize::MAX - size < dist {
                        return Err(vm.new_value_error("seek out of range".to_owned()));
                    }
                    size + dist
                }
                _ => return Err(vm.new_value_error("unknown seek type".to_owned())),
            };

            if new_pos > size || new_pos < 0 {
                return Err(vm.new_value_error("seek out of range".to_owned()));
            }

            self.pos.store(new_pos);

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

        #[pymethod]
        fn write(&self, bytes: ArgBytesLike, vm: &VirtualMachine) -> PyResult<PyIntRef> {
            let pos = self.inner_pos();
            let size = self.inner_size();

            let data = bytes.borrow_buf();

            if pos > size || size - pos < data.len() as isize {
                return Err(vm.new_value_error("data out of range".to_owned()));
            }

            let len = self.try_writable(vm, |mmap| {
                (&mut mmap[pos as usize..(pos as usize + data.len())])
                    .write(&data)
                    .map_err(|e| vm.new_os_error(e.to_string()))?;
                Ok(data.len())
            })??;

            self.advance_pos(len as isize);

            Ok(PyInt::from(len).into_ref(vm))
        }

        #[pymethod]
        fn write_byte(&self, byte: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let b = value_from_object(vm, &byte)?;

            let pos = self.inner_pos();
            let size = self.inner_size();

            if pos >= size {
                return Err(vm.new_value_error("write byte out of range".to_owned()));
            }

            self.try_writable(vm, |mmap| {
                mmap[pos as usize] = b;
            })?;

            self.advance_pos(1);

            Ok(())
        }

        fn get_item_by_index(&self, i: isize, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let i = wrap_index(i, self.len())
                .ok_or_else(|| vm.new_index_error("mmap index out of range".to_owned()))?;

            let b = match self.check_valid(vm)?.deref().as_ref().unwrap() {
                MmapObj::Read(mmap) => mmap[i as usize],
                MmapObj::Write(mmap) => mmap[i as usize],
            };

            Ok(PyInt::from(b).into_ref(vm).into())
        }

        fn getitem_by_slice(
            &self,
            slice: &SaturatedSlice,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            let (range, step, slicelen) = slice.adjust_indices(self.len());

            let mmap = self.check_valid(vm)?;

            if slicelen == 0 {
                return Ok(PyBytes::from(vec![]).into_ref(vm).into());
            } else if step == 1 {
                let bytes = match mmap.deref().as_ref().unwrap() {
                    MmapObj::Read(mmap) => &mmap[range],
                    MmapObj::Write(mmap) => &mmap[range],
                };
                return Ok(PyBytes::from(bytes.to_vec()).into_ref(vm).into());
            }

            let mut result_buf = Vec::with_capacity(slicelen);
            if step.is_negative() {
                for i in range.rev().step_by(step.unsigned_abs()) {
                    let b = match mmap.deref().as_ref().unwrap() {
                        MmapObj::Read(mmap) => mmap[i],
                        MmapObj::Write(mmap) => mmap[i],
                    };
                    result_buf.push(b);
                }
            } else {
                for i in range.step_by(step.unsigned_abs()) {
                    let b = match mmap.deref().as_ref().unwrap() {
                        MmapObj::Read(mmap) => mmap[i],
                        MmapObj::Write(mmap) => mmap[i],
                    };
                    result_buf.push(b);
                }
            }
            Ok(PyBytes::from(result_buf).into_ref(vm).into())
        }

        fn _getitem(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            match SequenceIndex::try_from_borrowed_object(vm, needle, "mmap")? {
                SequenceIndex::Int(i) => self.get_item_by_index(i, vm),
                SequenceIndex::Slice(slice) => self.getitem_by_slice(&slice, vm),
            }
        }

        #[pymethod(magic)]
        fn getitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            self._getitem(&needle, vm)
        }

        fn _setitem(
            zelf: PyRef<Self>,
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
            zelf: PyRef<Self>,
            i: isize,
            value: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let i = wrap_index(i, zelf.len())
                .ok_or_else(|| vm.new_index_error("mmap index out of range".to_owned()))?;

            let b = value_from_object(vm, &value)?;

            zelf.try_writable(vm, |mmap| {
                mmap[i as usize] = b;
            })?;

            Ok(())
        }

        fn setitem_by_slice(
            zelf: PyRef<Self>,
            slice: &SaturatedSlice,
            value: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let (range, step, slicelen) = slice.adjust_indices(zelf.len());

            let bytes = bytes_from_object(vm, &value)?;

            if bytes.len() != slicelen {
                return Err(vm.new_index_error("mmap slice assignment is wrong size".to_owned()));
            }

            if slicelen == 0 {
                // do nothing
                Ok(())
            } else if step == 1 {
                zelf.try_writable(vm, |mmap| {
                    (&mut mmap[range])
                        .write(&bytes)
                        .map_err(|e| vm.new_os_error(e.to_string()))?;
                    Ok(())
                })?
            } else {
                let mut bi = 0; // bytes index
                if step.is_negative() {
                    for i in range.rev().step_by(step.unsigned_abs()) {
                        zelf.try_writable(vm, |mmap| {
                            mmap[i] = bytes[bi];
                        })?;
                        bi += 1;
                    }
                } else {
                    for i in range.step_by(step.unsigned_abs()) {
                        zelf.try_writable(vm, |mmap| {
                            mmap[i] = bytes[bi];
                        })?;
                        bi += 1;
                    }
                }
                Ok(())
            }
        }

        #[pymethod(magic)]
        fn setitem(
            zelf: PyRef<Self>,
            needle: PyObjectRef,
            value: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            Self::_setitem(zelf, &needle, value, vm)
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
