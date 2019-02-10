
// extern crate py_code_object;
use std::collections::HashMap;
use std::cell::RefCell;
use std::rc::Rc;
use std::ops::Deref;


const CMP_OP: &'static [&'static str] = &[">",
                                          "<=",
                                          "==",
                                          "!=",
                                          ">",
                                          ">=",
                                          "in",
                                          "not in",
                                          "is",
                                          "is not",
                                          "exception match",
                                          "BAD"
                                         ];

impl Frame {
    /// Get the current bytecode offset calculated from curr_frame.lasti
    fn get_bytecode_offset(&self) -> Option<usize> {
        // Linear search the labels HashMap, inefficient. Consider build a reverse HashMap
        let mut last_offset = None;
        for (offset, instr_idx) in self.labels.iter() {
            if *instr_idx == self.lasti {
                last_offset = Some(*offset)
            }
        }
        last_offset
    }
}


pub struct VirtualMachine{
    frames: Vec<Frame>,
}

impl VirtualMachine {
    fn unwind(&mut self, reason: String) {
        let curr_frame = self.curr_frame();
        let curr_block = curr_frame.blocks[curr_frame.blocks.len()-1].clone(); // use last?
        curr_frame.why = reason; // Why do we need this?
        debug!("block status: {:?}, {:?}", curr_block.block_type, curr_frame.why);
        match (curr_block.block_type.as_ref(), curr_frame.why.as_ref()) {
            ("loop", "break") => {
                curr_frame.lasti = curr_block.handler; //curr_frame.labels[curr_block.handler]; // Jump to the end
                // Return the why as None
                curr_frame.blocks.pop();
            },
            ("loop", "none") => (), //skipped
            _ => panic!("block stack operation not implemented")
        }
    }

    // Can we get rid of the code parameter?

    fn make_frame(&self, code: PyCodeObject, callargs: HashMap<String, Rc<NativeType>>, globals: Option<HashMap<String, Rc<NativeType>>>) -> Frame {
        //populate the globals and locals
        let mut labels = HashMap::new();
        let mut curr_offset = 0;
        for (idx, op) in code.co_code.iter().enumerate() {
            labels.insert(curr_offset, idx);
            curr_offset += op.0;
        }
        //TODO: This is wrong, check https://github.com/nedbat/byterun/blob/31e6c4a8212c35b5157919abff43a7daa0f377c6/byterun/pyvm2.py#L95
        let globals = match globals {
            Some(g) => g,
            None => HashMap::new(),
        };
        let mut locals = globals;
        locals.extend(callargs);

        //TODO: move this into the __builtin__ module when we have a module type
        locals.insert("print".to_string(), Rc::new(NativeType::NativeFunction(builtins::print)));
        locals.insert("len".to_string(), Rc::new(NativeType::NativeFunction(builtins::len)));
        Frame {
            code: code,
            stack: vec![],
            blocks: vec![],
            // save the callargs as locals
            globals: locals.clone(),
            locals: locals,
            labels: labels,
            lasti: 0,
            return_value: NativeType::NoneType,
            why: "none".to_string(),
        }
    }

    // The Option<i32> is the return value of the frame, remove when we have implemented frame
    // TODO: read the op codes directly from the internal code object
    fn run_frame(&mut self, frame: Frame) -> NativeType {
        self.frames.push(frame);

        //let mut why = None;
        // Change this to a loop for jump
        loop {
            //while curr_frame.lasti < curr_frame.code.co_code.len() {
            let op_code = {
                let curr_frame = self.curr_frame();
                if curr_frame.code.co_code.len() == 0 { panic!("Trying to run an empty frame. Check if the bytecode is empty"); }
                let op_code = curr_frame.code.co_code[curr_frame.lasti].clone();
                curr_frame.lasti += 1;
                op_code
            };
            let why = self.dispatch(op_code);
            /*if curr_frame.blocks.len() > 0 {
              self.manage_block_stack(&why);
              }
              */
            if let Some(_) = why {
                break;
            }
        }
        let return_value = {
            //let curr_frame = self.frames.last_mut().unwrap();
            self.curr_frame().return_value.clone()
        };
        self.pop_frame();
        return_value
    }

    pub fn run_code(&mut self, code: PyCodeObject) {
        let frame = self.make_frame(code, HashMap::new(), None);
        self.run_frame(frame);
        // check if there are any leftover frame, fail if any
    }

