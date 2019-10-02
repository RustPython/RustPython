use crate::function::{OptionalArg, PyFuncArgs};
use crate::obj::objbytes::{PyBytes, PyBytesRef};
use crate::obj::objstr::PyStringRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyClassImpl, PyObjectRef, PyResult, PyValue};
use crate::vm::VirtualMachine;
use std::cell::RefCell;
use std::fmt;

use blake2::{Blake2b, Blake2s};
use digest::DynDigest;
use md5::Md5;
use sha1::Sha1;
use sha2::{Sha224, Sha256, Sha384, Sha512};
use sha3::{Sha3_224, Sha3_256, Sha3_384, Sha3_512}; // TODO: , Shake128, Shake256};

#[pyclass(name = "hasher")]
struct PyHasher {
    name: String,
    buffer: RefCell<HashWrapper>,
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
            name: name.to_string(),
            buffer: RefCell::new(d),
        }
    }

    #[pyslot(new)]
    fn tp_new(_cls: PyClassRef, _args: PyFuncArgs, vm: &VirtualMachine) -> PyResult {
        Ok(PyHasher::new("md5", HashWrapper::md5())
            .into_ref(vm)
            .into_object())
    }

    #[pyproperty(name = "name")]
    fn name(&self, _vm: &VirtualMachine) -> String {
        self.name.clone()
    }

    #[pyproperty(name = "digest_size")]
    fn digest_size(&self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_int(self.buffer.borrow().digest_size()))
    }

    #[pymethod(name = "update")]
    fn update(&self, data: PyBytesRef, vm: &VirtualMachine) -> PyResult {
        self.buffer.borrow_mut().input(data.get_value());
        Ok(vm.get_none())
    }

    #[pymethod(name = "digest")]
    fn digest(&self, _vm: &VirtualMachine) -> PyBytes {
        let result = self.get_digest();
        PyBytes::new(result)
    }

    #[pymethod(name = "hexdigest")]
    fn hexdigest(&self, _vm: &VirtualMachine) -> String {
        let result = self.get_digest();
        hex::encode(result)
    }

    fn get_digest(&self) -> Vec<u8> {
        self.buffer.borrow().get_digest()
    }
}

fn hashlib_new(
    name: PyStringRef,
    data: OptionalArg<PyBytesRef>,
    vm: &VirtualMachine,
) -> PyResult<PyHasher> {
    let hasher = match name.as_str() {
        "md5" => Ok(PyHasher::new("md5", HashWrapper::md5())),
        "sha1" => Ok(PyHasher::new("sha1", HashWrapper::sha1())),
        "sha224" => Ok(PyHasher::new("sha224", HashWrapper::sha224())),
        "sha256" => Ok(PyHasher::new("sha256", HashWrapper::sha256())),
        "sha384" => Ok(PyHasher::new("sha384", HashWrapper::sha384())),
        "sha512" => Ok(PyHasher::new("sha512", HashWrapper::sha512())),
        "sha3_224" => Ok(PyHasher::new("sha3_224", HashWrapper::sha3_224())),
        "sha3_256" => Ok(PyHasher::new("sha3_256", HashWrapper::sha3_256())),
        "sha3_384" => Ok(PyHasher::new("sha3_384", HashWrapper::sha3_384())),
        "sha3_512" => Ok(PyHasher::new("sha3_512", HashWrapper::sha3_512())),
        // TODO: "shake128" => Ok(PyHasher::new("shake128", HashWrapper::shake128())),
        // TODO: "shake256" => Ok(PyHasher::new("shake256", HashWrapper::shake256())),
        "blake2b" => Ok(PyHasher::new("blake2b", HashWrapper::blake2b())),
        "blake2s" => Ok(PyHasher::new("blake2s", HashWrapper::blake2s())),
        other => Err(vm.new_value_error(format!("Unknown hashing algorithm: {}", other))),
    }?;

    if let OptionalArg::Present(data) = data {
        hasher.update(data, vm)?;
    }

    Ok(hasher)
}

