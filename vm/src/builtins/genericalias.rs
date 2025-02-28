use once_cell::sync::Lazy;

use super::type_;
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
    VirtualMachine, atomic_func,
    builtins::{PyList, PyStr, PyTuple, PyTupleRef, PyType, PyTypeRef},
    class::PyClassImpl,
    common::hash,
    convert::ToPyObject,
    function::{FuncArgs, PyComparisonValue},
    protocol::{PyMappingMethods, PyNumberMethods},
    types::{
        AsMapping, AsNumber, Callable, Comparable, Constructor, GetAttr, Hashable, PyComparisonOp,
        Representable,
    },
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GenericAlias")
    }
}

impl PyPayload for PyGenericAlias {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.generic_alias_type
    }
}

impl Constructor for PyGenericAlias {
    type Args = FuncArgs;

    fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        if !args.kwargs.is_empty() {
            return Err(vm.new_type_error("GenericAlias() takes no keyword arguments".to_owned()));
        }
        let (origin, arguments): (_, PyObjectRef) = args.bind(vm)?;
        PyGenericAlias::new(origin, arguments, vm)
            .into_ref_with_type(vm, cls)
            .map(Into::into)
    }
}

#[pyclass(
    with(
        AsNumber,
        AsMapping,
        Callable,
        Comparable,
        Constructor,
        GetAttr,
        Hashable,
        Representable
    ),
    flags(BASETYPE)
)]
impl PyGenericAlias {
    pub fn new(origin: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> Self {
        let args = if let Ok(tuple) = args.try_to_ref::<PyTuple>(vm) {
            tuple.to_owned()
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

    fn repr(&self, vm: &VirtualMachine) -> PyResult<String> {
        fn repr_item(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
            if obj.is(&vm.ctx.ellipsis) {
                return Ok("...".to_string());
            }

            if vm
                .get_attribute_opt(obj.clone(), identifier!(vm, __origin__))?
                .is_some()
                && vm
                    .get_attribute_opt(obj.clone(), identifier!(vm, __args__))?
                    .is_some()
            {
                return Ok(obj.repr(vm)?.as_str().to_string());
            }

            match (
                vm.get_attribute_opt(obj.clone(), identifier!(vm, __qualname__))?
                    .and_then(|o| o.downcast_ref::<PyStr>().map(|n| n.as_str().to_string())),
                vm.get_attribute_opt(obj.clone(), identifier!(vm, __module__))?
                    .and_then(|o| o.downcast_ref::<PyStr>().map(|m| m.as_str().to_string())),
            ) {
                (None, _) | (_, None) => Ok(obj.repr(vm)?.as_str().to_string()),
                (Some(qualname), Some(module)) => Ok(if module == "builtins" {
                    qualname
                } else {
                    format!("{module}.{qualname}")
                }),
            }
        }

        Ok(format!(
            "{}[{}]",
            repr_item(self.origin.clone().into(), vm)?,
            if self.args.len() == 0 {
                "()".to_owned()
            } else {
                self.args
                    .iter()
                    .map(|o| repr_item(o.clone(), vm))
                    .collect::<PyResult<Vec<_>>>()?
                    .join(", ")
            }
        ))
    }

    #[pygetset(magic)]
    fn parameters(&self) -> PyObjectRef {
        self.parameters.clone().into()
    }

    #[pygetset(magic)]
    fn args(&self) -> PyObjectRef {
        self.args.clone().into()
    }

    #[pygetset(magic)]
    fn origin(&self) -> PyObjectRef {
        self.origin.clone().into()
    }

    #[pymethod(magic)]
    fn getitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let new_args = subs_parameters(
            |vm| self.repr(vm),
            self.args.clone(),
            self.parameters.clone(),
            needle,
            vm,
        )?;

        Ok(
            PyGenericAlias::new(self.origin.clone(), new_args.to_pyobject(vm), vm)
                .into_pyobject(vm),
        )
    }

    #[pymethod(magic)]
    fn dir(&self, vm: &VirtualMachine) -> PyResult<PyList> {
        let dir = vm.dir(Some(self.origin()))?;
        for exc in &ATTR_EXCEPTIONS {
            if !dir.contains((*exc).to_pyobject(vm), vm)? {
                dir.append((*exc).to_pyobject(vm));
            }
        }
        Ok(dir)
    }

    #[pymethod(magic)]
    fn reduce(zelf: &Py<Self>, vm: &VirtualMachine) -> (PyTypeRef, (PyTypeRef, PyTupleRef)) {
        (
            vm.ctx.types.generic_alias_type.to_owned(),
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

    #[pymethod(magic)]
    fn ror(zelf: PyObjectRef, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        type_::or_(other, zelf, vm)
    }

    #[pymethod(magic)]
    fn or(zelf: PyObjectRef, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        type_::or_(zelf, other, vm)
    }
}

pub(crate) fn is_typevar(obj: &PyObjectRef, vm: &VirtualMachine) -> bool {
    let class = obj.class();
    "TypeVar" == &*class.slot_name()
        && class
            .get_attr(identifier!(vm, __module__))
            .and_then(|o| o.downcast_ref::<PyStr>().map(|s| s.as_str() == "typing"))
            .unwrap_or(false)
}

pub(crate) fn make_parameters(args: &Py<PyTuple>, vm: &VirtualMachine) -> PyTupleRef {
    let mut parameters: Vec<PyObjectRef> = Vec::with_capacity(args.len());
    for arg in args {
        if is_typevar(arg, vm) {
            if !parameters.iter().any(|param| param.is(arg)) {
                parameters.push(arg.clone());
            }
        } else if let Ok(obj) = arg.get_attr(identifier!(vm, __parameters__), vm) {
            if let Ok(sub_params) = obj.try_to_ref::<PyTuple>(vm) {
                for sub_param in sub_params {
                    if !parameters.iter().any(|param| param.is(sub_param)) {
                        parameters.push(sub_param.clone());
                    }
                }
            }
        }
    }
    parameters.shrink_to_fit();

    PyTuple::new_ref(parameters, &vm.ctx)
}

#[inline]
fn tuple_index(tuple: &PyTupleRef, item: &PyObjectRef) -> Option<usize> {
    tuple.iter().position(|element| element.is(item))
}

fn subs_tvars(
    obj: PyObjectRef,
    params: &PyTupleRef,
    argitems: &[PyObjectRef],
    vm: &VirtualMachine,
) -> PyResult {
    obj.get_attr(identifier!(vm, __parameters__), vm)
        .ok()
        .and_then(|sub_params| {
            PyTupleRef::try_from_object(vm, sub_params)
                .ok()
                .and_then(|sub_params| {
                    if sub_params.len() > 0 {
                        let sub_args = sub_params
                            .iter()
                            .map(|arg| {
                                if let Some(idx) = tuple_index(params, arg) {
                                    argitems[idx].clone()
                                } else {
                                    arg.clone()
                                }
                            })
                            .collect::<Vec<_>>();
                        let sub_args: PyObjectRef = PyTuple::new_ref(sub_args, &vm.ctx).into();
                        Some(obj.get_item(&*sub_args, vm))
                    } else {
                        None
                    }
                })
        })
        .unwrap_or(Ok(obj))
}

pub fn subs_parameters<F: Fn(&VirtualMachine) -> PyResult<String>>(
    repr: F,
    args: PyTupleRef,
    parameters: PyTupleRef,
    needle: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<PyTupleRef> {
    let num_params = parameters.len();
    if num_params == 0 {
        return Err(vm.new_type_error(format!("There are no type variables left in {}", repr(vm)?)));
    }

    let items = needle.try_to_ref::<PyTuple>(vm);
    let arg_items = match items {
        Ok(tuple) => tuple.as_slice(),
        Err(_) => std::slice::from_ref(&needle),
    };

    let num_items = arg_items.len();
    if num_params != num_items {
        let plural = if num_items > num_params {
            "many"
        } else {
            "few"
        };
        return Err(vm.new_type_error(format!("Too {} arguments for {}", plural, repr(vm)?)));
    }

    let new_args = args
        .iter()
        .map(|arg| {
            if is_typevar(arg, vm) {
                let idx = tuple_index(&parameters, arg).unwrap();
                Ok(arg_items[idx].clone())
            } else {
                subs_tvars(arg.clone(), &parameters, arg_items, vm)
            }
        })
        .collect::<PyResult<Vec<_>>>()?;

    Ok(PyTuple::new_ref(new_args, &vm.ctx))
}

impl AsMapping for PyGenericAlias {
    fn as_mapping() -> &'static PyMappingMethods {
        static AS_MAPPING: Lazy<PyMappingMethods> = Lazy::new(|| PyMappingMethods {
            subscript: atomic_func!(|mapping, needle, vm| {
                PyGenericAlias::mapping_downcast(mapping).getitem(needle.to_owned(), vm)
            }),
            ..PyMappingMethods::NOT_IMPLEMENTED
        });
        &AS_MAPPING
    }
}

impl AsNumber for PyGenericAlias {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            or: Some(|a, b, vm| Ok(PyGenericAlias::or(a.to_owned(), b.to_owned(), vm))),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

impl Callable for PyGenericAlias {
    type Args = FuncArgs;
    fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        PyType::call(&zelf.origin, args, vm).map(|obj| {
            if let Err(exc) = obj.set_attr(identifier!(vm, __orig_class__), zelf.to_owned(), vm) {
                if !exc.fast_isinstance(vm.ctx.exceptions.attribute_error)
                    && !exc.fast_isinstance(vm.ctx.exceptions.type_error)
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
        zelf: &Py<Self>,
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
    fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<hash::PyHash> {
        Ok(zelf.origin.as_object().hash(vm)? ^ zelf.args.as_object().hash(vm)?)
    }
}

impl GetAttr for PyGenericAlias {
    fn getattro(zelf: &Py<Self>, attr: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
        for exc in ATTR_EXCEPTIONS.iter() {
            if *(*exc) == attr.to_string() {
                return zelf.as_object().generic_getattr(attr, vm);
            }
        }
        zelf.origin().get_attr(attr, vm)
    }
}

impl Representable for PyGenericAlias {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        zelf.repr(vm)
    }
}

pub fn init(context: &Context) {
    let generic_alias_type = &context.types.generic_alias_type;
    PyGenericAlias::extend_class(context, generic_alias_type);
}
