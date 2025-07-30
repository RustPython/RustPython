// spell-checker:ignore cmeth
/*! Python `super` class.

See also [CPython source code.](https://github.com/python/cpython/blob/50b48572d9a90c5bb36e2bef6179548ea927a35a/Objects/typeobject.c#L7663)
*/

use super::{PyStr, PyType, PyTypeRef};
use crate::{
    AsObject, Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    class::PyClassImpl,
    common::lock::PyRwLock,
    function::{FuncArgs, IntoFuncArgs, OptionalArg},
    types::{Callable, Constructor, GetAttr, GetDescriptor, Initializer, Representable},
};

#[pyclass(module = false, name = "super", traverse)]
#[derive(Debug)]
pub struct PySuper {
    inner: PyRwLock<PySuperInner>,
}

#[derive(Debug, Traverse)]
struct PySuperInner {
    typ: PyTypeRef,
    obj: Option<(PyObjectRef, PyTypeRef)>,
}

impl PySuperInner {
    fn new(typ: PyTypeRef, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<Self> {
        let obj = if vm.is_none(&obj) {
            None
        } else {
            let obj_type = super_check(typ.clone(), obj.clone(), vm)?;
            Some((obj, obj_type))
        };
        Ok(Self { typ, obj })
    }
}

impl PyPayload for PySuper {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.super_type
    }
}

impl Constructor for PySuper {
    type Args = FuncArgs;

    fn py_new(cls: PyTypeRef, _args: Self::Args, vm: &VirtualMachine) -> PyResult {
        let obj = Self {
            inner: PyRwLock::new(PySuperInner::new(
                vm.ctx.types.object_type.to_owned(), // is this correct?
                vm.ctx.none(),
                vm,
            )?),
        }
        .into_ref_with_type(vm, cls)?;
        Ok(obj.into())
    }
}

#[derive(FromArgs)]
pub struct InitArgs {
    #[pyarg(positional, optional)]
    py_type: OptionalArg<PyTypeRef>,
    #[pyarg(positional, optional)]
    py_obj: OptionalArg<PyObjectRef>,
}

impl Initializer for PySuper {
    type Args = InitArgs;

    fn init(
        zelf: PyRef<Self>,
        Self::Args { py_type, py_obj }: Self::Args,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        // Get the type:
        let (typ, obj) = if let OptionalArg::Present(ty) = py_type {
            (ty, py_obj.unwrap_or_none(vm))
        } else {
            let frame = vm
                .current_frame()
                .ok_or_else(|| vm.new_runtime_error("super(): no current frame"))?;

            if frame.code.arg_count == 0 {
                return Err(vm.new_runtime_error("super(): no arguments"));
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
                .ok_or_else(|| vm.new_runtime_error("super(): arg[0] deleted"))?;

            let mut typ = None;
            for (i, var) in frame.code.freevars.iter().enumerate() {
                if var.as_bytes() == b"__class__" {
                    let i = frame.code.cellvars.len() + i;
                    let class = frame.cells_frees[i]
                        .get()
                        .ok_or_else(|| vm.new_runtime_error("super(): empty __class__ cell"))?;
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
                    "super must be called with 1 argument or from inside class method",
                )
            })?;

            (typ, obj)
        };

        let inner = PySuperInner::new(typ, obj, vm)?;
        *zelf.inner.write() = inner;

        Ok(())
    }
}

#[pyclass(with(GetAttr, GetDescriptor, Constructor, Initializer, Representable))]
impl PySuper {
    #[pygetset]
    fn __thisclass__(&self) -> PyTypeRef {
        self.inner.read().typ.clone()
    }

    #[pygetset]
    fn __self_class__(&self) -> Option<PyTypeRef> {
        Some(self.inner.read().obj.as_ref()?.1.clone())
    }

    #[pygetset]
    fn __self__(&self) -> Option<PyObjectRef> {
        Some(self.inner.read().obj.as_ref()?.0.clone())
    }
}

impl GetAttr for PySuper {
    fn getattro(zelf: &Py<Self>, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
        let skip = |zelf: &Py<Self>, name| zelf.as_object().generic_getattr(name, vm);
        let (obj, start_type): (PyObjectRef, PyTypeRef) = match &zelf.inner.read().obj {
            Some(o) => o.clone(),
            None => return skip(zelf, name),
        };
        // We want __class__ to return the class of the super object
        // (i.e. super, or a subclass), not the class of su->obj.

        if name.as_bytes() == b"__class__" {
            return skip(zelf, name);
        }

        if let Some(name) = vm.ctx.interned_str(name) {
            // skip the classes in start_type.mro up to and including zelf.typ
            let mro: Vec<PyRef<PyType>> = start_type.mro_map_collect(|x| x.to_owned());
            let mro: Vec<_> = mro
                .iter()
                .skip_while(|cls| !cls.is(&zelf.inner.read().typ))
                .skip(1) // skip su->type (if any)
                .collect();
            for cls in &mro {
                if let Some(descr) = cls.get_direct_attr(name) {
                    return vm
                        .call_get_descriptor_specific(
                            &descr,
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
        zelf_obj: PyObjectRef,
        obj: Option<PyObjectRef>,
        _cls: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let (zelf, obj) = Self::_unwrap(&zelf_obj, obj, vm)?;
        if vm.is_none(&obj) || zelf.inner.read().obj.is_some() {
            return Ok(zelf_obj);
        }
        let zelf_class = zelf.as_object().class();
        if zelf_class.is(vm.ctx.types.super_type) {
            let typ = zelf.inner.read().typ.clone();
            Ok(Self {
                inner: PyRwLock::new(PySuperInner::new(typ, obj, vm)?),
            }
            .into_ref(&vm.ctx)
            .into())
        } else {
            let (obj, typ) = {
                let lock = zelf.inner.read();
                let obj = lock.obj.as_ref().map(|(o, _)| o.to_owned());
                let typ = lock.typ.clone();
                (obj, typ)
            };
            let obj = vm.unwrap_or_none(obj);
            PyType::call(zelf.class(), (typ, obj).into_args(vm), vm)
        }
    }
}

impl Representable for PySuper {
    #[inline]
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        let type_name = zelf.inner.read().typ.name().to_owned();
        let obj = zelf.inner.read().obj.clone();
        let repr = match obj {
            Some((_, ref ty)) => {
                format!("<super: <class '{}'>, <{} object>>", &type_name, ty.name())
            }
            None => format!("<super: <class '{type_name}'>, NULL>"),
        };
        Ok(repr)
    }
}

fn super_check(ty: PyTypeRef, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyTypeRef> {
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
    Err(vm.new_type_error("super(type, obj): obj must be an instance or subtype of type"))
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
