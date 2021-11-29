pub(crate) use hashlib::make_module;

#[pymodule]
mod hashlib {
    use crate::common::lock::{PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard};
    use crate::vm::{
        builtins::{PyBytes, PyBytesRef, PyStrRef, PyTypeRef},
        function::{FuncArgs, OptionalArg},
        PyResult, PyValue, VirtualMachine,
    };
    use blake2::{Blake2b, Blake2s};
    use digest::DynDigest;
    use md5::Md5;
    use sha1::Sha1;
    use sha2::{Sha224, Sha256, Sha384, Sha512};
    use sha3::{Sha3_224, Sha3_256, Sha3_384, Sha3_512}; // TODO: , shake_128, shake_256;

    #[derive(FromArgs)]
    #[allow(unused)]
    struct NewHashArgs {
        #[pyarg(positional)]
        name: PyStrRef,
        #[pyarg(any, optional)]
        data: OptionalArg<PyBytesRef>,
        #[pyarg(named, default = "true")]
        usedforsecurity: bool,
    }

    #[derive(FromArgs)]
    #[allow(unused)]
    struct BlakeHashArgs {
        #[pyarg(positional, optional)]
        data: OptionalArg<PyBytesRef>,
        #[pyarg(named, default = "true")]
        usedforsecurity: bool,
    }

    #[derive(FromArgs)]
    #[allow(unused)]
    struct HashArgs {
        #[pyarg(any, optional)]
        string: OptionalArg<PyBytesRef>,
        #[pyarg(named, default = "true")]
        usedforsecurity: bool,
    }

    #[pyattr]
    #[pyclass(module = "hashlib", name = "hasher")]
    #[derive(PyValue)]
    struct PyHasher {
        name: String,
        buffer: PyRwLock<HashWrapper>,
    }

