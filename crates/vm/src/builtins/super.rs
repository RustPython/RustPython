// spell-checker:ignore cmeth
/*! Python `super` class.

See also [CPython source code.](https://github.com/python/cpython/blob/50b48572d9a90c5bb36e2bef6179548ea927a35a/Objects/typeobject.c#L7663)
*/

use super::{PyStr, PyType, PyTypeRef};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyResult, VirtualMachine,
    builtins::function::PyCell,
    class::PyClassImpl,
    function::{FuncArgs, IntoFuncArgs, OptionalArg},
    object::PyAtomicRef,
    types::{Callable, Constructor, GetAttr, GetDescriptor, Initializer, Representable},
};

#[pyclass(module = false, name = "super", traverse)]
#[derive(Debug)]
pub struct PySuper {
    #[pymember(name = "__thisclass__")]
    typ: PyAtomicRef<Option<PyType>>,
    #[pymember(name = "__self__")]
    obj: PyAtomicRef<Option<PyObject>>,
    #[pymember(name = "__self_class__")]
    obj_type: PyAtomicRef<Option<PyType>>,
}

fn bind_super(
    typ: PyTypeRef,
    obj: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<(PyTypeRef, Option<PyObjectRef>, Option<PyTypeRef>)> {
    if vm.is_none(&obj) {
        return Ok((typ, None, None));
    }
    let obj_type = super_check(&typ, &obj, vm)?;
    Ok((typ, Some(obj), Some(obj_type)))
}

impl PySuper {
    fn empty() -> Self {
        Self {
            typ: PyAtomicRef::from(None),
            obj: PyAtomicRef::from(None),
            obj_type: PyAtomicRef::from(None),
        }
    }

    fn store_bound(
        &self,
        typ: Option<PyTypeRef>,
        obj: Option<PyObjectRef>,
        obj_type: Option<PyTypeRef>,
    ) {
        drop(self.typ.store(typ));
        drop(self.obj.store(obj));
        drop(self.obj_type.store(obj_type));
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

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        // `tp_new` leaves every field null. `tp_init` fills them in.
        Ok(Self::empty())
    }
}

#[derive(FromArgs)]
pub struct InitArgs {
    #[pyarg(
        positional,
        optional,
        name = "type",
        error_msg = "super() argument 1 must be a type"
    )]
    py_type: OptionalArg<PyTypeRef>,
    #[pyarg(positional, optional)]
    object: OptionalArg<PyObjectRef>,
}

impl Initializer for PySuper {
    type Args = InitArgs;

    fn init(
        zelf: &Py<Self>,
        Self::Args { py_type, object }: Self::Args,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let typ = match py_type {
            OptionalArg::Present(ty) => Some(ty),
            OptionalArg::Missing => None,
        };
        let obj = match object {
            OptionalArg::Present(obj) => Some(obj),
            OptionalArg::Missing => None,
        };
        super_init_impl(zelf, typ, obj, vm)
    }
}

/// `super_init_impl`. A null type takes the zero-argument path.
fn super_init_impl(
    zelf: &Py<PySuper>,
    typ: Option<PyTypeRef>,
    obj: Option<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let (typ, obj) = match typ {
        Some(typ) => (typ, obj.unwrap_or_else(|| vm.ctx.none())),
        None => super_init_without_args(vm)?,
    };
    let (typ, obj, obj_type) = bind_super(typ, obj, vm)?;
    zelf.store_bound(Some(typ), obj, obj_type);
    Ok(())
}

fn super_init_without_args(vm: &VirtualMachine) -> PyResult<(PyTypeRef, PyObjectRef)> {
    // Access the InterpreterFrame directly — no need to materialize
    // a FrameObject just to read code/locals.
    let iframe_ptr = crate::vm::thread::get_current_frame();
    if iframe_ptr.is_null() {
        return Err(vm.new_runtime_error("super(): no current frame"));
    }
    let iframe = unsafe { &*iframe_ptr };
    let code = iframe.code();

    if code.arg_count == 0 {
        return Err(vm.new_runtime_error("super(): no arguments"));
    }

    // SAFETY: InterpreterFrame is current and not concurrently mutated.
    use rustpython_compiler_core::bytecode::CO_FAST_CELL;
    let fastlocals = iframe.localsplus.fastlocals();
    let obj = fastlocals[0]
        .clone()
        .and_then(|val| {
            // If slot 0 is a merged cell (LOCAL|CELL), extract value from cell
            if code
                .localspluskinds
                .first()
                .is_some_and(|&k| k & CO_FAST_CELL != 0)
            {
                val.downcast_ref::<PyCell>().and_then(|c| c.get())
            } else {
                Some(val)
            }
        })
        .ok_or_else(|| vm.new_runtime_error("super(): arg[0] deleted"))?;

    let mut typ = None;
    // Search for __class__ in freevars using localspluskinds
    let nlocalsplus = code.localspluskinds.len();
    let nfrees = code.freevars.len();
    let free_start = nlocalsplus - nfrees;
    for (i, var) in code.freevars.iter().enumerate() {
        if var.as_bytes() == b"__class__" {
            let class = fastlocals[free_start + i]
                .as_ref()
                .and_then(|v| v.downcast_ref::<PyCell>())
                .and_then(|c| c.get())
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
        vm.new_type_error("super must be called with 1 argument or from inside class method")
    })?;

    Ok((typ, obj))
}

