use super::{
    PyDictRef, PyGenericAlias, PyStr, PyType, PyTypeRef,
    builtin_func::PyNativeFunction,
    classmethod::{
        descriptor_get_wrapped_attribute, descriptor_set_wrapped_attribute, functools_wraps,
    },
    type_::{PyAttributes, rendered_item_doc},
};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    class::{PyClassDef, PyClassImpl},
    common::{hash::HashSecret, lock::PyMutex, wtf8::Wtf8Buf},
    function::{FuncArgs, PySetterValue},
    object::PyAtomicRef,
    types::{Callable, Constructor, GetDescriptor, Initializer, Representable},
};

// Context::genesis builds native descriptors before the first interpreter
// chooses its hash seed. Delay hashing their metadata until that seed exists.
rustpython_common::static_cell! {
    static NATIVE_METADATA: PyMutex<NativeMetadata>;
}

#[derive(Default)]
struct NativeMetadata {
    hash_secret: Option<&'static HashSecret>,
    pending: Vec<(PyDictRef, PyAttributes)>,
}

#[pyclass(module = false, name = "staticmethod", traverse)]
#[derive(Debug)]
pub struct PyStaticMethod {
    #[pymember(name = "__func__")]
    #[pymember(name = "__wrapped__")]
    pub callable: PyAtomicRef<PyObject>,
}

impl PyPayload for PyStaticMethod {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.staticmethod_type
    }
}

impl GetDescriptor for PyStaticMethod {
    fn descr_get(
        zelf: &PyObject,
        obj: Option<&PyObject>,
        _cls: Option<&PyObject>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let (zelf, _obj) = Self::_unwrap(zelf, obj, vm)?;
        Ok(zelf.callable.load_owned())
    }
}

impl From<PyObjectRef> for PyStaticMethod {
    fn from(callable: PyObjectRef) -> Self {
        Self {
            callable: PyAtomicRef::from(callable),
        }
    }
}

#[derive(FromArgs)]
pub struct StaticMethodArgs {
    #[pyarg(positional)]
    function: PyObjectRef,
}

impl Constructor for PyStaticMethod {
    type Args = StaticMethodArgs;

    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // Validate the signature here, but defer storing the callable and
        // copying its attributes to `__init__` so that subclasses overriding
        // `__init__` without calling `super().__init__()` see `__func__` as
        // `None`, matching CPython.
        let _: StaticMethodArgs = args.bind_for(vm, Self::NAME)?;
        let result = Self {
            callable: PyAtomicRef::from(vm.ctx.none()),
        }
        .into_ref_with_type(vm, cls)?;
        Ok(PyObjectRef::from(result))
    }

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
    }
}

impl PyStaticMethod {
    pub(crate) fn new_native(
        callable: PyRef<PyNativeFunction>,
        class: &Py<PyType>,
        ctx: &Context,
    ) -> PyRef<Self> {
        let mut qualname = class.heaptype_ext.as_ref().map_or_else(
            || Wtf8Buf::from_string(class.name().to_string()),
            |ext| ext.qualname.read().as_wtf8().to_owned(),
        );
        qualname.push_str(".");
        qualname.push_str(callable.value.name);
        let attrs = [
            (
                identifier!(ctx, __module__),
                callable.module.load_owned().unwrap_or_else(|| ctx.none()),
            ),
            (
                identifier!(ctx, __name__),
                ctx.new_str(callable.value.name).into(),
            ),
            (identifier!(ctx, __qualname__), ctx.new_str(qualname).into()),
            (
                identifier!(ctx, __doc__),
                rendered_item_doc(callable.value.name, callable.value.item_doc())
                    .map_or_else(|| ctx.none(), |doc| ctx.new_str(doc).into()),
            ),
        ]
        .into_iter()
        .collect();
        let dict = ctx.new_dict();
        let wrapper = PyRef::new_ref(
            Self::new(callable.into()),
            ctx.types.staticmethod_type.to_owned(),
            Some(dict.clone()),
        );
        let mut metadata = NATIVE_METADATA.get_or_init(PyMutex::default).lock();
        if let Some(secret) = metadata.hash_secret {
            dict.entries.initialize_attributes(attrs, secret);
        } else {
            metadata.pending.push((dict, attrs));
        }
        wrapper
    }