fn md5(_vm: &VirtualMachine) -> PyResult<PyHasher> {
    Ok(PyHasher::new("md5", HashWrapper::md5()))
}

fn sha1(_vm: &VirtualMachine) -> PyResult<PyHasher> {
    Ok(PyHasher::new("sha1", HashWrapper::sha1()))
}

fn sha224(_vm: &VirtualMachine) -> PyResult<PyHasher> {
    Ok(PyHasher::new("sha224", HashWrapper::sha224()))
}

fn sha256(_vm: &VirtualMachine) -> PyResult<PyHasher> {
    Ok(PyHasher::new("sha256", HashWrapper::sha256()))
}

fn sha384(_vm: &VirtualMachine) -> PyResult<PyHasher> {
    Ok(PyHasher::new("sha384", HashWrapper::sha384()))
}

fn sha512(_vm: &VirtualMachine) -> PyResult<PyHasher> {
    Ok(PyHasher::new("sha512", HashWrapper::sha512()))
}

fn sha3_224(_vm: &VirtualMachine) -> PyResult<PyHasher> {
    Ok(PyHasher::new("sha3_224", HashWrapper::sha3_224()))
}

fn sha3_256(_vm: &VirtualMachine) -> PyResult<PyHasher> {
    Ok(PyHasher::new("sha3_256", HashWrapper::sha3_256()))
}

fn sha3_384(_vm: &VirtualMachine) -> PyResult<PyHasher> {
    Ok(PyHasher::new("sha3_384", HashWrapper::sha3_384()))
}

fn sha3_512(_vm: &VirtualMachine) -> PyResult<PyHasher> {
    Ok(PyHasher::new("sha3_512", HashWrapper::sha3_512()))
}

fn shake128(vm: &VirtualMachine) -> PyResult<PyHasher> {
    Err(vm.new_not_implemented_error("shake256".to_string()))
    // Ok(PyHasher::new("shake128", HashWrapper::shake128()))
}

fn shake256(vm: &VirtualMachine) -> PyResult<PyHasher> {
    Err(vm.new_not_implemented_error("shake256".to_string()))
    // TODO: Ok(PyHasher::new("shake256", HashWrapper::shake256()))
}

fn blake2b(_vm: &VirtualMachine) -> PyResult<PyHasher> {
    // TODO: handle parameters
    Ok(PyHasher::new("blake2b", HashWrapper::blake2b()))
}

fn blake2s(_vm: &VirtualMachine) -> PyResult<PyHasher> {
    // TODO: handle parameters
    Ok(PyHasher::new("blake2s", HashWrapper::blake2s()))
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let hasher_type = PyHasher::make_class(ctx);

    py_module!(vm, "hashlib", {
        "new" => ctx.new_rustfunc(hashlib_new),
        "md5" => ctx.new_rustfunc(md5),
        "sha1" => ctx.new_rustfunc(sha1),
        "sha224" => ctx.new_rustfunc(sha224),
        "sha256" => ctx.new_rustfunc(sha256),
        "sha384" => ctx.new_rustfunc(sha384),
        "sha512" => ctx.new_rustfunc(sha512),
        "sha3_224" => ctx.new_rustfunc(sha3_224),
        "sha3_256" => ctx.new_rustfunc(sha3_256),
        "sha3_384" => ctx.new_rustfunc(sha3_384),
        "sha3_512" => ctx.new_rustfunc(sha3_512),
        "shake128" => ctx.new_rustfunc(shake128),
        "shake256" => ctx.new_rustfunc(shake256),
        "blake2b" => ctx.new_rustfunc(blake2b),
        "blake2s" => ctx.new_rustfunc(blake2s),
        "hasher" => hasher_type,
    })
}

/// Generic wrapper patching around the hashing libraries.
struct HashWrapper {
    inner: Box<dyn DynDigest>,
}

impl HashWrapper {
    fn new<D: 'static>(d: D) -> Self
    where
        D: DynDigest + Sized,
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
        let cloned = self.inner.clone();
        cloned.result().to_vec()
    }
}
