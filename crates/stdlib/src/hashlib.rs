// spell-checker:ignore usedforsecurity HASHXOF hashopenssl dklen
// NOTE: Function names like `openssl_md5` match CPython's `_hashopenssl.c` interface
// for compatibility, but the implementation uses pure Rust crates (md5, sha2, etc.),
// not OpenSSL.

pub(crate) use _hashlib::module_def;

#[pymodule]
pub mod _hashlib {
    use crate::common::lock::PyRwLock;
    use crate::vm::{
        Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
        builtins::{
            PyBaseExceptionRef, PyBytes, PyFrozenSet, PyStr, PyTypeRef, PyUtf8StrRef, PyValueError,
        },
        class::StaticType,
        convert::ToPyObject,
        function::{ArgBytesLike, ArgStrOrBytesLike, FuncArgs, OptionalArg},
        types::{Constructor, Representable},
    };
    use blake2::{Blake2b512, Blake2s256};
    use digest::{DynDigest, OutputSizeUser, core_api::BlockSizeUser};
    use digest::{ExtendableOutput, Update};
    use dyn_clone::{DynClone, clone_trait_object};
    use hmac::Mac;
    use md5::Md5;
    use sha1::Sha1;
    use sha2::{Sha224, Sha256, Sha384, Sha512};
    use sha3::{Sha3_224, Sha3_256, Sha3_384, Sha3_512, Shake128, Shake256};

    const HASH_ALGORITHMS: &[&str] = &[
        "md5",
        "sha1",
        "sha224",
        "sha256",
        "sha384",
        "sha512",
        "sha3_224",
        "sha3_256",
        "sha3_384",
        "sha3_512",
        "shake_128",
        "shake_256",
        "blake2b",
        "blake2s",
    ];

    #[pyattr]
    const _GIL_MINSIZE: usize = 2048;

    #[pyattr]
    #[pyexception(name = "UnsupportedDigestmodError", base = PyValueError, impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub struct UnsupportedDigestmodError(PyValueError);

    #[pyattr]
    fn openssl_md_meth_names(vm: &VirtualMachine) -> PyObjectRef {
        PyFrozenSet::from_iter(
            vm,
            HASH_ALGORITHMS.iter().map(|n| vm.ctx.new_str(*n).into()),
        )
        .expect("failed to create openssl_md_meth_names frozenset")
        .into_ref(&vm.ctx)
        .into()
    }

    #[pyattr]
    fn _constructors(vm: &VirtualMachine) -> PyObjectRef {
        let dict = vm.ctx.new_dict();
        for name in HASH_ALGORITHMS {
            let s = vm.ctx.new_str(*name);
            dict.set_item(&*s, s.clone().into(), vm).unwrap();
        }
        dict.into()
    }

    #[derive(FromArgs, Debug)]
    #[allow(unused)]
    struct NewHashArgs {
        #[pyarg(positional)]
        name: PyUtf8StrRef,
        #[pyarg(any, optional)]
        data: OptionalArg<ArgBytesLike>,
        #[pyarg(named, default = true)]
        usedforsecurity: bool,
        #[pyarg(named, optional)]
        string: OptionalArg<ArgBytesLike>,
    }

    #[derive(FromArgs)]
    #[allow(unused)]
    pub struct BlakeHashArgs {
        #[pyarg(any, optional)]
        pub data: OptionalArg<ArgBytesLike>,
        #[pyarg(named, default = true)]
        usedforsecurity: bool,
        #[pyarg(named, optional)]
        pub string: OptionalArg<ArgBytesLike>,
    }

    impl From<NewHashArgs> for BlakeHashArgs {
        fn from(args: NewHashArgs) -> Self {
            Self {
                data: args.data,
                usedforsecurity: args.usedforsecurity,
                string: args.string,
            }
        }
    }

    #[derive(FromArgs, Debug)]
    #[allow(unused)]
    pub struct HashArgs {
        #[pyarg(any, optional)]
        pub data: OptionalArg<ArgBytesLike>,
        #[pyarg(named, default = true)]
        usedforsecurity: bool,
        #[pyarg(named, optional)]
        pub string: OptionalArg<ArgBytesLike>,
    }

