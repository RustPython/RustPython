use super::{genericalias, type_};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    atomic_func,
    builtins::{PyFrozenSet, PyGenericAlias, PyStr, PyTuple, PyTupleRef, PyType},
    class::PyClassImpl,
    common::hash,
    convert::{ToPyObject, ToPyResult},
    function::PyComparisonValue,
    protocol::{PyMappingMethods, PyNumberMethods},
    types::{AsMapping, AsNumber, Comparable, GetAttr, Hashable, PyComparisonOp, Representable},
};
use std::fmt;
use std::sync::LazyLock;

const CLS_ATTRS: &[&str] = &["__module__"];

#[pyclass(module = "types", name = "UnionType", traverse)]
pub struct PyUnion {
    args: PyTupleRef,
    parameters: PyTupleRef,
}

impl fmt::Debug for PyUnion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UnionObject")
    }
}

impl PyPayload for PyUnion {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.union_type
    }
}

impl PyUnion {
    pub fn new(args: PyTupleRef, vm: &VirtualMachine) -> Self {
        let parameters = make_parameters(&args, vm);
        Self { args, parameters }
    }

    /// Direct access to args field, matching CPython's _Py_union_args
    #[inline]
    pub const fn args(&self) -> &PyTupleRef {
        &self.args
    }

    fn repr(&self, vm: &VirtualMachine) -> PyResult<String> {
        fn repr_item(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
            if obj.is(vm.ctx.types.none_type) {
                return Ok("None".to_string());
            }

            if vm
                .get_attribute_opt(obj.clone(), identifier!(vm, __origin__))?
                .is_some()
                && vm
                    .get_attribute_opt(obj.clone(), identifier!(vm, __args__))?
                    .is_some()
            {
                return Ok(obj.repr(vm)?.to_string());
            }

            match (
                vm.get_attribute_opt(obj.clone(), identifier!(vm, __qualname__))?
                    .and_then(|o| o.downcast_ref::<PyStr>().map(|n| n.to_string())),
                vm.get_attribute_opt(obj.clone(), identifier!(vm, __module__))?
                    .and_then(|o| o.downcast_ref::<PyStr>().map(|m| m.to_string())),
            ) {
                (None, _) | (_, None) => Ok(obj.repr(vm)?.to_string()),
                (Some(qualname), Some(module)) => Ok(if module == "builtins" {
                    qualname
                } else {
                    format!("{module}.{qualname}")
                }),
            }
        }

        Ok(self
            .args
            .iter()
            .map(|o| repr_item(o.clone(), vm))
            .collect::<PyResult<Vec<_>>>()?
            .join(" | "))
    }
}

#[pyclass(
    flags(BASETYPE),
    with(Hashable, Comparable, AsMapping, AsNumber, Representable)
)]
impl PyUnion {
    #[pygetset]
    fn __parameters__(&self) -> PyObjectRef {
        self.parameters.clone().into()
    }

    #[pygetset]
    fn __args__(&self) -> PyObjectRef {
        self.args.clone().into()
    }

    #[pymethod]
    fn __instancecheck__(
        zelf: PyRef<Self>,
        obj: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        if zelf
            .args
            .iter()
            .any(|x| x.class().is(vm.ctx.types.generic_alias_type))
        {
            Err(vm.new_type_error("isinstance() argument 2 cannot be a parameterized generic"))
        } else {
            obj.is_instance(zelf.__args__().as_object(), vm)
        }
    }

    #[pymethod]
    fn __subclasscheck__(
        zelf: PyRef<Self>,
        obj: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        if zelf
            .args
            .iter()
            .any(|x| x.class().is(vm.ctx.types.generic_alias_type))
        {
            Err(vm.new_type_error("issubclass() argument 2 cannot be a parameterized generic"))
        } else {
            obj.is_subclass(zelf.__args__().as_object(), vm)
        }
    }

    #[pymethod(name = "__ror__")]
    #[pymethod]
    fn __or__(zelf: PyObjectRef, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        type_::or_(zelf, other, vm)
    }

