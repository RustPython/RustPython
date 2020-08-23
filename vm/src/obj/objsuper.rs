/*! Python `super` class.

See also:

https://github.com/python/cpython/blob/50b48572d9a90c5bb36e2bef6179548ea927a35a/Objects/typeobject.c#L7663

*/

use super::objstr::PyStringRef;
use super::objtype::{self, PyClass, PyClassRef};
use crate::function::OptionalArg;
use crate::pyobject::{
    BorrowValue, IdProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue,
    TryFromObject, TypeProtocol,
};
use crate::scope::NameProtocol;
use crate::slots::SlotDescriptor;
use crate::vm::VirtualMachine;

use itertools::Itertools;

pub type PySuperRef = PyRef<PySuper>;

#[pyclass(module = false, name = "super")]
#[derive(Debug)]
pub struct PySuper {
    typ: PyClassRef,
    obj: Option<(PyObjectRef, PyClassRef)>,
}

impl PyValue for PySuper {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.super_type.clone()
    }
}

#[pyimpl(with(SlotDescriptor))]
impl PySuper {
    fn new(typ: PyClassRef, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<Self> {
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

    #[pymethod(name = "__getattribute__")]
    fn getattribute(zelf: PyRef<Self>, name: PyStringRef, vm: &VirtualMachine) -> PyResult {
        let (inst, obj_type) = match zelf.obj.clone() {
            Some(o) => o,
            None => return vm.generic_getattribute(zelf.into_object(), name),
        };
        // skip the classes in obj_type.mro up to and including zelf.typ
        let mut it = obj_type.iter_mro().peekable();
        for _ in it.peeking_take_while(|cls| !cls.is(&zelf.typ)) {}
        for cls in it.skip(1) {
            if let Some(descr) = cls.get_attr(name.borrow_value()) {
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

    #[pyslot]
    fn tp_new(
        cls: PyClassRef,
        py_type: OptionalArg<PyClassRef>,
        py_obj: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PySuperRef> {
        // Get the type:
        let typ = if let OptionalArg::Present(ty) = py_type {
            ty
        } else {
            let obj = vm
                .current_scope()
                .load_cell(vm, "__class__")
                .ok_or_else(|| {
                    vm.new_type_error(
                        "super must be called with 1 argument or from inside class method"
                            .to_owned(),
                    )
                })?;
            PyClassRef::try_from_object(vm, obj)?
        };

        // Check type argument:
        if !objtype::isinstance(typ.as_object(), &vm.ctx.types.type_type) {
            return Err(vm.new_type_error(format!(
                "super() argument 1 must be type, not {}",
                typ.lease_class().name
            )));
        }

        // Get the bound object:
        let obj = if let OptionalArg::Present(obj) = py_obj {
            obj
        } else {
            let frame = vm.current_frame().expect("no current frame for super()");
            if let Some(first_arg) = frame.code.arg_names.get(0) {
                let locals = frame.scope.get_locals();
                locals
                    .get_item_option(first_arg.as_str(), vm)?
                    .ok_or_else(|| {
                        vm.new_type_error(format!("super argument {} was not supplied", first_arg))
                    })?
            } else {
                vm.get_none()
            }
        };

        PySuper::new(typ, obj, vm)?.into_ref_with_type(vm, cls)
    }
}

impl SlotDescriptor for PySuper {
    fn descr_get(
        vm: &VirtualMachine,
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        _cls: OptionalArg<PyObjectRef>,
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
            let obj = zelf.obj.clone().map_or(vm.get_none(), |(o, _)| o);
            vm.invoke(
                zelf_class.as_object(),
                vec![zelf.typ.clone().into_object(), obj],
            )
        }
    }
}

fn supercheck(ty: PyClassRef, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyClassRef> {
    if let Ok(cls) = obj.clone().downcast::<PyClass>() {
        if objtype::issubclass(&cls, &ty) {
            return Ok(cls);
        }
    }
    if objtype::isinstance(&obj, &ty) {
        return Ok(obj.class());
    }
    let class_attr = vm.get_attribute(obj, "__class__")?;
    if let Ok(cls) = class_attr.downcast::<PyClass>() {
        if !cls.is(&ty) && objtype::issubclass(&cls, &ty) {
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
