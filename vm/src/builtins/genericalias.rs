// spell-checker:ignore iparam
use std::sync::LazyLock;

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
        AsMapping, AsNumber, Callable, Comparable, Constructor, GetAttr, Hashable, Iterable,
        PyComparisonOp, Representable,
    },
};
use std::fmt;

// attr_exceptions
static ATTR_EXCEPTIONS: [&str; 12] = [
    "__class__",
    "__bases__",
    "__origin__",
    "__args__",
    "__unpacked__",
    "__parameters__",
    "__typing_unpacked_tuple_args__",
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
    starred: bool, // for __unpacked__ attribute
}

impl fmt::Debug for PyGenericAlias {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GenericAlias")
    }
}

impl PyPayload for PyGenericAlias {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.generic_alias_type
    }
}

impl Constructor for PyGenericAlias {
    type Args = FuncArgs;

    fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        if !args.kwargs.is_empty() {
            return Err(vm.new_type_error("GenericAlias() takes no keyword arguments"));
        }
        let (origin, arguments): (_, PyObjectRef) = args.bind(vm)?;
        let args = if let Ok(tuple) = arguments.try_to_ref::<PyTuple>(vm) {
            tuple.to_owned()
        } else {
            PyTuple::new_ref(vec![arguments], &vm.ctx)
        };
        Self::new(origin, args, false, vm)
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
        Iterable,
        Representable
    ),
    flags(BASETYPE)
)]
impl PyGenericAlias {
    pub fn new(origin: PyTypeRef, args: PyTupleRef, starred: bool, vm: &VirtualMachine) -> Self {
        let parameters = make_parameters(&args, vm);
        Self {
            origin,
            args,
            parameters,
            starred,
        }
    }

    /// Create a GenericAlias from an origin and PyObjectRef arguments (helper for compatibility)
    pub fn from_args(origin: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> Self {
        let args = if let Ok(tuple) = args.try_to_ref::<PyTuple>(vm) {
            tuple.to_owned()
        } else {
            PyTuple::new_ref(vec![args], &vm.ctx)
        };
        Self::new(origin, args, false, vm)
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

        let repr_str = format!(
            "{}[{}]",
            repr_item(self.origin.clone().into(), vm)?,
            if self.args.is_empty() {
                "()".to_owned()
            } else {
                self.args
                    .iter()
                    .map(|o| repr_item(o.clone(), vm))
                    .collect::<PyResult<Vec<_>>>()?
                    .join(", ")
            }
        );

        // Add * prefix if this is a starred GenericAlias
        Ok(if self.starred {
            format!("*{repr_str}")
        } else {
            repr_str
        })
    }

    #[pygetset]
    fn __parameters__(&self) -> PyObjectRef {
        self.parameters.clone().into()
    }

    #[pygetset]
    fn __args__(&self) -> PyObjectRef {
        self.args.clone().into()
    }

    #[pygetset]
    fn __origin__(&self) -> PyObjectRef {
        self.origin.clone().into()
    }

    #[pygetset]
    const fn __unpacked__(&self) -> bool {
        self.starred
    }

    #[pygetset]
    fn __typing_unpacked_tuple_args__(&self, vm: &VirtualMachine) -> PyObjectRef {
        if self.starred && self.origin.is(vm.ctx.types.tuple_type) {
            self.args.clone().into()
        } else {
            vm.ctx.none()
        }
    }

    #[pymethod]
    fn __getitem__(zelf: PyRef<Self>, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let new_args = subs_parameters(
            zelf.to_owned().into(),
            zelf.args.clone(),
            zelf.parameters.clone(),
            needle,
            vm,
        )?;

        Ok(Self::new(zelf.origin.clone(), new_args, false, vm).into_pyobject(vm))
    }

    #[pymethod]
    fn __dir__(&self, vm: &VirtualMachine) -> PyResult<PyList> {
        let dir = vm.dir(Some(self.__origin__()))?;
        for exc in &ATTR_EXCEPTIONS {
            if !dir.__contains__((*exc).to_pyobject(vm), vm)? {
                dir.append((*exc).to_pyobject(vm));
            }
        }
        Ok(dir)
    }

    #[pymethod]
    fn __reduce__(zelf: &Py<Self>, vm: &VirtualMachine) -> (PyTypeRef, (PyTypeRef, PyTupleRef)) {
        (
            vm.ctx.types.generic_alias_type.to_owned(),
            (zelf.origin.clone(), zelf.args.clone()),
        )
    }

    #[pymethod]
    fn __mro_entries__(&self, _bases: PyObjectRef, vm: &VirtualMachine) -> PyTupleRef {
        PyTuple::new_ref(vec![self.__origin__()], &vm.ctx)
    }

    #[pymethod]
    fn __instancecheck__(_zelf: PyRef<Self>, _obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error("isinstance() argument 2 cannot be a parameterized generic"))
    }

