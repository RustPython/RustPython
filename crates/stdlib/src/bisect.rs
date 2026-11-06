pub(crate) use _bisect::module_def;

#[pymodule]
mod _bisect {
    use crate::vm::{
        AsObject, PyObject, PyObjectRef, PyResult, TryFromBorrowedObject, VirtualMachine,
        function::{ArgIndex, Either, OptionalArg, PyComparisonValue},
        protocol::PySequence,
        types::PyComparisonOp,
    };

    #[derive(FromArgs)]
    struct BisectArgs {
        a: PyObjectRef,
        x: PyObjectRef,
        #[pyarg(any, optional, py_default = "0")]
        lo: OptionalArg<PyObjectRef>,
        // None means the sequence length.
        #[pyarg(any, optional)]
        hi: Option<PyObjectRef>,
        #[pyarg(named, optional)]
        key: Option<PyObjectRef>,
    }

    // Bind all arguments before calling __index__, then convert in the same
    // order as the CPython wrapper. Keep the original argument objects alive.
    fn parse_bounds(
        lo: &OptionalArg<PyObjectRef>,
        hi: Option<&PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<(usize, isize)> {
        let lo = match lo {
            OptionalArg::Missing => 0,
            OptionalArg::Present(obj) => {
                let index = ArgIndex::try_from_borrowed_object(vm, obj)?;
                isize::try_from(index.as_ref().as_bigint()).map_err(|_| {
                    vm.new_overflow_error("Python int too large to convert to C ssize_t")
                })?
            }
        };
        let lo = usize::try_from(lo).map_err(|_| vm.new_value_error("lo cannot be negative"))?;
        let hi = match hi {
            Some(obj) => {
                if !obj.number().is_index() {
                    let name = obj.class().slot_name();
                    let end = name.floor_char_boundary(name.len().min(200));
                    return Err(vm.new_type_error(format!(
                        "argument should be integer or None, not '{}'",
                        &name[..end]
                    )));
                }
                let index = ArgIndex::try_from_borrowed_object(vm, obj)?;
                isize::try_from(index.as_ref().as_bigint()).map_err(|_| {
                    let name = obj.class().slot_name();
                    let end = name.floor_char_boundary(name.len().min(200));
                    vm.new_overflow_error(format!(
                        "cannot fit '{}' into an index-sized integer",
                        &name[..end]
                    ))
                })?
            }
            None => -1,
        };
        Ok((lo, hi))
    }

    // The -1 sentinel is shared by an omitted/None hi and an explicit -1.
    // Other negative upper bounds have the same result as zero because lo is
    // nonnegative. Only the sentinel requires asking the sequence for length.
    #[inline]
    fn resolve_hi(hi: isize, sequence: PySequence<'_>, vm: &VirtualMachine) -> PyResult<usize> {
        if hi == -1 {
            sequence.length(vm)
        } else {
            Ok(usize::try_from(hi).unwrap_or(0))
        }
    }

    type SequenceItem = for<'a> fn(PySequence<'a>, isize, &VirtualMachine) -> PyResult;

    // Snapshot the item slot even when the search range is empty, as CPython does.
    fn sequence_item(sequence: PySequence<'_>, vm: &VirtualMachine) -> PyResult<SequenceItem> {
        sequence.slots().item.load().ok_or_else(|| {
            let msg = {
                let name = sequence.obj.class().slot_name();
                let end = name.floor_char_boundary(name.len().min(200));
                let name = &name[..end];
                if sequence.obj.mapping_unchecked().check() {
                    format!("{name} is not a sequence")
                } else {
                    format!("'{name}' object does not support indexing")
                }
            };
            vm.new_type_error(msg)
        })
    }

