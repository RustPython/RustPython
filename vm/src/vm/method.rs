//! This module will be replaced once #3100 is done
//! Do not expose this type to outside of this crate

use super::VirtualMachine;
use crate::{
    builtins::{PyBaseObject, PyStr, PyStrInterned},
    function::IntoFuncArgs,
    object::{AsObject, Py, PyObject, PyObjectRef, PyResult},
    types::PyTypeFlags,
};

#[derive(Debug)]
pub enum PyMethod {
    Function {
        target: PyObjectRef,
        func: PyObjectRef,
    },
    Attribute(PyObjectRef),
}

impl PyMethod {
    pub fn get(obj: PyObjectRef, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult<Self> {
        let cls = obj.class();
        let getattro = cls.mro_find_map(|cls| cls.slots.getattro.load()).unwrap();
        if getattro as usize != PyBaseObject::getattro as usize {
            return obj.get_attr(name, vm).map(Self::Attribute);
        }

        // any correct method name is always interned already.
        let interned_name = vm.ctx.interned_str(name);
        let mut is_method = false;

        let cls_attr = match interned_name.and_then(|name| cls.get_attr(name)) {
            Some(descr) => {
                let descr_cls = descr.class();
                let descr_get = if descr_cls
                    .slots
                    .flags
                    .has_feature(PyTypeFlags::METHOD_DESCRIPTOR)
                {
                    is_method = true;
                    None
                } else {
                    let descr_get = descr_cls.mro_find_map(|cls| cls.slots.descr_get.load());
                    if let Some(descr_get) = descr_get {
                        if descr_cls
                            .mro_find_map(|cls| cls.slots.descr_set.load())
                            .is_some()
                        {
                            let cls = cls.to_owned().into();
                            return descr_get(descr, Some(obj), Some(cls), vm).map(Self::Attribute);
                        }
                    }
                    descr_get
                };
                Some((descr, descr_get))
            }
            None => None,
        };

        if let Some(dict) = obj.dict() {
            if let Some(attr) = dict.get_item_opt(name, vm)? {
                return Ok(Self::Attribute(attr));
            }
        }

        if let Some((attr, descr_get)) = cls_attr {
            match descr_get {
                None if is_method => Ok(Self::Function {
                    target: obj,
                    func: attr,
                }),
                Some(descr_get) => {
                    let cls = cls.to_owned().into();
                    descr_get(attr, Some(obj), Some(cls), vm).map(Self::Attribute)
                }
                None => Ok(Self::Attribute(attr)),
            }
        } else if let Some(getter) = cls.get_attr(identifier!(vm, __getattr__)) {
            getter.call((obj, name.to_owned()), vm).map(Self::Attribute)
        } else {
            let exc = vm.new_attribute_error(format!(
                "'{}' object has no attribute '{}'",
                cls.name(),
                name
            ));
            vm.set_attribute_error_context(&exc, obj.clone(), name.to_owned());
            Err(exc)
        }
    }

    pub(crate) fn get_special<const DIRECT: bool>(
        obj: &PyObject,
        name: &'static PyStrInterned,
        vm: &VirtualMachine,
    ) -> PyResult<Option<Self>> {
        let obj_cls = obj.class();
        let attr = if DIRECT {
            obj_cls.get_direct_attr(name)
        } else {
            obj_cls.get_attr(name)
        };
        let func = match attr {
            Some(f) => f,
            None => {
                return Ok(None);
            }
        };
        let meth = if func
            .class()
            .slots
            .flags
            .has_feature(PyTypeFlags::METHOD_DESCRIPTOR)
        {
            Self::Function {
                target: obj.to_owned(),
                func,
            }
        } else {
            let obj_cls = obj_cls.to_owned().into();
            let attr = vm
                .call_get_descriptor_specific(&func, Some(obj.to_owned()), Some(obj_cls))
                .unwrap_or(Ok(func))?;
            Self::Attribute(attr)
        };
        Ok(Some(meth))
    }

    pub fn invoke(self, args: impl IntoFuncArgs, vm: &VirtualMachine) -> PyResult {
        let (func, args) = match self {
            PyMethod::Function { target, func } => (func, args.into_method_args(target, vm)),
            PyMethod::Attribute(func) => (func, args.into_args(vm)),
        };
        func.call(args, vm)
    }

    #[allow(dead_code)]
    pub fn invoke_ref(&self, args: impl IntoFuncArgs, vm: &VirtualMachine) -> PyResult {
        let (func, args) = match self {
            PyMethod::Function { target, func } => {
                (func, args.into_method_args(target.clone(), vm))
            }
            PyMethod::Attribute(func) => (func, args.into_args(vm)),
        };
        func.call(args, vm)
    }
}
