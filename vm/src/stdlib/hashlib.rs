use crate::function::PyFuncArgs;
use crate::obj::objbytes::PyBytesRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyClassImpl, PyObjectRef, PyResult, PyValue};
use crate::vm::VirtualMachine;
use std::cell::RefCell;
use std::fmt;

use crypto;
use crypto::digest::Digest;

#[pyclass(name = "hasher")]
struct PyHasher {
    name: String,
    buffer: Box<RefCell<Digest>>,
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
    // fn new<D: 'static>(d: D) -> Self where D: Digest, D: Sized {
    fn new<D: 'static>(name: &str, d: D) -> Self
    where
        D: Digest,
        D: Sized,
    {
        /*
        let d = match name {
            "md5" => crypto::md5::Md5::new(),
            crypto::sha2::Sha256::new()
        };
        */

        PyHasher {
            name: name.to_string(),
            buffer: Box::new(RefCell::new(d)),
        }
    }

    #[pymethod(name = "__new__")]
    fn py_new(_cls: PyClassRef, _args: PyFuncArgs, vm: &VirtualMachine) -> PyResult {
        Ok(PyHasher::new("md5", crypto::md5::Md5::new())
            .into_ref(vm)
            .into_object())
    }

    #[pymethod(name = "update")]
    fn update(&self, data: PyBytesRef, vm: &VirtualMachine) -> PyResult {
        self.buffer.borrow_mut().input(data.get_value());
        Ok(vm.get_none())
    }

    #[pymethod(name = "hexdigest")]
    fn hexdigest(&self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.new_str(self.buffer.borrow_mut().result_str()))
    }
}

fn md5(_vm: &VirtualMachine) -> PyResult<PyHasher> {
    Ok(PyHasher::new("md5", crypto::md5::Md5::new()))
}

fn sha1(_vm: &VirtualMachine) -> PyResult<PyHasher> {
    Ok(PyHasher::new("sha1", crypto::sha1::Sha1::new()))
}

fn sha224(_vm: &VirtualMachine) -> PyResult<PyHasher> {
    Ok(PyHasher::new("224", crypto::sha2::Sha224::new()))
}

fn sha256(_vm: &VirtualMachine) -> PyResult<PyHasher> {
    Ok(PyHasher::new("sha256", crypto::sha2::Sha256::new()))
}

fn sha384(_vm: &VirtualMachine) -> PyResult<PyHasher> {
    Ok(PyHasher::new("sha384", crypto::sha2::Sha384::new()))
}

fn sha512(_vm: &VirtualMachine) -> PyResult<PyHasher> {
    Ok(PyHasher::new("sha512", crypto::sha2::Sha512::new()))
}

fn sha3_224(_vm: &VirtualMachine) -> PyResult<PyHasher> {
    Ok(PyHasher::new("sha3_224", crypto::sha3::Sha3::sha3_224()))
}

fn sha3_256(_vm: &VirtualMachine) -> PyResult<PyHasher> {
    Ok(PyHasher::new("sha3_256", crypto::sha3::Sha3::sha3_256()))
}

fn sha3_384(_vm: &VirtualMachine) -> PyResult<PyHasher> {
    Ok(PyHasher::new("sha3_384", crypto::sha3::Sha3::sha3_384()))
}

fn sha3_512(_vm: &VirtualMachine) -> PyResult<PyHasher> {
    Ok(PyHasher::new("sha3_512", crypto::sha3::Sha3::sha3_512()))
}

fn blake2b(_vm: &VirtualMachine) -> PyResult<PyHasher> {
    // TODO: handle parameters
    Ok(PyHasher::new("blake2b", crypto::blake2b::Blake2b::new(0)))
}

fn blake2s(_vm: &VirtualMachine) -> PyResult<PyHasher> {
    // TODO: handle parameters
    Ok(PyHasher::new("blake2s", crypto::blake2s::Blake2s::new(0)))
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let hasher_type = PyHasher::make_class(ctx);

    py_module!(vm, "hashlib", {
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
        "blake2b" => ctx.new_rustfunc(blake2b),
        "blake2s" => ctx.new_rustfunc(blake2s),
        "hasher" => hasher_type,
    })
}
