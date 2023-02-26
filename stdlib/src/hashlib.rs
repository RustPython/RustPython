pub(crate) use hashlib::make_module;

#[pymodule]
mod hashlib {
    use crate::common::lock::PyRwLock;
    use crate::vm::{
        builtins::{PyBytes, PyStrRef, PyTypeRef},
        function::{ArgBytesLike, FuncArgs, OptionalArg},
        PyObjectRef, PyPayload, PyResult, VirtualMachine,
    };
    use blake2::{Blake2b512, Blake2s256};
    use digest::{core_api::BlockSizeUser, DynDigest};
    use digest::{ExtendableOutput, Update};
    use dyn_clone::{clone_trait_object, DynClone};
    use md5::Md5;
    use sha1::Sha1;
    use sha2::{Sha224, Sha256, Sha384, Sha512};
    use sha3::{Sha3_224, Sha3_256, Sha3_384, Sha3_512, Shake128, Shake256};

    #[derive(FromArgs)]
    #[allow(unused)]
    struct NewHashArgs {
        #[pyarg(positional)]
        name: PyStrRef,
        #[pyarg(any, optional)]
        data: OptionalArg<ArgBytesLike>,
        #[pyarg(named, default = "true")]
        usedforsecurity: bool,
    }

    #[derive(FromArgs)]
    #[allow(unused)]
    struct BlakeHashArgs {
        #[pyarg(positional, optional)]
        data: OptionalArg<ArgBytesLike>,
        #[pyarg(named, default = "true")]
        usedforsecurity: bool,
    }

    impl From<NewHashArgs> for BlakeHashArgs {
        fn from(args: NewHashArgs) -> Self {
            Self {
                data: args.data,
                usedforsecurity: args.usedforsecurity,
            }
        }
    }

    #[derive(FromArgs)]
    #[allow(unused)]
    struct HashArgs {
        #[pyarg(any, optional)]
        string: OptionalArg<ArgBytesLike>,
        #[pyarg(named, default = "true")]
        usedforsecurity: bool,
    }

    impl From<NewHashArgs> for HashArgs {
        fn from(args: NewHashArgs) -> Self {
            Self {
                string: args.data,
                usedforsecurity: args.usedforsecurity,
            }
        }
    }

    #[derive(FromArgs)]
    #[allow(unused)]
    struct XofDigestArgs {
        #[pyarg(positional)]
        length: isize,
    }

    impl XofDigestArgs {
        fn length(&self, vm: &VirtualMachine) -> PyResult<usize> {
            usize::try_from(self.length)
                .map_err(|_| vm.new_value_error("length must be non-negative".to_owned()))
        }
    }

    #[pyattr]
    #[pyclass(module = "hashlib", name = "HASH")]
    #[derive(PyPayload)]
    struct PyHasher {
        name: String,
        ctx: PyRwLock<HashWrapper>,
    }

