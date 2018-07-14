

fn subscript(rt: Executor, a, b: PyObjectRef) -> PyResult {
    match b.kind {
            (&NativeType::List(ref l), &NativeType::Int(ref index)) => {
                let pos_index = (index + l.borrow().len() as i32) % l.borrow().len() as i32;
                curr_frame.stack.push(Rc::new(l.borrow()[pos_index as usize].clone()))
            },
            (&NativeType::List(ref l), &NativeType::Slice(ref opt_start, ref opt_stop, ref opt_step)) => {
                let start = match opt_start {
                    &Some(start) => ((start + l.borrow().len() as i32) % l.borrow().len() as i32) as usize,
                    &None => 0,
                };
                let stop = match opt_stop {
                    &Some(stop) => ((stop + l.borrow().len() as i32) % l.borrow().len() as i32) as usize,
                    &None => l.borrow().len() as usize,
                };
                let step = match opt_step {
                    //Some(step) => step as usize,
                    &None => 1 as usize,
                    _ => unimplemented!(),
                };
                // TODO: we could potentially avoid this copy and use slice
                curr_frame.stack.push(Rc::new(NativeType::List(RefCell::new(l.borrow()[start..stop].to_vec()))));
            },
    }
}