    impl From<NewHashArgs> for HashArgs {
        fn from(args: NewHashArgs) -> Self {
            Self {
                data: args.data,
                usedforsecurity: args.usedforsecurity,
                string: args.string,
            }
        }
    }

    const KECCAK_WIDTH_BITS: usize = 1600;

    fn keccak_suffix(name: &str) -> Option<u8> {
        match name {
            "sha3_224" | "sha3_256" | "sha3_384" | "sha3_512" => Some(0x06),
            "shake_128" | "shake_256" => Some(0x1f),
            _ => None,
        }
    }

    fn keccak_rate_bits(name: &str, block_size: usize) -> Option<usize> {
        keccak_suffix(name).map(|_| block_size * 8)
    }

    fn keccak_capacity_bits(name: &str, block_size: usize) -> Option<usize> {
        keccak_rate_bits(name, block_size).map(|rate| KECCAK_WIDTH_BITS - rate)
    }

    fn missing_hash_attribute<T>(vm: &VirtualMachine, class_name: &str, attr: &str) -> PyResult<T> {
        Err(vm.new_attribute_error(format!("'{class_name}' object has no attribute '{attr}'")))
    }

    #[derive(FromArgs)]
    #[allow(unused)]
    struct XofDigestArgs {
        #[pyarg(positional)]
        length: isize,
    }

    impl XofDigestArgs {
        // Match CPython's SHAKE output guard in Modules/sha3module.c.
        const MAX_SHAKE_OUTPUT_LENGTH: usize = 1 << 29;

        fn length(&self, vm: &VirtualMachine) -> PyResult<usize> {
            let length = usize::try_from(self.length)
                .map_err(|_| vm.new_value_error("length must be non-negative"))?;
            if length >= Self::MAX_SHAKE_OUTPUT_LENGTH {
                return Err(vm.new_value_error("length is too large"));
            }
            Ok(length)
        }
    }

    #[derive(FromArgs)]
    #[allow(unused)]
    struct HmacDigestArgs {
        #[pyarg(positional)]
        key: ArgBytesLike,
        #[pyarg(positional)]
        msg: ArgBytesLike,
        #[pyarg(positional)]
        digest: PyObjectRef,
    }

    #[derive(FromArgs)]
    #[allow(unused)]
    struct Pbkdf2HmacArgs {
        #[pyarg(any)]
        hash_name: PyUtf8StrRef,
        #[pyarg(any)]
        password: ArgBytesLike,
        #[pyarg(any)]
        salt: ArgBytesLike,
        #[pyarg(any)]
        iterations: i64,
        #[pyarg(any, optional)]
        dklen: OptionalArg<PyObjectRef>,
    }

    fn resolve_data(
        data: OptionalArg<ArgBytesLike>,
        string: OptionalArg<ArgBytesLike>,
        vm: &VirtualMachine,
    ) -> PyResult<OptionalArg<ArgBytesLike>> {
        match (data.into_option(), string.into_option()) {
            (Some(d), None) => Ok(OptionalArg::Present(d)),
            (None, Some(s)) => Ok(OptionalArg::Present(s)),
            (None, None) => Ok(OptionalArg::Missing),
            (Some(_), Some(_)) => Err(vm.new_type_error(
                "'data' and 'string' are mutually exclusive \
                 and support for 'string' keyword parameter \
                 is slated for removal in a future version."
                    .to_owned(),
            )),
        }
    }

    fn resolve_digestmod(digestmod: &PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
        if let Some(name) = digestmod.downcast_ref::<PyStr>()
            && let Some(name_str) = name.to_str()
        {
            return Ok(name_str.to_lowercase());
        }
        if let Ok(name_obj) = digestmod.get_attr("__name__", vm)
            && let Some(name) = name_obj.downcast_ref::<PyStr>()
            && let Some(name_str) = name.to_str()
            && let Some(algo) = name_str.strip_prefix("openssl_")
        {
            return Ok(algo.to_owned());
        }
        Err(vm.new_exception_msg(
            UnsupportedDigestmodError::static_type().to_owned(),
            "unsupported digestmod".into(),
        ))
    }

