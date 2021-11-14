pub(crate) use _operator::make_module;

/// Operator Interface
///
/// This module exports a set of functions corresponding to the intrinsic
/// operators of Python.  For example, operator.add(x, y) is equivalent
/// to the expression x+y.  The function names are those used for special
/// methods; variants without leading and trailing '__' are also provided
/// for convenience.
#[pymodule]
mod _operator {
    use crate::common::cmp;
    use crate::{
        builtins::{PyInt, PyIntRef, PyStrRef, PyTupleRef, PyTypeRef},
        function::{ArgBytesLike, FuncArgs, KwArgs, OptionalArg},
        protocol::PyIter,
        types::{
            Callable, Constructor,
            PyComparisonOp::{Eq, Ge, Gt, Le, Lt, Ne},
        },
        utils::Either,
        vm::ReprGuard,
        IdProtocol, ItemProtocol, PyObjectRef, PyObjectView, PyRef, PyResult, PyValue,
        TypeProtocol, VirtualMachine,
    };

    /// Same as a < b.
    #[pyfunction]
    fn lt(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        a.rich_compare(b, Lt, vm)
    }

    /// Same as a <= b.
    #[pyfunction]
    fn le(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        a.rich_compare(b, Le, vm)
    }

    /// Same as a > b.
    #[pyfunction]
    fn gt(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        a.rich_compare(b, Gt, vm)
    }

    /// Same as a >= b.
    #[pyfunction]
    fn ge(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        a.rich_compare(b, Ge, vm)
    }

    /// Same as a == b.
    #[pyfunction]
    fn eq(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        a.rich_compare(b, Eq, vm)
    }

    /// Same as a != b.
    #[pyfunction]
    fn ne(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        a.rich_compare(b, Ne, vm)
    }

    /// Same as not a.
    #[pyfunction]
    fn not_(a: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        a.try_to_bool(vm).map(|r| !r)
    }

    /// Return True if a is true, False otherwise.
    #[pyfunction]
    fn truth(a: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        a.try_to_bool(vm)
    }

    /// Same as a is b.
    #[pyfunction]
    fn is_(a: PyObjectRef, b: PyObjectRef) -> PyResult<bool> {
        Ok(a.is(&b))
    }

    /// Same as a is not b.
    #[pyfunction]
    fn is_not(a: PyObjectRef, b: PyObjectRef) -> PyResult<bool> {
        Ok(!a.is(&b))
    }

    /// Return the absolute value of the argument.
    #[pyfunction]
    fn abs(a: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._abs(&a)
    }

    /// Return a + b, for a and b numbers.
    #[pyfunction]
    fn add(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._add(&a, &b)
    }

    /// Return the bitwise and of a and b.
    #[pyfunction]
    fn and_(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._and(&a, &b)
    }

    /// Return a // b.
    #[pyfunction]
    fn floordiv(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._floordiv(&a, &b)
    }