    pub(crate) fn init_native_metadata(secret: &'static HashSecret) {
        let mut metadata = NATIVE_METADATA.get_or_init(PyMutex::default).lock();
        metadata.hash_secret = Some(secret);
        for (dict, attrs) in metadata.pending.drain(..) {
            dict.entries.initialize_attributes(attrs, secret);
        }
    }

    #[must_use]
    pub fn new(callable: PyObjectRef) -> Self {
        Self {
            callable: PyAtomicRef::from(callable),
        }
    }

    #[deprecated(note = "use PyStaticMethod::new(...).into_ref() instead")]
    pub fn new_ref(callable: PyObjectRef, ctx: &Context) -> PyRef<Self> {
        Self::new(callable).into_ref(ctx)
    }
}

impl Initializer for PyStaticMethod {
    type Args = StaticMethodArgs;

    fn init(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        let callable = args.function;
        zelf.callable.store(callable.clone());
        functools_wraps(zelf.as_object(), &callable, vm)
    }
}

#[pyclass(
    with(Callable, GetDescriptor, Constructor, Initializer, Representable),
    flags(BASETYPE, HAS_DICT, HAS_WEAKREF)
)]
impl Py<PyStaticMethod> {
    #[pygetset]
    fn __annotations__(zelf: &Self, vm: &VirtualMachine) -> PyResult {
        let callable = zelf.callable.load_owned();
        descriptor_get_wrapped_attribute(
            &callable,
            zelf.as_object(),
            identifier!(vm.ctx, __annotations__),
            vm,
        )
    }

    #[pygetset(setter)]
    fn set___annotations__(zelf: &Self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
        descriptor_set_wrapped_attribute(
            zelf.as_object(),
            identifier!(vm.ctx, __annotations__),
            value,
            "staticmethod",
            vm,
        )
    }

    #[pygetset]
    fn __annotate__(zelf: &Self, vm: &VirtualMachine) -> PyResult {
        let callable = zelf.callable.load_owned();
        descriptor_get_wrapped_attribute(
            &callable,
            zelf.as_object(),
            identifier!(vm.ctx, __annotate__),
            vm,
        )
    }

    #[pygetset(setter)]
    fn set___annotate__(zelf: &Self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
        descriptor_set_wrapped_attribute(
            zelf.as_object(),
            identifier!(vm.ctx, __annotate__),
            value,
            "staticmethod",
            vm,
        )
    }

    #[pygetset]
    fn __isabstractmethod__(&self, vm: &VirtualMachine) -> PyObjectRef {
        let callable = self.callable.load_owned();

        if let Ok(Some(is_abstract)) = vm.get_attribute_opt(&callable, "__isabstractmethod__") {
            is_abstract
        } else {
            vm.ctx.new_bool(false).into()
        }
    }

    #[pygetset(setter)]
    fn set___isabstractmethod__(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.callable
            .load_owned()
            .set_attr("__isabstractmethod__", value, vm)?;
        Ok(())
    }

    #[pyclassmethod]
    fn __class_getitem__(
        cls: PyTypeRef,
        object: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyGenericAlias> {
        PyGenericAlias::from_args(cls, object, vm)
    }
}

impl Callable for PyStaticMethod {
    type Args = FuncArgs;
    #[inline]
    fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let callable = zelf.callable.load_owned();
        callable.call(args, vm)
    }
}

impl Representable for PyStaticMethod {
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let callable = zelf.callable.load_owned().repr(vm)?;
        let class = Self::class(&vm.ctx);

        let module = class.__module__(vm)?;
        match (
            class
                .__qualname__(vm)
                .downcast_ref::<PyStr>()
                .map(|n| n.as_wtf8()),
            module.downcast_ref::<PyStr>().map(|m| m.as_wtf8()),
        ) {
            (None, _) => Err(vm.new_type_error("Unknown qualified name")),
            (Some(qualname), Some(module)) if module != "builtins" => {
                Ok(format!("<{module}.{qualname}({callable})>"))
            }
            _ => Ok(format!("<{}({})>", class.slot_name(), callable)),
        }
    }
}

pub(crate) fn init(context: &'static Context) {
    PyStaticMethod::extend_class(context, context.types.staticmethod_type);
}
