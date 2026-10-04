//! This module will be replaced once #3100 is done
//! Do not expose this type to outside of this crate

use super::VirtualMachine;
use crate::{
    builtins::{PyBaseObject, PyStr, PyStrInterned, descriptor::PyMethodDescriptor},
    function::{IntoFuncArgs, PyMethodFlags},
    object::{AsObject, Py, PyObject, PyObjectRef, PyResult},
    types::{GetattroFunc, PyTypeFlags, fn_addr},
};

#[derive(Debug)]
pub(crate) enum PyMethod {
    Function {
        target: PyObjectRef,
        func: PyObjectRef,
    },
    Attribute(PyObjectRef),
}

impl PyMethod {
    pub(crate) fn get(obj: PyObjectRef, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult<Self> {
        let cls = obj.class();
        let getattro = cls.slots.getattro.load().unwrap();
        if fn_addr(getattro) != fn_addr(PyBaseObject::getattro as GetattroFunc) {
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
                    // For classmethods, we need descr_get to convert instance to class
                    if let Some(method_descr) = descr.downcast_ref::<PyMethodDescriptor>()
                        && method_descr.method.flags.contains(PyMethodFlags::CLASS)
                    {
                        descr_cls.slots.descr_get.load()
                    } else {
                        is_method = true;
                        None
                    }
                } else {
                    let descr_get = descr_cls.slots.descr_get.load();
                    if let Some(descr_get) = descr_get
                        && descr_cls.slots.descr_set.load().is_some()
                    {
                        return descr_get(descr.as_object(), Some(&obj), Some(cls.as_object()), vm)
                            .map(Self::Attribute);
                    }
                    descr_get
                };
                Some((descr, descr_get))
            }
            None => None,
        };

        if let Some(dict) = obj.dict()
            && let Some(attr) = dict.get_item_opt(name, vm)?
        {
            return Ok(Self::Attribute(attr));
        }

        if let Some((attr, descr_get)) = cls_attr {
            match descr_get {
                None if is_method => Ok(Self::Function {
                    target: obj,
                    func: attr,
                }),
                Some(descr_get) => {
                    descr_get(attr.as_object(), Some(&obj), Some(cls.as_object()), vm)
                        .map(Self::Attribute)
                }
                None => Ok(Self::Attribute(attr)),
            }
        } else if let Some(getter) = cls.get_attr(identifier!(vm, __getattr__)) {
            getter.call((obj, name.to_owned()), vm).map(Self::Attribute)
        } else {
            Err(vm.new_no_attribute_error(obj.clone(), name.to_owned()))
        }
    }

    pub(crate) fn get_special<const DIRECT: bool>(
        obj: &PyObject,
        name: &'static PyStrInterned,
        vm: &VirtualMachine,
    ) -> PyResult<Option<Self>> {
        Self::get_special_ex::<DIRECT>(obj, name, vm, false)
    }

    /// lookup_method_ex.
    ///
    /// `raise_attribute_error` is CPython's same-named argument: when false
    /// (`lookup_maybe_method`), AttributeError from `tp_descr_get` is cleared
    /// and the lookup reports a miss. When true (`lookup_method`), that error
    /// is kept.
    pub(crate) fn get_special_ex<const DIRECT: bool>(
        obj: &PyObject,
        name: &'static PyStrInterned,
        vm: &VirtualMachine,
        raise_attribute_error: bool,
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
            let attr = match vm.call_get_descriptor_specific(
                &func,
                Some(obj),
                Some(obj_cls.as_object()),
            ) {
                Some(Ok(attr)) => attr,
                Some(Err(e))
                    if !raise_attribute_error
                        && e.fast_isinstance(vm.ctx.exceptions.attribute_error) =>
                {
                    return Ok(None);
                }
                Some(Err(e)) => return Err(e),
                None => func,
            };
            Self::Attribute(attr)
        };
        Ok(Some(meth))
    }

    pub(crate) fn invoke(self, args: impl IntoFuncArgs, vm: &VirtualMachine) -> PyResult {
        let (func, args) = match self {
            Self::Function { target, func } => (func, args.into_method_args(target, vm)),
            Self::Attribute(func) => (func, args.into_args(vm)),
        };
        func.call(args, vm)
    }

    #[allow(dead_code)]
    pub(crate) fn invoke_ref(&self, args: impl IntoFuncArgs, vm: &VirtualMachine) -> PyResult {
        let (func, args) = match self {
            Self::Function { target, func } => (func, args.into_method_args(target.clone(), vm)),
            Self::Attribute(func) => (func, args.into_args(vm)),
        };
        func.call(args, vm)
    }
}
