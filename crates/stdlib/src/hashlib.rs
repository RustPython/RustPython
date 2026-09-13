// spell-checker:ignore usedforsecurity HASHXOF hashopenssl dklen fanout maxmem scrypt blake2b blake2s
// NOTE: Function names like `openssl_md5` match `_hashopenssl.c` interface
// for compatibility, but the implementation uses the rustpython-common
// hashlib engine.

pub(crate) use _hashlib::module_def;

#[pymodule]
pub(crate) mod _hashlib {
    use crate::vm::{
        Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
        builtins::{
            PyBaseExceptionRef, PyBytes, PyFrozenSet, PyStr, PyTypeRef, PyUtf8StrRef, PyValueError,
        },
        class::StaticType,
        function::{ArgBytesLike, ArgPrimitiveIndex, ArgStrOrBytesLike, FuncArgs, OptionalArg},
        types::{Constructor, Representable},
    };
    use core::mem::MaybeUninit;
    use rustpython_common::hashlib as backend;

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
    pub(crate) struct UnsupportedDigestmodError(PyValueError);

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
    pub(crate) struct BlakeHashArgs {
        #[pyarg(any, optional)]
        pub data: OptionalArg<ArgBytesLike>,
        #[pyarg(named, optional)]
        digest_size: OptionalArg<ArgPrimitiveIndex<i64>>,
        #[pyarg(named, optional)]
        key: OptionalArg<ArgBytesLike>,
        #[pyarg(named, optional)]
        salt: OptionalArg<ArgBytesLike>,
        #[pyarg(named, optional)]
        person: OptionalArg<ArgBytesLike>,
        #[pyarg(named, optional)]
        fanout: OptionalArg<ArgPrimitiveIndex<i64>>,
        #[pyarg(named, optional)]
        depth: OptionalArg<ArgPrimitiveIndex<i64>>,
        #[pyarg(named, optional)]
        leaf_size: OptionalArg<PyObjectRef>,
        #[pyarg(named, optional)]
        node_offset: OptionalArg<PyObjectRef>,
        #[pyarg(named, optional)]
        node_depth: OptionalArg<ArgPrimitiveIndex<i64>>,
        #[pyarg(named, optional)]
        inner_size: OptionalArg<ArgPrimitiveIndex<i64>>,
        #[pyarg(named, default = false)]
        last_node: bool,
        #[pyarg(named, default = true)]
        usedforsecurity: bool,
        #[pyarg(named, optional)]
        pub string: OptionalArg<ArgBytesLike>,
    }

    #[derive(FromArgs, Debug)]
    #[allow(unused)]
    pub(crate) struct HashArgs {
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
        // Match SHAKE output guard in Modules/sha3module.c.
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
                 is slated for removal in a future version.",
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

    fn unsupported_hash(name: &str, vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_exception_msg(
            UnsupportedDigestmodError::static_type().to_owned(),
            format!("unsupported hash type {name}").into(),
        )
    }

    fn shake_block_size(name: &str) -> Option<usize> {
        // SHAKE128: 1344 / 8 = 168
        // SHAKE256: 1088 / 8 = 136
        match name {
            "shake_128" => Some(168),
            "shake_256" => Some(136),
            _ => None,
        }
    }

    fn hasher_block_size(name: &str) -> usize {
        shake_block_size(name)
            .or_else(|| backend::digest_block_size(name))
            .unwrap_or(0)
    }

    #[repr(C, align(16))]
    #[repr(align(16))]
    struct RawHashState {
        words: [MaybeUninit<usize>; backend::HASH_STATE_STORAGE_WORDS],
    }

    const _: () = assert!(align_of::<RawHashState>() >= backend::HASH_STATE_STORAGE_ALIGN);

    struct HashCtx {
        raw: Box<RawHashState>,
    }

    // SAFETY: `raw` holds a `Mutex<HashState>` written by `state_init`.
    unsafe impl Send for HashCtx {}
    unsafe impl Sync for HashCtx {}

