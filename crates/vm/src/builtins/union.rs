use super::{dict::contextualize_hash_error, genericalias};
use crate::common::lock::{LazyLock, PyMutex};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    atomic_func,
    builtins::{PyFrozenSet, PySet, PyStr, PyTuple, PyTupleRef, PyType},
    class::PyClassImpl,
    common::hash,
    convert::ToPyObject,
    function::PyComparisonValue,
    protocol::{PyMappingMethods, PyNumberMethods},
    stdlib::_typing::{TypeAliasType, call_typing_func_object},
    types::{AsMapping, AsNumber, Comparable, GetAttr, Hashable, PyComparisonOp},
};
use alloc::fmt;

const CLS_ATTRS: &[&str] = &["__module__"];

#[pyclass(module = "typing", name = "Union", traverse)]
pub struct PyUnion {
    #[pymember(name = "__args__")]
    args: PyTupleRef,
    /// Frozenset of args that were hashable at creation time
    hashable_args: PyRef<PyFrozenSet>,
    /// Tuple of initially unhashable args, or None if all args were hashable
    unhashable_args: Option<PyTupleRef>,
    parameters: PyMutex<Option<PyTupleRef>>,
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
    /// Direct access to args field (_Py_union_args)
    #[inline]
    #[must_use]
    pub fn args(&self) -> &Py<PyTuple> {
        &self.args
    }

    fn repr(&self, vm: &VirtualMachine) -> PyResult<String> {
        fn repr_item(obj: &PyObject, vm: &VirtualMachine) -> PyResult<String> {
            if obj.is(vm.ctx.types.none_type) {
                return Ok("None".to_string());
            }

            if vm
                .get_attribute_opt(obj, identifier!(vm, __origin__))?
                .is_some()
                && vm
                    .get_attribute_opt(obj, identifier!(vm, __args__))?
                    .is_some()
            {
                return Ok(obj.repr(vm)?.to_string());
            }

            match (
                vm.get_attribute_opt(obj, identifier!(vm, __qualname__))?
                    .and_then(|o| o.downcast_ref::<PyStr>().map(|n| n.to_string())),
                vm.get_attribute_opt(obj, identifier!(vm, __module__))?
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
            .as_slice()
            .iter()
            .map(|o| repr_item(o, vm))
            .collect::<PyResult<Vec<_>>>()?
            .join(" | "))
    }
}

impl PyUnion {
    fn __or__(zelf: PyObjectRef, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let mut builder = UnionBuilder::new(true);
        builder.add(&zelf, vm)?;
        builder.add(&other, vm)?;
        builder.finish(vm)
    }
}

#[pyclass(
    flags(DISALLOW_INSTANTIATION, HAS_WEAKREF),
    with(Hashable, Comparable, AsMapping, AsNumber)
)]
impl Py<PyUnion> {
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
    fn __parameters__(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        genericalias::cached_parameters(&self.parameters, &self.args, vm)
    }

    #[pymethod]
    fn __instancecheck__(
        zelf: PyRef<PyUnion>,
        obj: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        if zelf
            .args
            .as_slice()
            .iter()
            .any(|x| x.class().is(vm.ctx.types.generic_alias_type))
        {
            Err(vm.new_type_error("isinstance() argument 2 cannot be a parameterized generic"))
        } else {
            obj.is_instance(zelf.args.as_object(), vm)
        }
    }

    #[pymethod]
    fn __subclasscheck__(
        zelf: PyRef<PyUnion>,
        obj: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        if zelf
            .args
            .as_slice()
            .iter()
            .any(|x| x.class().is(vm.ctx.types.generic_alias_type))
        {
            Err(vm.new_type_error("issubclass() argument 2 cannot be a parameterized generic"))
        } else {
            obj.is_subclass(zelf.args.as_object(), vm)
        }
    }

    #[pymethod]
    fn __mro_entries__(
        zelf: PyRef<PyUnion>,
        _object: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        Err(vm.new_type_error(format!("Cannot subclass {}", zelf.repr(vm)?)))
    }

