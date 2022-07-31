pub(crate) use _bisect::make_module;

#[pymodule]
mod _bisect {
    use crate::vm::{
        function::OptionalArg, types::PyComparisonOp, PyObjectRef, PyResult, VirtualMachine,
    };

    #[derive(FromArgs)]
    struct BisectArgs {
        a: PyObjectRef,
        x: PyObjectRef,
        #[pyarg(any, optional)]
        lo: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        hi: OptionalArg<PyObjectRef>,
        #[pyarg(named, default)]
        key: Option<PyObjectRef>,
    }

    // Handles objects that implement __index__ and makes sure index fits in needed isize.
    #[inline]
    fn handle_default(
        arg: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<Option<isize>> {
        arg.into_option()
            .map(|v| v.try_index(vm)?.try_to_primitive(vm))
            .transpose()
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
        lo: OptionalArg<PyObjectRef>,
        hi: OptionalArg<PyObjectRef>,
        seq_len: usize,
        vm: &VirtualMachine,
    ) -> PyResult<(usize, usize)> {
        // We only deal with positives for lo, try_from can't fail.
        // Default is always a Some so we can safely unwrap.
        let lo = handle_default(lo, vm)?
            .map(|value| {
                usize::try_from(value)
                    .map_err(|_| vm.new_value_error("lo must be non-negative".to_owned()))
            })
            .unwrap_or(Ok(0))?;
        let hi = handle_default(hi, vm)?
            .map(|value| usize::try_from(value).unwrap_or(0))
            .unwrap_or(seq_len);
        Ok((lo, hi))
    }

    /// Return the index where to insert item x in list a, assuming a is sorted.
    ///
    /// The return value i is such that all e in a[:i] have e < x, and all e in
    /// a[i:] have e >= x.  So if x already appears in the list, a.insert(x) will
    /// insert just before the leftmost x already there.
    ///
    /// Optional args lo (default 0) and hi (default len(a)) bound the
    /// slice of a to be searched.
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
                vm.invoke(key, (a_mid,))?
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

    /// Return the index where to insert item x in list a, assuming a is sorted.
    ///
    /// The return value i is such that all e in a[:i] have e <= x, and all e in
    /// a[i:] have e > x.  So if x already appears in the list, a.insert(x) will
    /// insert just after the rightmost x already there.
    ///
    /// Optional args lo (default 0) and hi (default len(a)) bound the
    /// slice of a to be searched.
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
                vm.invoke(key, (a_mid,))?
            } else {
                a_mid
            };
            if x.rich_compare_bool(&*comp, PyComparisonOp::Lt, vm)? {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        Ok(lo)
    }

    /// Insert item x in list a, and keep it sorted assuming a is sorted.
    ///
    /// If x is already in a, insert it to the left of the leftmost x.
    ///
    /// Optional args lo (default 0) and hi (default len(a)) bound the
    /// slice of a to be searched.
    #[pyfunction]
    fn insort_left(BisectArgs { a, x, lo, hi, key }: BisectArgs, vm: &VirtualMachine) -> PyResult {
        let x = if let Some(ref key) = key {
            vm.invoke(key, (x,))?
        } else {
            x
        };
        let index = bisect_left(
            BisectArgs {
                a: a.clone(),
                x: x.clone(),
                lo,
                hi,
                key,
            },
            vm,
        )?;
        vm.call_method(&a, "insert", (index, x))
    }

    /// Insert item x in list a, and keep it sorted assuming a is sorted.
    ///
    /// If x is already in a, insert it to the right of the rightmost x.
    ///
    /// Optional args lo (default 0) and hi (default len(a)) bound the
    /// slice of a to be searched
    #[pyfunction]
    fn insort_right(BisectArgs { a, x, lo, hi, key }: BisectArgs, vm: &VirtualMachine) -> PyResult {
        let x = if let Some(ref key) = key {
            vm.invoke(key, (x,))?
        } else {
            x
        };
        let index = bisect_right(
            BisectArgs {
                a: a.clone(),
                x: x.clone(),
                lo,
                hi,
                key,
            },
            vm,
        )?;
        vm.call_method(&a, "insert", (index, x))
    }
}
