/*! Python `super` class.

See also [CPython source code.](https://github.com/python/cpython/blob/50b48572d9a90c5bb36e2bef6179548ea927a35a/Objects/typeobject.c#L7663)
*/

use super::{PyStrRef, PyType, PyTypeRef};
use crate::{
    class::PyClassImpl,
    function::{IntoFuncArgs, OptionalArg},
    types::{Callable, Constructor, GetAttr, GetDescriptor},
    AsObject, Context, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
};

#[pyclass(module = false, name = "super")]
#[derive(Debug)]
pub struct PySuper {
    typ: PyTypeRef,
    obj: Option<(PyObjectRef, PyTypeRef)>,
}

impl PyPayload for PySuper {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.super_type
    }
}

#[derive(FromArgs)]
pub struct PySuperNewArgs {
    #[pyarg(positional, optional)]
    py_type: OptionalArg<PyTypeRef>,
    #[pyarg(positional, optional)]
    py_obj: OptionalArg<PyObjectRef>,
}

impl Constructor for PySuper {
    type Args = PySuperNewArgs;

    fn py_new(
        cls: PyTypeRef,
        Self::Args { py_type, py_obj }: Self::Args,
        vm: &VirtualMachine,
    ) -> PyResult {
        // Get the type:
        let (typ, obj) = if let OptionalArg::Present(ty) = py_type {
            (ty, py_obj.unwrap_or_none(vm))
        } else {
            let frame = vm
                .current_frame()
                .ok_or_else(|| vm.new_runtime_error("super(): no current frame".to_owned()))?;

            if frame.code.arg_count == 0 {
                return Err(vm.new_runtime_error("super(): no arguments".to_owned()));
            }
            let obj = frame.fastlocals.lock()[0]
                .clone()
                .or_else(|| {
                    if let Some(cell2arg) = frame.code.cell2arg.as_deref() {
                        cell2arg[..frame.code.cellvars.len()]
                            .iter()
                            .enumerate()
                            .find(|(_, arg_idx)| **arg_idx == 0)
                            .and_then(|(cell_idx, _)| frame.cells_frees[cell_idx].get())
                    } else {
                        None
                    }
                })
                .ok_or_else(|| vm.new_runtime_error("super(): arg[0] deleted".to_owned()))?;

            let mut typ = None;
            for (i, var) in frame.code.freevars.iter().enumerate() {
                if var.as_str() == "__class__" {
                    let i = frame.code.cellvars.len() + i;
                    let class = frame.cells_frees[i].get().ok_or_else(|| {
                        vm.new_runtime_error("super(): empty __class__ cell".to_owned())
                    })?;
                    typ = Some(class.downcast().map_err(|o| {
                        vm.new_type_error(format!(
                            "super(): __class__ is not a type ({})",
                            o.class().name()
                        ))
                    })?);
                    break;
                }
            }
            let typ = typ.ok_or_else(|| {
                vm.new_type_error(
                    "super must be called with 1 argument or from inside class method".to_owned(),
                )
            })?;

            (typ, obj)
        };

        PySuper::new(typ, obj, vm)?
            .into_ref_with_type(vm, cls)
            .map(Into::into)
    }
}

