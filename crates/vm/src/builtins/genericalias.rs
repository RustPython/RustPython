// spell-checker:ignore iparam gaiterobject
use crate::common::lock::LazyLock;

use super::type_;
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
    VirtualMachine, atomic_func,
    builtins::{PyList, PyStr, PyTuple, PyTupleRef, PyType},
    class::PyClassImpl,
    common::hash,
    convert::ToPyObject,
    function::{FuncArgs, PyComparisonValue},
    protocol::{PyMappingMethods, PyNumberMethods},
    types::{
        AsMapping, AsNumber, Callable, Comparable, Constructor, GetAttr, Hashable, IterNext,
        Iterable, PyComparisonOp, Representable,
    },
};
use alloc::fmt;

// Attributes that are looked up on the GenericAlias itself, not on __origin__
static ATTR_EXCEPTIONS: [&str; 9] = [
    "__class__",
    "__origin__",
    "__args__",
    "__unpacked__",
    "__parameters__",
    "__typing_unpacked_tuple_args__",
    "__mro_entries__",
    "__reduce_ex__", // needed so we don't look up object.__reduce_ex__
    "__reduce__",
];

// Attributes that are blocked from being looked up on __origin__
static ATTR_BLOCKED: [&str; 3] = ["__bases__", "__copy__", "__deepcopy__"];

#[pyclass(module = "types", name = "GenericAlias")]
pub struct PyGenericAlias {
    origin: PyObjectRef,
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

    fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
        if !args.kwargs.is_empty() {
            return Err(vm.new_type_error("GenericAlias() takes no keyword arguments"));
        }
        let (origin, arguments): (PyObjectRef, PyObjectRef) = args.bind(vm)?;
        let args = if let Ok(tuple) = arguments.try_to_ref::<PyTuple>(vm) {
            tuple.to_owned()
        } else {
            PyTuple::new_ref(vec![arguments], &vm.ctx)
        };
        Ok(Self::new(origin, args, false, vm))
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
    pub fn new(
        origin: impl Into<PyObjectRef>,
        args: PyTupleRef,
        starred: bool,
        vm: &VirtualMachine,
    ) -> Self {
        let parameters = make_parameters(&args, vm);
        Self {
            origin: origin.into(),
            args,
            parameters,
            starred,
        }
    }

