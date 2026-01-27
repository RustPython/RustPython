use super::{genericalias, type_};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    atomic_func,
    builtins::{PyFrozenSet, PySet, PyStr, PyTuple, PyTupleRef, PyType},
    class::PyClassImpl,
    common::hash,
    convert::ToPyObject,
    function::PyComparisonValue,
    protocol::{PyMappingMethods, PyNumberMethods},
    stdlib::typing::TypeAliasType,
    types::{AsMapping, AsNumber, Comparable, GetAttr, Hashable, PyComparisonOp, Representable},
};
use alloc::fmt;
use std::sync::LazyLock;

const CLS_ATTRS: &[&str] = &["__module__"];

#[pyclass(module = "typing", name = "Union", traverse)]
pub struct PyUnion {
    args: PyTupleRef,
    /// Frozenset of hashable args, or None if all args were hashable
    hashable_args: Option<PyRef<PyFrozenSet>>,
    /// Tuple of initially unhashable args, or None if all args were hashable
    unhashable_args: Option<PyTupleRef>,
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
    /// Create a new union from dedup result (internal use)
    fn from_components(result: UnionComponents, vm: &VirtualMachine) -> PyResult<Self> {
        let parameters = make_parameters(&result.args, vm)?;
        Ok(Self {
            args: result.args,
            hashable_args: result.hashable_args,
            unhashable_args: result.unhashable_args,
            parameters,
        })
    }

    /// Direct access to args field, matching CPython's _Py_union_args
    #[inline]
    pub fn args(&self) -> &Py<PyTuple> {
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
    flags(DISALLOW_INSTANTIATION),
    with(Hashable, Comparable, AsMapping, AsNumber, Representable)
)]
impl PyUnion {
    #[pygetset]
    fn __name__(&self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str("Union").into()
    }

    #[pygetset]
    fn __qualname__(&self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str("Union").into()
    }

    #[pygetset]
    fn __origin__(&self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.types.union_type.to_owned().into()
    }

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

    fn __or__(zelf: PyObjectRef, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        type_::or_(zelf, other, vm)
    }

    #[pymethod]
    fn __mro_entries__(zelf: PyRef<Self>, _args: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error(format!("Cannot subclass {}", zelf.repr(vm)?)))
    }

    #[pyclassmethod]
    fn __class_getitem__(
        _cls: crate::builtins::PyTypeRef,
        args: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        // Convert args to tuple if not already
        let args_tuple = if let Some(tuple) = args.downcast_ref::<PyTuple>() {
            tuple.to_owned()
        } else {
            PyTuple::new_ref(vec![args], &vm.ctx)
        };

        // Check for empty union
        if args_tuple.is_empty() {
            return Err(vm.new_type_error("Cannot create empty Union"));
        }

        // Create union using make_union to properly handle None -> NoneType conversion
        make_union(&args_tuple, vm)
    }
}

pub fn is_unionable(obj: PyObjectRef, vm: &VirtualMachine) -> bool {
    let cls = obj.class();
    cls.is(vm.ctx.types.none_type)
        || obj.downcastable::<PyType>()
        || cls.fast_issubclass(vm.ctx.types.generic_alias_type)
        || cls.is(vm.ctx.types.union_type)
        || obj.downcast_ref::<TypeAliasType>().is_some()
}

fn make_parameters(args: &Py<PyTuple>, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
    let parameters = genericalias::make_parameters(args, vm);
    let result = dedup_and_flatten_args(&parameters, vm)?;
    Ok(result.args)
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
        } else if arg.downcast_ref::<PyStr>().is_some() {
            // Convert string to ForwardRef
            match string_to_forwardref(arg.clone(), vm) {
                Ok(fr) => flattened_args.push(fr),
                Err(_) => flattened_args.push(arg.clone()),
            }
        } else {
            flattened_args.push(arg.clone());
        };
    }

    PyTuple::new_ref(flattened_args, &vm.ctx)
}

fn string_to_forwardref(arg: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    // Import annotationlib.ForwardRef and create a ForwardRef
    let annotationlib = vm.import("annotationlib", 0)?;
    let forwardref_cls = annotationlib.get_attr("ForwardRef", vm)?;
    forwardref_cls.call((arg,), vm)
}

/// Components for creating a PyUnion after deduplication
struct UnionComponents {
    /// All unique args in order
    args: PyTupleRef,
    /// Frozenset of hashable args (for fast equality comparison)
    hashable_args: Option<PyRef<PyFrozenSet>>,
    /// Tuple of unhashable args at creation time (for hash error message)
    unhashable_args: Option<PyTupleRef>,
}

