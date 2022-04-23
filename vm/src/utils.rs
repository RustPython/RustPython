use crate::{
    builtins::PyStr,
    convert::{ToPyException, ToPyObject},
    PyObjectRef, PyResult, VirtualMachine,
};

pub fn hash_iter<'a, I: IntoIterator<Item = &'a PyObjectRef>>(
    iter: I,
    vm: &VirtualMachine,
) -> PyResult<rustpython_common::hash::PyHash> {
    vm.state.hash_secret.hash_iter(iter, |obj| obj.hash(vm))
}

pub fn hash_iter_unordered<'a, I: IntoIterator<Item = &'a PyObjectRef>>(
    iter: I,
    vm: &VirtualMachine,
) -> PyResult<rustpython_common::hash::PyHash> {
    rustpython_common::hash::hash_iter_unordered(iter, |obj| obj.hash(vm))
}

impl ToPyObject for std::convert::Infallible {
    fn to_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        match self {}
    }
}

pub trait ToCString {
    fn to_cstring(&self, vm: &VirtualMachine) -> PyResult<std::ffi::CString>;
}

impl ToCString for &str {
    fn to_cstring(&self, vm: &VirtualMachine) -> PyResult<std::ffi::CString> {
        std::ffi::CString::new(*self).map_err(|err| err.to_pyexception(vm))
    }
}

impl ToCString for PyStr {
    fn to_cstring(&self, vm: &VirtualMachine) -> PyResult<std::ffi::CString> {
        std::ffi::CString::new(self.as_ref()).map_err(|err| err.to_pyexception(vm))
    }
}

pub(crate) fn collection_repr<'a, I>(
    class_name: Option<&str>,
    prefix: &str,
    suffix: &str,
    iter: I,
    vm: &VirtualMachine,
) -> PyResult<String>
where
    I: std::iter::Iterator<Item = &'a PyObjectRef>,
{
    let mut repr = String::new();
    if let Some(name) = class_name {
        repr.push_str(name);
        repr.push('(');
    }
    repr.push_str(prefix);
    {
        let mut parts_iter = iter.map(|o| o.repr(vm));
        repr.push_str(
            parts_iter
                .next()
                .transpose()?
                .expect("this is not called for empty collection")
                .as_str(),
        );
        for part in parts_iter {
            repr.push_str(", ");
            repr.push_str(part?.as_str());
        }
    }
    repr.push_str(suffix);
    if class_name.is_some() {
        repr.push(')');
    }

    Ok(repr)
}
