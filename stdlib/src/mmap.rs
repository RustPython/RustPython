pub(crate) use mmap::make_module;

#[pymodule]
mod mmap {
    use crate::vm::{
        builtins::PyTypeRef, convert::ToPyResult, function::OptionalArg, types::Constructor,
        FromArgs, PyObject, PyPayload, PyResult, TryFromBorrowedObject, VirtualMachine,
    };
    use memmap2::{MmapMut, MmapOptions};

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
    use libc::{MAP_ANON, MAP_ANONYMOUS, MAP_PRIVATE, MAP_SHARED, PROT_READ, PROT_WRITE};
    #[pyattr]
    const ACCESS_DEFAULT: u32 = AccessMode::Default as u32;
    #[pyattr]
    const ACCESS_READ: u32 = AccessMode::Read as u32;
    #[pyattr]
    const ACCESS_WRITE: u32 = AccessMode::Write as u32;
    #[pyattr]
    const ACCESS_COPY: u32 = AccessMode::Copy as u32;

    #[pyattr(name = "PAGESIZE")]
    fn pagesize(vm: &VirtualMachine) -> usize {
        page_size::get()
    }

    #[pyattr]
    #[pyclass(name = "mmap")]
    #[derive(Debug, PyPayload)]
    struct PyMmap {
        mmap: MmapMut,
        exports: usize,
        //     PyObject *weakreflist;
        access: AccessMode,
    }

    #[derive(FromArgs)]
    struct MmapNewArgs {
        #[pyarg(any)]
        fileno: std::os::unix::io::RawFd,
        #[pyarg(any)]
        length: isize,
        #[pyarg(any, default = "MAP_SHARED")]
        flags: libc::c_int,
        #[pyarg(any, default = "PROT_WRITE|PROT_READ")]
        prot: libc::c_int,
        #[pyarg(any, default = "AccessMode::Default")]
        access: AccessMode,
        #[pyarg(any, default = "0")]
        offset: u64,
    }

    impl Constructor for PyMmap {
        type Args = MmapNewArgs;

        fn py_new(
            cls: PyTypeRef,
            MmapNewArgs {
                fileno: fd,
                length,
                flags,
                prot,
                access,
                offset,
            }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            if length < 0 {
                return Err(
                    vm.new_overflow_error("memory mapped length must be positive".to_owned())
                );
            }
            // if offset < 0 {
            //     return Err(vm.new_overflow_error("memory mapped offset must be positive".to_owned()));
            // }
            if (access != AccessMode::Default)
                && ((flags != MAP_SHARED) || (prot != (PROT_WRITE | PROT_READ)))
            {
                return Err(vm.new_value_error(
                    "mmap can't specify both access and flags, prot.".to_owned(),
                ));
            }

            let (flags, prot, access) = match access {
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
                _ => return Err(vm.new_value_error("mmap invalid access parameter.".to_owned())),
            };

            let mut mmap_opt = MmapOptions::new();
            let mmap_opt = mmap_opt.offset(offset);
            // .len(map_size)
            let mmap = match access {
                AccessMode::Write => unsafe { mmap_opt.map_mut(fd) },
                // AccessMode::Read => mmap_opt.map(fd),
                AccessMode::Copy => unsafe { mmap_opt.map_copy(fd) },
                _ => unreachable!("access must be decided before here"),
            }
            .map_err(|_| vm.new_value_error("FIXME: mmap error".to_owned()))?;

            let m_obj = Self {
                mmap,
                exports: 0,
                access,
            };

            m_obj.to_pyresult(vm)
        }
    }

    #[pyimpl]
    impl PyMmap {
        #[pymethod]
        fn close(&self) -> PyResult<()> {
            if self.exports > 0 {
                // PyErr_SetString(PyExc_BufferError, "cannot close "\
                // "exported pointers exist");
            }
            // self.mmap = MmapMut::map_anon(0).unwrap();
            Ok(())
        }
    }
}