fn dedup_and_flatten_args(args: &Py<PyTuple>, vm: &VirtualMachine) -> PyResult<UnionComponents> {
    let args = flatten_args(args, vm);

    // Use set-based deduplication like CPython:
    // - For hashable elements: use Python's set semantics (hash + equality)
    // - For unhashable elements: use equality comparison
    //
    // This avoids calling __eq__ when hashes differ, matching CPython behavior
    // where `int | BadType` doesn't raise even if BadType.__eq__ raises.

    let mut new_args: Vec<PyObjectRef> = Vec::with_capacity(args.len());

    // Track hashable elements using a Python set (uses hash + equality)
    let hashable_set = PySet::default().into_ref(&vm.ctx);
    let mut hashable_list: Vec<PyObjectRef> = Vec::new();
    let mut unhashable_list: Vec<PyObjectRef> = Vec::new();

    for arg in &*args {
        // Try to hash the element first
        match arg.hash(vm) {
            Ok(_) => {
                // Element is hashable - use set for deduplication
                // Set membership uses hash first, then equality only if hashes match
                let contains = vm
                    .call_method(hashable_set.as_ref(), "__contains__", (arg.clone(),))
                    .and_then(|r| r.try_to_bool(vm))?;
                if !contains {
                    hashable_set.add(arg.clone(), vm)?;
                    hashable_list.push(arg.clone());
                    new_args.push(arg.clone());
                }
            }
            Err(_) => {
                // Element is unhashable - use equality comparison
                let mut is_duplicate = false;
                for existing in &unhashable_list {
                    match existing.rich_compare_bool(arg, PyComparisonOp::Eq, vm) {
                        Ok(true) => {
                            is_duplicate = true;
                            break;
                        }
                        Ok(false) => continue,
                        Err(e) => return Err(e),
                    }
                }
                if !is_duplicate {
                    unhashable_list.push(arg.clone());
                    new_args.push(arg.clone());
                }
            }
        }
    }

    new_args.shrink_to_fit();

    // Create hashable_args frozenset if there are hashable elements
    let hashable_args = if !hashable_list.is_empty() {
        Some(PyFrozenSet::from_iter(vm, hashable_list.into_iter())?.into_ref(&vm.ctx))
    } else {
        None
    };

    // Create unhashable_args tuple if there are unhashable elements
    let unhashable_args = if !unhashable_list.is_empty() {
        Some(PyTuple::new_ref(unhashable_list, &vm.ctx))
    } else {
        None
    };

    Ok(UnionComponents {
        args: PyTuple::new_ref(new_args, &vm.ctx),
        hashable_args,
        unhashable_args,
    })
}

pub fn make_union(args: &Py<PyTuple>, vm: &VirtualMachine) -> PyResult {
    let result = dedup_and_flatten_args(args, vm)?;
    Ok(match result.args.len() {
        1 => result.args[0].to_owned(),
        _ => PyUnion::from_components(result, vm)?.to_pyobject(vm),
    })
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
        let res;
        if new_args.is_empty() {
            res = make_union(&new_args, vm)?;
        } else {
            let mut tmp = new_args[0].to_owned();
            for arg in new_args.iter().skip(1) {
                tmp = vm._or(&tmp, arg)?;
            }
            res = tmp;
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
            or: Some(|a, b, vm| PyUnion::__or__(a.to_owned(), b.to_owned(), vm)),
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

            // Check if lengths are equal
            if zelf.args.len() != other.args.len() {
                return Ok(PyComparisonValue::Implemented(false));
            }

            // Fast path: if both unions have all hashable args, compare frozensets directly
            // Always use Eq here since eq_only handles Ne by negating the result
            if zelf.unhashable_args.is_none()
                && other.unhashable_args.is_none()
                && let (Some(a), Some(b)) = (&zelf.hashable_args, &other.hashable_args)
            {
                let eq = a
                    .as_object()
                    .rich_compare_bool(b.as_object(), PyComparisonOp::Eq, vm)?;
                return Ok(PyComparisonValue::Implemented(eq));
            }

            // Slow path: O(n^2) nested loop comparison for unhashable elements
            // Check if all elements in zelf.args are in other.args
            for arg_a in &*zelf.args {
                let mut found = false;
                for arg_b in &*other.args {
                    match arg_a.rich_compare_bool(arg_b, PyComparisonOp::Eq, vm) {
                        Ok(true) => {
                            found = true;
                            break;
                        }
                        Ok(false) => continue,
                        Err(e) => return Err(e), // Propagate comparison errors
                    }
                }
                if !found {
                    return Ok(PyComparisonValue::Implemented(false));
                }
            }

            // Check if all elements in other.args are in zelf.args (for symmetry)
            for arg_b in &*other.args {
                let mut found = false;
                for arg_a in &*zelf.args {
                    match arg_b.rich_compare_bool(arg_a, PyComparisonOp::Eq, vm) {
                        Ok(true) => {
                            found = true;
                            break;
                        }
                        Ok(false) => continue,
                        Err(e) => return Err(e), // Propagate comparison errors
                    }
                }
                if !found {
                    return Ok(PyComparisonValue::Implemented(false));
                }
            }

            Ok(PyComparisonValue::Implemented(true))
        })
    }
}

impl Hashable for PyUnion {
    #[inline]
    fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<hash::PyHash> {
        // If there are any unhashable args from creation time, the union is unhashable
        if let Some(ref unhashable_args) = zelf.unhashable_args {
            let n = unhashable_args.len();
            // Try to hash each previously unhashable arg to get an error
            for arg in unhashable_args.iter() {
                arg.hash(vm)?;
            }
            // All previously unhashable args somehow became hashable
            // But still raise an error to maintain consistent hashing
            return Err(vm.new_type_error(format!(
                "union contains {} unhashable element{}",
                n,
                if n > 1 { "s" } else { "" }
            )));
        }

        // If we have a stored frozenset of hashable args, use that
        if let Some(ref hashable_args) = zelf.hashable_args {
            return PyFrozenSet::hash(hashable_args, vm);
        }

        // Fallback: compute hash from args
        let mut args_to_hash = Vec::new();
        for arg in &*zelf.args {
            match arg.hash(vm) {
                Ok(_) => args_to_hash.push(arg.clone()),
                Err(e) => return Err(e),
            }
        }
        let set = PyFrozenSet::from_iter(vm, args_to_hash.into_iter())?;
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
