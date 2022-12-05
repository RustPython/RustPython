pub(crate) use _operator::make_module;

#[pymodule]
mod _operator {
    use crate::common::cmp;
    use crate::{
        builtins::{PyInt, PyIntRef, PyStrRef, PyTupleRef, PyTypeRef},
        function::Either,
        function::{ArgBytesLike, FuncArgs, KwArgs, OptionalArg},
        identifier,
        protocol::PyIter,
        recursion::ReprGuard,
        types::{Callable, Constructor, PyComparisonOp},
        AsObject, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    };

    #[pyfunction]
    fn lt(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        a.rich_compare(b, PyComparisonOp::Lt, vm)
    }

    #[pyfunction]
    fn le(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        a.rich_compare(b, PyComparisonOp::Le, vm)
    }

    #[pyfunction]
    fn gt(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        a.rich_compare(b, PyComparisonOp::Gt, vm)
    }

    #[pyfunction]
    fn ge(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        a.rich_compare(b, PyComparisonOp::Ge, vm)
    }

    #[pyfunction]
    fn eq(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        a.rich_compare(b, PyComparisonOp::Eq, vm)
    }

    #[pyfunction]
    fn ne(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        a.rich_compare(b, PyComparisonOp::Ne, vm)
    }

    #[pyfunction]
    fn not_(a: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        a.try_to_bool(vm).map(|r| !r)
    }

    #[pyfunction]
    fn truth(a: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        a.try_to_bool(vm)
    }

    #[pyfunction]
    fn is_(a: PyObjectRef, b: PyObjectRef) -> PyResult<bool> {
        Ok(a.is(&b))
    }

    #[pyfunction]
    fn is_not(a: PyObjectRef, b: PyObjectRef) -> PyResult<bool> {
        Ok(!a.is(&b))
    }

    #[pyfunction]
    fn abs(a: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._abs(&a)
    }

    #[pyfunction]
    fn add(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._add(&a, &b)
    }

    #[pyfunction]
    fn and_(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._and(&a, &b)
    }

    #[pyfunction]
    fn floordiv(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._floordiv(&a, &b)
    }

