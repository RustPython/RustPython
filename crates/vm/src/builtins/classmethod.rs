use super::{
    PyBoundMethod, PyGenericAlias, PyStr, PyStrInterned, PyType, PyTypeRef, object_get_dict,
};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    class::{PyClassDef, PyClassImpl},
    common::lock::PyMutex,
    function::{FuncArgs, PySetterValue},
    types::{Constructor, GetDescriptor, Initializer, Representable},
};

/// classmethod(function) -> method
///
/// Convert a function to be a class method.
///
/// A class method receives the class as implicit first argument,
/// just like an instance method receives the instance.
/// To declare a class method, use this idiom:
///
///   class C:
///       @classmethod
///       def f(cls, arg1, arg2, ...):
///           ...
///
/// It can be called either on the class (e.g. C.f()) or on an instance
/// (e.g. C().f()).  The instance is ignored except for its class.
/// If a class method is called for a derived class, the derived class
/// object is passed as the implied first argument.
///
/// Class methods are different than C++ or Java static methods.
/// If you want those, see the staticmethod builtin.
#[pyclass(
    module = false,
    name = "classmethod",
    text_signature = "(function, /)",
    traverse
)]
#[derive(Debug)]
pub struct PyClassMethod {
    callable: PyMutex<PyObjectRef>,
}

impl From<PyObjectRef> for PyClassMethod {
    fn from(callable: PyObjectRef) -> Self {
        Self {
            callable: PyMutex::new(callable),
        }
    }
}

impl PyPayload for PyClassMethod {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.classmethod_type
    }
}

impl GetDescriptor for PyClassMethod {
    fn descr_get(
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        cls: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let (zelf, _obj) = Self::_unwrap(&zelf, obj, vm)?;
        let cls = cls.unwrap_or_else(|| _obj.class().to_owned().into());
        let callable = zelf.callable.lock().clone();
        Ok(PyBoundMethod::new(cls, callable).into_ref(&vm.ctx).into())
    }
}

impl Constructor for PyClassMethod {
    type Args = PyObjectRef;

    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // Validate the signature here, but defer storing the callable and
        // copying its attributes to `__init__` so that subclasses overriding
        // `__init__` without calling `super().__init__()` see `__func__` as
        // `None`, matching CPython.
        let _: Self::Args = args.bind_for(vm, Self::NAME)?;
        let classmethod = Self {
            callable: PyMutex::new(vm.ctx.none()),
        };
        let result = PyRef::new_ref(classmethod, cls, Some(vm.ctx.new_dict()));
        Ok(PyObjectRef::from(result))
    }

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
    }
}

impl Initializer for PyClassMethod {
    type Args = PyObjectRef;

    fn init(zelf: PyRef<Self>, callable: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        *zelf.callable.lock() = callable.clone();
        functools_wraps(zelf.as_object(), &callable, vm)
    }
}

impl PyClassMethod {
    #[deprecated(note = "use PyClassMethod::from(...).into_ref() instead")]
    pub fn new_ref(callable: PyObjectRef, ctx: &Context) -> PyRef<Self> {
        Self::from(callable).into_ref(ctx)
    }
}

#[pyclass(
    with(GetDescriptor, Constructor, Initializer, Representable),
    flags(BASETYPE, HAS_DICT, HAS_WEAKREF)
)]
impl PyClassMethod {
    #[pymember]
    fn __func__(vm: &VirtualMachine, zelf: PyObjectRef) -> PyResult {
        let zelf: &Py<Self> = zelf.try_to_value(vm)?;
        Ok(zelf.callable.lock().clone())
    }

    #[pymember]
    fn __wrapped__(vm: &VirtualMachine, zelf: PyObjectRef) -> PyResult {
        let zelf: &Py<Self> = zelf.try_to_value(vm)?;
        Ok(zelf.callable.lock().clone())
    }