    impl HashCtx {
        fn new(name: &str) -> Option<Self> {
            let mut raw = Box::new(RawHashState {
                words: [MaybeUninit::uninit(); backend::HASH_STATE_STORAGE_WORDS],
            });
            let ok = unsafe {
                backend::state_init(
                    raw.words.as_mut_ptr().cast(),
                    backend::HASH_STATE_STORAGE_WORDS,
                    name,
                )
            };
            ok.then(|| Self { raw })
        }

        #[allow(clippy::too_many_arguments)]
        fn new_blake2(
            name: &str,
            digest_size: usize,
            key: &[u8],
            salt: &[u8],
            person: &[u8],
            fanout: u8,
            depth: u8,
            leaf_size: u32,
            node_offset: u64,
            node_depth: u8,
            inner_size: usize,
            last_node: bool,
        ) -> Option<Self> {
            let mut raw = Box::new(RawHashState {
                words: [MaybeUninit::uninit(); backend::HASH_STATE_STORAGE_WORDS],
            });
            let ok = unsafe {
                backend::state_init_blake2(
                    raw.words.as_mut_ptr().cast(),
                    backend::HASH_STATE_STORAGE_WORDS,
                    name,
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
                )
            };
            ok.then(|| Self { raw })
        }

        fn ptr(&self) -> *mut usize {
            self.raw.words.as_ptr().cast_mut().cast()
        }

        fn update(&self, data: &[u8]) {
            unsafe {
                backend::state_update(self.ptr(), backend::HASH_STATE_STORAGE_WORDS, data);
            }
        }

        fn digest(&self, length: usize) -> Vec<u8> {
            unsafe { backend::state_digest(self.ptr(), backend::HASH_STATE_STORAGE_WORDS, length) }
        }

        fn copy(&self) -> Self {
            let mut dst = Box::new(RawHashState {
                words: [MaybeUninit::uninit(); backend::HASH_STATE_STORAGE_WORDS],
            });
            unsafe {
                backend::state_copy(
                    self.ptr(),
                    dst.words.as_mut_ptr().cast(),
                    backend::HASH_STATE_STORAGE_WORDS,
                );
            }
            Self { raw: dst }
        }
    }

    impl Drop for HashCtx {
        fn drop(&mut self) {
            unsafe {
                backend::state_drop(self.ptr(), backend::HASH_STATE_STORAGE_WORDS);
            }
        }
    }

    #[repr(C, align(16))]
    #[repr(align(16))]
    struct RawHmacState {
        words: [MaybeUninit<usize>; backend::HMAC_STATE_STORAGE_WORDS],
    }

    const _: () = assert!(align_of::<RawHmacState>() >= backend::HMAC_STATE_STORAGE_ALIGN);

    struct HmacCtx {
        raw: Box<RawHmacState>,
    }

    // SAFETY: `raw` holds a `Mutex<HmacState>` written by `hmac_state_init`.
    unsafe impl Send for HmacCtx {}
    unsafe impl Sync for HmacCtx {}

    impl HmacCtx {
        fn new(name: &str, key: &[u8]) -> Option<Self> {
            let mut raw = Box::new(RawHmacState {
                words: [MaybeUninit::uninit(); backend::HMAC_STATE_STORAGE_WORDS],
            });
            let ok = unsafe {
                backend::hmac_state_init(
                    raw.words.as_mut_ptr().cast(),
                    backend::HMAC_STATE_STORAGE_WORDS,
                    name,
                    key,
                )
            };
            ok.then(|| Self { raw })
        }

        fn ptr(&self) -> *mut usize {
            self.raw.words.as_ptr().cast_mut().cast()
        }

        fn update(&self, data: &[u8]) {
            unsafe {
                backend::hmac_state_update(self.ptr(), backend::HMAC_STATE_STORAGE_WORDS, data);
            }
        }

        fn digest(&self) -> Vec<u8> {
            unsafe { backend::hmac_state_digest(self.ptr(), backend::HMAC_STATE_STORAGE_WORDS) }
        }