    // Note: Keep track of issue17567. Will need changes in order to strictly match behavior of
    // a.__index__ as raised in the issue. Currently, we accept int subclasses.
    #[pyfunction]
    fn index(a: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyIntRef> {
        a.try_index(vm)
    }

    #[pyfunction]
    fn invert(pos: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._invert(&pos)
    }

    #[pyfunction]
    fn lshift(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._lshift(&a, &b)
    }

    #[pyfunction(name = "mod")]
    fn mod_(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._mod(&a, &b)
    }

    #[pyfunction]
    fn mul(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._mul(&a, &b)
    }

    #[pyfunction]
    fn matmul(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._matmul(&a, &b)
    }

    #[pyfunction]
    fn neg(pos: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._neg(&pos)
    }

    #[pyfunction]
    fn or_(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._or(&a, &b)
    }

    #[pyfunction]
    fn pos(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._pos(&obj)
    }

    #[pyfunction]
    fn pow(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._pow(&a, &b)
    }

    #[pyfunction]
    fn rshift(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._rshift(&a, &b)
    }

    #[pyfunction]
    fn sub(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._sub(&a, &b)
    }

    #[pyfunction]
    fn truediv(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._truediv(&a, &b)
    }

    #[pyfunction]
    fn xor(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._xor(&a, &b)
    }

    // Sequence based operators

    #[pyfunction]
    fn concat(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        // Best attempt at checking that a is sequence-like.
        if !a.class().has_attr(identifier!(vm, __getitem__))
            || a.fast_isinstance(vm.ctx.types.dict_type)
        {
            return Err(
                vm.new_type_error(format!("{} object can't be concatenated", a.class().name()))
            );
        }
        vm._add(&a, &b)
    }

    #[pyfunction]
    fn contains(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._contains(a, b)
    }

    #[pyfunction(name = "countOf")]
    fn count_of(a: PyIter, b: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        let mut count: usize = 0;
        for element in a.iter_without_hint::<PyObjectRef>(vm)? {
            let element = element?;
            if element.is(&b) || vm.bool_eq(&b, &element)? {
                count += 1;
            }
        }
        Ok(count)
    }

    #[pyfunction]
    fn delitem(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        a.del_item(&*b, vm)
    }

    #[pyfunction]
    fn getitem(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        a.get_item(&*b, vm)
    }

    #[pyfunction(name = "indexOf")]
    fn index_of(a: PyIter, b: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        for (index, element) in a.iter_without_hint::<PyObjectRef>(vm)?.enumerate() {
            let element = element?;
            if element.is(&b) || vm.bool_eq(&b, &element)? {
                return Ok(index);
            }
        }
        Err(vm.new_value_error("sequence.index(x): x not in sequence".to_owned()))
    }

    #[pyfunction]
    fn setitem(
        a: PyObjectRef,
        b: PyObjectRef,
        c: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        a.set_item(&*b, c, vm)
    }

    #[pyfunction]
    fn length_hint(obj: PyObjectRef, default: OptionalArg, vm: &VirtualMachine) -> PyResult<usize> {
        let default: usize = default
            .map(|v| {
                if !v.fast_isinstance(vm.ctx.types.int_type) {
                    return Err(vm.new_type_error(format!(
                        "'{}' type cannot be interpreted as an integer",
                        v.class().name()
                    )));
                }
                v.payload::<PyInt>().unwrap().try_to_primitive(vm)
            })
            .unwrap_or(Ok(0))?;
        obj.length_hint(default, vm)
    }

    // Inplace Operators

    #[pyfunction]
    fn iadd(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._iadd(&a, &b)
    }

    #[pyfunction]
    fn iand(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._iand(&a, &b)
    }

    #[pyfunction]
    fn iconcat(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        // Best attempt at checking that a is sequence-like.
        if !a.class().has_attr(identifier!(vm, __getitem__))
            || a.fast_isinstance(vm.ctx.types.dict_type)
        {
            return Err(
                vm.new_type_error(format!("{} object can't be concatenated", a.class().name()))
            );
        }
        vm._iadd(&a, &b)
    }

    #[pyfunction]
    fn ifloordiv(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._ifloordiv(&a, &b)
    }

    #[pyfunction]
    fn ilshift(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._ilshift(&a, &b)
    }

    #[pyfunction]
    fn imod(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._imod(&a, &b)
    }

    #[pyfunction]
    fn imul(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._imul(&a, &b)
    }

    #[pyfunction]
    fn imatmul(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._imatmul(&a, &b)
    }

    #[pyfunction]
    fn ior(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._ior(&a, &b)
    }

    #[pyfunction]
    fn ipow(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._ipow(&a, &b)
    }

    #[pyfunction]
    fn irshift(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._irshift(&a, &b)
    }

    #[pyfunction]
    fn isub(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._isub(&a, &b)
    }

    #[pyfunction]
    fn itruediv(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._itruediv(&a, &b)
    }

    #[pyfunction]
    fn ixor(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._ixor(&a, &b)
    }

    #[pyfunction]
    fn _compare_digest(
        a: Either<PyStrRef, ArgBytesLike>,
        b: Either<PyStrRef, ArgBytesLike>,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        let res = match (a, b) {
            (Either::A(a), Either::A(b)) => {
                if !a.as_str().is_ascii() || !b.as_str().is_ascii() {
                    return Err(vm.new_type_error(
                        "comparing strings with non-ASCII characters is not supported".to_owned(),
                    ));
                }
                cmp::timing_safe_cmp(a.as_str().as_bytes(), b.as_str().as_bytes())
            }
            (Either::B(a), Either::B(b)) => {
                a.with_ref(|a| b.with_ref(|b| cmp::timing_safe_cmp(a, b)))
            }
            _ => {
                return Err(vm.new_type_error(
                    "unsupported operand types(s) or combination of types".to_owned(),
                ))
            }
        };
        Ok(res)
    }

    /// attrgetter(attr, ...) --> attrgetter object
    ///
    /// Return a callable object that fetches the given attribute(s) from its operand.
    /// After f = attrgetter('name'), the call f(r) returns r.name.
    /// After g = attrgetter('name', 'date'), the call g(r) returns (r.name, r.date).
    /// After h = attrgetter('name.first', 'name.last'), the call h(r) returns
    /// (r.name.first, r.name.last).
    #[pyattr]
    #[pyclass(name = "attrgetter")]
    #[derive(Debug, PyPayload)]
    struct PyAttrGetter {
        attrs: Vec<PyStrRef>,
    }

    #[pyclass(with(Callable, Constructor))]
    impl PyAttrGetter {
        #[pymethod(magic)]
        fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let fmt = if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
                let mut parts = Vec::with_capacity(zelf.attrs.len());
                for part in &zelf.attrs {
                    parts.push(part.as_object().repr(vm)?.as_str().to_owned());
                }
                parts.join(", ")
            } else {
                "...".to_owned()
            };
            Ok(format!("operator.attrgetter({fmt})"))
        }