    fn dispatch(&mut self, op_code: (usize, String, Option<usize>)) -> Option<String> {
        match (op_code.1.as_ref(), op_code.2) {
            ("LOAD_CONST", Some(consti)) => {
                // println!("Loading const at index: {}", consti);
                let curr_frame = self.curr_frame();
                curr_frame.stack.push(Rc::new(curr_frame.code.co_consts[consti].clone()));
                None
            },

            // TODO: universal stack element type
            ("LOAD_CONST", None) => {
                // println!("Loading const at index: {}", consti);
                self.curr_frame().stack.push(Rc::new(NativeType::NoneType));
                None
            },
            ("POP_TOP", None) => {
                self.curr_frame().stack.pop();
                None
            },
            ("LOAD_FAST", Some(var_num)) => {
                // println!("Loading const at index: {}", consti);
                let curr_frame = self.curr_frame();
                let ref name = curr_frame.code.co_varnames[var_num];
                curr_frame.stack.push(curr_frame.locals.get::<str>(name).unwrap().clone());
                None
            },
            ("STORE_NAME", Some(namei)) => {
                // println!("Loading const at index: {}", consti);
                let curr_frame = self.curr_frame();
                curr_frame.locals.insert(curr_frame.code.co_names[namei].clone(), curr_frame.stack.pop().unwrap().clone());
                None
            },
            ("LOAD_NAME", Some(namei)) => {
                // println!("Loading const at index: {}", consti);
                let curr_frame = self.curr_frame();
                if let Some(code) = curr_frame.locals.get::<str>(&curr_frame.code.co_names[namei]) {
                    curr_frame.stack.push(code.clone());
                }
                else {
                    panic!("Can't find symbol {:?} in the current frame", &curr_frame.code.co_names[namei]);
                }
                None
            },
            ("LOAD_GLOBAL", Some(namei)) => {
                // We need to load the underlying value the name points to, but stuff like
                // AssertionError is in the names right after compile, so we load the string
                // instead for now
                let curr_frame = self.curr_frame();
                let name = &curr_frame.code.co_names[namei];
                curr_frame.stack.push(curr_frame.globals.get::<str>(name).unwrap().clone());
                None
            },

            ("BUILD_LIST", Some(count)) => {
                let curr_frame = self.curr_frame();
                let mut vec = vec!();
                for _ in 0..count {
                    vec.push((*curr_frame.stack.pop().unwrap()).clone());
                }
                vec.reverse();
                curr_frame.stack.push(Rc::new(NativeType::List(RefCell::new(vec))));
                None
            },

            ("BUILD_SLICE", Some(count)) => {
                let curr_frame = self.curr_frame();
                assert!(count == 2 || count == 3);
                let mut vec = vec!();
                for _ in 0..count {
                    vec.push(curr_frame.stack.pop().unwrap());
                }
                vec.reverse();
                let mut out:Vec<Option<i32>> = vec.into_iter().map(|x| match *x {
                    NativeType::Int(n) => Some(n),
                    NativeType::NoneType => None,
                    _ => panic!("Expect Int or None as BUILD_SLICE arguments, got {:?}", x),
                }).collect();

                if out.len() == 2 {
                    out.push(None);
                }
                assert!(out.len() == 3);
                // TODO: assert the stop start and step are NativeType::Int
                // See https://users.rust-lang.org/t/how-do-you-assert-enums/1187/8
                curr_frame.stack.push(Rc::new(NativeType::Slice(out[0], out[1], out[2])));
                None
            },

            ("GET_ITER", None) => {
                let curr_frame = self.curr_frame();
                let tos = curr_frame.stack.pop().unwrap();
                let iter = match *tos {
                    //TODO: is this clone right?
                    // Return a Iterator instead              vvv
                    NativeType::Tuple(ref vec) => NativeType::Iter(vec.clone()),
                    NativeType::List(ref vec) => NativeType::Iter(vec.borrow().clone()),
                    _ => panic!("TypeError: object is not iterable")
                };
                curr_frame.stack.push(Rc::new(iter));
                None
            },

            ("FOR_ITER", Some(delta)) => {
                // This function should be rewrote to use Rust native iterator
                let curr_frame = self.curr_frame();
                let tos = curr_frame.stack.pop().unwrap();
                let result = match *tos {
                    NativeType::Iter(ref v) =>  {
                        if v.len() > 0 {
                            Some(v.clone()) // Unnessary clone here
                        }
                        else {
                            None
                        }
                    }
                    _ => panic!("FOR_ITER: Not an iterator")
                };
                if let Some(vec) = result {
                    let (first, rest) = vec.split_first().unwrap();
                    // Unnessary clone here
                    curr_frame.stack.push(Rc::new(NativeType::Iter(rest.to_vec())));
                    curr_frame.stack.push(Rc::new(first.clone()));
                }
                else {
                    // Iterator was already poped in the first line of this function
                    let last_offset = curr_frame.get_bytecode_offset().unwrap();
                    curr_frame.lasti = curr_frame.labels.get(&(last_offset + delta)).unwrap().clone();

                }
                None
            },

            ("COMPARE_OP", Some(cmp_op_i)) => {
                let curr_frame = self.curr_frame();
                let v1 = curr_frame.stack.pop().unwrap();
                let v2 = curr_frame.stack.pop().unwrap();
                match CMP_OP[cmp_op_i] {
                    // To avoid branch explotion, use an array of callables instead
                    "==" => {
                        match (v1.deref(), v2.deref()) {
                            (&NativeType::Int(ref v1i), &NativeType::Int(ref v2i)) => {
                                curr_frame.stack.push(Rc::new(NativeType::Boolean(v2i == v1i)));
                            },
                            (&NativeType::Float(ref v1f), &NativeType::Float(ref v2f)) => {
                                curr_frame.stack.push(Rc::new(NativeType::Boolean(v2f == v1f)));
                            },
                            (&NativeType::Str(ref v1s), &NativeType::Str(ref v2s)) => {
                                curr_frame.stack.push(Rc::new(NativeType::Boolean(v2s == v1s)));
                            },
                            (&NativeType::Int(ref v1i), &NativeType::Float(ref v2f)) => {
                                curr_frame.stack.push(Rc::new(NativeType::Boolean(v2f == &(*v1i as f64))));
                            },
                            (&NativeType::List(ref l1), &NativeType::List(ref l2)) => {
                                curr_frame.stack.push(Rc::new(NativeType::Boolean(l2 == l1)));
                            },
                            _ => panic!("TypeError in COMPARE_OP: can't compare {:?} with {:?}", v1, v2)
                        };
                    }
                    ">" => {
                        match (v1.deref(), v2.deref()) {
                            (&NativeType::Int(ref v1i), &NativeType::Int(ref v2i)) => {
                                curr_frame.stack.push(Rc::new(NativeType::Boolean(v2i < v1i)));
                            },
                            (&NativeType::Float(ref v1f), &NativeType::Float(ref v2f)) => {
                                curr_frame.stack.push(Rc::new(NativeType::Boolean(v2f < v1f)));
                            },
                            _ => panic!("TypeError in COMPARE_OP")
                        };
                    }
                    _ => panic!("Unimplemented COMPARE_OP operator")

                }
                None
                
            },
            ("POP_JUMP_IF_TRUE", Some(ref target)) => {
                let curr_frame = self.curr_frame();
                let v = curr_frame.stack.pop().unwrap();
                if *v == NativeType::Boolean(true) {
                    curr_frame.lasti = curr_frame.labels.get(target).unwrap().clone();
                }
                None

            }
            ("POP_JUMP_IF_FALSE", Some(ref target)) => {
                let curr_frame = self.curr_frame();
                let v = curr_frame.stack.pop().unwrap();
                if *v == NativeType::Boolean(false) {
                    curr_frame.lasti = curr_frame.labels.get(target).unwrap().clone();
                }
                None
                
            }
            ("JUMP_FORWARD", Some(ref delta)) => {
                let curr_frame = self.curr_frame();
                let last_offset = curr_frame.get_bytecode_offset().unwrap();
                curr_frame.lasti = curr_frame.labels.get(&(last_offset + delta)).unwrap().clone();
                None
            },
            ("JUMP_ABSOLUTE", Some(ref target)) => {
                let curr_frame = self.curr_frame();
                curr_frame.lasti = curr_frame.labels.get(target).unwrap().clone();
                None
            },
            ("BREAK_LOOP", None) => {
                // Do we still need to return the why if we use unwind from jsapy?
                self.unwind("break".to_string());
                None //?
            },
            ("RAISE_VARARGS", Some(argc)) => {
                let curr_frame = self.curr_frame();
                // let (exception, params, traceback) = match argc {
                let exception = match argc {
                    1 => curr_frame.stack.pop().unwrap(),
                    0 | 2 | 3 => panic!("Not implemented!"),
                    _ => panic!("Invalid parameter for RAISE_VARARGS, must be between 0 to 3")
                };
                panic!("{:?}", exception);
            }
            ("INPLACE_ADD", None) => {
                let curr_frame = self.curr_frame();
                let tos = curr_frame.stack.pop().unwrap();
                let tos1 = curr_frame.stack.pop().unwrap();
                match (tos.deref(), tos1.deref()) {
                    (&NativeType::Int(ref tosi), &NativeType::Int(ref tos1i)) => {
                        curr_frame.stack.push(Rc::new(NativeType::Int(tos1i + tosi)));
                    },
                    _ => panic!("TypeError in BINARY_ADD")
                }
                None
            },
            
            ("STORE_SUBSCR", None) => {
                let curr_frame = self.curr_frame();
                let tos = curr_frame.stack.pop().unwrap();
                let tos1 = curr_frame.stack.pop().unwrap();
                let tos2 = curr_frame.stack.pop().unwrap();
                match (tos1.deref(), tos.deref()) {
                    (&NativeType::List(ref refl), &NativeType::Int(index)) => {
                        refl.borrow_mut()[index as usize] = (*tos2).clone();
                    },
                    (&NativeType::Str(_), &NativeType::Int(_)) => {
                        // TODO: raise TypeError: 'str' object does not support item assignment
                        panic!("TypeError: 'str' object does not support item assignment")
                    },
                    _ => panic!("TypeError in STORE_SUBSCR")
                }
                curr_frame.stack.push(tos1);
                None
            },

            ("BINARY_ADD", None) => {
                let curr_frame = self.curr_frame();
                let v1 = curr_frame.stack.pop().unwrap();
                let v2 = curr_frame.stack.pop().unwrap();
                match (v1.deref(), v2.deref()) {
                    (&NativeType::Int(ref v1i), &NativeType::Int(ref v2i)) => {
                        curr_frame.stack.push(Rc::new(NativeType::Int(v2i + v1i)));
                    }
                    (&NativeType::Float(ref v1f), &NativeType::Int(ref v2i)) => {
                        curr_frame.stack.push(Rc::new(NativeType::Float(*v2i as f64 + v1f)));
                    } 
                    (&NativeType::Int(ref v1i), &NativeType::Float(ref v2f)) => {
                        curr_frame.stack.push(Rc::new(NativeType::Float(v2f + *v1i as f64)));
                    }
                    (&NativeType::Float(ref v1f), &NativeType::Float(ref v2f)) => {
                        curr_frame.stack.push(Rc::new(NativeType::Float(v2f + v1f)));
                    }
                    (&NativeType::Str(ref str1), &NativeType::Str(ref str2)) => {
                        curr_frame.stack.push(Rc::new(NativeType::Str(format!("{}{}", str2, str1))));
                    }
                    (&NativeType::List(ref l1), &NativeType::List(ref l2)) => {
                        let mut new_l = l2.clone();
                        // TODO: remove unnessary copy
                        new_l.borrow_mut().append(&mut l1.borrow().clone());
                        curr_frame.stack.push(Rc::new(NativeType::List(new_l)));

                    }
                    _ => panic!("TypeError in BINARY_ADD")
                }
                None
            },
            ("BINARY_POWER", None) => {
                let curr_frame = self.curr_frame();
                let v1 = curr_frame.stack.pop().unwrap();
                let v2 = curr_frame.stack.pop().unwrap();
                match (v1.deref(), v2.deref()) {
                    (&NativeType::Int(v1i), &NativeType::Int(v2i)) => {
                        curr_frame.stack.push(Rc::new(NativeType::Int(v2i.pow(v1i as u32))));
                    }
                    (&NativeType::Float(v1f), &NativeType::Int(v2i)) => {
                        curr_frame.stack.push(Rc::new(NativeType::Float((v2i as f64).powf(v1f))));
                    } 
                    (&NativeType::Int(v1i), &NativeType::Float(v2f)) => {
                        curr_frame.stack.push(Rc::new(NativeType::Float(v2f.powi(v1i))));
                    }
                    (&NativeType::Float(v1f), &NativeType::Float(v2f)) => {
                        curr_frame.stack.push(Rc::new(NativeType::Float(v2f.powf(v1f))));
                    }
                    _ => panic!("TypeError in BINARY_POWER")
                }
                None
            },
            ("BINARY_MULTIPLY", None) => {
                let curr_frame = self.curr_frame();
                let v1 = curr_frame.stack.pop().unwrap();
                let v2 = curr_frame.stack.pop().unwrap();
                match (v1.deref(), v2.deref()) {
                    (&NativeType::Int(v1i), &NativeType::Int(v2i)) => {
                        curr_frame.stack.push(Rc::new(NativeType::Int(v2i * v1i)));
                    },
                    /*
                    (NativeType::Float(v1f), NativeType::Int(v2i)) => {
                        curr_frame.stack.push(NativeType::Float((v2i as f64) * v1f));
                    },
                    (NativeType::Int(v1i), NativeType::Float(v2f)) => {
                        curr_frame.stack.push(NativeType::Float(v2f * (v1i as f64)));
                    },
                    (NativeType::Float(v1f), NativeType::Float(v2f)) => {
                        curr_frame.stack.push(NativeType::Float(v2f * v1f));
                    },
                    */
                    //TODO: String multiply
                    _ => panic!("TypeError in BINARY_MULTIPLY")
                }
                None
            },
            ("BINARY_TRUE_DIVIDE", None) => {
                let curr_frame = self.curr_frame();
                let v1 = curr_frame.stack.pop().unwrap();
                let v2 = curr_frame.stack.pop().unwrap();
                match (v1.deref(), v2.deref()) {
                    (&NativeType::Int(v1i), &NativeType::Int(v2i)) => {
                        curr_frame.stack.push(Rc::new(NativeType::Int(v2i / v1i)));
                    },
                    _ => panic!("TypeError in BINARY_DIVIDE")
                }
                None
            },
            ("BINARY_MODULO", None) => {
                let curr_frame = self.curr_frame();
                let v1 = curr_frame.stack.pop().unwrap();
                let v2 = curr_frame.stack.pop().unwrap();
                match (v1.deref(), v2.deref()) {
                    (&NativeType::Int(v1i), &NativeType::Int(v2i)) => {
                        curr_frame.stack.push(Rc::new(NativeType::Int(v2i % v1i)));
                    },
                    _ => panic!("TypeError in BINARY_MODULO")
                }
                None
            },
            ("BINARY_SUBTRACT", None) => {
                let curr_frame = self.curr_frame();
                let v1 = curr_frame.stack.pop().unwrap();
                let v2 = curr_frame.stack.pop().unwrap();
                match (v1.deref(), v2.deref()) {
                    (&NativeType::Int(v1i), &NativeType::Int(v2i)) => {
                        curr_frame.stack.push(Rc::new(NativeType::Int(v2i - v1i)));
                    },
                    _ => panic!("TypeError in BINARY_SUBSTRACT")
                }
                None
            },

            ("BINARY_SUBSCR", None) => {
                let curr_frame = self.curr_frame();
                let tos = curr_frame.stack.pop().unwrap();
                let tos1 = curr_frame.stack.pop().unwrap();
                debug!("tos: {:?}, tos1: {:?}", tos, tos1);
                match (tos1.deref(), tos.deref()) {
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
                    (&NativeType::Tuple(ref t), &NativeType::Int(ref index)) => curr_frame.stack.push(Rc::new(t[*index as usize].clone())),
                    (&NativeType::Str(ref s), &NativeType::Int(ref index)) => {
                        let idx = (index + s.len() as i32) % s.len() as i32;
                        curr_frame.stack.push(Rc::new(NativeType::Str(s.chars().nth(idx as usize).unwrap().to_string())));
                    },
                    (&NativeType::Str(ref s), &NativeType::Slice(ref opt_start, ref opt_stop, ref opt_step)) => {
                        let start = match opt_start {
                            &Some(start) if start > s.len()  as i32 => s.len(),
                            &Some(start) if start <= s.len() as i32 => ((start + s.len() as i32) % s.len() as i32) as usize,
                            &Some(_) => panic!("Bad start index for string slicing"),
                            &Some(start) => ((start + s.len() as i32) % s.len() as i32) as usize,
                            &None => 0,
                        };
                        let stop = match opt_stop {
                            &Some(stop) if stop > s.len() as i32 => s.len(),
                            &Some(stop) if stop <= s.len() as i32 => ((stop + s.len() as i32) % s.len() as i32) as usize, // Do we need this modding?
                            &Some(_) => panic!("Bad stop index for string slicing"),
                            &None => s.len() as usize,
                        };
                        let step = match opt_step {
                            //Some(step) => step as usize,
                            &None => 1 as usize,
                            _ => unimplemented!(),
                        };
                        curr_frame.stack.push(Rc::new(NativeType::Str(s[start..stop].to_string())));
                    },
                    // TODO: implement other Slice possibilities
                    _ => panic!("TypeError: indexing type {:?} with index {:?} is not supported (yet?)", tos1, tos)
                };
                None
            },
            ("ROT_TWO", None) => {
                let curr_frame = self.curr_frame();
                let tos = curr_frame.stack.pop().unwrap();
                let tos1 = curr_frame.stack.pop().unwrap();
                curr_frame.stack.push(tos);
                curr_frame.stack.push(tos1);
                None
            }
            ("CALL_FUNCTION", Some(argc)) => {
                let kw_count = (argc >> 8) as u8;
                let pos_count = (argc & 0xFF) as u8;
                // Pop the arguments based on argc
                let mut kw_args = HashMap::new();
                let mut pos_args = Vec::new();
                {
                    let curr_frame = self.curr_frame();
                    for _ in 0..kw_count {
                        let native_val = curr_frame.stack.pop().unwrap();
                        let native_key = curr_frame.stack.pop().unwrap();
                        if let (ref val, &NativeType::Str(ref key)) = (native_val, native_key.deref()) {

                            kw_args.insert(key.clone(), val.clone());
                        }
                        else {
                            panic!("Incorrect type found while building keyword argument list")
                        }
                    }
                    for _ in 0..pos_count {
                        pos_args.push(curr_frame.stack.pop().unwrap());
                    }
                }
                let locals = {
                    // FIXME: no clone here
                    self.curr_frame().locals.clone()
                };

                let func = {
                    match self.curr_frame().stack.pop().unwrap().deref() {
                        &NativeType::Function(ref func) => {
                            // pop argc arguments
                            // argument: name, args, globals
                            // build the callargs hashmap
                            pos_args.reverse();
                            let mut callargs = HashMap::new();
                            for (name, val) in func.code.co_varnames.iter().zip(pos_args) {
                                callargs.insert(name.to_string(), val);
                            }
                            // merge callargs with kw_args
                            let return_value = {
                                let frame = self.make_frame(func.code.clone(), callargs, Some(locals));
                                self.run_frame(frame)
                            };
                            self.curr_frame().stack.push(Rc::new(return_value));
                        },
                        &NativeType::NativeFunction(func) => {
                            pos_args.reverse();
                            let return_value = func(pos_args);
                            self.curr_frame().stack.push(Rc::new(return_value));
                        },
                        _ => panic!("The item on the stack should be a code object")
                    }
                };
                None
            },
            ("RETURN_VALUE", None) => {
                // Hmmm... what is this used?
                // I believe we need to push this to the next frame
                self.curr_frame().return_value = (*self.curr_frame().stack.pop().unwrap()).clone();
                Some("return".to_string())
            },
            ("SETUP_LOOP", Some(delta)) => {
                let curr_frame = self.curr_frame();
                let curr_offset = curr_frame.get_bytecode_offset().unwrap();
                curr_frame.blocks.push(Block {
                    block_type: "loop".to_string(),
                    handler: *curr_frame.labels.get(&(curr_offset + delta)).unwrap(),
                });
                None
            },
            ("POP_BLOCK", None) => {
                self.curr_frame().blocks.pop();
                None
            }
            ("SetLineno", _) | ("LABEL", _)=> {
                // Skip
                None
            },
            (name, _) => {
                panic!("Unrecongnizable op code: {}", name);
            }
        } // end match
    } // end dispatch function
}

#[test]
fn test_tuple_serialization(){
    let tuple = NativeType::Tuple(vec![NativeType::Int(1),NativeType::Int(2)]);
    println!("{}", serde_json::to_string(&tuple).unwrap());
}
