pub(crate) use _bisect::module_def;

#[pymodule]
mod _bisect {
    use crate::vm::{
        PyObject, PyObjectRef, PyResult, VirtualMachine, function::OptionalArg,
        protocol::PySequence, types::PyComparisonOp,
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
            OptionalArg::Present(obj) => obj.try_index(vm)?.try_to_primitive::<isize>(vm)?,
        };
        let lo = usize::try_from(lo).map_err(|_| vm.new_value_error("lo cannot be negative"))?;
        let hi = match hi {
            Some(obj) => obj.try_index(vm)?.try_to_primitive::<isize>(vm)?,
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
                let mut end = name.len().min(200);
                while !name.is_char_boundary(end) {
                    end -= 1;
                }
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
    fn bisect_left_impl(
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

        while lo < hi {
            // Handles issue 13496.
            let mid = (lo + hi) / 2;
            let a_mid = get_item(sequence, mid as isize, vm)?;
            let comp = if let Some(key) = key {
                key.call((a_mid,), vm)?
            } else {
                a_mid
            };
            if comp.rich_compare_bool(x, PyComparisonOp::Lt, vm)? {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        Ok(lo)
    }

    #[inline]
    fn bisect_right_impl(
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

        while lo < hi {
            // Handles issue 13496.
            let mid = (lo + hi) / 2;
            let a_mid = get_item(sequence, mid as isize, vm)?;
            let comp = if let Some(key) = key {
                key.call((a_mid,), vm)?
            } else {
                a_mid
            };
            if x.rich_compare_bool(&comp, PyComparisonOp::Lt, vm)? {
                hi = mid;
            } else {
                lo = mid + 1;
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
        bisect_left_impl(&a, &x, low, high, key.as_ref(), vm)
    }

    #[inline]
    #[pyfunction]
    fn bisect_right(
        BisectArgs { a, x, lo, hi, key }: BisectArgs,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let (low, high) = parse_bounds(&lo, hi.as_ref(), vm)?;
        bisect_right_impl(&a, &x, low, high, key.as_ref(), vm)
    }

    #[pyfunction]
    fn insort_left(BisectArgs { a, x, lo, hi, key }: BisectArgs, vm: &VirtualMachine) -> PyResult {
        let (low, high) = parse_bounds(&lo, hi.as_ref(), vm)?;
        // The search runs on the key, the insert has to put back the item itself.
        let needle = match &key {
            Some(key) => key.call((x.clone(),), vm)?,
            None => x.clone(),
        };
        let index = bisect_left_impl(&a, &needle, low, high, key.as_ref(), vm);
        drop(needle);
        let index = index?;
        vm.call_method(&a, "insert", (index, x))
    }

    #[pyfunction]
    fn insort_right(BisectArgs { a, x, lo, hi, key }: BisectArgs, vm: &VirtualMachine) -> PyResult {
        let (low, high) = parse_bounds(&lo, hi.as_ref(), vm)?;
        // The search runs on the key, the insert has to put back the item itself.
        let needle = match &key {
            Some(key) => key.call((x.clone(),), vm)?,
            None => x.clone(),
        };
        let index = bisect_right_impl(&a, &needle, low, high, key.as_ref(), vm);
        drop(needle);
        let index = index?;
        vm.call_method(&a, "insert", (index, x))
    }
}
