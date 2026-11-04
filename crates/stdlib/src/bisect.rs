pub(crate) use _bisect::module_def;

#[pymodule]
mod _bisect {
    use crate::vm::{
        AsObject, PyObject, PyObjectRef, PyResult, TryFromObject, VirtualMachine,
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
        let to_index = |obj: &PyObjectRef| {
            ArgIndex::try_from_object(vm, obj.clone())?
                .into_int_ref()
                .try_to_primitive::<isize>(vm)
        };
        let lo = match lo {
            OptionalArg::Missing => 0,
            OptionalArg::Present(obj) => to_index(obj)?,
        };
        let lo = usize::try_from(lo).map_err(|_| vm.new_value_error("lo cannot be negative"))?;
        let hi = match hi {
            Some(obj) => to_index(obj)?,
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
        // Keep the original needle type alive if a callback changes __class__.
        // The same-type shortcut uses its slot, not the midpoint's current slot.
        let needle_type = x.class().to_owned();
        let mut compare = needle_type.slots().richcompare.load();

        while lo < hi {
            // Handles issue 13496.
            let mid = (lo + hi) / 2;
            let a_mid = get_item(sequence, mid as isize, vm)?;
            let comp = if let Some(key) = key {
                // The callback may delete its argument. Release our reference
                // only after it returns, including when it raises an exception.
                let result = key.call((&*a_mid,), vm);
                drop(a_mid);
                result?
            } else {
                a_mid
            };
            let (left, right) = if LEFT { (&*comp, x) } else { (x, &*comp) };
            let direct = if let Some(compare) = compare.filter(|_| comp.class().is(&needle_type)) {
                match compare(left, right, PyComparisonOp::Lt, vm)? {
                    Either::A(obj) if obj.is(&vm.ctx.not_implemented) => None,
                    Either::A(obj) => Some(obj.try_to_bool(vm)?),
                    Either::B(PyComparisonValue::Implemented(value)) => Some(value),
                    Either::B(PyComparisonValue::NotImplemented) => None,
                }
            } else {
                // A different type does not disable the same-type shortcut
                // for subsequent midpoint objects.
                Some(left.rich_compare_bool(right, PyComparisonOp::Lt, vm)?)
            };
            let less = match direct {
                Some(value) => value,
                None => {
                    // NotImplemented disables the shortcut for this search.
                    // Generic dispatch retries __lt__ before reflected __gt__.
                    compare = None;
                    left.rich_compare_bool(right, PyComparisonOp::Lt, vm)?
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
        }
        Ok(lo)
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
            Some(key) => key.call((x.clone(),), vm)?,
            None => x.clone(),
        };
        let index = bisect_impl::<true>(&a, &needle, low, high, key.as_ref(), vm);
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
            Some(key) => key.call((x.clone(),), vm)?,
            None => x.clone(),
        };
        let index = bisect_impl::<false>(&a, &needle, low, high, key.as_ref(), vm);
        drop(needle);
        let index = index?;
        let result = vm.call_method(&a, "insert", (index, &*x))?;
        drop(result);
        Ok(vm.ctx.none())
    }
}
