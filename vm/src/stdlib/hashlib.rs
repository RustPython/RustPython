pub(crate) use hashlib::make_module;

#[pymodule]
mod hashlib {
    use crate::builtins::bytes::{PyBytes, PyBytesRef};
    use crate::builtins::pystr::PyStrRef;
    use crate::builtins::pytype::PyTypeRef;
    use crate::common::lock::{PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard};
    use crate::function::{FuncArgs, OptionalArg};
    use crate::pyobject::{BorrowValue, PyResult, PyValue, StaticType};
    use crate::vm::VirtualMachine;
    use blake2::{Blake2b, Blake2s};
    use digest::DynDigest;
    use md5::Md5;
    use sha1::Sha1;
    use sha2::{Sha224, Sha256, Sha384, Sha512};
    use sha3::{Sha3_224, Sha3_256, Sha3_384, Sha3_512}; // TODO: , Shake128, Shake256;
    use std::fmt;

    #[pyattr]
    #[pyclass(module = "hashlib", name = "hasher")]
    struct PyHasher {
        name: String,
        buffer: PyRwLock<HashWrapper>,
    }

    impl fmt::Debug for PyHasher {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "hasher {}", self.name)
        }
    }

    impl PyValue for PyHasher {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
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

        fn borrow_value(&self) -> PyRwLockReadGuard<'_, HashWrapper> {
            self.buffer.read()
        }

        fn borrow_value_mut(&self) -> PyRwLockWriteGuard<'_, HashWrapper> {
            self.buffer.write()
        }

        #[pyslot]
        fn tp_new(_cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            Ok(PyHasher::new("md5", HashWrapper::md5())
                .into_ref(vm)
                .into_object())
        }

        #[pyproperty(name = "name")]
        fn name(&self) -> String {
            self.name.clone()
        }

        #[pyproperty(name = "digest_size")]
        fn digest_size(&self, vm: &VirtualMachine) -> PyResult {
            Ok(vm.ctx.new_int(self.borrow_value().digest_size()))
        }

        #[pymethod(name = "update")]
        fn update(&self, data: PyBytesRef) {
            self.borrow_value_mut().input(data.borrow_value());
        }

        #[pymethod(name = "digest")]
        fn digest(&self) -> PyBytes {
            self.get_digest().into()
        }

        #[pymethod(name = "hexdigest")]
        fn hexdigest(&self) -> String {
            let result = self.get_digest();
            hex::encode(result)
        }

        fn get_digest(&self) -> Vec<u8> {
            self.borrow_value().get_digest()
        }
    }

    #[pyfunction(name = "new")]
    fn hashlib_new(
        name: PyStrRef,
        data: OptionalArg<PyBytesRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyHasher> {
        match name.borrow_value() {
            "md5" => md5(data),
            "sha1" => sha1(data),
            "sha224" => sha224(data),
            "sha256" => sha256(data),
            "sha384" => sha384(data),
            "sha512" => sha512(data),
            "sha3_224" => sha3_224(data),
            "sha3_256" => sha3_256(data),
            "sha3_384" => sha3_384(data),
            "sha3_512" => sha3_512(data),
            // TODO: "shake128" => shake128(data, ),
            // TODO: "shake256" => shake256(data, ),
            "blake2b" => blake2b(data),
            "blake2s" => blake2s(data),
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
    fn md5(data: OptionalArg<PyBytesRef>) -> PyResult<PyHasher> {
        init(PyHasher::new("md5", HashWrapper::md5()), data)
    }

    #[pyfunction]
    fn sha1(data: OptionalArg<PyBytesRef>) -> PyResult<PyHasher> {
        init(PyHasher::new("sha1", HashWrapper::sha1()), data)
    }

    #[pyfunction]
    fn sha224(data: OptionalArg<PyBytesRef>) -> PyResult<PyHasher> {
        init(PyHasher::new("sha224", HashWrapper::sha224()), data)
    }

    #[pyfunction]
    fn sha256(data: OptionalArg<PyBytesRef>) -> PyResult<PyHasher> {
        init(PyHasher::new("sha256", HashWrapper::sha256()), data)
    }

    #[pyfunction]
    fn sha384(data: OptionalArg<PyBytesRef>) -> PyResult<PyHasher> {
        init(PyHasher::new("sha384", HashWrapper::sha384()), data)
    }

    #[pyfunction]
    fn sha512(data: OptionalArg<PyBytesRef>) -> PyResult<PyHasher> {
        init(PyHasher::new("sha512", HashWrapper::sha512()), data)
    }

    #[pyfunction]
    fn sha3_224(data: OptionalArg<PyBytesRef>) -> PyResult<PyHasher> {
        init(PyHasher::new("sha3_224", HashWrapper::sha3_224()), data)
    }

    #[pyfunction]
    fn sha3_256(data: OptionalArg<PyBytesRef>) -> PyResult<PyHasher> {
        init(PyHasher::new("sha3_256", HashWrapper::sha3_256()), data)
    }

    #[pyfunction]
    fn sha3_384(data: OptionalArg<PyBytesRef>) -> PyResult<PyHasher> {
        init(PyHasher::new("sha3_384", HashWrapper::sha3_384()), data)
    }

    #[pyfunction]
    fn sha3_512(data: OptionalArg<PyBytesRef>) -> PyResult<PyHasher> {
        init(PyHasher::new("sha3_512", HashWrapper::sha3_512()), data)
    }

    #[pyfunction]
    fn shake128(_data: OptionalArg<PyBytesRef>, vm: &VirtualMachine) -> PyResult<PyHasher> {
        Err(vm.new_not_implemented_error("shake256".to_owned()))
    }

    #[pyfunction]
    fn shake256(_data: OptionalArg<PyBytesRef>, vm: &VirtualMachine) -> PyResult<PyHasher> {
        Err(vm.new_not_implemented_error("shake256".to_owned()))
    }

    #[pyfunction]
    fn blake2b(data: OptionalArg<PyBytesRef>) -> PyResult<PyHasher> {
        // TODO: handle parameters
        init(PyHasher::new("blake2b", HashWrapper::blake2b()), data)
    }

    #[pyfunction]
    fn blake2s(data: OptionalArg<PyBytesRef>) -> PyResult<PyHasher> {
        // TODO: handle parameters
        init(PyHasher::new("blake2s", HashWrapper::blake2s()), data)
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
            fn shake128() -> Self {
                Self::new(Shake128::default())
            }

            fn shake256() -> Self {
                Self::new(Shake256::default())
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