#[pyclass(
    with(GetAttr, GetDescriptor, Constructor, Initializer, Representable),
    flags(BASETYPE)
)]
impl PySuper {}

impl GetAttr for PySuper {
    fn getattro(zelf: &Py<Self>, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
        let skip = |zelf: &Py<Self>, name| zelf.as_object().generic_getattr(name, vm);
        let Some(obj) = zelf.obj.load_owned() else {
            return skip(zelf, name);
        };
        let Some(start_type) = zelf.obj_type.load_owned() else {
            return skip(zelf, name);
        };

        // We want __class__ to return the class of the super object
        // (i.e. super, or a subclass), not the class of su->obj.
        if name.as_bytes() == b"__class__" {
            return skip(zelf, name);
        }

        if let Some(name) = vm.ctx.interned_str(name) {
            // Walk start_type's MRO by reference (no Vec allocation, no
            // per-class clone) up to and including zelf.typ, then look for
            // the first class past it that declares `name` directly.
            // Both locks are dropped before any arbitrary Python code
            // (the descriptor call below) runs, so they can't be held
            // across a call that might re-enter and want them again.
            let Some(su_type) = zelf.typ.load_owned() else {
                return skip(zelf, name);
            };
            let descr = {
                let mro = start_type.mro.read();
                mro.iter()
                    .skip_while(|cls| !cls.is(&su_type))
                    .skip(1) // skip su->type (if any)
                    .find_map(|cls| cls.get_direct_attr(name))
            };
            if let Some(descr) = descr {
                return vm
                    .call_get_descriptor_specific(
                        &descr,
                        // Only pass 'obj' param if this is instance-mode super (See https://bugs.python.org/issue743267)
                        if obj.is(&start_type) {
                            None
                        } else {
                            Some(&obj)
                        },
                        Some(start_type.as_object()),
                    )
                    .unwrap_or(Ok(descr));
            }
        }
        skip(zelf, name)
    }
}

impl GetDescriptor for PySuper {
    fn descr_get(
        zelf_obj: &PyObject,
        obj: Option<&PyObject>,
        _cls: Option<&PyObject>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let (zelf, obj) = Self::_unwrap(zelf_obj, obj, vm)?;
        if vm.is_none(obj) || zelf.obj.deref().is_some() {
            return Ok(zelf_obj.to_owned());
        }
        let zelf_class = zelf.as_object().class();
        let typ = zelf.typ.load_owned();
        if zelf_class.is(vm.ctx.types.super_type) {
            let newobj = Self::empty().into_ref(&vm.ctx);
            // A null type takes the zero-argument path inside super_init_impl.
            super_init_impl(&newobj, typ, Some(obj.to_owned()), vm)?;
            Ok(newobj.into())
        } else {
            // Call stops at the first null argument.
            let args = match typ {
                Some(typ) => (typ, obj.to_owned()).into_args(vm),
                None => FuncArgs::default(),
            };
            PyType::call(zelf.class(), args, vm)
        }
    }
}

impl Representable for PySuper {
    #[inline]
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        let type_name = match zelf.typ.load_owned() {
            Some(ty) => ty.name().to_owned(),
            None => "NULL".to_owned(),
        };
        let repr = match zelf.obj_type.load_owned() {
            Some(ty) => format!("<super: <class '{}'>, <{} object>>", type_name, ty.name()),
            None => format!("<super: <class '{type_name}'>, NULL>"),
        };
        Ok(repr)
    }
}

fn super_check(ty: &Py<PyType>, obj: &PyObject, vm: &VirtualMachine) -> PyResult<PyTypeRef> {
    let typ = match obj.to_owned().downcast::<PyType>() {
        Ok(cls) if cls.fast_issubclass(ty) => return Ok(cls),
        Ok(cls) => Some(cls),
        Err(_) => None,
    };

    if obj.fast_isinstance(ty) {
        return Ok(obj.class().to_owned());
    }

    let class_attr = obj.get_attr("__class__", vm)?;
    if let Ok(cls) = class_attr.downcast::<PyType>()
        && !cls.is(obj.class())
        && cls.fast_issubclass(ty)
    {
        return Ok(cls);
    }

    let (type_or_instance, obj_str) = match typ {
        Some(t) => ("type", t.name().to_owned()),
        None => ("instance of", obj.class().name().to_owned()),
    };

    Err(vm.new_type_error(format!(
        "super(type, obj): obj ({} {}) is not an instance or subtype of type ({}).",
        type_or_instance,
        obj_str,
        ty.name(),
    )))
}

pub(crate) fn init(context: &'static Context) {
    let super_type = &context.types.super_type;
    PySuper::extend_class(context, super_type);

    const SUPER_DOC: &str = "\
super() -> same as super(__class__, <first argument>)
super(type) -> unbound super object
super(type, obj) -> bound super object; requires isinstance(obj, type)
super(type, type2) -> bound super object; requires issubclass(type2, type)
Typical use to call a cooperative superclass method:
class C(B):
    def meth(self, arg):
        super().meth(arg)
This works for class methods too:
class C(B):
    @classmethod
    def cmeth(cls, arg):
        super().cmeth(arg)
";

    extend_class!(context, super_type, {
        "__doc__" => context.new_str(SUPER_DOC),
    });
}
