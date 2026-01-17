// spell-checker:ignore usedforsecurity HASHXOF

pub(crate) use _hashlib::make_module;

#[pymodule]
pub mod _hashlib {
    use crate::common::lock::PyRwLock;
    use crate::vm::{
        Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
        builtins::{PyBytes, PyStrRef, PyTypeRef, PyValueError},
        class::StaticType,
        convert::ToPyObject,
        function::{ArgBytesLike, ArgStrOrBytesLike, FuncArgs, OptionalArg},
        types::{Constructor, Representable},
    };
    use blake2::{Blake2b512, Blake2s256};
    use digest::{DynDigest, core_api::BlockSizeUser};
    use digest::{ExtendableOutput, Update};
    use dyn_clone::{DynClone, clone_trait_object};
    use md5::Md5;
    use sha1::Sha1;
    use sha2::{Sha224, Sha256, Sha384, Sha512};
    use sha3::{Sha3_224, Sha3_256, Sha3_384, Sha3_512, Shake128, Shake256};

    #[pyattr]
    #[pyexception(name = "UnsupportedDigestmodError", base = PyValueError, impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct UnsupportedDigestmodError(PyValueError);

    #[derive(FromArgs, Debug)]
    #[allow(unused)]
    struct NewHashArgs {
        #[pyarg(positional)]
        name: PyStrRef,
        #[pyarg(any, optional)]
        data: OptionalArg<ArgBytesLike>,
        #[pyarg(named, default = true)]
        usedforsecurity: bool,
    }

    #[derive(FromArgs)]
    #[allow(unused)]
    pub struct BlakeHashArgs {
        #[pyarg(positional, optional)]
        pub data: OptionalArg<ArgBytesLike>,
        #[pyarg(named, default = true)]
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

    #[derive(FromArgs, Debug)]
    #[allow(unused)]
    pub struct HashArgs {
        #[pyarg(any, optional)]
        pub string: OptionalArg<ArgBytesLike>,
        #[pyarg(named, default = true)]
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
                .map_err(|_| vm.new_value_error("length must be non-negative"))
        }
    }

    #[pyattr]
    #[pyclass(module = "_hashlib", name = "HASH")]
    #[derive(PyPayload)]
    pub struct PyHasher {
        pub name: String,
        pub ctx: PyRwLock<HashWrapper>,
    }

