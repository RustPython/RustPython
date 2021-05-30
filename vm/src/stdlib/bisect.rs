pub(crate) use _bisect::make_module;

#[pymodule]
mod _bisect {
    use std::convert::TryFrom;

    use crate::function::OptionalArg;
    use crate::slots::PyComparisonOp::Lt;
    use crate::vm::VirtualMachine;
    use crate::{ItemProtocol, PyObjectRef, PyResult};
    use num_traits::ToPrimitive;

    #[derive(FromArgs)]
    struct BisectArgs {
        #[pyarg(any, name = "a")]
        a: PyObjectRef,
        #[pyarg(any, name = "x")]
        x: PyObjectRef,
        #[pyarg(any, optional, name = "lo")]
        lo: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional, name = "hi")]
        hi: OptionalArg<PyObjectRef>,
    }

    // Handles objects that implement __index__ and makes sure index fits in needed isize.
    #[inline]
    fn handle_default(
        arg: OptionalArg<PyObjectRef>,
        default: Option<isize>,
        vm: &VirtualMachine,
    ) -> PyResult<Option<isize>> {
        Ok(match arg {
            OptionalArg::Present(v) => {
                Some(vm.to_index(&v)?.as_bigint().to_isize().ok_or_else(|| {
                    vm.new_index_error("cannot fit 'int' into an index-sized integer".to_owned())
                })?)
            }
            OptionalArg::Missing => default,
        })
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
        let lo = handle_default(lo, Some(0), vm)?
            .map(|value| {
                if value < 0 {
                    return Err(vm.new_value_error("lo must be non-negative".to_owned()));
                }
                Ok(usize::try_from(value).unwrap())
            })
            .unwrap()?;
        let hi = handle_default(hi, None, vm)?
            .map(|value| {
                if value < 0 {
                    0
                } else {
                    // Can't fail.
                    usize::try_from(value).unwrap()
                }
            })
            .unwrap_or(seq_len);
        Ok((lo, hi))
    }

    #[inline]
    fn bisect_left_impl(
        BisectArgs { a, x, lo, hi }: BisectArgs,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let (mut lo, mut hi) = as_usize(lo, hi, vm.obj_len(&a)?, vm)?;

        while lo < hi {
            // Handles issue 13496.
            let mid = (lo + hi) / 2;
            if vm.bool_cmp(&a.get_item(mid, vm)?, &x, Lt)? {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        Ok(lo)
    }

    #[inline]
    fn bisect_right_impl(
        BisectArgs { a, x, lo, hi }: BisectArgs,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let (mut lo, mut hi) = as_usize(lo, hi, vm.obj_len(&a)?, vm)?;

        while lo < hi {
            // Handles issue 13496.
            let mid = (lo + hi) / 2;
            if vm.bool_cmp(&x, &a.get_item(mid, vm)?, Lt)? {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        Ok(lo)
    }

    /// Return the index where to insert item x in list a, assuming a is sorted.
    ///
    /// The return value i is such that all e in a[:i] have e < x, and all e in
    /// a[i:] have e >= x.  So if x already appears in the list, a.insert(x) will
    /// insert just before the leftmost x already there.
    ///
    /// Optional args lo (default 0) and hi (default len(a)) bound the
    /// slice of a to be searched.
    #[pyfunction]
    fn bisect_left(args: BisectArgs, vm: &VirtualMachine) -> PyResult<usize> {
        bisect_left_impl(args, vm)
    }

    /// Return the index where to insert item x in list a, assuming a is sorted.
    ///
    /// The return value i is such that all e in a[:i] have e <= x, and all e in
    /// a[i:] have e > x.  So if x already appears in the list, a.insert(x) will
    /// insert just after the rightmost x already there.
    ///
    /// Optional args lo (default 0) and hi (default len(a)) bound the
    /// slice of a to be searched.
    #[pyfunction]
    fn bisect_right(args: BisectArgs, vm: &VirtualMachine) -> PyResult<usize> {
        bisect_right_impl(args, vm)
    }

    /// Insert item x in list a, and keep it sorted assuming a is sorted.
    ///
    /// If x is already in a, insert it to the left of the leftmost x.
    ///
    /// Optional args lo (default 0) and hi (default len(a)) bound the
    /// slice of a to be searched.
    #[pyfunction]
    fn insort_left(BisectArgs { a, x, lo, hi }: BisectArgs, vm: &VirtualMachine) -> PyResult {
        let index = bisect_left_impl(
            BisectArgs {
                a: a.clone(),
                x: x.clone(),
                lo,
                hi,
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
    fn insort_right(BisectArgs { a, x, lo, hi }: BisectArgs, vm: &VirtualMachine) -> PyResult {
        let index = bisect_right_impl(
            BisectArgs {
                a: a.clone(),
                x: x.clone(),
                lo,
                hi,
            },
            vm,
        )?;
        vm.call_method(&a, "insert", (index, x))
    }
}
