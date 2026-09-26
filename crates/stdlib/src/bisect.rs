pub(crate) use _bisect::module_def;

#[pymodule]
mod _bisect {
    use crate::vm::{
        PyObjectRef, PyResult, VirtualMachine, function::ArgIndex, types::PyComparisonOp,
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

    // Handles defaults for lo, hi.
    //
    //  - lo must be >= 0 with a default of 0.
    //  - hi, while it could be negative, defaults to `0` and the same effect is achieved
    //    (while loop isn't entered); this way we keep it a usize (and, if my understanding
    //    is correct, issue 13496 is handled). Its default value is set to the length of the
    //    input sequence.
    #[inline]
    fn as_usize(
        lo: ArgIndex,
        hi: Option<ArgIndex>,
        seq_len: usize,
        vm: &VirtualMachine,
    ) -> PyResult<(usize, usize)> {
        // We only deal with positives for lo, try_from can't fail.
        let lo = usize::try_from(handle_default(lo, vm)?)
            .map_err(|_| vm.new_value_error("lo must be non-negative"))?;
        let hi = match hi {
            Some(value) => {
                let value: isize = value.into_int_ref().try_to_primitive(vm)?;
                usize::try_from(value).unwrap_or(0)
            }
            None => seq_len,
        };
        Ok((lo, hi))
    }

    #[inline]
    #[pyfunction]
    fn bisect_left(
        BisectArgs { a, x, lo, hi, key }: BisectArgs,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let (mut lo, mut hi) = as_usize(lo, hi, a.length(vm)?, vm)?;

        while lo < hi {
            // Handles issue 13496.
            let mid = (lo + hi) / 2;
            let a_mid = a.get_item(&mid, vm)?;
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
        let (mut lo, mut hi) = as_usize(lo, hi, a.length(vm)?, vm)?;

        while lo < hi {
            // Handles issue 13496.
            let mid = (lo + hi) / 2;
            let a_mid = a.get_item(&mid, vm)?;
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