    #[inline]
    fn bisect_impl<const LEFT: bool>(
        a: &PyObject,
        x: &PyObject,
        mut lo: usize,
        hi: isize,
        key: Option<&PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let sequence = a.sequence_unchecked();
        let mut hi = resolve_hi(hi, sequence, vm)?;
        let get_item = sequence_item(sequence, vm)?;
        // On failure, leave the recursion section before releasing the current
        // item. This owner must be declared before the guard for drop ordering.
        let mut error_item = None;
        let recursion_context = if LEFT {
            " in _bisect.bisect_left"
        } else {
            " in _bisect.bisect_right"
        };
        vm.enter_recursive_call(recursion_context)?;
        let recursion_guard = scopeguard::guard((), |()| vm.leave_recursive_call());
        // Snapshot identity and slot without keeping the original class alive:
        // a callback can change x.__class__ and collect the old heap type.
        let (needle_type_id, mut compare) = {
            let class = x.class();
            (class.get_id(), class.slots().richcompare.load())
        };

        let result = loop {
            if lo >= hi {
                break Ok(lo);
            }
            // Handles issue 13496.
            let mid = (lo + hi) / 2;
            let a_mid = get_item(sequence, mid as isize, vm)?;
            let comp = if let Some(key) = key {
                // The callback may delete its argument. Release our reference
                // only after it returns, including when it raises an exception.
                let result = match key.call((&*a_mid,), vm) {
                    Ok(result) => result,
                    Err(err) => {
                        error_item = Some(a_mid);
                        break Err(err);
                    }
                };
                drop(a_mid);
                result
            } else {
                a_mid
            };
            let (left, right) = if LEFT { (&*comp, x) } else { (x, &*comp) };
            let less = (|| {
                if let Some(compare_fn) =
                    compare.filter(|_| comp.class().get_id() == needle_type_id)
                {
                    match compare_fn(left, right, PyComparisonOp::Lt, vm)? {
                        Either::A(obj) if obj.is(&vm.ctx.not_implemented) => {}
                        Either::A(obj) => return obj.try_to_bool(vm),
                        Either::B(PyComparisonValue::Implemented(value)) => return Ok(value),
                        Either::B(PyComparisonValue::NotImplemented) => {}
                    }
                    // NotImplemented disables the shortcut for this search.
                    // Generic dispatch retries __lt__ before reflected __gt__.
                    compare = None;
                }
                // A different type does not disable the same-type shortcut
                // for subsequent midpoint objects.
                left.rich_compare_bool(right, PyComparisonOp::Lt, vm)
            })();
            let less = match less {
                Ok(value) => value,
                Err(err) => {
                    error_item = Some(comp);
                    break Err(err);
                }
            };
            // Comparison results have already been released after conversion
            // to bool. Dispose of the midpoint before starting another probe.
            drop(comp);
            if less == LEFT {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        };
        drop(recursion_guard);
        drop(error_item);
        result
    }

    #[inline]
    #[pyfunction]
    fn bisect_left(
        BisectArgs { a, x, lo, hi, key }: BisectArgs,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let (low, high) = parse_bounds(&lo, hi.as_ref(), vm)?;
        bisect_impl::<true>(&a, &x, low, high, key.as_ref(), vm)
    }

    #[inline]
    #[pyfunction]
    fn bisect_right(
        BisectArgs { a, x, lo, hi, key }: BisectArgs,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let (low, high) = parse_bounds(&lo, hi.as_ref(), vm)?;
        bisect_impl::<false>(&a, &x, low, high, key.as_ref(), vm)
    }

    #[pyfunction]
    fn insort_left(BisectArgs { a, x, lo, hi, key }: BisectArgs, vm: &VirtualMachine) -> PyResult {
        let (low, high) = parse_bounds(&lo, hi.as_ref(), vm)?;
        // The search runs on the key, the insert has to put back the item itself.
        let needle = match &key {
            Some(key) => Some(key.call((&*x,), vm)?),
            None => None,
        };
        let index = bisect_impl::<true>(
            &a,
            needle.as_deref().unwrap_or(&x),
            low,
            high,
            key.as_ref(),
            vm,
        );
        drop(needle);
        let index = index?;
        let result = vm.call_method(&a, "insert", (index, &*x))?;
        drop(result);
        Ok(vm.ctx.none())
    }

    #[pyfunction]
    fn insort_right(BisectArgs { a, x, lo, hi, key }: BisectArgs, vm: &VirtualMachine) -> PyResult {
        let (low, high) = parse_bounds(&lo, hi.as_ref(), vm)?;
        // The search runs on the key, the insert has to put back the item itself.
        let needle = match &key {
            Some(key) => Some(key.call((&*x,), vm)?),
            None => None,
        };
        let index = bisect_impl::<false>(
            &a,
            needle.as_deref().unwrap_or(&x),
            low,
            high,
            key.as_ref(),
            vm,
        );
        drop(needle);
        let index = index?;
        let result = vm.call_method(&a, "insert", (index, &*x))?;
        drop(result);
        Ok(vm.ctx.none())
    }
}