    fn hash_digest_size(name: &str) -> Option<usize> {
        match name {
            "md5" => Some(16),
            "sha1" => Some(20),
            "sha224" => Some(28),
            "sha256" => Some(32),
            "sha384" => Some(48),
            "sha512" => Some(64),
            "sha3_224" => Some(28),
            "sha3_256" => Some(32),
            "sha3_384" => Some(48),
            "sha3_512" => Some(64),
            "blake2b" => Some(64),
            "blake2s" => Some(32),
            _ => None,
        }
    }

    fn unsupported_hash(name: &str, vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_exception_msg(
            UnsupportedDigestmodError::static_type().to_owned(),
            format!("unsupported hash type {name}").into(),
        )
    }

    // Object-safe HMAC trait for type-erased dispatch
    trait DynHmac: Send + Sync {
        fn dyn_update(&mut self, data: &[u8]);
        fn dyn_finalize(&self) -> Vec<u8>;
        fn dyn_clone(&self) -> Box<dyn DynHmac>;
    }

    struct TypedHmac<D>(D);

    impl<D> DynHmac for TypedHmac<D>
    where
        D: Mac + Clone + Send + Sync + 'static,
    {
        fn dyn_update(&mut self, data: &[u8]) {
            Mac::update(&mut self.0, data);
        }

        fn dyn_finalize(&self) -> Vec<u8> {
            self.0.clone().finalize().into_bytes().to_vec()
        }

        fn dyn_clone(&self) -> Box<dyn DynHmac> {
            Box::new(TypedHmac(self.0.clone()))
        }
    }

    #[pyattr]
    #[pyclass(module = "_hashlib", name = "HMAC")]
    #[derive(PyPayload)]
    pub struct PyHmac {
        algo_name: String,
        digest_size: usize,
        block_size: usize,
        ctx: PyRwLock<Box<dyn DynHmac>>,
    }

