//! Implementation in line with the python `weakref` module.
//!
//! See also:
//! - [python weakref module](https://docs.python.org/3/library/weakref.html)
//! - [rust weak struct](https://doc.rust-lang.org/std/rc/struct.Weak.html)
//!
pub(crate) use _weakref::make_module;

#[pymodule]
mod _weakref {
    use crate::{builtins::PyTypeRef, PyObjectRef, VirtualMachine};

    #[pyattr(name = "ref")]
    fn ref_(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.types.weakref_type.clone()
    }
    #[pyattr]
    fn proxy(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.types.weakproxy_type.clone()
    }
    #[pyattr(name = "ReferenceType")]
    fn reference_type(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.types.weakref_type.clone()
    }
    #[pyattr(name = "ProxyType")]
    fn proxy_type(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.types.weakproxy_type.clone()
    }
    #[pyattr(name = "CallableProxyType")]
    fn callable_proxy_type(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.types.weakproxy_type.clone()
    }

    #[pyfunction]
    fn getweakrefcount(obj: PyObjectRef) -> usize {
        PyObjectRef::weak_count(&obj)
    }

    #[pyfunction]
    fn getweakrefs(_obj: PyObjectRef) -> Vec<PyObjectRef> {
        // TODO: implement this, may require a different gc
        Vec::new()
    }

    #[pyfunction]
    fn _remove_dead_weakref(_obj: PyObjectRef, _key: PyObjectRef) {
        // TODO
    }
}