        fn copy(&self) -> Self {
            let mut dst = Box::new(RawHmacState {
                words: [MaybeUninit::uninit(); backend::HMAC_STATE_STORAGE_WORDS],
            });
            unsafe {
                backend::hmac_state_copy(
                    self.ptr(),
                    dst.words.as_mut_ptr().cast(),
                    backend::HMAC_STATE_STORAGE_WORDS,
                );
            }
            Self { raw: dst }
        }
    }

    impl Drop for HmacCtx {
        fn drop(&mut self) {
            unsafe {
                backend::hmac_state_drop(self.ptr(), backend::HMAC_STATE_STORAGE_WORDS);
            }
        }
    }

    fn hash_ctx_from_data(name: &str, data: OptionalArg<ArgBytesLike>) -> Option<HashCtx> {
        let ctx = HashCtx::new(name)?;
        if let OptionalArg::Present(d) = data {
            d.with_ref(|bytes| ctx.update(bytes));
        }
        Some(ctx)
    }

    #[pyattr]
    #[pyclass(module = "_hashlib", name = "HMAC")]
    #[derive(PyPayload)]
    pub(crate) struct PyHmac {
        algo_name: String,
        digest_size: usize,
        block_size: usize,
        ctx: HmacCtx,
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
            Err(vm.new_type_error("cannot create '_hashlib.HMAC' instances"))
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
            msg.with_ref(|bytes| self.ctx.update(bytes));
        }

        #[pymethod]
        fn digest(&self) -> PyBytes {
            self.ctx.digest().into()
        }

        #[pymethod]
        fn hexdigest(&self) -> String {
            hex::encode(self.ctx.digest())
        }

        #[pymethod]
        fn copy(&self) -> Self {
            Self {
                algo_name: self.algo_name.clone(),
                digest_size: self.digest_size,
                block_size: self.block_size,
                ctx: self.ctx.copy(),
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
    pub(crate) struct PyHasher {
        pub name: String,
        digest_size: usize,
        ctx: HashCtx,
    }

    impl core::fmt::Debug for PyHasher {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "HASH {}", self.name)
        }
    }

    #[pyclass(with(Representable), flags(IMMUTABLETYPE))]
    impl PyHasher {
        fn new(name: &str, ctx: HashCtx, digest_size: usize) -> Self {
            Self {
                name: name.to_owned(),
                digest_size,
                ctx,
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
            self.digest_size
        }

        #[pygetset]
        fn block_size(&self) -> usize {
            hasher_block_size(&self.name)
        }

        #[pygetset]
        fn _capacity_bits(&self, vm: &VirtualMachine) -> PyResult<usize> {
            let block_size = hasher_block_size(&self.name);
            match keccak_capacity_bits(&self.name, block_size) {
                Some(capacity) => Ok(capacity),
                None => missing_hash_attribute(vm, "HASH", "_capacity_bits"),
            }
        }

        #[pygetset]
        fn _rate_bits(&self, vm: &VirtualMachine) -> PyResult<usize> {
            let block_size = hasher_block_size(&self.name);
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
            data.with_ref(|bytes| self.ctx.update(bytes));
        }

        #[pymethod]
        fn digest(&self) -> PyBytes {
            self.ctx.digest(self.digest_size).into()
        }

        #[pymethod]
        fn hexdigest(&self) -> String {
            hex::encode(self.ctx.digest(self.digest_size))
        }

        #[pymethod]
        fn copy(&self) -> Self {
            Self::new(&self.name, self.ctx.copy(), self.digest_size)
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
    pub(crate) struct PyHasherXof {
        name: String,
        ctx: HashCtx,
    }

    impl core::fmt::Debug for PyHasherXof {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "HASHXOF {}", self.name)
        }
    }