#[pyclass(with(GetAttr, GetDescriptor, Constructor))]
impl PySuper {
    fn new(typ: PyTypeRef, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<Self> {
        let obj = if vm.is_none(&obj) {
            None
        } else {
            let obj_type = supercheck(typ.clone(), obj.clone(), vm)?;
            Some((obj, obj_type))
        };
        Ok(Self { typ, obj })
    }

    #[pygetset(magic)]
    fn thisclass(&self) -> PyTypeRef {
        self.typ.clone()
    }

    #[pygetset(magic)]
    fn self_class(&self) -> Option<PyTypeRef> {
        Some(self.obj.as_ref()?.1.clone())
    }

    #[pygetset]
    fn __self__(&self) -> Option<PyObjectRef> {
        Some(self.obj.as_ref()?.0.clone())
    }

    #[pymethod(magic)]
    fn repr(&self) -> String {
        let typname = &self.typ.name();
        match self.obj {
            Some((_, ref ty)) => format!("<super: <class '{}'>, <{} object>>", typname, ty.name()),
            None => format!("<super: <class '{typname}'>, NULL>"),
        }
    }
}

impl GetAttr for PySuper {
    fn getattro(zelf: &Py<Self>, name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let skip = |zelf: &Py<Self>, name| zelf.as_object().generic_getattr(name, vm);
        let (obj, start_type): (PyObjectRef, PyTypeRef) = match zelf.obj.clone() {
            Some(o) => o,
            None => return skip(zelf, name),
        };
        // We want __class__ to return the class of the super object
        // (i.e. super, or a subclass), not the class of su->obj.

        if name.as_str() == "__class__" {
            return skip(zelf, name);
        }

        if let Some(name) = vm.ctx.interned_str(&*name) {
            // skip the classes in start_type.mro up to and including zelf.typ
            let mro: Vec<_> = start_type
                .iter_mro()
                .skip_while(|cls| !cls.is(&zelf.typ))
                .skip(1) // skip su->type (if any)
                .collect();
            for cls in mro {
                if let Some(descr) = cls.get_direct_attr(name) {
                    return vm
                        .call_get_descriptor_specific(
                            descr.clone(),
                            // Only pass 'obj' param if this is instance-mode super (See https://bugs.python.org/issue743267)
                            if obj.is(&start_type) { None } else { Some(obj) },
                            Some(start_type.as_object().to_owned()),
                        )
                        .unwrap_or(Ok(descr));
                }
            }
        }
        skip(zelf, name)
    }
}

impl GetDescriptor for PySuper {
    fn descr_get(
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        _cls: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let (zelf, obj) = Self::_unwrap(zelf, obj, vm)?;
        if vm.is_none(&obj) || zelf.obj.is_some() {
            return Ok(zelf.into());
        }
        let zelf_class = zelf.as_object().class();
        if zelf_class.is(vm.ctx.types.super_type) {
            Ok(PySuper::new(zelf.typ.clone(), obj, vm)?.into_pyobject(vm))
        } else {
            let obj = vm.unwrap_or_none(zelf.obj.clone().map(|(o, _)| o));
            PyType::call(zelf.class(), (zelf.typ.clone(), obj).into_args(vm), vm)
        }
    }
}

fn supercheck(ty: PyTypeRef, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyTypeRef> {
    if let Ok(cls) = obj.clone().downcast::<PyType>() {
        if cls.fast_issubclass(&ty) {
            return Ok(cls);
        }
    }
    if obj.fast_isinstance(&ty) {
        return Ok(obj.class().to_owned());
    }
    let class_attr = obj.get_attr("__class__", vm)?;
    if let Ok(cls) = class_attr.downcast::<PyType>() {
        if !cls.is(&ty) && cls.fast_issubclass(&ty) {
            return Ok(cls);
        }
    }
    Err(vm
        .new_type_error("super(type, obj): obj must be an instance or subtype of type".to_owned()))
}

pub fn init(context: &Context) {
    let super_type = &context.types.super_type;
    PySuper::extend_class(context, super_type);

    let super_doc = "super() -> same as super(__class__, <first argument>)\n\
                     super(type) -> unbound super object\n\
                     super(type, obj) -> bound super object; requires isinstance(obj, type)\n\
                     super(type, type2) -> bound super object; requires issubclass(type2, type)\n\
                     Typical use to call a cooperative superclass method:\n\
                     class C(B):\n    \
                     def meth(self, arg):\n        \
                     super().meth(arg)\n\
                     This works for class methods too:\n\
                     class C(B):\n    \
                     @classmethod\n    \
                     def cmeth(cls, arg):\n        \
                     super().cmeth(arg)\n";

    extend_class!(context, super_type, {
        "__doc__" => context.new_str(super_doc),
    });
}
