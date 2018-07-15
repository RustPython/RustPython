use super::pyobject::{Executor, PyObject, PyObjectKind, PyObjectRef, PyResult};

fn str_pos(s: &String, p: i32) -> usize {
    if p < 0 {
        s.len() - ((-p) as usize)
    } else if p as usize > s.len() {
        s.len()
    } else {
        p as usize
    }
}

pub fn subscript(rt: &mut Executor, value: &String, b: PyObjectRef) -> PyResult {
    // let value = a
    match &(*b.borrow()).kind {
        &PyObjectKind::Integer { value: ref pos } => {
            let idx = str_pos(value, *pos);
            Ok(rt.new_str(value[idx..idx + 1].to_string()))
        }
        &PyObjectKind::Slice {
            ref start,
            ref stop,
            ref step,
        } => {
            let start2: usize = match start {
                // &Some(_) => panic!("Bad start index for string slicing {:?}", start),
                &Some(start) => str_pos(value, start),
                &None => 0,
            };
            let stop2: usize = match stop {
                &Some(stop) => str_pos(value, stop),
                // &Some(_) => panic!("Bad stop index for string slicing"),
                &None => value.len() as usize,
            };
            let step2: usize = match step {
                //Some(step) => step as usize,
                &None => 1 as usize,
                _ => unimplemented!(),
            };
            Ok(rt.new_str(value[start2..stop2].to_string()))
        }
        _ => panic!(
            "TypeError: indexing type {:?} with index {:?} is not supported (yet?)",
            value, b
        ),
    }
}
