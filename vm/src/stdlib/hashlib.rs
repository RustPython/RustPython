use crate::common::cell::{PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard};
use crate::function::{OptionalArg, PyFuncArgs};
use crate::obj::objbytes::{PyBytes, PyBytesRef};
use crate::obj::objstr::PyStringRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{BorrowValue, PyClassImpl, PyObjectRef, PyResult, PyValue};
use crate::vm::VirtualMachine;
use std::fmt;

use blake2::{Blake2b, Blake2s};
use digest::DynDigest;
use md5::Md5;
use sha1::Sha1;
use sha2::{Sha224, Sha256, Sha384, Sha512};
use sha3::{Sha3_224, Sha3_256, Sha3_384, Sha3_512}; // TODO: , Shake128, Shake256};

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
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("hashlib", "hasher")
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
    fn tp_new(_cls: PyClassRef, _args: PyFuncArgs, vm: &VirtualMachine) -> PyResult {
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

fn hashlib_new(
    name: PyStringRef,
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

fn md5(data: OptionalArg<PyBytesRef>) -> PyResult<PyHasher> {
    init(PyHasher::new("md5", HashWrapper::md5()), data)
}

fn sha1(data: OptionalArg<PyBytesRef>) -> PyResult<PyHasher> {
    init(PyHasher::new("sha1", HashWrapper::sha1()), data)
}

fn sha224(data: OptionalArg<PyBytesRef>) -> PyResult<PyHasher> {
    init(PyHasher::new("sha224", HashWrapper::sha224()), data)
}

fn sha256(data: OptionalArg<PyBytesRef>) -> PyResult<PyHasher> {
    init(PyHasher::new("sha256", HashWrapper::sha256()), data)
}

fn sha384(data: OptionalArg<PyBytesRef>) -> PyResult<PyHasher> {
    init(PyHasher::new("sha384", HashWrapper::sha384()), data)
}

fn sha512(data: OptionalArg<PyBytesRef>) -> PyResult<PyHasher> {
    init(PyHasher::new("sha512", HashWrapper::sha512()), data)
}

fn sha3_224(data: OptionalArg<PyBytesRef>) -> PyResult<PyHasher> {
    init(PyHasher::new("sha3_224", HashWrapper::sha3_224()), data)
}

fn sha3_256(data: OptionalArg<PyBytesRef>) -> PyResult<PyHasher> {
    init(PyHasher::new("sha3_256", HashWrapper::sha3_256()), data)
}

fn sha3_384(data: OptionalArg<PyBytesRef>) -> PyResult<PyHasher> {
    init(PyHasher::new("sha3_384", HashWrapper::sha3_384()), data)
}

fn sha3_512(data: OptionalArg<PyBytesRef>) -> PyResult<PyHasher> {
    init(PyHasher::new("sha3_512", HashWrapper::sha3_512()), data)
}

fn shake128(_data: OptionalArg<PyBytesRef>, vm: &VirtualMachine) -> PyResult<PyHasher> {
    Err(vm.new_not_implemented_error("shake256".to_owned()))
}

fn shake256(_data: OptionalArg<PyBytesRef>, vm: &VirtualMachine) -> PyResult<PyHasher> {
    Err(vm.new_not_implemented_error("shake256".to_owned()))
}

fn blake2b(data: OptionalArg<PyBytesRef>) -> PyResult<PyHasher> {
    // TODO: handle parameters
    init(PyHasher::new("blake2b", HashWrapper::blake2b()), data)
}

fn blake2s(data: OptionalArg<PyBytesRef>) -> PyResult<PyHasher> {
    // TODO: handle parameters
    init(PyHasher::new("blake2s", HashWrapper::blake2s()), data)
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let hasher_type = PyHasher::make_class(ctx);

    py_module!(vm, "hashlib", {
        "new" => ctx.new_function(hashlib_new),
        "md5" => ctx.new_function(md5),
        "sha1" => ctx.new_function(sha1),
        "sha224" => ctx.new_function(sha224),
        "sha256" => ctx.new_function(sha256),
        "sha384" => ctx.new_function(sha384),
        "sha512" => ctx.new_function(sha512),
        "sha3_224" => ctx.new_function(sha3_224),
        "sha3_256" => ctx.new_function(sha3_256),
        "sha3_384" => ctx.new_function(sha3_384),
        "sha3_512" => ctx.new_function(sha3_512),
        "shake128" => ctx.new_function(shake128),
        "shake256" => ctx.new_function(shake256),
        "blake2b" => ctx.new_function(blake2b),
        "blake2s" => ctx.new_function(blake2s),
        "hasher" => hasher_type,
    })
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
        self.inner.input(data);
    }

    fn digest_size(&self) -> usize {
        self.inner.output_size()
    }

    fn get_digest(&self) -> Vec<u8> {
        let cloned = self.inner.box_clone();
        cloned.result().to_vec()
    }
}