    impl std::fmt::Debug for PyHasher {
        fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "hasher {}", self.name)
        }
    }

    #[pyimpl]
    impl PyHasher {
        fn new(name: &str, d: HashWrapper) -> Self {
            PyHasher {
                name: name.to_owned(),
                buffer: PyRwLock::new(d),
            }
        }

        fn read(&self) -> PyRwLockReadGuard<'_, HashWrapper> {
            self.buffer.read()
        }

        fn write(&self) -> PyRwLockWriteGuard<'_, HashWrapper> {
            self.buffer.write()
        }

        #[pyslot]
        fn slot_new(_cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            Ok(PyHasher::new("md5", HashWrapper::md5()).into_object(vm))
        }

        #[pyproperty]
        fn name(&self) -> String {
            self.name.clone()
        }

        #[pyproperty]
        fn digest_size(&self) -> usize {
            self.read().digest_size()
        }

        #[pymethod]
        fn update(&self, data: PyBytesRef) {
            self.write().input(data.as_bytes());
        }

        #[pymethod]
        fn digest(&self) -> PyBytes {
            self.get_digest().into()
        }

        #[pymethod]
        fn hexdigest(&self) -> String {
            let result = self.get_digest();
            hex::encode(result)
        }

        fn get_digest(&self) -> Vec<u8> {
            self.read().get_digest()
        }
    }

    #[pyfunction(name = "new")]
    fn hashlib_new(args: NewHashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        match args.name.as_str() {
            "md5" => md5(HashArgs {
                string: args.data,
                usedforsecurity: args.usedforsecurity,
            }),
            "sha1" => sha1(HashArgs {
                string: args.data,
                usedforsecurity: args.usedforsecurity,
            }),
            "sha224" => sha224(HashArgs {
                string: args.data,
                usedforsecurity: args.usedforsecurity,
            }),
            "sha256" => sha256(HashArgs {
                string: args.data,
                usedforsecurity: args.usedforsecurity,
            }),
            "sha384" => sha384(HashArgs {
                string: args.data,
                usedforsecurity: args.usedforsecurity,
            }),
            "sha512" => sha512(HashArgs {
                string: args.data,
                usedforsecurity: args.usedforsecurity,
            }),
            "sha3_224" => sha3_224(HashArgs {
                string: args.data,
                usedforsecurity: args.usedforsecurity,
            }),
            "sha3_256" => sha3_256(HashArgs {
                string: args.data,
                usedforsecurity: args.usedforsecurity,
            }),
            "sha3_384" => sha3_384(HashArgs {
                string: args.data,
                usedforsecurity: args.usedforsecurity,
            }),
            "sha3_512" => sha3_512(HashArgs {
                string: args.data,
                usedforsecurity: args.usedforsecurity,
            }),
            // TODO: "shake_128" => shake_128(args.data, ),
            // TODO: "shake_256" => shake_256(args.data, ),
            "blake2b" => blake2b(BlakeHashArgs {
                data: args.data,
                usedforsecurity: args.usedforsecurity,
            }),
            "blake2s" => blake2s(BlakeHashArgs {
                data: args.data,
                usedforsecurity: args.usedforsecurity,
            }),
            other => Err(vm.new_value_error(format!("Unknown hashing algorithm: {}", other))),
        }
    }

    fn init(hasher: PyHasher, data: OptionalArg<PyBytesRef>) -> PyResult<PyHasher> {
        if let OptionalArg::Present(data) = data {
            hasher.update(data);
        }

        Ok(hasher)
    }

    #[pyfunction]
    fn md5(args: HashArgs) -> PyResult<PyHasher> {
        init(PyHasher::new("md5", HashWrapper::md5()), args.string)
    }

    #[pyfunction]
    fn sha1(args: HashArgs) -> PyResult<PyHasher> {
        init(PyHasher::new("sha1", HashWrapper::sha1()), args.string)
    }

    #[pyfunction]
    fn sha224(args: HashArgs) -> PyResult<PyHasher> {
        init(PyHasher::new("sha224", HashWrapper::sha224()), args.string)
    }

    #[pyfunction]
    fn sha256(args: HashArgs) -> PyResult<PyHasher> {
        init(PyHasher::new("sha256", HashWrapper::sha256()), args.string)
    }

    #[pyfunction]
    fn sha384(args: HashArgs) -> PyResult<PyHasher> {
        init(PyHasher::new("sha384", HashWrapper::sha384()), args.string)
    }

    #[pyfunction]
    fn sha512(args: HashArgs) -> PyResult<PyHasher> {
        init(PyHasher::new("sha512", HashWrapper::sha512()), args.string)
    }

    #[pyfunction]
    fn sha3_224(args: HashArgs) -> PyResult<PyHasher> {
        init(
            PyHasher::new("sha3_224", HashWrapper::sha3_224()),
            args.string,
        )
    }

    #[pyfunction]
    fn sha3_256(args: HashArgs) -> PyResult<PyHasher> {
        init(
            PyHasher::new("sha3_256", HashWrapper::sha3_256()),
            args.string,
        )
    }

    #[pyfunction]
    fn sha3_384(args: HashArgs) -> PyResult<PyHasher> {
        init(
            PyHasher::new("sha3_384", HashWrapper::sha3_384()),
            args.string,
        )
    }

    #[pyfunction]
    fn sha3_512(args: HashArgs) -> PyResult<PyHasher> {
        init(
            PyHasher::new("sha3_512", HashWrapper::sha3_512()),
            args.string,
        )
    }

    #[pyfunction]
    fn shake_128(_args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        Err(vm.new_not_implemented_error("shake_256".to_owned()))
    }

    #[pyfunction]
    fn shake_256(_args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        Err(vm.new_not_implemented_error("shake_256".to_owned()))
    }

    #[pyfunction]
    fn blake2b(args: BlakeHashArgs) -> PyResult<PyHasher> {
        // TODO: handle parameters
        init(PyHasher::new("blake2b", HashWrapper::blake2b()), args.data)
    }

    #[pyfunction]
    fn blake2s(args: BlakeHashArgs) -> PyResult<PyHasher> {
        // TODO: handle parameters
        init(PyHasher::new("blake2s", HashWrapper::blake2s()), args.data)
    }

    trait ThreadSafeDynDigest: DynDigest + Sync + Send {}
    impl<T> ThreadSafeDynDigest for T where T: DynDigest + Sync + Send {}

    /// Generic wrapper patching around the hashing libraries.
    struct HashWrapper {
        inner: Box<dyn ThreadSafeDynDigest>,
    }

    impl HashWrapper {
        fn new<D: 'static>(d: D) -> Self
        where
            D: ThreadSafeDynDigest,
        {
            HashWrapper { inner: Box::new(d) }
        }

        fn md5() -> Self {
            Self::new(Md5::default())
        }

        fn sha1() -> Self {
            Self::new(Sha1::default())
        }

        fn sha224() -> Self {
            Self::new(Sha224::default())
        }

        fn sha256() -> Self {
            Self::new(Sha256::default())
        }

        fn sha384() -> Self {
            Self::new(Sha384::default())
        }

        fn sha512() -> Self {
            Self::new(Sha512::default())
        }

        fn sha3_224() -> Self {
            Self::new(Sha3_224::default())
        }

        fn sha3_256() -> Self {
            Self::new(Sha3_256::default())
        }

        fn sha3_384() -> Self {
            Self::new(Sha3_384::default())
        }

        fn sha3_512() -> Self {
            Self::new(Sha3_512::default())
        }

        /* TODO:
            fn shake_128() -> Self {
                Self::new(shake_128::default())
            }

            fn shake_256() -> Self {
                Self::new(shake_256::default())
            }
        */
        fn blake2b() -> Self {
            Self::new(Blake2b::default())
        }

        fn blake2s() -> Self {
            Self::new(Blake2s::default())
        }

        fn input(&mut self, data: &[u8]) {
            self.inner.update(data);
        }

        fn digest_size(&self) -> usize {
            self.inner.output_size()
        }

        fn get_digest(&self) -> Vec<u8> {
            let cloned = self.inner.box_clone();
            cloned.finalize().into_vec()
        }
    }
}