    #[pygetset]
    fn __annotations__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
        let callable = zelf.callable.lock();
        descriptor_get_wrapped_attribute(
            &callable,
            zelf.as_object(),
            identifier!(vm.ctx, __annotations__),
            vm,
        )
    }

    #[pygetset(setter)]
    fn set___annotations__(
        zelf: &Py<Self>,
        value: PySetterValue,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        descriptor_set_wrapped_attribute(
            zelf.as_object(),
            identifier!(vm.ctx, __annotations__),
            value,
            "classmethod",
            vm,
        )
    }

    #[pygetset]
    fn __annotate__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
        let callable = zelf.callable.lock();
        descriptor_get_wrapped_attribute(
            &callable,
            zelf.as_object(),
            identifier!(vm.ctx, __annotate__),
            vm,
        )
    }

    #[pygetset(setter)]
    fn set___annotate__(
        zelf: &Py<Self>,
        value: PySetterValue,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        descriptor_set_wrapped_attribute(
            zelf.as_object(),
            identifier!(vm.ctx, __annotate__),
            value,
            "classmethod",
            vm,
        )
    }

    #[pygetset]
    fn __isabstractmethod__(&self, vm: &VirtualMachine) -> PyObjectRef {
        let callable = self.callable.lock().clone();
        if let Ok(Some(is_abstract)) = vm.get_attribute_opt(callable, "__isabstractmethod__") {
            is_abstract
        } else {
            vm.ctx.new_bool(false).into()
        }
    }

    #[pygetset(setter)]
    fn set___isabstractmethod__(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.callable
            .lock()
            .set_attr("__isabstractmethod__", value, vm)?;
        Ok(())
    }

    #[pyclassmethod]
    fn __class_getitem__(
        cls: PyTypeRef,
        args: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyGenericAlias> {
        PyGenericAlias::from_args(cls, args, vm)
    }
}

impl Representable for PyClassMethod {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let callable = zelf.callable.lock().repr(vm)?;
        let class = Self::class(&vm.ctx);

        let repr = match (
            class
                .__qualname__(vm)
                .downcast_ref::<PyStr>()
                .map(|n| n.as_wtf8()),
            class
                .__module__(vm)
                .downcast_ref::<PyStr>()
                .map(|m| m.as_wtf8()),
        ) {
            (None, _) => return Err(vm.new_type_error("Unknown qualified name")),
            (Some(qualname), Some(module)) if module != "builtins" => {
                format!("<{module}.{qualname}({callable})>")
            }
            _ => format!("<{}({})>", class.slot_name(), callable),
        };
        Ok(repr)
    }
}

pub(crate) fn init(context: &'static Context) {
    PyClassMethod::extend_class(context, context.types.classmethod_type);
}

pub(crate) fn functools_wraps(
    wrapper: &PyObject,
    wrapped: &PyObject,
    vm: &VirtualMachine,
) -> PyResult<()> {
    for attr in [
        identifier!(vm.ctx, __module__),
        identifier!(vm.ctx, __name__),
        identifier!(vm.ctx, __qualname__),
        identifier!(vm.ctx, __doc__),
    ] {
        if let Some(value) = vm.get_attribute_opt(wrapped.to_owned(), attr)? {
            wrapper.set_attr(attr, value, vm)?;
        }
    }
    Ok(())
}

pub(crate) fn descriptor_get_wrapped_attribute(
    wrapped: &PyObject,
    obj: &PyObject,
    name: &'static PyStrInterned,
    vm: &VirtualMachine,
) -> PyResult {
    let dict = object_get_dict(obj.to_owned(), vm)?;
    if let Some(res) = dict.get_item_opt(name, vm)? {
        return Ok(res);
    }
    let res = wrapped.get_attr(name, vm)?;
    dict.set_item(name, res.clone(), vm)?;
    Ok(res)
}

pub(crate) fn descriptor_set_wrapped_attribute(
    obj: &PyObject,
    name: &'static PyStrInterned,
    value: PySetterValue,
    type_name: &str,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let dict = object_get_dict(obj.to_owned(), vm)?;
    match value {
        PySetterValue::Delete => match dict.del_item(name, vm) {
            Ok(()) => Ok(()),
            Err(e) if e.fast_isinstance(vm.ctx.exceptions.key_error) => Err(vm
                .new_attribute_error(format!(
                    "'{type_name}' object has no attribute '{}'",
                    name.as_str()
                ))),
            Err(e) => Err(e),
        },
        PySetterValue::Assign(value) => dict.set_item(name, value, vm),
    }
}