    #[pyclass(with(Representable), flags(IMMUTABLETYPE))]
    impl PyHasherXof {
        fn new(name: &str, ctx: HashCtx) -> Self {
            Self {
                name: name.to_owned(),
                ctx,
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
            hasher_block_size(&self.name)
        }

        #[pygetset]
        fn _capacity_bits(&self, vm: &VirtualMachine) -> PyResult<usize> {
            let block_size = hasher_block_size(&self.name);
            match keccak_capacity_bits(&self.name, block_size) {
                Some(capacity) => Ok(capacity),
                None => missing_hash_attribute(vm, "HASHXOF", "_capacity_bits"),
            }
        }

        #[pygetset]
        fn _rate_bits(&self, vm: &VirtualMachine) -> PyResult<usize> {
            let block_size = hasher_block_size(&self.name);
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
            data.with_ref(|bytes| self.ctx.update(bytes));
        }

        #[pymethod]
        fn digest(&self, args: XofDigestArgs, vm: &VirtualMachine) -> PyResult<PyBytes> {
            Ok(self.ctx.digest(args.length(vm)?).into())
        }

        #[pymethod]
        fn hexdigest(&self, args: XofDigestArgs, vm: &VirtualMachine) -> PyResult<String> {
            Ok(hex::encode(self.ctx.digest(args.length(vm)?)))
        }

        #[pymethod]
        fn copy(&self) -> Self {
            Self::new(&self.name, self.ctx.copy())
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

    fn new_fixed_hasher(name: &'static str, data: OptionalArg<ArgBytesLike>) -> PyHasher {
        PyHasher::new(
            name,
            hash_ctx_from_data(name, data).expect("canonical digest name"),
            backend::digest_output_size(name).unwrap_or(0),
        )
    }

    fn new_xof_hasher(name: &'static str, data: OptionalArg<ArgBytesLike>) -> PyHasherXof {
        PyHasherXof::new(
            name,
            hash_ctx_from_data(name, data).expect("canonical digest name"),
        )
    }

    #[pyfunction(name = "new")]
    fn hashlib_new(args: NewHashArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let data = resolve_data(args.data, args.string, vm)?;
        match args.name.as_str().to_lowercase().as_str() {
            "md5" => Ok(new_fixed_hasher("md5", data).into_pyobject(vm)),
            "sha1" => Ok(new_fixed_hasher("sha1", data).into_pyobject(vm)),
            "sha224" => Ok(new_fixed_hasher("sha224", data).into_pyobject(vm)),
            "sha256" => Ok(new_fixed_hasher("sha256", data).into_pyobject(vm)),
            "sha384" => Ok(new_fixed_hasher("sha384", data).into_pyobject(vm)),
            "sha512" => Ok(new_fixed_hasher("sha512", data).into_pyobject(vm)),
            "sha3_224" => Ok(new_fixed_hasher("sha3_224", data).into_pyobject(vm)),
            "sha3_256" => Ok(new_fixed_hasher("sha3_256", data).into_pyobject(vm)),
            "sha3_384" => Ok(new_fixed_hasher("sha3_384", data).into_pyobject(vm)),
            "sha3_512" => Ok(new_fixed_hasher("sha3_512", data).into_pyobject(vm)),
            "shake_128" => Ok(new_xof_hasher("shake_128", data).into_pyobject(vm)),
            "shake_256" => Ok(new_xof_hasher("shake_256", data).into_pyobject(vm)),
            "blake2b" => Ok(new_fixed_hasher("blake2b", data).into_pyobject(vm)),
            "blake2s" => Ok(new_fixed_hasher("blake2s", data).into_pyobject(vm)),
            other => Err(vm.new_value_error(format!("Unknown hashing algorithm: {other}"))),
        }
    }

    #[pyfunction(name = "openssl_md5")]
    pub(crate) fn local_md5(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(new_fixed_hasher("md5", data))
    }

    #[pyfunction(name = "openssl_sha1")]
    pub(crate) fn local_sha1(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(new_fixed_hasher("sha1", data))
    }

    #[pyfunction(name = "openssl_sha224")]
    pub(crate) fn local_sha224(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(new_fixed_hasher("sha224", data))
    }

    #[pyfunction(name = "openssl_sha256")]
    pub(crate) fn local_sha256(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(new_fixed_hasher("sha256", data))
    }

    #[pyfunction(name = "openssl_sha384")]
    pub(crate) fn local_sha384(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(new_fixed_hasher("sha384", data))
    }

    #[pyfunction(name = "openssl_sha512")]
    pub(crate) fn local_sha512(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(new_fixed_hasher("sha512", data))
    }

    #[pyfunction(name = "openssl_sha3_224")]
    pub(crate) fn local_sha3_224(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(new_fixed_hasher("sha3_224", data))
    }

    #[pyfunction(name = "openssl_sha3_256")]
    pub(crate) fn local_sha3_256(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(new_fixed_hasher("sha3_256", data))
    }

    #[pyfunction(name = "openssl_sha3_384")]
    pub(crate) fn local_sha3_384(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(new_fixed_hasher("sha3_384", data))
    }

    #[pyfunction(name = "openssl_sha3_512")]
    pub(crate) fn local_sha3_512(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(new_fixed_hasher("sha3_512", data))
    }

    #[pyfunction(name = "openssl_shake_128")]
    pub(crate) fn local_shake_128(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasherXof> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(new_xof_hasher("shake_128", data))
    }

    #[pyfunction(name = "openssl_shake_256")]
    pub(crate) fn local_shake_256(args: HashArgs, vm: &VirtualMachine) -> PyResult<PyHasherXof> {
        let data = resolve_data(args.data, args.string, vm)?;
        Ok(new_xof_hasher("shake_256", data))
    }

    fn parse_unsigned_int(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<u64> {
        let value = obj.try_index(vm)?;
        if value.as_bigint().sign() == malachite_bigint::Sign::Minus {
            return Err(vm.new_value_error("Cannot convert negative int"));
        }
        value.try_to_primitive(vm)
    }

    struct Blake2Limits {
        name: &'static str,
        display: &'static str,
        default_digest_size: i64,
        max_digest_size: usize,
        max_key_size: usize,
        max_salt_size: usize,
        max_person_size: usize,
        max_node_offset: u64,
    }

    pub(crate) struct Blake2Hash {
        name: &'static str,
        digest_size: usize,
        ctx: HashCtx,
    }

    impl Blake2Hash {
        pub(crate) fn name(&self) -> &'static str {
            self.name
        }

        pub(crate) fn digest_size(&self) -> usize {
            self.digest_size
        }

        pub(crate) fn block_size(&self) -> usize {
            hasher_block_size(self.name)
        }

        pub(crate) fn update(&self, data: &[u8]) {
            self.ctx.update(data);
        }

        pub(crate) fn digest(&self) -> Vec<u8> {
            self.ctx.digest(self.digest_size)
        }

        pub(crate) fn hexdigest(&self) -> String {
            hex::encode(self.digest())
        }

        pub(crate) fn copy(&self) -> Self {
            Self {
                name: self.name,
                digest_size: self.digest_size,
                ctx: self.ctx.copy(),
            }
        }

        fn into_hasher(self) -> PyHasher {
            PyHasher::new(self.name, self.ctx, self.digest_size)
        }
    }

    fn blake2_hasher(
        limits: Blake2Limits,
        args: BlakeHashArgs,
        vm: &VirtualMachine,
    ) -> PyResult<Blake2Hash> {
        let Blake2Limits {
            name,
            display,
            default_digest_size,
            max_digest_size,
            max_key_size,
            max_salt_size,
            max_person_size,
            max_node_offset,
        } = limits;
        let data = resolve_data(args.data, args.string, vm)?;
        let digest_size = args.digest_size.map_or(default_digest_size, |v| v.value);
        if digest_size < 1 || digest_size as u64 > max_digest_size as u64 {
            return Err(vm.new_value_error(format!(
                "digest_size for {display} must be between 1 and {max_digest_size} bytes, here it is {digest_size}"
            )));
        }
        let digest_size = digest_size as usize;

        let empty: &[u8] = &[];
        let key_buf;
        let salt_buf;
        let person_buf;
        let key = match args.key {
            OptionalArg::Present(ref buf) => {
                key_buf = buf.borrow_buf();
                &*key_buf
            }
            OptionalArg::Missing => empty,
        };
        let salt = match args.salt {
            OptionalArg::Present(ref buf) => {
                salt_buf = buf.borrow_buf();
                &*salt_buf
            }
            OptionalArg::Missing => empty,
        };
        let person = match args.person {
            OptionalArg::Present(ref buf) => {
                person_buf = buf.borrow_buf();
                &*person_buf
            }
            OptionalArg::Missing => empty,
        };

        if salt.len() > max_salt_size {
            return Err(vm.new_value_error(format!("maximum salt length is {max_salt_size} bytes")));
        }
        if person.len() > max_person_size {
            return Err(
                vm.new_value_error(format!("maximum person length is {max_person_size} bytes"))
            );
        }

        let fanout = args.fanout.map_or(1, |v| v.value);
        if !(0..=255).contains(&fanout) {
            return Err(vm.new_value_error("fanout must be between 0 and 255"));
        }
        let depth = args.depth.map_or(1, |v| v.value);
        if !(1..=255).contains(&depth) {
            return Err(vm.new_value_error("depth must be between 1 and 255"));
        }

        let leaf_size = match args.leaf_size.into_option() {
            Some(obj) => {
                let value = parse_unsigned_int(obj, vm)?;
                u32::try_from(value).map_err(|_| vm.new_overflow_error("leaf_size is too large"))?
            }
            None => 0,
        };
        let node_offset = match args.node_offset.into_option() {
            Some(obj) => {
                let value = parse_unsigned_int(obj, vm)?;
                if value > max_node_offset {
                    return Err(vm.new_overflow_error("node_offset is too large"));
                }
                value
            }
            None => 0,
        };

        let node_depth = args.node_depth.map_or(0, |v| v.value);
        if !(0..=255).contains(&node_depth) {
            return Err(vm.new_value_error("node_depth must be between 0 and 255"));
        }
        let inner_size = args.inner_size.map_or(0, |v| v.value);
        if inner_size < 0 || inner_size as u64 > max_digest_size as u64 {
            return Err(vm.new_value_error(format!(
                "inner_size must be between 0 and is {max_digest_size}"
            )));
        }
        if key.len() > max_key_size {
            return Err(vm.new_value_error(format!("maximum key length is {max_key_size} bytes")));
        }

        let ctx = HashCtx::new_blake2(
            name,
            digest_size,
            key,
            salt,
            person,
            fanout as u8,
            depth as u8,
            leaf_size,
            node_offset,
            node_depth as u8,
            inner_size as usize,
            args.last_node,
        )
        .ok_or_else(|| vm.new_value_error(format!("failed to initialize {name}")))?;
        if let OptionalArg::Present(d) = data {
            d.with_ref(|bytes| ctx.update(bytes));
        }
        Ok(Blake2Hash {
            name,
            digest_size,
            ctx,
        })
    }

    pub(crate) fn local_blake2b(args: BlakeHashArgs, vm: &VirtualMachine) -> PyResult<Blake2Hash> {
        blake2_hasher(
            Blake2Limits {
                name: "blake2b",
                display: "Blake2b",
                default_digest_size: 64,
                max_digest_size: 64,
                max_key_size: 64,
                max_salt_size: 16,
                max_person_size: 16,
                max_node_offset: u64::MAX,
            },
            args,
            vm,
        )
    }

    pub(crate) fn local_blake2s(args: BlakeHashArgs, vm: &VirtualMachine) -> PyResult<Blake2Hash> {
        blake2_hasher(
            Blake2Limits {
                name: "blake2s",
                display: "Blake2s",
                default_digest_size: 32,
                max_digest_size: 32,
                max_key_size: 32,
                max_salt_size: 8,
                max_person_size: 8,
                max_node_offset: (1 << 48) - 1,
            },
            args,
            vm,
        )
    }

    #[pyfunction(name = "openssl_blake2b")]
    fn openssl_blake2b(args: BlakeHashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        Ok(local_blake2b(args, vm)?.into_hasher())
    }

    #[pyfunction(name = "openssl_blake2s")]
    fn openssl_blake2s(args: BlakeHashArgs, vm: &VirtualMachine) -> PyResult<PyHasher> {
        Ok(local_blake2s(args, vm)?.into_hasher())
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
    ) -> PyResult<bool> {
        use constant_time_eq::constant_time_eq;

        match (&a, &b) {
            (ArgStrOrBytesLike::Str(a), ArgStrOrBytesLike::Str(b)) => {
                if !a.isascii() || !b.isascii() {
                    return Err(vm.new_type_error(
                        "comparing strings with non-ASCII characters is not supported",
                    ));
                }
                Ok(constant_time_eq(a.as_bytes(), b.as_bytes()))
            }
            (ArgStrOrBytesLike::Buf(a), ArgStrOrBytesLike::Buf(b)) => {
                Ok(a.with_ref(|a| b.with_ref(|b| constant_time_eq(a, b))))
            }
            _ => Err(vm.new_type_error(format!(
                "a bytes-like object is required, not '{}'",
                b.as_object().class().name()
            ))),
        }
    }

    #[derive(FromArgs, Debug)]
    #[allow(unused)]
    pub(crate) struct NewHMACHashArgs {
        #[pyarg(positional)]
        key: ArgBytesLike,
        #[pyarg(any, optional)]
        msg: OptionalArg<Option<ArgBytesLike>>,
        #[pyarg(named, optional)]
        digestmod: OptionalArg<PyObjectRef>,
    }

    fn new_hmac(
        name: String,
        key: &[u8],
        msg: Option<&ArgBytesLike>,
        vm: &VirtualMachine,
    ) -> PyResult<PyHmac> {
        let digest_size =
            backend::digest_output_size(&name).ok_or_else(|| unsupported_hash(&name, vm))?;
        let block_size =
            backend::digest_block_size(&name).ok_or_else(|| unsupported_hash(&name, vm))?;
        let ctx = HmacCtx::new(&name, key).ok_or_else(|| unsupported_hash(&name, vm))?;
        if let Some(m) = msg {
            m.with_ref(|bytes| ctx.update(bytes));
        }
        Ok(PyHmac {
            algo_name: name,
            digest_size,
            block_size,
            ctx,
        })
    }

    #[pyfunction]
    fn hmac_new(args: NewHMACHashArgs, vm: &VirtualMachine) -> PyResult<PyHmac> {
        let digestmod = args
            .digestmod
            .into_option()
            .ok_or_else(|| vm.new_type_error("Missing required parameter 'digestmod'."))?;
        let name = resolve_digestmod(&digestmod, vm)?;
        let key_buf = args.key.borrow_buf();
        let msg_data = args.msg.flatten();
        new_hmac(name, &key_buf, msg_data.as_ref(), vm)
    }

    #[pyfunction]
    fn hmac_digest(args: HmacDigestArgs, vm: &VirtualMachine) -> PyResult<PyBytes> {
        let name = resolve_digestmod(&args.digest, vm)?;
        let key_buf = args.key.borrow_buf();
        let msg_buf = args.msg.borrow_buf();
        let ctx = HmacCtx::new(&name, &key_buf).ok_or_else(|| unsupported_hash(&name, vm))?;
        ctx.update(&msg_buf);
        Ok(ctx.digest().into())
    }

    #[pyfunction]
    fn pbkdf2_hmac(args: Pbkdf2HmacArgs, vm: &VirtualMachine) -> PyResult<PyBytes> {
        let name = args.hash_name.as_str().to_lowercase();

        if args.iterations < 1 {
            return Err(vm.new_value_error("iteration value must be greater than 0."));
        }
        let rounds = usize::try_from(args.iterations)
            .map_err(|_| vm.new_overflow_error("iteration value is too great."))?;

        let dklen: usize = match args.dklen.into_option() {
            Some(obj) if vm.is_none(&obj) => {
                backend::digest_output_size(&name).ok_or_else(|| unsupported_hash(&name, vm))?
            }
            Some(obj) => {
                let len: i64 = obj.try_into_value(vm)?;
                if len < 1 {
                    return Err(vm.new_value_error("key length must be greater than 0."));
                }
                i32::try_from(len).map_err(|_| vm.new_overflow_error("key length is too great."))?
                    as usize
            }
            None => {
                backend::digest_output_size(&name).ok_or_else(|| unsupported_hash(&name, vm))?
            }
        };

        let password_buf = args.password.borrow_buf();
        let salt_buf = args.salt.borrow_buf();
        let derived = backend::compute_pbkdf2_hmac(&name, &password_buf, &salt_buf, rounds, dklen)
            .ok_or_else(|| unsupported_hash(&name, vm))?;
        Ok(derived.into())
    }

    #[derive(FromArgs)]
    struct ScryptArgs {
        #[pyarg(positional)]
        password: ArgBytesLike,
        #[pyarg(named)]
        salt: ArgBytesLike,
        #[pyarg(named)]
        n: ArgPrimitiveIndex<i64>,
        #[pyarg(named)]
        r: ArgPrimitiveIndex<i64>,
        #[pyarg(named)]
        p: ArgPrimitiveIndex<i64>,
        #[pyarg(named, default = 0)]
        maxmem: i64,
        #[pyarg(named, default = 64)]
        dklen: i64,
    }

    #[pyfunction]
    fn scrypt(args: ScryptArgs, vm: &VirtualMachine) -> PyResult<PyBytes> {
        const INT_MAX: i64 = i32::MAX as i64;
        const OPENSSL_DEFAULT_SCRYPT_MAXMEM: usize = 32 * 1024 * 1024;

        let password = args.password.borrow_buf();
        let salt = args.salt.borrow_buf();
        if password.len() > i32::MAX as usize {
            return Err(vm.new_overflow_error("password is too long."));
        }
        if salt.len() > i32::MAX as usize {
            return Err(vm.new_overflow_error("salt is too long."));
        }

        let n = u64::try_from(args.n.value).unwrap_or(0);
        if n < 2 || !n.is_power_of_two() {
            return Err(vm.new_value_error("n must be a power of 2."));
        }
        let log_n = u8::try_from(n.trailing_zeros()).map_err(|_| {
            vm.new_value_error("Invalid parameter combination for n, r, p, maxmem.")
        })?;

        let r = u32::try_from(args.r.value)
            .ok()
            .filter(|&value| value > 0)
            .ok_or_else(|| {
                vm.new_value_error("Invalid parameter combination for n, r, p, maxmem.")
            })?;
        let p = u32::try_from(args.p.value)
            .ok()
            .filter(|&value| value > 0)
            .ok_or_else(|| {
                vm.new_value_error("Invalid parameter combination for n, r, p, maxmem.")
            })?;

        let maxmem = args.maxmem;
        if !(0..=INT_MAX).contains(&maxmem) {
            return Err(vm.new_value_error(format!(
                "maxmem must be positive and smaller than {INT_MAX}"
            )));
        }
        let dklen = args.dklen;
        if !(1..=INT_MAX).contains(&dklen) {
            return Err(vm.new_value_error(format!(
                "dklen must be greater than 0 and smaller than {INT_MAX}"
            )));
        }
        let dklen = dklen as usize;

        let memory = usize::try_from(n)
            .ok()
            .and_then(|n| n.checked_mul(r as usize))
            .and_then(|v| v.checked_mul(128))
            .and_then(|v| v.checked_add((p as usize).checked_mul(r as usize)?.checked_mul(128)?))
            .and_then(|v| v.checked_add((r as usize).checked_mul(256)?))
            .ok_or_else(|| {
                vm.new_value_error("Invalid parameter combination for n, r, p, maxmem.")
            })?;
        let effective_maxmem = if maxmem == 0 {
            OPENSSL_DEFAULT_SCRYPT_MAXMEM
        } else {
            maxmem as usize
        };
        if memory > effective_maxmem {
            return Err(vm.new_value_error("[digital envelope routines] memory limit exceeded"));
        }

        let derived =
            backend::compute_scrypt(&password, &salt, log_n, r, p, dklen).ok_or_else(|| {
                vm.new_value_error("Invalid parameter combination for n, r, p, maxmem.")
            })?;
        Ok(derived.into())
    }
}
