pub(crate) use mmap::make_module;

#[pymodule]
mod mmap {
    use crate::vm::{
        builtins::PyTypeRef,
        function::{IntoPyResult, OptionalArg},
        types::Constructor,
        FromArgs, PyObject, PyResult, PyValue, TryFromBorrowedObject, VirtualMachine,
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
    use libc::{
        MAP_ANON, MAP_ANONYMOUS, MAP_DENYWRITE, MAP_EXECUTABLE, MAP_POPULATE, MAP_PRIVATE,
        MAP_SHARED, PROT_READ, PROT_WRITE,
    };
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
    #[derive(Debug, PyValue)]
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

            // if (PySys_Audit("mmap.__new__", "ini" _Py_PARSE_OFF_T,
            //                 fd, map_size, access, offset) < 0) {
            //     return NULL;
            // }

            // #[cfg(target_vendor = "apple")]
            // Issue #11277: fsync(2) is not enough on OS X - a special, OS X specific
            //   fcntl(2) is necessary to force DISKSYNC and get around mmap(2) bug
            // if fd != -1 {
            //     fcntl(fd, F_FULLFSYNC);
            // }

            // if fd != -1 {
            // Py_BEGIN_ALLOW_THREADS
            // fstat_result = _Py_fstat_noraise(fd, &status);
            // Py_END_ALLOW_THREADS
            // }

            // if (fd != -1 && fstat_result == 0 && S_ISREG(status.st_mode)) {
            //     if (map_size == 0) {
            //         if (status.st_size == 0) {
            //             PyErr_SetString(PyExc_ValueError,
            //                             "cannot mmap an empty file");
            //             return NULL;
            //         }
            //         if (offset >= status.st_size) {
            //             PyErr_SetString(PyExc_ValueError,
            //                             "mmap offset is greater than file size");
            //             return NULL;
            //         }
            //         if (status.st_size - offset > PY_SSIZE_T_MAX) {
            //             PyErr_SetString(PyExc_ValueError,
            //                              "mmap length is too large");
            //             return NULL;
            //         }
            //         map_size = (Py_ssize_t) (status.st_size - offset);
            //     } else if (offset > status.st_size || status.st_size - offset < map_size) {
            //         PyErr_SetString(PyExc_ValueError,
            //                         "mmap length is greater than file size");
            //         return NULL;
            //     }
            // }
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

            //     if (m_obj == NULL) {return NULL;}
            //     m_obj->data = NULL;
            //     m_obj->size = map_size;
            //     m_obj->pos = 0;
            //     m_obj->weakreflist = NULL;
            //     m_obj->exports = 0;
            //     m_obj->offset = offset;
            //     if (fd == -1) {
            //         m_obj->fd = -1;
            //         /* Assume the caller wants to map anonymous memory.
            //            This is the same behaviour as Windows.  mmap.mmap(-1, size)
            //            on both Windows and Unix map anonymous memory.
            //         */
            // #ifdef MAP_ANONYMOUS
            //         /* BSD way to map anonymous memory */
            //         flags |= MAP_ANONYMOUS;

            //         /* VxWorks only supports MAP_ANONYMOUS with MAP_PRIVATE flag */
            // #ifdef __VXWORKS__
            //         flags &= ~MAP_SHARED;
            //         flags |= MAP_PRIVATE;
            // #endif

            // #else
            //         /* SVR4 method to map anonymous memory is to open /dev/zero */
            //         fd = devzero = _Py_open("/dev/zero", O_RDWR);
            //         if (devzero == -1) {
            //             Py_DECREF(m_obj);
            //             return NULL;
            //         }
            // #endif
            //     }
            //     else {
            //         m_obj->fd = _Py_dup(fd);
            //         if (m_obj->fd == -1) {
            //             Py_DECREF(m_obj);
            //             return NULL;
            //         }
            //     }

            //     m_obj->data = mmap(NULL, map_size,
            //                        prot, flags,
            //                        fd, offset);

            //     if (devzero != -1) {
            //         close(devzero);
            //     }

            //     if (m_obj->data == (char *)-1) {
            //         m_obj->data = NULL;
            //         Py_DECREF(m_obj);
            //         PyErr_SetFromErrno(PyExc_OSError);
            //         return NULL;
            //     }
            //     m_obj->access = (AccessMode)access;
            //     return (PyObject *)m_obj;
            //     }
            m_obj.into_pyresult(vm)
        }
    }

    #[pyimpl]
    impl PyMmap {
        // {Py_tp_new, new_mmap_object},
        // {Py_tp_dealloc, mmap_object_dealloc},
        // {Py_tp_repr, mmap__repr__method},
        // {Py_tp_doc, (void *)mmap_doc},
        // {Py_tp_methods, mmap_object_methods},
        // {Py_tp_members, mmap_object_members},
        // {Py_tp_getset, mmap_object_getset},
        // {Py_tp_getattro, PyObject_GenericGetAttr},
        // {Py_tp_traverse, mmap_object_traverse},

        // /* as sequence */
        // {Py_sq_length, mmap_length},
        // {Py_sq_item, mmap_item},
        // {Py_sq_ass_item, mmap_ass_item},

        // /* as mapping */
        // {Py_mp_length, mmap_length},
        // {Py_mp_subscript, mmap_subscript},
        // {Py_mp_ass_subscript, mmap_ass_subscript},

        // /* as buffer */
        // {Py_bf_getbuffer, mmap_buffer_getbuf},
        // {Py_bf_releasebuffer, mmap_buffer_releasebuf},

        //     {"close",           (PyCFunction) mmap_close_method,        METH_NOARGS},
        #[pymethod]
        fn close(&self) -> PyResult<()> {
            if self.exports > 0 {
                // PyErr_SetString(PyExc_BufferError, "cannot close "\
                // "exported pointers exist");
            }
            // self.mmap = MmapMut::map_anon(0).unwrap();
            Ok(())
        }

        //     {"find",            (PyCFunction) mmap_find_method,         METH_VARARGS},
        //     {"rfind",           (PyCFunction) mmap_rfind_method,        METH_VARARGS},
        //     {"flush",           (PyCFunction) mmap_flush_method,        METH_VARARGS},
        // #ifdef HAVE_MADVISE
        //     {"madvise",         (PyCFunction) mmap_madvise_method,      METH_VARARGS},
        // #endif
        //     {"move",            (PyCFunction) mmap_move_method,         METH_VARARGS},
        //     {"read",            (PyCFunction) mmap_read_method,         METH_VARARGS},
        //     {"read_byte",       (PyCFunction) mmap_read_byte_method,    METH_NOARGS},
        //     {"readline",        (PyCFunction) mmap_read_line_method,    METH_NOARGS},
        //     {"resize",          (PyCFunction) mmap_resize_method,       METH_VARARGS},
        //     {"seek",            (PyCFunction) mmap_seek_method,         METH_VARARGS},
        //     {"size",            (PyCFunction) mmap_size_method,         METH_NOARGS},
        //     {"tell",            (PyCFunction) mmap_tell_method,         METH_NOARGS},
        //     {"write",           (PyCFunction) mmap_write_method,        METH_VARARGS},
        //     {"write_byte",      (PyCFunction) mmap_write_byte_method,   METH_VARARGS},
        //     {"__enter__",       (PyCFunction) mmap__enter__method,      METH_NOARGS},
        //     {"__exit__",        (PyCFunction) mmap__exit__method,       METH_VARARGS},
        // #ifdef MS_WINDOWS
        //     {"__sizeof__",      (PyCFunction) mmap__sizeof__method,     METH_NOARGS},
        // #endif
    }
}
