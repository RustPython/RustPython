use std::rc::Rc;
use std::cell::RefCell;
use std::ops::{Add, Mul, Sub};

/* Python objects and references.

Okay, so each python object itself is an class itself (PyObject). Each
python object can have several references to it (PyObjectRef). These
references are Rc (reference counting) rust smart pointers. So when
all references are destroyed, the object itself also can be cleaned up.
Basically reference counting, but then done by rust.

*/

/*
The PyRef type implements
https://doc.rust-lang.org/std/cell/index.html#introducing-mutability-inside-of-something-immutable
*/
pub type PyRef<T> = Rc<RefCell<T>>;
pub type PyObjectRef = PyRef<PyObject>;

#[derive(Debug)]
pub enum PyObject {
    String {
        value: String,
    },
    Integer {
        value: i32,
    },
    List {
        elements: Vec<PyObjectRef>,
    },
    Tuple {
        elements: Vec<PyObjectRef>,
    },
    Dict,
    Iterator {
        position: usize,
        iterated_obj: PyObjectRef,
    },
    None,
    RustFunction {
        function: fn(Vec<PyObjectRef>),
    },
}

/*
impl PyObjectRef {
    pub fn steal(&self) -> &mut PyObject {
        self.borrow_mut()
    }
}*/

impl PyObject {
    pub fn call(&self, args: Vec<PyObjectRef>) {
        match *self {
            PyObject::RustFunction { ref function } => {
                function(args);
            }
            _ => {
                println!("Not impl {:?}", self);
                panic!("Not impl");
            }
        }
    }

    pub fn str(&self) -> String {
        match *self {
            PyObject::String { ref value } => value.clone(),
            PyObject::Integer { ref value } => format!("{:?}", value),
            PyObject::List { ref elements } => format!("{:?}", elements),
            PyObject::Tuple { ref elements } => format!("{:?}", elements),
            PyObject::None => String::from("None"),
            _ => {
                println!("Not impl {:?}", self);
                panic!("Not impl");
            }
        }
    }

    // Implement iterator protocol:
    pub fn nxt(&mut self) -> Option<PyObjectRef> {
        match *self {
            PyObject::Iterator {
                ref mut position,
                iterated_obj: ref iterated_obj_ref,
            } => {
                let iterated_obj = &*iterated_obj_ref.borrow_mut();
                match iterated_obj {
                    &PyObject::List { ref elements } => {
                        if *position < elements.len() {
                            let obj_ref = elements[*position].clone();
                            *position += 1;
                            Some(obj_ref)
                        } else {
                            None
                        }
                    }
                    _ => {
                        panic!("NOT IMPL");
                    }
                }
            }
            _ => {
                panic!("NOT IMPL");
            }
        }
    }

    // Move this object into a reference object, transferring ownership.
    pub fn into_ref(self) -> PyObjectRef {
        Rc::new(RefCell::new(self))
    }
}

impl<'a> Add<&'a PyObject> for &'a PyObject {
    type Output = PyObject;

    fn add(self, rhs: &'a PyObject) -> Self::Output {
        match self {
            &PyObject::Integer { ref value } => {
                let value1 = value;
                match rhs {
                    &PyObject::Integer { ref value } => {
                        let value2 = value;
                        PyObject::Integer {
                            value: value1 + value2,
                        }
                    }
                    _ => {
                        panic!("NOT IMPL");
                    }
                }
            }
            _ => {
                // TODO: Lookup __add__ method in dictionary?
                panic!("NOT IMPL");
            }
        }
    }
}

impl<'a> Sub<&'a PyObject> for &'a PyObject {
    type Output = PyObject;

    fn sub(self, rhs: &'a PyObject) -> Self::Output {
        match self {
            &PyObject::Integer { value } => {
                let value1 = value;
                match rhs {
                    &PyObject::Integer { value } => {
                        let value2 = value;
                        PyObject::Integer {
                            value: value1 - value2,
                        }
                    }
                    _ => {
                        panic!("NOT IMPL");
                    }
                }
            }
            _ => {
                panic!("NOT IMPL");
            }
        }
    }
}

impl<'a> Mul<&'a PyObject> for &'a PyObject {
    type Output = PyObject;

    fn mul(self, rhs: &'a PyObject) -> Self::Output {
        match self {
            &PyObject::Integer { value } => {
                let value1 = value;
                match rhs {
                    &PyObject::Integer { value } => {
                        let value2 = value;
                        PyObject::Integer {
                            value: value1 * value2,
                        }
                    }
                    _ => {
                        panic!("NOT IMPL");
                    }
                }
            }
            &PyObject::String { ref value } => {
                let value1 = value;
                match rhs {
                    &PyObject::Integer { value } => {
                        let value2 = value;
                        let mut result = String::new();
                        for _x in 0..value2 {
                            result.push_str(value1.as_str());
                        }
                        PyObject::String { value: result }
                    }
                    _ => {
                        panic!("NOT IMPL");
                    }
                }
            }
            _ => {
                panic!("NOT IMPL");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PyObject;

    #[test]
    fn test_add_py_integers() {
        let a = PyObject::Integer { value: 33 };
        let b = PyObject::Integer { value: 12 };
        let c = &a + &b;
        match c {
            PyObject::Integer { value } => assert_eq!(value, 45),
            _ => assert!(false),
        }
    }

    #[test]
    fn test_multiply_str() {
        let a = PyObject::String {
            value: String::from("Hello "),
        };
        let b = PyObject::Integer { value: 4 };
        let c = &a * &b;
        match c {
            PyObject::String { value } => {
                assert_eq!(value, String::from("Hello Hello Hello Hello "))
            }
            _ => assert!(false),
        }
    }

}
