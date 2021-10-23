use crate::{
    builtins::{PyList, PyStr, PyStrRef, PyTuple, PyTupleRef, PyType, PyTypeRef},
    common::hash,
    function::{FuncArgs, IntoPyObject},
    types::{Callable, Comparable, Constructor, GetAttr, Hashable, PyComparisonOp},
    IdProtocol, PyClassImpl, PyComparisonValue, PyContext, PyObject, PyObjectRef, PyRef, PyResult,
    PyValue, TryFromObject, TypeProtocol, VirtualMachine,
};
use std::fmt;

static ATTR_EXCEPTIONS: [&str; 8] = [
    "__origin__",
    "__args__",
    "__parameters__",
    "__mro_entries__",
    "__reduce_ex__", // needed so we don't look up object.__reduce_ex__
    "__reduce__",
    "__copy__",
    "__deepcopy__",
];

#[pyclass(module = "types", name = "GenericAlias")]
pub struct PyGenericAlias {
    origin: PyTypeRef,
    args: PyTupleRef,
    parameters: PyTupleRef,
}

impl fmt::Debug for PyGenericAlias {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("GenericAlias")
    }
}

impl PyValue for PyGenericAlias {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.generic_alias_type
    }
}

#[derive(FromArgs)]
pub struct GenericAliasArgs {
    origin: PyTypeRef,
    arguments: PyObjectRef,
}

impl Constructor for PyGenericAlias {
    type Args = GenericAliasArgs;

    fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        PyGenericAlias::new(args.origin, args.arguments, vm).into_pyresult_with_type(vm, cls)
    }
}

