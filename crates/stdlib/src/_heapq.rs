pub(crate) use _heapq::module_def;

#[pymodule]
mod _heapq {

    use crate::vm::{
        AsObject, PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyList, PyListRef},
        types::PyComparisonOp,
    };

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

    fn siftup(heap: &PyListRef, mut pos: usize, vm: &VirtualMachine) -> PyResult<()> {
        let endpos = heap.__len__();
        let startpos = pos;

        if pos >= endpos {
            return Err(vm.new_index_error("index out of range"));
        };

        let limit = endpos >> 1; // smallest pos that has no child

        while pos < limit {
            // Set childpos to index of smaller child.
            let mut childpos = 2 * pos + 1; // leftmost child position

            if childpos + 1 < endpos {
                let (left, right) = {
                    let vec = heap.borrow_vec();
                    (vec[childpos].clone(), vec[childpos + 1].clone())
                };

                let cmp = left.rich_compare_bool(&right, PyComparisonOp::Lt, vm)?;

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

    const fn keep_top_bit(mut n: usize) -> usize {
        let mut i = 0;

        while n > 1 {
            n >>= 1;
            i += 1;
        }

        n << i
    }

    fn cache_friendly_heapify(heap: &PyListRef, vm: &VirtualMachine) -> PyResult<()> {
        let m = heap.__len__() >> 1; // index of first childless node
        let leftmost = keep_top_bit(m + 1) - 1; // leftmost node in row of m 
        let mhalf = m >> 1; // parent of first childless node

        for i in (mhalf..leftmost).rev() {
            let mut j = i;

            loop {
                siftup(heap, j, vm)?;

                if j & 1 == 0 {
                    break;
                }

                j >>= 1;
            }
        }

        for i in (leftmost..m).rev() {
            let mut j = i;

            loop {
                siftup(heap, j, vm)?;

                if j & 1 == 0 {
                    break;
                }

                j >>= 1;
            }
        }

        Ok(())
    }

    fn heapify_internal(heap: &PyListRef, vm: &VirtualMachine) -> PyResult<()> {
        let n = heap.__len__();

        if n > 2500 {
            return cache_friendly_heapify(heap, vm);
        }

        for i in (0..(n >> 1)).rev() {
            siftup(heap, i, vm)?;
        }

        Ok(())
    }

    #[pyfunction]
    fn heappush(heap: PyObjectRef, item: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let lst = heap.downcast::<PyList>().map_err(|obj| {
            vm.new_type_error(format!(
                "heappush() argument 1 must be list, not {}",
                obj.class().name()
            ))
        })?;

        {
            let mut vec = lst.borrow_vec_mut();
            vec.push(item);
        }

        let size = lst.__len__();

        siftdown(&lst, 0, size - 1, vm)
    }

    #[pyfunction]
    fn heappop(heap: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let lst = heap.downcast::<PyList>().map_err(|obj| {
            vm.new_type_error(format!(
                "heappop() argument 1 must be list, not {}",
                obj.class().name()
            ))
        })?;

        let Some(lastelt) = lst.borrow_vec_mut().pop() else {
            return Err(vm.new_index_error("index out of range"));
        };

        if lst.borrow_vec().is_empty() {
            return Ok(lastelt);
        };

        let returnitem = {
            let mut vec = lst.borrow_vec_mut();
            let root = vec[0].clone();
            vec[0] = lastelt;
            root
        };

        siftup(&lst, 0, vm)?;
        Ok(returnitem)
    }

    #[pyfunction]
    fn heapify(heap: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let lst = heap.downcast::<PyList>().map_err(|obj| {
            vm.new_type_error(format!(
                "heapify() argument 1 must be list, not {}",
                obj.class().name()
            ))
        })?;

        heapify_internal(&lst, vm)
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
}
