pub(crate) use _operator::module_def;

#[pymodule]
mod _operator {
    use crate::{
        AsObject, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyIntRef, PyStr, PyStrRef, PyTupleRef, PyType, PyTypeRef},
        common::wtf8::{Wtf8, Wtf8Buf},
        function::{
            ArgBytesLike, ArgumentError, Either, FromArgs, FuncArgs, OptionalArg, Param, PosArgs,
        },
        protocol::PyIter,
        recursion::ReprGuard,
        types::{Callable, Constructor, PyComparisonOp, Representable},
    };
    use constant_time_eq::constant_time_eq;

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
    fn is_(a: PyObjectRef, b: PyObjectRef) -> bool {
        a.is(&b)
    }

    #[pyfunction]
    fn is_not(a: PyObjectRef, b: PyObjectRef) -> bool {
        !a.is(&b)
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
    fn invert(a: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._invert(&a)
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
    fn neg(a: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._neg(&a)
    }

    #[pyfunction]
    fn or_(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._or(&a, &b)
    }

    #[pyfunction]
    fn pos(a: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._pos(&a)
    }

    #[pyfunction]
    fn pow(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm._pow(&a, &b, vm.ctx.none.as_object())
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
    fn contains(a: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        vm._contains(&a, &b)
    }

    #[pyfunction(name = "countOf")]
    fn count_of(a: PyIter, b: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        let mut count: usize = 0;
        for element in a.iter::<PyObjectRef>(vm)? {
            let element = element?;
            if element.is(&b) || vm.bool_eq(&element, &b)? {
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
        for (index, element) in a.iter::<PyObjectRef>(vm)?.enumerate() {
            let element = element?;
            if element.is(&b) || vm.bool_eq(&element, &b)? {
                return Ok(index);
            }
        }
        Err(vm.new_value_error("sequence.index(x): x not in sequence"))
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

    #[derive(FromArgs)]
    struct LengthHintArgs {
        #[pyarg(positional)]
        obj: PyObjectRef,
        #[pyarg(positional, default = 0)]
        default: isize,
    }

    #[pyfunction]
    fn length_hint(args: LengthHintArgs, vm: &VirtualMachine) -> PyResult<isize> {
        let LengthHintArgs { obj, default } = args;
        Ok(vm
            .length_hint_opt(obj)?
            .map_or(default, |hint| hint as isize))
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
            return Err(vm.new_type_error(format!(
                "'{}' object can't be concatenated",
                a.class().name()
            )));
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
        vm._ipow(&a, &b, vm.ctx.none.as_object())
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
                if !a.isascii() || !b.isascii() {
                    return Err(vm.new_type_error(
                        "comparing strings with non-ASCII characters is not supported",
                    ));
                }
                constant_time_eq(a.as_bytes(), b.as_bytes())
            }
            (Either::B(a), Either::B(b)) => a.with_ref(|a| b.with_ref(|b| constant_time_eq(a, b))),
            _ => {
                return Err(
                    vm.new_type_error("unsupported operand types(s) or combination of types")
                );
            }
        };
        Ok(res)
    }

    #[pyattr]
    #[pyclass(name = "attrgetter", module = "operator", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyAttrGetter {
        attrs: Vec<Vec<PyStrRef>>,
    }

    impl PyAttrGetter {
        // Split at construction, as CPython does. Dotted str subclasses become
        // exact strings; names without a dot keep the original string object.
        fn prepare_attr(attr: PyStrRef, vm: &VirtualMachine) -> Vec<PyStrRef> {
            if !attr.as_bytes().contains(&b'.') {
                return vec![attr];
            }
            attr.as_bytes()
                .split(|&b| b == b'.')
                .map(|part| {
                    let part = Wtf8::from_bytes(part).expect("originally valid WTF-8");
                    vm.ctx.new_str(part)
                })
                .collect()
        }

        fn attr_name(parts: &[PyStrRef], vm: &VirtualMachine) -> PyStrRef {
            if parts.len() == 1 {
                return parts[0].clone();
            }
            let mut name = Wtf8Buf::new();
            for (index, part) in parts.iter().enumerate() {
                if index != 0 {
                    name.push_char('.');
                }
                name.push_wtf8(part.as_wtf8());
            }
            vm.ctx.new_str(name)
        }

        // Go through dotted parts of string and call getattr on whatever is returned.
        fn get_single_attr(
            mut obj: PyObjectRef,
            parts: &[PyStrRef],
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            for part in parts {
                obj = obj.get_attr(part, vm)?;
            }
            Ok(obj)
        }
    }

    #[pyclass(with(Callable, Constructor, Representable))]
    impl Py<PyAttrGetter> {
        #[pygetset]
        fn __text_signature__(&self) -> &'static str {
            "(obj, /)"
        }

        #[pymethod]
        fn __reduce__(zelf: PyRef<PyAttrGetter>, vm: &VirtualMachine) -> (PyTypeRef, PyTupleRef) {
            let attrs = vm.ctx.new_tuple(
                zelf.attrs
                    .iter()
                    .map(|parts| PyAttrGetter::attr_name(parts, vm).into())
                    .collect(),
            );
            (zelf.class().to_owned(), attrs)
        }
    }

    // These constructors reject keywords before checking arity or converting
    // values. Keep that ordering while sharing the positional binder.
    fn getter_args(
        args: &mut FuncArgs,
        name: &str,
        vm: &VirtualMachine,
    ) -> Result<Vec<PyObjectRef>, ArgumentError> {
        if !args.kwargs.is_empty() {
            return Err(vm
                .new_type_error(format!("{name}() takes no keyword arguments"))
                .into());
        }
        if args.args.is_empty() {
            return Err(vm.new_arity_type_error(name, 1..=1, 0).into());
        }
        Ok(PosArgs::<PyObjectRef>::from_args(vm, args)?.into_vec())
    }

    struct AttrGetterArgs(Vec<PyObjectRef>);

    impl FromArgs for AttrGetterArgs {
        // CPython reports "expected 1 argument" even though this is variadic.
        const BINDING_PARAMS: Option<&'static [Param]> = None;

        const PARAMS: Option<&'static [Param]> = Some(&[
            Param::positional_only("attr"),
            Param::var_positional("attrs"),
        ]);

        fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
            getter_args(args, "attrgetter", vm).map(Self)
        }
    }

    impl Constructor for PyAttrGetter {
        type Args = AttrGetterArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let mut attrs = Vec::with_capacity(args.0.len());
            for obj in args.0 {
                let attr = obj
                    .downcast::<PyStr>()
                    .map_err(|_| vm.new_type_error("attribute name must be a string"))?;
                attrs.push(Self::prepare_attr(attr, vm));
            }
            Ok(Self { attrs })
        }
    }

    impl Callable for PyAttrGetter {
        type Args = PyObjectRef;
        fn call(zelf: &Py<Self>, obj: Self::Args, vm: &VirtualMachine) -> PyResult {
            // Handle case where we only have one attribute.
            if zelf.attrs.len() == 1 {
                return Self::get_single_attr(obj, &zelf.attrs[0], vm);
            }
            // Build tuple and call get_single on each element in attrs.
            let mut results = Vec::with_capacity(zelf.attrs.len());
            for o in &zelf.attrs {
                results.push(Self::get_single_attr(obj.clone(), o, vm)?);
            }
            Ok(vm.ctx.new_tuple(results).into())
        }
    }

    impl Representable for PyAttrGetter {
        #[inline]
        fn repr_wtf8(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Wtf8Buf> {
            let mut result = Wtf8Buf::from("operator.attrgetter(");
            if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
                let mut first = true;
                for part in &zelf.attrs {
                    if !first {
                        result.push_str(", ");
                    }
                    first = false;
                    result.push_wtf8(Self::attr_name(part, vm).as_object().repr(vm)?.as_wtf8());
                }
            } else {
                result.push_str("...");
            }
            result.push_char(')');
            Ok(result)
        }
    }

    #[pyattr]
    #[pyclass(name = "itemgetter", module = "operator", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItemGetter {
        items: Vec<PyObjectRef>,
    }

    #[pyclass(with(Callable, Constructor, Representable))]
    impl Py<PyItemGetter> {
        #[pygetset]
        fn __text_signature__(&self) -> &'static str {
            "(obj, /)"
        }

        #[pymethod]
        fn __reduce__(zelf: PyRef<PyItemGetter>, vm: &VirtualMachine) -> PyObjectRef {
            let items = vm.ctx.new_tuple(zelf.items.to_vec());
            vm.new_pyobj((zelf.class().to_owned(), items))
        }
    }
    struct ItemGetterArgs(Vec<PyObjectRef>);

    impl FromArgs for ItemGetterArgs {
        // CPython reports "expected 1 argument" even though this is variadic.
        const BINDING_PARAMS: Option<&'static [Param]> = None;

        const PARAMS: Option<&'static [Param]> = Some(&[
            Param::positional_only("item"),
            Param::var_positional("items"),
        ]);

        fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
            getter_args(args, "itemgetter", vm).map(Self)
        }
    }

    impl Constructor for PyItemGetter {
        type Args = ItemGetterArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self { items: args.0 })
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

    impl Representable for PyItemGetter {
        #[inline]
        fn repr_wtf8(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Wtf8Buf> {
            let mut result = Wtf8Buf::from("operator.itemgetter(");
            if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
                let mut first = true;
                for item in &zelf.items {
                    if !first {
                        result.push_str(", ");
                    }
                    first = false;
                    result.push_wtf8(item.repr(vm)?.as_wtf8());
                }
            } else {
                result.push_str("...");
            }
            result.push_char(')');
            Ok(result)
        }
    }

    #[pyattr]
    #[pyclass(name = "methodcaller", module = "operator", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyMethodCaller {
        name: PyStrRef,
        args: FuncArgs,
    }

    #[pyclass(with(Callable, Constructor, Representable))]
    impl Py<PyMethodCaller> {
        #[pygetset]
        fn __text_signature__(&self) -> &'static str {
            "(obj, /)"
        }

        #[pymethod]
        fn __reduce__(zelf: PyRef<PyMethodCaller>, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
            // With no kwargs, return (type(obj), (name, *args)) tuple.
            if zelf.args.kwargs.is_empty() {
                let mut py_args = vec![zelf.name.as_object().to_owned()];
                py_args.append(&mut zelf.args.args.clone());
                Ok(vm.new_tuple((zelf.class().to_owned(), vm.ctx.new_tuple(py_args))))
            } else {
                // If we have kwargs, create a partial function that contains them and pass back that
                // along with the args.
                let partial = vm.import("functools", 0)?.get_attr("partial", vm)?;
                let args = FuncArgs::new(
                    vec![zelf.class().to_owned().into(), zelf.name.clone().into()],
                    zelf.args.kwargs.clone(),
                );
                let callable = partial.call(args, vm)?;
                Ok(vm.new_tuple((callable, vm.ctx.new_tuple(zelf.args.args.clone()))))
            }
        }
    }

    struct MethodCallerArgs {
        name: PyStrRef,
        args: FuncArgs,
    }

    impl FromArgs for MethodCallerArgs {
        // CPython names the missing method name in a bespoke constructor error.
        const BINDING_PARAMS: Option<&'static [Param]> = None;

        const PARAMS: Option<&'static [Param]> = Some(&[
            Param::positional_only("name"),
            Param::var_positional("args"),
            Param::var_keyword("kwargs"),
        ]);

        fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
            let name = OptionalArg::<PyObjectRef>::from_args(vm, args)?
                .into_option()
                .ok_or_else(|| {
                    vm.new_type_error("methodcaller needs at least one argument, the method name")
                })?
                .downcast::<PyStr>()
                .map_err(|_| vm.new_type_error("method name must be a string"))?;
            Ok(Self {
                name,
                args: FuncArgs::from_args(vm, args)?,
            })
        }
    }

    impl Constructor for PyMethodCaller {
        type Args = MethodCallerArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self {
                name: args.name,
                args: args.args,
            })
        }
    }

    impl Callable for PyMethodCaller {
        type Args = PyObjectRef;

        #[inline]
        fn call(zelf: &Py<Self>, obj: Self::Args, vm: &VirtualMachine) -> PyResult {
            obj.get_attr(&zelf.name, vm)?.call(zelf.args.clone(), vm)
        }
    }

    impl Representable for PyMethodCaller {
        #[inline]
        fn repr_wtf8(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Wtf8Buf> {
            let mut result = Wtf8Buf::from("operator.methodcaller(");
            if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
                let args = &zelf.args.args;
                let kwargs = &zelf.args.kwargs;
                result.push_wtf8(zelf.name.as_object().repr(vm)?.as_wtf8());
                for v in args {
                    result.push_str(", ");
                    result.push_wtf8(v.repr(vm)?.as_wtf8());
                }
                for (key, value) in kwargs {
                    result.push_str(", ");
                    result.push_wtf8(key);
                    result.push_char('=');
                    result.push_wtf8(value.repr(vm)?.as_wtf8());
                }
            } else {
                result.push_str("...");
            }
            result.push_char(')');
            Ok(result)
        }
    }
}