    #[pyclassmethod]
    fn __class_getitem__(
        _cls: crate::builtins::PyTypeRef,
        object: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        let mut builder = UnionBuilder::new(true);
        if let Some(tuple) = object.downcast_ref_if_exact::<PyTuple>(vm) {
            builder.add_tuple(tuple, vm)?;
        } else {
            builder.add(&object, vm)?;
        }
        builder.finish(vm)
    }

    #[inline]
    #[pymethod]
    fn __repr__(zelf: &Self, vm: &VirtualMachine) -> PyResult<String> {
        zelf.repr(vm)
    }
}

fn is_unionable(obj: &PyObject, vm: &VirtualMachine) -> bool {
    let cls = obj.class();
    vm.is_none(obj)
        || cls.is(vm.ctx.types.sentinel_type)
        || obj.downcastable::<PyType>()
        || cls.fast_issubclass(vm.ctx.types.generic_alias_type)
        || cls.is(vm.ctx.types.union_type)
        || obj.downcast_ref::<TypeAliasType>().is_some()
}

fn type_check(arg: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
    // Fast path to avoid calling into typing.py
    if is_unionable(&arg, vm) {
        return Ok(arg);
    }
    let message_str: PyObjectRef = vm
        .ctx
        .new_str("Union[arg, ...]: each arg must be a type.")
        .into();
    call_typing_func_object(vm, "_type_check", (arg, message_str))
}

pub(crate) fn or_op(zelf: PyObjectRef, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    if !is_unionable(&zelf, vm) || !is_unionable(&other, vm) {
        return Ok(vm.ctx.not_implemented());
    }

    // Type, GenericAlias, Sentinel and TypeAliasType have already checked both
    // operands. Unlike Union.__or__, their slots do not recheck flattened args.
    let mut builder = UnionBuilder::new(false);
    builder.add(&zelf, vm)?;
    builder.add(&other, vm)?;
    builder.finish(vm)
}

fn contains_arg(args: &[PyObjectRef], needle: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
    for arg in args {
        if vm.identical_or_equal(arg, needle)? {
            return Ok(true);
        }
    }
    Ok(false)
}

struct UnionBuilder {
    args: Vec<PyObjectRef>,
    hashable_args: PySet,
    unhashable_args: Vec<PyObjectRef>,
    is_checked: bool,
}

impl Drop for UnionBuilder {
    fn drop(&mut self) {
        // Match list cleanup order, keeping the cached set alive until all
        // ordered args have been released and unhashable args alive until last.
        for arg in self.args.drain(..).rev() {
            drop(arg);
        }
        drop(core::mem::take(&mut self.hashable_args));
        for arg in self.unhashable_args.drain(..).rev() {
            drop(arg);
        }
    }
}

impl UnionBuilder {
    fn new(is_checked: bool) -> Self {
        Self {
            args: Vec::new(),
            hashable_args: PySet::default(),
            unhashable_args: Vec::new(),
            is_checked,
        }
    }

    fn add_tuple(&mut self, args: &Py<PyTuple>, vm: &VirtualMachine) -> PyResult<()> {
        for arg in args {
            self.add(arg, vm)?;
        }
        Ok(())
    }

    fn add(&mut self, arg: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
        let arg = if vm.is_none(arg) {
            vm.ctx.types.none_type.as_object()
        } else if let Some(union) = arg.downcast_ref::<PyUnion>() {
            return self.add_tuple(&union.args, vm);
        } else {
            arg
        };

        if self.is_checked {
            // Keep the original input borrowed and the checked result owned
            // until insertion finishes, including when checking replaces it.
            let checked = type_check(arg.to_owned(), vm)?;
            self.add_unchecked(&checked, vm)
        } else {
            self.add_unchecked(arg, vm)
        }
    }