    impl std::fmt::Debug for PyHasher {
        fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "HASH {}", self.name)
        }
    }

    #[pyclass]
    impl PyHasher {
        fn new(name: &str, d: HashWrapper) -> Self {
            PyHasher {
                name: name.to_owned(),
                ctx: PyRwLock::new(d),
            }
        }

        #[pyslot]
        fn slot_new(_cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            Err(vm.new_type_error("cannot create 'hashlib.HASH' instances".into()))
        }

        #[pygetset]
        fn name(&self) -> String {
            self.name.clone()
        }

        #[pygetset]
        fn digest_size(&self) -> usize {
            self.ctx.read().digest_size()
        }

        #[pygetset]
        fn block_size(&self) -> usize {
            self.ctx.read().block_size()
        }

        #[pymethod]
        fn update(&self, data: ArgBytesLike) {
            data.with_ref(|bytes| self.ctx.write().update(bytes));
        }

        #[pymethod]
        fn digest(&self) -> PyBytes {
            self.ctx.read().finalize().into()
        }

        #[pymethod]
        fn hexdigest(&self) -> String {
            hex::encode(self.ctx.read().finalize())
        }

        #[pymethod]
        fn copy(&self) -> Self {
            PyHasher::new(&self.name, self.ctx.read().clone())
        }
    }

    #[pyattr]
    #[pyclass(module = "hashlib", name = "HASHXOF")]
    #[derive(PyPayload)]
    struct PyHasherXof {
        name: String,
        ctx: PyRwLock<HashXofWrapper>,
    }

    impl std::fmt::Debug for PyHasherXof {
        fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "HASHXOF {}", self.name)
        }
    }

    #[pyclass]
    impl PyHasherXof {
        fn new(name: &str, d: HashXofWrapper) -> Self {
            PyHasherXof {
                name: name.to_owned(),
                ctx: PyRwLock::new(d),
            }
        }

        #[pyslot]
        fn slot_new(_cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            Err(vm.new_type_error("cannot create 'hashlib.HASHXOF' instances".into()))
        }

        #[pygetset]
        fn name(&self) -> String {
            self.name.clone()
        }

        #[pygetset]
        fn digest_size(&self) -> usize {
            0
        }

        #[pygetset]
        fn block_size(&self) -> usize {
            self.ctx.read().block_size()
        }

        #[pymethod]
        fn update(&self, data: ArgBytesLike) {
            data.with_ref(|bytes| self.ctx.write().update(bytes));
        }

        #[pymethod]
        fn digest(&self, args: XofDigestArgs, vm: &VirtualMachine) -> PyResult<PyBytes> {
            Ok(self.ctx.read().finalize_xof(args.length(vm)?).into())
        }

        #[pymethod]
        fn hexdigest(&self, args: XofDigestArgs, vm: &VirtualMachine) -> PyResult<String> {
            Ok(hex::encode(self.ctx.read().finalize_xof(args.length(vm)?)))
        }

        #[pymethod]
        fn copy(&self) -> Self {
            PyHasherXof::new(&self.name, self.ctx.read().clone())
        }
    }

    #[pyfunction(name = "new")]
    fn hashlib_new(args: NewHashArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        match args.name.as_str().to_lowercase().as_str() {
            "md5" => Ok(md5(args.into()).into_pyobject(vm)),
            "sha1" => Ok(sha1(args.into()).into_pyobject(vm)),
            "sha224" => Ok(sha224(args.into()).into_pyobject(vm)),
            "sha256" => Ok(sha256(args.into()).into_pyobject(vm)),
            "sha384" => Ok(sha384(args.into()).into_pyobject(vm)),
            "sha512" => Ok(sha512(args.into()).into_pyobject(vm)),
            "sha3_224" => Ok(sha3_224(args.into()).into_pyobject(vm)),
            "sha3_256" => Ok(sha3_256(args.into()).into_pyobject(vm)),
            "sha3_384" => Ok(sha3_384(args.into()).into_pyobject(vm)),
            "sha3_512" => Ok(sha3_512(args.into()).into_pyobject(vm)),
            "shake_128" => Ok(shake_128(args.into()).into_pyobject(vm)),
            "shake_256" => Ok(shake_256(args.into()).into_pyobject(vm)),
            "blake2b" => Ok(blake2b(args.into()).into_pyobject(vm)),
            "blake2s" => Ok(blake2s(args.into()).into_pyobject(vm)),
            other => Err(vm.new_value_error(format!("Unknown hashing algorithm: {other}"))),
        }
    }

    #[pyfunction]
    fn md5(args: HashArgs) -> PyHasher {
        PyHasher::new("md5", HashWrapper::new::<Md5>(args.string))
    }

    #[pyfunction]
    fn sha1(args: HashArgs) -> PyHasher {
        PyHasher::new("sha1", HashWrapper::new::<Sha1>(args.string))
    }

    #[pyfunction]
    fn sha224(args: HashArgs) -> PyHasher {
        PyHasher::new("sha224", HashWrapper::new::<Sha224>(args.string))
    }

    #[pyfunction]
    fn sha256(args: HashArgs) -> PyHasher {
        PyHasher::new("sha256", HashWrapper::new::<Sha256>(args.string))
    }

    #[pyfunction]
    fn sha384(args: HashArgs) -> PyHasher {
        PyHasher::new("sha384", HashWrapper::new::<Sha384>(args.string))
    }

    #[pyfunction]
    fn sha512(args: HashArgs) -> PyHasher {
        PyHasher::new("sha512", HashWrapper::new::<Sha512>(args.string))
    }

    #[pyfunction]
    fn sha3_224(args: HashArgs) -> PyHasher {
        PyHasher::new("sha3_224", HashWrapper::new::<Sha3_224>(args.string))
    }

    #[pyfunction]
    fn sha3_256(args: HashArgs) -> PyHasher {
        PyHasher::new("sha3_256", HashWrapper::new::<Sha3_256>(args.string))
    }

    #[pyfunction]
    fn sha3_384(args: HashArgs) -> PyHasher {
        PyHasher::new("sha3_384", HashWrapper::new::<Sha3_384>(args.string))
    }

    #[pyfunction]
    fn sha3_512(args: HashArgs) -> PyHasher {
        PyHasher::new("sha3_512", HashWrapper::new::<Sha3_512>(args.string))
    }

    #[pyfunction]
    fn shake_128(args: HashArgs) -> PyHasherXof {
        PyHasherXof::new("shake_128", HashXofWrapper::new_shake_128(args.string))
    }

    #[pyfunction]
    fn shake_256(args: HashArgs) -> PyHasherXof {
        PyHasherXof::new("shake_256", HashXofWrapper::new_shake_256(args.string))
    }

    #[pyfunction]
    fn blake2b(args: BlakeHashArgs) -> PyHasher {
        PyHasher::new("blake2b", HashWrapper::new::<Blake2b512>(args.data))
    }

    #[pyfunction]
    fn blake2s(args: BlakeHashArgs) -> PyHasher {
        PyHasher::new("blake2s", HashWrapper::new::<Blake2s256>(args.data))
    }

    trait ThreadSafeDynDigest: DynClone + DynDigest + Sync + Send {}
    impl<T> ThreadSafeDynDigest for T where T: DynClone + DynDigest + Sync + Send {}

    clone_trait_object!(ThreadSafeDynDigest);

    /// Generic wrapper patching around the hashing libraries.
    #[derive(Clone)]
    struct HashWrapper {
        block_size: usize,
        inner: Box<dyn ThreadSafeDynDigest>,
    }

    impl HashWrapper {
        fn new<D>(data: OptionalArg<ArgBytesLike>) -> Self
        where
            D: ThreadSafeDynDigest + BlockSizeUser + Default + 'static,
        {
            let mut h = HashWrapper {
                block_size: D::block_size(),
                inner: Box::<D>::default(),
            };
            if let OptionalArg::Present(d) = data {
                d.with_ref(|bytes| h.update(bytes));
            }
            h
        }

        fn update(&mut self, data: &[u8]) {
            self.inner.update(data);
        }

        fn block_size(&self) -> usize {
            self.block_size
        }

        fn digest_size(&self) -> usize {
            self.inner.output_size()
        }

        fn finalize(&self) -> Vec<u8> {
            let cloned = self.inner.box_clone();
            cloned.finalize().into_vec()
        }
    }

    #[derive(Clone)]
    enum HashXofWrapper {
        Shake128(Shake128),
        Shake256(Shake256),
    }

    impl HashXofWrapper {
        fn new_shake_128(data: OptionalArg<ArgBytesLike>) -> Self {
            let mut h = HashXofWrapper::Shake128(Shake128::default());
            if let OptionalArg::Present(d) = data {
                d.with_ref(|bytes| h.update(bytes));
            }
            h
        }

        fn new_shake_256(data: OptionalArg<ArgBytesLike>) -> Self {
            let mut h = HashXofWrapper::Shake256(Shake256::default());
            if let OptionalArg::Present(d) = data {
                d.with_ref(|bytes| h.update(bytes));
            }
            h
        }

        fn update(&mut self, data: &[u8]) {
            match self {
                HashXofWrapper::Shake128(h) => h.update(data),
                HashXofWrapper::Shake256(h) => h.update(data),
            }
        }

        fn block_size(&self) -> usize {
            match self {
                HashXofWrapper::Shake128(_) => Shake128::block_size(),
                HashXofWrapper::Shake256(_) => Shake256::block_size(),
            }
        }

        fn finalize_xof(&self, length: usize) -> Vec<u8> {
            match self {
                HashXofWrapper::Shake128(h) => h.clone().finalize_boxed(length).into_vec(),
                HashXofWrapper::Shake256(h) => h.clone().finalize_boxed(length).into_vec(),
            }
        }
    }
}
