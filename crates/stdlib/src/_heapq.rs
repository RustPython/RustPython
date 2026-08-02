// cspell:ignore siftup siftdown mhalf
pub(crate) use _heapq::module_def;

#[pymodule]
mod _heapq {

    use crate::vm::{
        AsObject, PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyList, PyListRef},
        types::PyComparisonOp,
    };

    /// [CPython's siftdown](https://github.com/python/cpython/blob/v3.14.5/Modules/_heapqmodule.c#L25-L68)
    fn siftdown(
        heap: &PyListRef,
        startpos: usize,
        mut pos: usize,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let size = heap.__len__();

        let newitem = match heap.borrow_vec().get(pos) {
            Some(v) => v.clone(),
            None => return Err(vm.new_index_error("index out of range")),
        };

        // Follow the path to the root, moving parents down until finding
        // a place newitem fits.
        while pos > startpos {
            let parentpos = (pos - 1) >> 1;
            let parent = {
                let vec = heap.borrow_vec();
                vec[parentpos].clone()
            };

            let cmp = newitem.rich_compare_bool(&parent, PyComparisonOp::Lt, vm)?;

            if size != heap.__len__() {
                return Err(vm.new_runtime_error("list changed size during iteration"));
            }

            if !cmp {
                break;
            }

            {
                let mut vec = heap.borrow_vec_mut();
                vec.swap(pos, parentpos);
            }

            pos = parentpos;
        }

        Ok(())
    }

    /// [CPython's siftup](https://github.com/python/cpython/blob/v3.14.5/Modules/_heapqmodule.c#L70-L118)
    fn siftup(heap: &PyListRef, mut pos: usize, vm: &VirtualMachine) -> PyResult<()> {
        let endpos = heap.__len__();
        let startpos = pos;

        if pos >= endpos {
            return Err(vm.new_index_error("index out of range"));
        };

        let limit = endpos >> 1; // smallest pos that has no child

        // Bubble up the smaller child until hitting a leaf.
        while pos < limit {
            // Set childpos to index of smaller child.
            let mut childpos = 2 * pos + 1; // leftmost child position

            if childpos + 1 < endpos {
                let (a, b) = {
                    let vec = heap.borrow_vec();
                    (vec[childpos].clone(), vec[childpos + 1].clone())
                };

                let cmp = a.rich_compare_bool(&b, PyComparisonOp::Lt, vm)?;

                if endpos != heap.__len__() {
                    return Err(vm.new_runtime_error("list changed size during iteration"));
                }

                if !cmp {
                    childpos += 1;
                }
            }

            {
                // Move the smaller child up
                let mut vec = heap.borrow_vec_mut();
                vec.swap(pos, childpos);
            }

            pos = childpos;
        }

        // Bubble it up to its final resting place (by sifting its parents down)
        siftdown(heap, startpos, pos, vm)
    }

    /// configurable `sift_func` for doing heappush operation.
    ///
    /// A generic implementation of:
    /// - [CPython's _heapq_heappush_impl](https://github.com/python/cpython/blob/v3.14.5/Modules/_heapqmodule.c#L131-L150)
    /// - [CPython's _heapq_heappush_max_impl](https://github.com/python/cpython/blob/v3.14.5/Modules/_heapqmodule.c#L512-L532)
    fn heappush_internal<F>(
        heap: &PyListRef,
        item: PyObjectRef,
        siftdown_func: F,
        vm: &VirtualMachine,
    ) -> PyResult<()>
    where
        F: Fn(&PyListRef, usize, usize, &VirtualMachine) -> PyResult<()>,
    {
        {
            let mut vec = heap.borrow_vec_mut();
            vec.push(item);
        }

        let size = heap.__len__();

        siftdown_func(heap, 0, size - 1, vm)
    }

    #[pyfunction]
    fn heappush(heap: PyObjectRef, item: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let lst = heap.downcast::<PyList>().map_err(|obj| {
            vm.new_type_error(format!(
                "heappush() argument 1 must be list, not {}",
                obj.class().name()
            ))
        })?;

        heappush_internal(&lst, item, siftdown, vm)
    }

    /// [CPython's heappop_internal](https://github.com/python/cpython/blob/v3.14.5/Modules/_heapqmodule.c#L152-L183)
    fn heappop_internal<F>(
        heap: &PyListRef,
        siftup_func: F,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef>
    where
        F: Fn(&PyListRef, usize, &VirtualMachine) -> PyResult<()>,
    {
        let Some(lastelt) = heap.borrow_vec_mut().pop() else {
            return Err(vm.new_index_error("index out of range"));
        };

        if heap.borrow_vec().is_empty() {
            return Ok(lastelt);
        };

        let returnitem = {
            let mut vec = heap.borrow_vec_mut();
            let root = vec[0].clone();
            vec[0] = lastelt;
            root
        };

        siftup_func(heap, 0, vm)?;
        Ok(returnitem)
    }

    #[pyfunction]
    fn heappop(heap: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let lst = heap.downcast::<PyList>().map_err(|obj| {
            vm.new_type_error(format!(
                "heappop() argument 1 must be list, not {}",
                obj.class().name()
            ))
        })?;

        heappop_internal(&lst, siftup, vm)
    }

    /// [CPython's heapreplace_internal](https://github.com/python/cpython/blob/v3.14.5/Modules/_heapqmodule.c#L202-L220)
    fn heapreplace_internal<F>(
        heap: &PyListRef,
        item: PyObjectRef,
        siftup_func: F,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef>
    where
        F: Fn(&PyListRef, usize, &VirtualMachine) -> PyResult<()>,
    {
        let returnitem = {
            let mut vec = heap.borrow_vec_mut();
            let root = match vec.first() {
                Some(v) => v.clone(),
                None => return Err(vm.new_index_error("index out of range")),
            };

            vec[0] = item;
            root
        };

        siftup_func(heap, 0, vm)?;
        Ok(returnitem)
    }

    #[pyfunction]
    fn heapreplace(
        heap: PyObjectRef,
        item: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let lst = heap.downcast::<PyList>().map_err(|obj| {
            vm.new_type_error(format!(
                "heapreplace() argument 1 must be list, not {}",
                obj.class().name()
            ))
        })?;

        heapreplace_internal(&lst, item, siftup, vm)
    }

    #[pyfunction]
    fn heappushpop(
        heap: PyObjectRef,
        item: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let lst = heap.downcast::<PyList>().map_err(|obj| {
            vm.new_type_error(format!(
                "heappushpop() argument 1 must be list, not {}",
                obj.class().name()
            ))
        })?;

        let top = {
            let vec = lst.borrow_vec();
            match vec.first() {
                Some(v) => v.clone(),
                None => return Ok(item),
            }
        };

        let cmp = top.rich_compare_bool(&item, PyComparisonOp::Lt, vm)?;
        if !cmp {
            return Ok(item);
        }

        let returnitem = {
            let mut vec = lst.borrow_vec_mut();
            let root = match vec.first() {
                Some(v) => v.clone(),
                None => return Err(vm.new_index_error("index out of range")),
            };

            vec[0] = item;
            root
        };

        siftup(&lst, 0, vm)?;
        Ok(returnitem)
    }

    /// [CPython's keep_top_bit](https://github.com/python/cpython/blob/v3.14.5/Modules/_heapqmodule.c#L299-L309)
    const fn keep_top_bit(mut n: usize) -> usize {
        let mut i = 0;

        while n > 1 {
            n >>= 1;
            i += 1;
        }

        n << i
    }

    /// [CPython's cache_friendly_heapify](https://github.com/python/cpython/blob/v3.14.5/Modules/_heapqmodule.c#L311-L362)
    fn cache_friendly_heapify<F>(
        heap: &PyListRef,
        siftup_func: F,
        vm: &VirtualMachine,
    ) -> PyResult<()>
    where
        F: Fn(&PyListRef, usize, &VirtualMachine) -> PyResult<()>,
    {
        let m = heap.__len__() >> 1; // index of first childless node
        let leftmost = keep_top_bit(m + 1) - 1; // leftmost node in row of m 
        let mhalf = m >> 1; // parent of first childless node

        for i in (mhalf..leftmost).rev() {
            let mut j = i;

            loop {
                siftup_func(heap, j, vm)?;

                if j & 1 == 0 {
                    break;
                }

                j >>= 1;
            }
        }

        for i in (leftmost..m).rev() {
            let mut j = i;

            loop {
                siftup_func(heap, j, vm)?;

                if j & 1 == 0 {
                    break;
                }

                j >>= 1;
            }
        }

        Ok(())
    }

    /// [CPython's heapify_internal](https://github.com/python/cpython/blob/v3.14.5/Modules/_heapqmodule.c#L364-L388)
    fn heapify_internal<F>(heap: &PyListRef, siftup_func: F, vm: &VirtualMachine) -> PyResult<()>
    where
        F: Fn(&PyListRef, usize, &VirtualMachine) -> PyResult<()>,
    {
        let n = heap.__len__();

        if n > 2500 {
            return cache_friendly_heapify(heap, siftup_func, vm);
        }

        for i in (0..(n >> 1)).rev() {
            siftup_func(heap, i, vm)?;
        }

        Ok(())
    }

    #[pyfunction]
    fn heapify(heap: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let lst = heap.downcast::<PyList>().map_err(|obj| {
            vm.new_type_error(format!(
                "heapify() argument 1 must be list, not {}",
                obj.class().name()
            ))
        })?;

        heapify_internal(&lst, siftup, vm)
    }

    /// [CPython's siftdown_max](https://github.com/python/cpython/blob/v3.14.5/Modules/_heapqmodule.c#L407-L449)
    fn siftdown_max(
        heap: &PyListRef,
        startpos: usize,
        mut pos: usize,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let size = heap.__len__();

        let newitem = match heap.borrow_vec().get(pos) {
            Some(v) => v.clone(),
            None => return Err(vm.new_index_error("index out of range")),
        };

        // Follow the path to the root, moving parents down until finding
        // a place newitem fits.
        while pos > startpos {
            let parentpos = (pos - 1) >> 1;
            let parent = {
                let vec = heap.borrow_vec();
                vec[parentpos].clone()
            };

            let cmp = parent.rich_compare_bool(&newitem, PyComparisonOp::Lt, vm)?;

            if size != heap.__len__() {
                return Err(vm.new_runtime_error("list changed size during iteration"));
            }

            if !cmp {
                break;
            }

            {
                let mut vec = heap.borrow_vec_mut();
                vec.swap(pos, parentpos);
            }

            pos = parentpos;
        }

        Ok(())
    }

    /// [CPython's siftup_max](https://github.com/python/cpython/blob/v3.14.5/Modules/_heapqmodule.c#L451-L499)
    fn siftup_max(heap: &PyListRef, mut pos: usize, vm: &VirtualMachine) -> PyResult<()> {
        let endpos = heap.__len__();
        let startpos = pos;

        if pos >= endpos {
            return Err(vm.new_index_error("index out of range"));
        };

        let limit = endpos >> 1; // smallest pos that has no child

        // Bubble up the larger child until hitting a leaf.
        while pos < limit {
            // Set childpos to index of larger child.
            let mut childpos = 2 * pos + 1; // leftmost child position

            if childpos + 1 < endpos {
                let (a, b) = {
                    let vec = heap.borrow_vec();
                    (vec[childpos + 1].clone(), vec[childpos].clone())
                };

                let cmp = a.rich_compare_bool(&b, PyComparisonOp::Lt, vm)?;

                if endpos != heap.__len__() {
                    return Err(vm.new_runtime_error("list changed size during iteration"));
                }

                if !cmp {
                    childpos += 1;
                }
            }

            {
                // Move the larger child up
                let mut vec = heap.borrow_vec_mut();
                vec.swap(pos, childpos);
            }

            pos = childpos;
        }

        // Bubble it up to its final resting place (by sifting its parents down)
        siftdown_max(heap, startpos, pos, vm)
    }

    #[pyfunction]
    fn heappush_max(heap: PyObjectRef, item: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let lst = heap.downcast::<PyList>().map_err(|obj| {
            vm.new_type_error(format!(
                "heappush_max() argument 1 must be list, not {}",
                obj.class().name()
            ))
        })?;

        heappush_internal(&lst, item, siftdown_max, vm)
    }

    #[pyfunction]
    fn heappop_max(heap: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let lst = heap.downcast::<PyList>().map_err(|obj| {
            vm.new_type_error(format!(
                "heappop_max() argument 1 must be list, not {}",
                obj.class().name()
            ))
        })?;

        heappop_internal(&lst, siftup_max, vm)
    }

    #[pyfunction]
    fn heapreplace_max(
        heap: PyObjectRef,
        item: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let lst = heap.downcast::<PyList>().map_err(|obj| {
            vm.new_type_error(format!(
                "heapreplace_max() argument 1 must be list, not {}",
                obj.class().name()
            ))
        })?;

        heapreplace_internal(&lst, item, siftup_max, vm)
    }

    #[pyfunction]
    fn heapify_max(heap: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let lst = heap.downcast::<PyList>().map_err(|obj| {
            vm.new_type_error(format!(
                "heapify_max() argument 1 must be list, not {}",
                obj.class().name()
            ))
        })?;

        heapify_internal(&lst, siftup_max, vm)
    }

    #[pyfunction]
    fn heappushpop_max(
        heap: PyObjectRef,
        item: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let lst = heap.downcast::<PyList>().map_err(|obj| {
            vm.new_type_error(format!(
                "heappushpop_max() argument 1 must be list, not {}",
                obj.class().name()
            ))
        })?;

        let top = {
            let vec = lst.borrow_vec();
            match vec.first() {
                Some(v) => v.clone(),
                None => return Ok(item),
            }
        };

        let cmp = item.rich_compare_bool(&top, PyComparisonOp::Lt, vm)?;
        if !cmp {
            return Ok(item);
        }

        let returnitem = {
            let mut vec = lst.borrow_vec_mut();
            let root = match vec.first() {
                Some(v) => v.clone(),
                None => return Err(vm.new_index_error("index out of range")),
            };

            vec[0] = item;
            root
        };

        siftup_max(&lst, 0, vm)?;
        Ok(returnitem)
    }
}
