use super::pyobject::{Executor, PyContext, PyObject, PyObjectKind, PyObjectRef, PyResult};

fn get_pos(l: &Vec<PyObjectRef>, p: i32) -> usize {
    if p < 0 {
        l.len() - ((-p) as usize)
    } else {
        p as usize
    }
}

pub fn subscript(rt: &mut Executor, l: &Vec<PyObjectRef>, b: PyObjectRef) -> PyResult {
    match &(b.borrow()).kind {
        PyObjectKind::Integer { value } => {
            let pos_index = get_pos(l, *value);
            let obj = l[pos_index].clone();
            Ok(obj)
        }
        PyObjectKind::Slice { start, stop, step } => {
            let start = match start {
                &Some(start) => get_pos(l, start),
                &None => 0,
            };
            let stop = match stop {
                &Some(stop) => get_pos(l, stop),
                &None => l.len() as usize,
            };
            let step = match step {
                //Some(step) => step as usize,
                &None => 1 as usize,
                _ => unimplemented!(),
            };
            // TODO: we could potentially avoid this copy and use slice
            let obj = PyObject::new(
                PyObjectKind::Tuple {
                    elements: l[start..stop].to_vec(),
                },
                rt.get_type(),
            );
            Ok(obj)
        }
        _ => panic!(
            "TypeError: indexing type {:?} with index {:?} is not supported (yet?)",
            l, b
        ),
    }
}