    impl core::fmt::Debug for PyHasher {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "HASH {}", self.name)
        }
    }

    #[pyclass(with(Representable))]
    impl PyHasher {
        fn new(name: &str, d: HashWrapper) -> Self {
            Self {
                name: name.to_owned(),
                ctx: PyRwLock::new(d),
            }
        }

        #[pyslot]
        fn slot_new(_cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            Err(vm.new_type_error("cannot create '_hashlib.HASH' instances"))
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
            Self::new(&self.name, self.ctx.read().clone())
        }
    }

    impl Representable for PyHasher {
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok(format!(
                "<{} _hashlib.HASH object @ {:#x}>",
                zelf.name, zelf as *const _ as usize
            ))
        }
    }

    #[pyattr]
    #[pyclass(module = "_hashlib", name = "HASHXOF")]
    #[derive(PyPayload)]
    pub struct PyHasherXof {
        name: String,
        ctx: PyRwLock<HashXofWrapper>,
    }

    impl core::fmt::Debug for PyHasherXof {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "HASHXOF {}", self.name)
        }
    }

    #[pyclass]
    impl PyHasherXof {
        fn new(name: &str, d: HashXofWrapper) -> Self {
            Self {
                name: name.to_owned(),
                ctx: PyRwLock::new(d),
            }
        }

        #[pyslot]
        fn slot_new(_cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            Err(vm.new_type_error("cannot create '_hashlib.HASHXOF' instances"))
        }

        #[pygetset]
        fn name(&self) -> String {
            self.name.clone()
        }

        #[pygetset]
        const fn digest_size(&self) -> usize {
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
            Self::new(&self.name, self.ctx.read().clone())
        }
    }

    #[pyfunction(name = "new")]
    fn hashlib_new(args: NewHashArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        match args.name.as_str().to_lowercase().as_str() {
            "md5" => Ok(local_md5(args.into()).into_pyobject(vm)),
            "sha1" => Ok(local_sha1(args.into()).into_pyobject(vm)),
            "sha224" => Ok(local_sha224(args.into()).into_pyobject(vm)),
            "sha256" => Ok(local_sha256(args.into()).into_pyobject(vm)),
            "sha384" => Ok(local_sha384(args.into()).into_pyobject(vm)),
            "sha512" => Ok(local_sha512(args.into()).into_pyobject(vm)),
            "sha3_224" => Ok(local_sha3_224(args.into()).into_pyobject(vm)),
            "sha3_256" => Ok(local_sha3_256(args.into()).into_pyobject(vm)),
            "sha3_384" => Ok(local_sha3_384(args.into()).into_pyobject(vm)),
            "sha3_512" => Ok(local_sha3_512(args.into()).into_pyobject(vm)),
            "shake_128" => Ok(local_shake_128(args.into()).into_pyobject(vm)),
            "shake_256" => Ok(local_shake_256(args.into()).into_pyobject(vm)),
            "blake2b" => Ok(local_blake2b(args.into()).into_pyobject(vm)),
            "blake2s" => Ok(local_blake2s(args.into()).into_pyobject(vm)),
            other => Err(vm.new_value_error(format!("Unknown hashing algorithm: {other}"))),
        }
    }

    #[pyfunction(name = "openssl_md5")]
    pub fn local_md5(args: HashArgs) -> PyHasher {
        PyHasher::new("md5", HashWrapper::new::<Md5>(args.string))
    }

    #[pyfunction(name = "openssl_sha1")]
    pub fn local_sha1(args: HashArgs) -> PyHasher {
        PyHasher::new("sha1", HashWrapper::new::<Sha1>(args.string))
    }

    #[pyfunction(name = "openssl_sha224")]
    pub fn local_sha224(args: HashArgs) -> PyHasher {
        PyHasher::new("sha224", HashWrapper::new::<Sha224>(args.string))
    }

    #[pyfunction(name = "openssl_sha256")]
    pub fn local_sha256(args: HashArgs) -> PyHasher {
        PyHasher::new("sha256", HashWrapper::new::<Sha256>(args.string))
    }

    #[pyfunction(name = "openssl_sha384")]
    pub fn local_sha384(args: HashArgs) -> PyHasher {
        PyHasher::new("sha384", HashWrapper::new::<Sha384>(args.string))
    }

    #[pyfunction(name = "openssl_sha512")]
    pub fn local_sha512(args: HashArgs) -> PyHasher {
        PyHasher::new("sha512", HashWrapper::new::<Sha512>(args.string))
    }

    #[pyfunction(name = "openssl_sha3_224")]
    pub fn local_sha3_224(args: HashArgs) -> PyHasher {
        PyHasher::new("sha3_224", HashWrapper::new::<Sha3_224>(args.string))
    }

    #[pyfunction(name = "openssl_sha3_256")]
    pub fn local_sha3_256(args: HashArgs) -> PyHasher {
        PyHasher::new("sha3_256", HashWrapper::new::<Sha3_256>(args.string))
    }

    #[pyfunction(name = "openssl_sha3_384")]
    pub fn local_sha3_384(args: HashArgs) -> PyHasher {
        PyHasher::new("sha3_384", HashWrapper::new::<Sha3_384>(args.string))
    }

    #[pyfunction(name = "openssl_sha3_512")]
    pub fn local_sha3_512(args: HashArgs) -> PyHasher {
        PyHasher::new("sha3_512", HashWrapper::new::<Sha3_512>(args.string))
    }

    #[pyfunction(name = "openssl_shake_128")]
    pub fn local_shake_128(args: HashArgs) -> PyHasherXof {
        PyHasherXof::new("shake_128", HashXofWrapper::new_shake_128(args.string))
    }

    #[pyfunction(name = "openssl_shake_256")]
    pub fn local_shake_256(args: HashArgs) -> PyHasherXof {
        PyHasherXof::new("shake_256", HashXofWrapper::new_shake_256(args.string))
    }

    #[pyfunction(name = "openssl_blake2b")]
    pub fn local_blake2b(args: BlakeHashArgs) -> PyHasher {
        PyHasher::new("blake2b", HashWrapper::new::<Blake2b512>(args.data))
    }

    #[pyfunction(name = "openssl_blake2s")]
    pub fn local_blake2s(args: BlakeHashArgs) -> PyHasher {
        PyHasher::new("blake2s", HashWrapper::new::<Blake2s256>(args.data))
    }

    #[pyfunction]
    fn compare_digest(
        a: ArgStrOrBytesLike,
        b: ArgStrOrBytesLike,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        const fn is_str(arg: &ArgStrOrBytesLike) -> bool {
            matches!(arg, ArgStrOrBytesLike::Str(_))
        }

        if is_str(&a) != is_str(&b) {
            return Err(vm.new_type_error(format!(
                "a bytes-like object is required, not '{}'",
                b.as_object().class().name()
            )));
        }

        let a_hash = a.borrow_bytes().to_vec();
        let b_hash = b.borrow_bytes().to_vec();

        Ok((a_hash == b_hash).to_pyobject(vm))
    }

    #[derive(FromArgs, Debug)]
    #[allow(unused)]
    pub struct NewHMACHashArgs {
        #[pyarg(positional)]
        key: ArgBytesLike,
        #[pyarg(any, optional)]
        msg: OptionalArg<ArgBytesLike>,
        #[pyarg(named, optional)]
        digestmod: OptionalArg<PyObjectRef>,
    }

    #[pyfunction]
    fn hmac_new(args: NewHMACHashArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        // Raise UnsupportedDigestmodError so Python's hmac.py falls back to pure-Python implementation
        let _ = args;
        Err(vm.new_exception_msg(
            UnsupportedDigestmodError::static_type().to_owned(),
            "unsupported hash type".to_owned(),
        ))
    }

    pub trait ThreadSafeDynDigest: DynClone + DynDigest + Sync + Send {}
    impl<T> ThreadSafeDynDigest for T where T: DynClone + DynDigest + Sync + Send {}

    clone_trait_object!(ThreadSafeDynDigest);

    /// Generic wrapper patching around the hashing libraries.
    #[derive(Clone)]
    pub struct HashWrapper {
        block_size: usize,
        inner: Box<dyn ThreadSafeDynDigest>,
    }

    impl HashWrapper {
        pub fn new<D>(data: OptionalArg<ArgBytesLike>) -> Self
        where
            D: ThreadSafeDynDigest + BlockSizeUser + Default + 'static,
        {
            let mut h = Self {
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

        const fn block_size(&self) -> usize {
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
    pub enum HashXofWrapper {
        Shake128(Shake128),
        Shake256(Shake256),
    }

    impl HashXofWrapper {
        pub fn new_shake_128(data: OptionalArg<ArgBytesLike>) -> Self {
            let mut h = Self::Shake128(Shake128::default());
            if let OptionalArg::Present(d) = data {
                d.with_ref(|bytes| h.update(bytes));
            }
            h
        }

        pub fn new_shake_256(data: OptionalArg<ArgBytesLike>) -> Self {
            let mut h = Self::Shake256(Shake256::default());
            if let OptionalArg::Present(d) = data {
                d.with_ref(|bytes| h.update(bytes));
            }
            h
        }

        fn update(&mut self, data: &[u8]) {
            match self {
                Self::Shake128(h) => h.update(data),
                Self::Shake256(h) => h.update(data),
            }
        }

        fn block_size(&self) -> usize {
            match self {
                Self::Shake128(_) => Shake128::block_size(),
                Self::Shake256(_) => Shake256::block_size(),
            }
        }

        fn finalize_xof(&self, length: usize) -> Vec<u8> {
            match self {
                Self::Shake128(h) => h.clone().finalize_boxed(length).into_vec(),
                Self::Shake256(h) => h.clone().finalize_boxed(length).into_vec(),
            }
        }
    }
}