    /// Create a GenericAlias from an origin and PyObjectRef arguments (helper for compatibility)
    pub fn from_args(
        origin: impl Into<PyObjectRef>,
        args: PyObjectRef,
        vm: &VirtualMachine,
    ) -> Self {
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

        fn repr_arg(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
            // ParamSpec args can be lists - format their items with repr_item
            if obj.class().is(vm.ctx.types.list_type) {
                let list = obj.downcast_ref::<crate::builtins::PyList>().unwrap();
                let len = list.borrow_vec().len();
                let mut parts = Vec::with_capacity(len);
                // Use indexed access so list mutation during repr causes IndexError
                for i in 0..len {
                    let item =
                        list.borrow_vec().get(i).cloned().ok_or_else(|| {
                            vm.new_index_error("list index out of range".to_owned())
                        })?;
                    parts.push(repr_item(item, vm)?);
                }
                Ok(format!("[{}]", parts.join(", ")))
            } else {
                repr_item(obj, vm)
            }
        }

        let repr_str = format!(
            "{}[{}]",
            repr_item(self.origin.clone(), vm)?,
            if self.args.is_empty() {
                "()".to_owned()
            } else {
                self.args
                    .iter()
                    .map(|o| repr_arg(o.clone(), vm))
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
        self.origin.clone()
    }

    #[pygetset]
    const fn __unpacked__(&self) -> bool {
        self.starred
    }

    #[pygetset]
    fn __typing_unpacked_tuple_args__(&self, vm: &VirtualMachine) -> PyObjectRef {
        if self.starred && self.origin.is(vm.ctx.types.tuple_type.as_object()) {
            self.args.clone().into()
        } else {
            vm.ctx.none()
        }
    }

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
    fn __reduce__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        if zelf.starred {
            // (next, (iter(GenericAlias(origin, args)),))
            let next_fn = vm.builtins.get_attr("next", vm)?;
            let non_starred = Self::new(zelf.origin.clone(), zelf.args.clone(), false, vm);
            let iter_obj = PyGenericAliasIterator {
                obj: crate::common::lock::PyMutex::new(Some(non_starred.into_pyobject(vm))),
            }
            .into_pyobject(vm);
            Ok(PyTuple::new_ref(
                vec![next_fn, PyTuple::new_ref(vec![iter_obj], &vm.ctx).into()],
                &vm.ctx,
            ))
        } else {
            Ok(PyTuple::new_ref(
                vec![
                    vm.ctx.types.generic_alias_type.to_owned().into(),
                    PyTuple::new_ref(vec![zelf.origin.clone(), zelf.args.clone().into()], &vm.ctx)
                        .into(),
                ],
                &vm.ctx,
            ))
        }
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

    fn __ror__(zelf: PyObjectRef, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        type_::or_(other, zelf, vm)
    }

    fn __or__(zelf: PyObjectRef, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        type_::or_(zelf, other, vm)
    }
}

pub(crate) fn make_parameters(args: &Py<PyTuple>, vm: &VirtualMachine) -> PyTupleRef {
    make_parameters_from_slice(args.as_slice(), vm)
}

fn make_parameters_from_slice(args: &[PyObjectRef], vm: &VirtualMachine) -> PyTupleRef {
    let mut parameters: Vec<PyObjectRef> = Vec::with_capacity(args.len());

    for arg in args {
        // We don't want __parameters__ descriptor of a bare Python class.
        if arg.class().is(vm.ctx.types.type_type) {
            continue;
        }

        // Check for __typing_subst__ attribute
        if arg.get_attr(identifier!(vm, __typing_subst__), vm).is_ok() {
            if tuple_index(&parameters, arg).is_none() {
                parameters.push(arg.clone());
            }
        } else if let Ok(subparams) = arg.get_attr(identifier!(vm, __parameters__), vm)
            && let Ok(sub_params) = subparams.try_to_ref::<PyTuple>(vm)
        {
            for sub_param in sub_params {
                if tuple_index(&parameters, sub_param).is_none() {
                    parameters.push(sub_param.clone());
                }
            }
        } else if arg.try_to_ref::<PyTuple>(vm).is_ok() || arg.try_to_ref::<PyList>(vm).is_ok() {
            // Recursively extract parameters from lists/tuples (ParamSpec args)
            let items: Vec<PyObjectRef> = if let Ok(t) = arg.try_to_ref::<PyTuple>(vm) {
                t.as_slice().to_vec()
            } else {
                let list = arg.downcast_ref::<PyList>().unwrap();
                list.borrow_vec().to_vec()
            };
            let sub = make_parameters_from_slice(&items, vm);
            for sub_param in sub.iter() {
                if tuple_index(&parameters, sub_param).is_none() {
                    parameters.push(sub_param.clone());
                }
            }
        }
    }

    PyTuple::new_ref(parameters, &vm.ctx)
}

#[inline]
fn tuple_index(vec: &[PyObjectRef], item: &PyObject) -> Option<usize> {
    vec.iter().position(|element| element.is(item))
}

fn is_unpacked_typevartuple(arg: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
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
    params: &Py<PyTuple>,
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
        if let Ok(sub_args) = item.get_attr(identifier!(vm, __typing_unpacked_tuple_args__), vm)
            && !sub_args.is(&vm.ctx.none)
            && let Ok(tuple) = sub_args.try_to_ref::<PyTuple>(vm)
        {
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
        if let Ok(prepare) = param.get_attr(identifier!(vm, __typing_prepare_subst__), vm)
            && !prepare.is(&vm.ctx.none)
        {
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

    // Step 3: Extract final arg items
    let arg_items = if let Ok(tuple) = item.try_to_ref::<PyTuple>(vm) {
        tuple.as_slice().to_vec()
    } else {
        vec![item.clone()]
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

        // Recursively substitute params in lists/tuples
        let is_list = arg.try_to_ref::<PyList>(vm).is_ok();
        if arg.try_to_ref::<PyTuple>(vm).is_ok() || is_list {
            let sub_items: Vec<PyObjectRef> = if let Ok(t) = arg.try_to_ref::<PyTuple>(vm) {
                t.as_slice().to_vec()
            } else {
                arg.downcast_ref::<PyList>().unwrap().borrow_vec().to_vec()
            };
            let sub_tuple = PyTuple::new_ref(sub_items, &vm.ctx);
            let sub_result = subs_parameters(
                alias.clone(),
                sub_tuple,
                parameters.clone(),
                item.clone(),
                vm,
            )?;
            let substituted: PyObjectRef = if is_list {
                // Convert tuple back to list
                PyList::from(sub_result.as_slice().to_vec())
                    .into_ref(&vm.ctx)
                    .into()
            } else {
                sub_result.into()
            };
            new_args.push(substituted);
            continue;
        }

        // Check if this is an unpacked TypeVarTuple
        let unpack = is_unpacked_typevartuple(arg, vm)?;

        // Try __typing_subst__ method first
        let substituted_arg = if let Ok(subst) = arg.get_attr(identifier!(vm, __typing_subst__), vm)
        {
            if let Some(iparam) = tuple_index(parameters.as_slice(), arg) {
                subst.call((arg_items[iparam].clone(),), vm)?
            } else {
                subs_tvars(arg.clone(), &parameters, &arg_items, vm)?
            }
        } else {
            subs_tvars(arg.clone(), &parameters, &arg_items, vm)?
        };

        if unpack {
            if let Ok(tuple) = substituted_arg.try_to_ref::<PyTuple>(vm) {
                for elem in tuple {
                    new_args.push(elem.clone());
                }
            } else {
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
            or: Some(|a, b, vm| PyGenericAlias::__or__(a.to_owned(), b.to_owned(), vm)),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

impl Callable for PyGenericAlias {
    type Args = FuncArgs;
    fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        zelf.origin.call(args, vm).map(|obj| {
            if let Err(exc) = obj.set_attr(identifier!(vm, __orig_class__), zelf.to_owned(), vm)
                && !exc.fast_isinstance(vm.ctx.exceptions.attribute_error)
                && !exc.fast_isinstance(vm.ctx.exceptions.type_error)
            {
                return Err(exc);
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
            if zelf.starred != other.starred {
                return Ok(PyComparisonValue::Implemented(false));
            }
            Ok(PyComparisonValue::Implemented(
                zelf.__origin__()
                    .rich_compare_bool(&other.__origin__(), PyComparisonOp::Eq, vm)?
                    && zelf.__args__().rich_compare_bool(
                        &other.__args__(),
                        PyComparisonOp::Eq,
                        vm,
                    )?,
            ))
        })
    }
}

impl Hashable for PyGenericAlias {
    #[inline]
    fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<hash::PyHash> {
        Ok(zelf.origin.hash(vm)? ^ zelf.args.as_object().hash(vm)?)
    }
}

impl GetAttr for PyGenericAlias {
    fn getattro(zelf: &Py<Self>, attr: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
        let attr_str = attr.as_str();
        for exc in &ATTR_EXCEPTIONS {
            if *exc == attr_str {
                return zelf.as_object().generic_getattr(attr, vm);
            }
        }
        for blocked in &ATTR_BLOCKED {
            if *blocked == attr_str {
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
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(PyGenericAliasIterator {
            obj: crate::common::lock::PyMutex::new(Some(zelf.into())),
        }
        .into_pyobject(vm))
    }
}

// gaiterobject - yields one starred GenericAlias then exhausts
#[pyclass(module = "types", name = "generic_alias_iterator")]
#[derive(Debug, PyPayload)]
pub struct PyGenericAliasIterator {
    obj: crate::common::lock::PyMutex<Option<PyObjectRef>>,
}

#[pyclass(with(Representable, Iterable, IterNext))]
impl PyGenericAliasIterator {
    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        let iter_fn = vm.builtins.get_attr("iter", vm)?;
        let guard = self.obj.lock();
        let arg: PyObjectRef = if let Some(ref obj) = *guard {
            // Not yet exhausted: (iter, (obj,))
            PyTuple::new_ref(vec![obj.clone()], &vm.ctx).into()
        } else {
            // Exhausted: (iter, ((),))
            let empty = PyTuple::new_ref(vec![], &vm.ctx);
            PyTuple::new_ref(vec![empty.into()], &vm.ctx).into()
        };
        Ok(PyTuple::new_ref(vec![iter_fn, arg], &vm.ctx))
    }
}

impl Representable for PyGenericAliasIterator {
    fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        Ok("<generic_alias_iterator>".to_owned())
    }
}

impl Iterable for PyGenericAliasIterator {
    fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyResult {
        Ok(zelf.into())
    }
}

impl crate::types::IterNext for PyGenericAliasIterator {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<crate::protocol::PyIterReturn> {
        use crate::protocol::PyIterReturn;
        let mut guard = zelf.obj.lock();
        let obj = match guard.take() {
            Some(obj) => obj,
            None => return Ok(PyIterReturn::StopIteration(None)),
        };
        // Create a starred GenericAlias from the original
        let alias = obj.downcast_ref::<PyGenericAlias>().ok_or_else(|| {
            vm.new_type_error("generic_alias_iterator expected GenericAlias".to_owned())
        })?;
        let starred = PyGenericAlias::new(alias.origin.clone(), alias.args.clone(), true, vm);
        Ok(PyIterReturn::Return(starred.into_pyobject(vm)))
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
    PyGenericAlias::extend_class(context, context.types.generic_alias_type);
    PyGenericAliasIterator::extend_class(context, context.types.generic_alias_iterator_type);
}