    #[pymethod]
    fn __subclasscheck__(_zelf: PyRef<Self>, _obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error("issubclass() argument 2 cannot be a parameterized generic"))
    }

    #[pymethod]
    fn __ror__(zelf: PyObjectRef, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        type_::or_(other, zelf, vm)
    }

    #[pymethod]
    fn __or__(zelf: PyObjectRef, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        type_::or_(zelf, other, vm)
    }
}

pub(crate) fn make_parameters(args: &Py<PyTuple>, vm: &VirtualMachine) -> PyTupleRef {
    let mut parameters: Vec<PyObjectRef> = Vec::with_capacity(args.len());
    let mut iparam = 0;

    for arg in args {
        // We don't want __parameters__ descriptor of a bare Python class.
        if arg.class().is(vm.ctx.types.type_type) {
            continue;
        }

        // Check for __typing_subst__ attribute
        if arg.get_attr(identifier!(vm, __typing_subst__), vm).is_ok() {
            // Use tuple_add equivalent logic
            if tuple_index(&parameters, arg).is_none() {
                if iparam >= parameters.len() {
                    parameters.resize(iparam + 1, vm.ctx.none());
                }
                parameters[iparam] = arg.clone();
                iparam += 1;
            }
        } else if let Ok(subparams) = arg.get_attr(identifier!(vm, __parameters__), vm) {
            if let Ok(sub_params) = subparams.try_to_ref::<PyTuple>(vm) {
                let len2 = sub_params.len();
                // Resize if needed
                if iparam + len2 > parameters.len() {
                    parameters.resize(iparam + len2, vm.ctx.none());
                }
                for sub_param in sub_params {
                    // Use tuple_add equivalent logic
                    if tuple_index(&parameters[..iparam], sub_param).is_none() {
                        if iparam >= parameters.len() {
                            parameters.resize(iparam + 1, vm.ctx.none());
                        }
                        parameters[iparam] = sub_param.clone();
                        iparam += 1;
                    }
                }
            }
        }
    }

    // Resize to actual size
    parameters.truncate(iparam);
    PyTuple::new_ref(parameters, &vm.ctx)
}

#[inline]
fn tuple_index(vec: &[PyObjectRef], item: &PyObjectRef) -> Option<usize> {
    vec.iter().position(|element| element.is(item))
}

fn is_unpacked_typevartuple(arg: &PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
    if arg.class().is(vm.ctx.types.type_type) {
        return Ok(false);
    }

    if let Ok(attr) = arg.get_attr(identifier!(vm, __typing_is_unpacked_typevartuple__), vm) {
        attr.try_to_bool(vm)
    } else {
        Ok(false)
    }
}

fn subs_tvars(
    obj: PyObjectRef,
    params: &PyTupleRef,
    arg_items: &[PyObjectRef],
    vm: &VirtualMachine,
) -> PyResult {
    obj.get_attr(identifier!(vm, __parameters__), vm)
        .ok()
        .and_then(|sub_params| {
            PyTupleRef::try_from_object(vm, sub_params)
                .ok()
                .filter(|sub_params| !sub_params.is_empty())
                .map(|sub_params| {
                    let mut sub_args = Vec::new();

                    for arg in sub_params.iter() {
                        if let Some(idx) = tuple_index(params.as_slice(), arg) {
                            let param = &params[idx];
                            let substituted_arg = &arg_items[idx];

                            // Check if this is a TypeVarTuple (has tp_iter)
                            if param.class().slots.iter.load().is_some()
                                && substituted_arg.try_to_ref::<PyTuple>(vm).is_ok()
                            {
                                // TypeVarTuple case - extend with tuple elements
                                if let Ok(tuple) = substituted_arg.try_to_ref::<PyTuple>(vm) {
                                    for elem in tuple {
                                        sub_args.push(elem.clone());
                                    }
                                    continue;
                                }
                            }

                            sub_args.push(substituted_arg.clone());
                        } else {
                            sub_args.push(arg.clone());
                        }
                    }

                    let sub_args: PyObjectRef = PyTuple::new_ref(sub_args, &vm.ctx).into();
                    obj.get_item(&*sub_args, vm)
                })
        })
        .unwrap_or(Ok(obj))
}