    impl core::fmt::Debug for PyHmac {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "HMAC {}", self.algo_name)
        }
    }

    #[pyclass(with(Representable), flags(IMMUTABLETYPE))]
    impl PyHmac {
        #[pyslot]
        fn slot_new(_cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            Err(vm.new_type_error("cannot create '_hashlib.HMAC' instances".to_owned()))
        }

        #[pygetset]
        fn name(&self) -> String {
            format!("hmac-{}", self.algo_name)
        }

        #[pygetset]
        fn digest_size(&self) -> usize {
            self.digest_size
        }

        #[pygetset]
        fn block_size(&self) -> usize {
            self.block_size
        }

        #[pymethod]
        fn update(&self, msg: ArgBytesLike) {
            msg.with_ref(|bytes| self.ctx.write().dyn_update(bytes));
        }

        #[pymethod]
        fn digest(&self) -> PyBytes {
            self.ctx.read().dyn_finalize().into()
        }

        #[pymethod]
        fn hexdigest(&self) -> String {
            hex::encode(self.ctx.read().dyn_finalize())
        }

        #[pymethod]
        fn copy(&self) -> Self {
            Self {
                algo_name: self.algo_name.clone(),
                digest_size: self.digest_size,
                block_size: self.block_size,
                ctx: PyRwLock::new(self.ctx.read().dyn_clone()),
            }
        }
    }

    impl Representable for PyHmac {
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok(format!(
                "<{} HMAC object @ {:#x}>",
                zelf.algo_name, zelf as *const _ as usize
            ))
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

    #[pyclass(with(Representable), flags(IMMUTABLETYPE))]
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

        #[pygetset]
        fn _capacity_bits(&self, vm: &VirtualMachine) -> PyResult<usize> {
            let block_size = self.ctx.read().block_size();
            match keccak_capacity_bits(&self.name, block_size) {
                Some(capacity) => Ok(capacity),
                None => missing_hash_attribute(vm, "HASH", "_capacity_bits"),
            }
        }

        #[pygetset]
        fn _rate_bits(&self, vm: &VirtualMachine) -> PyResult<usize> {
            let block_size = self.ctx.read().block_size();
            match keccak_rate_bits(&self.name, block_size) {
                Some(rate) => Ok(rate),
                None => missing_hash_attribute(vm, "HASH", "_rate_bits"),
            }
        }

        #[pygetset]
        fn _suffix(&self, vm: &VirtualMachine) -> PyResult<PyBytes> {
            match keccak_suffix(&self.name) {
                Some(suffix) => Ok(vec![suffix].into()),
                None => missing_hash_attribute(vm, "HASH", "_suffix"),
            }
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

    #[pyclass(with(Representable), flags(IMMUTABLETYPE))]
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

        #[pygetset]
        fn _capacity_bits(&self, vm: &VirtualMachine) -> PyResult<usize> {
            let block_size = self.ctx.read().block_size();
            match keccak_capacity_bits(&self.name, block_size) {
                Some(capacity) => Ok(capacity),
                None => missing_hash_attribute(vm, "HASHXOF", "_capacity_bits"),
            }
        }

        #[pygetset]
        fn _rate_bits(&self, vm: &VirtualMachine) -> PyResult<usize> {
            let block_size = self.ctx.read().block_size();
            match keccak_rate_bits(&self.name, block_size) {
                Some(rate) => Ok(rate),
                None => missing_hash_attribute(vm, "HASHXOF", "_rate_bits"),
            }
        }

        #[pygetset]
        fn _suffix(&self, vm: &VirtualMachine) -> PyResult<PyBytes> {
            match keccak_suffix(&self.name) {
                Some(suffix) => Ok(vec![suffix].into()),
                None => missing_hash_attribute(vm, "HASHXOF", "_suffix"),
            }
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

    impl Representable for PyHasherXof {
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok(format!(
                "<{} _hashlib.HASHXOF object @ {:#x}>",
                zelf.name, zelf as *const _ as usize
            ))
        }
    }

    #[pyfunction(name = "new")]
    fn hashlib_new(args: NewHashArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let data = resolve_data(args.data, args.string, vm)?;
        match args.name.as_str().to_lowercase().as_str() {
            "md5" => Ok(PyHasher::new("md5", HashWrapper::new::<Md5>(data)).into_pyobject(vm)),
            "sha1" => Ok(PyHasher::new("sha1", HashWrapper::new::<Sha1>(data)).into_pyobject(vm)),
            "sha224" => {
                Ok(PyHasher::new("sha224", HashWrapper::new::<Sha224>(data)).into_pyobject(vm))
            }
            "sha256" => {
                Ok(PyHasher::new("sha256", HashWrapper::new::<Sha256>(data)).into_pyobject(vm))
            }
            "sha384" => {
                Ok(PyHasher::new("sha384", HashWrapper::new::<Sha384>(data)).into_pyobject(vm))
            }
            "sha512" => {
                Ok(PyHasher::new("sha512", HashWrapper::new::<Sha512>(data)).into_pyobject(vm))
            }
            "sha3_224" => {
                Ok(PyHasher::new("sha3_224", HashWrapper::new::<Sha3_224>(data)).into_pyobject(vm))
            }
            "sha3_256" => {
                Ok(PyHasher::new("sha3_256", HashWrapper::new::<Sha3_256>(data)).into_pyobject(vm))
            }
            "sha3_384" => {
                Ok(PyHasher::new("sha3_384", HashWrapper::new::<Sha3_384>(data)).into_pyobject(vm))
            }
            "sha3_512" => {
                Ok(PyHasher::new("sha3_512", HashWrapper::new::<Sha3_512>(data)).into_pyobject(vm))
            }
            "shake_128" => Ok(
                PyHasherXof::new("shake_128", HashXofWrapper::new_shake_128(data))
                    .into_pyobject(vm),
            ),
            "shake_256" => Ok(
                PyHasherXof::new("shake_256", HashXofWrapper::new_shake_256(data))
                    .into_pyobject(vm),
            ),
            "blake2b" => Ok(
                PyHasher::new("blake2b", HashWrapper::new::<Blake2b512>(data)).into_pyobject(vm),
            ),
            "blake2s" => Ok(
                PyHasher::new("blake2s", HashWrapper::new::<Blake2s256>(data)).into_pyobject(vm),
            ),
            other => Err(vm.new_value_error(format!("Unknown hashing algorithm: {other}"))),
        }
    }

    #[pyfunction(name = "openssl_md5")]
    pub fn local_md5(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(PyHasher::new("md5", HashWrapper::new::<Md5>(data)))
    }

    #[pyfunction(name = "openssl_sha1")]
    pub fn local_sha1(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(PyHasher::new("sha1", HashWrapper::new::<Sha1>(data)))
    }

    #[pyfunction(name = "openssl_sha224")]
    pub fn local_sha224(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(PyHasher::new("sha224", HashWrapper::new::<Sha224>(data)))
    }

    #[pyfunction(name = "openssl_sha256")]
    pub fn local_sha256(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(PyHasher::new("sha256", HashWrapper::new::<Sha256>(data)))
    }

    #[pyfunction(name = "openssl_sha384")]
    pub fn local_sha384(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(PyHasher::new("sha384", HashWrapper::new::<Sha384>(data)))
    }

    #[pyfunction(name = "openssl_sha512")]
    pub fn local_sha512(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(PyHasher::new("sha512", HashWrapper::new::<Sha512>(data)))
    }

    #[pyfunction(name = "openssl_sha3_224")]
    pub fn local_sha3_224(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(PyHasher::new(
            "sha3_224",
            HashWrapper::new::<Sha3_224>(data),
        ))
    }

    #[pyfunction(name = "openssl_sha3_256")]
    pub fn local_sha3_256(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(PyHasher::new(
            "sha3_256",
            HashWrapper::new::<Sha3_256>(data),
        ))
    }

    #[pyfunction(name = "openssl_sha3_384")]
    pub fn local_sha3_384(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(PyHasher::new(
            "sha3_384",
            HashWrapper::new::<Sha3_384>(data),
        ))
    }

    #[pyfunction(name = "openssl_sha3_512")]
    pub fn local_sha3_512(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(PyHasher::new(
            "sha3_512",
            HashWrapper::new::<Sha3_512>(data),
        ))
    }

    #[pyfunction(name = "openssl_shake_128")]
    pub fn local_shake_128(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasherXof> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(PyHasherXof::new(
            "shake_128",
            HashXofWrapper::new_shake_128(data),
        ))
    }

    #[pyfunction(name = "openssl_shake_256")]
    pub fn local_shake_256(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasherXof> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(PyHasherXof::new(
            "shake_256",
            HashXofWrapper::new_shake_256(data),
        ))
    }

    #[pyfunction(name = "openssl_blake2b")]
    pub fn local_blake2b(args: BlakeHashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(PyHasher::new(
            "blake2b",
            HashWrapper::new::<Blake2b512>(data),
        ))
    }

    #[pyfunction(name = "openssl_blake2s")]
    pub fn local_blake2s(args: BlakeHashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(PyHasher::new(
            "blake2s",
            HashWrapper::new::<Blake2s256>(data),
        ))
    }

    #[pyfunction]
    fn get_fips_mode() -> i32 {
        0
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
        msg: OptionalArg<Option<ArgBytesLike>>,
        #[pyarg(named, optional)]
        digestmod: OptionalArg<PyObjectRef>,
    }

    #[pyfunction]
    fn hmac_new(args: NewHMACHashArgs, vm: &VirtualMachine) -> PyResult<PyHmac> {
        let digestmod = args.digestmod.into_option().ok_or_else(|| {
            vm.new_type_error("Missing required parameter 'digestmod'.".to_owned())
        })?;
        let name = resolve_digestmod(&digestmod, vm)?;

        let key_buf = args.key.borrow_buf();
        let msg_data = args.msg.flatten();

        macro_rules! make_hmac {
            ($hash_ty:ty) => {{
                let mut mac = <hmac::Hmac<$hash_ty> as Mac>::new_from_slice(&key_buf)
                    .map_err(|_| vm.new_value_error("invalid key length".to_owned()))?;
                if let Some(ref m) = msg_data {
                    m.with_ref(|bytes| Mac::update(&mut mac, bytes));
                }
                Ok(PyHmac {
                    algo_name: name,
                    digest_size: <$hash_ty as OutputSizeUser>::output_size(),
                    block_size: <$hash_ty as BlockSizeUser>::block_size(),
                    ctx: PyRwLock::new(Box::new(TypedHmac(mac))),
                })
            }};
        }

        match name.as_str() {
            "md5" => make_hmac!(Md5),
            "sha1" => make_hmac!(Sha1),
            "sha224" => make_hmac!(Sha224),
            "sha256" => make_hmac!(Sha256),
            "sha384" => make_hmac!(Sha384),
            "sha512" => make_hmac!(Sha512),
            "sha3_224" => make_hmac!(Sha3_224),
            "sha3_256" => make_hmac!(Sha3_256),
            "sha3_384" => make_hmac!(Sha3_384),
            "sha3_512" => make_hmac!(Sha3_512),
            _ => Err(unsupported_hash(&name, vm)),
        }
    }

    #[pyfunction]
    fn hmac_digest(args: HmacDigestArgs, vm: &VirtualMachine) -> PyResult<PyBytes> {
        let name = resolve_digestmod(&args.digest, vm)?;

        let key_buf = args.key.borrow_buf();
        let msg_buf = args.msg.borrow_buf();

        macro_rules! do_hmac {
            ($hash_ty:ty) => {{
                let mut mac = <hmac::Hmac<$hash_ty> as Mac>::new_from_slice(&key_buf)
                    .map_err(|_| vm.new_value_error("invalid key length".to_owned()))?;
                Mac::update(&mut mac, &msg_buf);
                Ok(mac.finalize().into_bytes().to_vec().into())
            }};
        }

        match name.as_str() {
            "md5" => do_hmac!(Md5),
            "sha1" => do_hmac!(Sha1),
            "sha224" => do_hmac!(Sha224),
            "sha256" => do_hmac!(Sha256),
            "sha384" => do_hmac!(Sha384),
            "sha512" => do_hmac!(Sha512),
            "sha3_224" => do_hmac!(Sha3_224),
            "sha3_256" => do_hmac!(Sha3_256),
            "sha3_384" => do_hmac!(Sha3_384),
            "sha3_512" => do_hmac!(Sha3_512),
            _ => Err(unsupported_hash(&name, vm)),
        }
    }

    #[pyfunction]
    fn pbkdf2_hmac(args: Pbkdf2HmacArgs, vm: &VirtualMachine) -> PyResult<PyBytes> {
        let name = args.hash_name.as_str().to_lowercase();

        if args.iterations < 1 {
            return Err(vm.new_value_error("iteration value must be greater than 0.".to_owned()));
        }
        let rounds = u32::try_from(args.iterations)
            .map_err(|_| vm.new_overflow_error("iteration value is too great.".to_owned()))?;

        let dklen: usize = match args.dklen.into_option() {
            Some(obj) if vm.is_none(&obj) => {
                hash_digest_size(&name).ok_or_else(|| unsupported_hash(&name, vm))?
            }
            Some(obj) => {
                let len: i64 = obj.try_into_value(vm)?;
                if len < 1 {
                    return Err(vm.new_value_error("key length must be greater than 0.".to_owned()));
                }
                usize::try_from(len)
                    .map_err(|_| vm.new_overflow_error("key length is too great.".to_owned()))?
            }
            None => hash_digest_size(&name).ok_or_else(|| unsupported_hash(&name, vm))?,
        };

        let password_buf = args.password.borrow_buf();
        let salt_buf = args.salt.borrow_buf();
        let mut dk = vec![0u8; dklen];

        macro_rules! do_pbkdf2 {
            ($hash_ty:ty) => {{
                pbkdf2::pbkdf2_hmac::<$hash_ty>(&password_buf, &salt_buf, rounds, &mut dk);
                Ok(dk.into())
            }};
        }

        match name.as_str() {
            "md5" => do_pbkdf2!(Md5),
            "sha1" => do_pbkdf2!(Sha1),
            "sha224" => do_pbkdf2!(Sha224),
            "sha256" => do_pbkdf2!(Sha256),
            "sha384" => do_pbkdf2!(Sha384),
            "sha512" => do_pbkdf2!(Sha512),
            "sha3_224" => do_pbkdf2!(Sha3_224),
            "sha3_256" => do_pbkdf2!(Sha3_256),
            "sha3_384" => do_pbkdf2!(Sha3_384),
            "sha3_512" => do_pbkdf2!(Sha3_512),
            _ => Err(unsupported_hash(&name, vm)),
        }
    }

    pub trait ThreadSafeDynDigest: DynClone + DynDigest + Sync + Send {}
    impl<T> ThreadSafeDynDigest for T where T: DynClone + DynDigest + Sync + Send {}

    clone_trait_object!(ThreadSafeDynDigest);

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