#[pyimpl(
    with(Callable, Comparable, Constructor, GetAttr, Hashable),
    flags(BASETYPE)
)]
impl PyGenericAlias {
    pub fn new(origin: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> Self {
        let args: PyTupleRef = if let Ok(tuple) = PyTupleRef::try_from_object(vm, args.clone()) {
            tuple
        } else {
            PyTuple::new_ref(vec![args], &vm.ctx)
        };

        let parameters = make_parameters(&args, vm);
        Self {
            origin,
            args,
            parameters,
        }
    }

    #[pymethod(magic)]
    fn repr(&self, vm: &VirtualMachine) -> PyResult<String> {
        fn repr_item(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
            if obj.is(&vm.ctx.ellipsis) {
                return Ok("...".to_string());
            }

            if vm.get_attribute_opt(obj.clone(), "__origin__")?.is_some()
                && vm.get_attribute_opt(obj.clone(), "__args__")?.is_some()
            {
                return Ok(obj.repr(vm)?.as_str().to_string());
            }

            match (
                vm.get_attribute_opt(obj.clone(), "__qualname__")?
                    .and_then(|o| o.downcast_ref::<PyStr>().map(|n| n.as_str().to_string())),
                vm.get_attribute_opt(obj.clone(), "__module__")?
                    .and_then(|o| o.downcast_ref::<PyStr>().map(|m| m.as_str().to_string())),
            ) {
                (None, _) | (_, None) => Ok(obj.repr(vm)?.as_str().to_string()),
                (Some(qualname), Some(module)) => Ok(if module == "builtins" {
                    qualname
                } else {
                    format!("{}.{}", module, qualname)
                }),
            }
        }

        Ok(format!(
            "{}[{}]",
            repr_item(self.origin.as_object().to_owned(), vm)?,
            self.args
                .as_slice()
                .iter()
                .map(|o| repr_item(o.clone(), vm))
                .collect::<PyResult<Vec<_>>>()?
                .join(", ")
        ))
    }

    #[pyproperty(magic)]
    fn parameters(&self) -> PyObjectRef {
        self.parameters.as_object().to_owned()
    }

    #[pyproperty(magic)]
    fn args(&self) -> PyObjectRef {
        self.args.as_object().to_owned()
    }

    #[pyproperty(magic)]
    fn origin(&self) -> PyObjectRef {
        self.origin.as_object().to_owned()
    }

    #[pymethod(magic)]
    fn dir(&self, vm: &VirtualMachine) -> PyResult<PyList> {
        let dir = vm.dir(Some(self.origin()))?;
        for exc in ATTR_EXCEPTIONS.iter() {
            if !dir.contains((*exc).into_pyobject(vm), vm)? {
                dir.append((*exc).into_pyobject(vm));
            }
        }
        Ok(dir)
    }

    #[pymethod(magic)]
    fn reduce(zelf: PyRef<Self>, vm: &VirtualMachine) -> (PyTypeRef, (PyTypeRef, PyTupleRef)) {
        (
            vm.ctx.types.generic_alias_type.clone(),
            (zelf.origin.clone(), zelf.args.clone()),
        )
    }

    #[pymethod(magic)]
    fn mro_entries(&self, _bases: PyObjectRef, vm: &VirtualMachine) -> PyTupleRef {
        PyTuple::new_ref(vec![self.origin()], &vm.ctx)
    }

    #[pymethod(magic)]
    fn instancecheck(_zelf: PyRef<Self>, _obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Err(vm
            .new_type_error("isinstance() argument 2 cannot be a parameterized generic".to_owned()))
    }

    #[pymethod(magic)]
    fn subclasscheck(_zelf: PyRef<Self>, _obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Err(vm
            .new_type_error("issubclass() argument 2 cannot be a parameterized generic".to_owned()))
    }
}

fn is_typevar(obj: &PyObjectRef) -> bool {
    let class = obj.class();
    class.slot_name() == "TypeVar"
        && class
            .get_attr("__module__")
            .and_then(|o| o.downcast_ref::<PyStr>().map(|s| s.as_str() == "typing"))
            .unwrap_or(false)
}

fn make_parameters(args: &PyTupleRef, vm: &VirtualMachine) -> PyTupleRef {
    let mut parameters: Vec<PyObjectRef> = vec![];
    for arg in args.as_slice() {
        if is_typevar(arg) {
            parameters.push(arg.clone());
        } else if let Ok(tuple) = arg
            .clone()
            .get_attr("__parameters__", vm)
            .and_then(|obj| PyTupleRef::try_from_object(vm, obj))
        {
            for subparam in tuple.as_slice() {
                parameters.push(subparam.clone());
            }
        }
    }

    PyTuple::new_ref(parameters, &vm.ctx)
}

impl Callable for PyGenericAlias {
    type Args = FuncArgs;
    fn call(zelf: &crate::PyObjectView<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        PyType::call(&zelf.origin, args, vm).map(|obj| {
            if let Err(exc) = obj.set_attr("__orig_class__", zelf.to_owned(), vm) {
                if !exc.isinstance(&vm.ctx.exceptions.attribute_error)
                    && !exc.isinstance(&vm.ctx.exceptions.type_error)
                {
                    return Err(exc);
                }
            }
            Ok(obj)
        })?
    }
}

impl Comparable for PyGenericAlias {
    fn cmp(
        zelf: &crate::PyObjectView<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        op.eq_only(|| {
            let other = class_or_notimplemented!(Self, other);
            Ok(PyComparisonValue::Implemented(
                if !zelf
                    .origin()
                    .rich_compare_bool(&other.origin(), PyComparisonOp::Eq, vm)?
                {
                    false
                } else {
                    zelf.args()
                        .rich_compare_bool(&other.args(), PyComparisonOp::Eq, vm)?
                },
            ))
        })
    }
}

impl Hashable for PyGenericAlias {
    #[inline]
    fn hash(zelf: &crate::PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<hash::PyHash> {
        Ok(zelf.origin.as_object().hash(vm)? ^ zelf.args.as_object().hash(vm)?)
    }
}

impl GetAttr for PyGenericAlias {
    fn getattro(zelf: PyRef<Self>, attr: PyStrRef, vm: &VirtualMachine) -> PyResult {
        for exc in ATTR_EXCEPTIONS.iter() {
            if *(*exc) == attr.to_string() {
                return vm.generic_getattribute(zelf.as_object().to_owned(), attr);
            }
        }
        zelf.origin().get_attr(attr, vm)
    }
}

pub fn init(context: &PyContext) {
    let generic_alias_type = &context.types.generic_alias_type;
    PyGenericAlias::extend_class(context, generic_alias_type);
}