// CPython's _unpack_args equivalent
fn unpack_args(item: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
    let mut new_args = Vec::new();

    let arg_items = if let Ok(tuple) = item.try_to_ref::<PyTuple>(vm) {
        tuple.as_slice().to_vec()
    } else {
        vec![item]
    };

    for item in arg_items {
        // Skip PyType objects - they can't be unpacked
        if item.class().is(vm.ctx.types.type_type) {
            new_args.push(item);
            continue;
        }

        // Try to get __typing_unpacked_tuple_args__
        if let Ok(sub_args) = item.get_attr(identifier!(vm, __typing_unpacked_tuple_args__), vm) {
            if !sub_args.is(&vm.ctx.none) {
                if let Ok(tuple) = sub_args.try_to_ref::<PyTuple>(vm) {
                    // Check for ellipsis at the end
                    let has_ellipsis_at_end = tuple
                        .as_slice()
                        .last()
                        .is_some_and(|item| item.is(&vm.ctx.ellipsis));

                    if !has_ellipsis_at_end {
                        // Safe to unpack - add all elements's PyList_SetSlice
                        for arg in tuple {
                            new_args.push(arg.clone());
                        }
                        continue;
                    }
                }
            }
        }

        // Default case: add the item as-is's PyList_Append
        new_args.push(item);
    }

    Ok(PyTuple::new_ref(new_args, &vm.ctx))
}

// _Py_subs_parameters
pub fn subs_parameters(
    alias: PyObjectRef, // = self
    args: PyTupleRef,
    parameters: PyTupleRef,
    item: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<PyTupleRef> {
    let n_params = parameters.len();
    if n_params == 0 {
        return Err(vm.new_type_error(format!("{} is not a generic class", alias.repr(vm)?)));
    }

    // Step 1: Unpack args
    let mut item: PyObjectRef = unpack_args(item, vm)?.into();

    // Step 2: Call __typing_prepare_subst__ on each parameter
    for param in parameters.iter() {
        if let Ok(prepare) = param.get_attr(identifier!(vm, __typing_prepare_subst__), vm) {
            if !prepare.is(&vm.ctx.none) {
                // Call prepare(self, item)
                item = if item.try_to_ref::<PyTuple>(vm).is_ok() {
                    prepare.call((alias.clone(), item.clone()), vm)?
                } else {
                    // Create a tuple with the single item's "O(O)" format
                    let tuple_args = PyTuple::new_ref(vec![item.clone()], &vm.ctx);
                    prepare.call((alias.clone(), tuple_args.to_pyobject(vm)), vm)?
                };
            }
        }
    }

    // Step 3: Extract final arg items
    let arg_items = if let Ok(tuple) = item.try_to_ref::<PyTuple>(vm) {
        tuple.as_slice().to_vec()
    } else {
        vec![item]
    };
    let n_items = arg_items.len();

    if n_items != n_params {
        return Err(vm.new_type_error(format!(
            "Too {} arguments for {}; actual {}, expected {}",
            if n_items > n_params { "many" } else { "few" },
            alias.repr(vm)?,
            n_items,
            n_params
        )));
    }

    // Step 4: Replace all type variables
    let mut new_args = Vec::new();

    for arg in args.iter() {
        // Skip PyType objects
        if arg.class().is(vm.ctx.types.type_type) {
            new_args.push(arg.clone());
            continue;
        }

        // Check if this is an unpacked TypeVarTuple's _is_unpacked_typevartuple
        let unpack = is_unpacked_typevartuple(arg, vm)?;

        // Try __typing_subst__ method first,
        let substituted_arg = if let Ok(subst) = arg.get_attr(identifier!(vm, __typing_subst__), vm)
        {
            // Find parameter index's tuple_index
            if let Some(iparam) = tuple_index(parameters.as_slice(), arg) {
                subst.call((arg_items[iparam].clone(),), vm)?
            } else {
                // This shouldn't happen in well-formed generics but handle gracefully
                subs_tvars(arg.clone(), &parameters, &arg_items, vm)?
            }
        } else {
            // Use subs_tvars for objects with __parameters__
            subs_tvars(arg.clone(), &parameters, &arg_items, vm)?
        };

        if unpack {
            // Handle unpacked TypeVarTuple's tuple_extend
            if let Ok(tuple) = substituted_arg.try_to_ref::<PyTuple>(vm) {
                for elem in tuple {
                    new_args.push(elem.clone());
                }
            } else {
                // This shouldn't happen but handle gracefully
                new_args.push(substituted_arg);
            }
        } else {
            new_args.push(substituted_arg);
        }
    }

    Ok(PyTuple::new_ref(new_args, &vm.ctx))
}

impl AsMapping for PyGenericAlias {
    fn as_mapping() -> &'static PyMappingMethods {
        static AS_MAPPING: LazyLock<PyMappingMethods> = LazyLock::new(|| PyMappingMethods {
            subscript: atomic_func!(|mapping, needle, vm| {
                let zelf = PyGenericAlias::mapping_downcast(mapping);
                PyGenericAlias::__getitem__(zelf.to_owned(), needle.to_owned(), vm)
            }),
            ..PyMappingMethods::NOT_IMPLEMENTED
        });
        &AS_MAPPING
    }
}