        #[pymethod(magic)]
        fn reduce(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<(PyTypeRef, PyTupleRef)> {
            let attrs = vm
                .ctx
                .new_tuple(zelf.attrs.iter().map(|v| v.clone().into()).collect());
            Ok((zelf.class().to_owned(), attrs))
        }

        // Go through dotted parts of string and call getattr on whatever is returned.
        fn get_single_attr(
            obj: PyObjectRef,
            attr: &str,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            let parts = attr.split('.').collect::<Vec<_>>();
            if parts.len() == 1 {
                return obj.get_attr(parts[0], vm);
            }
            let mut obj = obj;
            for part in parts {
                obj = obj.get_attr(part, vm)?;
            }
            Ok(obj)
        }
    }

    impl Constructor for PyAttrGetter {
        type Args = FuncArgs;

        fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
            let nattr = args.args.len();
            // Check we get no keyword and at least one positional.
            if !args.kwargs.is_empty() {
                return Err(vm.new_type_error("attrgetter() takes no keyword arguments".to_owned()));
            }
            if nattr == 0 {
                return Err(vm.new_type_error("attrgetter expected 1 argument, got 0.".to_owned()));
            }
            let mut attrs = Vec::with_capacity(nattr);
            for o in args.args {
                if let Ok(r) = o.try_into_value(vm) {
                    attrs.push(r);
                } else {
                    return Err(vm.new_type_error("attribute name must be a string".to_owned()));
                }
            }
            PyAttrGetter { attrs }
                .into_ref_with_type(vm, cls)
                .map(Into::into)
        }
    }

    impl Callable for PyAttrGetter {
        type Args = PyObjectRef;
        fn call(zelf: &Py<Self>, obj: Self::Args, vm: &VirtualMachine) -> PyResult {
            // Handle case where we only have one attribute.
            if zelf.attrs.len() == 1 {
                return Self::get_single_attr(obj, zelf.attrs[0].as_str(), vm);
            }
            // Build tuple and call get_single on each element in attrs.
            let mut results = Vec::with_capacity(zelf.attrs.len());
            for o in &zelf.attrs {
                results.push(Self::get_single_attr(obj.clone(), o.as_str(), vm)?);
            }
            Ok(vm.ctx.new_tuple(results).into())
        }
    }

    /// itemgetter(item, ...) --> itemgetter object
    ///
    /// Return a callable object that fetches the given item(s) from its operand.
    /// After f = itemgetter(2), the call f(r) returns r[2].
    /// After g = itemgetter(2, 5, 3), the call g(r) returns (r[2], r[5], r[3])
    #[pyattr]
    #[pyclass(name = "itemgetter")]
    #[derive(Debug, PyPayload)]
    struct PyItemGetter {
        items: Vec<PyObjectRef>,
    }

    #[pyclass(with(Callable, Constructor))]
    impl PyItemGetter {
        #[pymethod(magic)]
        fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let fmt = if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
                let mut items = Vec::with_capacity(zelf.items.len());
                for item in &zelf.items {
                    items.push(item.repr(vm)?.as_str().to_owned());
                }
                items.join(", ")
            } else {
                "...".to_owned()
            };
            Ok(format!("operator.itemgetter({fmt})"))
        }

        #[pymethod(magic)]
        fn reduce(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyObjectRef {
            let items = vm.ctx.new_tuple(zelf.items.to_vec());
            vm.new_pyobj((zelf.class().to_owned(), items))
        }
    }
    impl Constructor for PyItemGetter {
        type Args = FuncArgs;

        fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
            // Check we get no keyword and at least one positional.
            if !args.kwargs.is_empty() {
                return Err(vm.new_type_error("itemgetter() takes no keyword arguments".to_owned()));
            }
            if args.args.is_empty() {
                return Err(vm.new_type_error("itemgetter expected 1 argument, got 0.".to_owned()));
            }
            PyItemGetter { items: args.args }
                .into_ref_with_type(vm, cls)
                .map(Into::into)
        }
    }

    impl Callable for PyItemGetter {
        type Args = PyObjectRef;
        fn call(zelf: &Py<Self>, obj: Self::Args, vm: &VirtualMachine) -> PyResult {
            // Handle case where we only have one attribute.
            if zelf.items.len() == 1 {
                return obj.get_item(&*zelf.items[0], vm);
            }
            // Build tuple and call get_single on each element in attrs.
            let mut results = Vec::with_capacity(zelf.items.len());
            for item in &zelf.items {
                results.push(obj.get_item(&**item, vm)?);
            }
            Ok(vm.ctx.new_tuple(results).into())
        }
    }

    /// methodcaller(name, ...) --> methodcaller object
    ///
    /// Return a callable object that calls the given method on its operand.
    /// After f = methodcaller('name'), the call f(r) returns r.name().
    /// After g = methodcaller('name', 'date', foo=1), the call g(r) returns
    /// r.name('date', foo=1).
    #[pyattr]
    #[pyclass(name = "methodcaller")]
    #[derive(Debug, PyPayload)]
    struct PyMethodCaller {
        name: PyStrRef,
        args: FuncArgs,
    }

    #[pyclass(with(Callable, Constructor))]
    impl PyMethodCaller {
        #[pymethod(magic)]
        fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let fmt = if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
                let args = &zelf.args.args;
                let kwargs = &zelf.args.kwargs;
                let mut fmt = vec![zelf.name.as_object().repr(vm)?.as_str().to_owned()];
                if !args.is_empty() {
                    let mut parts = Vec::with_capacity(args.len());
                    for v in args {
                        parts.push(v.repr(vm)?.as_str().to_owned());
                    }
                    fmt.push(parts.join(", "));
                }
                // build name=value pairs from KwArgs.
                if !kwargs.is_empty() {
                    let mut parts = Vec::with_capacity(kwargs.len());
                    for (key, value) in kwargs {
                        let value_repr = value.repr(vm)?;
                        parts.push(format!("{key}={value_repr}"));
                    }
                    fmt.push(parts.join(", "));
                }
                fmt.join(", ")
            } else {
                "...".to_owned()
            };
            Ok(format!("operator.methodcaller({fmt})"))
        }

        #[pymethod(magic)]
        fn reduce(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
            // With no kwargs, return (type(obj), (name, *args)) tuple.
            if zelf.args.kwargs.is_empty() {
                let mut pargs = vec![zelf.name.as_object().to_owned()];
                pargs.append(&mut zelf.args.args.clone());
                Ok(vm.new_tuple((zelf.class().to_owned(), vm.ctx.new_tuple(pargs))))
            } else {
                // If we have kwargs, create a partial function that contains them and pass back that
                // along with the args.
                let partial = vm.import("functools", None, 0)?.get_attr("partial", vm)?;
                let callable = vm.invoke(
                    &partial,
                    FuncArgs::new(
                        vec![zelf.class().to_owned().into(), zelf.name.clone().into()],
                        KwArgs::new(zelf.args.kwargs.clone()),
                    ),
                )?;
                Ok(vm.new_tuple((callable, vm.ctx.new_tuple(zelf.args.args.clone()))))
            }
        }
    }

    impl Constructor for PyMethodCaller {
        type Args = (PyObjectRef, FuncArgs);

        fn py_new(cls: PyTypeRef, (name, args): Self::Args, vm: &VirtualMachine) -> PyResult {
            if let Ok(name) = name.try_into_value(vm) {
                PyMethodCaller { name, args }
                    .into_ref_with_type(vm, cls)
                    .map(Into::into)
            } else {
                Err(vm.new_type_error("method name must be a string".to_owned()))
            }
        }
    }

    impl Callable for PyMethodCaller {
        type Args = PyObjectRef;

        #[inline]
        fn call(zelf: &Py<Self>, obj: Self::Args, vm: &VirtualMachine) -> PyResult {
            vm.call_method(&obj, zelf.name.as_str(), zelf.args.clone())
        }
    }
}
