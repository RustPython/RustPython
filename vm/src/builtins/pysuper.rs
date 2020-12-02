/*! Python `super` class.

See also:

https://github.com/python/cpython/blob/50b48572d9a90c5bb36e2bef6179548ea927a35a/Objects/typeobject.c#L7663

*/

use super::pystr::PyStrRef;
use super::pytype::{PyType, PyTypeRef};
use crate::function::OptionalArg;
use crate::pyobject::{
    BorrowValue, IdProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue,
    TypeProtocol,
};
use crate::slots::{SlotDescriptor, SlotGetattro};
use crate::vm::VirtualMachine;

#[pyclass(module = false, name = "super")]
#[derive(Debug)]
pub struct PySuper {
    typ: PyTypeRef,
    obj: Option<(PyObjectRef, PyTypeRef)>,
}

impl PyValue for PySuper {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.super_type
    }
}

#[pyimpl(with(SlotGetattro, SlotDescriptor))]
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

    #[pymethod(name = "__repr__")]
    fn repr(&self) -> String {
        let typname = &self.typ.name;
        match self.obj {
            Some((_, ref ty)) => format!("<super: <class '{}'>, <{} object>>", typname, ty.name),
            None => format!("<super: <class '{}'>, NULL>", typname),
        }
    }

    #[pyslot]
    fn tp_new(
        cls: PyTypeRef,
        py_type: OptionalArg<PyTypeRef>,
        py_obj: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
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
                if var.borrow_value() == "__class__" {
                    let i = frame.code.cellvars.len() + i;
                    let class = frame.cells_frees[i].get().ok_or_else(|| {
                        vm.new_runtime_error("super(): empty __class__ cell".to_owned())
                    })?;
                    typ = Some(class.downcast().map_err(|o| {
                        vm.new_type_error(format!(
                            "super(): __class__ is not a type ({})",
                            o.class().name
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

        PySuper::new(typ, obj, vm)?.into_ref_with_type(vm, cls)
    }
}

impl SlotGetattro for PySuper {
    fn getattro(zelf: PyRef<Self>, name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let (inst, obj_type) = match zelf.obj.clone() {
            Some(o) => o,
            None => return vm.generic_getattribute(zelf.into_object(), name),
        };
        // skip the classes in obj_type.mro up to and including zelf.typ
        let it = obj_type
            .iter_mro()
            .skip_while(|cls| !cls.is(&zelf.typ))
            .skip(1);
        for cls in it {
            if let Some(descr) = cls.get_direct_attr(name.borrow_value()) {
                return vm
                    .call_get_descriptor_specific(
                        descr.clone(),
                        if inst.is(&obj_type) { None } else { Some(inst) },
                        Some(obj_type.clone().into_object()),
                    )
                    .unwrap_or(Ok(descr));
            }
        }
        vm.generic_getattribute(zelf.into_object(), name)
    }
}

impl SlotDescriptor for PySuper {
    fn descr_get(
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        _cls: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let (zelf, obj) = Self::_unwrap(zelf, obj, vm)?;
        if vm.is_none(&obj) || zelf.obj.is_some() {
            return Ok(zelf.into_object());
        }
        let zelf_class = zelf.as_object().class();
        if zelf_class.is(&vm.ctx.types.super_type) {
            Ok(PySuper::new(zelf.typ.clone(), obj, vm)?
                .into_ref(vm)
                .into_object())
        } else {
            let obj = vm.unwrap_or_none(zelf.obj.clone().map(|(o, _)| o));
            vm.invoke(
                zelf.as_object().clone_class().as_object(),
                (zelf.typ.clone(), obj),
            )
        }
    }
}

fn supercheck(ty: PyTypeRef, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyTypeRef> {
    if let Ok(cls) = obj.clone().downcast::<PyType>() {
        if cls.issubclass(&ty) {
            return Ok(cls);
        }
    }
    if obj.isinstance(&ty) {
        return Ok(obj.clone_class());
    }
    let class_attr = vm.get_attribute(obj, "__class__")?;
    if let Ok(cls) = class_attr.downcast::<PyType>() {
        if !cls.is(&ty) && cls.issubclass(&ty) {
            return Ok(cls);
        }
    }
    Err(vm
        .new_type_error("super(type, obj): obj must be an instance or subtype of type".to_owned()))
}

pub fn init(context: &PyContext) {
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
