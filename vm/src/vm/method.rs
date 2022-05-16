//! This module will be replaced once #3100 is done
//! Do not expose this type to outside of this crate

use super::VirtualMachine;
use crate::{
    builtins::{PyBaseObject, PyStrRef},
    function::IntoFuncArgs,
    object::{AsObject, PyObjectRef, PyResult},
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
    pub fn get(obj: PyObjectRef, name: PyStrRef, vm: &VirtualMachine) -> PyResult<Self> {
        let cls = obj.class();
        let getattro = cls.mro_find_map(|cls| cls.slots.getattro.load()).unwrap();
        if getattro as usize != PyBaseObject::getattro as usize {
            drop(cls);
            return obj.get_attr(name, vm).map(Self::Attribute);
        }

        let mut is_method = false;

        let cls_attr = match cls.get_attr(name.as_str()) {
            Some(descr) => {
                let descr_cls = descr.class();
                let descr_get = if descr_cls.slots.flags.has_feature(PyTypeFlags::METHOD_DESCR) {
                    is_method = true;
                    None
                } else {
                    let descr_get = descr_cls.mro_find_map(|cls| cls.slots.descr_get.load());
                    if let Some(descr_get) = descr_get {
                        if descr_cls
                            .mro_find_map(|cls| cls.slots.descr_set.load())
                            .is_some()
                        {
                            drop(descr_cls);
                            let cls = cls.into_owned().into();
                            return descr_get(descr, Some(obj), Some(cls), vm).map(Self::Attribute);
                        }
                    }
                    descr_get
                };
                drop(descr_cls);
                Some((descr, descr_get))
            }
            None => None,
        };

        if let Some(dict) = obj.dict() {
            if let Some(attr) = dict.get_item_opt(&*name, vm)? {
                return Ok(Self::Attribute(attr));
            }
        }

        if let Some((attr, descr_get)) = cls_attr {
            match descr_get {
                None if is_method => {
                    drop(cls);
                    Ok(Self::Function {
                        target: obj,
                        func: attr,
                    })
                }
                Some(descr_get) => {
                    let cls = cls.into_owned().into();
                    descr_get(attr, Some(obj), Some(cls), vm).map(Self::Attribute)
                }
                None => Ok(Self::Attribute(attr)),
            }
        } else if let Some(getter) = cls.get_attr("__getattr__") {
            drop(cls);
            vm.invoke(&getter, (obj, name)).map(Self::Attribute)
        } else {
            let exc = vm.new_attribute_error(format!(
                "'{}' object has no attribute '{}'",
                cls.name(),
                name
            ));
            vm.set_attribute_error_context(&exc, obj.clone(), name);
            Err(exc)
        }
    }

    pub(crate) fn get_special(
        obj: PyObjectRef,
        name: &str,
        vm: &VirtualMachine,
    ) -> PyResult<Result<Self, PyObjectRef>> {
        let obj_cls = obj.class();
        let func = match obj_cls.get_attr(name) {
            Some(f) => f,
            None => {
                drop(obj_cls);
                return Ok(Err(obj));
            }
        };
        let meth = if func
            .class()
            .slots
            .flags
            .has_feature(PyTypeFlags::METHOD_DESCR)
        {
            drop(obj_cls);
            Self::Function { target: obj, func }
        } else {
            let obj_cls = obj_cls.into_owned().into();
            let attr = vm
                .call_get_descriptor_specific(func, Some(obj), Some(obj_cls))
                .unwrap_or_else(Ok)?;
            Self::Attribute(attr)
        };
        Ok(Ok(meth))
    }

    pub fn invoke(self, args: impl IntoFuncArgs, vm: &VirtualMachine) -> PyResult {
        let (func, args) = match self {
            PyMethod::Function { target, func } => (func, args.into_method_args(target, vm)),
            PyMethod::Attribute(func) => (func, args.into_args(vm)),
        };
        vm.invoke(&func, args)
    }

    #[allow(dead_code)]
    pub fn invoke_ref(&self, args: impl IntoFuncArgs, vm: &VirtualMachine) -> PyResult {
        let (func, args) = match self {
            PyMethod::Function { target, func } => {
                (func, args.into_method_args(target.clone(), vm))
            }
            PyMethod::Attribute(func) => (func, args.into_args(vm)),
        };
        vm.invoke(func, args)
    }
}