impl AsNumber for PyGenericAlias {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            or: Some(|a, b, vm| Ok(PyGenericAlias::__or__(a.to_owned(), b.to_owned(), vm))),
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
                if !zelf.__origin__().rich_compare_bool(
                    &other.__origin__(),
                    PyComparisonOp::Eq,
                    vm,
                )? {
                    false
                } else {
                    zelf.__args__()
                        .rich_compare_bool(&other.__args__(), PyComparisonOp::Eq, vm)?
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
        for exc in &ATTR_EXCEPTIONS {
            if *(*exc) == attr.to_string() {
                return zelf.as_object().generic_getattr(attr, vm);
            }
        }
        zelf.__origin__().get_attr(attr, vm)
    }
}

impl Representable for PyGenericAlias {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        zelf.repr(vm)
    }
}

impl Iterable for PyGenericAlias {
    // ga_iter
    // spell-checker:ignore gaiterobject
    // TODO: gaiterobject
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        // CPython's ga_iter creates an iterator that yields one starred GenericAlias
        // we don't have gaiterobject yet

        let starred_alias = Self::new(
            zelf.origin.clone(),
            zelf.args.clone(),
            true, // starred
            vm,
        );
        let starred_ref = PyRef::new_ref(
            starred_alias,
            vm.ctx.types.generic_alias_type.to_owned(),
            None,
        );
        let items = vec![starred_ref.into()];
        let iter_tuple = PyTuple::new_ref(items, &vm.ctx);
        Ok(iter_tuple.to_pyobject(vm).get_iter(vm)?.into())
    }
}

/// Creates a GenericAlias from type parameters, equivalent to CPython's _Py_subscript_generic
/// This is used for PEP 695 classes to create Generic[T] from type parameters
// _Py_subscript_generic
pub fn subscript_generic(type_params: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    // Get typing module and _GenericAlias
    let typing_module = vm.import("typing", 0)?;
    let generic_type = typing_module.get_attr("Generic", vm)?;

    // Call typing._GenericAlias(Generic, type_params)
    let generic_alias_class = typing_module.get_attr("_GenericAlias", vm)?;

    let args = if let Ok(tuple) = type_params.try_to_ref::<PyTuple>(vm) {
        tuple.to_owned()
    } else {
        PyTuple::new_ref(vec![type_params], &vm.ctx)
    };

    // Create _GenericAlias instance
    generic_alias_class.call((generic_type, args.to_pyobject(vm)), vm)
}

pub fn init(context: &Context) {
    let generic_alias_type = &context.types.generic_alias_type;
    PyGenericAlias::extend_class(context, generic_alias_type);
}
