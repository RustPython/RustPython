use super::pyobject::{PyObject, PyObjectKind, PyObjectRef, PyResult};

pub fn get_pos(l: &Vec<PyObjectRef>, p: i32) -> usize {
    if p < 0 {
        l.len() - ((-p) as usize)
    } else {
        p as usize
    }
}

pub fn get_slice_items(l: &Vec<PyObjectRef>, slice: &PyObjectRef) -> Vec<PyObjectRef> {
    // TODO: we could potentially avoid this copy and use slice
    match &(slice.borrow()).kind {
        PyObjectKind::Slice { start, stop, step } => {
            let start = match start {
                &Some(start) => get_pos(l, start),
                &None => 0,
            };
            let stop = match stop {
                &Some(stop) => get_pos(l, stop),
                &None => l.len() as usize,
            };
            match step {
                &None | &Some(1) => l[start..stop].to_vec(),
                &Some(num) => {
                    if num < 0 {
                        unimplemented!("negative step indexing not yet supported")
                    };
                    l[start..stop]
                        .iter()
                        .step_by(num as usize)
                        .cloned()
                        .collect()
                }
            }
        }
        kind => panic!("get_slice_items called with non-slice: {:?}", kind),
    }
}
