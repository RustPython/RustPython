// spell-checker:ignore usedforsecurity HASHXOF blake2b blake2s

pub(crate) use _blake2::module_def;

#[pymodule]
mod _blake2 {
    use crate::hashlib::_hashlib::{Blake2Hash, BlakeHashArgs, local_blake2b, local_blake2s};
    use crate::vm::{
        Context, Py, PyPayload, PyResult, VirtualMachine,
        builtins::{PyBytes, PyIntRef, PyModule, PyType, PyTypeRef},
        function::{ArgBytesLike, FuncArgs},
        types::Constructor,
    };

    macro_rules! blake_sig {
        ($name:ident, $digest:literal) => {
            #[derive(FromArgs)]
            struct $name {
                #[pyarg(any, default = b"")]
                data: ArgBytesLike,
                #[pyarg(named, default = $digest)]
                digest_size: i64,
                #[pyarg(named, default = b"")]
                key: ArgBytesLike,
                #[pyarg(named, default = b"")]
                salt: ArgBytesLike,
                #[pyarg(named, default = b"")]
                person: ArgBytesLike,
                #[pyarg(named, default = 1)]
                fanout: i64,
                #[pyarg(named, default = 1)]
                depth: i64,
                #[pyarg(named, default = 0)]
                leaf_size: i64,
                #[pyarg(named, default = 0)]
                node_offset: i64,
                #[pyarg(named, default = 0)]
                node_depth: i64,
                #[pyarg(named, default = 0)]
                inner_size: i64,
                #[pyarg(named, default = false)]
                last_node: bool,
                #[pyarg(named, default = true)]
                usedforsecurity: bool,
                #[pyarg(named, optional)]
                string: Option<ArgBytesLike>,
            }

            impl $name {
                fn touch(self) {
                    let Self {
                        data,
                        digest_size,
                        key,
                        salt,
                        person,
                        fanout,
                        depth,
                        leaf_size,
                        node_offset,
                        node_depth,
                        inner_size,
                        last_node,
                        usedforsecurity,
                        string,
                    } = self;
                    let _ = (
                        data,
                        digest_size,
                        key,
                        salt,
                        person,
                        fanout,
                        depth,
                        leaf_size,
                        node_offset,
                        node_depth,
                        inner_size,
                        last_node,
                        usedforsecurity,
                        string,
                    );
                }
            }
        };
    }

    blake_sig!(Blake2bSig, 64);
    blake_sig!(Blake2sSig, 32);

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

        #[pyslot]
        fn slot_new(_cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let args: BlakeHashArgs = args.bind(vm)?;
            Ok(Self {
                inner: local_blake2b(args, vm)?,
            }
            .into_pyobject(vm))
        }

        #[pygetset]
        fn name(&self) -> &'static str {
            self.inner.name()
        }

        #[pygetset]
        fn digest_size(&self) -> usize {
            self.inner.digest_size()
        }

        #[pygetset]
        fn block_size(&self) -> usize {
            self.inner.block_size()
        }

        #[pymethod]
        fn update(&self, data: ArgBytesLike) {
            data.with_ref(|bytes| self.inner.update(bytes));
        }

        #[pymethod]
        fn digest(&self) -> PyBytes {
            self.inner.digest().into()
        }

        #[pymethod]
        fn hexdigest(&self) -> String {
            self.inner.hexdigest()
        }

        #[pymethod]
        fn copy(&self) -> Self {
            Self {
                inner: self.inner.copy(),
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
        type Args = Blake2bSig;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            args.touch();
            Err(vm.new_type_error("use slot_new"))
        }
    }

    impl Constructor for PyBlake2s {
        type Args = Blake2sSig;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            args.touch();
            Err(vm.new_type_error("use slot_new"))
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

        #[pyslot]
        fn slot_new(_cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let args: BlakeHashArgs = args.bind(vm)?;
            Ok(Self {
                inner: local_blake2s(args, vm)?,
            }
            .into_pyobject(vm))
        }

        #[pygetset]
        fn name(&self) -> &'static str {
            self.inner.name()
        }

        #[pygetset]
        fn digest_size(&self) -> usize {
            self.inner.digest_size()
        }

        #[pygetset]
        fn block_size(&self) -> usize {
            self.inner.block_size()
        }

        #[pymethod]
        fn update(&self, data: ArgBytesLike) {
            data.with_ref(|bytes| self.inner.update(bytes));
        }

        #[pymethod]
        fn digest(&self) -> PyBytes {
            self.inner.digest().into()
        }

        #[pymethod]
        fn hexdigest(&self) -> String {
            self.inner.hexdigest()
        }

        #[pymethod]
        fn copy(&self) -> Self {
            Self {
                inner: self.inner.copy(),
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
