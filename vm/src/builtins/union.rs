use super::{genericalias, type_};
use crate::{
    builtins::{PyFrozenSet, PyStr, PyStrRef, PyTuple, PyTupleRef, PyType},
    class::PyClassImpl,
    common::hash,
    convert::ToPyObject,
    function::PyComparisonValue,
    protocol::PyMappingMethods,
    types::{AsMapping, Comparable, GetAttr, Hashable, PyComparisonOp},
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
    VirtualMachine,
};
use std::fmt;

const CLS_ATTRS: &[&str] = &["__module__"];

#[pyclass(module = "types", name = "UnionType")]
pub struct PyUnion {
    args: PyTupleRef,
    parameters: PyTupleRef,
}

impl fmt::Debug for PyUnion {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("UnionObject")
    }
}

impl PyPayload for PyUnion {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.union_type
    }
}

#[pyclass(with(Hashable, Comparable, AsMapping), flags(BASETYPE))]
impl PyUnion {
    pub fn new(args: PyTupleRef, vm: &VirtualMachine) -> Self {
        let parameters = make_parameters(&args, vm);
        Self { args, parameters }
    }

    #[pymethod(magic)]
    fn repr(&self, vm: &VirtualMachine) -> PyResult<String> {
        fn repr_item(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
            if vm.is_none(&obj) {
                return Ok("None".to_string());
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
                    format!("{}.{}", module, qualname)
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

    #[pygetset(magic)]
    fn parameters(&self) -> PyObjectRef {
        self.parameters.clone().into()
    }

    #[pygetset(magic)]
    fn args(&self) -> PyObjectRef {
        self.args.clone().into()
    }

    #[pymethod(magic)]
    fn instancecheck(zelf: PyRef<Self>, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        if zelf
            .args
            .iter()
            .any(|x| x.class().is(vm.ctx.types.generic_alias_type))
        {
            Err(vm.new_type_error(
                "isinstance() argument 2 cannot be a parameterized generic".to_owned(),
            ))
        } else {
            obj.is_instance(zelf.args().as_object(), vm)
        }
    }

    #[pymethod(magic)]
    fn subclasscheck(zelf: PyRef<Self>, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        if zelf
            .args
            .iter()
            .any(|x| x.class().is(vm.ctx.types.generic_alias_type))
        {
            Err(vm.new_type_error(
                "issubclass() argument 2 cannot be a parameterized generic".to_owned(),
            ))
        } else {
            obj.is_subclass(zelf.args().as_object(), vm)
        }
    }

    #[pymethod(name = "__ror__")]
    #[pymethod(magic)]
    fn or(zelf: PyObjectRef, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        type_::or_(zelf, other, vm)
    }
}

pub fn is_unionable(obj: PyObjectRef, vm: &VirtualMachine) -> bool {
    obj.class().is(vm.ctx.types.none_type)
        || obj.payload_if_subclass::<PyType>(vm).is_some()
        || obj.class().is(vm.ctx.types.generic_alias_type)
        || obj.class().is(vm.ctx.types.union_type)
}

fn is_typevar(obj: &PyObjectRef, vm: &VirtualMachine) -> bool {
    let class = obj.class();
    class.slot_name() == "TypeVar"
        && class
            .get_attr(identifier!(vm, __module__))
            .and_then(|o| o.downcast_ref::<PyStr>().map(|s| s.as_str() == "typing"))
            .unwrap_or(false)
}

fn make_parameters(args: &PyTupleRef, vm: &VirtualMachine) -> PyTupleRef {
    let mut parameters: Vec<PyObjectRef> = Vec::with_capacity(args.len());
    for arg in args {
        if is_typevar(arg, vm) {
            if !parameters.iter().any(|param| param.is(arg)) {
                parameters.push(arg.clone());
            }
        } else if let Ok(subparams) = arg
            .clone()
            .get_attr(identifier!(vm, __parameters__), vm)
            .and_then(|obj| PyTupleRef::try_from_object(vm, obj))
        {
            for subparam in &subparams {
                if !parameters.iter().any(|param| param.is(subparam)) {
                    parameters.push(subparam.clone());
                }
            }
        }
    }
    parameters.shrink_to_fit();

    dedup_and_flatten_args(PyTuple::new_ref(parameters, &vm.ctx), vm)
}

fn flatten_args(args: PyTupleRef, vm: &VirtualMachine) -> PyTupleRef {
    let mut total_args = 0;
    for arg in &args {
        if let Some(pyref) = arg.downcast_ref::<PyUnion>() {
            total_args += pyref.args.len();
        } else {
            total_args += 1;
        };
    }

    let mut flattened_args = Vec::with_capacity(total_args);
    for arg in &args {
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

fn dedup_and_flatten_args(args: PyTupleRef, vm: &VirtualMachine) -> PyTupleRef {
    let args = flatten_args(args, vm);

    let mut new_args: Vec<PyObjectRef> = Vec::with_capacity(args.len());
    for arg in &args {
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

pub fn make_union(args: PyTupleRef, vm: &VirtualMachine) -> PyObjectRef {
    let args = dedup_and_flatten_args(args, vm);
    match args.len() {
        1 => args.fast_getitem(0),
        _ => PyUnion::new(args, vm).to_pyobject(vm),
    }
}

impl PyUnion {
    fn getitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let new_args = genericalias::subs_parameters(
            |vm| self.repr(vm),
            self.args.clone(),
            self.parameters.clone(),
            needle,
            vm,
        )?;
        let mut res;
        if new_args.len() == 0 {
            res = make_union(new_args, vm);
        } else {
            res = new_args.fast_getitem(0);
            for arg in new_args.iter().skip(1) {
                res = vm._or(&res, arg)?;
            }
        }

        Ok(res)
    }
}

impl AsMapping for PyUnion {
    const AS_MAPPING: PyMappingMethods = PyMappingMethods {
        length: None,
        subscript: Some(|mapping, needle, vm| {
            Self::mapping_downcast(mapping).getitem(needle.to_owned(), vm)
        }),
        ass_subscript: None,
    };
}

impl Comparable for PyUnion {
    fn cmp(
        zelf: &crate::Py<Self>,
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
    fn hash(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<hash::PyHash> {
        let set = PyFrozenSet::from_iter(vm, zelf.args.into_iter().cloned())?;
        PyFrozenSet::hash(&set.into_ref(vm), vm)
    }
}

impl GetAttr for PyUnion {
    fn getattro(zelf: &Py<Self>, attr: PyStrRef, vm: &VirtualMachine) -> PyResult {
        for &exc in CLS_ATTRS {
            if *exc == attr.to_string() {
                return zelf.as_object().generic_getattr(attr, vm);
            }
        }
        zelf.as_object().to_pyobject(vm).get_attr(attr, vm)
    }
}

pub fn init(context: &Context) {
    let union_type = &context.types.union_type;
    PyUnion::extend_class(context, union_type);
}
