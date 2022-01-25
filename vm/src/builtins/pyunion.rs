use crate::{
    builtins::{PyFrozenSet, PyStr, PyStrRef, PyTuple, PyTupleRef, PyTypeRef},
    common::hash,
    function::IntoPyObject,
    protocol::PyMappingMethods,
    types::{AsMapping, Comparable, GetAttr, Hashable, Iterable, PyComparisonOp},
    IdProtocol, PyClassImpl, PyComparisonValue, PyContext, PyObject, PyObjectRef, PyObjectView,
    PyRef, PyResult, PyValue, TryFromObject, TypeProtocol, VirtualMachine,
};
use std::fmt;

use super::genericalias;

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

impl PyValue for PyUnion {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.union_type
    }
}

#[pyimpl(with(Hashable, Comparable, AsMapping), flags(BASETYPE))]
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

        Ok(self
            .args
            .as_slice()
            .iter()
            .map(|o| repr_item(o.clone(), vm))
            .collect::<PyResult<Vec<_>>>()?
            .join(" | "))
    }

    #[pyproperty(magic)]
    fn parameters(&self) -> PyObjectRef {
        self.parameters.as_object().to_owned()
    }

    #[pyproperty(magic)]
    fn args(&self) -> PyObjectRef {
        self.args.as_object().to_owned()
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

fn is_unionable(obj: PyObjectRef, vm: &VirtualMachine) -> bool {
    obj.class().is(&vm.ctx.types.none_type)
        || obj.class().is(&vm.ctx.types.type_type)
        || obj.class().is(&vm.ctx.types.generic_alias_type)
        || obj.class().is(&vm.ctx.types.union_type)
}

pub fn union_type_or(zelf: PyObjectRef, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
    if !is_unionable(zelf.clone(), vm) || !is_unionable(other.clone(), vm) {
        return vm.ctx.not_implemented();
    }

    let tuple = PyTuple::new_ref(vec![zelf, other], &vm.ctx);
    make_union(tuple, vm)
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
    let mut parameters: Vec<PyObjectRef> = Vec::with_capacity(args.len());
    for arg in args.as_slice() {
        if is_typevar(arg) {
            if !parameters.iter().any(|param| param.is(arg)) {
                parameters.push(arg.clone());
            }
        } else if let Ok(subparams) = arg
            .clone()
            .get_attr("__parameters__", vm)
            .and_then(|obj| PyTupleRef::try_from_object(vm, obj))
        {
            for subparam in subparams.as_slice() {
                if !parameters.iter().any(|param| param.is(subparam)) {
                    parameters.push(subparam.clone());
                }
            }
        }
    }
    parameters.shrink_to_fit();

    PyTuple::new_ref(parameters, &vm.ctx)
}

fn flatten_args(args: PyTupleRef, vm: &VirtualMachine) -> PyTupleRef {
    let mut total_args = 0;
    for arg in args.as_slice() {
        if let Some(pyref) = arg.downcast_ref::<PyUnion>() {
            total_args += pyref.args.len();
        } else {
            total_args += 1;
        };
    }

    let mut flattened_args = Vec::with_capacity(total_args);
    for arg in args.as_slice() {
        if let Some(pyref) = arg.downcast_ref::<PyUnion>() {
            for arg in pyref.args.as_slice() {
                flattened_args.push(arg.clone());
            }
        } else {
            flattened_args.push(arg.clone());
        };
    }

    PyTuple::new_ref(flattened_args, &vm.ctx)
}

fn dedup_and_flatten_args(args: PyTupleRef, vm: &VirtualMachine) -> PyTupleRef {
    let args = flatten_args(args, vm);

    let mut new_args: Vec<PyObjectRef> = Vec::with_capacity(args.len());
    for arg in args.as_slice() {
        if !new_args.iter().any(|param| {
            match (
                PyTypeRef::try_from_object(vm, param.clone()),
                PyTypeRef::try_from_object(vm, arg.clone()),
            ) {
                (Ok(a), Ok(b))
                    if a.is(&vm.ctx.types.generic_alias_type)
                        && b.is(&vm.ctx.types.generic_alias_type) =>
                {
                    param
                        .rich_compare_bool(arg, PyComparisonOp::Eq, vm)
                        .ok()
                        .unwrap()
                }
                _ => param.is(arg),
            }
        }) {
            new_args.push(arg.clone());
        }
    }

    new_args.shrink_to_fit();

    PyTuple::new_ref(new_args, &vm.ctx)
}

fn make_union(args: PyTupleRef, vm: &VirtualMachine) -> PyObjectRef {
    let args = dedup_and_flatten_args(args, vm);
    match args.len() {
        1 => args.fast_getitem(0),
        _ => PyUnion::new(args, vm).into_pyobject(vm),
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
            for arg in new_args.as_slice().iter().skip(1) {
                res = vm._or(&res, arg)?;
            }
        }

        Ok(res)
    }

    const MAPPING_METHODS: PyMappingMethods = PyMappingMethods {
        length: None,
        subscript: Some(|mapping, needle, vm| {
            Self::mapping_downcast(mapping).getitem(needle.to_owned(), vm)
        }),
        ass_subscript: None,
    };
}

impl AsMapping for PyUnion {
    fn as_mapping(_zelf: &PyObjectView<Self>, _vm: &VirtualMachine) -> PyMappingMethods {
        Self::MAPPING_METHODS
    }
}

impl Comparable for PyUnion {
    fn cmp(
        zelf: &crate::PyObjectView<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        op.eq_only(|| {
            let other = class_or_notimplemented!(Self, other);
            Ok(PyComparisonValue::Implemented(
                zelf.args()
                    .rich_compare_bool(other.args().as_ref(), PyComparisonOp::Eq, vm)?,
            ))
        })
    }
}

impl Hashable for PyUnion {
    #[inline]
    fn hash(zelf: &crate::PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<hash::PyHash> {
        let it = PyTuple::iter(zelf.args.clone(), vm);
        let set = PyFrozenSet::from_iter(vm, it)?;
        PyFrozenSet::hash(&set.into_ref(vm), vm)
    }
}

impl GetAttr for PyUnion {
    fn getattro(zelf: PyRef<Self>, attr: PyStrRef, vm: &VirtualMachine) -> PyResult {
        for exc in CLS_ATTRS.iter() {
            if *(*exc) == attr.to_string() {
                return vm.generic_getattribute(zelf.as_object().to_owned(), attr);
            }
        }
        zelf.as_object().into_pyobject(vm).get_attr(attr, vm)
    }
}

pub fn init(context: &PyContext) {
    let union_type = &context.types.union_type;
    PyUnion::extend_class(context, union_type);
}
