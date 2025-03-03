//! Implementation in line with the python `weakref` module.
//!
//! See also:
//! - [python weakref module](https://docs.python.org/3/library/weakref.html)
//! - [rust weak struct](https://doc.rust-lang.org/std/rc/struct.Weak.html)
//!
pub(crate) use _weakref::make_module;

#[pymodule]
mod _weakref {
    use crate::{
        PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyDictRef, PyTypeRef, PyWeak},
    };

    #[pyattr(name = "ref")]
    fn ref_(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.types.weakref_type.to_owned()
    }
    #[pyattr]
    fn proxy(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.types.weakproxy_type.to_owned()
    }
    #[pyattr(name = "ReferenceType")]
    fn reference_type(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.types.weakref_type.to_owned()
    }
    #[pyattr(name = "ProxyType")]
    fn proxy_type(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.types.weakproxy_type.to_owned()
    }
    #[pyattr(name = "CallableProxyType")]
    fn callable_proxy_type(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.types.weakproxy_type.to_owned()
    }

    #[pyfunction]
    fn getweakrefcount(obj: PyObjectRef) -> usize {
        obj.weak_count().unwrap_or(0)
    }

    #[pyfunction]
    fn getweakrefs(obj: PyObjectRef) -> Vec<PyObjectRef> {
        match obj.get_weak_references() {
            Some(v) => v.into_iter().map(Into::into).collect(),
            None => vec![],
        }
    }

    #[pyfunction]
    fn _remove_dead_weakref(
        dict: PyDictRef,
        key: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        dict._as_dict_inner()
            .delete_if(vm, &*key, |wr| {
                let wr = wr
                    .payload::<PyWeak>()
                    .ok_or_else(|| vm.new_type_error("not a weakref".to_owned()))?;
                Ok(wr.is_dead())
            })
            .map(drop)
    }
}
