// spell-checker:ignore usedforsecurity HASHXOF blake2b blake2s

pub(crate) use _blake2::module_def;

#[pymodule]
mod _blake2 {
    use crate::hashlib::_hashlib::{
        Blake2Hash, Blake2bArgs, Blake2sArgs, local_blake2b, local_blake2s,
    };
    use crate::vm::{
        Context, Py, PyPayload, PyResult, VirtualMachine,
        builtins::{PyBytes, PyIntRef, PyModule, PyType},
        function::ArgBytesLike,
        types::Constructor,
    };

    #[pyattr(name = "_GIL_MINSIZE")]
    const GIL_MINSIZE: u16 = 2048;

    #[pyattr]
    const BLAKE2B_SALT_SIZE: i32 = 16;

    #[pyattr]
    const BLAKE2B_PERSON_SIZE: i32 = 16;

    #[pyattr]
    const BLAKE2B_MAX_KEY_SIZE: i32 = 64;

    #[pyattr]
    const BLAKE2B_MAX_DIGEST_SIZE: i32 = 64;

    #[pyattr]
    const BLAKE2S_SALT_SIZE: i32 = 8;

    #[pyattr]
    const BLAKE2S_PERSON_SIZE: i32 = 8;

    #[pyattr]
    const BLAKE2S_MAX_KEY_SIZE: i32 = 32;

    #[pyattr]
    const BLAKE2S_MAX_DIGEST_SIZE: i32 = 32;

    #[pyattr]
    #[pyclass(module = "_blake2", name = "blake2b")]
    #[derive(PyPayload)]
    struct PyBlake2b {
        inner: Blake2Hash,
    }

    impl core::fmt::Debug for PyBlake2b {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("blake2b")
        }
    }

    #[pyclass(with(Constructor), flags(IMMUTABLETYPE))]
    impl PyBlake2b {
        #[pyattr(name = "SALT_SIZE")]
        fn salt_size(ctx: &Context) -> PyIntRef {
            ctx.new_int(BLAKE2B_SALT_SIZE)
        }
        #[pyattr(name = "PERSON_SIZE")]
        fn person_size(ctx: &Context) -> PyIntRef {
            ctx.new_int(BLAKE2B_PERSON_SIZE)
        }
        #[pyattr(name = "MAX_KEY_SIZE")]
        fn max_key_size(ctx: &Context) -> PyIntRef {
            ctx.new_int(BLAKE2B_MAX_KEY_SIZE)
        }
        #[pyattr(name = "MAX_DIGEST_SIZE")]
        fn max_digest_size(ctx: &Context) -> PyIntRef {
            ctx.new_int(BLAKE2B_MAX_DIGEST_SIZE)
        }

        #[pygetset]
        fn name(zelf: &Py<Self>) -> &'static str {
            zelf.inner.name()
        }

        #[pygetset]
        fn digest_size(zelf: &Py<Self>) -> usize {
            zelf.inner.digest_size()
        }

        #[pygetset]
        fn block_size(zelf: &Py<Self>) -> usize {
            zelf.inner.block_size()
        }

        #[pymethod]
        fn update(zelf: &Py<Self>, data: ArgBytesLike) {
            data.with_ref(|bytes| zelf.inner.update(bytes));
        }

        #[pymethod]
        fn digest(zelf: &Py<Self>) -> PyBytes {
            zelf.inner.digest().into()
        }

        #[pymethod]
        fn hexdigest(zelf: &Py<Self>) -> String {
            zelf.inner.hexdigest()
        }

        #[pymethod]
        fn copy(zelf: &Py<Self>) -> Self {
            Self {
                inner: zelf.inner.copy(),
            }
        }
    }

    #[pyattr]
    #[pyclass(module = "_blake2", name = "blake2s")]
    #[derive(PyPayload)]
    struct PyBlake2s {
        inner: Blake2Hash,
    }

    impl Constructor for PyBlake2b {
        type Args = Blake2bArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self {
                inner: local_blake2b(args, vm)?,
            })
        }
    }

    impl Constructor for PyBlake2s {
        type Args = Blake2sArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self {
                inner: local_blake2s(args, vm)?,
            })
        }
    }

    impl core::fmt::Debug for PyBlake2s {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("blake2s")
        }
    }

    #[pyclass(with(Constructor), flags(IMMUTABLETYPE))]
    impl PyBlake2s {
        #[pyattr(name = "SALT_SIZE")]
        fn salt_size(ctx: &Context) -> PyIntRef {
            ctx.new_int(BLAKE2S_SALT_SIZE)
        }
        #[pyattr(name = "PERSON_SIZE")]
        fn person_size(ctx: &Context) -> PyIntRef {
            ctx.new_int(BLAKE2S_PERSON_SIZE)
        }
        #[pyattr(name = "MAX_KEY_SIZE")]
        fn max_key_size(ctx: &Context) -> PyIntRef {
            ctx.new_int(BLAKE2S_MAX_KEY_SIZE)
        }
        #[pyattr(name = "MAX_DIGEST_SIZE")]
        fn max_digest_size(ctx: &Context) -> PyIntRef {
            ctx.new_int(BLAKE2S_MAX_DIGEST_SIZE)
        }

        #[pygetset]
        fn name(zelf: &Py<Self>) -> &'static str {
            zelf.inner.name()
        }

        #[pygetset]
        fn digest_size(zelf: &Py<Self>) -> usize {
            zelf.inner.digest_size()
        }

        #[pygetset]
        fn block_size(zelf: &Py<Self>) -> usize {
            zelf.inner.block_size()
        }

        #[pymethod]
        fn update(zelf: &Py<Self>, data: ArgBytesLike) {
            data.with_ref(|bytes| zelf.inner.update(bytes));
        }

        #[pymethod]
        fn digest(zelf: &Py<Self>) -> PyBytes {
            zelf.inner.digest().into()
        }

        #[pymethod]
        fn hexdigest(zelf: &Py<Self>) -> String {
            zelf.inner.hexdigest()
        }

        #[pymethod]
        fn copy(zelf: &Py<Self>) -> Self {
            Self {
                inner: zelf.inner.copy(),
            }
        }
    }

    #[expect(clippy::unnecessary_wraps, reason = "Needs to comply with a signature")]
    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        let _ = vm.import("_hashlib", 0);
        __module_exec(vm, module);
        Ok(())
    }
}