    fn add_unchecked(&mut self, arg: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
        // Only the initial hash failure classifies an argument as unhashable.
        // Membership and insertion hash again, wrapping only exact TypeErrors
        // from hashing. Errors from equality must propagate unchanged.
        if arg.hash(vm).is_ok() {
            let key_hash = || {
                arg.hash(vm)
                    .map_err(|exc| contextualize_hash_error(arg, "set", "element", exc, vm))
            };
            let hash = key_hash()?;
            if self
                .hashable_args
                .inner
                .contains_known_hash(arg, hash, vm)?
            {
                return Ok(());
            }
            let hash = key_hash()?;
            self.hashable_args.inner.insert_known_hash(arg, hash, vm)?;
        } else {
            if contains_arg(&self.unhashable_args, arg, vm)? {
                return Ok(());
            }
            self.unhashable_args.push(arg.to_owned());
        }
        self.args.push(arg.to_owned());
        Ok(())
    }

    fn finish(mut self, vm: &VirtualMachine) -> PyResult {
        match self.args.len() {
            0 => Err(vm.new_type_error("Cannot take a Union of no types.")),
            1 => Ok(self.args[0].clone()),
            _ => {
                // Keep even an empty hashable partition, and preserve its stored
                // hashes without another round of user hash/equality callbacks.
                let args = PyTuple::new_ref(core::mem::take(&mut self.args), &vm.ctx);
                let hashable_args = core::mem::take(&mut self.hashable_args)
                    .into_frozen()
                    .into_ref(&vm.ctx);
                let unhashable_args = (!self.unhashable_args.is_empty())
                    .then(|| PyTuple::new_ref(core::mem::take(&mut self.unhashable_args), &vm.ctx));
                Ok(PyUnion {
                    args,
                    hashable_args,
                    unhashable_args,
                    parameters: PyMutex::new(None),
                }
                .to_pyobject(vm))
            }
        }
    }
}

pub fn make_union(args: &Py<PyTuple>, vm: &VirtualMachine) -> PyResult {
    let mut builder = UnionBuilder::new(true);
    builder.add_tuple(args, vm)?;
    builder.finish(vm)
}

impl PyUnion {
    fn getitem(zelf: &Py<Self>, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let parameters = genericalias::cached_parameters(&zelf.parameters, &zelf.args, vm)?;
        let new_args =
            genericalias::subs_parameters(zelf.as_object(), &zelf.args, &parameters, needle, vm)?;

        make_union(&new_args, vm)
    }
}

impl AsMapping for PyUnion {
    fn as_mapping() -> &'static PyMappingMethods {
        static AS_MAPPING: LazyLock<PyMappingMethods> = LazyLock::new(|| PyMappingMethods {
            subscript: atomic_func!(|mapping, needle, vm| {
                let zelf = PyUnion::mapping_downcast(mapping);
                PyUnion::getitem(zelf, needle.to_owned(), vm)
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

            // Creation-time partitions determine equality, even if a member's
            // hashability has changed since either union was constructed.
            if !zelf.hashable_args.as_object().rich_compare_bool(
                other.hashable_args.as_object(),
                PyComparisonOp::Eq,
                vm,
            )? {
                return Ok(PyComparisonValue::Implemented(false));
            }

            match (&zelf.unhashable_args, &other.unhashable_args) {
                (Some(a), Some(b)) => {
                    if a.as_slice().len() != b.as_slice().len() {
                        return Ok(PyComparisonValue::Implemented(false));
                    }
                    // Tuple containment compares the stored member to the
                    // needle, with an identity shortcut, in both directions.
                    for (needles, haystack) in [(a, b), (b, a)] {
                        for arg in &**needles {
                            if !contains_arg(haystack.as_slice(), arg, vm)? {
                                return Ok(PyComparisonValue::Implemented(false));
                            }
                        }
                    }
                }
                (None, None) => {}
                _ => {
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
            let n = unhashable_args.as_slice().len();
            // Try to hash each previously unhashable arg to get an error
            for arg in unhashable_args.as_slice() {
                arg.hash(vm)?;
            }
            // All previously unhashable args somehow became hashable
            // But still raise an error to maintain consistent hashing
            return Err(vm.new_type_error(format!("union contains {n} unhashable elements")));
        }

        PyFrozenSet::hash(&zelf.hashable_args, vm)
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

pub(crate) fn init(context: &'static Context) {
    let union_type = &context.types.union_type;
    PyUnion::extend_class(context, union_type);
}