    // Note: Keep track of issue17567. Will need changes in order to strictly match behavior of
    // a.__index__ as raised in the issue. Currently, we accept int subclasses.
    /// Return a converted to an integer. Equivalent to a.__index__().
    #[pyfunction]
    fn index(a: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyIntRef> {
        vm.to_index(&a)
    }

    /// Return the bitwise inverse of the number obj. This is equivalent to ~obj.
    #[pyfunction]
    fn invert(pos: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._invert(&pos)
    }

    /// Return a shifted left by b.
    #[pyfunction]
    fn lshift(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._lshift(&a, &b)
    }

    /// Return a % b
    #[pyfunction(name = "mod")]
    fn mod_(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._mod(&a, &b)
    }

    /// Return a * b
    #[pyfunction]
    fn mul(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._mul(&a, &b)
    }

    /// Return a @ b
    #[pyfunction]
    fn mat_mul(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._matmul(&a, &b)
    }

    /// Return obj negated (-obj).
    #[pyfunction]
    fn neg(pos: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._neg(&pos)
    }

    /// Return the bitwise or of a and b.
    #[pyfunction]
    fn or_(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._or(&a, &b)
    }

    /// Return obj positive (+obj).
    #[pyfunction]
    fn pos(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._pos(&obj)
    }

    /// Return a ** b, for a and b numbers.
    #[pyfunction]
    fn pow(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._pow(&a, &b)
    }

    /// Return a shifted right by b.
    #[pyfunction]
    fn rshift(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._rshift(&a, &b)
    }

    /// Return a - b.
    #[pyfunction]
    fn sub(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._sub(&a, &b)
    }

    /// Return a / b where 2/3 is .66 rather than 0. This is also known as "true" division.
    #[pyfunction]
    fn truediv(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._truediv(&a, &b)
    }

    /// Return the bitwise exclusive or of a and b.
    #[pyfunction]
    fn xor(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._xor(&a, &b)
    }

    // Sequence based operators

    /// Return a + b for a and b sequences.
    #[pyfunction]
    fn concat(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        // Best attempt at checking that a is sequence-like.
        if !a.class().has_attr("__getitem__") || a.isinstance(&vm.ctx.types.dict_type) {
            return Err(
                vm.new_type_error(format!("{} object can't be concatenated", a.class().name()))
            );
        }
        vm._add(&a, &b)
    }

    /// Return the outcome of the test b in a. Note the reversed operands.
    #[pyfunction]
    fn contains(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._membership(a, b)
    }

    /// Return the number of occurrences of b in a.
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

    /// Remove the value of a at index b.
    #[pyfunction]
    fn delitem(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        a.del_item(b, vm)
    }

    /// Return the value of a at index b.
    #[pyfunction]
    fn getitem(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        a.get_item(b, vm)
    }

    /// Return the number of occurrences of b in a.
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

    /// Set the value of a at index b to c.
    #[pyfunction]
    fn setitem(
        a: PyObjectRef,
        b: PyObjectRef,
        c: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        a.set_item(b, c, vm)
    }

    /// Return an estimate of the number of items in obj.
    ///
    /// This is useful for presizing containers when building from an iterable.
    ///
    /// If the object supports len(), the result will be exact.
    /// Otherwise, it may over- or under-estimate by an arbitrary amount.
    /// The result will be an integer >= 0.
    #[pyfunction]
    fn length_hint(obj: PyObjectRef, default: OptionalArg, vm: &VirtualMachine) -> PyResult<usize> {
        let default: usize = default
            .map(|v| {
                if !v.isinstance(&vm.ctx.types.int_type) {
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

    /// a = iadd(a, b) is equivalent to a += b.
    #[pyfunction]
    fn iadd(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._iadd(&a, &b)
    }

    /// a = iand(a, b) is equivalent to a &= b.
    #[pyfunction]
    fn iand(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._iand(&a, &b)
    }

    /// a = iconcat(a, b) is equivalent to a += b for a and b sequences.
    #[pyfunction]
    fn iconcat(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        // Best attempt at checking that a is sequence-like.
        if !a.class().has_attr("__getitem__") || a.isinstance(&vm.ctx.types.dict_type) {
            return Err(
                vm.new_type_error(format!("{} object can't be concatenated", a.class().name()))
            );
        }
        vm._iadd(&a, &b)
    }

    /// a = ifloordiv(a, b) is equivalent to a //= b.
    #[pyfunction]
    fn ifloordiv(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._ifloordiv(&a, &b)
    }

    /// a = ilshift(a, b) is equivalent to a <<= b.
    #[pyfunction]
    fn ilshift(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._ilshift(&a, &b)
    }

    /// a = imod(a, b) is equivalent to a %= b.
    #[pyfunction]
    fn imod(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._imod(&a, &b)
    }

    /// a = imul(a, b) is equivalent to a *= b.
    #[pyfunction]
    fn imul(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._imul(&a, &b)
    }

    /// a = imatmul(a, b) is equivalent to a @= b.
    #[pyfunction]
    fn imatmul(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._imatmul(&a, &b)
    }

    /// a = ior(a, b) is equivalent to a |= b.
    #[pyfunction]
    fn ior(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._ior(&a, &b)
    }

    /// a = ipow(a, b) is equivalent to a **= b.
    #[pyfunction]
    fn ipow(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._ipow(&a, &b)
    }

    /// a = irshift(a, b) is equivalent to a >>= b.
    #[pyfunction]
    fn irshift(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._irshift(&a, &b)
    }

    /// a = isub(a, b) is equivalent to a -= b.
    #[pyfunction]
    fn isub(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._isub(&a, &b)
    }

    /// a = itruediv(a, b) is equivalent to a /= b.
    #[pyfunction]
    fn itruediv(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._itruediv(&a, &b)
    }

    /// a = ixor(a, b) is equivalent to a ^= b.
    #[pyfunction]
    fn ixor(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._ixor(&a, &b)
    }

    /// Return 'a == b'.
    ///
    /// This function uses an approach designed to prevent
    /// timing analysis, making it appropriate for cryptography.
    ///
    /// a and b must both be of the same type: either str (ASCII only),
    /// or any bytes-like object.
    ///
    /// Note: If a and b are of different lengths, or if an error occurs,
    /// a timing attack could theoretically reveal information about the
    /// types and lengths of a and b--but not their values.
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
    #[derive(Debug, PyValue)]
    struct PyAttrGetter {
        attrs: Vec<PyStrRef>,
    }

    #[pyimpl(with(Callable, Constructor))]
    impl PyAttrGetter {
        #[pymethod(magic)]
        fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let fmt = if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
                let mut parts = Vec::with_capacity(zelf.attrs.len());
                for part in zelf.attrs.iter() {
                    parts.push(part.as_object().repr(vm)?.as_str().to_owned());
                }
                parts.join(", ")
            } else {
                "...".to_owned()
            };
            Ok(format!("operator.attrgetter({})", fmt))
        }

        #[pymethod(magic)]
        fn reduce(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<(PyTypeRef, PyTupleRef)> {
            let attrs = vm.ctx.new_tuple(
                zelf.attrs
                    .iter()
                    .map(|v| v.as_object().to_owned())
                    .collect(),
            );
            Ok((zelf.clone_class(), attrs))
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
            PyAttrGetter { attrs }.into_pyresult_with_type(vm, cls)
        }
    }

    impl Callable for PyAttrGetter {
        type Args = PyObjectRef;
        fn call(zelf: &PyObjectView<Self>, obj: Self::Args, vm: &VirtualMachine) -> PyResult {
            // Handle case where we only have one attribute.
            if zelf.attrs.len() == 1 {
                return Self::get_single_attr(obj, zelf.attrs[0].as_str(), vm);
            }
            // Build tuple and call get_single on each element in attrs.
            let mut results = Vec::with_capacity(zelf.attrs.len());
            for o in zelf.attrs.iter() {
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
    #[derive(Debug, PyValue)]
    struct PyItemGetter {
        items: Vec<PyObjectRef>,
    }

    #[pyimpl(with(Callable, Constructor))]
    impl PyItemGetter {
        #[pymethod(magic)]
        fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let fmt = if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
                let mut items = Vec::with_capacity(zelf.items.len());
                for item in zelf.items.iter() {
                    items.push(item.repr(vm)?.as_str().to_owned());
                }
                items.join(", ")
            } else {
                "...".to_owned()
            };
            Ok(format!("operator.itemgetter({})", fmt))
        }

        #[pymethod(magic)]
        fn reduce(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyObjectRef {
            let items = vm.ctx.new_tuple(zelf.items.to_vec());
            vm.new_pyobj((zelf.clone_class(), items))
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
            PyItemGetter { items: args.args }.into_pyresult_with_type(vm, cls)
        }
    }

    impl Callable for PyItemGetter {
        type Args = PyObjectRef;
        fn call(zelf: &PyObjectView<Self>, obj: Self::Args, vm: &VirtualMachine) -> PyResult {
            // Handle case where we only have one attribute.
            if zelf.items.len() == 1 {
                return obj.get_item(zelf.items[0].clone(), vm);
            }
            // Build tuple and call get_single on each element in attrs.
            let mut results = Vec::with_capacity(zelf.items.len());
            for item in zelf.items.iter() {
                results.push(obj.get_item(item.clone(), vm)?);
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
    #[derive(Debug, PyValue)]
    struct PyMethodCaller {
        name: PyStrRef,
        args: FuncArgs,
    }

    #[pyimpl(with(Callable, Constructor))]
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
                        parts.push(format!("{}={}", key, value_repr));
                    }
                    fmt.push(parts.join(", "));
                }
                fmt.join(", ")
            } else {
                "...".to_owned()
            };
            Ok(format!("operator.methodcaller({})", fmt))
        }

        #[pymethod(magic)]
        fn reduce(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
            // With no kwargs, return (type(obj), (name, *args)) tuple.
            if zelf.args.kwargs.is_empty() {
                let mut pargs = vec![zelf.name.as_object().to_owned()];
                pargs.append(&mut zelf.args.args.clone());
                Ok(vm.new_tuple((zelf.clone_class(), vm.ctx.new_tuple(pargs))))
            } else {
                // If we have kwargs, create a partial function that contains them and pass back that
                // along with the args.
                let partial = vm.import("functools", None, 0)?.get_attr("partial", vm)?;
                let callable = vm.invoke(
                    &partial,
                    FuncArgs::new(
                        vec![zelf.clone_class().into(), zelf.name.as_object().to_owned()],
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
                PyMethodCaller { name, args }.into_pyresult_with_type(vm, cls)
            } else {
                Err(vm.new_type_error("method name must be a string".to_owned()))
            }
        }
    }

    impl Callable for PyMethodCaller {
        type Args = PyObjectRef;

        #[inline]
        fn call(zelf: &PyObjectView<Self>, obj: Self::Args, vm: &VirtualMachine) -> PyResult {
            vm.call_method(&obj, zelf.name.as_str(), zelf.args.clone())
        }
    }
}
