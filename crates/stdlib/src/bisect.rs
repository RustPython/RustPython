pub(crate) use _bisect::module_def;

#[pymodule]
mod _bisect {
    use crate::vm::{
        PyObjectRef, PyResult, VirtualMachine, function::ArgIndex, protocol::PySequence,
        types::PyComparisonOp,
    };

    #[derive(FromArgs)]
    struct BisectArgs {
        a: PyObjectRef,
        x: PyObjectRef,
        #[pyarg(any, default = 0)]
        lo: ArgIndex,
        // None means the sequence length.
        #[pyarg(any, optional)]
        hi: Option<ArgIndex>,
        #[pyarg(named, optional)]
        key: Option<PyObjectRef>,
    }

    // Handles objects that implement __index__ and makes sure index fits in needed isize.
    #[inline]
    fn handle_default(arg: ArgIndex, vm: &VirtualMachine) -> PyResult<isize> {
        arg.into_int_ref().try_to_primitive(vm)
    }

    // The -1 sentinel is shared by an omitted/None hi and an explicit -1.
    // Other negative upper bounds have the same result as zero because lo is
    // nonnegative. Only the sentinel requires asking the sequence for length.
    #[inline]
    fn as_usize(
        lo: ArgIndex,
        hi: Option<ArgIndex>,
        sequence: PySequence<'_>,
        vm: &VirtualMachine,
    ) -> PyResult<(usize, usize)> {
        let lo = usize::try_from(handle_default(lo, vm)?)
            .map_err(|_| vm.new_value_error("lo must be non-negative"))?;
        let hi = match hi {
            Some(value) => handle_default(value, vm)?,
            None => -1,
        };
        let hi = if hi == -1 {
            sequence.length(vm)?
        } else {
            usize::try_from(hi).unwrap_or(0)
        };
        Ok((lo, hi))
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
    #[pyfunction]
    fn bisect_left(
        BisectArgs { a, x, lo, hi, key }: BisectArgs,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let sequence = a.sequence_unchecked();
        let (mut lo, mut hi) = as_usize(lo, hi, sequence, vm)?;
        let get_item = sequence_item(sequence, vm)?;

        while lo < hi {
            // Handles issue 13496.
            let mid = (lo + hi) / 2;
            let a_mid = get_item(sequence, mid as isize, vm)?;
            let comp = if let Some(ref key) = key {
                key.call((a_mid,), vm)?
            } else {
                a_mid
            };
            if comp.rich_compare_bool(&x, PyComparisonOp::Lt, vm)? {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        Ok(lo)
    }

    #[inline]
    #[pyfunction]
    fn bisect_right(
        BisectArgs { a, x, lo, hi, key }: BisectArgs,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let sequence = a.sequence_unchecked();
        let (mut lo, mut hi) = as_usize(lo, hi, sequence, vm)?;
        let get_item = sequence_item(sequence, vm)?;

        while lo < hi {
            // Handles issue 13496.
            let mid = (lo + hi) / 2;
            let a_mid = get_item(sequence, mid as isize, vm)?;
            let comp = if let Some(ref key) = key {
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

    #[pyfunction]
    fn insort_left(BisectArgs { a, x, lo, hi, key }: BisectArgs, vm: &VirtualMachine) -> PyResult {
        // The search runs on the key, the insert has to put back the item itself.
        let needle = match key {
            Some(ref key) => key.call((x.clone(),), vm)?,
            None => x.clone(),
        };
        let index = bisect_left(
            BisectArgs {
                a: a.clone(),
                x: needle,
                lo,
                hi,
                key,
            },
            vm,
        )?;
        vm.call_method(&a, "insert", (index, x))
    }

    #[pyfunction]
    fn insort_right(BisectArgs { a, x, lo, hi, key }: BisectArgs, vm: &VirtualMachine) -> PyResult {
        // The search runs on the key, the insert has to put back the item itself.
        let needle = match key {
            Some(ref key) => key.call((x.clone(),), vm)?,
            None => x.clone(),
        };
        let index = bisect_right(
            BisectArgs {
                a: a.clone(),
                x: needle,
                lo,
                hi,
                key,
            },
            vm,
        )?;
        vm.call_method(&a, "insert", (index, x))
    }
}