    #[pyclassmethod]
    fn __class_getitem__(
        cls: crate::builtins::PyTypeRef,
        args: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyGenericAlias {
        PyGenericAlias::from_args(cls, args, vm)
    }
}

pub fn is_unionable(obj: PyObjectRef, vm: &VirtualMachine) -> bool {
    obj.class().is(vm.ctx.types.none_type)
        || obj.downcastable::<PyType>()
        || obj.class().is(vm.ctx.types.generic_alias_type)
        || obj.class().is(vm.ctx.types.union_type)
}

fn make_parameters(args: &Py<PyTuple>, vm: &VirtualMachine) -> PyTupleRef {
    let parameters = genericalias::make_parameters(args, vm);
    dedup_and_flatten_args(&parameters, vm)
}

fn flatten_args(args: &Py<PyTuple>, vm: &VirtualMachine) -> PyTupleRef {
    let mut total_args = 0;
    for arg in args {
        if let Some(pyref) = arg.downcast_ref::<PyUnion>() {
            total_args += pyref.args.len();
        } else {
            total_args += 1;
        };
    }

    let mut flattened_args = Vec::with_capacity(total_args);
    for arg in args {
        if let Some(pyref) = arg.downcast_ref::<PyUnion>() {
            flattened_args.extend(pyref.args.iter().cloned());
        } else if vm.is_none(arg) {
            flattened_args.push(vm.ctx.types.none_type.to_owned().into());
        } else {
            flattened_args.push(arg.clone());
        };
    }

    PyTuple::new_ref(flattened_args, &vm.ctx)
}

fn dedup_and_flatten_args(args: &Py<PyTuple>, vm: &VirtualMachine) -> PyTupleRef {
    let args = flatten_args(args, vm);

    let mut new_args: Vec<PyObjectRef> = Vec::with_capacity(args.len());
    for arg in &*args {
        if !new_args.iter().any(|param| {
            param
                .rich_compare_bool(arg, PyComparisonOp::Eq, vm)
                .expect("types are always comparable")
        }) {
            new_args.push(arg.clone());
        }
    }

    new_args.shrink_to_fit();

    PyTuple::new_ref(new_args, &vm.ctx)
}

pub fn make_union(args: &Py<PyTuple>, vm: &VirtualMachine) -> PyObjectRef {
    let args = dedup_and_flatten_args(args, vm);
    match args.len() {
        1 => args[0].to_owned(),
        _ => PyUnion::new(args, vm).to_pyobject(vm),
    }
}

impl PyUnion {
    fn getitem(zelf: PyRef<Self>, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let new_args = genericalias::subs_parameters(
            zelf.to_owned().into(),
            zelf.args.clone(),
            zelf.parameters.clone(),
            needle,
            vm,
        )?;
        let mut res;
        if new_args.is_empty() {
            res = make_union(&new_args, vm);
        } else {
            res = new_args[0].to_owned();
            for arg in new_args.iter().skip(1) {
                res = vm._or(&res, arg)?;
            }
        }

        Ok(res)
    }
}

impl AsMapping for PyUnion {
    fn as_mapping() -> &'static PyMappingMethods {
        static AS_MAPPING: LazyLock<PyMappingMethods> = LazyLock::new(|| PyMappingMethods {
            subscript: atomic_func!(|mapping, needle, vm| {
                let zelf = PyUnion::mapping_downcast(mapping);
                PyUnion::getitem(zelf.to_owned(), needle.to_owned(), vm)
            }),
            ..PyMappingMethods::NOT_IMPLEMENTED
        });
        &AS_MAPPING
    }
}

impl AsNumber for PyUnion {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            or: Some(|a, b, vm| PyUnion::__or__(a.to_owned(), b.to_owned(), vm).to_pyresult(vm)),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

impl Comparable for PyUnion {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        op.eq_only(|| {
            let other = class_or_notimplemented!(Self, other);
            let a = PyFrozenSet::from_iter(vm, zelf.args.into_iter().cloned())?;
            let b = PyFrozenSet::from_iter(vm, other.args.into_iter().cloned())?;
            Ok(PyComparisonValue::Implemented(
                a.into_pyobject(vm).as_object().rich_compare_bool(
                    b.into_pyobject(vm).as_object(),
                    PyComparisonOp::Eq,
                    vm,
                )?,
            ))
        })
    }
}

impl Hashable for PyUnion {
    #[inline]
    fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<hash::PyHash> {
        let set = PyFrozenSet::from_iter(vm, zelf.args.into_iter().cloned())?;
        PyFrozenSet::hash(&set.into_ref(&vm.ctx), vm)
    }
}

impl GetAttr for PyUnion {
    fn getattro(zelf: &Py<Self>, attr: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
        for &exc in CLS_ATTRS {
            if *exc == attr.to_string() {
                return zelf.as_object().generic_getattr(attr, vm);
            }
        }
        zelf.as_object().get_attr(attr, vm)
    }
}

impl Representable for PyUnion {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        zelf.repr(vm)
    }
}

pub fn init(context: &Context) {
    let union_type = &context.types.union_type;
    PyUnion::extend_class(context, union_type);
}
